# Experiment: vectorized (batched) generic join

## Goal
Batch execution of the generic join: extend a batch of bindings at once (sort by prefix key, merged range scans / sorted seeks instead of one iterator per binding), columnar intermediates with structural sharing, predicates and aggregates over columns. Execution-layer only; planner and variable order unchanged. Toggle: env var, e.g. TRIPLOX_BATCHED_JOIN=1.

## Status

### Profile (re-derived)
macOS `sample` on the release bench binary, `QUERY_FILTER=triangles RUNS=25`, 20s at 1ms.
Query work runs on a **tokio worker thread**, not the thread named `triplox-incremental-query`
(that one is parked in `block_on`); pick the busy worker (13964 samples). Symbols come from
`atos -o target/release/deps/datalog_bench-* -l <load addr>`; `sample` itself prints `???`.

Self (leaf) time, top frames:

| % | frame |
|---|---|
| 33.6 | libsystem_malloc (malloc/free) |
| 19.6 | libsystem_platform (memcpy/memmove/memcmp) |
| 6.4 | `<Bytes as Ord>::cmp` |
| 6.4 | `btree::search::search_tree` |
| 6.1 | libsystem_kernel |
| 1.9 | `<Vec<T> as Clone>::clone` |
| 1.6 | memcmp stub |
| 1.4 | `bytes::shared_clone` |
| 1.3 | `bytes::release_shared` |
| 1.2 | `RawVecInner::finish_grow` |

Inclusive time, triplox frames:

| % | frame |
|---|---|
| 99.1 | `GenericJoinEngine::execute` |
| 91.8 | `TriplePattern::join` |
| 16.0 | `TemporalFilterIterator::seek` |
| 12.3 | `TemporalFilterIterator::next` |
| 10.1 | `BindingBag::extend_rows` |
| 3.5 | `drop_in_place<BindingBag>` |
| 3.0 | `BindingBag::project` (the reorder after propose) |
| 2.4 | `TriplePattern::count` (1.1 of it `estimate_key_count`) |

Reading: **~55% of the query is malloc + memcpy + Bytes compares, not storage access.**
Storage iteration (seek+next through slatedb) is ~28%. The allocation traffic comes from the
row-oriented representation:
- `candidate_extensions` allocates a `Vec<Bytes>` **per candidate value** (`vec![var_pair.slice(..)]`)
  and then deep-clones the whole `Vec<Vec<Bytes>>` group **per input row**.
- `BindingBag::extend_rows` allocates one `Vec<Bytes>` **per output row** and memcpies the prefix.
- `project`/`reorder` after every propose allocates another `Vec<Bytes>` per row.
- Grouping input rows by the bound value uses `BTreeMap<&Bytes, Vec<usize>>`: that is the
  6.4% `search_tree` + 6.4% `Bytes::cmp` + a `Vec<usize>` allocation per distinct key.
`count` is *not* hot for triangles (single-proposer stages skip counting).

So the columnar batch attacks the right thing: replace per-row `Vec<Bytes>` with one
`Vec<u32>` parent-row map plus one `Vec<Bytes>` value column per level, and replace the
BTreeMap grouping with a sort of row indices.

### Code
- src/query/vectorized/batch.rs only (Batch with Own/Parent columns and row maps, ColumnView),
  not yet wired into the crate.

## Next
1. Wire src/query/vectorized/mod.rs into the crate; write the batched engine for pure triple-pattern + predicate + aggregate queries first; fall back to the existing engine for not/or/rules.
2. Toggle + equivalence test on/off on a small graph.
3. cargo test -p triplox, clippy.
4. A/B bench; include the profile top-10 in EXPERIMENT.md.
5. EXPERIMENT.md, fmt, commit.

### Environment note
The machine ran out of disk during this session (0 bytes free for a while; ~1.5 GiB free after).
Other worktrees' `target/` dirs are 1-10 GiB each. Keep builds minimal.

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
