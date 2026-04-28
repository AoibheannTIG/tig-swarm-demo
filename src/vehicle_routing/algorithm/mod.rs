use super::*;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use rand::{rngs::SmallRng, Rng, SeedableRng};

pub fn help() {
    // Print help information about your algorithm here. It will be invoked with `help_algorithm` script
    println!("Greedy nearest-neighbor heuristic for VRPTW");
}

pub fn solve_challenge(
    challenge: &Challenge,
    save_solution: &dyn Fn(&Solution) -> Result<()>,
    _hyperparameters: &Option<Map<String, Value>>,
) -> Result<()> {

    #[derive(Serialize, Deserialize)]
    pub struct Hyperparameters {
        // Optionally define hyperparameters here
    }
    fn is_feasible(
        route: &Vec<usize>,
        distance_matrix: &Vec<Vec<i32>>,
        service_time: i32,
        ready_times: &Vec<i32>,
        due_times: &Vec<i32>,
        mut curr_node: usize,
        mut curr_time: i32,
        start_pos: usize,
    ) -> bool {
        let mut valid = true;
        for pos in start_pos..route.len() {
            let next_node = route[pos];
            curr_time += distance_matrix[curr_node][next_node];
            if curr_time > due_times[route[pos]] {
                valid = false;
                break;
            }
            curr_time = curr_time.max(ready_times[next_node]) + service_time;
            curr_node = next_node;
        }
        valid
    }
    
    fn find_best_insertion(
        route: &Vec<usize>,
        remaining_nodes: Vec<usize>,
        distance_matrix: &Vec<Vec<i32>>,
        service_time: i32,
        ready_times: &Vec<i32>,
        due_times: &Vec<i32>,
    ) -> Option<(usize, usize)> {
        let alpha1 = 1;
        let alpha2 = 0;
        let lambda = 1;
    
        let mut best_c2 = None;
        let mut best = None;
        for insert_node in remaining_nodes {
            let mut best_c1 = None;
    
            let mut curr_time = 0;
            let mut curr_node = 0;
            for pos in 1..route.len() {
                let next_node = route[pos];
                let new_arrival_time =
                    ready_times[insert_node].max(curr_time + distance_matrix[curr_node][insert_node]);
                if new_arrival_time > due_times[insert_node] {
                    continue;
                }
                let old_arrival_time =
                    ready_times[next_node].max(curr_time + distance_matrix[curr_node][next_node]);
    
                // Distance criterion: c11 = d(i,u) + d(u,j) - mu * d(i,j)
                let c11 = distance_matrix[curr_node][insert_node]
                    + distance_matrix[insert_node][next_node]
                    - distance_matrix[curr_node][next_node];
    
                // Time criterion: c12 = b_ju - b_j (the shift in arrival time at position 'pos').
                let c12 = new_arrival_time - old_arrival_time;
    
                let c1 = -(alpha1 * c11 + alpha2 * c12);
                let c2 = lambda * distance_matrix[0][insert_node] + c1;
    
                if best_c1.is_none_or(|x| c1 > x)
                    && best_c2.is_none_or(|x| c2 > x)
                    && is_feasible(
                        route,
                        distance_matrix,
                        service_time,
                        ready_times,
                        due_times,
                        insert_node,
                        new_arrival_time + service_time,
                        pos,
                    )
                {
                    best_c1 = Some(c1);
                    best_c2 = Some(c2);
                    best = Some((insert_node, pos));
                }
    
                curr_time = ready_times[next_node]
                    .max(curr_time + distance_matrix[curr_node][next_node])
                    + service_time;
                curr_node = next_node;
            }
        }
        best
    }
    
    let mut routes = Vec::new();

    let mut nodes: Vec<usize> = (1..challenge.num_nodes).collect();
    nodes.sort_by(|&a, &b| challenge.distance_matrix[0][a].cmp(&challenge.distance_matrix[0][b]));

    let mut remaining: Vec<bool> = vec![true; challenge.num_nodes];
    remaining[0] = false;

    // popping furthest node from depot
    while let Some(node) = nodes.pop() {
        if !remaining[node] {
            continue;
        }
        remaining[node] = false;
        let mut route = vec![0, node, 0];
        let mut route_demand = challenge.demands[node];

        while let Some((best_node, best_pos)) = find_best_insertion(
            &route,
            remaining
                .iter()
                .enumerate()
                .filter(|(n, &flag)| {
                    flag && route_demand + challenge.demands[*n] <= challenge.max_capacity
                })
                .map(|(n, _)| n)
                .collect(),
            &challenge.distance_matrix,
            challenge.service_time,
            &challenge.ready_times,
            &challenge.due_times,
        ) {
            remaining[best_node] = false;
            route_demand += challenge.demands[best_node];
            route.insert(best_pos, best_node);
        }

        routes.push(route);
    }

    fn route_feasible(
        route: &Vec<usize>,
        dm: &Vec<Vec<i32>>,
        service_time: i32,
        ready_times: &Vec<i32>,
        due_times: &Vec<i32>,
    ) -> bool {
        let mut t: i32 = 0;
        let mut prev: usize = route[0];
        for k in 1..route.len() {
            let n = route[k];
            t = (t + dm[prev][n]).max(ready_times[n]);
            if t > due_times[n] {
                return false;
            }
            t += service_time;
            prev = n;
        }
        true
    }

    save_solution(&Solution { routes: routes.clone() })?;

    let dm = &challenge.distance_matrix;
    let st = challenge.service_time;
    let rt = &challenge.ready_times;
    let dt = &challenge.due_times;
    let mut improved_any = true;
    while improved_any {
        improved_any = false;
        for r in 0..routes.len() {
            if routes[r].len() < 5 {
                continue;
            }
            let mut local = true;
            while local {
                local = false;
                let n = routes[r].len();
                let mut best_delta: i64 = 0;
                let mut best_ij: Option<(usize, usize)> = None;
                for i in 0..n - 3 {
                    for j in i + 2..n - 1 {
                        let a = routes[r][i];
                        let b = routes[r][i + 1];
                        let c = routes[r][j];
                        let d = routes[r][j + 1];
                        let delta = (dm[a][c] as i64 + dm[b][d] as i64)
                            - (dm[a][b] as i64 + dm[c][d] as i64);
                        if delta < best_delta {
                            let mut cand = routes[r].clone();
                            cand[i + 1..=j].reverse();
                            if route_feasible(&cand, dm, st, rt, dt) {
                                best_delta = delta;
                                best_ij = Some((i, j));
                            }
                        }
                    }
                }
                if let Some((i, j)) = best_ij {
                    routes[r][i + 1..=j].reverse();
                    local = true;
                    improved_any = true;
                }
            }
        }
        if improved_any {
            save_solution(&Solution { routes: routes.clone() })?;
        }
    }

    Ok(())
}



