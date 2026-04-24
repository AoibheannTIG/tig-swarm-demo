# Active challenge: Satisfiability (3-CNF SAT)

Decide whether a Boolean formula in 3-CNF can be satisfied; if it can, find an assignment.

- **Algorithm file:** `src/satisfiability/algorithm/mod.rs`
- **Per-instance score:** `QUALITY_PRECISION` (1,000,000) when every clause is satisfied, otherwise `0`. SAT is the only challenge without an algorithmic baseline — the per-instance score is binary rather than relative.
- **Aggregate score:** per-track arithmetic mean of per-instance scores, then shifted geometric mean across tracks. Higher is better.
- **Tracks:** parameterised by `(n_vars, ratio)`. The phase-transition ratio for random 3-SAT is ≈ 4267 (i.e., `clauses ≈ 4.267 × variables`). Lower ratios are mostly satisfiable; higher ratios are mostly unsatisfiable.

## Types

```rust
pub struct Challenge {
    pub seed: [u8; 32],
    pub num_variables: usize,
    pub clauses: Vec<Vec<i32>>,   // 3-CNF, 1-indexed; positive = literal, negative = ¬literal
}
pub struct Solution {
    pub variables: Vec<bool>,     // length == num_variables
}
```

## Strategy tags (suggestion)

`local_search`, `walksat`, `unit_propagation`, `cdcl`, `metaheuristic`, `decomposition`, `data_structure`, `other`

## Tips

- Random walks (WalkSAT) and probSAT are remarkably strong on uniform 3-SAT near the threshold.
- Unit propagation + decision heuristics (DPLL/CDCL) dominate on industrial / structured instances; the generated benchmarks here are random 3-SAT, so local search is usually competitive.
- Save partial assignments (`save_solution`) eagerly — if you land on a satisfying assignment by accident at iteration 1000 and never beat it, the timeout still saves your win.
- The score is binary per-instance (1M or 0), so feasibility is everything. Aim for 100% satisfaction; partial-satisfaction heuristics that never close out clauses score 0.
