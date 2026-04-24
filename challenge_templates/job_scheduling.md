# Active challenge: Flexible Job-Shop Scheduling (FJSP)

Schedule operations across machines to minimise the makespan (finish time of the last operation), respecting per-job operation order and per-operation machine eligibility.

- **Algorithm file:** `src/job_scheduling/algorithm/mod.rs`
- **Per-instance score:** baseline-relative quality `(sota_makespan − your_makespan) / sota_makespan × QUALITY_PRECISION`, clamped to ±10 × QUALITY_PRECISION. The baseline is the SOTA dispatching-rules variant (`compute_sota_baseline`, also `src/job_scheduling/baselines/dispatching_rules.rs`). The evaluator additionally rejects any solution worse than the simpler greedy baseline as infeasible.
- **Aggregate score:** per-track arithmetic mean of per-instance quality, then shifted geometric mean across tracks. Higher is better.
- **Tracks:** parameterised by `(n, scenario)` where `scenario ∈ {FLOW_SHOP, HYBRID_FLOW_SHOP, JOB_SHOP, FJSP_MEDIUM, FJSP_HIGH}` controls reentrance, flexibility, and product mix.

## Types

```rust
pub struct Challenge {
    pub seed: [u8; 32],
    pub num_jobs: usize,
    pub num_machines: usize,
    pub num_operations: usize,
    pub jobs_per_product: Vec<usize>,
    // per product: ordered ops; per op: { eligible machine_id -> processing_time }
    pub product_processing_times: Vec<Vec<HashMap<usize, u32>>>,
}
pub struct Solution {
    pub job_schedule: Vec<Vec<(usize, u32)>>,  // per-job: (machine_0_indexed, start_time) per op
}
```

## Strategy tags (suggestion)

`dispatching_rules`, `local_search`, `tabu`, `simulated_annealing`, `genetic_algorithm`, `large_neighborhood`, `metaheuristic`, `other`

## Tips

- The seed is the dispatching-rules baseline (SPT / FCFS variants with random restarts). To beat it, pair construction with critical-block neighbourhood moves (swap consecutive ops on the makespan-critical path).
- Heavy reentrance scenarios (FJSP_HIGH) reward path-relinking and large-neighbourhood search; flow-shop scenarios are dominated by good initial sequencing.
- Time-window-style precedence makes it easy to write infeasible schedules — validate by simulation before `save_solution()`.
- Anytime: emit a feasible schedule early, then improve incrementally so timeouts still publish a usable result.
