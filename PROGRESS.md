# Experiment: sparse adjacency matrices for ref attributes

> **Follow-on experiment: matrix algebra (`TRIPLOX_MATRIX_ALGEBRA=1`).** See the
> "Matrix algebra" section at the end of this file. The original CSR/AdjacencyPattern
> experiment below is unchanged and still the default when only `TRIPLOX_ADJ_MATRIX=1`
> is set.

## Goal
RedisGraph/GraphBLAS-style: per (ref attribute, basis) build CSR adjacency from one AEV scan, cache on the node, and serve `[?x :ref ?y]` triple patterns from it (row lookups for hops, intersections for triangle closing edge, row nnz for degree) instead of SlateDB iterators. Toggle: env var (check src/query/adjacency.rs for the name actually used). Time-travel queries may fall back.

## Status
- Compiles clean.
- New: src/query/adjacency.rs (AdjMatrix, Csr both orientations, cache keyed by (attr, tx_id), build timing), src/query/patterns/adjacency.rs (AdjacencyPattern implementing ExecPattern, same contract as TriplePattern).
- Modified: db_value.rs, node.rs, query.rs, query/engine.rs (candidate-set intersection stage), query/exec_pattern.rs, query/patterns/mod.rs, query/plan.rs (planner substitutes AdjacencyPattern for ref-attribute triple patterns when enabled).
- Toggle: env var `TRIPLOX_ADJ_MATRIX=1` (src/node.rs `adjacency_matrix_enabled`). `TRIPLOX_ADJ_MATRIX_LOG` prints per-matrix nnz/bytes/build_ms.
- Equivalence test `query::patterns::adjacency::tests::adjacency_path_matches_iterator_path` PASSES (11 queries, on vs off, 40-vertex graph incl. a retraction). The previously broken query was not the not-clause one but a repeated-variable self-loop pattern (`[?a :g/to ?a]`), which the engine does not support at all; it was replaced by a both-sides-constant validate-path query.

- `cargo test -p triplox` PASSES: 610 lib + 32 integration, 0 failed (both with the toggle off and with `TRIPLOX_ADJ_MATRIX=1`, i.e. the whole suite is a second equivalence check).
- `cargo clippy -p triplox --all-targets` PASSES with zero warnings.

- A/B bench DONE: interleaved off/on/off/on/off/on, RUNS=5 each, VERTICES=2000 EDGE_PROB=0.01. Aggregated to results/sparse-matrix-off.json and results/sparse-matrix-on.json in the scratchpad results dir. Row counts identical off/on/baseline for all 10 queries.
- FINAL medians after both fixes (median of 3 run-medians, ms): triangles 797->24 (33x), two_hop 324->207 (1.6x), three_hop 14748->9157 (1.6x, noisy), out_degree 52->6.7 (7.8x), in_degree_top 52->6.9 (7.5x), neighbors_of_42 1.54->0.02 (64x), weight_filter/weight_sum/label_lookup flat, heavy_neighbors 81->29 (2.8x). No regressions.
- (Pre-fix medians, kept for the record): triangles 575->20 (28.5x), two_hop_count 238->133 (1.8x), three_hop_count 7034->5589 (1.3x, very noisy), out_degree 47->6.7 (7.1x), in_degree_top 47->6.5 (7.2x), neighbors_of_42 1.40->0.03 (52x), weight_filter/weight_sum/label_lookup unchanged (no ref attr), heavy_neighbors 69->668 (9.6x SLOWER - regression to explain).
- Matrix: attr :g/to, nnz=39764, rows_out=2000, rows_in=2000, bytes=1451992 (1.45 MB, ~36 B/edge across both orientations), build_ms ~38-40 once per (attr, tx).
- ROOT CAUSE of the heavy_neighbors regression, measured with temporary stage instrumentation (since removed): in `[?a :g/to ?b] [?b :g/weight ?w] [(> ?w 950)]` the stage adding ?b has two proposers. `slatedb_estimates::RangeStats::estimate_key_count_with_prefix` returns **0** for every prefix on this dataset (all data still memtable/WAL-resident, no SST stats), so with the toggle off both proposers report count 0, the tie goes to the first proposer, and the `:g/to` pattern proposes ~20 rows per ?a. With the toggle on, AdjacencyPattern reports the TRUE nnz (7..37) and therefore LOSES to the estimator's 0; the `:g/weight` pattern proposes all 2000 entities per row = 4,000,000 rows, later validated down to 39764. An exact counter loses to an estimator that lies low. This is a pre-existing cost-model bug that the matrix exposes, not an adjacency-execution problem.
- SECOND FIX: `AdjacencyPattern::candidate_sets` now returns None when the other side of the pattern is unbound (every row's set would be the whole key list, no more selective than any other proposer). Without this, `neighbors_of_42` regressed 0.03ms -> 0.53ms because the adjacency pattern's all-keys "set" demoted the selective `[?a :g/id 42]` lookup to a validator.
- FIXED in `GenericJoinEngine::execute_intersecting_stage`: it now intersects the candidate sets of the proposers that CAN supply them and demotes the rest to validators (previously it bailed unless every proposer supplied a set). Any stage with at least one adjacency proposer now bypasses the broken cost model. heavy_neighbors ON went 668ms -> ~42ms (vs ~70-80ms OFF). Tests and clippy still clean off and on.
- NOTE: this machine's disk hit 100% mid-experiment (concurrent agents). Keep bench outputs tiny; do not start large new builds without checking `df -k /`.

## Next
- Done. EXPERIMENT.md written, final interleaved A/B in results/sparse-matrix-{off,on}.json.
- VERTICES=5000 EDGE_PROB=0.004 scaling run attempted and abandoned: three_hop_count reached 3.7 GB RSS with no end in sight (tens of millions of binding rows). Noted in EXPERIMENT.md. If anyone retries it, they need a bench harness that can skip three_hop_count.

## Shared context

Toolchain: every shell needs `export PATH="$HOME/.rustup/toolchains/1.95.0-aarch64-apple-darwin/bin:$PATH"` and `export CARGO_BUILD_JOBS=3`. Never run `cargo clean`. Work only in this worktree.

Benchmark: `cargo bench --bench datalog_bench` (env VERTICES, EDGE_PROB, RUNS, BENCH_OUT). Baseline JSON: /private/tmp/claude-501/-Users-ben-code-triplox/2cc95019-6318-4527-8430-7ad4b12972b9/scratchpad/results/baseline.json. Baseline medians (ms): triangles 537 (7821 rows), two_hop_count 223, three_hop_count 7041, out_degree 44 (2000 rows), in_degree_top 44 (2000 rows), neighbors_of_42 1.1 (21 rows), weight_filter 3.7 (198 rows), weight_sum 3.6, heavy_neighbors 68 (1908 rows), label_lookup 0.02 (1 row).

Definition of done:
1. Feature behind an env toggle; results identical on/off (row counts match baseline; an on/off equivalence test on a small graph).
2. `cargo test -p triplox` and `cargo clippy -p triplox --all-targets` pass, or failures are listed here honestly.
3. A/B interleaved bench, 3 runs each; JSON written to /private/tmp/claude-501/-Users-ben-code-triplox/2cc95019-6318-4527-8430-7ad4b12972b9/scratchpad/results/sparse-matrix-off.json and /private/tmp/claude-501/-Users-ben-code-triplox/2cc95019-6318-4527-8430-7ad4b12972b9/scratchpad/results/sparse-matrix-on.json.
4. EXPERIMENT.md at worktree root: what was built, hook points (file:line), A/B table, what failed, honest verdict and integration cost.
5. `cargo fmt`; commit on this branch; do not push; no GitHub issues/PRs.

## Progress protocol

After every milestone (compiles, test passes, bench run, doc written): update the Status and Next sections below, then `git add -A && git commit -q -m "wip(sparse-matrix): <milestone>"`. Commit even if incomplete. This file is the hand-off; a fresh agent must be able to continue from it alone.


# Experiment 2: matrix algebra over the CSR matrices

## Goal
Evaluate aggregate-only chain and triangle queries as sparse matrix algebra instead of
enumerating join tuples. Second toggle `TRIPLOX_MATRIX_ALGEBRA=1`, which needs
`TRIPLOX_ADJ_MATRIX=1` to supply the matrices. Everything else falls back unchanged.

## Status
- New: `src/query/algebra.rs` (shape recognition + evaluation), `src/query/bitset.rs`
  (dense boolean rows and hand-written aarch64 NEON AND/OR/popcount kernels).
- Modified: `src/query.rs` (`execute_query` tries `algebra::try_execute` first, plus the
  two new module declarations), `src/query/adjacency.rs` (`AdjMatrix::bits`, lazily built
  dense form per orientation), `src/db_value.rs` (`matrix_algebra` flag on the DB value),
  `src/node.rs` (`matrix_algebra_enabled`, `db_with_modes`, `db_as_of_with_modes`).
- Shapes handled: k-hop chains `[?x0 :a ?x1] .. [?x(k-1) :a ?xk]` with `(count ?xi)` or
  `(count-distinct ?x0 | ?xk)`, optionally anchored by clauses that mention exactly one
  endpoint and nothing else; and triangles `[?a :a ?b] [?b :a ?c] [?a :a ?c]` with
  `(count ?v)` for any of the three variables.
- Semirings: integer (path counts, `1^T A^k 1`) for `count`; boolean for
  `count-distinct`, run as bitset OR/AND + popcount when every hop shares one attribute.
- NEON: `or_into`, `and_popcount`, `popcount` in `src/query/bitset.rs` use
  `vorrq_u64` / `vandq_u64` / `vcntq_u8` / `vaddvq_u8`, 128 bits per step, with a scalar
  fallback for other architectures. Dense rows are capped at 64 MB, above which the
  sorted-merge CSR path is used instead.
- Equivalence tests pass: `query::algebra::tests::algebra_path_matches_generic_join`
  (3 random graphs, retractions, latest and as-of bases, 17 algebra queries asserted
  handled + 14 fallback queries asserted declined, all compared off/adjacency/algebra) and
  `dense_and_sparse_kernels_agree`.
- New bench queries on this branch only: `three_hop_count_distinct`, `two_hop_from_42`,
  `triangle_count`. The existing 10 are unchanged.
- Log line: `TRIPLOX_MATRIX_ALGEBRA_LOG=1` prints the shape, hop count, whether the
  bitset kernels were used, and the answer.

## Next
- Full `cargo test` in all three configurations, clippy, fmt.
- `stats_run.sh triplox-exp-sparse-matrix sparse-algebra 4 5 off= adj=... algebra=...`
- Write `EXPERIMENT-ALGEBRA.md`.
