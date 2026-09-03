# Experiment: columnar (SoA) segment encoding for AEV/AVE

## Goal
Pack runs of sorted datoms for one attribute into one SlateDB key (keyed by the last datom key so forward seek lands on the right segment) with column-wise encoding: first/second component, op bitmap, tx column; FOR + bit-packing for integer columns, offsets+bytes otherwise. Column-only reads for AE/AV-style scans. Toggle: layout switch (see src/segment.rs and how bootstrap/node pass a layout). Ingest may be slow but must be correct. Temporal filtering must work via the tx column.

## Status (as of hand-off)
- Compiles (cargo check --all-targets clean).
- New: src/segment.rs (format, encode/decode, segmentation rules in module doc), src/iterator/segment_iterator.rs (ScanMode Pair/First, temporal version resolution per logical key).
- Modified: bootstrap.rs (init_db_with_layout), db_value.rs, indexer.rs (write path), iterator/mod.rs, lib.rs, node.rs, query/patterns/triple.rs, tx.rs (lookup_tx_completion takes a layout).
- Previous agent was mid-way through applying the node.rs / tx.rs / bootstrap.rs plumbing edits with a script when it stopped. The tree compiles, but verify the layout toggle actually reaches the indexer and the iterators (grep for the layout type) and that the columnar path is exercised at all, not just the row path.

## Next
1. Confirm the toggle plumbs end to end; run the bench once with columnar on and check row counts against baseline.
2. Equivalence test across layouts on a small graph (include an as-of query).
3. cargo test -p triplox, clippy.
4. A/B bench; report SlateDB key count and bytes per index before/after and ingest_ms.
5. EXPERIMENT.md, fmt, commit.

## Shared context

Toolchain: every shell needs `export PATH="$HOME/.rustup/toolchains/1.95.0-aarch64-apple-darwin/bin:$PATH"` and `export CARGO_BUILD_JOBS=3`. Never run `cargo clean`. Work only in this worktree.

Benchmark: `cargo bench --bench datalog_bench` (env VERTICES, EDGE_PROB, RUNS, BENCH_OUT). Baseline JSON: /private/tmp/claude-501/-Users-ben-code-triplox/2cc95019-6318-4527-8430-7ad4b12972b9/scratchpad/results/baseline.json. Baseline medians (ms): triangles 537 (7821 rows), two_hop_count 223, three_hop_count 7041, out_degree 44 (2000 rows), in_degree_top 44 (2000 rows), neighbors_of_42 1.1 (21 rows), weight_filter 3.7 (198 rows), weight_sum 3.6, heavy_neighbors 68 (1908 rows), label_lookup 0.02 (1 row).

Definition of done:
1. Feature behind an env toggle; results identical on/off (row counts match baseline; an on/off equivalence test on a small graph).
2. `cargo test -p triplox` and `cargo clippy -p triplox --all-targets` pass, or failures are listed here honestly.
3. A/B interleaved bench, 3 runs each; JSON written to /private/tmp/claude-501/-Users-ben-code-triplox/2cc95019-6318-4527-8430-7ad4b12972b9/scratchpad/results/columnar-storage-off.json and /private/tmp/claude-501/-Users-ben-code-triplox/2cc95019-6318-4527-8430-7ad4b12972b9/scratchpad/results/columnar-storage-on.json.
4. EXPERIMENT.md at worktree root: what was built, hook points (file:line), A/B table, what failed, honest verdict and integration cost.
5. `cargo fmt`; commit on this branch; do not push; no GitHub issues/PRs.

## Progress protocol

After every milestone (compiles, test passes, bench run, doc written): update the Status and Next sections below, then `git add -A && git commit -q -m "wip(columnar-storage): <milestone>"`. Commit even if incomplete. This file is the hand-off; a fresh agent must be able to continue from it alone.
