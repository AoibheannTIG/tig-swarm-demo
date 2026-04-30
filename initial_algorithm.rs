// initial_algorithm.rs
//
// The starting algorithm broadcast to every agent on a fresh trajectory:
// the agent's first iteration, and again whenever a trajectory reset
// draws the "fresh start" slot from the inactive-algorithms pool.
//
// Edit this file before running `python setup.py create` to provide a
// custom starter. Left unchanged, agents start from this stub and must
// author the body themselves before they can produce a feasible
// solution.
//
// The signature below is uniform across all five supported challenges;
// the concrete shapes of `Challenge` and `Solution` come from the
// active challenge's module via `super::*`. See CHALLENGE.md for the
// per-challenge type shapes, scoring rules, and tips.

use super::*;
use anyhow::Result;
use serde_json::{Map, Value};

pub fn solve_challenge(
    _challenge: &Challenge,
    _save_solution: &dyn Fn(&Solution) -> Result<()>,
    _hyperparameters: &Option<Map<String, Value>>,
) -> Result<()> {
    // TODO: build a Solution for `_challenge` and call `_save_solution(&sol)`.
    // `_save_solution` can be called multiple times; only the most recent
    // call is kept, so save your best in-progress solution as you find
    // improvements (the solver has a hard timeout — see CLAUDE.md).
    unimplemented!("initial algorithm not yet implemented for this swarm");
}
