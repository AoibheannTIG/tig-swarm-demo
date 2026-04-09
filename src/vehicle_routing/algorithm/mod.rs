use super::*;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize)]
pub struct Hyperparameters {}

pub fn solve_challenge(
    challenge: &Challenge,
    save_solution: &dyn Fn(&Solution) -> Result<()>,
    _hyperparameters: &Option<Map<String, Value>>,
) -> Result<()> {
    let start = Instant::now();
    let deadline = start + Duration::from_millis(4500);
    let alns_deadline = start + Duration::from_millis(3200);
    let n = challenge.num_nodes;
    let dm = &challenge.distance_matrix;
    let fleet = challenge.fleet_size;
    let mut rng: u64 = 0xdeadbeef ^ (n as u64);

    // Phase 1: NN construction
    let mut routes = nn_construction(challenge);
    save_solution(&Solution { routes: routes.clone() })?;

    // Phase 2: Merge routes
    merge_to_fleet(&mut routes, challenge);
    save_solution(&Solution { routes: routes.clone() })?;

    // Phase 3: Quick greedy local search (budget-limited)
    let ls_deadline = start + Duration::from_millis(800);
    greedy_local_search(&mut routes, challenge, &ls_deadline);
    save_solution(&Solution { routes: routes.clone() })?;

    // Precompute Shaw relatedness: distance + time window overlap
    // shaw_neighbors[i] = sorted list of most related customers to i
    let max_nn = 30.min(n - 1);
    let shaw_neighbors: Vec<Vec<usize>> = (0..n).map(|i| {
        if i == 0 { return Vec::new(); }
        let mut scored: Vec<(i64, usize)> = (1..n)
            .filter(|&j| j != i)
            .map(|j| {
                let dist_rel = dm[i][j] as i64;
                let tw_overlap = 0i64.max(
                    challenge.due_times[i].min(challenge.due_times[j]) as i64
                    - challenge.ready_times[i].max(challenge.ready_times[j]) as i64
                );
                // Lower score = more related (close distance, overlapping time windows)
                dist_rel * 2 - tw_overlap
            })
            .enumerate()
            .map(|(idx, score)| (score, idx + 1)) // idx+1 because we skip j==i
            .collect();
        // Rebuild: we want customer indices, not raw enumerate indices
        let mut scored2: Vec<(i64, usize)> = (1..n)
            .filter(|&j| j != i)
            .map(|j| {
                let dist_rel = dm[i][j] as i64;
                let tw_overlap = 0i64.max(
                    challenge.due_times[i].min(challenge.due_times[j]) as i64
                    - challenge.ready_times[i].max(challenge.ready_times[j]) as i64
                );
                (dist_rel * 2 - tw_overlap, j)
            })
            .collect();
        scored2.sort_unstable();
        scored2.iter().take(max_nn).map(|&(_, j)| j).collect()
    }).collect();
    let fleet_penalty: i64 = 100_000;
    let penalized_cost = |rs: &[Vec<usize>]| -> i64 {
        let dist: i64 = rs.iter().map(|r| route_dist(r, dm) as i64).sum();
        let excess = if rs.len() > fleet { (rs.len() - fleet) as i64 } else { 0 };
        dist + excess * fleet_penalty
    };

    let mut best_routes = routes.clone();
    let mut best_pen = penalized_cost(&routes);
    let mut current_pen = best_pen;

    // ====== Phase 4: ALNS ======
    let num_destroy_ops = 4; // random, worst, shaw, route_removal
    let num_repair_ops = 2;  // greedy, regret
    let mut d_weights = vec![1.0f64; num_destroy_ops];
    let mut r_weights = vec![1.0f64; num_repair_ops];
    let mut d_scores = vec![0.0f64; num_destroy_ops];
    let mut r_scores = vec![0.0f64; num_repair_ops];
    let mut d_uses = vec![0u32; num_destroy_ops];
    let mut r_uses = vec![0u32; num_repair_ops];

    let mut temperature = (current_pen as f64).abs().max(1000.0) * 0.04;
    let cooling = 0.9995;
    let mut iters = 0u32;
    let mut seg_iters = 0u32;

    while Instant::now() < alns_deadline {
        iters += 1;
        seg_iters += 1;

        let d_op = roulette_select(&d_weights, rand_lcg(&mut rng));
        let r_op = roulette_select(&r_weights, rand_lcg(&mut rng));

        let total_custs: usize = routes.iter().map(|r| r.len().saturating_sub(2)).sum();
        let min_d = (total_custs / 10).max(3);
        let max_d = (total_custs * 3 / 10).max(min_d + 1);
        let destroy_count = min_d + (rand_lcg(&mut rng) as usize) % (max_d - min_d);

        let removed = match d_op {
            0 => random_removal(&routes, destroy_count, &mut rng),
            1 => worst_removal(&routes, destroy_count, dm, &mut rng),
            2 => shaw_removal(&routes, destroy_count, &shaw_neighbors, &mut rng),
            _ => route_removal(&routes, destroy_count, &mut rng),
        };

        if removed.is_empty() { continue; }

        let mut partial = routes.clone();
        for &c in &removed {
            for route in &mut partial {
                if let Some(pos) = route.iter().position(|&x| x == c) {
                    route.remove(pos);
                    break;
                }
            }
        }
        partial.retain(|r| r.len() > 2);

        let new_routes = match r_op {
            0 => greedy_insertion(partial, &removed, challenge),
            _ => regret_insertion(partial, &removed, challenge),
        };

        let new_pen = penalized_cost(&new_routes);
        let delta = new_pen as f64 - current_pen as f64;

        let accept = delta < 0.0 || (temperature > 0.5 && {
            (rand_lcg(&mut rng) % 1_000_000) as f64 / 1_000_000.0 < (-delta / temperature).exp()
        });

        let score = if new_pen < best_pen { 33.0 }
            else if accept && delta < 0.0 { 9.0 }
            else if accept { 3.0 }
            else { 0.0 };

        d_scores[d_op] += score;
        r_scores[r_op] += score;
        d_uses[d_op] += 1;
        r_uses[r_op] += 1;

        if accept {
            routes = new_routes;
            current_pen = new_pen;
            if current_pen < best_pen {
                best_pen = current_pen;
                best_routes = routes.clone();
                if routes.len() <= fleet {
                    let _ = save_solution(&Solution { routes: routes.clone() });
                }
            }
        }

        temperature *= cooling;

        if seg_iters >= 80 {
            let decay = 0.8;
            for i in 0..num_destroy_ops {
                if d_uses[i] > 0 {
                    d_weights[i] = d_weights[i] * decay + (1.0 - decay) * (d_scores[i] / d_uses[i] as f64);
                }
                d_weights[i] = d_weights[i].max(0.1);
                d_scores[i] = 0.0; d_uses[i] = 0;
            }
            for i in 0..num_repair_ops {
                if r_uses[i] > 0 {
                    r_weights[i] = r_weights[i] * decay + (1.0 - decay) * (r_scores[i] / r_uses[i] as f64);
                }
                r_weights[i] = r_weights[i].max(0.1);
                r_scores[i] = 0.0; r_uses[i] = 0;
            }
            seg_iters = 0;

            if iters % 1200 == 0 && current_pen > best_pen {
                temperature = (best_pen as f64).abs().max(1000.0) * 0.02;
                routes = best_routes.clone();
                current_pen = best_pen;
            }
        }
    }

    // ====== Phase 5: SA fine-tuning ======
    routes = best_routes.clone();
    current_pen = best_pen;
    let mut sa_temp = (best_pen as f64).abs().max(1000.0) * 0.02;
    let mut route_dists: Vec<i32> = routes.iter().map(|r| route_dist(r, dm)).collect();
    let mut stag = 0u32;

    while Instant::now() < deadline {
        let op = if routes.len() > fleet {
            let r = rand_lcg(&mut rng) % 10;
            if r < 5 { 1 } else if r < 7 { 4 } else { (rand_lcg(&mut rng) % 3) as u64 }
        } else {
            rand_lcg(&mut rng) % 5
        };

        let result = match op {
            0 => try_intra_2opt(&routes, &route_dists, challenge, &mut rng, dm),
            1 => try_relocate(&routes, &route_dists, challenge, &mut rng, dm, fleet, fleet_penalty),
            2 => try_exchange(&routes, &route_dists, challenge, &mut rng, dm),
            3 => try_or_opt(&routes, challenge, &mut rng),
            _ => try_cross_2opt(&routes, challenge, &mut rng),
        };

        match result {
            SaResult::Failed => {}
            SaResult::Delta { delta_pen, apply } => {
                let d = delta_pen as f64;
                if d < 0.0 || (sa_temp > 0.1 && (rand_lcg(&mut rng) % 1_000_000) as f64 / 1_000_000.0 < (-d / sa_temp).exp()) {
                    apply(&mut routes, &mut route_dists);
                    current_pen += delta_pen;
                    if current_pen < best_pen {
                        best_pen = current_pen;
                        best_routes = routes.clone();
                        if routes.len() <= fleet { let _ = save_solution(&Solution { routes: routes.clone() }); }
                        stag = 0;
                    }
                }
            }
            SaResult::Full(new_routes) => {
                let new_pen = penalized_cost(&new_routes);
                let d = new_pen as f64 - current_pen as f64;
                if d < 0.0 || (sa_temp > 0.1 && (rand_lcg(&mut rng) % 1_000_000) as f64 / 1_000_000.0 < (-d / sa_temp).exp()) {
                    route_dists = new_routes.iter().map(|r| route_dist(r, dm)).collect();
                    routes = new_routes;
                    current_pen = new_pen;
                    if current_pen < best_pen {
                        best_pen = current_pen;
                        best_routes = routes.clone();
                        if routes.len() <= fleet { let _ = save_solution(&Solution { routes: routes.clone() }); }
                        stag = 0;
                    }
                }
            }
        }

        sa_temp *= 0.9998;
        stag += 1;
        if stag > 5000 {
            sa_temp = (best_pen as f64).abs().max(1000.0) * 0.015;
            routes = best_routes.clone();
            current_pen = best_pen;
            route_dists = routes.iter().map(|r| route_dist(r, dm)).collect();
            stag = 0;
        }
    }

    if best_routes.len() <= fleet {
        save_solution(&Solution { routes: best_routes })?;
    }
    Ok(())
}

// ---- Utilities ----

fn rand_lcg(state: &mut u64) -> u64 {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *state >> 33
}

fn roulette_select(weights: &[f64], rand_val: u64) -> usize {
    let total: f64 = weights.iter().sum();
    let r = (rand_val % 1_000_000) as f64 / 1_000_000.0 * total;
    let mut cum = 0.0;
    for (i, &w) in weights.iter().enumerate() {
        cum += w;
        if r <= cum { return i; }
    }
    weights.len() - 1
}

// ---- Destroy operators ----

fn random_removal(routes: &[Vec<usize>], count: usize, rng: &mut u64) -> Vec<usize> {
    let mut custs: Vec<usize> = routes.iter()
        .flat_map(|r| r.iter().filter(|&&x| x != 0).copied()).collect();
    let mut out = Vec::with_capacity(count);
    for i in 0..count.min(custs.len()) {
        let j = i + (rand_lcg(rng) as usize) % (custs.len() - i);
        custs.swap(i, j);
        out.push(custs[i]);
    }
    out
}

fn worst_removal(routes: &[Vec<usize>], count: usize, dm: &[Vec<i32>], rng: &mut u64) -> Vec<usize> {
    let mut costs: Vec<(i64, usize)> = Vec::new();
    for route in routes {
        for i in 1..route.len() - 1 {
            let (prev, c, next) = (route[i-1], route[i], route[i+1]);
            costs.push((dm[prev][c] as i64 + dm[c][next] as i64 - dm[prev][next] as i64, c));
        }
    }
    costs.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    let mut out = Vec::with_capacity(count);
    let mut used = vec![false; costs.len()];
    for _ in 0..count.min(costs.len()) {
        let rem: Vec<usize> = (0..costs.len()).filter(|&i| !used[i]).collect();
        if rem.is_empty() { break; }
        let r = (rand_lcg(rng) % 1_000_000) as f64 / 1_000_000.0;
        let idx = (r.powf(3.0) * rem.len() as f64).min(rem.len() as f64 - 1.0) as usize;
        used[rem[idx]] = true;
        out.push(costs[rem[idx]].1);
    }
    out
}

fn shaw_removal(routes: &[Vec<usize>], count: usize, nn: &[Vec<usize>], rng: &mut u64) -> Vec<usize> {
    let custs: Vec<usize> = routes.iter()
        .flat_map(|r| r.iter().filter(|&&x| x != 0).copied()).collect();
    if custs.is_empty() { return Vec::new(); }

    let seed = custs[(rand_lcg(rng) as usize) % custs.len()];
    let mut out = vec![seed];
    let mut used = vec![false; nn.len()];
    used[seed] = true;

    while out.len() < count {
        let ref_c = out[(rand_lcg(rng) as usize) % out.len()];
        let mut found = false;
        for &nb in &nn[ref_c] {
            if !used[nb] && custs.contains(&nb) {
                out.push(nb);
                used[nb] = true;
                found = true;
                break;
            }
        }
        if !found {
            let unrem: Vec<usize> = custs.iter().filter(|&&c| !used[c]).copied().collect();
            if unrem.is_empty() { break; }
            let c = unrem[(rand_lcg(rng) as usize) % unrem.len()];
            out.push(c);
            used[c] = true;
        }
    }
    out
}

fn route_removal(routes: &[Vec<usize>], target_count: usize, rng: &mut u64) -> Vec<usize> {
    if routes.is_empty() { return Vec::new(); }
    let mut out = Vec::new();
    let mut used_routes = vec![false; routes.len()];

    // Remove entire routes until we've removed enough customers
    while out.len() < target_count {
        let avail: Vec<usize> = (0..routes.len()).filter(|&i| !used_routes[i]).collect();
        if avail.is_empty() { break; }
        let ri = avail[(rand_lcg(rng) as usize) % avail.len()];
        used_routes[ri] = true;
        for &c in &routes[ri] {
            if c != 0 { out.push(c); }
        }
    }
    out
}

// ---- Repair operators ----

fn greedy_insertion(mut routes: Vec<Vec<usize>>, removed: &[usize], ch: &Challenge) -> Vec<Vec<usize>> {
    let dm = &ch.distance_matrix;
    let mut to_insert: Vec<usize> = removed.to_vec();
    let mut route_loads: Vec<i32> = routes.iter()
        .map(|r| r.iter().filter(|&&x| x != 0).map(|&x| ch.demands[x]).sum()).collect();

    while !to_insert.is_empty() {
        let mut best_cost = i32::MAX;
        let mut best_ci = 0;
        let mut best_ri = 0;
        let mut best_pos = 0;

        for (ci, &cust) in to_insert.iter().enumerate() {
            for (ri, route) in routes.iter().enumerate() {
                if route_loads[ri] + ch.demands[cust] > ch.max_capacity { continue; }
                for pos in 1..route.len() {
                    let cost = dm[route[pos-1]][cust] + dm[cust][route[pos]] - dm[route[pos-1]][route[pos]];
                    if cost < best_cost {
                        let mut test = route.clone();
                        test.insert(pos, cust);
                        if is_feasible(&test, ch) {
                            best_cost = cost;
                            best_ci = ci; best_ri = ri; best_pos = pos;
                        }
                    }
                }
            }
        }

        if best_cost < i32::MAX {
            let cust = to_insert.remove(best_ci);
            routes[best_ri].insert(best_pos, cust);
            route_loads[best_ri] += ch.demands[cust];
        } else {
            let cust = to_insert.remove(0);
            routes.push(vec![0, cust, 0]);
            route_loads.push(ch.demands[cust]);
        }
    }
    routes
}

fn regret_insertion(mut routes: Vec<Vec<usize>>, removed: &[usize], ch: &Challenge) -> Vec<Vec<usize>> {
    let dm = &ch.distance_matrix;
    let mut to_insert: Vec<usize> = removed.to_vec();
    let mut route_loads: Vec<i32> = routes.iter()
        .map(|r| r.iter().filter(|&&x| x != 0).map(|&x| ch.demands[x]).sum()).collect();

    while !to_insert.is_empty() {
        let mut best_regret = i64::MIN;
        let mut best_ci = 0;
        let mut best_ri = 0;
        let mut best_pos = 0;
        let mut best_is_new = false;

        for (ci, &cust) in to_insert.iter().enumerate() {
            let mut route_bests: Vec<(i32, usize, usize)> = Vec::new();

            for (ri, route) in routes.iter().enumerate() {
                if route_loads[ri] + ch.demands[cust] > ch.max_capacity { continue; }

                let mut best_in_route = i32::MAX;
                let mut best_pos_in_route = 1;
                for pos in 1..route.len() {
                    let cost = dm[route[pos-1]][cust] + dm[cust][route[pos]] - dm[route[pos-1]][route[pos]];
                    if cost < best_in_route {
                        let mut test = route.clone();
                        test.insert(pos, cust);
                        if is_feasible(&test, ch) {
                            best_in_route = cost;
                            best_pos_in_route = pos;
                        }
                    }
                }
                if best_in_route < i32::MAX {
                    route_bests.push((best_in_route, ri, best_pos_in_route));
                }
            }

            if route_bests.is_empty() {
                if (i64::MAX - 1) > best_regret {
                    best_regret = i64::MAX - 1;
                    best_ci = ci; best_is_new = true;
                }
                continue;
            }

            route_bests.sort_unstable_by_key(|&(c, _, _)| c);
            let regret = if route_bests.len() >= 2 {
                (route_bests[1].0 - route_bests[0].0) as i64
            } else {
                (route_bests[0].0 as i64).abs() + 1000
            };

            if regret > best_regret {
                best_regret = regret;
                best_ci = ci;
                best_ri = route_bests[0].1;
                best_pos = route_bests[0].2;
                best_is_new = false;
            }
        }

        let cust = to_insert.remove(best_ci);
        if best_is_new {
            routes.push(vec![0, cust, 0]);
            route_loads.push(ch.demands[cust]);
        } else {
            routes[best_ri].insert(best_pos, cust);
            route_loads[best_ri] += ch.demands[cust];
        }
    }
    routes
}

// ---- SA operators ----

enum SaResult {
    Failed,
    Delta { delta_pen: i64, apply: Box<dyn FnOnce(&mut Vec<Vec<usize>>, &mut Vec<i32>)> },
    Full(Vec<Vec<usize>>),
}

fn try_intra_2opt(routes: &[Vec<usize>], rd: &[i32], ch: &Challenge, rng: &mut u64, dm: &[Vec<i32>]) -> SaResult {
    if routes.is_empty() { return SaResult::Failed; }
    let ri = (rand_lcg(rng) as usize) % routes.len();
    let route = &routes[ri];
    if route.len() < 4 { return SaResult::Failed; }
    let n = route.len();
    let i = 1 + (rand_lcg(rng) as usize) % (n - 3);
    let j = (i + 1 + (rand_lcg(rng) as usize) % (n - 1 - i)).min(n - 2);
    let mut nr = route.clone();
    nr[i..=j].reverse();
    if !is_feasible(&nr, ch) { return SaResult::Failed; }
    let nd = route_dist(&nr, dm);
    let delta = (nd - rd[ri]) as i64;
    SaResult::Delta { delta_pen: delta, apply: Box::new(move |rs: &mut Vec<Vec<usize>>, ds: &mut Vec<i32>| { rs[ri] = nr; ds[ri] = nd; }) }
}

fn try_relocate(routes: &[Vec<usize>], rd: &[i32], ch: &Challenge, rng: &mut u64, dm: &[Vec<i32>], fleet: usize, fp: i64) -> SaResult {
    if routes.len() < 2 { return SaResult::Failed; }
    let src = (rand_lcg(rng) as usize) % routes.len();
    if routes[src].len() <= 2 { return SaResult::Failed; }
    let pos = 1 + (rand_lcg(rng) as usize) % (routes[src].len() - 2);
    let cust = routes[src][pos];
    let dst = loop { let d = (rand_lcg(rng) as usize) % routes.len(); if d != src { break d; } };
    let dl: i32 = routes[dst].iter().filter(|&&x| x != 0).map(|&x| ch.demands[x]).sum::<i32>() + ch.demands[cust];
    if dl > ch.max_capacity { return SaResult::Failed; }

    let mut bi = 1; let mut bc = i32::MAX;
    for ins in 1..routes[dst].len() {
        let c = dm[routes[dst][ins-1]][cust] + dm[cust][routes[dst][ins]] - dm[routes[dst][ins-1]][routes[dst][ins]];
        if c < bc { bc = c; bi = ins; }
    }

    let mut ns = routes[src].clone(); ns.remove(pos);
    let mut nd = routes[dst].clone(); nd.insert(bi, cust);
    if ns.len() > 2 && !is_feasible(&ns, ch) { return SaResult::Failed; }
    if !is_feasible(&nd, ch) { return SaResult::Failed; }

    let nsd = if ns.len() > 2 { route_dist(&ns, dm) } else { 0 };
    let ndd = route_dist(&nd, dm);
    let mut dp = nsd as i64 + ndd as i64 - rd[src] as i64 - rd[dst] as i64;
    if ns.len() <= 2 && routes.len() > fleet { dp -= fp; }

    SaResult::Delta { delta_pen: dp, apply: Box::new(move |rs: &mut Vec<Vec<usize>>, ds: &mut Vec<i32>| {
        rs[src] = ns; ds[src] = nsd; rs[dst] = nd; ds[dst] = ndd;
        let mut i = 0;
        while i < rs.len() { if rs[i].len() <= 2 { rs.remove(i); ds.remove(i); } else { i += 1; } }
    }) }
}

fn try_exchange(routes: &[Vec<usize>], rd: &[i32], ch: &Challenge, rng: &mut u64, dm: &[Vec<i32>]) -> SaResult {
    if routes.len() < 2 { return SaResult::Failed; }
    let r1 = (rand_lcg(rng) as usize) % routes.len();
    let r2 = (rand_lcg(rng) as usize) % routes.len();
    if r1 == r2 || routes[r1].len() <= 2 || routes[r2].len() <= 2 { return SaResult::Failed; }
    let p1 = 1 + (rand_lcg(rng) as usize) % (routes[r1].len() - 2);
    let p2 = 1 + (rand_lcg(rng) as usize) % (routes[r2].len() - 2);
    let (c1, c2) = (routes[r1][p1], routes[r2][p2]);
    let l1: i32 = routes[r1].iter().filter(|&&x| x != 0).map(|&x| ch.demands[x]).sum::<i32>() - ch.demands[c1] + ch.demands[c2];
    let l2: i32 = routes[r2].iter().filter(|&&x| x != 0).map(|&x| ch.demands[x]).sum::<i32>() - ch.demands[c2] + ch.demands[c1];
    if l1 > ch.max_capacity || l2 > ch.max_capacity { return SaResult::Failed; }
    let mut nr1 = routes[r1].clone(); nr1[p1] = c2;
    let mut nr2 = routes[r2].clone(); nr2[p2] = c1;
    if !is_feasible(&nr1, ch) || !is_feasible(&nr2, ch) { return SaResult::Failed; }
    let (nd1, nd2) = (route_dist(&nr1, dm), route_dist(&nr2, dm));
    let delta = (nd1 + nd2 - rd[r1] - rd[r2]) as i64;
    SaResult::Delta { delta_pen: delta, apply: Box::new(move |rs: &mut Vec<Vec<usize>>, ds: &mut Vec<i32>| { rs[r1] = nr1; rs[r2] = nr2; ds[r1] = nd1; ds[r2] = nd2; }) }
}

fn try_or_opt(routes: &[Vec<usize>], ch: &Challenge, rng: &mut u64) -> SaResult {
    if routes.is_empty() { return SaResult::Failed; }
    let src = (rand_lcg(rng) as usize) % routes.len();
    if routes[src].len() <= 2 { return SaResult::Failed; }
    let cc = routes[src].len() - 2;
    let sl = 1 + (rand_lcg(rng) as usize) % 3.min(cc);
    let ms = cc - sl;
    let start = 1 + (rand_lcg(rng) as usize) % (ms + 1);
    let seg: Vec<usize> = routes[src][start..start + sl].to_vec();
    let dst = (rand_lcg(rng) as usize) % routes.len();

    if src != dst {
        let sd: i32 = seg.iter().map(|&c| ch.demands[c]).sum();
        let dl: i32 = routes[dst].iter().filter(|&&x| x != 0).map(|&x| ch.demands[x]).sum::<i32>() + sd;
        if dl > ch.max_capacity { return SaResult::Failed; }
    }

    let mut nr = routes.to_vec();
    nr[src] = routes[src][..start].iter().chain(routes[src][start + sl..].iter()).copied().collect();
    let ed = if src == dst { src } else { dst };
    if nr[ed].len() <= 1 { return SaResult::Failed; }
    let ins = 1 + (rand_lcg(rng) as usize) % (nr[ed].len() - 1);
    let mut dr = nr[ed][..ins].to_vec();
    dr.extend_from_slice(&seg);
    dr.extend_from_slice(&nr[ed][ins..]);
    nr[ed] = dr;

    if nr[src].len() > 2 && !is_feasible(&nr[src], ch) { return SaResult::Failed; }
    if !is_feasible(&nr[ed], ch) { return SaResult::Failed; }
    nr.retain(|r| r.len() > 2);
    SaResult::Full(nr)
}

fn try_cross_2opt(routes: &[Vec<usize>], ch: &Challenge, rng: &mut u64) -> SaResult {
    if routes.len() < 2 { return SaResult::Failed; }
    let r1 = (rand_lcg(rng) as usize) % routes.len();
    let r2 = (rand_lcg(rng) as usize) % routes.len();
    if r1 == r2 || routes[r1].len() <= 2 || routes[r2].len() <= 2 { return SaResult::Failed; }
    let c1 = 1 + (rand_lcg(rng) as usize) % (routes[r1].len() - 2);
    let c2 = 1 + (rand_lcg(rng) as usize) % (routes[r2].len() - 2);
    let mut nr1: Vec<usize> = routes[r1][..=c1].to_vec(); nr1.extend_from_slice(&routes[r2][c2+1..]);
    let mut nr2: Vec<usize> = routes[r2][..=c2].to_vec(); nr2.extend_from_slice(&routes[r1][c1+1..]);
    let l1: i32 = nr1.iter().filter(|&&x| x != 0).map(|&x| ch.demands[x]).sum();
    let l2: i32 = nr2.iter().filter(|&&x| x != 0).map(|&x| ch.demands[x]).sum();
    if l1 > ch.max_capacity || l2 > ch.max_capacity { return SaResult::Failed; }
    if !is_feasible(&nr1, ch) || !is_feasible(&nr2, ch) { return SaResult::Failed; }
    let mut nr = routes.to_vec(); nr[r1] = nr1; nr[r2] = nr2;
    nr.retain(|r| r.len() > 2);
    SaResult::Full(nr)
}

// ---- Construction helpers ----

fn nn_construction(ch: &Challenge) -> Vec<Vec<usize>> {
    let n = ch.num_nodes;
    let dm = &ch.distance_matrix;
    let mut visited = vec![false; n];
    visited[0] = true;
    let mut routes = Vec::new();

    while visited.iter().skip(1).any(|&v| !v) {
        let seed = (1..n).filter(|&i| !visited[i]).min_by_key(|&i| ch.due_times[i]).unwrap();
        visited[seed] = true;
        let mut route = vec![0, seed];
        let mut load = ch.demands[seed];
        let mut time = dm[0][seed].max(ch.ready_times[seed]) + ch.service_time;
        let mut cur = seed;

        loop {
            let mut bd = i32::MAX;
            let mut best = None;
            for j in 1..n {
                if visited[j] { continue; }
                if load + ch.demands[j] > ch.max_capacity { continue; }
                let arr = time + dm[cur][j];
                if arr > ch.due_times[j] { continue; }
                let dep = arr.max(ch.ready_times[j]) + ch.service_time;
                if dep + dm[j][0] > ch.due_times[0] { continue; }
                if dm[cur][j] < bd { bd = dm[cur][j]; best = Some(j); }
            }
            match best {
                Some(j) => {
                    visited[j] = true; route.push(j); load += ch.demands[j];
                    time = (time + dm[cur][j]).max(ch.ready_times[j]) + ch.service_time;
                    cur = j;
                }
                None => break,
            }
        }
        route.push(0);
        routes.push(route);
    }
    routes
}

fn merge_to_fleet(routes: &mut Vec<Vec<usize>>, ch: &Challenge) {
    while routes.len() > ch.fleet_size {
        let mut best_delta = i64::MAX;
        let mut best_i = 0;
        let mut best_j = 1;
        let mut best_merged: Option<Vec<usize>> = None;

        for i in 0..routes.len() {
            for j in i+1..routes.len() {
                let ci: i32 = routes[i].iter().filter(|&&x| x != 0).map(|&x| ch.demands[x]).sum();
                let cj: i32 = routes[j].iter().filter(|&&x| x != 0).map(|&x| ch.demands[x]).sum();
                if ci + cj > ch.max_capacity { continue; }

                // Try concatenation
                let mut m = routes[i][..routes[i].len()-1].to_vec();
                m.extend_from_slice(&routes[j][1..]);
                if is_feasible(&m, ch) {
                    let d = route_dist(&m, &ch.distance_matrix) as i64
                        - route_dist(&routes[i], &ch.distance_matrix) as i64
                        - route_dist(&routes[j], &ch.distance_matrix) as i64;
                    if d < best_delta { best_delta = d; best_i = i; best_j = j; best_merged = Some(m); }
                }

                // Try cheapest insertion
                let jc: Vec<usize> = routes[j].iter().filter(|&&x| x != 0).copied().collect();
                let mut cand = routes[i].clone();
                let orig = route_dist(&routes[i], &ch.distance_matrix) + route_dist(&routes[j], &ch.distance_matrix);
                let mut ok = true;
                for &c in &jc {
                    let mut mc = i32::MAX; let mut mp = 0; let mut found = false;
                    for pos in 1..cand.len() {
                        let cost = ch.distance_matrix[cand[pos-1]][c] + ch.distance_matrix[c][cand[pos]] - ch.distance_matrix[cand[pos-1]][cand[pos]];
                        if cost < mc { let mut t = cand.clone(); t.insert(pos, c); if is_feasible(&t, ch) { mc = cost; mp = pos; found = true; } }
                    }
                    if found { cand.insert(mp, c); } else { ok = false; break; }
                }
                if ok {
                    let d = route_dist(&cand, &ch.distance_matrix) as i64 - orig as i64;
                    if d < best_delta { best_delta = d; best_i = i; best_j = j; best_merged = Some(cand); }
                }
            }
        }

        match best_merged {
            Some(m) => { routes[best_i] = m; routes.remove(best_j); }
            None => break,
        }
    }
}

fn greedy_local_search(routes: &mut Vec<Vec<usize>>, ch: &Challenge, deadline: &Instant) {
    loop {
        if Instant::now() >= *deadline { break; }
        let mut any = false;
        for r in 0..routes.len() {
            if two_opt_route(&mut routes[r], ch) { any = true; }
            if Instant::now() >= *deadline { return; }
        }
        if greedy_relocate(routes, ch) { any = true; }
        if !any { break; }
    }
}

fn two_opt_route(route: &mut Vec<usize>, ch: &Challenge) -> bool {
    let n = route.len();
    if n < 4 { return false; }
    let dm = &ch.distance_matrix;
    let mut improved = false;
    let mut changed = true;
    while changed {
        changed = false;
        'seg: for i in 1..n-2 {
            for j in i+1..n-1 {
                if dm[route[i-1]][route[j]] + dm[route[i]][route[j+1]] < dm[route[i-1]][route[i]] + dm[route[j]][route[j+1]] {
                    let mut c = route.clone(); c[i..=j].reverse();
                    if is_feasible(&c, ch) { *route = c; improved = true; changed = true; break 'seg; }
                }
            }
        }
    }
    improved
}

fn greedy_relocate(routes: &mut Vec<Vec<usize>>, ch: &Challenge) -> bool {
    let dm = &ch.distance_matrix;
    for src in 0..routes.len() {
        if routes[src].len() <= 3 { continue; }
        for pos in 1..routes[src].len()-1 {
            let cust = routes[src][pos];
            let rg = dm[routes[src][pos-1]][cust] + dm[cust][routes[src][pos+1]] - dm[routes[src][pos-1]][routes[src][pos+1]];
            for dst in 0..routes.len() {
                if src == dst { continue; }
                let dl: i32 = routes[dst].iter().filter(|&&x| x != 0).map(|&x| ch.demands[x]).sum::<i32>() + ch.demands[cust];
                if dl > ch.max_capacity { continue; }
                for ins in 1..routes[dst].len() {
                    let ic = dm[routes[dst][ins-1]][cust] + dm[cust][routes[dst][ins]] - dm[routes[dst][ins-1]][routes[dst][ins]];
                    if ic < rg {
                        let mut ns = routes[src].clone(); ns.remove(pos);
                        let mut nd = routes[dst].clone(); nd.insert(ins, cust);
                        if (ns.len() == 2 || is_feasible(&ns, ch)) && is_feasible(&nd, ch) {
                            routes[src] = ns; routes[dst] = nd;
                            routes.retain(|r| r.len() > 2);
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

fn is_feasible(route: &[usize], ch: &Challenge) -> bool {
    let mut time = 0i32;
    let mut load = 0i32;
    for i in 1..route.len() {
        let (prev, curr) = (route[i-1], route[i]);
        let travel = ch.distance_matrix[prev][curr];
        if curr == 0 { return time + travel <= ch.due_times[0]; }
        time += travel;
        if time > ch.due_times[curr] { return false; }
        if time < ch.ready_times[curr] { time = ch.ready_times[curr]; }
        time += ch.service_time;
        load += ch.demands[curr];
        if load > ch.max_capacity { return false; }
    }
    true
}

fn route_dist(route: &[usize], dm: &[Vec<i32>]) -> i32 {
    (1..route.len()).map(|i| dm[route[i-1]][route[i]]).sum()
}

fn total_distance(routes: &[Vec<usize>], dm: &[Vec<i32>]) -> i64 {
    routes.iter().map(|r| route_dist(r, dm) as i64).sum()
}
