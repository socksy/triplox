# Deliverable branch: experiments/query-optimizations

## Goal
One branch holding all the query-optimization experiment code, integrated, building, and
behind environment toggles. Started from `exp/combo-full` (vectorized join, columnar
segments, adjacency matrices, matrix algebra) plus the `experiments/` directory; then
`exp/zone-maps` merged in, and `exp/datom-segments` integrated as a third segment layout.

## Status
- `experiments/patches` removed.
- `exp/zone-maps` merged. Conflicts in PROGRESS.md, benches/datalog_bench.rs, src/db_value.rs,
  src/node.rs, src/query/patterns/triple.rs, src/query/plan.rs, all resolved by keeping both
  sides. `DB` now carries `zone_maps` next to `layout` and `adjacency`; the planner's
  `with_value_bounds` is applied to the TriplePattern it already built for the adjacency
  fallback; the bench keeps QUERY_FILTER and the engine marker as well as ASOF and the zone
  map counters. `DB::zone_map` returns None unless the layout is `Row`: a zone map summarises
  runs of raw datom keys, and under either segmented layout the keys under an index prefix
  are segment keys.
- `exp/datom-segments` integrated as `SegmentLayout::RowSegments`. `TRIPLOX_SEGMENT_LAYOUT`
  now takes `row` (default), `row-segments` and `columnar`; `TRIPLOX_SEGMENT_SIZE` sizes the
  latter two. Its `src/segment.rs` became `src/row_segment.rs` (encode/decode_segment,
  KeyCursor, write_segmented) so it coexists with the columnar `src/segment.rs`; the
  layout enum, the Node/Indexer/DB/bootstrap layout plumbing and the write dispatch are
  shared. Readers went to `KeyCursor`, which is transparent to the row/row-segments shapes;
  columnar AEV/AVE reads still route to `SegmentIterator`. `write_index_entries_inner` now
  writes through a `KeySink` with one arm per layout. The CDC tx-eid filter from
  datom-segments is in `src/slate/cdc.rs`.

## Next
- Verification: cargo test per configuration, clippy, fmt, row-count equivalence across arms.
- experiments/README.md update.

---

# Combination experiment: combo-full

> **Follow-on experiment: matrix algebra (`TRIPLOX_MATRIX_ALGEBRA=1`).** See the
> "Matrix algebra" section at the end of this file. The original CSR/AdjacencyPattern
> experiment below is unchanged and still the default when only `TRIPLOX_ADJ_MATRIX=1`
> is set.

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
After every milestone (compiles, test passes, bench run, doc written): update the Status and Next sections below, then `git add -A && git commit -q -m "wip(sparse-matrix): <milestone>"`. Commit even if incomplete. This file is the hand-off; a fresh agent must be able to continue from it alone.


# Experiment 2: matrix algebra over the CSR matrices

## Goal
Evaluate aggregate-only chain and triangle queries as sparse matrix algebra instead of
enumerating join tuples. Second toggle `TRIPLOX_MATRIX_ALGEBRA=1`, which needs
`TRIPLOX_ADJ_MATRIX=1` to supply the matrices. Everything else falls back unchanged.

## Status
- New: `src/query/algebra.rs` (shape recognition + evaluation), `src/query/bitset.rs`
  (dense boolean rows and hand-written aarch64 NEON AND/OR/popcount kernels).
- Modified: `src/query.rs` (`execute_query` tries `algebra::try_execute` first, plus the
  two new module declarations), `src/query/adjacency.rs` (`AdjMatrix::bits`, lazily built
  dense form per orientation), `src/db_value.rs` (`matrix_algebra` flag on the DB value),
  `src/node.rs` (`matrix_algebra_enabled`, `db_with_modes`, `db_as_of_with_modes`).
- Shapes handled: k-hop chains `[?x0 :a ?x1] .. [?x(k-1) :a ?xk]` with `(count ?xi)` or
  `(count-distinct ?x0 | ?xk)`, optionally anchored by clauses that mention exactly one
  endpoint and nothing else; and triangles `[?a :a ?b] [?b :a ?c] [?a :a ?c]` with
  `(count ?v)` for any of the three variables.
- Semirings: integer (path counts, `1^T A^k 1`) for `count`; boolean for
  `count-distinct`, run as bitset OR/AND + popcount when every hop shares one attribute.
- NEON: `or_into`, `and_popcount`, `popcount` in `src/query/bitset.rs` use
  `vorrq_u64` / `vandq_u64` / `vcntq_u8` / `vaddvq_u8`, 128 bits per step, with a scalar
  fallback for other architectures. Dense rows are capped at 64 MB, above which the
  sorted-merge CSR path is used instead.
- Equivalence tests pass: `query::algebra::tests::algebra_path_matches_generic_join`
  (3 random graphs, retractions, latest and as-of bases, 17 algebra queries asserted
  handled + 14 fallback queries asserted declined, all compared off/adjacency/algebra) and
  `dense_and_sparse_kernels_agree`.
- New bench queries on this branch only: `three_hop_count_distinct`, `two_hop_from_42`,
  `triangle_count`. The existing 10 are unchanged.
- Log line: `TRIPLOX_MATRIX_ALGEBRA_LOG=1` prints the shape, hop count, whether the
  bitset kernels were used, and the answer.

- Statistics pass done: results/stats/stats-sparse-algebra.json (3 arms, 4×5). three_hop_count
  6771 ms -> 0.88 ms, three_hop_count_distinct 7310 -> 0.11 ms, triangle_count 600 -> 1.06 ms,
  two_hop_count 238 -> 0.59 ms. Row counts identical across arms.
- cargo test: 648 passed / 0 failed in all three configurations. Clippy and fmt clean.
- EXPERIMENT-ALGEBRA.md written.

## Next
- Done. Possible follow-ups: feed the matrices from the change feed; more shapes.

---

# Merged in: exp/zone-maps

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
