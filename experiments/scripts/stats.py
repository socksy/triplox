#!/usr/bin/env python3
"""Aggregate sampled A/B runs into per-query statistics.
Usage: stats.py <results_dir> <experiment> <arm1> <arm2> ...  (arm files: <exp>-<arm>-p<N>.json)
Writes <results_dir>/stats-<exp>.json with, per arm and query: n, mean, sd, median, ci95 (bootstrap), and for each
non-'off' arm vs 'off': ratio of medians, bootstrap 95% CI of the ratio, Mann-Whitney U two-sided p-value.
"""
import json, sys, glob, math, random, statistics as st
random.seed(7)

def mannwhitney(a, b):
    # exact-ish normal approximation with tie correction
    n1, n2 = len(a), len(b)
    allv = sorted([(v, 0) for v in a] + [(v, 1) for v in b])
    ranks = {}
    i = 0
    rank_sum_a = 0.0
    ties = []
    while i < len(allv):
        j = i
        while j < len(allv) and allv[j][0] == allv[i][0]:
            j += 1
        r = (i + 1 + j) / 2.0
        cnt = j - i
        if cnt > 1: ties.append(cnt)
        for k in range(i, j):
            if allv[k][1] == 0: rank_sum_a += r
        i = j
    u1 = rank_sum_a - n1 * (n1 + 1) / 2
    u = min(u1, n1 * n2 - u1)
    mu = n1 * n2 / 2
    n = n1 + n2
    tc = sum(t**3 - t for t in ties) / (n * (n - 1)) if n > 1 else 0
    sigma = math.sqrt(n1 * n2 / 12 * ((n + 1) - tc))
    if sigma == 0: return 1.0
    z = (u - mu + 0.5) / sigma
    p = math.erfc(-z / math.sqrt(2))  # 2*Phi(z), z <= 0
    return max(0.0, min(1.0, p))

def boot(fn, a, b=None, k=4000):
    out = []
    for _ in range(k):
        sa = [random.choice(a) for _ in a]
        if b is None: out.append(fn(sa))
        else: out.append(fn(sa, [random.choice(b) for _ in b]))
    out.sort()
    return out[int(0.025 * k)], out[int(0.975 * k)]

def main():
    d, exp, arms = sys.argv[1], sys.argv[2], sys.argv[3:]
    samples = {}
    for arm in arms:
        files = sorted(glob.glob(f"{d}/{exp}-{arm}-p*.json"))
        if not files: sys.exit(f"no files for {arm}")
        for f in files:
            for q in json.load(open(f))["queries"]:
                samples.setdefault(arm, {}).setdefault(q["query"], []).extend(q["samples_ms"])
    out = {"experiment": exp, "arms": arms, "queries": {}}
    for q in samples[arms[0]]:
        row = {}
        for arm in arms:
            xs = samples[arm].get(q)
            if not xs: continue
            lo, hi = boot(st.median, xs)
            row[arm] = {"n": len(xs), "mean": st.fmean(xs), "sd": st.pstdev(xs) if len(xs) > 1 else 0.0, "median": st.median(xs), "ci95_median": [lo, hi]}
        base = samples[arms[0]].get(q)
        for arm in arms[1:]:
            xs = samples[arm].get(q)
            if not xs or not base: continue
            r = st.median(xs) / st.median(base)
            lo, hi = boot(lambda a, b: st.median(b) / st.median(a), base, xs)
            row[arm]["ratio_vs_" + arms[0]] = r
            row[arm]["ratio_ci95"] = [lo, hi]
            row[arm]["p_mannwhitney"] = mannwhitney(base, xs)
        out["queries"][q] = row
    json.dump(out, open(f"{d}/stats-{exp}.json", "w"), indent=1)
    for q, row in out["queries"].items():
        parts = [f"{q:<20}"]
        for arm in arms[1:]:
            if arm in row and "ratio_vs_" + arms[0] in row[arm]:
                r = row[arm]; parts.append(f"{arm}: {r['ratio_vs_'+arms[0]]:.2f}x [{r['ratio_ci95'][0]:.2f},{r['ratio_ci95'][1]:.2f}] p={r['p_mannwhitney']:.3g}")
        print("  ".join(parts))
main()
