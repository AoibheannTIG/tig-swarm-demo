#!/usr/bin/env python3
"""Run the active challenge's benchmark and emit JSON for publish.py.

Reads swarm-wide config from `https://tig-swarm-demo-production.up.railway.app/api/swarm_config` (or from
`./swarm.config.json` as a fallback for offline use) to pick the challenge,
the per-track instance counts, and the per-instance solver timeout. Builds
the right cargo binary, generates instances on first run (cached under
`datasets/<challenge>/generated/`), then runs solver + evaluator on each
instance in parallel.

# Scoring

Each upstream evaluator returns a baseline-relative *quality* per instance
in the integer range [-QUALITY_PRECISION × 10, +QUALITY_PRECISION × 10]
(QUALITY_PRECISION = 1,000,000). The baseline is the upstream baseline
algorithm for that challenge:

    - satisfiability: binary (1M if all clauses satisfied, else 0).
    - vehicle_routing: Solomon nearest-neighbor (`solomon::run`).
    - knapsack: greedy by value-density (`compute_greedy_baseline`).
    - job_scheduling: SOTA dispatching rules (`compute_sota_baseline`).
    - energy_arbitrage: max(greedy, conservative) (`compute_baseline`).

Higher quality is always better. Aggregation is two-step:

    1. Per-track score = arithmetic mean of per-instance quality scores
       in that track. Infeasible instances contribute `-QUALITY_PRECISION`
       (the worst feasibly-bounded value).
    2. Cross-track score = shifted geometric mean across the per-track
       averages. The shift (+QUALITY_PRECISION × 10 + 1) keeps every
       value strictly positive so the geometric mean is well-defined for
       any combination of negative and positive track scores.

The geometric mean rewards balanced performance — a single bad track
drags the overall score down more than the arithmetic mean would.

Output JSON shape:

    {
      "challenge": "...",
      "score": 1234567.8,           # cross-track shifted geo mean of quality
      "feasible": true,
      "instances_solved": 25,
      "instances_feasible": 25,
      "instances_infeasible": 0,
      "track_scores": {"track_key": <mean quality>, ...},
      "viz_data": { ... per-challenge or null ... },
      # VRP-only legacy fields, present only when the challenge is VRP:
      "num_vehicles": 96,
      "total_distance": 12345.6,
      "route_data": { ... }
    }
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
import math
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

ROOT_DIR = Path(__file__).parent.parent

# Mirrors `QUALITY_PRECISION` in src/lib.rs and the upstream tig-monorepo.
# All vendored evaluators clamp their (baseline-relative) quality to
# ±10 × QUALITY_PRECISION before scaling, so the final per-instance score
# is bounded in [-QUALITY_CLAMP, +QUALITY_CLAMP].
QUALITY_PRECISION = 1_000_000
QUALITY_CLAMP = 10 * QUALITY_PRECISION

# Per-instance penalty for an infeasible instance. Set to the worst
# feasible-bounded value rather than -∞ so the per-track mean stays in a
# sensible range and the shifted geometric mean is well-defined.
INFEASIBLE_QUALITY = -QUALITY_PRECISION

# Constant added to each per-track mean before taking the geometric mean.
# Quality range after clamping is [-10M, +10M]; shift by +10M+1 → strictly
# positive in [1, 20M+1] before geo mean, then unshift the result.
GEOMEAN_SHIFT = QUALITY_CLAMP + 1

# Wizard-baked URL with env-var override; mirrors scripts/publish.py so the
# two stay in lockstep when the wizard re-runs.
SERVER = os.environ.get("TIG_SWARM_SERVER") or "https://tig-swarm-demo-production.up.railway.app"
if SERVER.startswith("$"):
    SERVER = ""  # offline mode — read from swarm.config.json instead


# ── Config loading ──────────────────────────────────────────────────


def load_swarm_config() -> dict:
    """Pull live swarm config from the server, falling back to local cache.

    The server is the source of truth (the owner can change the active
    challenge mid-experiment). swarm.config.json is the offline fallback so
    `python scripts/benchmark.py` works without a server reachable, which
    is useful for ad-hoc local testing of `algorithm/mod.rs` edits.
    """
    if SERVER:
        try:
            with urllib.request.urlopen(f"{SERVER}/api/swarm_config", timeout=4) as r:
                return json.load(r)
        except (urllib.error.URLError, urllib.error.HTTPError, TimeoutError, OSError) as e:
            print(f"warning: couldn't reach {SERVER}/api/swarm_config ({e})", file=sys.stderr)
    cfg_path = ROOT_DIR / "swarm.config.json"
    if cfg_path.exists():
        local = json.loads(cfg_path.read_text())
        return {
            "challenge": local.get("challenge", "vehicle_routing"),
            "tracks": local.get("tracks", {}),
            "timeout": local.get("timeout", 30),
            "scoring_direction": local.get("scoring_direction", "min"),
        }
    print("error: no swarm config available (server unreachable, no swarm.config.json)", file=sys.stderr)
    sys.exit(1)


# ── Build & instance generation ────────────────────────────────────


def build(challenge: str) -> tuple[str, str, str]:
    """Build solver, evaluator, generator with the active challenge feature.
    Returns absolute paths to the three binaries."""
    for binary, feature_set in (
        ("tig_solver", f"solver,{challenge}"),
        ("tig_evaluator", f"evaluator,{challenge}"),
        ("tig_generator", f"generator,{challenge}"),
    ):
        subprocess.run(
            ["cargo", "build", "-r", "--bin", binary, "--features", feature_set],
            cwd=ROOT_DIR, check=True, capture_output=True,
        )
    return (
        str(ROOT_DIR / "target/release/tig_solver"),
        str(ROOT_DIR / "target/release/tig_evaluator"),
        str(ROOT_DIR / "target/release/tig_generator"),
    )


def materialize_instances(
    challenge: str, tracks: dict, generator_bin: str
) -> list[tuple[str, str, Path]]:
    """Generate instances per the active swarm config, cached on disk.

    `tracks` is the `test.json` shape: `{"seed": "test", "track_key": count, ...}`.
    Each (track_key, count) becomes `count` instances under
    `datasets/<challenge>/generated/<track_key>/{0..count-1}.txt`. Generation
    is skipped when the cache already has at least `count` files for the
    track — re-running the wizard with smaller counts won't regenerate.

    Returns a list of `(track_key, instance_filename, instance_path)`.
    """
    seed = str(tracks.get("seed", "test"))
    out: list[tuple[str, str, Path]] = []
    base = ROOT_DIR / "datasets" / challenge / "generated"
    for track_key, count in tracks.items():
        if track_key == "seed" or not isinstance(count, int) or count <= 0:
            continue
        track_dir = base / track_key
        track_dir.mkdir(parents=True, exist_ok=True)
        existing = sorted(p for p in track_dir.glob("*.txt"))
        if len(existing) < count:
            print(
                f"  generating {count - len(existing)} new instances for "
                f"{challenge}/{track_key} (have {len(existing)})…",
                file=sys.stderr,
            )
            subprocess.run(
                [
                    generator_bin, challenge, track_key,
                    "--seed", seed,
                    "-n", str(count),
                    "-o", str(track_dir),
                ],
                check=True, capture_output=True,
            )
        for i in range(count):
            inst = track_dir / f"{i}.txt"
            if inst.exists():
                out.append((track_key, f"{track_key}/{i}", inst))
    return out


# ── Per-instance run ───────────────────────────────────────────────


def parse_evaluator_score(eval_result: subprocess.CompletedProcess) -> tuple[float | None, str | None]:
    if eval_result.returncode != 0:
        return None, (eval_result.stderr or eval_result.stdout or f"evaluator exit {eval_result.returncode}").splitlines()[0]
    stdout = (eval_result.stdout or "").strip()
    if not stdout:
        return None, "evaluator produced no output"
    try:
        payload = json.loads(stdout)
    except json.JSONDecodeError:
        return None, f"invalid evaluator JSON: {stdout[:80]}"
    score = payload.get("score", payload.get("distance"))
    if not isinstance(score, (int, float)):
        return None, "evaluator JSON missing numeric score"
    return float(score), None


def run_instance(
    challenge: str, track_key: str, instance_id: str, instance_path: Path,
    solver: str, evaluator: str, timeout: int,
) -> dict:
    with tempfile.NamedTemporaryFile(suffix=".sol", delete=False) as tmp:
        sol_path = tmp.name
    try:
        try:
            subprocess.run(
                [solver, challenge, str(instance_path), sol_path],
                capture_output=True, text=True, timeout=timeout,
            )
        except subprocess.TimeoutExpired:
            pass  # save_solution may have written a partial; evaluator will judge
        if not os.path.exists(sol_path) or os.path.getsize(sol_path) == 0:
            return {"instance": instance_id, "track": track_key, "error": "no solution saved", "feasible": False}
        try:
            eval_result = subprocess.run(
                [evaluator, challenge, str(instance_path), sol_path],
                capture_output=True, text=True, timeout=max(10, timeout),
            )
        except subprocess.TimeoutExpired:
            return {"instance": instance_id, "track": track_key, "error": "evaluator timeout", "feasible": False}
        score, err = parse_evaluator_score(eval_result)
        if err:
            return {"instance": instance_id, "track": track_key, "error": err, "feasible": False}
        result = {
            "instance": instance_id,
            "track": track_key,
            "score": score,
            "feasible": True,
        }
        if challenge == "vehicle_routing":
            from_vrp = _vrp_extras(str(instance_path), sol_path)
            result.update(from_vrp)
        elif challenge == "job_scheduling":
            gantt = _jsp_extras(str(instance_path), sol_path)
            result.update(gantt)
        elif challenge == "knapsack":
            knapsack = _knapsack_extras(str(instance_path), sol_path)
            result.update(knapsack)
        elif challenge == "energy_arbitrage":
            energy = _energy_extras(str(instance_path), sol_path)
            result.update(energy)
        return result
    finally:
        if os.path.exists(sol_path):
            os.unlink(sol_path)


# ── VRP-specific extras (route_data + num_vehicles) ───────────────


def _vrp_parse_positions(inst_path: str) -> dict:
    positions = {}
    in_customer = False
    try:
        with open(inst_path) as f:
            for line in f:
                line = line.strip()
                if line.startswith("CUST NO"):
                    in_customer = True
                    continue
                if in_customer and line:
                    parts = line.split()
                    if len(parts) >= 3:
                        try:
                            positions[int(parts[0])] = (int(parts[1]), int(parts[2]))
                        except ValueError:
                            pass
    except OSError:
        pass
    return positions


def _vrp_parse_routes(sol_path: str) -> list:
    routes = []
    try:
        with open(sol_path) as f:
            for line in f:
                line = line.strip()
                if line.startswith("Route"):
                    parts = line.split(":")
                    if len(parts) == 2:
                        nodes = [int(x) for x in parts[1].split() if x.strip()]
                        routes.append(nodes)
    except OSError:
        pass
    return routes


def _vrp_extras(inst_path: str, sol_path: str) -> dict:
    positions = _vrp_parse_positions(inst_path)
    routes = _vrp_parse_routes(sol_path)
    if not positions or not routes:
        return {"num_vehicles": len(routes), "route_data": None}
    depot = positions.get(0, (500, 500))
    route_data = {
        "depot": {"x": depot[0], "y": depot[1]},
        "routes": [
            {
                "vehicle_id": i,
                "path": [
                    {"x": positions[node][0], "y": positions[node][1], "customer_id": node}
                    for node in route_nodes
                    if node in positions
                ],
            }
            for i, route_nodes in enumerate(routes)
        ],
    }
    return {"num_vehicles": len(routes), "route_data": route_data}


# ── Job-scheduling-specific extras (Gantt viz_data) ──────────────


def _jsp_parse_solution(sol_path: str) -> list | None:
    """Decode a job-scheduling solution file (base64 → gzip → bincode)."""
    import base64
    import gzip
    import struct

    try:
        with open(sol_path) as f:
            b64_str = json.load(f)
        if not isinstance(b64_str, str):
            return None
        compressed = base64.b64decode(b64_str)
        data = gzip.decompress(compressed)
    except Exception:
        return None

    offset = 0

    def read_u64() -> int:
        nonlocal offset
        val = struct.unpack_from("<Q", data, offset)[0]
        offset += 8
        return val

    def read_u32() -> int:
        nonlocal offset
        val = struct.unpack_from("<I", data, offset)[0]
        offset += 4
        return val

    try:
        num_jobs = read_u64()
        schedule: list[list[tuple[int, int]]] = []
        for _ in range(num_jobs):
            num_ops = read_u64()
            ops = []
            for _ in range(num_ops):
                machine = read_u64()
                start_time = read_u32()
                ops.append((machine, start_time))
            schedule.append(ops)
        return schedule
    except struct.error:
        return None


def _jsp_extras(inst_path: str, sol_path: str) -> dict:
    """Build Gantt chart viz payload from instance + solution files."""
    schedule = _jsp_parse_solution(sol_path)
    if schedule is None:
        return {"gantt_data": None}

    try:
        with open(inst_path) as f:
            challenge = json.load(f)
    except (OSError, json.JSONDecodeError):
        return {"gantt_data": None}

    jobs_per_product = challenge["jobs_per_product"]
    proc_times = challenge["product_processing_times"]

    bars = []
    job_idx = 0
    makespan = 0
    for product_idx, n_jobs in enumerate(jobs_per_product):
        for _ in range(n_jobs):
            if job_idx >= len(schedule):
                break
            ops = schedule[job_idx]
            product_ops = proc_times[product_idx]
            for op_idx, (machine, start_time) in enumerate(ops):
                if op_idx < len(product_ops):
                    duration = product_ops[op_idx].get(str(machine), 0)
                else:
                    duration = 0
                end_time = start_time + duration
                if end_time > makespan:
                    makespan = end_time
                bars.append({
                    "job": job_idx,
                    "op": op_idx,
                    "machine": machine,
                    "start": start_time,
                    "end": end_time,
                })
            job_idx += 1

    return {
        "gantt_data": {
            "num_machines": challenge["num_machines"],
            "num_jobs": challenge["num_jobs"],
            "makespan": makespan,
            "bars": bars,
        }
    }


# ── Knapsack-specific extras (interaction matrix viz_data) ─────────


def _knapsack_parse_solution(sol_path: str) -> list[int] | None:
    """Decode a knapsack solution file (base64 → gzip → bincode)."""
    import base64
    import gzip
    import struct

    try:
        with open(sol_path) as f:
            b64_str = json.load(f)
        if not isinstance(b64_str, str):
            return None
        compressed = base64.b64decode(b64_str)
        data = gzip.decompress(compressed)
    except Exception:
        return None

    offset = 0

    def read_u64() -> int:
        nonlocal offset
        val = struct.unpack_from("<Q", data, offset)[0]
        offset += 8
        return val

    try:
        num_items = read_u64()
        items = [read_u64() for _ in range(num_items)]
        return items
    except struct.error:
        return None


def _knapsack_extras(inst_path: str, sol_path: str) -> dict:
    """Build interaction-matrix viz payload from instance + solution files.

    The matrix sent to the dashboard is K×K where K = len(selected items),
    capped at MAX_VIZ_ITEMS to keep the payload and rendering tractable.
    """
    MAX_VIZ_ITEMS = 200

    items = _knapsack_parse_solution(sol_path)
    if items is None:
        return {"knapsack_data": None}

    try:
        with open(inst_path) as f:
            challenge = json.load(f)
    except (OSError, json.JSONDecodeError):
        return {"knapsack_data": None}

    n = challenge["num_items"]
    interaction_values = challenge["interaction_values"]
    weights = challenge["weights"]
    max_weight = challenge["max_weight"]

    sorted_items = sorted(i for i in items if i < n)
    total_weight = sum(weights[i] for i in sorted_items)
    total_value = 0
    for idx_a, i in enumerate(sorted_items):
        for j in sorted_items[idx_a + 1:]:
            total_value += interaction_values[i][j]

    viz_items = sorted_items[:MAX_VIZ_ITEMS]
    k = len(viz_items)
    sub_matrix = [[0] * k for _ in range(k)]
    for ri, i in enumerate(viz_items):
        for rj, j in enumerate(viz_items):
            if i != j:
                sub_matrix[ri][rj] = interaction_values[i][j]

    return {
        "knapsack_data": {
            "num_selected": len(sorted_items),
            "num_items": n,
            "viz_items": viz_items,
            "interaction_values": sub_matrix,
            "total_value": max(0, total_value),
            "max_weight": max_weight,
            "total_weight": total_weight,
        }
    }


# ── Energy-arbitrage-specific extras (schedule + DA prices) ────────


def _energy_parse_solution(sol_path: str) -> list[list[float]] | None:
    """Decode an energy_arbitrage solution file (base64 → gzip → bincode).

    The schedule is Vec<Vec<f64>>: outer vec = timesteps, inner = batteries.
    """
    import base64
    import gzip
    import struct

    try:
        with open(sol_path) as f:
            b64_str = json.load(f)
        if not isinstance(b64_str, str):
            return None
        compressed = base64.b64decode(b64_str)
        data = gzip.decompress(compressed)
    except Exception:
        return None

    offset = 0

    def read_u64() -> int:
        nonlocal offset
        val = struct.unpack_from("<Q", data, offset)[0]
        offset += 8
        return val

    def read_f64() -> float:
        nonlocal offset
        val = struct.unpack_from("<d", data, offset)[0]
        offset += 8
        return val

    try:
        num_steps = read_u64()
        schedule = []
        for _ in range(num_steps):
            num_batteries = read_u64()
            actions = [read_f64() for _ in range(num_batteries)]
            schedule.append(actions)
        return schedule
    except struct.error:
        return None


def _energy_extras(inst_path: str, sol_path: str) -> dict:
    """Build energy viz payload: per-step aggregate charge/discharge + DA prices."""
    schedule = _energy_parse_solution(sol_path)
    if schedule is None:
        return {"energy_data": None}

    try:
        with open(inst_path) as f:
            challenge = json.load(f)
    except (OSError, json.JSONDecodeError):
        return {"energy_data": None}

    da_prices = challenge.get("market", {}).get("day_ahead_prices", [])
    num_steps = len(schedule)

    agg_charge = []
    agg_discharge = []
    for t in range(num_steps):
        charge = 0.0
        discharge = 0.0
        for u in schedule[t]:
            if u < 0:
                charge += u
            else:
                discharge += u
        agg_charge.append(round(charge, 4))
        agg_discharge.append(round(discharge, 4))

    avg_da = []
    for t in range(min(num_steps, len(da_prices))):
        prices_at_t = da_prices[t]
        avg_da.append(round(sum(prices_at_t) / len(prices_at_t), 2) if prices_at_t else 0)

    return {
        "energy_data": {
            "num_steps": num_steps,
            "num_batteries": len(schedule[0]) if schedule else 0,
            "agg_charge": agg_charge,
            "agg_discharge": agg_discharge,
            "avg_da_price": avg_da,
        }
    }


# ── Aggregation & main ────────────────────────────────────────────


def _shifted_geomean(values: list[float], shift: float = GEOMEAN_SHIFT) -> float:
    """Geometric mean of `values` after adding `shift`, then subtract `shift`
    back so the result is on the original scale.

    Every per-track mean lives in [-QUALITY_CLAMP, +QUALITY_CLAMP], so the
    shifted values live in [1, 2 × QUALITY_CLAMP + 1] — strictly positive,
    so the geometric mean is well-defined regardless of how many tracks
    underperformed the baseline. The result is approximately the per-track
    average when all tracks score similarly, but penalised toward the
    worst track when the spread is wide.
    """
    if not values:
        return 0.0
    log_sum = sum(math.log(v + shift) for v in values)
    return math.exp(log_sum / len(values)) - shift


def aggregate(results: list[dict]) -> dict:
    """Group per-instance qualities by track, average each track, then
    combine via shifted geometric mean. Infeasible instances contribute
    `INFEASIBLE_QUALITY` to their track's average — they're worse than
    matching the baseline, but bounded so the geomean stays well-defined.
    """
    by_track: dict[str, list[float]] = defaultdict(list)
    feasible_count = 0
    infeasible_count = 0
    for r in results:
        track = r.get("track", "unknown")
        if r.get("feasible"):
            by_track[track].append(float(r["score"]))
            feasible_count += 1
        else:
            by_track[track].append(float(INFEASIBLE_QUALITY))
            infeasible_count += 1

    # Per-track arithmetic mean of per-instance quality.
    track_scores: dict[str, float] = {
        track: sum(scores) / len(scores)
        for track, scores in by_track.items()
        if scores
    }

    overall = _shifted_geomean(list(track_scores.values()))

    return {
        "score": overall,
        "feasible": infeasible_count == 0 and feasible_count > 0,
        "instances_solved": len(results),
        "instances_feasible": feasible_count,
        "instances_infeasible": infeasible_count,
        "track_scores": track_scores,
    }


def main() -> int:
    print("Loading swarm config…", file=sys.stderr)
    cfg = load_swarm_config()
    challenge = cfg["challenge"]
    timeout = int(cfg.get("timeout", 30))
    # Direction is no longer used by aggregation — every challenge's
    # quality score is higher-is-better. Kept here for forward-compat
    # with downstream callers that still read it.
    _direction = cfg.get("scoring_direction", "max")  # noqa: F841
    tracks = cfg.get("tracks") or {}

    print(f"Building tig binaries for {challenge}…", file=sys.stderr)
    solver, evaluator, generator = build(challenge)

    print(f"Materialising instances under datasets/{challenge}/generated/…", file=sys.stderr)
    instances = materialize_instances(challenge, tracks, generator)
    if not instances:
        print(
            "error: no instances to run. Run `python setup.py init` to set track counts, "
            "or check datasets/<challenge>/test.json.",
            file=sys.stderr,
        )
        return 2
    print(f"  {len(instances)} instance(s) total", file=sys.stderr)

    workers = min(len(instances), os.cpu_count() or 1)
    results: list[dict] = []
    with ThreadPoolExecutor(max_workers=workers) as pool:
        futures = {
            pool.submit(run_instance, challenge, tk, iid, ipath, solver, evaluator, timeout): iid
            for tk, iid, ipath in instances
        }
        for fut in as_completed(futures):
            results.append(fut.result())

    agg = aggregate(results)
    out: dict = {
        "challenge": challenge,
        **agg,
        "errors": [f"{r['instance']}: {r['error']}" for r in results if "error" in r] or None,
    }

    # VRP-specific legacy fields for the existing dashboard route panel.
    if challenge == "vehicle_routing":
        all_routes = {
            r["instance"]: r["route_data"]
            for r in results
            if r.get("route_data")
        }
        out["num_vehicles"] = sum(r.get("num_vehicles", 0) for r in results if r.get("feasible"))
        out["total_distance"] = sum(r["score"] for r in results if r.get("feasible"))
        out["route_data"] = all_routes or None
        out["viz_data"] = all_routes or None  # generic alias for non-VRP dashboards
    elif challenge == "job_scheduling":
        all_gantt = {
            r["instance"]: r["gantt_data"]
            for r in results
            if r.get("gantt_data")
        }
        out["viz_data"] = all_gantt or None
        out["num_vehicles"] = 0
        out["total_distance"] = out["score"]
    elif challenge == "knapsack":
        all_knapsack = {
            r["instance"]: r["knapsack_data"]
            for r in results
            if r.get("knapsack_data")
        }
        out["viz_data"] = all_knapsack or None
        out["num_vehicles"] = 0
        out["total_distance"] = out["score"]
    elif challenge == "energy_arbitrage":
        all_energy = {
            r["instance"]: r["energy_data"]
            for r in results
            if r.get("energy_data")
        }
        out["viz_data"] = all_energy or None
        out["num_vehicles"] = 0
        out["total_distance"] = out["score"]
    else:
        out["viz_data"] = None
        out["num_vehicles"] = 0
        out["total_distance"] = out["score"]

    print(json.dumps(out, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
