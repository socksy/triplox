# Combination experiment: combo-full

## Goal
Merge these finished experiment branches into this worktree, in this order: exp/vectorized-join exp/columnar-storage exp/sparse-matrix.
Resolve conflicts so that BOTH features work and both toggles remain independent. Then measure the stack against baseline and attribute the gain: bench with all toggles off, each toggle on alone, and all on together, interleaved. The question is whether the gains compose, cancel, or interact (e.g. does batched execution remove the columnar three_hop regression; does the adjacency cache still win once scans are cheap).

Each source branch has an EXPERIMENT.md (design, toggle env var names, hook points, known regressions) and an equivalence test. Read them before merging. Keep their EXPERIMENT.md/PROGRESS.md content by moving each to docs/<branch>-EXPERIMENT.md so the merge conflict on those two files is trivial; this worktree's own PROGRESS.md (this file) and COMBINED.md are the live documents.

## Status
- All three merged, `cargo check --all-targets` clean. Conflicts: PROGRESS.md (all three), src/query/patterns/triple.rs (import block, columnar), src/db_value.rs + src/node.rs (DB/Node gain both `layout` and `adjacency*` fields).
- Fixed a real cross-experiment bug: `AdjMatrix::build` scanned AEV with a raw `TemporalFilterIterator`, which under columnar reads segment headers instead of datoms. It now picks `SegmentIterator` when the layout is columnar. Proven: reverting the fix makes `adjacency_path_matches_iterator_path` fail with wrong (not erroring) results under columnar+adj.
- Added `TRIPLOX_ENGINE_LOG=1` at the `execute_query` fork and a `--query <name>` stderr marker in the bench, so the engine actually chosen is observable per query.
- Test status (`cargo test -p triplox`):
  - off: 619 lib + 23 + 3 + 6, all green.
  - TRIPLOX_BATCHED_JOIN=1 alone: all green.
  - TRIPLOX_ADJ_MATRIX=1 alone: all green.
  - TRIPLOX_SEGMENT_LAYOUT=columnar alone: 608 passed / 11 failed.
  - all three on: same 608 / 11, no extra failures from combining.
  - The 11 are columnar's documented 10 plus `query::vectorized::tests::batched_engine_matches_the_row_engine`, which is the same harness artifact: the test writes raw AEV/AVE/AE/AV row keys with `slate.put` and then reads through the query engine, which in columnar mode reports "bad segment header". So the batched-vs-row equivalence test cannot run under the columnar layout; bench row counts are the substitute check there.

- `cargo clippy -p triplox --all-targets`: clean with toggles off and with all three on.

- Engine probe (`scratchpad/probe.sh`, VERTICES=500 RUNS=1, logs/probe-*.err) confirms the predicted interaction. With `TRIPLOX_ADJ_MATRIX=1` the batched engine is rejected for every query that touches the ref attribute `:g/to`: triangles, two_hop_count, three_hop_count, out_degree, in_degree_top, neighbors_of_42, heavy_neighbors all run `engine=row batched_toggle=true supported=false`. Only weight_filter, weight_sum and label_lookup stay batched, and those are exactly the queries batching helps least. Columnar does not change engine selection either way.

- Interleaved bench done (5 arms x 3 passes, 5 timed runs/query/pass, VERTICES=2000 EDGE_PROB=0.01, 39764 edges). JSON in scratchpad/results/combo-full-{off,batched,columnar,adj,all}.json. Row counts identical in all five arms and equal to the hand-off baseline.
- Headline: three_hop_count is 3.86x faster with batching alone but only 1.75x with everything on, because the adjacency pattern disables batching. triangles/out_degree/neighbors_of_42 are all-adjacency wins that columnar and batching add nothing to. weight_filter/weight_sum are the only queries where two toggles multiply (columnar 2.7x x batched -> 5.5x), and they are the only ref-free multi-stage queries left on the batched engine.

## Next
1. Sixth arm: `BatchPattern for AdjacencyPattern` so the matrices and the batched engine actually compose; re-measure.
2. COMBINED.md.
3. Interleaved bench (off / batched / columnar / adj / all, 3 passes) via scratchpad/bench.sh.
4. COMBINED.md.

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
