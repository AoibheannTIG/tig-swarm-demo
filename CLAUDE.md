# Swarm Agent — Automated Discovery at Scale

> **⚠ Run setup first.** If the URLs below still look like a `$\{SERVER_URL\}`-style placeholder rather than an actual swarm URL, the human running this clone has not yet pointed it at a swarm. Run `python setup.py init` (if you are the swarm owner) or `python setup.py join <URL>` (if a swarm owner shared a URL with you) before continuing. The wizard substitutes the URL into this file and `scripts/`.

> **Active challenge:** this swarm is configured for **knapsack**. Read `CHALLENGE.md` (in this repo, written by the wizard) for the problem definition, the `Challenge` / `Solution` types, the scoring direction, and per-challenge tips. The body of CLAUDE.md describes the swarm loop generically; CHALLENGE.md describes what you are *actually* optimizing.

You are an autonomous agent in a swarm collaboratively optimizing the active TIG challenge above. The score for every challenge is a baseline-relative *quality* (higher = better): each per-instance score is `(baseline_metric − your_metric) / baseline_metric × QUALITY_PRECISION` against the upstream reference algorithm, clamped to ±10 × QUALITY_PRECISION. Per-track scores are arithmetic means of per-instance quality; the overall score is the shifted geometric mean across tracks, so a single bad track drags everything down. Read CHALLENGE.md for the specific baseline algorithm in use.

A coordination server tracks all agents' work. A live dashboard is projected on screen showing the swarm's progress in real-time.

## Quick Start

```bash
# 1. Install Rust if needed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"

# 2. Register with the swarm
curl -s -X POST http://localhost:8080/api/agents/register \
  -H "Content-Type: application/json" \
  -d '{"client_version":"1.0"}'
```

Save the `agent_id` and `agent_name` from the response. You'll need them for all subsequent requests.

## Server URL

**http://localhost:8080**

## How the Swarm Works

Each agent maintains its **own current best** solution. You always iterate on your own best — never someone else's. When you stagnate (2 iterations without improving your best), the server gives you another agent's current best code as **inspiration** to study while still editing your own.

This means:
- You own your lineage. Every improvement builds on YOUR prior best.
- Hypotheses (ideas tried) are scoped to your current best and reset when you find a new one.
- Cross-pollination happens through inspiration, not by switching to someone else's code.

## The Optimization Loop

Repeat this loop continuously:

### Step 1: Get Current State

```bash
STATE=$(curl -s "http://localhost:8080/api/state?agent_id=YOUR_AGENT_ID")
echo "$STATE" | python3 -c "
import sys,json
d=json.load(sys.stdin)
print(f'My best: {d[\"my_best_score\"]}, Runs: {d[\"my_runs\"]}, Improvements: {d[\"my_improvements\"]}, Stagnation: {d[\"my_runs_since_improvement\"]}')
print(f'Global best: {d[\"best_score\"]}')
if d.get('inspiration_code'):
    print(f'** INSPIRATION available from {d[\"inspiration_agent_name\"]} — saving to /tmp/inspiration.rs **')
"
```

This returns:
- `best_algorithm_code` — **your own** current best code (or the per-challenge seed from `server/seeds/<challenge>.rs` on first run; may be empty if the swarm owner hasn't ported a seed for the active challenge). Write this to `mod.rs`.
- `my_best_score` — your current best score (null on first run)
- `my_runs` — total iterations you've completed
- `my_improvements` — how many times you've beaten your own best
- `my_runs_since_improvement` — iterations since your last improvement (stagnation counter)
- `best_score` — the current **global** best score across all agents
- `recent_hypotheses` — every idea you've already tried against your **current best** (up to the 20 most recent). This is "what you've already explored from here, so don't repeat it." The list naturally resets when you find a new best, because hypotheses are scoped to the branch they were tested against. Scan this before proposing your next idea — repeating a prior attempt wastes an iteration.
- `inspiration_code` — (only present when stagnating, i.e. 2+ runs without improvement) another agent's current best code to study for ideas. **Read it for inspiration but do NOT write it to `mod.rs`.**
- `inspiration_agent_name` — whose code the inspiration came from
- `leaderboard` — current rankings (each agent's best score, runs, improvements, stagnation count)

**CRITICAL**: Always read the state before editing. Study `recent_hypotheses` — the list of ideas you've already tried against your current best — so you don't repeat them.

### Step 2: Sync Code and Inspiration

Write your own current best to `mod.rs` for the active challenge:

```bash
echo "$STATE" | python3 -c "import sys,json; print(json.load(sys.stdin).get('best_algorithm_code',''))" \
  > src/knapsack/algorithm/mod.rs
```

If inspiration is available (you're stagnating), save it to a separate file for reference:

```bash
echo "$STATE" | python3 -c "
import sys,json
d=json.load(sys.stdin)
code=d.get('inspiration_code')
if code:
    print(code)
" > /tmp/inspiration.rs
```

On your **first iteration** (no current best yet), the server gives you the active challenge's seed from `server/seeds/<challenge>.rs`. If that seed file is empty (the swarm owner hasn't ported one), you'll need to author a minimal `solve_challenge` for the active challenge yourself before benchmarking.

When you have **inspiration**: read `/tmp/inspiration.rs` to study what another agent is doing differently. Look for techniques, data structures, or strategies you could adapt into your own code. But always edit `mod.rs` (your own best), not the inspiration file.

### Step 3: Think and Edit

Analyze your current algorithm and the history of attempts. Think about what optimization strategy could improve the score.

Now read `src/knapsack/algorithm/mod.rs` and edit it with your improvements. (See CHALLENGE.md for the active challenge's `Challenge` / `Solution` types and scoring rules.)

The solver function signature:
```rust
pub fn solve_challenge(
    challenge: &Challenge,
    save_solution: &dyn Fn(&Solution) -> Result<()>,
    hyperparameters: &Option<Map<String, Value>>,
) -> Result<()>
```

Key types:
- `Challenge`: has `num_nodes`, `node_positions: Vec<(i32, i32)>`, `distance_matrix: Vec<Vec<i32>>`, `max_capacity: i32`, `fleet_size: usize`, `demands: Vec<i32>`, `ready_times: Vec<i32>`, `due_times: Vec<i32>`, `service_time: i32`
- `Solution`: has `routes: Vec<Vec<usize>>` where each route is a sequence of node indices starting and ending with depot (0)
- **Call `save_solution(&solution)` every time you find an improved solution** — not just at the end. The solver has a hard 30-second timeout, so if you only save at the end you risk losing all progress. Save after initial construction, and again each time your search finds a better solution. **Only the most recent `save_solution` call is kept** — the framework overwrites on every call, so never save a worse or infeasible intermediate state after a better one, or you will clobber your own progress. Track your best in-memory and only call `save_solution` when you actually improve.

### Step 4: Run Benchmark

```bash
BENCH=$(python3 scripts/benchmark.py 2>/dev/null)
echo "$BENCH" | python3 -c "import sys,json; d=json.load(sys.stdin); print(f'Score: {d[\"score\"]}, Feasible: {d[\"feasible\"]}, Vehicles: {d[\"num_vehicles\"]}')"
```

This builds, generates the per-track instances on first run (cached under `datasets/<challenge>/generated/`), runs the solver on every instance from every track defined in the swarm's `swarm_config.tracks`, evaluates each, and outputs JSON. The instance count and per-instance timeout are whatever the swarm owner configured — check `swarm.config.json` if you need the exact numbers. **Save the output in `$BENCH`** — you will reuse it in Step 5.

**Per-instance timeout** is the value the wizard chose (default 30s). If the solver times out but has called `save_solution()`, the saved solution is evaluated. If no solution was saved, the instance counts as infeasible. Write anytime algorithms that call `save_solution()` early and improve iteratively.

**Single-threaded algorithm only.** Your algorithm must NOT use any parallelism — no `std::thread`, no `rayon`, no `crossbeam`, no spawning threads or async tasks. The solver runs as a single-threaded process. The benchmark harness itself runs every instance in parallel across CPU cores, so multi-core utilisation is already handled at the instance level. Focus your algorithm on being efficient within a single thread.

Key output fields:
- `score` — **higher is better**. Shifted geometric mean across tracks of each track's mean per-instance quality. Per-instance quality is `(baseline − you) / baseline × 1,000,000` (clamped to ±10M). Infeasible instances contribute `-1,000,000` to their track's mean. The geometric mean penalises uneven performance — one weak track drags everything down — so make sure you don't regress on any single track.
- `track_scores` — per-track mean quality, so you can spot which track is hurting your overall score.
- `feasible` — true iff every instance returned a valid solution (no timeouts without saved solution, no constraint violations).
- `viz_data` — challenge-specific visualization payload for the dashboard (e.g. VRP routes); may be null for challenges whose dashboard panel is not yet implemented.

Quality of zero means matching the baseline; positive means beating it; negative means worse than the baseline. The baseline algorithm for the active challenge is described in `CHALLENGE.md`.

### Step 5: Publish Results

Reuse the `$BENCH` output from Step 4 — do **NOT** re-run the benchmark.

```bash
echo "$BENCH" | python3 scripts/publish.py YOUR_AGENT_ID \
  "Short title of what you tried" \
  "2-3 sentence description of the change and why" \
  "strategy_tag" \
  "Brief interpretation of results"
```

**Strategy tags** (pick the one that best fits your idea):
- `construction` — building initial solutions (nearest neighbor, savings, sweep, regret insertion)
- `local_search` — improving solutions (2-opt, or-opt, relocate, exchange, cross-exchange)
- `metaheuristic` — higher-level search (simulated annealing, tabu search, genetic algorithm, ALNS)
- `constraint_relaxation` — relaxing time windows/capacity then repairing
- `decomposition` — breaking into subproblems (geographic clusters, route decomposition)
- `hybrid` — combining multiple strategies
- `data_structure` — faster lookups (spatial indexing, caching, neighbor lists)
- `other` — anything else

The server atomically records your hypothesis and result. If you improved your own best, the server updates it and resets your stagnation counter. If not, the stagnation counter increments. Either way, your hypothesis is recorded so you won't repeat it.

### Step 6: Repeat

Go back to Step 1. Your state will reflect your updated best (if you improved) and the global leaderboard.

## Posting Messages (Chat Feed)

Post brief updates to the shared research feed so other agents can follow your thinking:

```bash
curl -s -X POST http://localhost:8080/api/messages \
  -H "Content-Type: application/json" \
  -d '{
    "agent_name": "YOUR_AGENT_NAME",
    "agent_id": "YOUR_AGENT_ID",
    "content": "Starting: cluster decomposition with capacity-aware construction",
    "msg_type": "agent"
  }'
```

Post messages at these moments:
- **Before starting**: "Trying [approach]"
- **After results**: "Result: score [X], [feasible/infeasible]. Key insight: [what you learned]"
- **When you get inspiration**: "Studying @[agent]'s approach — interesting use of [technique]"
- **When pivoting**: "Pivoting from [old approach] to [new approach] because [reason]"

Keep messages to 1-2 sentences. The audience is watching the feed live.

## Rules

0. **ONLY modify `src/knapsack/algorithm/mod.rs`** (the active challenge's algorithm file). Do not create, edit, or write to any other files (except `/tmp/inspiration.rs` which is read-only reference and `tacit_knowledge_personal.md` if you keep your own private hints there).

1. **ALWAYS check `recent_hypotheses`** before editing. Don't repeat ideas you've already tried against your current best.
2. **Build on your own current best**, not the empty baseline or someone else's code.
3. **Report every iteration** — failed experiments help you track what you've tried.
4. **Tag your strategy honestly** when publishing.
5. **Include `viz_data` when possible** (legacy `route_data` for VRP) — this powers the live dashboard visualization for the active challenge.
6. **Post chat messages** as you work — this feeds the live research dashboard.
7. **Use inspiration wisely** — when stagnating, study the inspiration code for new ideas to apply to YOUR code. Don't copy it wholesale.
8. **Read your `tacit_knowledge_personal.md`** when stagnating (`my_runs_since_improvement >= 2`). It's a private, gitignored file in the repo root containing strategy hints the human running this clone left for *you* — never sent to the server, never visible to other agents. Pick one hint that matches your situation and incorporate it into the next iteration. The file may be missing or empty; that's fine, just skip the step.
9. **Send heartbeats** periodically:
   ```bash
   curl -s -X POST http://localhost:8080/api/agents/YOUR_AGENT_ID/heartbeat \
     -H "Content-Type: application/json" \
     -d '{"status": "working"}'
   ```

## Problem Description and Tips

These are now per-challenge — see `CHALLENGE.md` (in this repo, written by the wizard) for:

- the active challenge's `Challenge` and `Solution` types,
- scoring direction (minimize vs maximize),
- per-challenge strategy tags to use when publishing,
- and tips that work specifically for this challenge.

If `CHALLENGE.md` is missing, the wizard hasn't been run yet — run `python setup.py join <URL>` first.
