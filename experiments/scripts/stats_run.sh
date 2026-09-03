#!/bin/bash
# Interleaved A/B statistics run.
# usage: stats_run.sh <repo-dir> <experiment> <passes> <runs> arm1=ENV,ENV... arm2=... ('off=' for no env)
# Writes results/stats/<experiment>-<arm>-p<N>.json and results/stats/stats-<experiment>.json next to this script's parent.
set +u
export CARGO_PROFILE_BENCH_DEBUG=0 CARGO_PROFILE_RELEASE_DEBUG=0
HERE=$(cd "$(dirname "$0")" && pwd); R=$HERE/../results/stats; L=$HERE/../logs; mkdir -p "$R" "$L"
repo=$1; exp=$2; passes=$3; runs=$4; shift 4
cd "$repo" || exit 1
cargo bench --bench datalog_bench --no-run > "$L/build-$exp.log" 2>&1 || { echo "BUILD FAILED $exp"; exit 1; }
for p in $(seq 1 "$passes"); do
  for spec in "$@"; do
    name=${spec%%=*}; envs=${spec#*=}; envargs=()
    if [ -n "$envs" ]; then IFS=',' read -ra kv <<< "$envs"; for e in "${kv[@]}"; do envargs+=("$e"); done; fi
    env "${envargs[@]}" VERTICES=2000 EDGE_PROB=0.01 RUNS="$runs" BENCH_OUT="$R/$exp-$name-p$p.json" cargo bench --bench datalog_bench > "$L/$exp-$name-p$p.log" 2>&1
    echo "$exp $name pass $p rc=$? $(date +%H:%M:%S)"
  done
done
python3 "$HERE/stats.py" "$R" "$exp" $(for spec in "$@"; do echo -n "${spec%%=*} "; done)
