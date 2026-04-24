# Active challenge: Quadratic Knapsack (QKP)

Pick a subset of items maximising linear value plus pairwise interaction value, subject to a weight budget.

- **Algorithm file:** `src/knapsack/algorithm/mod.rs`
- **Per-instance score:** baseline-relative quality `(your_value − greedy_value) / greedy_value × QUALITY_PRECISION`, clamped to ±10 × QUALITY_PRECISION. The baseline is the value-density greedy in `compute_greedy_baseline` (also see `src/knapsack/baselines/tabu_search.rs` for a stronger reference).
- **Aggregate score:** per-track arithmetic mean of per-instance quality, then shifted geometric mean across tracks. Higher is better.
- **Tracks:** parameterised by `(n_items, budget)`. Smaller budgets force harder selection trade-offs.

## Types

```rust
pub struct Challenge {
    pub seed: [u8; 32],
    pub num_items: usize,
    pub weights: Vec<u32>,
    pub values: Vec<u32>,
    pub interaction_values: Vec<Vec<i32>>,  // symmetric, zero diagonal
    pub max_weight: u32,
}
pub struct Solution {
    pub items: Vec<usize>,                  // selected item indices
}
```

## Strategy tags (suggestion)

`greedy`, `local_search`, `tabu`, `simulated_annealing`, `branch_and_bound`, `dp`, `metaheuristic`, `data_structure`, `other`

## Tips

- The seed algorithm is tabu search starting from a value-density greedy. To beat it, attack the *interaction* terms — adding an item that pairs well with already-selected items can be net-positive even if its solo value-to-weight is poor.
- Simulated annealing with neighbour swap (drop one + add one) and short tabu tenures is a strong baseline.
- The score is normalised per-instance, so beating the greedy by a wider margin on hard instances pays off.
- For very large item counts, a sparse representation of `interaction_values` (only non-zero pairs) avoids quadratic memory and speeds up neighbour-evaluation.
