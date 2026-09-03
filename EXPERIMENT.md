# Zone maps for scan pruning and predicate pushdown

Branch: `exp/zone-maps`. Toggle: `TRIPLOX_ZONE_MAPS=1` (off by default).

## What was built

A zone map is an in-memory summary of one `(index, attribute)` key prefix. The keys under
that prefix are cut into runs of 256 consecutive keys in key order, and each run stores:

- the start key of the run,
- the byte min and max of one non-prefix key component — V for AEV, E for AVE,
- the oldest tx id in the run.

Two things use it.

**Predicate pushdown.** The planner collects comparison predicates (`<`, `<=`, `>`, `>=`)
against literals and attaches the resulting `ValueBounds` to the triple pattern whose value
variable they constrain. Two scan paths then use those bounds:

- Pattern with a *bound* entity (`[?a :g/to ?b] [?b :g/weight ?w] [(> ?w 950)]`). The scan
  seeks AEV once per distinct entity. Before each seek, the zone map is asked whether any run
  overlapping that entity's key range can hold a value in range. If not, the seek is skipped.
- Pattern with a *free* entity (`[?e :g/weight ?w] [(> ?w 900)]`). AVE is sorted by encoded
  value, so no map is needed: the scan seeks straight to the first value that can match and
  stops as soon as it passes the last one.

**Temporal run skipping.** `TemporalFilterIterator` already discards keys newer than the
query basis one at a time. With a zone map it can check the current run's oldest tx: if even
that is newer than the basis, every key in the run is invisible and the iterator seeks
directly to the next run's start.

Soundness of the cache: index keys are never deleted, and any key written after basis `S`
carries a tx id `> S`. So a map built at basis `S` is valid for every query at a basis `<= S`.
Cached per `(index, attribute)` and rebuilt when a query arrives at a newer basis.

## Hook points

| What | Where |
| --- | --- |
| Zone map, bounds, cache, counters | `src/zone_map.rs` (new, ~450 lines + tests) |
| Predicate collection and attachment | `src/query/plan.rs:860` (`pushdown_value_bounds`), applied at `src/query/plan.rs:388` |
| Bounds on the pattern executor | `src/query/patterns/triple.rs:51`, `:90` |
| AEV per-entity seek pruning | `src/query/patterns/triple.rs:219-238` |
| AVE sorted-scan seek and early exit | `src/query/patterns/triple.rs:259-281` |
| Zone map handed to the temporal iterator | `src/query/patterns/triple.rs:142-155` |
| Run skipping on basis | `src/iterator/temporal_filter_iterator.rs:145-152` |
| Cache owned by the DB handle | `src/slate/mod.rs:59`, `src/db_value.rs:115` |

## The bug this experiment found

`codec::encode_i64` is `(value ^ i64::MAX).to_be_bytes()`. That is strictly order
**reversing** over the whole `i64` range, not order preserving: `enc(901) < enc(900)`,
`enc(0) < enc(-1)`, `enc(i64::MAX) < enc(i64::MIN)`. So `Long`, `BigInt` and `Instant` sort
*descending* in index keys, while `Double`, `Float` and `String` (which use a different
scheme) sort ascending. The comment at `src/codec.rs:27` calls the encoding order preserving,
and the section header says "XOR sign bit", but the code XORs with `MAX`, not with `MIN`.

The handed-over `ValueBounds` compared raw encoded bytes as if all types sorted ascending, so
every integer bound was inverted — `> 900` pruned exactly the runs it should have kept. The
three unit tests that shipped with it encoded the same assumption and failed on first run.

Fixed by giving `ValueBounds` a per-type-tag direction (`src/zone_map.rs`, `Direction` and
`value_cmp`). Comparisons now happen in the type's own order; the sorted-scan path asks for
`seek_target` / `past_byte_end`, which pick the *upper* bound as the seek target under a
descending encoding. `long_encoding_is_order_reversing` pins the behaviour.

This is a property of triplox's codec, not of this experiment. Anything else that assumes
index keys give ascending numeric ranges — range predicates, min/max shortcuts, a future
segment layout with per-segment min/max — has the same trap waiting.

## Correctness

- `cargo test -p triplox`: 647 pass, 0 fail — run twice, once normally and once with
  `TRIPLOX_ZONE_MAPS=1` exported for the whole suite. Every existing query test therefore
  doubles as an equivalence check.
- `results_match_with_zone_maps_on_and_off` (`src/zone_map.rs`): 10 queries covering `>`,
  `>=`, `<`, `<=`, a two-sided window, an empty result, a negative bound, a bound entity, a
  string bound and an aggregate; each run at the latest basis and at an earlier basis, with a
  cold cache and a warm one. Results compared row for row against the same queries with the
  feature off.
- `zone_map_prunes_and_counts_skips`: asserts the map is actually consulted and actually
  prunes, so the equivalence test cannot pass by silently doing nothing.
- Row counts in the benchmark are identical with the feature on and off, and match the
  pre-experiment baseline, for all 13 queries.
- `cargo clippy --workspace --all-targets`: clean. `cargo fmt`: applied.

## A/B benchmark

2000 vertices, G(n,p) with p=0.01, 39764 edges. Three passes per configuration, three runs
per query per pass, interleaved off/on. Reported value is the median of the three pass
medians, in milliseconds. Raw data: `/private/tmp/claude-501/-Users-ben-code-triplox/2cc95019-6318-4527-8430-7ad4b12972b9/scratchpad/results/zone-maps-off.json` and `zone-maps-on.json` in the same directory (per-pass files under `passes/`).

| query | rows | off | on | change | v_skips / v_checks | t_skips | build ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| triangles | 7821 | 719.3 | 611.8 | -15% | 0 / 0 | 0 | 22.3 |
| two_hop_count | 1 | 302.3 | 274.4 | -9% | 0 / 0 | 0 | 0 |
| three_hop_count | 1 | 12980.8 | 9564.3 | -26% | 0 / 0 | 0 | 0 |
| out_degree | 2000 | 50.0 | 48.2 | -4% | 0 / 0 | 0 | 0 |
| in_degree_top | 2000 | 49.3 | 47.8 | -3% | 0 / 0 | 0 | 0 |
| neighbors_of_42 | 21 | 1.69 | 1.66 | -2% | 0 / 0 | 0 | 1.4 |
| **weight_filter** | 198 | 4.38 | 2.67 | **-39%** | 1230 / 2000 | 0 | 1.3 |
| weight_sum | 1 | 4.25 | 3.96 | -7% | 0 / 0 | 0 | 0 |
| heavy_neighbors | 1908 | 72.3 | 70.5 | -3% | 1230 / 2000 | 0 | 0 |
| label_lookup | 1 | 0.03 | 0.07 | +120% | 0 / 0 | 0 | 2.2 |
| **asof_out_degree** | 0 | 23.1 | 4.7 | **-80%** | 0 / 0 | 155 | 0 |
| **asof_two_hop_count** | 1 | 23.4 | 5.0 | **-79%** | 0 / 0 | 155 | 0 |
| **asof_mid_out_degree** | 996 | 35.3 | 26.9 | **-24%** | 0 / 0 | 77 | 0 |

The machine was shared with four other build-heavy agents throughout, so absolute times run
15-80% above the pre-experiment baseline and `three_hop_count` swung between 5.4s and 13.4s
across passes. Only the differences that line up with a non-zero counter should be believed.

### What is real

**As-of queries: 4-5x.** `asof_out_degree` and `asof_two_hop_count` query a basis taken
before the edges were written, so nearly every AEV key under `:g/to` is newer than the basis.
Without a zone map the iterator walks and discards all 39764 of them; with one it jumps 155
runs and touches almost nothing. `asof_mid_out_degree` picks a basis halfway through and skips
77 runs, for a smaller but still clear 24%. This is the strongest result in the experiment and
it needs no predicate at all.

**Value-bounded scan over a free entity: -39%.** `weight_filter` reads 198 of 2000 keys
instead of all 2000, because AVE is value-sorted and the scan can seek and stop.

### What is not

**The no-predicate, latest-basis rows.** `triangles`, `two_hop_count`, `three_hop_count`,
`out_degree`, `in_degree_top` and `weight_sum` all show 0 skips of both kinds. There is no
mechanism by which zone maps make them faster, so -3% to -26% is noise plus ordering bias: in
each pass the `on` run came second and inherited a warmer page cache.

**Per-entity seek pruning: measurable, not valuable.** `heavy_neighbors` prunes 1230 of 2000
seeks — 61% — and gains 3%, which is inside the noise. The seeks it skips were already cheap;
the scan is dominated by the 39764-edge join that feeds it. Pruning seeks is the part of this
design that sounded best on paper and delivered least.

**`label_lookup` is a real regression.** A point lookup that took 0.02ms now pays 2.2ms to
build a zone map it never uses.

## What failed, and the cost of fixing it

**Zone maps are built whether or not anything will use them.**
`src/query/patterns/triple.rs:143` asks for a map on *every* AEV/AVE/EAV/VAE scan, because
the temporal iterator might want one. At the latest basis no key is ever newer than the basis,
so `t_skips` is always 0 and the whole build is wasted: 22ms for `:g/to`, 2.2ms for
`:g/label`. On a 2000-vertex graph that is 3% of `triangles` and 100x of `label_lookup`.

The fix is to build lazily — hand the iterator something that builds on the first key it finds
newer than the basis, rather than an `Arc<ZoneMap>` built up front. Head-basis queries would
then never build at all, and the as-of wins would be untouched. Maybe half a day. Until that
is done the feature cannot be turned on by default.

**Pruning only works when values correlate with key order.** A zone map summarises 256
*consecutive keys*, so it prunes only if those keys have a narrow value range. The benchmark's
`:g/weight = i % 1000` correlates with entity id, which is why it prunes 61%. Values scattered
across their domain give every run the full range and prune nothing — `zone_map_prunes_and_counts_skips`
originally failed for exactly this reason, and had to be rewritten with correlated data.
Nothing in triplox orders datoms by value within an attribute, so whether this fires at all is
a property of the caller's data, not of the database.

**Rebuild on every new basis.** The cache is keyed by `(index, attribute)` and invalidated
whenever a query arrives at a newer basis than the map was built at. A workload that writes
between queries rebuilds by full prefix scan every time, at roughly 22ms per 40k keys. For a
read-mostly store that is fine; for anything write-heavy the map never pays for itself. Making
maps incremental — appending runs for new keys instead of rescanning — is the real fix and is
substantially more work than the feature itself.

**Memory is unbounded.** One entry per `(index, attribute)` ever queried, never evicted, each
holding start key and two component byte strings per 256 keys. Small for the benchmark;
unbounded for a schema with many attributes.

## Verdict

Half of this belongs in triplox and half does not.

The **temporal run skipping** is worth keeping. It is a 4-5x win on as-of queries, it needs no
planner support, and it addresses a genuine weakness — a query at an old basis currently pays
for every datom written since. Landing it means fixing the eager build first, then bounding
the cache. Call it a week including incremental rebuild, which write-heavy workloads need.

The **predicate pushdown** splits. The AVE sorted-scan seek (`weight_filter`, -39%) is the
good part and it does not need zone maps at all — it is 20 lines using the fact that AVE is
already sorted by value, and it should be landed on its own. The AEV per-entity seek pruning,
which is the part that actually requires zone maps, pruned 61% of seeks for a 3% gain; it is
not worth the machinery, and it only fires when the caller's values happen to correlate with
insertion order.

If a future segment layout stores per-segment min/max on disk, most of this comes back for
free and with none of the build cost — which is a better place to spend the effort than
making the in-memory version incremental.

The most valuable output of this experiment is not the feature. It is
`long_encoding_is_order_reversing`: triplox's integer keys sort backwards, the comment in
`codec.rs` says otherwise, and the next person to write a range scan will hit it too.
