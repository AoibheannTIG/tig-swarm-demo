#!/usr/bin/env python3
"""Publish benchmark results to the swarm coordination server.

Usage:
    python3 scripts/benchmark.py 2>/dev/null \
      | python3 scripts/publish.py AGENT_ID "title" "description" strategy_tag "notes"
"""

import json
import os
import sys
import urllib.request
from pathlib import Path

# The wizard rewrites the literal placeholder below to the swarm's URL.
# TIG_SWARM_SERVER env var overrides — useful for ad-hoc testing without
# rerunning setup. The startswith("$") check catches the un-substituted
# placeholder so a contributor who forgot to run setup.py join gets a
# loud failure instead of a silent post to nowhere.
SERVER = os.environ.get("TIG_SWARM_SERVER") or "${SERVER_URL}"
if SERVER.startswith("$"):
    sys.exit(
        "publish.py: server URL not configured. Run "
        "`python setup.py join <swarm-url>` (or set TIG_SWARM_SERVER)."
    )
ALGO_PATH = Path(__file__).parent.parent / "src/vehicle_routing/algorithm/mod.rs"


def main():
    if len(sys.argv) < 5:
        print(
            "Usage: python3 scripts/publish.py <agent_id> <title> <description> <strategy_tag> [notes]",
            file=sys.stderr,
        )
        sys.exit(1)

    agent_id = sys.argv[1]
    title = sys.argv[2]
    description = sys.argv[3]
    strategy_tag = sys.argv[4]
    notes = sys.argv[5] if len(sys.argv) > 5 else ""

    bench = json.load(sys.stdin)
    code = ALGO_PATH.read_text()

    payload = {
        "agent_id": agent_id,
        "title": title,
        "description": description,
        "strategy_tag": strategy_tag,
        "algorithm_code": code,
        "score": bench["score"],
        "feasible": bench["feasible"],
        "num_vehicles": bench["num_vehicles"],
        "total_distance": bench.get("total_distance", bench["score"]),
        "notes": notes,
        "route_data": bench.get("route_data"),
    }

    req = urllib.request.Request(
        f"{SERVER}/api/iterations",
        data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )

    with urllib.request.urlopen(req) as resp:
        result = json.load(resp)
        print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
