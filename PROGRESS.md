# Experiment: zone maps for scan pruning and predicate pushdown

## Goal
Per-run (RUN_SIZE keys in key order) min/max of the non-prefix component (V for AEV, E for AVE) plus oldest/newest tx id, per (index, attribute), built in memory by scanning the prefix and cached per basis (sound for any basis <= build basis, see module doc). Planner pushes comparison predicates on a single-pattern variable into the scan as V bounds; iterators skip runs. Toggle: TRIPLOX_ZONE_MAPS=1. Also measure an as-of query at an early basis.

## Status
- BUG FOUND AND FIXED: `codec::encode_i64` is `value ^ i64::MAX`, which is strictly order
  *reversing*, so Long/BigInt/Instant sort DESCENDING in index keys (floats and strings sort
  ascending). The handed-over `ValueBounds` compared raw bytes assuming ascending order, so
  every integer bound was inverted. `ValueBounds` now carries a per-type-tag `Direction` and
  compares in value order; the AV sorted-scan path uses `seek_target`/`past_byte_end`, which
  pick the upper bound as the seek target for descending encodings.
- `cargo test -p triplox --lib zone_map`: 8/8 pass, including
  `results_match_with_zone_maps_on_and_off` (10 queries x {latest, as-of} x {cold, warm cache})
  and `zone_map_prunes_and_counts_skips`.
- `cargo test -p triplox`: 647 pass, 0 fail, both with the toggle off and with
  TRIPLOX_ZONE_MAPS=1 forced on for the whole suite. `cargo clippy --workspace --all-targets`:
  clean. `cargo fmt` applied.
- KEY LIMITATION found: run pruning only fires when values correlate with key order. With
  values scattered across the whole domain every run spans the whole range and nothing prunes.
  The bench's `:g/weight = i % 1000` does correlate with entity id, so it prunes.

### MACHINE DISK
`/` swings between ~145Mi and ~32Gi free; five agents share it. Builds fail with ENOSPC
mid-link when it is low; just retry. I deleted only this worktree's `target/release`.
`target/tmp/retry_tests.sh <log> <cmd...>` waits for headroom and retries on ENOSPC.

## Next
Experiment complete. EXPERIMENT.md at the worktree root has the full write-up. Nothing is
blocked. If someone picks this up:
1. Route a value-bounded pattern through AVE as a range scan in the planner. The single
   pattern `[?e :g/weight ?w] [(> ?w 900)]` is currently executed as 2000 per-entity seeks,
   so the sorted-scan code at src/query/patterns/triple.rs:259-281 never runs.
2. Build the temporal zone map lazily (on the first key newer than the basis) so head-basis
   queries stop paying for a map that can never skip anything. This is what makes
   label_lookup 100x slower today.

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
