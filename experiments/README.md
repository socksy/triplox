# Query experiments

Everything behind the report `report/triplox-query-experiments.pdf`: the code as it was
measured, the raw measurements, the scripts that produced the statistics, and the report source.

## Code

This branch's tree is the full stack as benchmarked in chapter 7.3: vectorized join, columnar
segments, adjacency matrices and matrix algebra, all behind environment toggles, plus the fixes
their merge needed. It is `exp/combo-full` at 9c8840e.

| toggle | effect | chapter |
|---|---|---|
| `TRIPLOX_BATCHED_JOIN=1` | vectorized join | 2 |
| `TRIPLOX_SEGMENT_LAYOUT=columnar` | columnar segments in AEV/AVE | 4 |
| `TRIPLOX_ADJ_MATRIX=1` | CSR adjacency matrices for ref attributes | 5 |
| `TRIPLOX_MATRIX_ALGEBRA=1` | count queries as matrix algebra (needs `TRIPLOX_ADJ_MATRIX=1`) | 5.5 |
| `TRIPLOX_ENGINE_LOG=1`, `TRIPLOX_MATRIX_ALGEBRA_LOG=1` | print which engine or shape answered a query | |

The other experiments were measured on their own branches, which conflict with this tree.
They are included unchanged as patch series against the benchmark harness commit
(`bench/harness`, 7b3773a, itself `patches/00-bench-harness` against `main` 16019bb):

| patches | branch | measured in | toggle |
|---|---|---|---|
| `patches/exp-vectorized-join` | 8f6bdbe | chapter 2 | `TRIPLOX_BATCHED_JOIN=1` |
| `patches/exp-datom-segments` | 8496b52 | chapter 3 | `TRIPLOX_SEGMENT_SIZE=256` |
| `patches/exp-columnar-storage` | 4a17068 | chapter 4 | `TRIPLOX_SEGMENT_LAYOUT=columnar` |
| `patches/exp-sparse-matrix` | 5e02742 | chapter 5 | `TRIPLOX_ADJ_MATRIX=1`, `TRIPLOX_MATRIX_ALGEBRA=1` |
| `patches/exp-zone-maps` | 19ae876 | chapter 6 | `TRIPLOX_ZONE_MAPS=1`, as-of queries with `ASOF=1` |
| `patches/exp-combo-vec-seg` | a34f195 | chapter 7.1 | `TRIPLOX_BATCHED_JOIN=1`, `TRIPLOX_SEGMENT_SIZE=256` |
| `patches/exp-combo-vec-col` | 5dd1e59 | chapter 7.2 | `TRIPLOX_BATCHED_JOIN=1`, `TRIPLOX_SEGMENT_LAYOUT=columnar` |
| `patches/exp-combo-full` | 9c8840e | chapter 7.3 | this tree |

To rebuild one of them: `git checkout 7b3773a && git am experiments/patches/exp-zone-maps/*.patch`.
Each branch carries its own EXPERIMENT.md and PROGRESS.md; this tree has them under `docs/`,
`COMBINED.md` and `EXPERIMENT-ALGEBRA.md`.

## Benchmark

`cargo bench --bench datalog_bench` builds an in-memory node, loads a seeded G(2000, 0.01) graph
and times the queries. Environment: `VERTICES`, `EDGE_PROB`, `RUNS`, `BATCH_SIZE`, `BENCH_OUT`
(JSON with per-iteration samples). The node is in-memory; nothing here touches an object store.
The repo pins rustc 1.95.

## Measurements

`results/stats/<experiment>-<arm>-p<N>.json` are the raw runs: four passes, five iterations
each, arms interleaved within a pass, on an idle machine. `results/stats/stats-<experiment>.json`
aggregates them: medians, bootstrap 95% intervals (4000 resamples), ratio of medians with its
interval, two-sided Mann-Whitney p.

`scripts/stats_run.sh <repo> <experiment> 4 5 "off=" "on=TOGGLE=1"` reproduces a run.
`scripts/stats.py` aggregates; `scripts/merge_stats.py` writes `report/stats.json`.

Chapter to experiment: 2 vectorized-join, 3 datom-segments, 4 columnar-storage, 5 sparse-matrix
and sparse-algebra, 6 zone-maps, 7.1 combo-vec-seg, 7.2 combo-vec-col, 7.3 combo-full and
combo-full-algebra.

## Report

`report/report.typ` reads `report/stats.json` and `report/results.json`; `report/compile.sh`
builds the PDF with Typst 0.14 and the Source fonts from nixpkgs.
