use super::*;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use rand::{rngs::SmallRng, Rng, SeedableRng};
use rand::prelude::{SliceRandom, IteratorRandom};


pub fn solve_challenge(
    challenge: &Challenge,
    save_solution: &dyn Fn(&Solution) -> Result<()>,
    _hyperparameters: &Option<Map<String, Value>>,
) -> Result<()> {
    // EVOLVE-BLOCK-START   
     
    // #[derive(Serialize, Deserialize)]
    // pub struct Hyperparameters {
    //     Optionally define hyperparameters here
    // }
    
    // Simple greedy: sort by value/weight ratio (ignoring interactions)
    let num_items = challenge.values.len();

    let mut items: Vec<usize> = (0..num_items).collect();
    items.sort_by(|&a, &b| {
        let ratio_a = challenge.values[a] as f64 / challenge.weights[a] as f64;
        let ratio_b = challenge.values[b] as f64 / challenge.weights[b] as f64;
        ratio_b.partial_cmp(&ratio_a).unwrap()
    });
    
    let mut selected = Vec::new();
    let mut total_weight: u32 = 0;
    
    for item_idx in items {
        if total_weight + challenge.weights[item_idx] <= challenge.max_weight {
            selected.push(item_idx);
            total_weight += challenge.weights[item_idx];
        }
    }
    // EVOLVE-BLOCK-END
    
    save_solution(&Solution { items: selected })?;
    Ok(())
}