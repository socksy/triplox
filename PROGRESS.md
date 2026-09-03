# Combination experiment: combo-full

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
