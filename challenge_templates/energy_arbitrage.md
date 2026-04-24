# Active challenge: Energy Market Arbitrage

Operate a battery fleet across a transmission network with day-ahead and real-time prices to maximise profit, subject to state-of-charge bounds, transmission congestion, and degradation costs.

- **Algorithm file:** `src/energy_arbitrage/algorithm/mod.rs`
- **Per-instance score:** baseline-relative quality `(your_profit − baseline_profit) / baseline_profit × QUALITY_PRECISION`, clamped to ±10 × QUALITY_PRECISION. The baseline is the better of the greedy buy-low-sell-high and the conservative do-nothing policies (`compute_baseline`; sources at `src/energy_arbitrage/baselines/{greedy,conservative}.rs`).
- **Aggregate score:** per-track arithmetic mean of per-instance quality, then shifted geometric mean across tracks. Higher is better.
- **Tracks:** parameterised by `scenario ∈ {BASELINE, CONGESTED, MULTIDAY, DENSE, CAPSTONE}`, scaling network size, line limits, volatility, and battery heterogeneity.

## Types

```rust
pub struct Challenge {
    pub seed: [u8; 32],
    pub num_steps: usize,
    pub num_batteries: usize,
    pub network: Network,
    pub batteries: Vec<Battery>,
    pub exogenous_injections: Vec<Vec<f64>>,
    pub market: Market,
}
pub struct Solution {
    pub schedule: Vec<Vec<f64>>,  // per-step charge (negative) / discharge (positive) MWh per battery
}
```

This challenge is **policy-based**: the upstream API exposes `Challenge::grid_optimize(policy)` which steps through time with a commitment chain and asks your `policy(challenge, state)` for an action vector each step. The algorithm/mod.rs wraps this into the standard `solve_challenge(...)` surface so the swarm dispatch is uniform.

## Strategy tags (suggestion)

`greedy`, `dynamic_programming`, `model_predictive_control`, `dual_decomposition`, `convex_relaxation`, `lookahead`, `metaheuristic`, `other`

## Tips

- The seed is greedy buy-low/sell-high. To beat it, use the price chain commitments to do model-predictive control — solve a short-horizon LP each step rather than acting myopically.
- Network congestion (line PTDF limits) frequently makes the greedy infeasible; project into the feasible polytope before committing to an action.
- Battery heterogeneity (capacity, round-trip efficiency, ramp limits, degradation) means a one-size policy underperforms — segment batteries by characteristics.
- `grid_optimize` is one-shot per Challenge instance (enforced atomically); make sure your policy is deterministic given `state`.
