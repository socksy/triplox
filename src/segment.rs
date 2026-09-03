//! Columnar segment storage for the AEV and AVE indexes.
//!
//! In the row layout every datom is its own SlateDB key. In the columnar layout
//! runs of sorted datoms for one attribute are packed into a segment stored under
//! one key: the full row key of the segment's *last* datom. Keying by the last
//! datom means a forward `seek(target)` lands on the segment that contains
//! `target`, which is all a forward-only LSM iterator can give us.
//!
//! Segment value layout (all integers little-endian):
//!
//! ```text
//! u8 version | u8 index byte | u32 n
//! column first   (E for AEV, V for AVE)
//! column second  (V for AEV, E for AVE)
//! column ops     (bitmap, 1 = retract)
//! column tx      (i64 tx_eid)
//! ```
//!
//! Every column is `u32 byte_len` + payload so a reader can skip the ones it
//! does not need. Integer columns are frame-of-reference + bit-packed. A value
//! column whose entries are all `DataType::Long` is stored as an integer
//! column, otherwise as an offset array + concatenated encoded bytes.
//!
//! Segmentation is deliberately simple: segments never cross an attribute
//! boundary, a write merges new datoms into the segment whose key range covers
//! them and re-chunks the result into `segment_size` pieces, and datoms that
//! sort after the last existing segment start a new segment. There is no
//! background merge of small segments.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::OnceLock;

use anyhow::{bail, Result};
use bytes::Bytes;
use slatedb::{DbReadOps, WriteBatch};

use crate::codec;
use crate::protocol::TAG_LONG;
use crate::slate::DEFAULT_SCAN_OPTIONS;
use crate::util::next_prefix;

pub const DEFAULT_SEGMENT_SIZE: usize = 1024;
const SEGMENT_VERSION: u8 = 1;
const COLUMN_LONGS: u8 = 0;
const COLUMN_RAW: u8 = 1;

/// Index byte + attribute id: the prefix every segment of one attribute shares.
pub(crate) const ATTR_PREFIX_LEN: usize = codec::CODEC_LENGTH + codec::ATTRIBUTE_LENGTH;

/// How many datoms share one SlateDB key, and in what shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentLayout {
    /// One datom per key, empty values.
    Row,
    /// Row-major segments: up to `segment_size` whole datom keys in one value,
    /// in every index, keyed by the segment's last datom (see `crate::row_segment`).
    RowSegments { segment_size: usize },
    /// AEV/AVE packed into struct-of-arrays segments, AE/AV served from the first column.
    Columnar { segment_size: usize },
}

impl SegmentLayout {
    /// `TRIPLOX_SEGMENT_LAYOUT` picks the shape (`row`, `row-segments`, `columnar`)
    /// and `TRIPLOX_SEGMENT_SIZE` sizes the segments of the two segmented shapes.
    pub fn from_env() -> Self {
        static LAYOUT: OnceLock<SegmentLayout> = OnceLock::new();
        *LAYOUT.get_or_init(|| {
            let name = std::env::var("TRIPLOX_SEGMENT_LAYOUT").unwrap_or_default();
            let segment_size = std::env::var("TRIPLOX_SEGMENT_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|n: &usize| *n > 0)
                .unwrap_or(DEFAULT_SEGMENT_SIZE);
            if name.eq_ignore_ascii_case("columnar") {
                SegmentLayout::Columnar { segment_size }
            } else if name.eq_ignore_ascii_case("row-segments") && segment_size > 1 {
                SegmentLayout::RowSegments { segment_size }
            } else {
                SegmentLayout::Row
            }
        })
    }

    pub fn is_columnar(&self) -> bool {
        matches!(self, SegmentLayout::Columnar { .. })
    }

    /// True when one SlateDB value holds several whole datom keys.
    pub fn is_row_segments(&self) -> bool {
        matches!(self, SegmentLayout::RowSegments { .. })
    }

    pub fn segment_size(&self) -> usize {
        match self {
            SegmentLayout::Row => 1,
            SegmentLayout::RowSegments { segment_size }
            | SegmentLayout::Columnar { segment_size } => *segment_size,
        }
    }
}

pub(crate) fn is_segmented_index(index: u8) -> bool {
    index == codec::AEV || index == codec::AVE
}

// ---------------------------------------------------------------------------
// Integer column: frame of reference + bit packing
// ---------------------------------------------------------------------------

fn encode_i64_column(vals: &[i64], out: &mut Vec<u8>) {
    let min = vals.iter().copied().min().unwrap_or(0);
    let max = vals.iter().copied().max().unwrap_or(0);
    let range = (max as i128 - min as i128) as u128;
    let width = (128 - range.leading_zeros()) as u8;
    out.extend_from_slice(&min.to_le_bytes());
    out.push(width);
    let mut acc: u128 = 0;
    let mut nbits: u32 = 0;
    for &v in vals {
        let d = (v as i128 - min as i128) as u64;
        acc |= (d as u128) << nbits;
        nbits += width as u32;
        while nbits >= 8 {
            out.push(acc as u8);
            acc >>= 8;
            nbits -= 8;
        }
    }
    if nbits > 0 {
        out.push(acc as u8);
    }
}

fn decode_i64_column(mut buf: &[u8], n: usize) -> Result<Vec<i64>> {
    if buf.len() < 9 {
        bail!("segment integer column truncated");
    }
    let min = i64::from_le_bytes(buf[..8].try_into().unwrap());
    let width = buf[8] as u32;
    buf = &buf[9..];
    let mut vals = Vec::with_capacity(n);
    if width == 0 {
        vals.resize(n, min);
        return Ok(vals);
    }
    let mask: u128 = (1u128 << width) - 1;
    let mut acc: u128 = 0;
    let mut nbits: u32 = 0;
    let mut pos = 0;
    for _ in 0..n {
        while nbits < width {
            let Some(&b) = buf.get(pos) else {
                bail!("segment integer column truncated");
            };
            acc |= (b as u128) << nbits;
            nbits += 8;
            pos += 1;
        }
        let d = (acc & mask) as u64;
        acc >>= width;
        nbits -= width;
        vals.push((min as i128 + d as i128) as i64);
    }
    Ok(vals)
}

// ---------------------------------------------------------------------------
// Generic column: all-Long integers or raw encoded bytes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Column {
    Longs(Vec<i64>),
    Raw { offsets: Vec<u32>, bytes: Vec<u8> },
}

impl Column {
    /// Build from encoded `DataType` bytes (type tag included).
    fn from_encoded(parts: &[&[u8]]) -> Self {
        let all_long = parts
            .iter()
            .all(|p| p.len() == codec::ENTITY_LENGTH && p[0] == TAG_LONG);
        if all_long {
            let vals = parts
                .iter()
                .map(|p| i64::from_be_bytes(p[1..9].try_into().unwrap()) ^ i64::MAX)
                .collect();
            return Column::Longs(vals);
        }
        let mut offsets = Vec::with_capacity(parts.len() + 1);
        let mut bytes = Vec::new();
        offsets.push(0);
        for p in parts {
            bytes.extend_from_slice(p);
            offsets.push(bytes.len() as u32);
        }
        Column::Raw { offsets, bytes }
    }

    fn encode(&self, out: &mut Vec<u8>) {
        let start = out.len();
        out.extend_from_slice(&0u32.to_le_bytes());
        match self {
            Column::Longs(vals) => {
                out.push(COLUMN_LONGS);
                encode_i64_column(vals, out);
            }
            Column::Raw { offsets, bytes } => {
                out.push(COLUMN_RAW);
                for o in offsets {
                    out.extend_from_slice(&o.to_le_bytes());
                }
                out.extend_from_slice(bytes);
            }
        }
        let len = (out.len() - start - 4) as u32;
        out[start..start + 4].copy_from_slice(&len.to_le_bytes());
    }

    fn decode(buf: &[u8], n: usize) -> Result<Self> {
        match buf.first() {
            Some(&COLUMN_LONGS) => Ok(Column::Longs(decode_i64_column(&buf[1..], n)?)),
            Some(&COLUMN_RAW) => {
                let buf = &buf[1..];
                let offsets_len = (n + 1) * 4;
                if buf.len() < offsets_len {
                    bail!("segment raw column truncated");
                }
                let offsets = buf[..offsets_len]
                    .chunks_exact(4)
                    .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                    .collect();
                Ok(Column::Raw {
                    offsets,
                    bytes: buf[offsets_len..].to_vec(),
                })
            }
            other => bail!("unknown segment column kind {other:?}"),
        }
    }

    pub(crate) fn len(&self) -> usize {
        match self {
            Column::Longs(v) => v.len(),
            Column::Raw { offsets, .. } => offsets.len() - 1,
        }
    }

    /// Encoded bytes of entry `i`. Longs are re-encoded into `scratch`.
    #[inline]
    pub(crate) fn get<'a>(&'a self, i: usize, scratch: &'a mut [u8; 9]) -> &'a [u8] {
        match self {
            Column::Longs(v) => {
                scratch[0] = TAG_LONG;
                scratch[1..].copy_from_slice(&codec::encode_i64_bytes(v[i]));
                scratch
            }
            Column::Raw { offsets, bytes } => &bytes[offsets[i] as usize..offsets[i + 1] as usize],
        }
    }

    /// Byte-equality of entries `i` and `j`.
    #[inline]
    pub(crate) fn same(&self, i: usize, j: usize) -> bool {
        match self {
            Column::Longs(v) => v[i] == v[j],
            Column::Raw { offsets, bytes } => {
                bytes[offsets[i] as usize..offsets[i + 1] as usize]
                    == bytes[offsets[j] as usize..offsets[j + 1] as usize]
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Segment
// ---------------------------------------------------------------------------

/// A decoded segment. `second` and `txs` are empty when only the first column was read.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Segment {
    pub index: u8,
    pub first: Column,
    pub second: Column,
    pub ops: Vec<u8>,
    pub txs: Vec<i64>,
}

fn split_row_key(index: u8, rest: &[u8]) -> Result<(&[u8], &[u8], i64, u8)> {
    if rest.len() < codec::ENTITY_LENGTH + codec::TX_EID_OP_SUFFIX {
        bail!("row key too short for segment");
    }
    let logical = &rest[..rest.len() - codec::TX_EID_OP_SUFFIX];
    let mut t = &rest[rest.len() - codec::TX_EID_OP_SUFFIX..rest.len() - codec::OP_LENGTH];
    let tx = codec::decode_i64(&mut t).map_err(|e| anyhow::anyhow!("{e}"))?;
    let op = rest[rest.len() - 1];
    let (first, second) = match index {
        codec::AEV => logical.split_at(codec::ENTITY_LENGTH),
        codec::AVE => logical.split_at(logical.len() - codec::ENTITY_LENGTH),
        other => bail!("index {other} is not segmented"),
    };
    Ok((first, second, tx, op))
}

impl Segment {
    /// Build from sorted full row keys that share the same `ATTR_PREFIX_LEN` prefix.
    pub(crate) fn from_row_keys(index: u8, keys: &[Bytes]) -> Result<Self> {
        let mut firsts = Vec::with_capacity(keys.len());
        let mut seconds = Vec::with_capacity(keys.len());
        let mut txs = Vec::with_capacity(keys.len());
        let mut ops = vec![0u8; keys.len().div_ceil(8)];
        for (i, key) in keys.iter().enumerate() {
            let (first, second, tx, op) = split_row_key(index, &key[ATTR_PREFIX_LEN..])?;
            firsts.push(first);
            seconds.push(second);
            txs.push(tx);
            if op == codec::RETRACT {
                ops[i / 8] |= 1 << (i % 8);
            }
        }
        Ok(Segment {
            index,
            first: Column::from_encoded(&firsts),
            second: Column::from_encoded(&seconds),
            ops,
            txs,
        })
    }

    pub(crate) fn encode(&self) -> Vec<u8> {
        let n = self.first.len();
        let mut out = Vec::with_capacity(n * 8);
        out.push(SEGMENT_VERSION);
        out.push(self.index);
        out.extend_from_slice(&(n as u32).to_le_bytes());
        self.first.encode(&mut out);
        self.second.encode(&mut out);
        out.extend_from_slice(&(self.ops.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.ops);
        let start = out.len();
        out.extend_from_slice(&0u32.to_le_bytes());
        encode_i64_column(&self.txs, &mut out);
        let len = (out.len() - start - 4) as u32;
        out[start..start + 4].copy_from_slice(&len.to_le_bytes());
        out
    }

    fn header(buf: &[u8]) -> Result<(u8, usize, &[u8])> {
        if buf.len() < 6 || buf[0] != SEGMENT_VERSION {
            bail!("bad segment header");
        }
        let n = u32::from_le_bytes(buf[2..6].try_into().unwrap()) as usize;
        Ok((buf[1], n, &buf[6..]))
    }

    fn take_column(buf: &[u8]) -> Result<(&[u8], &[u8])> {
        if buf.len() < 4 {
            bail!("segment column length truncated");
        }
        let len = u32::from_le_bytes(buf[..4].try_into().unwrap()) as usize;
        if buf.len() < 4 + len {
            bail!("segment column truncated");
        }
        Ok((&buf[4..4 + len], &buf[4 + len..]))
    }

    /// Number of datoms without decoding any column.
    pub(crate) fn count(buf: &[u8]) -> Result<usize> {
        Ok(Self::header(buf)?.1)
    }

    pub(crate) fn decode(buf: &[u8]) -> Result<Self> {
        let (index, n, rest) = Self::header(buf)?;
        let (first, rest) = Self::take_column(rest)?;
        let (second, rest) = Self::take_column(rest)?;
        let (ops, rest) = Self::take_column(rest)?;
        let (txs, _) = Self::take_column(rest)?;
        Ok(Segment {
            index,
            first: Column::decode(first, n)?,
            second: Column::decode(second, n)?,
            ops: ops.to_vec(),
            txs: decode_i64_column(txs, n)?,
        })
    }

    /// Decode only the first column and the op bitmap (an AE/AV style scan).
    pub(crate) fn decode_first_column(buf: &[u8]) -> Result<Self> {
        let (index, n, rest) = Self::header(buf)?;
        let (first, rest) = Self::take_column(rest)?;
        let (_second, rest) = Self::take_column(rest)?;
        let (ops, _) = Self::take_column(rest)?;
        Ok(Segment {
            index,
            first: Column::decode(first, n)?,
            // n+1 zero offsets: every second entry reads as empty, so key
            // comparisons see only the first column.
            second: Column::Raw {
                offsets: vec![0; n + 1],
                bytes: Vec::new(),
            },
            ops: ops.to_vec(),
            txs: Vec::new(),
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.first.len()
    }

    #[inline]
    pub(crate) fn is_retract(&self, i: usize) -> bool {
        self.ops[i / 8] & (1 << (i % 8)) != 0
    }

    /// Full row key of datom `i` (attribute prefix included).
    pub(crate) fn row_key(&self, attr_prefix: &[u8], i: usize) -> Vec<u8> {
        let mut s1 = [0u8; 9];
        let mut s2 = [0u8; 9];
        let first = self.first.get(i, &mut s1);
        let second = self.second.get(i, &mut s2);
        let mut key = Vec::with_capacity(attr_prefix.len() + first.len() + second.len() + 9);
        key.extend_from_slice(attr_prefix);
        key.extend_from_slice(first);
        key.extend_from_slice(second);
        key.extend_from_slice(&codec::encode_i64_bytes(self.txs[i]));
        key.push(if self.is_retract(i) {
            codec::RETRACT
        } else {
            codec::ADD
        });
        key
    }

    /// Compare datom `i`'s key (without attribute prefix) against `target`,
    /// which may be a prefix of a key or extend past the first column.
    pub(crate) fn cmp_key(&self, i: usize, target: &[u8]) -> Ordering {
        let mut s1 = [0u8; 9];
        let mut s2 = [0u8; 9];
        let tx = if self.txs.is_empty() {
            [0u8; 8]
        } else {
            codec::encode_i64_bytes(self.txs[i])
        };
        let op = [if self.txs.is_empty() || !self.is_retract(i) {
            codec::ADD
        } else {
            codec::RETRACT
        }];
        let has_tail = !self.txs.is_empty();
        let parts: [&[u8]; 4] = [
            self.first.get(i, &mut s1),
            self.second.get(i, &mut s2),
            if has_tail { &tx } else { &[] },
            if has_tail { &op } else { &[] },
        ];
        cmp_parts(&parts, target)
    }

    /// Index of the first datom whose key is `>= target` (without attribute prefix).
    pub(crate) fn lower_bound(&self, target: &[u8]) -> usize {
        let (mut lo, mut hi) = (0, self.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.cmp_key(mid, target) == Ordering::Less {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }

    /// Does datom `i`'s key (without attribute prefix) start with `prefix`?
    pub(crate) fn key_starts_with(&self, i: usize, prefix: &[u8]) -> bool {
        let mut s1 = [0u8; 9];
        let mut s2 = [0u8; 9];
        let first = self.first.get(i, &mut s1);
        if prefix.len() <= first.len() {
            return first.starts_with(prefix);
        }
        if !prefix.starts_with(first) {
            return false;
        }
        let rest = &prefix[first.len()..];
        let second = self.second.get(i, &mut s2);
        if rest.len() <= second.len() {
            return second.starts_with(rest);
        }
        // Prefixes never reach into the tx/op suffix.
        false
    }
}

/// Lexicographically compare the virtual concatenation of `parts` with `target`.
fn cmp_parts(parts: &[&[u8]], target: &[u8]) -> Ordering {
    let mut t = target;
    for part in parts {
        let n = part.len().min(t.len());
        match part[..n].cmp(&t[..n]) {
            Ordering::Equal => {}
            other => return other,
        }
        if part.len() > n {
            return Ordering::Greater;
        }
        t = &t[n..];
    }
    if t.is_empty() {
        Ordering::Equal
    } else {
        Ordering::Less
    }
}

/// Exclusive end of the attribute's key range, if one exists.
pub(crate) fn attr_range_end(attr_prefix: &[u8]) -> Option<Vec<u8>> {
    next_prefix(&attr_prefix[..ATTR_PREFIX_LEN])
}

// ---------------------------------------------------------------------------
// Write path
// ---------------------------------------------------------------------------

/// Merge new full row keys for one segmented index into the existing segments.
/// Keys are sorted, grouped by attribute, and each group is merged into the
/// segments whose ranges cover it; the merged runs are re-chunked to `segment_size`.
pub(crate) async fn merge_into_segments<D>(
    db: &D,
    batch: &mut WriteBatch,
    index: u8,
    mut keys: Vec<Bytes>,
    segment_size: usize,
) -> Result<()>
where
    D: DbReadOps + Sync,
{
    keys.sort();
    keys.dedup();
    let mut groups: BTreeMap<Vec<u8>, Vec<Bytes>> = BTreeMap::new();
    for key in keys {
        groups
            .entry(key[..ATTR_PREFIX_LEN].to_vec())
            .or_default()
            .push(key);
    }

    for (attr_prefix, keys) in groups {
        let mut iter = match attr_range_end(&attr_prefix) {
            Some(end) => {
                db.scan_with_options(attr_prefix.clone()..end, &DEFAULT_SCAN_OPTIONS)
                    .await?
            }
            None => {
                db.scan_with_options(attr_prefix.clone().., &DEFAULT_SCAN_OPTIONS)
                    .await?
            }
        };
        let mut i = 0;
        while i < keys.len() {
            iter.seek(&keys[i]).await?;
            let existing = match iter.next().await? {
                Some(kv) if kv.key.starts_with(&attr_prefix) => Some(kv),
                _ => None,
            };
            let (mut merged, old_key) = match existing {
                Some(kv) => {
                    let segment = Segment::decode(&kv.value)?;
                    let rows: Vec<Bytes> = (0..segment.len())
                        .map(|r| Bytes::from(segment.row_key(&attr_prefix, r)))
                        .collect();
                    (rows, Some(kv.key))
                }
                None => (Vec::new(), None),
            };
            let end = match &old_key {
                Some(seg_key) => keys[i..].partition_point(|k| k <= seg_key) + i,
                None => keys.len(),
            };
            merged.extend_from_slice(&keys[i..end]);
            merged.sort();
            merged.dedup();
            i = end;

            let mut keep_old_key = false;
            for chunk in merged.chunks(segment_size) {
                let seg_key = chunk.last().unwrap();
                if Some(seg_key) == old_key.as_ref() {
                    keep_old_key = true;
                }
                let segment = Segment::from_row_keys(index, chunk)?;
                batch.put(seg_key, segment.encode());
            }
            if let Some(seg_key) = old_key {
                if !keep_old_key {
                    batch.delete(seg_key);
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Read helpers for non-query callers
// ---------------------------------------------------------------------------

/// All row keys under `prefix` (which must start with an attribute prefix),
/// reconstructed from segments, in key order.
pub(crate) async fn row_keys_with_prefix<D>(db: &D, prefix: &[u8]) -> Result<Vec<Bytes>>
where
    D: DbReadOps + Sync,
{
    if prefix.len() < ATTR_PREFIX_LEN {
        bail!("segment prefix scans need at least an attribute prefix");
    }
    let attr_prefix = &prefix[..ATTR_PREFIX_LEN];
    let mut iter = match attr_range_end(attr_prefix) {
        Some(end) => {
            db.scan_with_options(prefix.to_vec()..end, &DEFAULT_SCAN_OPTIONS)
                .await?
        }
        None => {
            db.scan_with_options(prefix.to_vec().., &DEFAULT_SCAN_OPTIONS)
                .await?
        }
    };
    let rest = &prefix[ATTR_PREFIX_LEN..];
    let mut out = Vec::new();
    while let Some(kv) = iter.next().await? {
        let segment = Segment::decode(&kv.value)?;
        let start = segment.lower_bound(rest);
        for i in start..segment.len() {
            if !segment.key_starts_with(i, rest) {
                return Ok(out);
            }
            out.push(Bytes::from(segment.row_key(attr_prefix, i)));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Encode;
    use crate::ops::DataType;

    fn row_key(index: u8, attr: i64, e: i64, v: &DataType, tx: i64, op: u8) -> Bytes {
        let mut key = vec![index];
        key.extend_from_slice(&codec::encode_i64_bytes(attr));
        let e = DataType::Long(e).encode();
        let v = v.encode();
        match index {
            codec::AEV => {
                key.extend_from_slice(&e);
                key.extend_from_slice(&v);
            }
            codec::AVE => {
                key.extend_from_slice(&v);
                key.extend_from_slice(&e);
            }
            _ => unreachable!(),
        }
        key.extend_from_slice(&codec::encode_i64_bytes(tx));
        key.push(op);
        Bytes::from(key)
    }

    #[test]
    fn i64_column_roundtrip() {
        let cases: Vec<Vec<i64>> = vec![
            vec![],
            vec![5],
            vec![1, 2, 3, 4, 5],
            vec![i64::MIN, i64::MAX, 0, -1],
            (0..1000).map(|i| i * 7 - 300).collect(),
        ];
        for vals in cases {
            let mut buf = Vec::new();
            encode_i64_column(&vals, &mut buf);
            assert_eq!(decode_i64_column(&buf, vals.len()).unwrap(), vals);
        }
    }

    #[test]
    fn segment_roundtrip_and_ordering() {
        for index in [codec::AEV, codec::AVE] {
            let mut keys: Vec<Bytes> = Vec::new();
            for e in 0..50 {
                for v in 0..3 {
                    keys.push(row_key(
                        index,
                        7,
                        e,
                        &DataType::Long(v * 11),
                        100 + e,
                        codec::ADD,
                    ));
                }
                keys.push(row_key(
                    index,
                    7,
                    e,
                    &DataType::String(format!("s{e}")),
                    101,
                    codec::RETRACT,
                ));
            }
            keys.sort();
            let attr_prefix = keys[0][..ATTR_PREFIX_LEN].to_vec();
            let segment = Segment::from_row_keys(index, &keys).unwrap();
            let decoded = Segment::decode(&segment.encode()).unwrap();
            assert_eq!(decoded, segment);
            for (i, key) in keys.iter().enumerate() {
                assert_eq!(decoded.row_key(&attr_prefix, i), key.as_ref());
                assert_eq!(decoded.cmp_key(i, &key[ATTR_PREFIX_LEN..]), Ordering::Equal);
                assert_eq!(decoded.lower_bound(&key[ATTR_PREFIX_LEN..]), i);
                assert!(decoded.key_starts_with(i, &key[ATTR_PREFIX_LEN..key.len() - 9]));
            }
            let first_only = Segment::decode_first_column(&segment.encode()).unwrap();
            assert_eq!(first_only.first, segment.first);
            assert_eq!(first_only.ops, segment.ops);
        }
    }

    #[test]
    fn all_long_values_pack_as_integers() {
        let keys: Vec<Bytes> = (0..10)
            .map(|e| row_key(codec::AEV, 1, e, &DataType::Long(e * 2), 5, codec::ADD))
            .collect();
        let segment = Segment::from_row_keys(codec::AEV, &keys).unwrap();
        assert!(matches!(segment.second, Column::Longs(_)));
        // 10 datoms: header + three tiny packed columns + bitmap is well under row size.
        assert!(segment.encode().len() < keys.iter().map(|k| k.len()).sum::<usize>());
    }
}
