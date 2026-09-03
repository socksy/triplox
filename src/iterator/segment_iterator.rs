use std::cmp::Ordering;
use std::sync::Arc;

use anyhow::{bail, Error};
use bytes::Bytes;
use slatedb::{DbMetadataOps, DbReadOps};
use tokio::runtime::Handle;

use crate::codec;
use crate::index::IndexType;
use crate::segment::{attr_range_end, Segment, ATTR_PREFIX_LEN};
use crate::slate::DEFAULT_SCAN_OPTIONS;

use super::slate_iterator::{Extractor, Index};

/// Which columns a scan needs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ScanMode {
    /// AEV/AVE: decode every column, resolve temporal versions per logical key.
    Pair,
    /// AE/AV: decode the first column and the op bitmap only, emit each
    /// distinct first component that has at least one assertion.
    First,
}

/// Iterates datoms stored in columnar segments and presents them with the same
/// key shapes the row layout produces, so extractors and callers stay unchanged.
///
/// AE/AV scans are served from the first column of AEV/AVE segments.
pub(crate) struct SegmentIterator<M>
where
    M: DbMetadataOps + Send + Sync,
{
    inner: slatedb::DbIterator,
    handle: Handle,
    extractor: Extractor,
    range_stats: Arc<slatedb_estimates::RangeStats<M>>,
    segment_size: usize,
    mode: ScanMode,
    caller_index: u8,
    caller_prefix: Bytes,
    attr_prefix: Vec<u8>,
    rest_prefix: Vec<u8>,
    as_of: i64,
    segment: Option<(Bytes, Segment)>,
    pos: usize,
    current: Option<Bytes>,
    last_group: Option<Vec<u8>>,
    group_emitted: bool,
    done: bool,
}

impl<M> SegmentIterator<M>
where
    M: DbMetadataOps + Send + Sync,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new<D>(
        index_type: IndexType,
        prefix: &[u8],
        slate: &D,
        handle: Handle,
        extractor: Extractor,
        as_of: i64,
        range_stats: Arc<slatedb_estimates::RangeStats<M>>,
        segment_size: usize,
    ) -> Result<Self, Error>
    where
        D: DbReadOps + Send + Sync,
    {
        if prefix.len() < ATTR_PREFIX_LEN {
            bail!("segment scans need at least an attribute prefix");
        }
        let (storage_index, mode) = match index_type {
            IndexType::AEV => (codec::AEV, ScanMode::Pair),
            IndexType::AVE => (codec::AVE, ScanMode::Pair),
            IndexType::AE => (codec::AEV, ScanMode::First),
            IndexType::AV => (codec::AVE, ScanMode::First),
            other => bail!("{other:?} is not stored in segments"),
        };
        let mut attr_prefix = vec![storage_index];
        attr_prefix.extend_from_slice(&prefix[1..ATTR_PREFIX_LEN]);
        let rest_prefix = prefix[ATTR_PREFIX_LEN..].to_vec();
        let mut start = attr_prefix.clone();
        start.extend_from_slice(&rest_prefix);
        let inner = match attr_range_end(&attr_prefix) {
            Some(end) => {
                handle.block_on(slate.scan_with_options(start..end, &DEFAULT_SCAN_OPTIONS))?
            }
            None => handle.block_on(slate.scan_with_options(start.., &DEFAULT_SCAN_OPTIONS))?,
        };
        let mut iter = Self {
            inner,
            handle,
            extractor,
            range_stats,
            segment_size,
            mode,
            caller_index: prefix[0],
            caller_prefix: Bytes::copy_from_slice(prefix),
            attr_prefix,
            rest_prefix,
            as_of,
            segment: None,
            pos: 0,
            current: None,
            last_group: None,
            group_emitted: false,
            done: false,
        };
        if iter.load_next_segment()? {
            let rest = iter.rest_prefix.clone();
            iter.pos = iter.segment.as_ref().unwrap().1.lower_bound(&rest);
        }
        iter.advance_to_next_valid()?;
        Ok(iter)
    }

    fn load_next_segment(&mut self) -> Result<bool, Error> {
        self.segment = None;
        self.pos = 0;
        let Some(kv) = self.handle.block_on(self.inner.next())? else {
            return Ok(false);
        };
        if !kv.key.starts_with(&self.attr_prefix) {
            return Ok(false);
        }
        let segment = match self.mode {
            ScanMode::Pair => Segment::decode(&kv.value)?,
            ScanMode::First => Segment::decode_first_column(&kv.value)?,
        };
        self.segment = Some((kv.key, segment));
        Ok(true)
    }

    /// Bytes identifying datom `i`'s group: the logical key (Pair) or first component (First).
    fn group_bytes(&self, segment: &Segment, i: usize) -> Vec<u8> {
        let mut s1 = [0u8; 9];
        let mut s2 = [0u8; 9];
        let mut out = segment.first.get(i, &mut s1).to_vec();
        if self.mode == ScanMode::Pair {
            out.extend_from_slice(segment.second.get(i, &mut s2));
        }
        out
    }

    fn same_group(&self, segment: &Segment, i: usize) -> bool {
        let Some(last) = &self.last_group else {
            return false;
        };
        let mut s1 = [0u8; 9];
        let mut s2 = [0u8; 9];
        let first = segment.first.get(i, &mut s1);
        match self.mode {
            ScanMode::First => first == last.as_slice(),
            ScanMode::Pair => {
                if !last.starts_with(first) {
                    return false;
                }
                segment.second.get(i, &mut s2) == &last[first.len()..]
            }
        }
    }

    fn caller_key(&self, segment: &Segment, i: usize) -> Bytes {
        match self.mode {
            ScanMode::Pair => Bytes::from(segment.row_key(&self.attr_prefix, i)),
            ScanMode::First => {
                let mut s1 = [0u8; 9];
                let mut key = Vec::with_capacity(ATTR_PREFIX_LEN + 9);
                key.push(self.caller_index);
                key.extend_from_slice(&self.attr_prefix[1..]);
                key.extend_from_slice(segment.first.get(i, &mut s1));
                Bytes::from(key)
            }
        }
    }

    fn advance_to_next_valid(&mut self) -> Result<(), Error> {
        loop {
            if self.done {
                self.current = None;
                return Ok(());
            }
            if self.segment.is_none() && !self.load_next_segment()? {
                self.done = true;
                continue;
            }
            let (_, segment) = self.segment.as_ref().unwrap();
            if self.pos >= segment.len() {
                self.segment = None;
                continue;
            }
            let i = self.pos;
            self.pos += 1;
            if !segment.key_starts_with(i, &self.rest_prefix) {
                self.done = true;
                continue;
            }
            match self.mode {
                ScanMode::Pair => {
                    if self.same_group(segment, i) || segment.txs[i] > self.as_of {
                        continue;
                    }
                    let group = self.group_bytes(segment, i);
                    self.last_group = Some(group);
                    if segment.is_retract(i) {
                        continue;
                    }
                }
                ScanMode::First => {
                    if self.same_group(segment, i) {
                        if self.group_emitted || segment.is_retract(i) {
                            continue;
                        }
                    } else {
                        let group = self.group_bytes(segment, i);
                        self.last_group = Some(group);
                        self.group_emitted = false;
                        if segment.is_retract(i) {
                            continue;
                        }
                    }
                    self.group_emitted = true;
                }
            }
            let (_, segment) = self.segment.as_ref().unwrap();
            self.current = Some(self.caller_key(segment, i));
            return Ok(());
        }
    }

    fn current_logical(&self) -> Option<&[u8]> {
        let current = self.current.as_ref()?;
        Some(match self.mode {
            ScanMode::Pair => &current[..current.len() - codec::TX_EID_OP_SUFFIX],
            ScanMode::First => current.as_ref(),
        })
    }
}

impl<M> Index for SegmentIterator<M>
where
    M: DbMetadataOps + Send + Sync,
{
    fn count(&self) -> Result<u64, Error> {
        let mut prefix = self.attr_prefix.clone();
        prefix.extend_from_slice(&self.rest_prefix);
        let segments = self
            .handle
            .block_on(self.range_stats.estimate_key_count_with_prefix(&prefix))?;
        Ok(segments.saturating_mul(self.segment_size as u64))
    }

    fn seek(&mut self, extension: Bytes) -> Result<(), Error> {
        let mut target = self.caller_prefix.to_vec();
        target.extend_from_slice(&extension);
        if let Some(current) = self.current_logical() {
            if current >= target.as_slice() {
                return Ok(());
            }
        }
        if self.done {
            return Ok(());
        }
        target[0] = self.attr_prefix[0];
        let rest_target = target[ATTR_PREFIX_LEN..].to_vec();

        let in_segment = match &self.segment {
            Some((seg_key, _)) => seg_key.as_ref().cmp(target.as_slice()) != Ordering::Less,
            None => false,
        };
        if !in_segment {
            self.handle.block_on(self.inner.seek(&target))?;
            if !self.load_next_segment()? {
                self.done = true;
                self.current = None;
                return Ok(());
            }
        }
        let (_, segment) = self.segment.as_ref().unwrap();
        self.pos = segment.lower_bound(&rest_target).max(self.pos);
        self.last_group = None;
        self.group_emitted = false;
        self.advance_to_next_valid()
    }

    fn next(&mut self) -> Result<Option<Bytes>, Error> {
        self.advance_to_next_valid()?;
        self.get_value()
    }

    fn get_value(&self) -> Result<Option<Bytes>, Error> {
        Ok(self.current.clone().map(|key| (self.extractor)(key)))
    }

    fn has_next(&self) -> bool {
        self.current.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Encode;
    use crate::ops::DataType;
    use crate::segment::merge_into_segments;
    use crate::slate::in_memory_slate;
    use crate::util::make_extractor;
    use slatedb::WriteBatch;

    fn row_key(index: u8, attr: i64, e: i64, v: &DataType, tx: i64, op: u8) -> Bytes {
        let mut key = vec![index];
        key.extend_from_slice(&codec::encode_i64_bytes(attr));
        let e = DataType::Long(e).encode();
        let v = v.encode();
        if index == codec::AEV {
            key.extend_from_slice(&e);
            key.extend_from_slice(&v);
        } else {
            key.extend_from_slice(&v);
            key.extend_from_slice(&e);
        }
        key.extend_from_slice(&codec::encode_i64_bytes(tx));
        key.push(op);
        Bytes::from(key)
    }

    fn attr_prefix(index: u8, attr: i64) -> Vec<u8> {
        let mut p = vec![index];
        p.extend_from_slice(&codec::encode_i64_bytes(attr));
        p
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pair_scan_resolves_versions_across_segments() {
        let components = in_memory_slate().await;
        let db = components.db.clone();
        // 40 entities, 3 values each, entity 5 retracted at tx 20, entity 7 re-added later.
        let mut keys = Vec::new();
        for e in 0..40 {
            for v in 0..3 {
                keys.push(row_key(
                    codec::AEV,
                    9,
                    e,
                    &DataType::Long(v),
                    10,
                    codec::ADD,
                ));
            }
        }
        keys.push(row_key(
            codec::AEV,
            9,
            5,
            &DataType::Long(1),
            20,
            codec::RETRACT,
        ));
        keys.push(row_key(
            codec::AEV,
            9,
            7,
            &DataType::Long(1),
            20,
            codec::RETRACT,
        ));
        keys.push(row_key(
            codec::AEV,
            9,
            7,
            &DataType::Long(1),
            30,
            codec::ADD,
        ));
        let mut batch = WriteBatch::new();
        merge_into_segments(db.as_ref(), &mut batch, codec::AEV, keys, 16)
            .await
            .unwrap();
        db.write(batch).await.unwrap();

        let range_stats = components.range_stats.clone();
        let handle = Handle::current();
        tokio::task::spawn_blocking(move || {
            let prefix = attr_prefix(codec::AEV, 9);
            let prefix_len = prefix.len();
            let collect = |as_of: i64| {
                let mut iter = SegmentIterator::new(
                    IndexType::AEV,
                    &prefix,
                    db.as_ref(),
                    handle.clone(),
                    Box::new(move |key: Bytes| {
                        key.slice(prefix_len..key.len() - codec::TX_EID_OP_SUFFIX)
                    }),
                    as_of,
                    range_stats.clone(),
                    16,
                )
                .unwrap();
                let mut out = Vec::new();
                while let Some(v) = iter.get_value().unwrap() {
                    out.push(v);
                    iter.next().unwrap();
                }
                out
            };
            assert_eq!(collect(5).len(), 0);
            assert_eq!(collect(10).len(), 120);
            assert_eq!(collect(20).len(), 118);
            assert_eq!(collect(30).len(), 119);

            // Seek to entity 7 as of 30 and read its group.
            let mut iter = SegmentIterator::new(
                IndexType::AEV,
                &prefix,
                db.as_ref(),
                handle.clone(),
                Box::new(move |key: Bytes| {
                    key.slice(prefix_len..key.len() - codec::TX_EID_OP_SUFFIX)
                }),
                30,
                range_stats.clone(),
                16,
            )
            .unwrap();
            let e7 = Bytes::from(DataType::Long(7).encode());
            iter.seek(e7.clone()).unwrap();
            let mut group = Vec::new();
            while let Some(v) = iter.get_value().unwrap() {
                if !v.starts_with(&e7) {
                    break;
                }
                group.push(v.slice(e7.len()..));
                iter.next().unwrap();
            }
            let expected: Vec<Bytes> = [2i64, 1, 0]
                .iter()
                .map(|v| Bytes::from(DataType::Long(*v).encode()))
                .collect();
            assert_eq!(group, expected);
        })
        .await
        .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn first_column_scan_matches_ae_semantics() {
        let components = in_memory_slate().await;
        let db = components.db.clone();
        let mut keys = Vec::new();
        for e in 0..30 {
            keys.push(row_key(codec::AEV, 3, e, &DataType::Long(e), 1, codec::ADD));
        }
        // Entity 100 only ever retracted: AE would not list it.
        keys.push(row_key(
            codec::AEV,
            3,
            100,
            &DataType::Long(1),
            1,
            codec::RETRACT,
        ));
        let mut batch = WriteBatch::new();
        merge_into_segments(db.as_ref(), &mut batch, codec::AEV, keys, 8)
            .await
            .unwrap();
        db.write(batch).await.unwrap();

        let range_stats = components.range_stats.clone();
        let handle = Handle::current();
        tokio::task::spawn_blocking(move || {
            let prefix = attr_prefix(codec::AE, 3);
            let mut iter = SegmentIterator::new(
                IndexType::AE,
                &prefix,
                db.as_ref(),
                handle,
                Box::new(|key| make_extractor(1, IndexType::AE)(key)),
                i64::MAX,
                range_stats,
                8,
            )
            .unwrap();
            let mut seen = Vec::new();
            while let Some(v) = iter.get_value().unwrap() {
                seen.push(v);
                iter.next().unwrap();
            }
            let mut expected: Vec<Bytes> = (0..30)
                .map(|e| Bytes::from(DataType::Long(e).encode()))
                .collect();
            expected.sort();
            assert_eq!(seen, expected);
        })
        .await
        .unwrap();
    }
}
