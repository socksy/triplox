# Experiment: matrix algebra over the CSR adjacency matrices

Toggle: `TRIPLOX_MATRIX_ALGEBRA=1`, which needs `TRIPLOX_ADJ_MATRIX=1` for the matrices.
The original CSR/AdjacencyPattern path (EXPERIMENT.md) is unchanged.

## What was built

- `src/query/algebra.rs`: shape recognition and evaluation. `try_execute` runs before the
  generic join in `execute_query` and returns `Ok(None)` for every shape it does not handle.
- `src/query/bitset.rs`: dense boolean rows and aarch64 NEON kernels for OR, AND and
  population count (`vorrq_u64`, `vandq_u64`, `vcntq_u8`, `vaddvq_u8`), with a scalar
  fallback on other architectures.
- `AdjMatrix::bits`: a lazily built dense bit matrix per orientation, capped at 64 MB.
- Node/DB plumbing: `matrix_algebra` flag, `db_with_modes`, `db_as_of_with_modes`.
- Log line `TRIPLOX_MATRIX_ALGEBRA_LOG=1`: shape, hop count, bitset use, answer.

## Shapes handled

1. Chain: `[?x0 :a ?x1] ... [?x(k-1) :a ?xk]`, every edge a ref attribute with a matrix,
   inner variables used nowhere else, all chain variables distinct, one aggregate in the
   find clause: `count` over any chain variable, or `count-distinct` over an endpoint.
   Other clauses are allowed only if together they mention exactly one endpoint and nothing
   else; that endpoint's entity set comes from a sub-query through the normal engine and
   seeds the first vector.
2. Triangle: `[?a :a ?b] [?b :a ?c] [?a :a ?c]` with `count` over any of the three, no other
   clauses. Evaluated as the masked product A·(A ∘ Aᵀ) summed.

Declined: inputs, `:with`, ordering, limits, any other aggregate, any tuple-returning query,
repeated variables, chains with a branch.

## Semirings and how the semantics were checked

`count` over a chain of distinct variables counts rows of the join relation, and each row is
one distinct path, so the answer is 1ᵀ·A₁·…·A_k·1 over the integers. `count-distinct` over an
endpoint is the size of the reachable set, the same product over booleans. When every hop
uses one attribute the boolean product runs on the dense bit matrix with the NEON kernels.
The equivalence test `query::algebra::tests::algebra_path_matches_generic_join` builds three
random graphs with retractions and compares the algebra answer with the generic join at head
and at an as-of basis: 17 queries asserted handled and equal, 14 asserted declined.
`dense_and_sparse_kernels_agree` checks the bitset kernels against the CSR merge path.

## Results

Statistics pass on an idle machine: 4 passes × 5 iterations per arm, interleaved.
Ratio is median(arm) / median(off) with a bootstrap 95% interval. Source:
scratchpad `results/stats/stats-sparse-algebra.json`.

| query | off ms | matrices | algebra |
|---|---:|---:|---:|
| triangles | 574 | 0.034 [0.033, 0.035] | 0.034 [0.033, 0.035] |
| two_hop_count | 238 | 0.573 [0.562, 0.582] | 0.0025 [0.0024, 0.0025] (0.59 ms) |
| three_hop_count | 6771 | 0.878 [0.862, 0.910] | 0.00013 (0.88 ms) |
| out_degree | 47.6 | 0.140 | 0.136 |
| in_degree_top | 47.6 | 0.144 | 0.141 |
| neighbors_of_42 | 1.38 | 0.024 | 0.021 |
| weight_filter | 3.99 | 0.996 (no effect) | 0.970 |
| weight_sum | 3.91 | 0.979 | 0.991 (no effect) |
| heavy_neighbors | 70.9 | 0.384 | 0.381 |
| label_lookup | 0.02 | no effect | no effect |
| three_hop_count_distinct | 7310 | 0.885 | 0.000015 (0.11 ms) |
| two_hop_from_42 | 3.56 | 0.028 | 0.008 (0.03 ms) |
| triangle_count | 600 | 0.033 | 0.0018 (1.06 ms) |

Row counts are identical across the three arms for all 13 queries. Queries the algebra does
not handle are unchanged from the matrices arm.

## Tests

`cargo test -p triplox`: 616 lib + 23 + 3 + 6 integration = 648 passed, 0 failed, 1 ignored,
in each of three configurations: toggles off, `TRIPLOX_ADJ_MATRIX=1`, and both toggles on.
`cargo clippy -p triplox --all-targets`: no warnings. `cargo fmt --check`: clean.

## Not built

- Chains over mixed attributes with `count-distinct` use the sorted-merge CSR path, not the
  bit kernels.
- No shape beyond chains and the triangle count. No recursive rules (Triplox has none yet).
- No incremental maintenance of the matrices or the bit matrices; both are rebuilt per basis.

## Integration cost

The recogniser is a planner rule with strict preconditions and a fallback, so the risk is a
false positive that returns a fast wrong number. The equivalence test is the guard. The dense
bit matrix is n² bits per orientation (0.5 MB at 2000 vertices) and needs the same memory
bound and change-feed maintenance as the CSR cache.
