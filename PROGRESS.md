# Experiment: sparse adjacency matrices for ref attributes

## Goal
RedisGraph/GraphBLAS-style: per (ref attribute, basis) build CSR adjacency from one AEV scan, cache on the node, and serve `[?x :ref ?y]` triple patterns from it (row lookups for hops, intersections for triangle closing edge, row nnz for degree) instead of SlateDB iterators. Toggle: env var (check src/query/adjacency.rs for the name actually used). Time-travel queries may fall back.

## Status
- Compiles clean.
- New: src/query/adjacency.rs (AdjMatrix, Csr both orientations, cache keyed by (attr, tx_id), build timing), src/query/patterns/adjacency.rs (AdjacencyPattern implementing ExecPattern, same contract as TriplePattern).
- Modified: db_value.rs, node.rs, query.rs, query/engine.rs (candidate-set intersection stage), query/exec_pattern.rs, query/patterns/mod.rs, query/plan.rs (planner substitutes AdjacencyPattern for ref-attribute triple patterns when enabled).
- Toggle: env var `TRIPLOX_ADJ_MATRIX=1` (src/node.rs `adjacency_matrix_enabled`). `TRIPLOX_ADJ_MATRIX_LOG` prints per-matrix nnz/bytes/build_ms.
- Equivalence test `query::patterns::adjacency::tests::adjacency_path_matches_iterator_path` PASSES (11 queries, on vs off, 40-vertex graph incl. a retraction). The previously broken query was not the not-clause one but a repeated-variable self-loop pattern (`[?a :g/to ?a]`), which the engine does not support at all; it was replaced by a both-sides-constant validate-path query.

## Next
1. cargo test -p triplox (full), clippy.
2. A/B bench (3 off / 3 on, alternating), plus VERTICES=5000 EDGE_PROB=0.004 once if time allows. Record matrix build time and memory.
3. EXPERIMENT.md, fmt, commit.

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
