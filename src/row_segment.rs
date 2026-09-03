//! Row-major segments: up to N sorted datom keys stored under one SlateDB key.
//!
//! Selected by `SegmentLayout::RowSegments`. Physical layout, per index:
//! - plain entry: `key = datom key`, `value = ""` (the `Row` layout, and what size 1 writes)
//! - segment: `key = last datom key in the segment`, `value = (u16 BE len, datom key)*`
//!
//! Keying a segment by its last datom means a forward SlateDB seek to `target` lands on
//! exactly the segment that could contain `target`. Readers accept both shapes in the
//! same index, so the writer's segment size can change between transactions, and a
//! reader over an unsegmented index behaves exactly like the raw SlateDB iterator.
//!
//! Unlike `crate::segment`, every index is segmented and the datom keys are stored
//! whole, so nothing here depends on the key's internal structure.

use anyhow::{bail, Result};
use bytes::Bytes;
use slatedb::config::ScanOptions;
use slatedb::{DbReadOps, IterationOrder, WriteBatch};

use crate::slate::DEFAULT_SCAN_OPTIONS;
use crate::util::next_prefix;

pub fn encode_segment(keys: &[Bytes]) -> Vec<u8> {
    let mut out = Vec::with_capacity(keys.iter().map(|k| k.len() + 2).sum());
    for key in keys {
        let len = u16::try_from(key.len()).expect("datom key exceeds u16 length");
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(key);
    }
    out
}

/// Datom keys held by one SlateDB entry. An empty value is a plain single-datom entry.
pub fn decode_segment(key: &Bytes, value: &Bytes) -> Result<Vec<Bytes>> {
    if value.is_empty() {
        return Ok(vec![key.clone()]);
    }
    let mut keys = Vec::new();
    let mut pos = 0;
    while pos < value.len() {
        if pos + 2 > value.len() {
            bail!("truncated segment length prefix");
        }
        let len = u16::from_be_bytes([value[pos], value[pos + 1]]) as usize;
        pos += 2;
        if pos + len > value.len() {
            bail!("truncated segment entry");
        }
        keys.push(value.slice(pos..pos + len));
        pos += len;
    }
    Ok(keys)
}

enum Current {
    None,
    Single(Bytes),
    Segment(Vec<Bytes>),
}

/// Forward cursor over datom keys of one index, transparent to the physical layout.
/// Pull style: `next()` yields the current datom and advances; `seek()` positions so
/// that the next `next()` yields the first datom `>= target`. Forward-only, like SlateDB.
pub(crate) struct KeyCursor {
    inner: slatedb::DbIterator,
    end: Option<Vec<u8>>,
    current: Current,
    pos: usize,
    exhausted: bool,
}

impl KeyCursor {
    /// Cursor over all datoms starting with `prefix` (first byte is the index byte).
    pub async fn scan_prefix<D>(db: &D, prefix: &[u8]) -> Result<Self>
    where
        D: DbReadOps + Sync,
    {
        assert!(!prefix.is_empty(), "prefix must start with an index byte");
        // A segment holding the tail of the prefix range is keyed past it, so scan to the
        // end of the index and stop at the first datom outside the prefix.
        let index_end = next_prefix(&prefix[..1]);
        let range = (
            std::ops::Bound::Included(prefix.to_vec()),
            match &index_end {
                Some(end) => std::ops::Bound::Excluded(end.clone()),
                None => std::ops::Bound::Unbounded,
            },
        );
        let inner = db.scan_with_options(range, &DEFAULT_SCAN_OPTIONS).await?;
        let mut cursor = Self {
            inner,
            end: next_prefix(prefix),
            current: Current::None,
            pos: 0,
            exhausted: false,
        };
        cursor.load_next().await?;
        cursor.skip_below(prefix);
        Ok(cursor)
    }

    async fn load_next(&mut self) -> Result<()> {
        match self.inner.next().await? {
            None => {
                self.current = Current::None;
                self.exhausted = true;
            }
            Some(kv) => {
                self.current = if kv.value.is_empty() {
                    Current::Single(kv.key)
                } else {
                    Current::Segment(decode_segment(&kv.key, &kv.value)?)
                };
                self.pos = 0;
            }
        }
        Ok(())
    }

    fn current_len(&self) -> usize {
        match &self.current {
            Current::None => 0,
            Current::Single(_) => 1,
            Current::Segment(keys) => keys.len(),
        }
    }

    fn current_at(&self, pos: usize) -> &Bytes {
        match &self.current {
            Current::Single(key) => key,
            Current::Segment(keys) => &keys[pos],
            Current::None => unreachable!("no current entry"),
        }
    }

    /// Advance `pos` past datoms below `target` within the loaded entry.
    fn skip_below(&mut self, target: &[u8]) {
        match &self.current {
            Current::None => {}
            Current::Single(key) => {
                if key.as_ref() < target {
                    self.pos = 1;
                }
            }
            Current::Segment(keys) => {
                let from = self.pos;
                self.pos = from + keys[from..].partition_point(|k| k.as_ref() < target);
            }
        }
    }

    fn past_end(&self, key: &[u8]) -> bool {
        self.end.as_deref().is_some_and(|end| key >= end)
    }

    pub fn peek(&self) -> Option<&Bytes> {
        if self.exhausted || self.pos >= self.current_len() {
            return None;
        }
        Some(self.current_at(self.pos))
    }

    pub async fn next(&mut self) -> Result<Option<Bytes>> {
        if self.exhausted {
            return Ok(None);
        }
        if self.pos >= self.current_len() {
            self.load_next().await?;
            if self.exhausted {
                return Ok(None);
            }
        }
        let key = self.current_at(self.pos).clone();
        if self.past_end(&key) {
            self.exhausted = true;
            self.current = Current::None;
            return Ok(None);
        }
        self.pos += 1;
        Ok(Some(key))
    }

    pub async fn seek(&mut self, target: &[u8]) -> Result<()> {
        if self.exhausted {
            return Ok(());
        }
        if self.past_end(target) {
            self.exhausted = true;
            self.current = Current::None;
            return Ok(());
        }
        let len = self.current_len();
        if len > 0 && self.current_at(len - 1).as_ref() >= target {
            self.skip_below(target);
            return Ok(());
        }
        // Target is beyond the loaded entry, whose key is the last SlateDB key returned,
        // so the forward seek is legal.
        self.inner.seek(target).await?;
        self.load_next().await?;
        self.skip_below(target);
        Ok(())
    }
}

/// Segment being edited during a write.
struct Pending {
    orig_key: Option<Bytes>,
    datoms: Vec<Bytes>,
}

impl Pending {
    fn insert(&mut self, key: Bytes) {
        match self.datoms.binary_search(&key) {
            Ok(_) => {}
            Err(pos) => self.datoms.insert(pos, key),
        }
    }

    /// Write the merged datoms back as segments of at most `segment_size`, keyed by the
    /// last datom of each. Splitting happens here, once the run is complete, because a
    /// key seen later in the same batch can still belong to an earlier part of the run.
    fn flush(self, batch: &mut WriteBatch, segment_size: usize) {
        if self.datoms.is_empty() {
            return;
        }
        let mut orig_rewritten = false;
        for chunk in self.datoms.chunks(segment_size) {
            let last = chunk.last().expect("non-empty chunk");
            orig_rewritten |= self.orig_key.as_ref() == Some(last);
            batch.put(last, encode_segment(chunk));
        }
        if let Some(orig) = &self.orig_key {
            if !orig_rewritten {
                batch.delete(orig);
            }
        }
    }
}

/// Merge sorted datom `keys` into the segments of their indexes, rewriting touched
/// segments and splitting those that grow past `segment_size`.
pub(crate) async fn write_segmented<D>(
    db: &D,
    batch: &mut WriteBatch,
    mut keys: Vec<Vec<u8>>,
    segment_size: usize,
) -> Result<()>
where
    D: DbReadOps + Sync,
{
    keys.sort_unstable();
    keys.dedup();
    let descending = ScanOptions {
        order: IterationOrder::Descending,
        ..DEFAULT_SCAN_OPTIONS
    };

    for group in keys.chunk_by(|a, b| a[0] == b[0]) {
        let index = group[0][0];
        let mut iter = db
            .scan_with_options(vec![index]..vec![index + 1], &DEFAULT_SCAN_OPTIONS)
            .await?;
        let mut pending: Option<Pending> = None;
        let mut at_tail = false;

        for key in group {
            let key = Bytes::copy_from_slice(key);
            let belongs_to_pending = pending
                .as_ref()
                .is_some_and(|p| at_tail || p.datoms.last().is_some_and(|last| key <= *last));
            if !belongs_to_pending {
                // `key` is above everything in `pending`, so the seek is forward.
                iter.seek(&key).await?;
                match iter.next().await? {
                    Some(kv) => {
                        if let Some(p) = pending.take() {
                            p.flush(batch, segment_size);
                        }
                        pending = Some(Pending {
                            datoms: decode_segment(&kv.key, &kv.value)?,
                            orig_key: Some(kv.key),
                        });
                    }
                    None => {
                        // Nothing at or above `key`, so `key` extends the last segment of
                        // the index. That is not necessarily `pending`: untouched segments
                        // can sit between them, and appending to `pending` would make two
                        // segments cover overlapping datom ranges.
                        at_tail = true;
                        let mut last = db
                            .scan_with_options(vec![index]..key.to_vec(), &descending)
                            .await?;
                        let tail = last.next().await?;
                        let adopt = tail.as_ref().is_some_and(|kv| {
                            pending
                                .as_ref()
                                .and_then(|p| p.orig_key.as_ref())
                                .is_none_or(|orig| *orig < kv.key)
                        });
                        if adopt {
                            let kv = tail.expect("adopted tail segment");
                            if let Some(p) = pending.take() {
                                p.flush(batch, segment_size);
                            }
                            pending = Some(Pending {
                                datoms: decode_segment(&kv.key, &kv.value)?,
                                orig_key: Some(kv.key),
                            });
                        } else if pending.is_none() {
                            pending = Some(Pending {
                                orig_key: None,
                                datoms: Vec::new(),
                            });
                        }
                    }
                }
            }
            pending.as_mut().expect("pending segment").insert(key);
        }
        if let Some(p) = pending.take() {
            p.flush(batch, segment_size);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slate::{in_memory_slate, DEFAULT_WRITE_OPTIONS};

    fn key(index: u8, suffix: &[u8]) -> Vec<u8> {
        let mut k = vec![index];
        k.extend_from_slice(suffix);
        k
    }

    async fn all_keys(db: &slatedb::Db, prefix: &[u8]) -> Vec<Vec<u8>> {
        let mut cursor = KeyCursor::scan_prefix(db, prefix).await.unwrap();
        let mut out = Vec::new();
        while let Some(k) = cursor.next().await.unwrap() {
            out.push(k.to_vec());
        }
        out
    }

    async fn slate_entries(db: &slatedb::Db, index: u8) -> usize {
        let mut iter = db
            .scan_with_options(vec![index]..vec![index + 1], &DEFAULT_SCAN_OPTIONS)
            .await
            .unwrap();
        let mut n = 0;
        while iter.next().await.unwrap().is_some() {
            n += 1;
        }
        n
    }

    #[test]
    fn segment_roundtrip() {
        let keys = vec![Bytes::from("aa"), Bytes::from("b"), Bytes::from("ccc")];
        let value = Bytes::from(encode_segment(&keys));
        assert_eq!(decode_segment(&keys[2], &value).unwrap(), keys);
        assert_eq!(
            decode_segment(&keys[0], &Bytes::new()).unwrap(),
            vec![keys[0].clone()]
        );
    }

    #[tokio::test]
    async fn segmented_writes_match_plain_reads_for_every_size() {
        // 300 keys in shuffled batches, written under several segment sizes, must read
        // back identically and honour prefix scans and seeks.
        let mut expected: Vec<Vec<u8>> = (0..300u32)
            .map(|i| key(1, &format!("{:03}", (i * 7919) % 300).into_bytes()))
            .collect();
        expected.sort();
        expected.dedup();

        for size in [1usize, 2, 4, 64] {
            let slate = in_memory_slate().await;
            let db = slate.db.clone();
            let keys: Vec<Vec<u8>> = (0..300u32)
                .map(|i| key(1, &format!("{:03}", (i * 7919) % 300).into_bytes()))
                .collect();
            for chunk in keys.chunks(37) {
                let mut batch = WriteBatch::new();
                write_segmented(db.as_ref(), &mut batch, chunk.to_vec(), size)
                    .await
                    .unwrap();
                db.write_with_options(batch, &DEFAULT_WRITE_OPTIONS)
                    .await
                    .unwrap();
            }
            assert_eq!(all_keys(&db, &[1]).await, expected, "size {size}");
            let entries = slate_entries(&db, 1).await;
            assert!(entries >= 300 / size, "size {size}: {entries} entries");
            if size > 1 {
                assert!(entries < 300, "size {size}: {entries} entries");
            }

            // Prefix in the middle of a segment.
            let sub: Vec<Vec<u8>> = expected
                .iter()
                .filter(|k| k.starts_with(&key(1, b"12")))
                .cloned()
                .collect();
            assert_eq!(all_keys(&db, &key(1, b"12")).await, sub, "size {size}");

            // Seeks: within the current segment, across segments, and past the end.
            let mut cursor = KeyCursor::scan_prefix(db.as_ref(), &[1]).await.unwrap();
            cursor.seek(&key(1, b"005")).await.unwrap();
            assert_eq!(cursor.next().await.unwrap().unwrap(), key(1, b"005"));
            cursor.seek(&key(1, b"0055")).await.unwrap();
            assert_eq!(cursor.next().await.unwrap().unwrap(), key(1, b"006"));
            cursor.seek(&key(1, b"250")).await.unwrap();
            assert_eq!(cursor.peek().unwrap(), &key(1, b"250"));
            cursor.seek(&key(1, b"299")).await.unwrap();
            assert_eq!(cursor.next().await.unwrap().unwrap(), key(1, b"299"));
            assert_eq!(cursor.next().await.unwrap(), None);
            cursor.seek(&key(1, b"300")).await.unwrap();
            assert_eq!(cursor.next().await.unwrap(), None);
        }
    }

    #[tokio::test]
    async fn plain_and_segmented_entries_mix_in_one_index() {
        let slate = in_memory_slate().await;
        let db = slate.db.clone();
        let first: Vec<Vec<u8>> = (0..20u32)
            .map(|i| key(2, &format!("{i:02}").into_bytes()))
            .collect();
        let mut batch = WriteBatch::new();
        write_segmented(db.as_ref(), &mut batch, first.clone(), 1)
            .await
            .unwrap();
        db.write_with_options(batch, &DEFAULT_WRITE_OPTIONS)
            .await
            .unwrap();
        let second: Vec<Vec<u8>> = (0..20u32)
            .map(|i| key(2, &format!("{i:02}x").into_bytes()))
            .collect();
        let mut batch = WriteBatch::new();
        write_segmented(db.as_ref(), &mut batch, second.clone(), 8)
            .await
            .unwrap();
        db.write_with_options(batch, &DEFAULT_WRITE_OPTIONS)
            .await
            .unwrap();

        let mut expected = first;
        expected.extend(second);
        expected.sort();
        assert_eq!(all_keys(&db, &[2]).await, expected);
    }

    mod equivalence {
        use edn::kw;
        use edn::Keyword;

        use crate::ops::{DataType, EntityRef, TxOp};
        use crate::segment::SegmentLayout;
        use crate::{Database, Node, QueryNode, SubmitNode, TransactionResult, TxKey};

        fn attr(ident: Keyword, value_type: &str, cardinality: &str, unique: bool) -> TxOp {
            let mut fields = vec![
                (kw!(:db/ident), DataType::Keyword(ident)),
                (
                    kw!(:db/valueType),
                    DataType::Keyword(Keyword::namespaced("db.type", value_type)),
                ),
                (
                    kw!(:db/cardinality),
                    DataType::Keyword(Keyword::namespaced("db.cardinality", cardinality)),
                ),
            ];
            if unique {
                fields.push((
                    kw!(:db/unique),
                    DataType::Keyword(Keyword::namespaced("db.unique", "identity")),
                ));
            }
            TxOp::put(fields)
        }

        async fn commit(node: &Node<crate::memory_log::MemoryLog>, ops: Vec<TxOp>) -> TxKey {
            match node.execute_tx(ops).await.unwrap() {
                TransactionResult::TxCommitted(key) => key,
                TransactionResult::TxAborted(_, err) => panic!("tx aborted: {err}"),
            }
        }

        fn by_id(id: i64) -> EntityRef {
            EntityRef::LookupRef(kw!(:g/id), DataType::Long(id))
        }

        const QUERIES: &[&str] = &[
            "[:find ?a ?b ?c :where [?a :g/to ?b] [?b :g/to ?c] [?a :g/to ?c]]",
            "[:find (count ?c) :where [?a :g/to ?b] [?b :g/to ?c]]",
            "[:find ?a (count ?b) :where [?a :g/to ?b]]",
            "[:find ?b (count ?a) :where [?a :g/to ?b]]",
            "[:find ?b :where [?a :g/id 7] [?a :g/to ?b]]",
            "[:find ?e :where [?e :g/weight ?w] [(> ?w 40)]]",
            "[:find (sum ?w) :where [?e :g/weight ?w]]",
            "[:find ?a ?b :where [?a :g/to ?b] [?b :g/weight ?w] [(> ?w 30)]]",
            "[:find ?e :where [?e :g/label \"label-13\"]]",
            "[:find ?e ?l ?w :where [?e :g/label ?l] [?e :g/weight ?w]]",
            "[:find ?id ?w :where [?e :g/id ?id] [?e :g/weight ?w] [?e :g/to ?o] [?o :g/id 3]]",
        ];

        /// Load a small graph with retractions and card-one updates, then answer every
        /// query at the current basis and at an earlier one.
        async fn run(segment_size: usize) -> (Vec<Vec<Vec<DataType>>>, u64) {
            let layout = if segment_size > 1 {
                SegmentLayout::RowSegments { segment_size }
            } else {
                SegmentLayout::Row
            };
            let node = Node::memory_node_with_layout(layout).await;
            commit(
                &node,
                vec![
                    attr(kw!(:g/id), "long", "one", true),
                    attr(kw!(:g/to), "ref", "many", false),
                    attr(kw!(:g/label), "string", "one", false),
                    attr(kw!(:g/weight), "long", "one", false),
                ],
            )
            .await;

            let n = 60i64;
            let vertices: Vec<TxOp> = (0..n)
                .map(|i| {
                    TxOp::put([
                        (kw!(:g/id), DataType::Long(i)),
                        (kw!(:g/label), DataType::String(format!("label-{i}"))),
                        (kw!(:g/weight), DataType::Long(i % 50)),
                    ])
                })
                .collect();
            for chunk in vertices.chunks(23) {
                commit(&node, chunk.to_vec()).await;
            }
            // Ref values need entity ids: look them up once.
            let db = node.db().await.unwrap();
            let rows = db
                .query("[:find ?id ?e :where [?e :g/id ?id]]")
                .await
                .unwrap();
            let mut eid = vec![0i64; n as usize];
            for row in rows {
                let [DataType::Long(id), DataType::Long(e)] = row.as_slice() else {
                    panic!("unexpected row {row:?}");
                };
                eid[*id as usize] = *e;
            }
            let edges: Vec<TxOp> = (0..n)
                .flat_map(|i| {
                    [1, 2, 3, 7]
                        .into_iter()
                        .map(move |k| (i, (i * k + 3) % n))
                        .filter(|(a, b)| a != b)
                })
                .map(|(a, b)| TxOp::Add {
                    entity: by_id(a),
                    attribute: kw!(:g/to),
                    value: DataType::Long(eid[b as usize]),
                })
                .collect();
            for chunk in edges.chunks(41) {
                commit(&node, chunk.to_vec()).await;
            }
            let before_changes = commit(
                &node,
                vec![TxOp::Add {
                    entity: by_id(5),
                    attribute: kw!(:g/to),
                    value: DataType::Long(eid[6]),
                }],
            )
            .await;

            // Retract some edges, bump weights (card-one auto retract), drop an entity.
            let mut changes = Vec::new();
            for i in (0..n).step_by(9) {
                changes.push(TxOp::Retract {
                    entity: by_id(i),
                    attribute: kw!(:g/to),
                    value: DataType::Long(eid[((i * 7 + 3) % n) as usize]),
                });
                changes.push(TxOp::Add {
                    entity: by_id(i),
                    attribute: kw!(:g/weight),
                    value: DataType::Long(45 + i),
                });
            }
            changes.push(TxOp::RetractEntity(by_id(17)));
            commit(&node, changes).await;
            commit(
                &node,
                vec![TxOp::Add {
                    entity: by_id(2),
                    attribute: kw!(:g/to),
                    value: DataType::Long(eid[3]),
                }],
            )
            .await;

            let db = node.db().await.unwrap();
            let db_before = node.db_as_of(before_changes).await.unwrap();
            let mut results = Vec::new();
            for q in QUERIES {
                let mut rows = db.query(*q).await.unwrap();
                rows.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
                results.push(rows);
                let mut rows = db_before.query(*q).await.unwrap();
                rows.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
                results.push(rows);
            }
            let keys = node
                .index_storage()
                .await
                .unwrap()
                .iter()
                .map(|s| s.keys)
                .sum();
            node.close().await.unwrap();
            (results, keys)
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn query_results_are_identical_across_segment_sizes() {
            let (plain, plain_keys) = run(1).await;
            for (i, rows) in plain.iter().enumerate() {
                assert!(
                    !rows.is_empty(),
                    "query {} returned no rows (as_of={})",
                    QUERIES[i / 2],
                    i % 2 == 1
                );
            }
            for size in [3usize, 16, 256] {
                let (segmented, keys) = run(size).await;
                assert_eq!(segmented.len(), plain.len());
                for (i, (a, b)) in plain.iter().zip(&segmented).enumerate() {
                    assert_eq!(
                        a,
                        b,
                        "segment size {size}: query {} differs",
                        QUERIES[i / 2]
                    );
                }
                assert!(
                    keys < plain_keys,
                    "segment size {size} should use fewer keys: {keys} vs {plain_keys}"
                );
            }
        }
    }
}
