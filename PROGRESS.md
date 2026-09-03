# Combination experiment: combo-full

## Goal
Merge these finished experiment branches into this worktree, in this order: exp/vectorized-join exp/columnar-storage exp/sparse-matrix.
Resolve conflicts so that BOTH features work and both toggles remain independent. Then measure the stack against baseline and attribute the gain: bench with all toggles off, each toggle on alone, and all on together, interleaved. The question is whether the gains compose, cancel, or interact (e.g. does batched execution remove the columnar three_hop regression; does the adjacency cache still win once scans are cheap).

Each source branch has an EXPERIMENT.md (design, toggle env var names, hook points, known regressions) and an equivalence test. Read them before merging. Keep their EXPERIMENT.md/PROGRESS.md content by moving each to docs/<branch>-EXPERIMENT.md so the merge conflict on those two files is trivial; this worktree's own PROGRESS.md (this file) and COMBINED.md are the live documents.

## Status
- All three branches merged; `cargo check --all-targets` clean.
  - `exp/vectorized-join`: conflict PROGRESS.md only.
  - `exp/columnar-storage`: conflicts PROGRESS.md + src/query/patterns/triple.rs (import block only; took both).
  - `exp/sparse-matrix`: conflicts PROGRESS.md, src/db_value.rs (DB gains both `layout` and `adjacency` fields), src/node.rs (Node gains both `layout` and `adjacency_cache`; `db_as_of`/`db_with_adjacency` now apply `.with_layout` then `attach_adjacency`).
  - Each source EXPERIMENT.md/PROGRESS.md moved to docs/<branch>-*.
- Known interactions identified by reading, still to be confirmed by running:
  1. `AdjacencyPattern` has no `BatchPattern`, so `BatchedJoinEngine::supports` rejects any query with a ref triple pattern -> silent fallback to the row engine when ADJ+BATCHED are both on.
  2. `AdjMatrix::build` builds a raw `TemporalFilterIterator` over AEV row keys, bypassing `SegmentIterator`. Under `TRIPLOX_SEGMENT_LAYOUT=columnar` there are no AEV row keys, so the matrix would be built from segment headers. Must be fixed.
  3. Columnar composes with the batched join for free: `TriplePattern::as_batch` reaches storage through the same `create_iterator`/`estimate_count` helpers columnar patches.

## Next
1. Fix (2): route `AdjMatrix::build` through the columnar segment iterator.
2. Add a `TRIPLOX_ENGINE_LOG` trace at the `execute_query` fork so the bench reports which engine ran per query.
3. Run source equivalence tests + full suite in each configuration.
4. Interleaved bench, then COMBINED.md.

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
