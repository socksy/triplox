//! Sparse adjacency matrices for ref attributes. One matrix per (attribute, basis),
//! built from a single AEV scan and cached on the node. Both orientations are kept
//! as CSR so row lookups work from either the entity or the value side.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{bail, Result};
use bytes::Bytes;
use slatedb::{DbMetadataOps, DbReadOps};

use crate::codec::{self, decode_datatype, encode_datatype};
use crate::db_value::DB;
use crate::index::IndexType;
use crate::iterator::slate_iterator::{Extractor, Index};
use crate::iterator::temporal_filter_iterator::TemporalFilterIterator;
use crate::ops::DataType;
use crate::query::bitset::{BitMatrix, BitMatrixCell};

pub(crate) fn encode_entity(id: i64) -> Bytes {
    let mut buf = Vec::with_capacity(codec::ENTITY_LENGTH);
    encode_datatype(&DataType::Long(id), &mut buf);
    Bytes::from(buf)
}

pub(crate) fn decode_entity(bytes: &[u8]) -> Option<i64> {
    let mut cursor = bytes;
    match decode_datatype(&mut cursor) {
        Ok(DataType::Long(id)) if cursor.is_empty() => Some(id),
        _ => None,
    }
}

/// Compressed sparse rows keyed by a sorted list of row ids.
pub(crate) struct Csr {
    keys: Vec<i64>,
    offsets: Vec<usize>,
    cols: Vec<i64>,
    // Every column id pre-encoded as a 9-byte entity so extensions are zero-copy slices.
    cols_enc: Bytes,
    keys_enc: Bytes,
}

impl Csr {
    // `pairs` must be sorted by (key, col) and free of duplicates.
    fn from_sorted_pairs(pairs: &[(i64, i64)]) -> Self {
        let mut keys = Vec::new();
        let mut offsets = vec![0];
        let mut cols = Vec::with_capacity(pairs.len());
        let mut cols_enc = Vec::with_capacity(pairs.len() * codec::ENTITY_LENGTH);
        for (key, col) in pairs {
            if keys.last() != Some(key) {
                keys.push(*key);
                offsets.push(cols.len());
            }
            cols.push(*col);
            encode_datatype(&DataType::Long(*col), &mut cols_enc);
            *offsets.last_mut().unwrap() = cols.len();
        }
        let mut keys_enc = Vec::with_capacity(keys.len() * codec::ENTITY_LENGTH);
        for key in &keys {
            encode_datatype(&DataType::Long(*key), &mut keys_enc);
        }
        Self {
            keys,
            offsets,
            cols,
            cols_enc: Bytes::from(cols_enc),
            keys_enc: Bytes::from(keys_enc),
        }
    }

    fn row_range(&self, key: i64) -> Option<std::ops::Range<usize>> {
        let index = self.keys.binary_search(&key).ok()?;
        Some(self.offsets[index]..self.offsets[index + 1])
    }

    pub(crate) fn keys(&self) -> &[i64] {
        &self.keys
    }

    pub(crate) fn has_key(&self, key: i64) -> bool {
        self.keys.binary_search(&key).is_ok()
    }

    pub(crate) fn row(&self, key: i64) -> &[i64] {
        self.row_range(key)
            .map(|range| &self.cols[range])
            .unwrap_or(&[])
    }

    pub(crate) fn contains(&self, key: i64, col: i64) -> bool {
        self.row(key).binary_search(&col).is_ok()
    }

    pub(crate) fn nnz(&self, key: i64) -> usize {
        self.row(key).len()
    }

    /// Encoded column ids of one row, sliced from the shared buffer.
    pub(crate) fn row_encoded(&self, key: i64) -> Vec<Bytes> {
        self.row_range(key)
            .map(|range| {
                range
                    .map(|i| {
                        self.cols_enc
                            .slice(i * codec::ENTITY_LENGTH..(i + 1) * codec::ENTITY_LENGTH)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn keys_encoded(&self) -> Vec<Bytes> {
        (0..self.keys.len())
            .map(|i| {
                self.keys_enc
                    .slice(i * codec::ENTITY_LENGTH..(i + 1) * codec::ENTITY_LENGTH)
            })
            .collect()
    }

    fn bytes(&self) -> usize {
        self.keys.len() * 8
            + self.offsets.len() * 8
            + self.cols.len() * 8
            + self.cols_enc.len()
            + self.keys_enc.len()
    }
}

pub(crate) struct AdjMatrix {
    // entity -> values
    pub(crate) out: Csr,
    // value -> entities
    pub(crate) inn: Csr,
    pub(crate) nnz: usize,
    pub(crate) build_time: std::time::Duration,
    out_bits: BitMatrixCell,
    inn_bits: BitMatrixCell,
}

impl AdjMatrix {
    /// Dense boolean form of one orientation, over the node space shared by both. None when
    /// the graph is too large for a dense matrix, in which case callers use the CSR instead.
    pub(crate) fn bits(&self, backwards: bool) -> Option<&BitMatrix> {
        let (cell, rows, other) = if backwards {
            (&self.inn_bits, &self.inn, &self.out)
        } else {
            (&self.out_bits, &self.out, &self.inn)
        };
        cell.get_or_init(|| BitMatrix::build(rows, other.keys()))
    }

    pub(crate) fn bytes(&self) -> usize {
        self.out.bytes() + self.inn.bytes()
    }

    /// Scan the AEV index for `attribute` as of the DB basis and build both orientations.
    pub(crate) fn build<D, M>(db: &DB<D, M>, attribute: i64) -> Result<Self>
    where
        D: DbReadOps + Send + Sync + 'static,
        M: DbMetadataOps + Send + Sync + 'static,
    {
        let start = Instant::now();
        let mut prefix = vec![codec::index_type_to_prefix(IndexType::AEV)?];
        codec::encode_i64(attribute, &mut prefix);
        let prefix_len = prefix.len();
        let extractor: Extractor =
            Box::new(move |key| key.slice(prefix_len..key.len() - codec::TX_EID_OP_SUFFIX));
        let mut iterator = TemporalFilterIterator::new(
            &prefix,
            db.sdb(),
            db.handle().clone(),
            extractor,
            db.as_of(),
            Arc::clone(db.range_stats()),
        )?;

        let mut pairs = Vec::new();
        while let Some(ev) = iterator.get_value()? {
            if ev.len() != 2 * codec::ENTITY_LENGTH {
                bail!("attribute {attribute} has a non-entity value, not a ref attribute");
            }
            let (entity, value) = (
                decode_entity(&ev[..codec::ENTITY_LENGTH]),
                decode_entity(&ev[codec::ENTITY_LENGTH..]),
            );
            let (Some(entity), Some(value)) = (entity, value) else {
                bail!("attribute {attribute} has a non-entity value, not a ref attribute");
            };
            pairs.push((entity, value));
            iterator.next()?;
        }
        // The key encoding does not sort like i64, so both orientations are re-sorted.
        pairs.sort_unstable();
        let out = Csr::from_sorted_pairs(&pairs);
        let mut transposed: Vec<(i64, i64)> = pairs.iter().map(|(e, v)| (*v, *e)).collect();
        transposed.sort_unstable();
        let inn = Csr::from_sorted_pairs(&transposed);
        Ok(Self {
            out,
            inn,
            nnz: pairs.len(),
            build_time: start.elapsed(),
            out_bits: BitMatrixCell::default(),
            inn_bits: BitMatrixCell::default(),
        })
    }
}

/// Matrices keyed by (attribute, tx_id). Shared across DB values of one node.
#[derive(Default)]
pub(crate) struct AdjacencyCache {
    matrices: Mutex<HashMap<(i64, i64), Arc<AdjMatrix>>>,
}

impl AdjacencyCache {
    pub(crate) fn get_or_build<D, M>(&self, db: &DB<D, M>, attribute: i64) -> Result<Arc<AdjMatrix>>
    where
        D: DbReadOps + Send + Sync + 'static,
        M: DbMetadataOps + Send + Sync + 'static,
    {
        let key = (attribute, db.tx_key().tx_id);
        if let Some(matrix) = self.matrices.lock().unwrap().get(&key) {
            return Ok(Arc::clone(matrix));
        }
        let matrix = Arc::new(AdjMatrix::build(db, attribute)?);
        if std::env::var_os("TRIPLOX_ADJ_MATRIX_LOG").is_some() {
            eprintln!(
                "adjacency matrix attr={attribute} tx={} nnz={} rows_out={} rows_in={} bytes={} build_ms={:.2}",
                key.1,
                matrix.nnz,
                matrix.out.keys().len(),
                matrix.inn.keys().len(),
                matrix.bytes(),
                matrix.build_time.as_secs_f64() * 1000.0
            );
        }
        self.matrices
            .lock()
            .unwrap()
            .insert(key, Arc::clone(&matrix));
        Ok(matrix)
    }
}

/// Sorted-slice intersection of every set in `sets`.
pub(crate) fn intersect_sorted(sets: &[&[i64]]) -> Vec<i64> {
    let Some((first, rest)) = sets.split_first() else {
        return Vec::new();
    };
    let mut current: Vec<i64> = first.to_vec();
    for other in rest {
        let (mut i, mut j) = (0, 0);
        let mut next = Vec::with_capacity(current.len().min(other.len()));
        while i < current.len() && j < other.len() {
            match current[i].cmp(&other[j]) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    next.push(current[i]);
                    i += 1;
                    j += 1;
                }
            }
        }
        current = next;
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csr_rows_and_membership() {
        let csr = Csr::from_sorted_pairs(&[(1, 2), (1, 5), (3, 1)]);
        assert_eq!(csr.keys(), &[1, 3]);
        assert_eq!(csr.row(1), &[2, 5]);
        assert_eq!(csr.row(3), &[1]);
        assert!(csr.row(2).is_empty());
        assert!(csr.contains(1, 5));
        assert!(!csr.contains(1, 3));
        assert_eq!(csr.nnz(1), 2);
        assert_eq!(csr.row_encoded(1), vec![encode_entity(2), encode_entity(5)]);
        assert_eq!(csr.keys_encoded(), vec![encode_entity(1), encode_entity(3)]);
        assert_eq!(decode_entity(&encode_entity(-7)), Some(-7));
    }

    #[test]
    fn intersects_sorted_sets() {
        assert_eq!(
            intersect_sorted(&[&[1, 2, 3, 5], &[2, 3, 4], &[3, 5]]),
            vec![3]
        );
        assert!(intersect_sorted(&[]).is_empty());
        assert_eq!(intersect_sorted(&[&[1, 2]]), vec![1, 2]);
    }
}
