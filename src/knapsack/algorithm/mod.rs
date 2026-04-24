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

    fn recompute_interaction_sums(
        challenge: &Challenge,
        selected: &[usize],
        interaction_sum: &mut [i32],
    ) {
        let n = interaction_sum.len();
        for x in 0..n {
            interaction_sum[x] = challenge.values[x] as i32;
            for &s in selected {
                interaction_sum[x] += challenge.interaction_values[x][s];
            }
        }
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

        let mut tabu_list = vec![0u32; n];
        let mut stall = 0;

        while stall < 200 && Instant::now() < deadline {
            let mut min_sel_val = i32::MAX;
            for &s in &selected {
                min_sel_val = min_sel_val.min(interaction_sum[s]);
            }

            let mut best_diff = 0i32;
            let mut best_swap: Option<(usize, usize)> = None;

            // 1-for-1 swap
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

            // Add-only move: if there's slack capacity, try adding without removing
            let slack = challenge.max_weight - total_weight;
            let mut best_add: Option<usize> = None;
            let mut best_add_val = best_diff;
            for ui in 0..unselected.len() {
                let ni = unselected[ui];
                if tabu_list[ni] > 0 {
                    continue;
                }
                if challenge.weights[ni] > slack {
                    continue;
                }
                let val = interaction_sum[ni];
                if val > best_add_val {
                    best_add_val = val;
                    best_add = Some(ui);
                }
            }

            // Drop-only move: try removing an item if it has negative marginal value
            let mut best_drop: Option<usize> = None;
            let mut best_drop_val = best_diff;
            for si in 0..selected.len() {
                let ri = selected[si];
                if tabu_list[ri] > 0 {
                    continue;
                }
                let marginal = interaction_sum[ri];
                let neg = -marginal;
                if neg > best_drop_val {
                    best_drop_val = neg;
                    best_drop = Some(si);
                }
            }

            // Pick best move
            enum Move {
                Swap(usize, usize),
                Add(usize),
                Drop(usize),
                None,
            }
            let chosen = if let Some(ui) = best_add {
                if best_add_val >= best_diff && best_add_val >= best_drop_val {
                    Move::Add(ui)
                } else if best_drop_val > best_diff {
                    if let Some(si) = best_drop { Move::Drop(si) } else { Move::None }
                } else if let Some((ui, si)) = best_swap {
                    Move::Swap(ui, si)
                } else {
                    Move::None
                }
            } else if best_drop_val > best_diff {
                if let Some(si) = best_drop { Move::Drop(si) } else { Move::None }
            } else if let Some((ui, si)) = best_swap {
                Move::Swap(ui, si)
            } else {
                Move::None
            };

            match chosen {
                Move::Swap(ui, si) => {
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
                }
                Move::Add(ui) => {
                    let ni = unselected[ui];
                    is_selected[ni] = true;
                    total_weight += challenge.weights[ni];
                    selected.push(ni);
                    unselected.swap_remove(ui);
                    for x in 0..n {
                        interaction_sum[x] += challenge.interaction_values[x][ni];
                    }
                    tabu_list[ni] = tabu_tenure;
                }
                Move::Drop(si) => {
                    let ri = selected[si];
                    is_selected[ri] = false;
                    total_weight -= challenge.weights[ri];
                    selected.swap_remove(si);
                    unselected.push(ri);
                    for x in 0..n {
                        interaction_sum[x] -= challenge.interaction_values[x][ri];
                    }
                    tabu_list[ri] = tabu_tenure;
                }
                Move::None => {
                    break;
                }
            }

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
        }

        if Instant::now() >= deadline {
            break;
        }

        // Perturbation: kick random items, re-fill greedily
        let kick_count = (best_selected.len() / 5).max(3).min(best_selected.len());
        selected = best_selected.clone();
        is_selected = best_is_selected.clone();
        total_weight = best_weight;
        unselected = (0..n).filter(|i| !is_selected[*i]).collect();
        recompute_interaction_sums(challenge, &selected, &mut interaction_sum);

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
