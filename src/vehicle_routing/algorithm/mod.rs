use super::*;
use anyhow::Result;
use serde_json::{Map, Value};
use std::time::Instant;

pub fn help() {
    println!("Insertion + 2-opt + or-opt + relocate + exchange for VRPTW");
}

pub fn solve_challenge(
    challenge: &Challenge,
    save_solution: &dyn Fn(&Solution) -> Result<()>,
    _hyperparameters: &Option<Map<String, Value>>,
) -> Result<()> {
    let start = Instant::now();
    let time_limit_ms = 28_000u128;
    let n = challenge.num_nodes;
    let dm = &challenge.distance_matrix;
    let rt = &challenge.ready_times;
    let dt = &challenge.due_times;
    let st = challenge.service_time;
    let demands = &challenge.demands;
    let cap = challenge.max_capacity;

    fn check_route_tw(route: &[usize], dm: &[Vec<i32>], st: i32, rt: &[i32], dt: &[i32]) -> bool {
        let mut time = 0i32;
        for i in 0..route.len() - 1 {
            time += dm[route[i]][route[i + 1]];
            if time > dt[route[i + 1]] {
                return false;
            }
            time = time.max(rt[route[i + 1]]) + st;
        }
        true
    }

    fn route_cost(route: &[usize], dm: &[Vec<i32>]) -> i32 {
        let mut c = 0i32;
        for i in 0..route.len() - 1 {
            c += dm[route[i]][route[i + 1]];
        }
        c
    }

    fn route_demand(route: &[usize], demands: &[i32]) -> i32 {
        route.iter().map(|&n| demands[n]).sum()
    }

    // --- Construction: Solomon I1 insertion heuristic ---
    let mut routes: Vec<Vec<usize>> = Vec::new();
    let mut nodes: Vec<usize> = (1..n).collect();
    nodes.sort_by(|&a, &b| dm[0][a].cmp(&dm[0][b]));
    let mut remaining = vec![true; n];
    remaining[0] = false;

    while let Some(seed) = nodes.pop() {
        if !remaining[seed] {
            continue;
        }
        remaining[seed] = false;
        let mut route = vec![0, seed, 0];
        let mut rd = demands[seed];

        loop {
            let candidates: Vec<usize> = remaining
                .iter()
                .enumerate()
                .filter(|(i, &f)| f && rd + demands[*i] <= cap)
                .map(|(i, _)| i)
                .collect();
            if candidates.is_empty() {
                break;
            }

            let mut best: Option<(usize, usize, i32)> = None;
            for &cand in &candidates {
                let mut time = 0i32;
                let mut prev = 0usize;
                for pos in 1..route.len() {
                    let next = route[pos];
                    let arr = time + dm[prev][cand];
                    if arr <= dt[cand] {
                        let new_arr = arr.max(rt[cand]) + st;
                        let mut t2 = new_arr;
                        let mut feasible = true;
                        let mut prev2 = cand;
                        for p2 in pos..route.len() {
                            t2 += dm[prev2][route[p2]];
                            if t2 > dt[route[p2]] {
                                feasible = false;
                                break;
                            }
                            t2 = t2.max(rt[route[p2]]) + st;
                            prev2 = route[p2];
                        }
                        if feasible {
                            let c1 = dm[prev][cand] + dm[cand][next] - dm[prev][next];
                            let c2 = dm[0][cand] - c1;
                            if best.is_none() || c2 > best.unwrap().2 {
                                best = Some((cand, pos, c2));
                            }
                        }
                    }
                    time = (time + dm[prev][next]).max(rt[next]) + st;
                    prev = next;
                }
            }
            match best {
                Some((node, pos, _)) => {
                    remaining[node] = false;
                    rd += demands[node];
                    route.insert(pos, node);
                }
                None => break,
            }
        }
        routes.push(route);
    }

    let mut total_cost: i64 = routes.iter().map(|r| route_cost(r, dm) as i64).sum();
    save_solution(&Solution { routes: routes.clone() })?;

    // --- Cycle local search operators until no improvement or timeout ---
    let mut global_improved = true;
    while global_improved && start.elapsed().as_millis() < time_limit_ms {
        global_improved = false;

        // Operator 1: intra-route 2-opt
        let mut improved = true;
        while improved && start.elapsed().as_millis() < time_limit_ms {
            improved = false;
            for ri in 0..routes.len() {
                let rlen = routes[ri].len();
                if rlen <= 4 { continue; }
                for i in 1..rlen - 2 {
                    if start.elapsed().as_millis() >= time_limit_ms { break; }
                    for j in (i + 1)..rlen - 1 {
                        let delta = dm[routes[ri][i - 1]][routes[ri][j]]
                            + dm[routes[ri][i]][routes[ri][j + 1]]
                            - dm[routes[ri][i - 1]][routes[ri][i]]
                            - dm[routes[ri][j]][routes[ri][j + 1]];
                        if delta < 0 {
                            let mut cand = routes[ri].clone();
                            cand[i..=j].reverse();
                            if check_route_tw(&cand, dm, st, rt, dt) {
                                routes[ri] = cand;
                                total_cost += delta as i64;
                                improved = true;
                                global_improved = true;
                                save_solution(&Solution { routes: routes.clone() })?;
                                break;
                            }
                        }
                    }
                    if improved { break; }
                }
                if improved { break; }
            }
        }

        // Operator 2: intra-route or-opt (move segments of 1, 2, 3)
        for seg_len in 1..=3 {
            improved = true;
            while improved && start.elapsed().as_millis() < time_limit_ms {
                improved = false;
                for ri in 0..routes.len() {
                    let rlen = routes[ri].len();
                    if rlen <= 3 + seg_len { continue; }
                    'or_opt_outer: for i in 1..rlen - 1 - seg_len + 1 {
                        if start.elapsed().as_millis() >= time_limit_ms { break; }
                        let seg_end = i + seg_len - 1;
                        if seg_end >= rlen - 1 { continue; }
                        let remove_cost = dm[routes[ri][i - 1]][routes[ri][i]]
                            + dm[routes[ri][seg_end]][routes[ri][seg_end + 1]]
                            - dm[routes[ri][i - 1]][routes[ri][seg_end + 1]];
                        for j in 1..rlen - seg_len {
                            if j >= i && j <= seg_end + 1 { continue; }
                            let actual_j = if j > seg_end { j + seg_len - seg_len } else { j };
                            let _ = actual_j;
                            let insert_cost = dm[routes[ri][j - 1]][routes[ri][i]]
                                + dm[routes[ri][seg_end]][routes[ri][j]]
                                - dm[routes[ri][j - 1]][routes[ri][j]];
                            let delta = insert_cost - remove_cost;
                            if delta < 0 {
                                let seg: Vec<usize> = routes[ri][i..=seg_end].to_vec();
                                let mut new_route: Vec<usize> = Vec::with_capacity(rlen);
                                for k in 0..rlen {
                                    if k == i { continue; }
                                    if k > i && k <= seg_end { continue; }
                                    if k == j && j < i {
                                        for &s in &seg { new_route.push(s); }
                                    }
                                    new_route.push(routes[ri][k]);
                                    if k == j - 1 && j > seg_end {
                                        for &s in &seg { new_route.push(s); }
                                    }
                                }
                                if new_route.len() == rlen && check_route_tw(&new_route, dm, st, rt, dt) {
                                    total_cost += (route_cost(&new_route, dm) - route_cost(&routes[ri], dm)) as i64;
                                    routes[ri] = new_route;
                                    improved = true;
                                    global_improved = true;
                                    save_solution(&Solution { routes: routes.clone() })?;
                                    break 'or_opt_outer;
                                }
                            }
                        }
                    }
                    if improved { break; }
                }
            }
        }

        // Operator 3: inter-route relocate
        improved = true;
        while improved && start.elapsed().as_millis() < time_limit_ms {
            improved = false;
            'reloc: for ri in 0..routes.len() {
                for ci in 1..routes[ri].len() - 1 {
                    if start.elapsed().as_millis() >= time_limit_ms { break 'reloc; }
                    let cust = routes[ri][ci];
                    let remove_save = dm[routes[ri][ci - 1]][cust]
                        + dm[cust][routes[ri][ci + 1]]
                        - dm[routes[ri][ci - 1]][routes[ri][ci + 1]];

                    let mut best_gain = 0i32;
                    let mut best_target: Option<(usize, usize)> = None;

                    for rj in 0..routes.len() {
                        if ri == rj { continue; }
                        if route_demand(&routes[rj], demands) + demands[cust] > cap { continue; }
                        for pos in 1..routes[rj].len() {
                            let insert_cost = dm[routes[rj][pos - 1]][cust]
                                + dm[cust][routes[rj][pos]]
                                - dm[routes[rj][pos - 1]][routes[rj][pos]];
                            let gain = remove_save - insert_cost;
                            if gain > best_gain {
                                let mut cand = routes[rj].clone();
                                cand.insert(pos, cust);
                                if check_route_tw(&cand, dm, st, rt, dt) {
                                    best_gain = gain;
                                    best_target = Some((rj, pos));
                                }
                            }
                        }
                    }

                    if let Some((rj, pos)) = best_target {
                        routes[ri].remove(ci);
                        routes[rj].insert(pos, cust);
                        if routes[ri].len() <= 2 {
                            routes.remove(ri);
                        }
                        total_cost -= best_gain as i64;
                        improved = true;
                        global_improved = true;
                        save_solution(&Solution { routes: routes.clone() })?;
                        break 'reloc;
                    }
                }
            }
        }

        // Operator 4: inter-route exchange (swap two customers between routes)
        improved = true;
        while improved && start.elapsed().as_millis() < time_limit_ms {
            improved = false;
            'exch: for ri in 0..routes.len() {
                for ci in 1..routes[ri].len() - 1 {
                    if start.elapsed().as_millis() >= time_limit_ms { break 'exch; }
                    let c1 = routes[ri][ci];
                    for rj in (ri + 1)..routes.len() {
                        for cj in 1..routes[rj].len() - 1 {
                            let c2 = routes[rj][cj];
                            let dem_diff_ri = demands[c2] - demands[c1];
                            let dem_diff_rj = demands[c1] - demands[c2];
                            if route_demand(&routes[ri], demands) + dem_diff_ri > cap { continue; }
                            if route_demand(&routes[rj], demands) + dem_diff_rj > cap { continue; }

                            let old_ri = dm[routes[ri][ci - 1]][c1] + dm[c1][routes[ri][ci + 1]];
                            let new_ri = dm[routes[ri][ci - 1]][c2] + dm[c2][routes[ri][ci + 1]];
                            let old_rj = dm[routes[rj][cj - 1]][c2] + dm[c2][routes[rj][cj + 1]];
                            let new_rj = dm[routes[rj][cj - 1]][c1] + dm[c1][routes[rj][cj + 1]];
                            let delta = (new_ri - old_ri) + (new_rj - old_rj);
                            if delta < 0 {
                                let mut ri_cand = routes[ri].clone();
                                let mut rj_cand = routes[rj].clone();
                                ri_cand[ci] = c2;
                                rj_cand[cj] = c1;
                                if check_route_tw(&ri_cand, dm, st, rt, dt)
                                    && check_route_tw(&rj_cand, dm, st, rt, dt)
                                {
                                    routes[ri] = ri_cand;
                                    routes[rj] = rj_cand;
                                    total_cost += delta as i64;
                                    improved = true;
                                    global_improved = true;
                                    save_solution(&Solution { routes: routes.clone() })?;
                                    break 'exch;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
