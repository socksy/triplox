//! Dense bitset rows over the boolean semiring, with hand-written NEON kernels.
//!
//! The CSR form in `super::adjacency` is good for row slices but its set operations are
//! branchy sorted merges. For the boolean-semiring shapes (reachability, triangle masking)
//! a dense bit-per-node row turns those into word-parallel AND/OR plus popcount, which is
//! what the ARM kernels below execute 128 bits at a time.
//!
//! Memory is quadratic in the node count, so `BitMatrix::build` refuses to build above
//! `MAX_BITMATRIX_BYTES` and callers fall back to the sorted-merge path.

use std::sync::OnceLock;

use super::adjacency::Csr;

/// Dense rows cost n^2/8 bytes; above this the sparse path is used instead.
const MAX_BITMATRIX_BYTES: usize = 64 << 20;

/// Sorted node ids; a node's position in this list is its bit position.
pub(crate) struct NodeIndex {
    ids: Vec<i64>,
}

impl NodeIndex {
    fn from_sorted_union(left: &[i64], right: &[i64]) -> Self {
        let mut ids = Vec::with_capacity(left.len() + right.len());
        let (mut i, mut j) = (0, 0);
        while i < left.len() && j < right.len() {
            match left[i].cmp(&right[j]) {
                std::cmp::Ordering::Less => {
                    ids.push(left[i]);
                    i += 1;
                }
                std::cmp::Ordering::Greater => {
                    ids.push(right[j]);
                    j += 1;
                }
                std::cmp::Ordering::Equal => {
                    ids.push(left[i]);
                    i += 1;
                    j += 1;
                }
            }
        }
        ids.extend_from_slice(&left[i..]);
        ids.extend_from_slice(&right[j..]);
        Self { ids }
    }

    pub(crate) fn len(&self) -> usize {
        self.ids.len()
    }

    pub(crate) fn position(&self, id: i64) -> Option<usize> {
        self.ids.binary_search(&id).ok()
    }

    pub(crate) fn id(&self, position: usize) -> i64 {
        self.ids[position]
    }
}

/// One bit per (row node, column node), rows padded to whole 128-bit vectors.
pub(crate) struct BitMatrix {
    index: NodeIndex,
    words_per_row: usize,
    bits: Vec<u64>,
}

impl BitMatrix {
    /// Dense form of `out`, over the node space `out.keys() union in_keys`. Returns None
    /// when the dense form would exceed the memory cap.
    pub(crate) fn build(out: &Csr, in_keys: &[i64]) -> Option<Self> {
        let index = NodeIndex::from_sorted_union(out.keys(), in_keys);
        let n = index.len();
        // Two u64 words per NEON vector, so pad each row to an even word count.
        let words_per_row = n.div_ceil(64).next_multiple_of(2);
        if n.checked_mul(words_per_row)?.checked_mul(8)? > MAX_BITMATRIX_BYTES {
            return None;
        }
        let mut bits = vec![0u64; n * words_per_row];
        for row in 0..n {
            let base = row * words_per_row;
            for column in out.row(index.id(row)) {
                if let Some(bit) = index.position(*column) {
                    bits[base + bit / 64] |= 1u64 << (bit % 64);
                }
            }
        }
        Some(Self {
            index,
            words_per_row,
            bits,
        })
    }

    pub(crate) fn index(&self) -> &NodeIndex {
        &self.index
    }

    pub(crate) fn words_per_row(&self) -> usize {
        self.words_per_row
    }

    pub(crate) fn row(&self, position: usize) -> &[u64] {
        let base = position * self.words_per_row;
        &self.bits[base..base + self.words_per_row]
    }

    /// An empty accumulator shaped like one row.
    pub(crate) fn zero_row(&self) -> Vec<u64> {
        vec![0u64; self.words_per_row]
    }

    /// Bit set for `ids`, ignoring ids the matrix has never seen.
    pub(crate) fn row_of_ids(&self, ids: &[i64]) -> Vec<u64> {
        let mut words = self.zero_row();
        for id in ids {
            if let Some(bit) = self.index.position(*id) {
                words[bit / 64] |= 1u64 << (bit % 64);
            }
        }
        words
    }

    /// Every row node, as a bit set.
    pub(crate) fn all_rows(&self) -> Vec<u64> {
        let mut words = self.zero_row();
        for bit in 0..self.index.len() {
            words[bit / 64] |= 1u64 << (bit % 64);
        }
        words
    }

    /// OR together the rows of every set bit in `active`.
    pub(crate) fn spread(&self, active: &[u64]) -> Vec<u64> {
        let mut next = self.zero_row();
        for position in set_bits(active) {
            or_into(&mut next, self.row(position));
        }
        next
    }
}

/// Positions of the set bits, low to high.
pub(crate) fn set_bits(words: &[u64]) -> impl Iterator<Item = usize> + '_ {
    words.iter().enumerate().flat_map(|(word, value)| {
        let mut value = *value;
        std::iter::from_fn(move || {
            (value != 0).then(|| {
                let bit = value.trailing_zeros() as usize;
                value &= value - 1;
                word * 64 + bit
            })
        })
    })
}

// ---------------------------------------------------------------------------
// Word-parallel kernels
// ---------------------------------------------------------------------------

/// `dst |= src`. Slices must be the same length and a multiple of two words.
pub(crate) fn or_into(dst: &mut [u64], src: &[u64]) {
    debug_assert_eq!(dst.len(), src.len());
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON is unconditionally present on aarch64, and the loop reads and
        // writes two in-bounds u64 lanes per step from equally long slices.
        unsafe {
            use std::arch::aarch64::{vld1q_u64, vorrq_u64, vst1q_u64};
            let chunks = dst.len() / 2;
            let (dst_ptr, src_ptr) = (dst.as_mut_ptr(), src.as_ptr());
            for chunk in 0..chunks {
                let offset = chunk * 2;
                let a = vld1q_u64(dst_ptr.add(offset));
                let b = vld1q_u64(src_ptr.add(offset));
                vst1q_u64(dst_ptr.add(offset), vorrq_u64(a, b));
            }
            for lane in chunks * 2..dst.len() {
                dst[lane] |= src[lane];
            }
        }
        return;
    }
    #[cfg(not(target_arch = "aarch64"))]
    for (lhs, rhs) in dst.iter_mut().zip(src) {
        *lhs |= *rhs;
    }
}

/// Population count of `left & right`, i.e. one entry of the masked product.
pub(crate) fn and_popcount(left: &[u64], right: &[u64]) -> u64 {
    debug_assert_eq!(left.len(), right.len());
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON is unconditionally present on aarch64, and every load reads two
        // in-bounds u64 lanes from equally long slices.
        unsafe {
            use std::arch::aarch64::{
                vaddvq_u8, vandq_u64, vcntq_u8, vld1q_u64, vreinterpretq_u8_u64,
            };
            let chunks = left.len() / 2;
            let (left_ptr, right_ptr) = (left.as_ptr(), right.as_ptr());
            let mut total: u64 = 0;
            for chunk in 0..chunks {
                let offset = chunk * 2;
                let a = vld1q_u64(left_ptr.add(offset));
                let b = vld1q_u64(right_ptr.add(offset));
                let bytes = vreinterpretq_u8_u64(vandq_u64(a, b));
                // 16 lanes of at most 8 bits set, so the u8 horizontal sum cannot overflow.
                total += u64::from(vaddvq_u8(vcntq_u8(bytes)));
            }
            for lane in chunks * 2..left.len() {
                total += u64::from((left[lane] & right[lane]).count_ones());
            }
            return total;
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    left.iter()
        .zip(right)
        .map(|(a, b)| u64::from((a & b).count_ones()))
        .sum()
}

/// Population count of a bit set.
pub(crate) fn popcount(words: &[u64]) -> u64 {
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON is unconditionally present on aarch64 and every load is in bounds.
        unsafe {
            use std::arch::aarch64::{vaddvq_u8, vcntq_u8, vld1q_u64, vreinterpretq_u8_u64};
            let chunks = words.len() / 2;
            let ptr = words.as_ptr();
            let mut total: u64 = 0;
            for chunk in 0..chunks {
                let value = vld1q_u64(ptr.add(chunk * 2));
                total += u64::from(vaddvq_u8(vcntq_u8(vreinterpretq_u8_u64(value))));
            }
            for lane in chunks * 2..words.len() {
                total += u64::from(words[lane].count_ones());
            }
            return total;
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    words.iter().map(|w| u64::from(w.count_ones())).sum()
}

/// Lazily built dense form of one adjacency orientation.
#[derive(Default)]
pub(crate) struct BitMatrixCell(OnceLock<Option<BitMatrix>>);

impl BitMatrixCell {
    pub(crate) fn get_or_init(
        &self,
        build: impl FnOnce() -> Option<BitMatrix>,
    ) -> Option<&BitMatrix> {
        self.0.get_or_init(build).as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(bits: &[usize], len: usize) -> Vec<u64> {
        let mut out = vec![0u64; len];
        for bit in bits {
            out[bit / 64] |= 1u64 << (bit % 64);
        }
        out
    }

    #[test]
    fn or_and_popcount_kernels_agree_with_scalar() {
        for len in [2usize, 4, 6, 10] {
            let left = words(&[0, 63, 64, 65, len * 64 - 1], len);
            let right = words(&[1, 63, 64, len * 64 - 1], len);
            let expected_and: u64 = left
                .iter()
                .zip(&right)
                .map(|(a, b)| u64::from((a & b).count_ones()))
                .sum();
            assert_eq!(and_popcount(&left, &right), expected_and);
            assert_eq!(popcount(&left), 5);

            let mut merged = left.clone();
            or_into(&mut merged, &right);
            let expected_or: Vec<u64> = left.iter().zip(&right).map(|(a, b)| a | b).collect();
            assert_eq!(merged, expected_or);
        }
    }

    #[test]
    fn set_bits_enumerates_positions() {
        let value = words(&[0, 5, 64, 130], 4);
        assert_eq!(set_bits(&value).collect::<Vec<_>>(), vec![0, 5, 64, 130]);
        assert_eq!(popcount(&value), 4);
    }

    #[test]
    fn node_index_unions_sorted_lists() {
        let index = NodeIndex::from_sorted_union(&[1, 4, 9], &[4, 5]);
        assert_eq!(index.len(), 4);
        assert_eq!(index.position(5), Some(2));
        assert_eq!(index.position(7), None);
        assert_eq!(index.id(3), 9);
    }
}
