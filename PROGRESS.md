# Experiment: zone maps for scan pruning and predicate pushdown

## Goal
Per-run (RUN_SIZE keys in key order) min/max of the non-prefix component (V for AEV, E for AVE) plus oldest/newest tx id, per (index, attribute), built in memory by scanning the prefix and cached per basis (sound for any basis <= build basis, see module doc). Planner pushes comparison predicates on a single-pattern variable into the scan as V bounds; iterators skip runs. Toggle: TRIPLOX_ZONE_MAPS=1. Also measure an as-of query at an early basis.

## Status (as of hand-off)
- Compiles clean (`cargo check -p triplox --lib --tests`, `cargo check --all-targets`).
- Plumbing is complete: planner pushdown (src/query/plan.rs), AEV per-entity seek pruning and
  AV sorted-scan seek/early-exit (src/query/patterns/triple.rs), temporal run skipping
  (src/iterator/temporal_filter_iterator.rs), cache on SlateComponents (src/slate/mod.rs).
- New tests in src/zone_map.rs: `results_match_with_zone_maps_on_and_off` (10 queries x
  {latest, as-of} x {cold cache, warm cache}) and `zone_map_prunes_and_counts_skips`
  (asserts seeks_checked > 0 and seeks_skipped > 0). NOT YET RUN - see disk note.

### MACHINE DISK IS FULL (blocking)
`/` fluctuates between ~145Mi and ~4Gi free; five agents share it. `cargo test` and
`cargo bench` builds fail with ENOSPC mid-link. I deleted only this worktree's
`target/release` (never `cargo clean`). Do not touch other worktrees' targets.
Strategy: wait for free space with a background monitor, then build in this order -
(1) `cargo test -p triplox --lib`, (2) `cargo clippy`, (3) `cargo bench` (release, biggest).

## Next
1. Verify pushdown and run-skipping work end to end (skip counters > 0 on weight_filter, rows still 198).
2. Equivalence test on/off including >= vs > boundaries and an as-of query.
3. cargo test -p triplox, clippy.
4. A/B bench; report skipped runs per query; add an as-of query to your own runs.
5. EXPERIMENT.md (include interaction with a future segment layout), fmt, commit.

## Shared context

Toolchain: every shell needs `export PATH="$HOME/.rustup/toolchains/1.95.0-aarch64-apple-darwin/bin:$PATH"` and `export CARGO_BUILD_JOBS=3`. Never run `cargo clean`. Work only in this worktree.

Benchmark: `cargo bench --bench datalog_bench` (env VERTICES, EDGE_PROB, RUNS, BENCH_OUT). Baseline JSON: /private/tmp/claude-501/-Users-ben-code-triplox/2cc95019-6318-4527-8430-7ad4b12972b9/scratchpad/results/baseline.json. Baseline medians (ms): triangles 537 (7821 rows), two_hop_count 223, three_hop_count 7041, out_degree 44 (2000 rows), in_degree_top 44 (2000 rows), neighbors_of_42 1.1 (21 rows), weight_filter 3.7 (198 rows), weight_sum 3.6, heavy_neighbors 68 (1908 rows), label_lookup 0.02 (1 row).

Definition of done:
1. Feature behind an env toggle; results identical on/off (row counts match baseline; an on/off equivalence test on a small graph).
2. `cargo test -p triplox` and `cargo clippy -p triplox --all-targets` pass, or failures are listed here honestly.
3. A/B interleaved bench, 3 runs each; JSON written to /private/tmp/claude-501/-Users-ben-code-triplox/2cc95019-6318-4527-8430-7ad4b12972b9/scratchpad/results/zone-maps-off.json and /private/tmp/claude-501/-Users-ben-code-triplox/2cc95019-6318-4527-8430-7ad4b12972b9/scratchpad/results/zone-maps-on.json.
4. EXPERIMENT.md at worktree root: what was built, hook points (file:line), A/B table, what failed, honest verdict and integration cost.
5. `cargo fmt`; commit on this branch; do not push; no GitHub issues/PRs.

## Progress protocol

After every milestone (compiles, test passes, bench run, doc written): update the Status and Next sections below, then `git add -A && git commit -q -m "wip(zone-maps): <milestone>"`. Commit even if incomplete. This file is the hand-off; a fresh agent must be able to continue from it alone.
