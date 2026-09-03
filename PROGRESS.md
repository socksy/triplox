# Combination experiment: combo-full

> **Follow-on experiment: matrix algebra (`TRIPLOX_MATRIX_ALGEBRA=1`).** See the
> "Matrix algebra" section at the end of this file. The original CSR/AdjacencyPattern
> experiment below is unchanged and still the default when only `TRIPLOX_ADJ_MATRIX=1`
> is set.

## Goal
Merge these finished experiment branches into this worktree, in this order: exp/vectorized-join exp/columnar-storage exp/sparse-matrix.
Resolve conflicts so that BOTH features work and both toggles remain independent. Then measure the stack against baseline and attribute the gain: bench with all toggles off, each toggle on alone, and all on together, interleaved. The question is whether the gains compose, cancel, or interact (e.g. does batched execution remove the columnar three_hop regression; does the adjacency cache still win once scans are cheap).

Each source branch has an EXPERIMENT.md (design, toggle env var names, hook points, known regressions) and an equivalence test. Read them before merging. Keep their EXPERIMENT.md/PROGRESS.md content by moving each to docs/<branch>-EXPERIMENT.md so the merge conflict on those two files is trivial; this worktree's own PROGRESS.md (this file) and COMBINED.md are the live documents.

## Status
DONE. All three branches merged on exp/combo-full, three composition fixes made, three interleaved
bench sweeps run, COMBINED.md written. See COMBINED.md for the full write-up.

Summary:
- Merge conflicts: PROGRESS.md (all three), src/query/patterns/triple.rs (imports only),
  src/db_value.rs and src/node.rs (DB/Node take both `layout` and `adjacency*`). Source branch
  docs moved to docs/<branch>-EXPERIMENT.md and docs/<branch>-PROGRESS.md.
- Three interactions found and fixed:
  1. `AdjMatrix::build` scanned AEV with a raw iterator; under columnar it read segment headers and
     built a wrong matrix, silently. Now picks `SegmentIterator` when the layout is columnar.
  2. `AdjacencyPattern` had no `BatchPattern`, so `TRIPLOX_ADJ_MATRIX=1` silently disabled the
     batched engine for all 7 ref-attribute queries (confirmed with `TRIPLOX_ENGINE_LOG=1`).
     Implemented `BatchPattern for AdjacencyPattern`.
  3. That alone regressed heavy_neighbors to 0.62x, because `execute_intersecting_stage` existed
     only in the row engine. Ported it to `BatchedJoinEngine` with a new
     `BatchPattern::candidate_sets_batch`.
- Bench: composed stack is the fastest arm on all ten queries; 3.4x-47x on the eight that carry
  signal; row counts identical everywhere and equal to the baseline.
  JSON: scratchpad/results/combo-full-{off,batched,columnar,adj,all,adj-composed,all-composed}.json
- Tests: green in every configuration except any that includes columnar, where 11 fail (columnar's
  documented 10 plus `query::vectorized::tests::batched_engine_matches_the_row_engine`, the same
  raw-row-key harness artifact). Clippy clean off and all-on. `cargo fmt` applied.

## Next
Nothing outstanding for this experiment. Follow-ups are listed at the end of COMBINED.md.

## Shared context

Toolchain: every shell needs `export PATH="$HOME/.rustup/toolchains/1.95.0-aarch64-apple-darwin/bin:$PATH"`, `export CARGO_BUILD_JOBS=4`, and to save disk `export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_BENCH_DEBUG=0`. Never run `cargo clean`. Work only in this worktree. Two other combination agents build on this machine concurrently; timings are noisy; the disk has about 40GB free shared by all three, so if a build fails with ENOSPC wait and retry rather than deleting anything outside this worktree's target dir.

Benchmark: `cargo bench --bench datalog_bench` (env VERTICES, EDGE_PROB, RUNS, BENCH_OUT). Baseline JSON: /private/tmp/claude-501/-Users-ben-code-triplox/2cc95019-6318-4527-8430-7ad4b12972b9/scratchpad/results/baseline.json. Baseline medians (ms) on an idle machine: triangles 537 (7821 rows), two_hop_count 223, three_hop_count 7041, out_degree 44 (2000 rows), in_degree_top 44 (2000 rows), neighbors_of_42 1.1 (21 rows), weight_filter 3.7 (198 rows), weight_sum 3.6, heavy_neighbors 68 (1908 rows), label_lookup 0.02 (1 row). Per-experiment A/B JSONs are in /private/tmp/claude-501/-Users-ben-code-triplox/2cc95019-6318-4527-8430-7ad4b12972b9/scratchpad/results as <experiment>-off.json / -on.json.

Definition of done:
1. All source toggles work independently and together; row counts match baseline in every configuration; equivalence tests from every source branch pass.
2. `cargo test -p triplox` and `cargo clippy -p triplox --all-targets` pass with toggles off and with all on, or failures are listed honestly.
3. Interleaved bench with attribution, JSON written as above.
4. COMBINED.md at worktree root: what was merged, conflicts and how resolved, the attribution table, interactions, honest verdict.
5. `cargo fmt`; commit on this branch; do not push; no GitHub issues/PRs.

## Progress protocol

After every milestone (a merge compiles, a test passes, a bench pass finishes, a doc section is written): update the Status and Next sections above, then `git add -A && git commit -q -m "wip(combo-full): <milestone>"`. Commit even if incomplete. Run long cargo commands in the background and poll their output file, or split them, so no single tool call is silent for more than a few minutes. This file is the hand-off; a fresh agent must be able to continue from it alone.
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

- Statistics pass done: results/stats/stats-sparse-algebra.json (3 arms, 4×5). three_hop_count
  6771 ms -> 0.88 ms, three_hop_count_distinct 7310 -> 0.11 ms, triangle_count 600 -> 1.06 ms,
  two_hop_count 238 -> 0.59 ms. Row counts identical across arms.
- cargo test: 648 passed / 0 failed in all three configurations. Clippy and fmt clean.
- EXPERIMENT-ALGEBRA.md written.

## Next
- Done. Possible follow-ups: feed the matrices from the change feed; more shapes.
