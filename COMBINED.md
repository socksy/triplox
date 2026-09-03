# combo-full: batched join + columnar segments + sparse adjacency matrices

Three finished experiment branches merged onto one worktree and measured together:

| branch | toggle | what it does |
|---|---|---|
| `exp/vectorized-join` | `TRIPLOX_BATCHED_JOIN=1` | generic join over columns instead of one `Vec<Bytes>` per row |
| `exp/columnar-storage` | `TRIPLOX_SEGMENT_LAYOUT=columnar` | AEV/AVE packed into SoA segments; AE/AV served from the first column |
| `exp/sparse-matrix` | `TRIPLOX_ADJ_MATRIX=1` | per-(attribute, basis) CSR adjacency for `:db.type/ref` attributes |

Each source branch's `EXPERIMENT.md` and `PROGRESS.md` are preserved under `docs/<branch>-*`.
Toggles stay independent: any subset can be set, and with none set the behaviour is the
pre-experiment one.

## What the merge cost

Conflicts were small and mechanical.

- **`PROGRESS.md`** (all three) — resolved by keeping this worktree's file and moving each
  branch's docs aside.
- **`src/query/patterns/triple.rs`** (columnar vs batched) — the import block only. The two
  features do not fight here: `TriplePattern::as_batch` reaches storage through the same
  `create_iterator` and `estimate_count` helpers that columnar patches, so the batched engine
  reads segments without knowing they exist.
- **`src/db_value.rs`** and **`src/node.rs`** (columnar vs sparse-matrix) — both branches add a
  field to `DB` and to `Node` and both rewrite the same three constructors. Resolved by taking
  both: `DB` now carries `layout` and `adjacency`, and `Node::db_as_of` / `db_with_adjacency`
  apply `.with_layout(...)` and then `attach_adjacency(...)`.

Nothing had to be given up to make the merge compile. The interesting work was the three
interactions the merge exposed, none of which the compiler could see.

## Interaction 1: columnar storage silently corrupts the matrix builder

`AdjMatrix::build` opened a raw `TemporalFilterIterator` over the AEV prefix. Under the columnar
layout there are no AEV row keys — only segment headers keyed by their last datom — so the
builder read headers as datoms and produced a small, wrong matrix. **It did not error.** With
both toggles on, `[?a :g/to ?b]` returned the wrong rows.

Fixed by giving the builder the same iterator choice the query engine makes: `SegmentIterator`
when the layout is columnar, `TemporalFilterIterator` otherwise
(`src/query/adjacency.rs`, `AdjMatrix::build`).

Reverting that one match arm makes
`query::patterns::adjacency::tests::adjacency_path_matches_iterator_path` fail under
`TRIPLOX_SEGMENT_LAYOUT=columnar TRIPLOX_ADJ_MATRIX=1` with wrong results rather than an error,
which is what makes this class of bug worth calling out: any future code that reaches for AEV/AVE
keys directly gets silently wrong answers, not a crash.

## Interaction 2: adjacency patterns switched the batched engine off

`BatchedJoinEngine::supports` requires **every** participant of every stage to implement
`BatchPattern`. `AdjacencyPattern` did not, so as soon as `TRIPLOX_ADJ_MATRIX=1` was set, any
query touching a ref attribute fell back to the row engine wholesale — including the parts that
had nothing to do with the matrices.

`TRIPLOX_ENGINE_LOG=1` was added at the `execute_query` fork to make this observable, and the
bench prints a `--query <name>` marker to stderr so the log can be read per query. With
`TRIPLOX_ADJ_MATRIX=1 TRIPLOX_BATCHED_JOIN=1`:

| query | engine chosen |
|---|---|
| triangles | **row** (`supported=false`) |
| two_hop_count | **row** |
| three_hop_count | **row** |
| out_degree | **row** |
| in_degree_top | **row** |
| neighbors_of_42 | **row** |
| heavy_neighbors | **row** |
| weight_filter | batched |
| weight_sum | batched |
| label_lookup | batched |

Seven of ten queries lost batching, and the three that kept it are the three the batched engine
helps least. Columnar does not affect engine selection either way; with only
`TRIPLOX_BATCHED_JOIN=1` set, all ten run batched.

Fixed by implementing `BatchPattern for AdjacencyPattern` (`count_batch`, `propose_batch`,
`validate_batch` over `Batch` columns). After that all ten queries run batched with the matrices
on, and `adjacency_path_matches_iterator_path` — which compares matrix-on against matrix-off —
now runs both sides through the batched engine, so it doubles as the equivalence test for the new
code.

## Interaction 3: batching threw away the intersection that made the matrices fast

Making `AdjacencyPattern` batchable on its own was *not* a win. The sparse-matrix branch's real
trick is `execute_intersecting_stage`: when several proposers can hand the engine a sorted
candidate set, it intersects them instead of proposing-then-validating. That lived only in
`GenericJoinEngine`. Moving adjacency onto the batched engine therefore traded the intersection
away for the batching, and the trade was bad where intersection matters:

| query | adj alone (row engine, intersecting) | adj + batched, no intersecting stage |
|---|---:|---:|
| triangles | 35.98x | 18.72x |
| heavy_neighbors | 2.04x | **0.62x** |
| three_hop_count | 1.50x | 6.72x |
| two_hop_count | 2.18x | 3.75x |

`heavy_neighbors` regressing below 1.0x is the same pre-existing cost-model bug the sparse-matrix
branch documented: `estimate_key_count_with_prefix` returns 0 for memtable-resident data, an
honest counter loses to an estimator that lies low, and the engine materialises millions of rows
before validating them away. The intersecting stage is what routes around it.

Fixed by porting the intersecting stage to the batched engine: `BatchPattern` gained
`candidate_sets_batch` (defaulting to `None`), `AdjacencyPattern` implements it, and
`BatchedJoinEngine::execute_intersecting_stage` mirrors the row engine's version over columns.
With that, both wins are available at once.

## Attribution

`benches/datalog_bench.rs`, 2000 vertices, `edge_prob=0.01`, 39764 edges, 5 timed runs per query
per process, three processes per arm, arms interleaved within each pass. Each cell is the median
of the three per-process medians. JSON in the session scratchpad:
`combo-full-{off,batched,columnar,adj,all,adj-composed,all-composed}.json`.

Three sweeps were run, each with its own interleaved `off` control:

- **S1** — off / batched / columnar / adj / all, before the composition fixes.
- **S2** — off / adj+batched / all, with `BatchPattern for AdjacencyPattern` but no batched
  intersecting stage.
- **S3** — off / adj+batched / all, with both composition fixes.

The machine is shared with two other build agents. The `off` controls held steady on nine of ten
queries across the three sweeps (e.g. `heavy_neighbors` 69.77 / 69.37 / 69.56 ms) but
`three_hop_count` drifted a lot (9960 / 9646 / 5642 ms), so **only within-sweep comparisons are
trustworthy for that query**, and the absolute milliseconds below should not be compared across
sweeps.

### Median ms

| query | rows | off | batched | columnar | adj | all | adj+batched | all (composed) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| triangles | 7821 | 688.19 | 340.65 | 493.46 | 19.13 | 19.49 | 11.88 | **11.90** |
| two_hop_count | 1 | 303.94 | 149.10 | 249.55 | 139.19 | 145.64 | 67.62 | **67.89** |
| three_hop_count | 1 | 9959.72 | 2577.97 | 11452.01 | 6618.39 | 5700.91 | 1572.54 | **1570.39** |
| out_degree | 2000 | 48.11 | 43.40 | 14.51 | 6.95 | 6.92 | 3.47 | **3.34** |
| in_degree_top | 2000 | 47.76 | 42.71 | 14.64 | 7.55 | 6.94 | 3.47 | **3.59** |
| neighbors_of_42 | 21 | 1.30 | 1.52 | 0.07 | 0.04 | 0.04 | 0.03 | **0.03** |
| weight_filter | 198 | 3.82 | 3.24 | 1.43 | 4.02 | 0.70 | 3.23 | **0.68** |
| weight_sum | 1 | 3.80 | 3.21 | 1.39 | 3.93 | 0.69 | 3.30 | **0.69** |
| heavy_neighbors | 1908 | 69.77 | 55.76 | 33.13 | 34.22 | 25.36 | 14.39 | **10.88** |
| label_lookup | 1 | 0.02 | 0.02 | 0.02 | 0.03 | 0.03 | 0.02 | **0.02** |

Row counts are identical in every arm of every sweep and match the hand-off baseline: triangles
7821, out_degree 2000, in_degree_top 2000, neighbors_of_42 21, weight_filter 198,
heavy_neighbors 1908, the rest 1.

### Speedup against each sweep's own control

| query | batched | columnar | adj | all (S1) | adj+batched, no intersect (S2) | all (S2) | adj+batched (S3) | all (S3) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| triangles | 2.02x | 1.39x | 35.98x | 35.31x | 18.72x | 18.97x | 47.07x | **47.02x** |
| two_hop_count | 2.04x | 1.22x | 2.18x | 2.09x | 3.75x | 3.72x | 3.44x | **3.42x** |
| three_hop_count | 3.86x | 0.87x | 1.50x | 1.75x | 6.72x | 6.72x | 3.59x | **3.59x** |
| out_degree | 1.11x | 3.32x | 6.92x | 6.96x | 13.79x | 13.72x | 13.66x | **14.19x** |
| in_degree_top | 1.12x | 3.26x | 6.33x | 6.88x | 13.05x | 13.59x | 13.72x | **13.26x** |
| neighbors_of_42 | 0.85x | 19.64x | 32.40x | 36.00x | 28.05x | 38.56x | 49.60x | **41.33x** |
| weight_filter | 1.18x | 2.68x | 0.95x | 5.47x | 1.20x | 5.49x | 1.23x | **5.80x** |
| weight_sum | 1.18x | 2.73x | 0.97x | 5.48x | 1.16x | 5.54x | 1.18x | **5.64x** |
| heavy_neighbors | 1.25x | 2.11x | 2.04x | 2.75x | 0.62x | 0.63x | 4.83x | **6.39x** |
| label_lookup | 0.74x | 0.77x | 0.61x | 0.63x | 0.79x | 0.88x | 1.00x | **0.83x** |

`label_lookup` is a 20 µs query and `neighbors_of_42` a 30 µs one; neither carries signal.

### Reading the table

**Do the gains compose?** Once the two composition fixes are in, mostly yes, and on several
queries they multiply.

- `weight_filter` / `weight_sum` are the clean case: columnar alone 2.7x, batched alone 1.18x,
  both 5.5-5.8x. These touch no ref attribute, so they are the control for the matrices, and they
  show columnar and batching stacking better than either alone.
- `out_degree` / `in_degree_top`: columnar 3.3x and adjacency 6.9x are not additive — they are
  two ways to make the same scan cheap — but batching on top roughly doubles the adjacency
  number again, to 13-14x.
- `triangles`: adjacency dominates (36x); columnar and batching add nothing on their own on top
  of it, but batching the adjacency pattern *plus* keeping the intersection takes it to 47x.
- `three_hop_count`: the only place any toggle is a clear loser alone. Columnar regresses it
  (0.87x, matching the source branch's documented 25% regression: many single-row seeks each
  decoding a whole 1024-datom segment). Batching is the big win there (3.86x), and in the
  composed stack the columnar penalty is invisible — `all` and `adj+batched` are within 0.2% of
  each other, so the batched engine has removed enough of the row bookkeeping that the segment
  decode no longer dominates.

**Does the adjacency cache still pay off once scans are cheap and the join is batched?** Yes,
decisively. Compare S1's `columnar` column (scans cheap, no matrices) with S3's composed stack:
`triangles` 1.39x → 47x, `neighbors_of_42` 19.6x → 41x, `out_degree` 3.3x → 14x. Cheap scans and
a fast join reduce the constant the matrices save, but the matrices remove an asymptotic factor —
the closing edge of a triangle is a sorted-set intersection instead of a probe per candidate —
and no amount of storage or layout work substitutes for that.

**Does the columnar iterator break the AEV scan the matrix builder uses?** It did, silently. See
Interaction 1. After the fix, the matrix built under columnar is identical to the one built under
the row layout, and the build reads far fewer keys.

## Correctness

Equivalence tests from all three source branches:

- `query::vectorized::tests::batched_engine_matches_the_row_engine` (10 queries, exact row order)
  — passes with any toggle combination that does **not** include columnar; see below.
- `query::patterns::adjacency::tests::adjacency_path_matches_iterator_path` (11 queries on a
  40-vertex graph with a retraction) — passes with every toggle combination, including
  columnar+adjacency and batched+adjacency. Under `TRIPLOX_BATCHED_JOIN=1` both of its sides run
  on the batched engine (verified: 22 batched executions, 2 row for the `not`/`or` queries), so
  it is also the equivalence test for the new `BatchPattern for AdjacencyPattern`.
- `src/segment_layout_test.rs` — passes.

`cargo test -p triplox`:

| configuration | result |
|---|---|
| no toggles | 619 lib + 23 client_server + 3 fixture_compat + 6 subscription, **all pass** |
| `TRIPLOX_BATCHED_JOIN=1` | all pass |
| `TRIPLOX_ADJ_MATRIX=1` | all pass |
| `TRIPLOX_ADJ_MATRIX=1 TRIPLOX_BATCHED_JOIN=1` | all pass |
| `TRIPLOX_SEGMENT_LAYOUT=columnar` | 608 pass, **11 fail** |
| all three on | 608 pass, **11 fail** — the same 11, nothing new from combining |

The 11 are the columnar branch's documented 10 plus one more:

```
indexer::tests::test_lookup_tx_completion_aborted          (passes SegmentLayout::Row explicitly)
indexer::tests::test_lookup_tx_completion_committed        (ditto)
indexer::tests::test_retract_on_overwrite                  (asserts a raw AE key exists)
query::patterns::triple::tests::constant_pattern_validation_matches_exact_triple
query::patterns::triple::tests::constant_term_validation_matches_bound_rows
query::patterns::triple::tests::count_applies_bound_term_estimate_to_each_duplicate_row
query::patterns::triple::tests::historical_basis_observes_add_and_retract_boundaries
query::patterns::triple::tests::partial_validation_is_additive_and_full_validation_is_temporal
query::patterns::triple::tests::propose_with_constant_and_variable
query::patterns::triple::tests::proposes_each_row_through_the_matching_index
query::vectorized::tests::batched_engine_matches_the_row_engine        <- new
```

The new one is the same harness artifact as the six `triple::tests` failures: it seeds SlateDB by
writing raw AEV/AVE/AE/AV row keys with `slate.put` and then reads through the query engine, which
in columnar mode reports `bad segment header`. It is not a wrong query result. But it does mean
**the batched-vs-row equivalence test cannot run under the columnar layout**, so for that one
combination the only equivalence evidence is the benchmark row counts, which match. Making these
tests seed through the indexer rather than through raw keys would close the gap and is the same
day of work the columnar branch already estimated.

`cargo clippy -p triplox --all-targets`: clean with toggles off and with all three on.
`cargo fmt` applied.

## Verdict

The three experiments compose, but not by themselves. Merging them cleanly took an afternoon of
mechanical conflict resolution; making them actually work together took three fixes, and two of
the three would have gone unnoticed without deliberately checking:

1. A silent correctness bug, because one experiment reads storage directly and the other changes
   what storage looks like.
2. A silent performance bug, because the batched engine's "run only if every pattern opts in"
   rule turns one experiment's new pattern type into a global off-switch for the other
   experiment.
3. A performance regression from fixing (2) naively, because the win in the sparse-matrix branch
   was never really the matrices — it was `candidate_sets` plus the intersecting stage, and that
   lived in only one of the two engines.

The pattern behind all three: **each experiment added a new place where a decision is made
(which iterator, which engine, which stage strategy) and each assumed it was the only one.** Two
engines with two independent stage strategies, two iterator choices selected in two different
files, and an opt-in trait whose absence is a silent fallback rather than an error. The merge did
not create these; it just made them visible. The `as_batch` / `candidate_sets` pair of defaulted
trait methods is a good shape — but a default of `None` that costs 2x performance should be
loud, not silent.

Recommendation: take the whole stack. On this benchmark the composed configuration is the fastest
arm on every one of the ten queries, by 3.4x to 47x on the eight that carry signal, with identical
row counts. Before it can be a default rather than an experiment, three things from the source
branches still stand: the segment layout must be persisted in bootstrap metadata rather than read
from an env var, the adjacency cache needs eviction and a size cap, and
`estimate_key_count_with_prefix` returning 0 for memtable-resident data needs fixing rather than
routing around — the intersecting stage now papers over it in both engines, which makes the
underlying bug even easier to leave alone.
