use super::*;
use anyhow::Result;
use rand::{rngs::SmallRng, Rng, SeedableRng};
use serde_json::{Map, Value};
use std::time::Instant;

pub fn solve_challenge(
    challenge: &Challenge,
    save_solution: &dyn Fn(&Solution) -> Result<()>,
    _hyperparameters: &Option<Map<String, Value>>,
) -> Result<()> {
    let n = challenge.num_items;
    let deadline = Instant::now() + std::time::Duration::from_secs(28);
    let mut rng = SmallRng::from_seed(challenge.seed);

    let mut selected: Vec<usize> = Vec::with_capacity(n);
    let mut is_selected = vec![false; n];
    let mut total_weight: u32 = 0;

    // Greedy construction: sort by total interaction value / weight
    let mut item_ratios: Vec<(usize, f64)> = (0..n)
        .map(|i| {
            let total: i32 = challenge.values[i] as i32
                + challenge.interaction_values[i].iter().sum::<i32>();
            (i, total as f64 / challenge.weights[i] as f64)
        })
        .collect();
    item_ratios.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    for &(item, _) in &item_ratios {
        if total_weight + challenge.weights[item] <= challenge.max_weight {
            selected.push(item);
            is_selected[item] = true;
            total_weight += challenge.weights[item];
        }
    }

    let mut unselected: Vec<usize> = (0..n).filter(|i| !is_selected[*i]).collect();

    let mut interaction_sum = vec![0i32; n];
    for x in 0..n {
        interaction_sum[x] = challenge.values[x] as i32;
        for &s in &selected {
            interaction_sum[x] += challenge.interaction_values[x][s];
        }
    }

    fn compute_obj(challenge: &Challenge, selected: &[usize]) -> i64 {
        let mut val: i64 = 0;
        for &i in selected {
            val += challenge.values[i] as i64;
        }
        for a in 0..selected.len() {
            for b in (a + 1)..selected.len() {
                val += challenge.interaction_values[selected[a]][selected[b]] as i64;
            }
        }
        val
    }

    let mut best_obj = compute_obj(challenge, &selected);
    save_solution(&Solution { items: selected.clone() })?;

    let mut best_selected = selected.clone();
    let mut best_is_selected = is_selected.clone();
    let mut best_weight = total_weight;

    let tabu_tenure = 7u32;

    loop {
        if Instant::now() >= deadline {
            break;
        }

        // Tabu swap local search
        let mut tabu_list = vec![0u32; n];
        let mut stall = 0;

        while stall < 200 && Instant::now() < deadline {
            let mut min_sel_val = i32::MAX;
            for &s in &selected {
                min_sel_val = min_sel_val.min(interaction_sum[s]);
            }

            let mut best_diff = 0i32;
            let mut best_swap: Option<(usize, usize)> = None;

            for ui in 0..unselected.len() {
                let ni = unselected[ui];
                if tabu_list[ni] > 0 {
                    continue;
                }
                let nv = interaction_sum[ni];
                if nv < best_diff + min_sel_val {
                    continue;
                }
                let min_w = challenge.weights[ni] as i32
                    - (challenge.max_weight as i32 - total_weight as i32);

                for si in 0..selected.len() {
                    let ri = selected[si];
                    if tabu_list[ri] > 0 {
                        continue;
                    }
                    if min_w > 0 && (challenge.weights[ri] as i32) < min_w {
                        continue;
                    }
                    let rv = interaction_sum[ri];
                    let diff = nv - rv - challenge.interaction_values[ni][ri];
                    if diff > best_diff {
                        best_diff = diff;
                        best_swap = Some((ui, si));
                    }
                }
            }

            if let Some((ui, si)) = best_swap {
                let ni = unselected[ui];
                let ri = selected[si];

                is_selected[ni] = true;
                is_selected[ri] = false;
                total_weight = total_weight + challenge.weights[ni] - challenge.weights[ri];

                selected.swap_remove(si);
                unselected.swap_remove(ui);
                selected.push(ni);
                unselected.push(ri);

                for x in 0..n {
                    interaction_sum[x] += challenge.interaction_values[x][ni]
                        - challenge.interaction_values[x][ri];
                }

                tabu_list[ni] = tabu_tenure;
                tabu_list[ri] = tabu_tenure;
                for t in tabu_list.iter_mut() {
                    *t = t.saturating_sub(1);
                }

                let obj = compute_obj(challenge, &selected);
                if obj > best_obj {
                    best_obj = obj;
                    best_selected = selected.clone();
                    best_is_selected = is_selected.clone();
                    best_weight = total_weight;
                    save_solution(&Solution { items: selected.clone() })?;
                    stall = 0;
                } else {
                    stall += 1;
                }
            } else {
                break;
            }
        }

        if Instant::now() >= deadline {
            break;
        }

        // Perturbation: kick random items out and re-greedily fill
        let kick_count = (selected.len() / 5).max(3).min(selected.len());
        selected = best_selected.clone();
        is_selected = best_is_selected.clone();
        total_weight = best_weight;
        unselected = (0..n).filter(|i| !is_selected[*i]).collect();

        // Recompute interaction sums from scratch
        for x in 0..n {
            interaction_sum[x] = challenge.values[x] as i32;
            for &s in &selected {
                interaction_sum[x] += challenge.interaction_values[x][s];
            }
        }

        // Remove random selected items
        for _ in 0..kick_count {
            if selected.is_empty() {
                break;
            }
            let idx = rng.gen_range(0..selected.len());
            let item = selected.swap_remove(idx);
            is_selected[item] = false;
            total_weight -= challenge.weights[item];
            unselected.push(item);
            for x in 0..n {
                interaction_sum[x] -= challenge.interaction_values[x][item];
            }
        }

        // Re-fill greedily by current interaction sum / weight
        let mut candidates: Vec<(usize, f64)> = unselected
            .iter()
            .map(|&i| (i, interaction_sum[i] as f64 / challenge.weights[i] as f64))
            .collect();
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        for (item, _) in candidates {
            if is_selected[item] {
                continue;
            }
            if total_weight + challenge.weights[item] <= challenge.max_weight {
                selected.push(item);
                is_selected[item] = true;
                total_weight += challenge.weights[item];
                for x in 0..n {
                    interaction_sum[x] += challenge.interaction_values[x][item];
                }
            }
        }

        unselected = (0..n).filter(|i| !is_selected[*i]).collect();
    }

    Ok(())
}
