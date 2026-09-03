# Experiment: vectorized (batched) generic join

## Goal
Batch execution of the generic join: extend a batch of bindings at once (sort by prefix key, merged range scans / sorted seeks instead of one iterator per binding), columnar intermediates with structural sharing, predicates and aggregates over columns. Execution-layer only; planner and variable order unchanged. Toggle: env var, e.g. TRIPLOX_BATCHED_JOIN=1.

## Status (as of hand-off)
- Profiling done with macOS `sample` on the release bench binary, restricted to the query thread. The agent added a QUERY_FILTER env var to benches/datalog_bench.rs to profile single queries. The profile numbers themselves were not written down; re-derive the top frames quickly (sample the triangles query) and record them here and in EXPERIMENT.md.
- New: src/query/vectorized/batch.rs only (Batch with Own/Parent columns and row maps, ColumnView). Not yet referenced from lib.rs / query mod, so it is not compiled into the crate.
- Planned but not written: scan cache, batched engine, sinks (aggregate/project), pattern hooks in triple.rs / predicate.rs, the toggle.
- Design notes from the agent: fast path for predicates uses eval_binary_op semantics; A/B test compares Node::db() results on/off.

## Next
1. Wire src/query/vectorized/mod.rs into the crate; write the batched engine for pure triple-pattern + predicate + aggregate queries first; fall back to the existing engine for not/or/rules.
2. Toggle + equivalence test on/off on a small graph.
3. cargo test -p triplox, clippy.
4. A/B bench; include the profile top-10 in EXPERIMENT.md.
5. EXPERIMENT.md, fmt, commit.

## Shared context

Toolchain: every shell needs `export PATH="$HOME/.rustup/toolchains/1.95.0-aarch64-apple-darwin/bin:$PATH"` and `export CARGO_BUILD_JOBS=3`. Never run `cargo clean`. Work only in this worktree.

Benchmark: `cargo bench --bench datalog_bench` (env VERTICES, EDGE_PROB, RUNS, BENCH_OUT). Baseline JSON: /private/tmp/claude-501/-Users-ben-code-triplox/2cc95019-6318-4527-8430-7ad4b12972b9/scratchpad/results/baseline.json. Baseline medians (ms): triangles 537 (7821 rows), two_hop_count 223, three_hop_count 7041, out_degree 44 (2000 rows), in_degree_top 44 (2000 rows), neighbors_of_42 1.1 (21 rows), weight_filter 3.7 (198 rows), weight_sum 3.6, heavy_neighbors 68 (1908 rows), label_lookup 0.02 (1 row).

Definition of done:
1. Feature behind an env toggle; results identical on/off (row counts match baseline; an on/off equivalence test on a small graph).
2. `cargo test -p triplox` and `cargo clippy -p triplox --all-targets` pass, or failures are listed here honestly.
3. A/B interleaved bench, 3 runs each; JSON written to /private/tmp/claude-501/-Users-ben-code-triplox/2cc95019-6318-4527-8430-7ad4b12972b9/scratchpad/results/vectorized-join-off.json and /private/tmp/claude-501/-Users-ben-code-triplox/2cc95019-6318-4527-8430-7ad4b12972b9/scratchpad/results/vectorized-join-on.json.
4. EXPERIMENT.md at worktree root: what was built, hook points (file:line), A/B table, what failed, honest verdict and integration cost.
5. `cargo fmt`; commit on this branch; do not push; no GitHub issues/PRs.

## Progress protocol

After every milestone (compiles, test passes, bench run, doc written): update the Status and Next sections below, then `git add -A && git commit -q -m "wip(vectorized-join): <milestone>"`. Commit even if incomplete. This file is the hand-off; a fresh agent must be able to continue from it alone.
