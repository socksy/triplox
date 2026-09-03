# Sparse adjacency matrices for ref attributes

RedisGraph and GraphBLAS treat a graph as a set of sparse boolean matrices, one per edge
type, and answer traversals with matrix row lookups and set intersections instead of index
scans. This experiment does the same thing to triplox: for each `:db.type/ref` attribute,
build a CSR adjacency matrix once from a single AEV scan, cache it, and serve
`[?x :ref ?y]` triple patterns from it.

Everything is behind `TRIPLOX_ADJ_MATRIX=1`. With the toggle off nothing changes.

## What was built

**`src/query/adjacency.rs`** (286 lines) — the data structure.

- `Csr` (line 35): `keys` (sorted i64), `offsets`, `cols` — one sorted row of neighbours per
  key. `row`, `nnz`, `contains`, `has_key` are all binary search plus a slice.
- `AdjMatrix` (line 132) holds two of them: `out` (entity → values) and `inn` (value →
  entities), so both directions of a pattern are a row lookup rather than a scan.
- `AdjMatrix::build` (line 147) does one `TemporalFilterIterator` pass over the AEV prefix
  for the attribute, collects `(entity, value)` pairs, and sorts twice. The key encoding is
  `value ^ i64::MAX` big-endian, which does not sort like i64, so both orientations are
  re-sorted rather than reusing index order.
- `AdjacencyCache` (line 199): `HashMap<(attribute, tx_id), Arc<AdjMatrix>>` behind a mutex,
  hung off the node and shared across `DB` values. Keying on `tx_id` is what makes
  time-travel safe — an as-of query at an older basis simply builds (and caches) its own
  matrix.
- `intersect_sorted` (line 234) — the primitive the triangle query lives on.

**`src/query/patterns/adjacency.rs`** (439 lines) — `AdjacencyPattern`, an `ExecPattern`
with the same contract as `TriplePattern`. `count` reports exact `nnz`, `candidate_sets`
hands the engine a `&[i64]` row to intersect, `join` proposes a row or validates membership
with `contains`/`has_key`. `AdjacencyPattern::new` returns `None` for shapes it will not
serve (repeated variables, non-entity terms), so the planner falls back.

## Hook points

| where | what |
|---|---|
| `src/node.rs:49` | `adjacency_matrix_enabled()` reads `TRIPLOX_ADJ_MATRIX` |
| `src/node.rs:268`, `src/node.rs:357` | attach the cache and the ref-attribute set to each `DB` value |
| `src/db_value.rs:95` | `DB::adjacency(attribute)` — `None` unless the toggle is on and the attribute is a ref |
| `src/query/plan.rs:379` | materialization substitutes `AdjacencyPattern` for a ref-attribute triple pattern, falling back to `TriplePattern` |
|  `src/query/exec_pattern.rs:62` | `candidate_sets` added to the `ExecPattern` trait, defaulting to `None` |
| `src/query/engine.rs:54` | `execute_intersecting_stage` — intersects the candidate sets of the proposers that can supply one, validates with the rest |

`TRIPLOX_ADJ_MATRIX_LOG=1` prints nnz, both row counts, bytes and build time per matrix.

## Results

2000 vertices, 39764 edges, `edge_prob=0.01`. Three off runs and three on runs, interleaved
off/on/off/on/off/on, five iterations per query per run. Each cell is the median (or min) of
the three run medians (mins). Other benchmark agents were running on the same machine, so
the absolute numbers are noisy — the interleaving is what keeps the comparison fair.

| query | rows | off median ms | on median ms | median speedup | off min ms | on min ms | min speedup |
|---|---|---|---|---|---|---|---|
| `triangles` | 7821 | 797.48 | 24.21 | **32.95x** | 611.78 | 20.27 | 30.18x |
| `two_hop_count` | 1 | 323.83 | 206.50 | **1.57x** | 252.08 | 181.99 | 1.39x |
| `three_hop_count` | 1 | 14747.70 | 9157.03 | **1.61x** | 8660.62 | 3330.64 | 2.60x |
| `out_degree` | 2000 | 52.27 | 6.71 | **7.79x** | 47.90 | 6.33 | 7.56x |
| `in_degree_top` | 2000 | 51.99 | 6.90 | **7.53x** | 48.37 | 6.59 | 7.34x |
| `neighbors_of_42` | 21 | 1.54 | 0.02 | **64.12x** | 1.38 | 0.02 | 68.85x |
| `weight_filter` | 198 | 4.43 | 3.96 | **1.12x** | 3.85 | 3.67 | 1.05x |
| `weight_sum` | 1 | 4.28 | 3.91 | **1.10x** | 3.81 | 3.77 | 1.01x |
| `heavy_neighbors` | 1908 | 81.22 | 28.64 | **2.84x** | 74.00 | 27.20 | 2.72x |
| `label_lookup` | 1 | 0.05 | 0.03 | **1.57x** | 0.01 | 0.02 | 0.94x |
Row counts are identical off, on, and against the pre-experiment baseline for all ten queries.

The matrix itself: `nnz=39764`, 2000 rows in each orientation, **1451992 bytes** (1.45 MB,
about 36 bytes per edge across both directions), built in **38-40 ms**, once per
(attribute, tx_id).

Where the wins come from:

- `triangles` (33x) is the shape this technique exists for. The closing edge becomes an
  intersection of two sorted rows instead of an index probe per candidate.
- `neighbors_of_42` (64x) and the degree queries (7.5x) are pure row lookups. 6.7 ms for
  2000 rows is roughly the cost of touching the data at all.
- `two_hop_count` and `three_hop_count` gain much less because their cost is dominated by
  materialising 789k and 15.7M binding rows, not by reading edges. The matrix removes the
  IO, and what is left is the row machinery.
- `weight_filter`, `weight_sum` and `label_lookup` touch no ref attribute and are unchanged,
  which is the control.

## What went wrong

The first A/B run showed `heavy_neighbors` — `[?a :g/to ?b] [?b :g/weight ?w] [(> ?w 950)]`
— **9.6x slower** with the matrix on: 69 ms off, 668 ms on. Stage-level instrumentation
found the cause, and it was not the matrix.

The stage that adds `?b` has two proposers, the `:g/to` pattern and the `:g/weight` pattern.
The engine asks each for a count and shards rows to the cheapest. But
`estimate_key_count_with_prefix` returns **0 for every prefix** on this dataset — the whole
graph is still in the memtable and WAL, and there are no SST statistics to estimate from. So
with the toggle off both proposers answer 0, the tie goes to the first, and the `:g/to`
pattern proposes ~20 rows per `?a`. Correct, by luck.

Turn the matrix on and `AdjacencyPattern` answers with the *true* nnz, 7 to 37. It loses to
the estimator's 0. The `:g/weight` pattern wins every row, proposes all 2000 entities for
each of 2000 rows, and the engine materialises 4,000,000 rows before validating them back
down to 39764. An exact counter loses to an estimator that lies low.

This is a pre-existing cost-model bug that the matrix merely exposed; any pattern that
reports honest counts would hit it. Two changes fixed it without touching the estimator:

1. `execute_intersecting_stage` used to bail unless *every* proposer could supply a
   candidate set. It now intersects the sets from the proposers that can and demotes the
   rest to validators. Any stage with at least one adjacency proposer therefore bypasses the
   broken counts entirely. `heavy_neighbors` went 668 ms → 42 ms.
2. That alone pessimised `neighbors_of_42` (0.03 ms → 0.53 ms): with `?b` unbound the
   adjacency "candidate set" is the entire key list, which demoted the selective
   `[?a :g/id 42]` lookup to a validator. `candidate_sets` now returns `None` when the other
   side of the pattern is unbound, since an all-keys set is no more selective than anything
   another proposer could offer.

`heavy_neighbors` ends at 2.8x faster and `neighbors_of_42` back at 64x.

One more thing that does not work: a repeated variable in a single pattern, `[?a :g/to ?a]`.
`AdjacencyPattern::new` rejects it — but so does the existing engine, which does not support
that shape at all, so nothing was lost.

## Correctness

- `query::patterns::adjacency::tests::adjacency_path_matches_iterator_path` runs 11 queries
  on a 40-vertex graph (including a retraction) with the toggle off and on and asserts the
  result sets are equal.
- `cargo test -p triplox` passes with the toggle off **and** with `TRIPLOX_ADJ_MATRIX=1`:
  610 lib + 32 integration tests, 0 failures. Running the whole suite under the toggle is
  itself a second equivalence check.
- `cargo clippy -p triplox --all-targets` is clean.
- Row counts match the baseline for every benchmark query.

## Verdict

Worth taking, for graph-shaped workloads, with real conditions attached.

The gains are large and land exactly where the theory says they should: intersection-heavy
patterns and point lookups on ref attributes. The cost — 36 bytes per edge and a 40 ms build
per (attribute, basis) — is small at this size and the fallback path is total: any shape the
matrix does not handle goes back to `TriplePattern`.

What stands between this and production:

- **Memory is unbounded.** The cache never evicts and holds a matrix per (attribute, tx_id).
  A busy node with many bases would accumulate them. It needs an eviction policy, and a size
  cap above which an attribute is not materialised at all.
- **The build is eager and synchronous.** The first query touching a ref attribute at a new
  basis pays the full scan (40 ms here, linear in edges). Real use wants an incremental
  update from the transaction log rather than a rebuild per basis — the CDC machinery
  already streams the datoms that would be needed.
- **The cost model needs fixing regardless.** `estimate_key_count_with_prefix` returning 0
  for memtable-resident data means the generic join is currently choosing proposers by
  pattern order, not by cost, on any dataset that has not been compacted. The engine changes
  here route around it for adjacency patterns; they do not fix it.
- **`candidate_sets` on the trait is a good shape to keep** even if the matrices are
  dropped. Letting a pattern hand the engine a sorted set to intersect, rather than only a
  count and a proposal, is what made the triangle query fast, and other patterns could
  supply one.

Integration cost is moderate: about 725 new lines in two self-contained modules, plus small
hooks in five existing files. The engine change is the only one that alters shared behaviour,
and it is a strict improvement — more proposers get intersected than before.
