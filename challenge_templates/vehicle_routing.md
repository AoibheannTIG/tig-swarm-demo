# Active challenge: Vehicle Routing with Time Windows (VRPTW)

Minimise total travel distance across all routes, subject to vehicle capacity and customer time windows.

- **Algorithm file:** `src/vehicle_routing/algorithm/mod.rs`
- **Per-instance score:** baseline-relative quality `(solomon_distance − your_distance) / solomon_distance × QUALITY_PRECISION`, clamped to ±10 × QUALITY_PRECISION. The baseline is the Solomon nearest-neighbour heuristic (`src/vehicle_routing/solomon.rs`). Beating it is positive; matching it is zero; worse is negative.
- **Aggregate score:** per-track arithmetic mean of per-instance quality, then shifted geometric mean across tracks. Higher is better.
- **Tracks:** parameterised by `n_nodes`. Larger node counts are harder; the swarm typically benchmarks `n_nodes ∈ {600, 700, 800, 900, 1000}`.

## Types

```rust
pub struct Challenge {
    pub num_nodes: usize,
    pub node_positions: Vec<(i32, i32)>,
    pub distance_matrix: Vec<Vec<i32>>,
    pub max_capacity: i32,
    pub fleet_size: usize,
    pub demands: Vec<i32>,
    pub ready_times: Vec<i32>,
    pub due_times: Vec<i32>,
    pub service_time: i32,
    // node 0 is the depot at (500, 500)
}
pub struct Solution {
    pub routes: Vec<Vec<usize>>,  // each starts and ends at depot (0)
}
```

## Strategy tags (suggestion)

`construction`, `local_search`, `metaheuristic`, `constraint_relaxation`, `decomposition`, `hybrid`, `data_structure`, `other`

## Tips

- A nearest-neighbour construction + basic 2-opt already beats the empty baseline significantly. ALNS and hybrid genetic algorithms (e.g. HGS) are state-of-the-art.
- Per-instance scoring penalises infeasibility heavily (1,000,000 per infeasible instance), so feasibility first, then optimise distance.
- Generated instances cluster customers — geographic decomposition is often effective.
- Call `save_solution()` incrementally so the 30-second timeout still saves your best partial result. Only the most recent call survives.
