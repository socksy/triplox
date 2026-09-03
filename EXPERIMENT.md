# Columnar (SoA) segment encoding for AEV/AVE

## What was built

Runs of consecutive sorted datoms for one attribute are packed into a single SlateDB
entry instead of one empty-valued key per datom. A segment is keyed by its **last**
datom key, so an ordinary forward seek on the row key lands on the segment that
contains it. Inside the value the datoms are stored column-wise: first component,
second component, an op bitmap, and a tx column. Integer columns (entity ids, tx ids,
long values) are frame-of-reference encoded and bit-packed; anything else becomes an
offsets array plus a bytes blob.

Two indexes are segmented: AEV and AVE. The AE and AV indexes disappear entirely —
they were distinct-prefix indexes used to answer "which entities have attribute A"
and "which values does A take", and both are now served by decoding only the first
column of the corresponding segment (`Segment::decode_first_column`), which skips the
rest of the value. EAV and VAE keep the row layout.

Temporal queries work because every datom carries its tx in the tx column, so the
as-of / historical filters run over the decoded segment exactly as they ran over row keys.

Toggle: `TRIPLOX_SEGMENT_LAYOUT=columnar` (plus optional `TRIPLOX_SEGMENT_SIZE`,
default 1024). Unset means the original row layout, byte for byte.

## Hook points

| What | Where |
|---|---|
| Layout enum and env toggle | `src/segment.rs:52`, `src/segment.rs:59` |
| Which indexes get segmented | `src/segment.rs:89` (`is_segmented_index`) |
| FOR + bit-packed integer column | `src/segment.rs:96` (`encode_i64_column`) |
| Segment encode / decode / count | `src/segment.rs:317`, `src/segment.rs:359`, `src/segment.rs:355` |
| First-column-only decode (AE/AV path) | `src/segment.rs:375` |
| Write path: merge a batch into segments | `src/segment.rs:515` (`merge_into_segments`) |
| Indexer picks row vs columnar | `src/indexer.rs:78` |
| Query engine picks the iterator | `src/query/patterns/triple.rs:148` |
| AE/AV requests rewritten onto segments | `src/query/patterns/triple.rs:117-131` |
| Segment scan iterator | `src/iterator/segment_iterator.rs:30`, `seek` at `:255`, `next` at `:288` |
| Layout carried on the Db value | `src/db_value.rs:64`, `src/db_value.rs:95` |
| Bootstrap / node wiring | `src/bootstrap.rs:86`, `src/node.rs:112` |
| Tx-completion lookup (only AEV reader outside the query engine) | `src/tx.rs:620` |
| Equivalence + storage tests | `src/segment_layout_test.rs` |

## A/B benchmark

`benches/datalog_bench.rs`, 2000 vertices, edge_prob 0.01, 39764 edges. Three
processes per side, interleaved off/on/off/on/off/on, 5 timed runs per query per
process. The number below is the median across the three per-process medians.
JSON: `columnar-storage-off.json` / `columnar-storage-on.json` in the shared results dir.

| query | rows | off (ms) | on (ms) | on/off |
|---|---:|---:|---:|---:|
| triangles | 7821 | 652.9 | 517.5 | **0.79** |
| two_hop_count | 1 | 265.5 | 213.9 | **0.81** |
| three_hop_count | 1 | 10512.9 | 13113.9 | **1.25** |
| out_degree | 2000 | 48.6 | 15.5 | **0.32** |
| in_degree_top | 2000 | 48.4 | 16.5 | **0.34** |
| neighbors_of_42 | 21 | 1.71 | 0.10 | **0.06** |
| weight_filter | 198 | 4.08 | 1.64 | **0.40** |
| weight_sum | 1 | 4.02 | 1.69 | **0.42** |
| heavy_neighbors | 1908 | 72.6 | 37.6 | **0.52** |
| label_lookup | 1 | 0.04 | 0.03 | 0.73 |
| ingest | — | 153.4 | 143.4 | 0.93 |

Row counts are identical on both sides and match the recorded baseline.

### Storage

`storage_report_at_scale` (ignored by default), 2000 vertices, fanout 20,
segment_size 1024, counting every live SlateDB key and its bytes:

| | keys | key bytes | value bytes | total |
|---|---:|---:|---:|---:|
| row | 154913 | 5338684 | 13 | 5338697 |
| columnar | 48427 | 1748571 | 324615 | 2073186 |

61% smaller overall, and 69% fewer keys. AEV alone goes from 46173 keys / 1667278
bytes to 55 keys / 154941 bytes. The remaining bulk is EAV, which was left in the
row layout.

## What failed, and what is honest about the numbers

**three_hop_count regressed 25%.** It is also the noisiest query in the suite
(per-process medians: off 10513 / 10519 / 8065, on 14090 / 9089 / 13114), so the
interval is wide, but the direction is consistent enough that it should not be
explained away. The plausible cause is the join order: three_hop drives a deep
nested loop that re-seeks into AEV many times with a fresh bound value each time.
Every one of those seeks now decodes a whole 1024-datom segment to find one row,
where the row layout could seek straight to a single key. Small result sets that
sit behind many independent seeks pay for the segment; large scans amortise it.
The bench was also run on a machine with four other agents compiling, so the
absolute milliseconds are worse than the recorded baseline on both sides —
compare on/off, not against the baseline column.

**A real bug found and fixed:** `Segment::decode_first_column` left the second
column with a single offset, so any AE/AV `lower_bound` / `cmp_key` indexed past the
end and panicked. Fixed by giving it n+1 zero offsets, so the second column reads
as empty and key comparison sees only the first column.

**Ten unit tests fail under `TRIPLOX_SEGMENT_LAYOUT=columnar`.** All ten are
test-harness artifacts, not wrong query results:

- six `query::patterns::triple::tests::*` write raw row keys straight into SlateDB
  and then read through the query engine, which in columnar mode finds a row key
  where a segment header should be;
- `count_applies_bound_term_estimate_to_each_duplicate_row` asserts an exact
  key-count estimate, and the columnar estimator deliberately multiplies by
  segment_size (6144 vs 6);
- `indexer::tests::test_retract_on_overwrite` asserts a raw AE key exists, and
  columnar intentionally writes no AE;
- two `indexer::tests::test_lookup_tx_completion_*` pass `SegmentLayout::Row`
  explicitly while the indexer under test reads the layout from the env.

With the toggle off, `cargo test -p triplox` is fully green (613 lib + 23
client_server + 3 fixture_compat + 6 subscription) and
`cargo clippy -p triplox --all-targets` is clean.

**Not attempted:** EAV segmentation, compaction/rewrite of segments after many small
transactions, and any cross-segment prefetch. Ingest here is one merge per batch;
under a workload of many tiny transactions the read-modify-write of the tail segment
would dominate, and this bench (batches of 1000) does not exercise that.

## Verdict

Worth pursuing. Storage drops 61% and every scan-shaped query gets faster, some by
an order of magnitude (`neighbors_of_42` 17x, `out_degree` 3x). The regression is
confined to deep pointer-chasing joins that do many single-row seeks, and it is
addressable — either by keeping a small per-segment first-column index so a seek can
skip decoding, or by choosing segment_size per attribute from its cardinality.

Integration cost is moderate. The write path change is contained
(`merge_into_segments` behind one match in the indexer), and the read path change is
one iterator swap in `triple.rs`. The awkward part is that the layout is a property
of the *database*, not of a call site: it has to be threaded through `Db`, the
indexer, bootstrap, and `tx::lookup_tx_completion`, and any future code that reads
AEV/AVE keys directly has to go through the iterator or it will see segment headers.
Before merging, the layout should be persisted in the bootstrap metadata rather than
read from an env var, so an existing database cannot be opened with the wrong reader.
Making the ten env-sensitive unit tests layout-parameterised is a day of work, not a
redesign.
