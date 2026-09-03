# Query experiments

**Report (PDF):**
<https://github.com/socksy/triplox/blob/experiments/query-optimizations/experiments/report/triplox-query-experiments.pdf>

This directory holds the raw measurements, the scripts that produced the statistics, and the
report source. The code is the branch itself.

## Code

Every technique from the report is in this tree, behind an environment toggle. With no toggle
set the behaviour is the pre-experiment one.

| toggle | effect | chapter |
|---|---|---|
| `TRIPLOX_BATCHED_JOIN=1` | vectorized join over columns instead of rows | 2 |
| `TRIPLOX_SEGMENT_LAYOUT=row-segments` + `TRIPLOX_SEGMENT_SIZE=N` | row-major segments: N whole datom keys per SlateDB key, in every index | 3 |
| `TRIPLOX_SEGMENT_LAYOUT=columnar` + `TRIPLOX_SEGMENT_SIZE=N` | AEV/AVE packed into struct-of-arrays segments; AE/AV served from the first column | 4 |
| `TRIPLOX_ADJ_MATRIX=1` | CSR adjacency matrices for ref attributes | 5 |
| `TRIPLOX_MATRIX_ALGEBRA=1` | count queries as sparse matrix algebra (needs `TRIPLOX_ADJ_MATRIX=1`) | 5.5 |
| `TRIPLOX_ZONE_MAPS=1` | comparison predicates pushed into scans as value bounds, plus temporal run skipping | 6 |

`TRIPLOX_SEGMENT_LAYOUT` takes `row` (the default), `row-segments` or `columnar`. The two
segmented layouts are alternatives, not composable. Zone maps summarise runs of raw datom
keys, so they are inactive under either segmented layout.

Log toggles: `TRIPLOX_ENGINE_LOG=1` prints which engine answered each query,
`TRIPLOX_MATRIX_ALGEBRA_LOG=1` prints the algebra shape.

Design notes per technique are in `docs/*-EXPERIMENT.md`, `COMBINED.md` (the four-technique
stack) and `EXPERIMENT-ALGEBRA.md`.

## What the report measured

Chapters 2, 4, 5 and 5.5, and the 7.x combinations, were measured on the branches they were
written on. Chapters 3 (row-segments) and 6 (zone maps) were measured on their own branches
too; that code was integrated into this tree afterwards. It is verified here to give
identical query results in every configuration, but it has not been re-timed on this branch.
The timings in the report belong to the source branches, not to this tree.

Chapter to experiment: 2 vectorized-join, 3 datom-segments, 4 columnar-storage, 5
sparse-matrix and sparse-algebra, 6 zone-maps, 7.1 combo-vec-seg, 7.2 combo-vec-col, 7.3
combo-full and combo-full-algebra.

## Benchmark

`cargo bench --bench datalog_bench` builds an in-memory node, loads a seeded G(2000, 0.01) graph
and times the queries. Environment: `VERTICES`, `EDGE_PROB`, `RUNS`, `BATCH_SIZE`,
`QUERY_FILTER`, `ASOF=1` (adds three as-of queries), `BENCH_OUT` (JSON with per-iteration
samples). The node is in-memory; nothing here touches an object store. The repo pins rustc 1.95.

## Measurements

`results/stats/<experiment>-<arm>-p<N>.json` are the raw runs: four passes, five iterations
each, arms interleaved within a pass, on an idle machine. `results/stats/stats-<experiment>.json`
aggregates them: medians, bootstrap 95% intervals (4000 resamples), ratio of medians with its
interval, two-sided Mann-Whitney p.

`scripts/stats_run.sh <repo> <experiment> 4 5 "off=" "on=TOGGLE=1"` reproduces a run.
`scripts/stats.py` aggregates; `scripts/merge_stats.py` writes `report/stats.json`.

## Report

`report/report.typ` reads `report/stats.json` and `report/results.json`; `report/compile.sh`
builds the PDF with Typst 0.14 and the Source fonts from nixpkgs.
