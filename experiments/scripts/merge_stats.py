#!/usr/bin/env python3
"""Merge results/stats/stats-*.json, with raw samples attached, into report/stats.json."""
import json, glob, os
HERE = os.path.dirname(os.path.abspath(__file__))
R = os.path.join(HERE, "..", "results", "stats")
out = {}
for f in sorted(glob.glob(f"{R}/stats-*.json")):
    d = json.load(open(f)); exp = d["experiment"]
    for arm in d["arms"]:
        for pf in sorted(glob.glob(f"{R}/{exp}-{arm}-p*.json")):
            for q in json.load(open(pf))["queries"]:
                row = d["queries"].get(q["query"])
                if row and arm in row: row[arm].setdefault("samples", []).extend(q["samples_ms"])
    out[exp] = {"arms": d["arms"], "queries": d["queries"]}
json.dump(out, open(os.path.join(HERE, "..", "report", "stats.json"), "w"))
print("merged:", list(out))
