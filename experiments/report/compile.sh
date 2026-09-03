#!/bin/bash
# Builds the report with Typst 0.14 and the Source fonts from nixpkgs. Needs nix with flakes.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
args=()
for p in source-serif source-sans source-code-pro; do
  d=$(nix build "nixpkgs#$p" --no-link --print-out-paths)
  args+=(--font-path "$d/share/fonts/opentype")
done
cd "$HERE" && nix shell nixpkgs#typst -c typst compile "${args[@]}" report.typ triplox-query-experiments.pdf
