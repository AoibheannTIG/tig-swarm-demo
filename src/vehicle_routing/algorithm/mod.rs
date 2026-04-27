use super::*;
use anyhow::Result;
use rand::{rngs::SmallRng, Rng, SeedableRng};
use serde_json::{Map, Value};
use std::time::Instant;

pub fn help() {
    println!("ALNS with simulated annealing for VRPTW");
}

pub fn solve_challenge(
    challenge: &Challenge,
    save_solution: &dyn Fn(&Solution) -> Result<()>,
    _hyperparameters: &Option<Map<String, Value>>,
) -> Result<()> {
    let start = Instant::now();
    let time_limit_ms = 27_500u128;
    let n = challenge.num_nodes;
    let dm = &challenge.distance_matrix;
    let rt = &challenge.ready_times;
    let dt = &challenge.due_times;
    let st = challenge.service_time;
    let demands = &challenge.demands;
    let cap = challenge.max_capacity;
    let fleet = challenge.fleet_size;
    let mut rng = SmallRng::seed_from_u64(n as u64 * 31337 + 42);

    fn check_tw(route: &[usize], dm: &[Vec<i32>], st: i32, rt: &[i32], dt: &[i32]) -> bool {
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

    fn route_cost(route: &[usize], dm: &[Vec<i32>]) -> i64 {
        let mut c = 0i64;
        for i in 0..route.len() - 1 {
            c += dm[route[i]][route[i + 1]] as i64;
        }
        c
    }

    fn solution_cost(routes: &[Vec<usize>], dm: &[Vec<i32>]) -> i64 {
        routes.iter().map(|r| route_cost(r, dm)).sum()
    }

    fn route_demand(route: &[usize], demands: &[i32]) -> i32 {
        route.iter().map(|&node| demands[node]).sum()
    }

    // ===== Construction: Solomon I1 insertion heuristic =====
    let mut routes: Vec<Vec<usize>> = Vec::new();
    let mut node_order: Vec<usize> = (1..n).collect();
    node_order.sort_by(|&a, &b| dm[0][a].cmp(&dm[0][b]));
    let mut unrouted = vec![true; n];
    unrouted[0] = false;

    while let Some(seed_node) = node_order.pop() {
        if !unrouted[seed_node] {
            continue;
        }
        unrouted[seed_node] = false;
        let mut route = vec![0, seed_node, 0];
        let mut load = demands[seed_node];

        loop {
            let mut best: Option<(usize, usize, i32)> = None;
            for node in 0..n {
                if !unrouted[node] || load + demands[node] > cap {
                    continue;
                }
                let mut t = 0i32;
                for pos in 1..route.len() {
                    let prev = route[pos - 1];
                    let next = route[pos];
                    let arr = t + dm[prev][node];
                    if arr > dt[node] {
                        t = (t + dm[prev][next]).max(rt[next]) + st;
                        continue;
                    }
                    let mut t2 = arr.max(rt[node]) + st;
                    let mut ok = true;
                    let mut p2 = node;
                    for k in pos..route.len() {
                        t2 += dm[p2][route[k]];
                        if t2 > dt[route[k]] {
                            ok = false;
                            break;
                        }
                        t2 = t2.max(rt[route[k]]) + st;
                        p2 = route[k];
                    }
                    if ok {
                        let c1 = dm[prev][node] + dm[node][next] - dm[prev][next];
                        let c2 = dm[0][node] - c1;
                        if best.is_none() || c2 > best.unwrap().2 {
                            best = Some((node, pos, c2));
                        }
                    }
                    t = (t + dm[prev][next]).max(rt[next]) + st;
                }
            }
            match best {
                Some((node, pos, _)) => {
                    unrouted[node] = false;
                    load += demands[node];
                    route.insert(pos, node);
                }
                None => break,
            }
        }
        routes.push(route);
    }

    // Post-construction: merge excess routes to respect fleet_size
    while routes.len() > fleet {
        let min_ri = routes
            .iter()
            .enumerate()
            .min_by_key(|(_, r)| r.len())
            .map(|(i, _)| i)
            .unwrap();
        let small_route = routes.remove(min_ri);
        let customers: Vec<usize> = small_route[1..small_route.len() - 1].to_vec();
        let mut failed = false;
        for cu in customers {
            let mut best_ic = i32::MAX;
            let mut best_rj = 0;
            let mut best_p = 0;
            let mut found = false;
            for (rj, route) in routes.iter().enumerate() {
                if route_demand(route, demands) + demands[cu] > cap {
                    continue;
                }
                for p in 1..route.len() {
                    let ic =
                        dm[route[p - 1]][cu] + dm[cu][route[p]] - dm[route[p - 1]][route[p]];
                    if ic < best_ic {
                        let mut cand = route.clone();
                        cand.insert(p, cu);
                        if check_tw(&cand, dm, st, rt, dt) {
                            best_ic = ic;
                            best_rj = rj;
                            best_p = p;
                            found = true;
                        }
                    }
                }
            }
            if found {
                routes[best_rj].insert(best_p, cu);
            } else {
                routes.push(vec![0, cu, 0]);
                failed = true;
            }
        }
        if failed {
            break;
        }
    }

    let mut best_cost = solution_cost(&routes, dm);
    let mut best_routes = routes.clone();
    save_solution(&Solution {
        routes: routes.clone(),
    })?;

    // ===== Local Search =====
    fn do_ls(
        routes: &mut Vec<Vec<usize>>,
        dm: &[Vec<i32>],
        st: i32,
        rt: &[i32],
        dt: &[i32],
        demands: &[i32],
        cap: i32,
        start: &Instant,
        deadline: u128,
    ) {
        let mut any = true;
        while any && start.elapsed().as_millis() < deadline {
            any = false;

            // 2-opt (intra-route)
            let mut imp = true;
            while imp && start.elapsed().as_millis() < deadline {
                imp = false;
                'two_opt: for ri in 0..routes.len() {
                    let rlen = routes[ri].len();
                    if rlen <= 4 {
                        continue;
                    }
                    for i in 1..rlen - 2 {
                        if start.elapsed().as_millis() >= deadline {
                            return;
                        }
                        for j in (i + 1)..rlen - 1 {
                            let d = dm[routes[ri][i - 1]][routes[ri][j]]
                                + dm[routes[ri][i]][routes[ri][j + 1]]
                                - dm[routes[ri][i - 1]][routes[ri][i]]
                                - dm[routes[ri][j]][routes[ri][j + 1]];
                            if d < 0 {
                                let mut c = routes[ri].clone();
                                c[i..=j].reverse();
                                if check_tw(&c, dm, st, rt, dt) {
                                    routes[ri] = c;
                                    imp = true;
                                    any = true;
                                    break 'two_opt;
                                }
                            }
                        }
                    }
                }
            }

            // Or-opt: intra-route single-customer relocate
            imp = true;
            while imp && start.elapsed().as_millis() < deadline {
                imp = false;
                'oropt: for ri in 0..routes.len() {
                    let rlen = routes[ri].len();
                    if rlen <= 3 {
                        continue;
                    }
                    for i in 1..rlen - 1 {
                        if start.elapsed().as_millis() >= deadline {
                            return;
                        }
                        let cu = routes[ri][i];
                        let savings = dm[routes[ri][i - 1]][cu] + dm[cu][routes[ri][i + 1]]
                            - dm[routes[ri][i - 1]][routes[ri][i + 1]];
                        let mut nr: Vec<usize> = Vec::with_capacity(rlen);
                        for k in 0..rlen {
                            if k != i {
                                nr.push(routes[ri][k]);
                            }
                        }
                        for j in 1..nr.len() {
                            let ic = dm[nr[j - 1]][cu] + dm[cu][nr[j]] - dm[nr[j - 1]][nr[j]];
                            if ic < savings {
                                let mut cand = nr.clone();
                                cand.insert(j, cu);
                                if check_tw(&cand, dm, st, rt, dt) {
                                    routes[ri] = cand;
                                    imp = true;
                                    any = true;
                                    break 'oropt;
                                }
                            }
                        }
                    }
                }
            }

            // Relocate (inter-route)
            imp = true;
            while imp && start.elapsed().as_millis() < deadline {
                imp = false;
                'rel: for ri in 0..routes.len() {
                    for ci in 1..routes[ri].len() - 1 {
                        if start.elapsed().as_millis() >= deadline {
                            return;
                        }
                        let cu = routes[ri][ci];
                        let sv = dm[routes[ri][ci - 1]][cu] + dm[cu][routes[ri][ci + 1]]
                            - dm[routes[ri][ci - 1]][routes[ri][ci + 1]];
                        let mut bg = 0i32;
                        let mut bt: Option<(usize, usize)> = None;
                        for rj in 0..routes.len() {
                            if ri == rj {
                                continue;
                            }
                            if route_demand(&routes[rj], demands) + demands[cu] > cap {
                                continue;
                            }
                            for p in 1..routes[rj].len() {
                                let ic = dm[routes[rj][p - 1]][cu] + dm[cu][routes[rj][p]]
                                    - dm[routes[rj][p - 1]][routes[rj][p]];
                                if sv - ic > bg {
                                    let mut cand = routes[rj].clone();
                                    cand.insert(p, cu);
                                    if check_tw(&cand, dm, st, rt, dt) {
                                        bg = sv - ic;
                                        bt = Some((rj, p));
                                    }
                                }
                            }
                        }
                        if let Some((rj, p)) = bt {
                            routes[ri].remove(ci);
                            routes[rj].insert(p, cu);
                            if routes[ri].len() <= 2 {
                                routes.remove(ri);
                            }
                            imp = true;
                            any = true;
                            break 'rel;
                        }
                    }
                }
            }

            // Exchange (inter-route swap)
            imp = true;
            while imp && start.elapsed().as_millis() < deadline {
                imp = false;
                'exc: for ri in 0..routes.len() {
                    for ci in 1..routes[ri].len() - 1 {
                        if start.elapsed().as_millis() >= deadline {
                            return;
                        }
                        let c1 = routes[ri][ci];
                        for rj in (ri + 1)..routes.len() {
                            for cj in 1..routes[rj].len() - 1 {
                                let c2 = routes[rj][cj];
                                if route_demand(&routes[ri], demands) + demands[c2] - demands[c1]
                                    > cap
                                {
                                    continue;
                                }
                                if route_demand(&routes[rj], demands) + demands[c1] - demands[c2]
                                    > cap
                                {
                                    continue;
                                }
                                let delta = dm[routes[ri][ci - 1]][c2]
                                    + dm[c2][routes[ri][ci + 1]]
                                    - dm[routes[ri][ci - 1]][c1]
                                    - dm[c1][routes[ri][ci + 1]]
                                    + dm[routes[rj][cj - 1]][c1]
                                    + dm[c1][routes[rj][cj + 1]]
                                    - dm[routes[rj][cj - 1]][c2]
                                    - dm[c2][routes[rj][cj + 1]];
                                if delta < 0 {
                                    let mut rc = routes[ri].clone();
                                    let mut rjc = routes[rj].clone();
                                    rc[ci] = c2;
                                    rjc[cj] = c1;
                                    if check_tw(&rc, dm, st, rt, dt)
                                        && check_tw(&rjc, dm, st, rt, dt)
                                    {
                                        routes[ri] = rc;
                                        routes[rj] = rjc;
                                        imp = true;
                                        any = true;
                                        break 'exc;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Initial local search
    do_ls(
        &mut routes,
        dm,
        st,
        rt,
        dt,
        demands,
        cap,
        &start,
        time_limit_ms,
    );
    let c = solution_cost(&routes, dm);
    if c < best_cost && routes.len() <= fleet {
        best_cost = c;
        best_routes = routes.clone();
        save_solution(&Solution {
            routes: routes.clone(),
        })?;
    }

    // ===== ALNS: Destroy + Repair with Simulated Annealing =====
    let mut temp = best_cost as f64 * 0.03;
    let cool = 0.97;
    let mut cur_cost = solution_cost(&routes, dm);

    while start.elapsed().as_millis() < time_limit_ms {
        let mut tr = routes.clone();
        let num_cust: usize = tr.iter().map(|r| r.len().saturating_sub(2)).sum();
        if num_cust < 4 {
            break;
        }
        let lo = (num_cust / 8).max(3);
        let hi = (num_cust * 3 / 10).max(lo);
        let k = rng.gen_range(lo..=hi).min(num_cust);

        // Destroy: choose between random, worst, or Shaw removal
        let mut removed = Vec::new();
        let mut removed_set = vec![false; n];
        let destroy_choice = rng.gen_range(0u32..10);

        let remove_node = |cu: usize, tr: &mut Vec<Vec<usize>>| {
            for route in tr.iter_mut() {
                if let Some(pos) = route[1..route.len() - 1].iter().position(|&x| x == cu) {
                    route.remove(pos + 1);
                    return;
                }
            }
        };

        if destroy_choice < 4 {
            // Shaw removal: remove geographically related customers
            let all_custs: Vec<usize> = tr
                .iter()
                .flat_map(|r| r[1..r.len() - 1].iter().copied())
                .collect();
            if all_custs.is_empty() {
                break;
            }
            let seed = all_custs[rng.gen_range(0..all_custs.len())];
            let mut related: Vec<(usize, i32)> = all_custs
                .iter()
                .filter(|&&c| c != seed)
                .map(|&c| (c, dm[seed][c] + (rt[seed] - rt[c]).abs() / 2))
                .collect();
            related.sort_by_key(|&(_, r)| r);

            removed_set[seed] = true;
            removed.push(seed);
            remove_node(seed, &mut tr);

            for _ in 1..k {
                let remaining: Vec<usize> = related
                    .iter()
                    .filter(|&&(c, _)| !removed_set[c])
                    .map(|&(c, _)| c)
                    .collect();
                if remaining.is_empty() {
                    break;
                }
                let p = rng.gen::<f64>().powi(3);
                let idx = (p * remaining.len() as f64) as usize % remaining.len();
                let cu = remaining[idx];
                removed_set[cu] = true;
                removed.push(cu);
                remove_node(cu, &mut tr);
            }
        } else {
            // Worst removal (destroy_choice 4-7) or random removal (8-9)
            let use_worst = destroy_choice < 8;
            for _ in 0..k {
                let mut cands: Vec<(usize, i32)> = Vec::new();
                for route in tr.iter() {
                    for ci in 1..route.len() - 1 {
                        let cu = route[ci];
                        if removed_set[cu] {
                            continue;
                        }
                        let sv = dm[route[ci - 1]][cu] + dm[cu][route[ci + 1]]
                            - dm[route[ci - 1]][route[ci + 1]];
                        cands.push((cu, sv));
                    }
                }
                if cands.is_empty() {
                    break;
                }

                let idx = if use_worst {
                    cands.sort_by(|a, b| b.1.cmp(&a.1));
                    let p = rng.gen::<f64>().powi(3);
                    (p * cands.len() as f64) as usize % cands.len()
                } else {
                    rng.gen_range(0..cands.len())
                };
                let (cu, _) = cands[idx];
                removed_set[cu] = true;
                removed.push(cu);
                remove_node(cu, &mut tr);
            }
        }
        tr.retain(|r| r.len() > 2);

        // Repair: regret-2 insertion (NO time limit — must reinsert all nodes)
        while !removed.is_empty() {
            let mut best_ri = 0usize;
            let mut best_route_idx = 0usize;
            let mut best_pos = 0usize;
            let mut best_regret = i32::MIN;
            let mut found = false;

            for (ri, &cu) in removed.iter().enumerate() {
                let mut b1_cost = i32::MAX;
                let mut b1_route = 0usize;
                let mut b1_pos = 0usize;
                let mut b2_cost = i32::MAX;

                for (rj, route) in tr.iter().enumerate() {
                    if route_demand(route, demands) + demands[cu] > cap {
                        continue;
                    }
                    for p in 1..route.len() {
                        let ic = dm[route[p - 1]][cu] + dm[cu][route[p]]
                            - dm[route[p - 1]][route[p]];
                        if ic < b1_cost || ic < b2_cost {
                            let mut cand = route.clone();
                            cand.insert(p, cu);
                            if check_tw(&cand, dm, st, rt, dt) {
                                if ic < b1_cost {
                                    b2_cost = b1_cost;
                                    b1_cost = ic;
                                    b1_route = rj;
                                    b1_pos = p;
                                } else {
                                    b2_cost = ic;
                                }
                            }
                        }
                    }
                }

                if b1_cost < i32::MAX {
                    let regret = if b2_cost < i32::MAX {
                        b2_cost - b1_cost
                    } else {
                        10000
                    };
                    if regret > best_regret {
                        best_regret = regret;
                        best_ri = ri;
                        best_route_idx = b1_route;
                        best_pos = b1_pos;
                        found = true;
                    }
                }
            }

            if found {
                let cu = removed.remove(best_ri);
                tr[best_route_idx].insert(best_pos, cu);
            } else {
                for cu in removed.drain(..) {
                    tr.push(vec![0, cu, 0]);
                }
            }
        }

        // Quick local search on repaired solution
        let ls_dl = (start.elapsed().as_millis() + 1500).min(time_limit_ms);
        do_ls(&mut tr, dm, st, rt, dt, demands, cap, &start, ls_dl);

        // Feasibility gate: skip if too many routes or missing nodes
        if tr.len() > fleet {
            temp *= cool;
            continue;
        }
        {
            let mut ok = true;
            let mut visited = vec![false; n];
            visited[0] = true;
            for route in &tr {
                for &node in &route[1..route.len() - 1] {
                    visited[node] = true;
                }
            }
            if !visited.iter().all(|&v| v) {
                ok = false;
            }
            if !ok {
                temp *= cool;
                continue;
            }
        }

        let new_cost = solution_cost(&tr, dm);
        let delta = new_cost - cur_cost;

        let accept = if delta < 0 {
            true
        } else if temp > 1.0 {
            rng.gen::<f64>() < (-(delta as f64) / temp).exp()
        } else {
            false
        };

        if accept {
            routes = tr;
            cur_cost = new_cost;
            if cur_cost < best_cost {
                best_cost = cur_cost;
                best_routes = routes.clone();
                save_solution(&Solution {
                    routes: routes.clone(),
                })?;
            }
        }
        temp *= cool;
    }

    save_solution(&Solution {
        routes: best_routes,
    })?;
    Ok(())
}

