# Experiment: columnar (SoA) segment encoding for AEV/AVE

## Goal
Pack runs of sorted datoms for one attribute into one SlateDB key (keyed by the last datom key so forward seek lands on the right segment) with column-wise encoding: first/second component, op bitmap, tx column; FOR + bit-packing for integer columns, offsets+bytes otherwise. Column-only reads for AE/AV-style scans. Toggle: layout switch (see src/segment.rs and how bootstrap/node pass a layout). Ingest may be slow but must be correct. Temporal filtering must work via the tx column.

## Status

- Compiles clean. Toggle plumbed end to end via `SegmentLayout::from_env()` (`TRIPLOX_SEGMENT_LAYOUT=columnar`, `TRIPLOX_SEGMENT_SIZE`): bootstrap.rs, indexer.rs, node.rs, db_value.rs all read it; the query engine picks `SegmentIterator` in src/query/patterns/triple.rs:148. Only `tx::lookup_tx_completion` reads AEV/AVE outside the query engine and it is layout-aware.
- src/segment_layout_test.rs: row-vs-columnar equivalence over 9 queries (current DB and as-of a pre-retraction tx) plus a per-index key/byte/datom-count storage report. **PASSES.**
- Fixed a real bug: `Segment::decode_first_column` left the second column with one offset, so any AE/AV `lower_bound`/`cmp_key` panicked (index out of bounds). Now n+1 zero offsets so second reads as empty.
- Equivalence-test storage report (30 vertices, ~87 edges, segment_size 64):
  row: 1165 keys, 36289 bytes total. columnar: 335 keys, 16421 bytes total (-55%). AEV datoms 247 == 247.

### Disk hazard (read this)
The machine disk hit 100% repeatedly; five agents build concurrently. Two things matter:
- Build with debug info off, which cuts this worktree's target from ~3.2G to ~1.6G:
  `export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_SPLIT_DEBUGINFO=off CARGO_PROFILE_BENCH_DEBUG=0`
- When bash tool calls fail with ENOSPC the command does not run at all. Check `df -h /System/Volumes/Data` first.
`target/debug` and `target/release` were deleted once each to make room (no `cargo clean`).

## Next
1. Full `cargo test -p triplox` + `cargo clippy -p triplox --all-targets`.
2. Interleaved A/B bench, 3 runs each, JSON to results/columnar-storage-{off,on}.json.
3. EXPERIMENT.md, fmt, commit.

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
