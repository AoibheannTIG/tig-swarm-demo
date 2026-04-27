#!/usr/bin/env python3
"""TIG Swarm setup wizard.

Two modes:

  python setup.py init        Owner: stand up a new swarm. Picks the challenge,
                              instance counts per track, timeout, and server
                              URL; writes swarm.config.json; templates files;
                              optionally pushes config to a running server.

  python setup.py join URL    Contributor: point this clone at someone else's
                              swarm URL. Templates the URL into CLAUDE.md /
                              scripts and creates a stub tacit_knowledge_personal.md
                              for the agent's private hints.

Re-running either mode is safe — it overwrites the same set of files.

Files this script reads / writes:
  - CLAUDE.md, README.md, scripts/publish.py
    (templated: ${SERVER_URL} -> the chosen URL)
  - swarm.config.json (owner-only mirror of what's stored on the server)
  - CHALLENGE.md (per-challenge docs, from challenge_templates/)
  - tacit_knowledge_personal.md (per-contributor, gitignored)
  - datasets/<challenge>/test.json (rewritten with chosen track counts)
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).parent

# Files that contain the ${SERVER_URL} placeholder. The wizard rewrites every
# occurrence in-place. Re-running setup with a different URL safely re-runs
# the substitution because the placeholder is restored at the same time.
TEMPLATED_FILES = [
    ROOT / "CLAUDE.md",
    ROOT / "README.md",
    ROOT / "scripts" / "publish.py",
]

# The literal placeholder strings the tracked files carry. NEVER replace
# arbitrary URLs — too easy to clobber rustup / GitHub / localhost dev URLs
# that happen to live in the same files.
PLACEHOLDER_URL = "${SERVER_URL}"
PLACEHOLDER_CHALLENGE = "${CHALLENGE_NAME}"
PLACEHOLDER_ALGO = "${ALGORITHM_PATH}"

# Per-challenge defaults for the wizard prompts. tracks shape mirrors what
# datasets/<challenge>/test.json must contain. scoring_direction is uniformly
# "max" for every challenge: each upstream evaluator returns a baseline-
# relative quality score where higher is better — even VRP and JSP, whose
# raw objective is to minimise distance/makespan, return `(baseline − ours)
# / baseline` so the per-instance score is "max" semantics.
CHALLENGES = {
    "satisfiability": {
        "scoring_direction": "max",
        "track_keys": [
            "n_vars=5000,ratio=4267",
            "n_vars=7500,ratio=4267",
            "n_vars=10000,ratio=4267",
            "n_vars=100000,ratio=4150",
            "n_vars=100000,ratio=4200",
        ],
    },
    "vehicle_routing": {
        "scoring_direction": "max",
        "track_keys": [
            "n_nodes=600",
            "n_nodes=700",
            "n_nodes=800",
            "n_nodes=900",
            "n_nodes=1000",
        ],
    },
    "knapsack": {
        "scoring_direction": "max",
        "track_keys": [
            "n_items=1000,budget=10",
            "n_items=1000,budget=25",
            "n_items=1000,budget=5",
            "n_items=5000,budget=10",
            "n_items=5000,budget=25",
        ],
    },
    "job_scheduling": {
        "scoring_direction": "max",
        "track_keys": [
            "n=20,s=FLOW_SHOP",
            "n=20,s=HYBRID_FLOW_SHOP",
            "n=20,s=JOB_SHOP",
            "n=20,s=FJSP_MEDIUM",
            "n=20,s=FJSP_HIGH",
        ],
    },
    "energy_arbitrage": {
        "scoring_direction": "max",
        "track_keys": [
            "s=BASELINE",
            "s=CONGESTED",
            "s=MULTIDAY",
            "s=DENSE",
            "s=CAPSTONE",
        ],
    },
}

DEFAULT_TIMEOUT = 30
DEFAULT_INSTANCES_PER_TRACK = 20


# ── Helpers ──────────────────────────────────────────────────────────


def prompt(label: str, default: str | None = None) -> str:
    suffix = f" [{default}]" if default is not None else ""
    while True:
        ans = input(f"{label}{suffix}: ").strip()
        if ans:
            return ans
        if default is not None:
            return default


def prompt_choice(label: str, choices: list[str], default: str) -> str:
    print(label)
    for i, c in enumerate(choices, 1):
        marker = " (default)" if c == default else ""
        print(f"  {i}. {c}{marker}")
    while True:
        ans = input(f"Pick 1-{len(choices)} [{default}]: ").strip()
        if not ans:
            return default
        if ans.isdigit() and 1 <= int(ans) <= len(choices):
            return choices[int(ans) - 1]
        if ans in choices:
            return ans
        print("  invalid choice; try again")


def prompt_int(label: str, default: int, minimum: int = 0) -> int:
    while True:
        ans = input(f"{label} [{default}]: ").strip()
        if not ans:
            return default
        try:
            v = int(ans)
        except ValueError:
            print("  expected integer")
            continue
        if v < minimum:
            print(f"  must be >= {minimum}")
            continue
        return v


def _swap(text: str, placeholder: str, prior: str | None, new: str) -> str:
    """Replace placeholder OR prior value with new. Skip prior if it matches
    the placeholder (already a noop) or new (nothing to do)."""
    text = text.replace(placeholder, new)
    if prior and prior != placeholder and prior != new:
        text = text.replace(prior, new)
    return text


def template_files(
    server_url: str,
    challenge: str | None = None,
    algorithm_path: str | None = None,
    prior: dict | None = None,
) -> None:
    """Substitute swarm-specific placeholders into every tracked file that
    contains them. Idempotent across re-runs — uses the prior values from
    swarm.config.json (if present) so a switch from challenge X → Y doesn't
    leave stale strings in the body of CLAUDE.md.
    """
    prior = prior or {}
    prior_url = prior.get("server_url")
    prior_challenge = prior.get("challenge")
    prior_algo = prior.get("algorithm_path")
    for path in TEMPLATED_FILES:
        if not path.exists():
            print(f"  skipping {path} (missing)")
            continue
        text = path.read_text()
        new = _swap(text, PLACEHOLDER_URL, prior_url, server_url)
        if challenge:
            new = _swap(new, PLACEHOLDER_CHALLENGE, prior_challenge, challenge)
        if algorithm_path:
            new = _swap(new, PLACEHOLDER_ALGO, prior_algo, algorithm_path)
        if new != text:
            path.write_text(new)
            print(f"  templated {path.relative_to(ROOT)}")


def write_swarm_config(cfg: dict) -> None:
    out = ROOT / "swarm.config.json"
    out.write_text(json.dumps(cfg, indent=2) + "\n")
    print(f"  wrote {out.relative_to(ROOT)}")


def read_prior_swarm_config() -> dict | None:
    out = ROOT / "swarm.config.json"
    if not out.exists():
        return None
    try:
        return json.loads(out.read_text())
    except Exception:
        return None


def push_config_to_server(server_url: str, admin_key: str, cfg: dict) -> None:
    """POST swarm config to a running server. Best-effort: if the server
    isn't running yet, skip gracefully and tell the user how to do it later."""
    payload = {
        "admin_key": admin_key,
        "challenge": cfg["challenge"],
        "tracks": cfg["tracks"],
        "timeout": cfg["timeout"],
        "scoring_direction": cfg["scoring_direction"],
        "swarm_name": cfg.get("swarm_name", ""),
        "owner_name": cfg.get("owner_name", ""),
        "stagnation_threshold": cfg.get("stagnation_threshold", 2),
        "stagnation_limit": cfg.get("stagnation_limit", 0),
    }
    req = urllib.request.Request(
        f"{server_url.rstrip('/')}/api/swarm_config",
        data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=4) as resp:
            json.load(resp)
        print(f"  POSTed config to {server_url}/api/swarm_config")
    except (urllib.error.URLError, urllib.error.HTTPError, TimeoutError, OSError) as e:
        print(
            f"  could not reach {server_url} ({e}). Start the server and re-run "
            f"this setup, or POST swarm.config.json yourself once it's up."
        )


def open_in_editor(path: Path) -> None:
    editor = os.environ.get("EDITOR") or os.environ.get("VISUAL") or "nano"
    print(f"  opening {path.relative_to(ROOT)} in {editor} (Ctrl-X to exit nano)…")
    try:
        os.system(f"{editor} {path}")
    except Exception as e:
        print(f"  could not launch editor: {e}; edit {path} by hand")


def write_challenge_md(challenge: str) -> None:
    src = ROOT / "challenge_templates" / f"{challenge}.md"
    dst = ROOT / "CHALLENGE.md"
    if not src.exists():
        print(f"  warning: no challenge template at {src.relative_to(ROOT)}; skipping CHALLENGE.md")
        return
    dst.write_text(src.read_text())
    print(f"  wrote {dst.relative_to(ROOT)} (from {src.relative_to(ROOT)})")


def init_personal_tacit_knowledge() -> Path:
    """Create the contributor's private tacit_knowledge file if missing.
    The agent later renames it to tacit_knowledge_<agent_name>.md after it
    learns its own name from POST /api/agents/register."""
    path = ROOT / "tacit_knowledge_personal.md"
    if path.exists():
        return path
    path.write_text(
        "# Personal tacit knowledge\n\n"
        "Hints only **your local Claude agent** sees. Never sent to the server.\n"
        "Other agents in the swarm cannot read this file.\n\n"
        "Read by your agent when stagnating (`my_runs_since_improvement >= 2`).\n\n"
        "## When stuck, try…\n\n"
        "- (replace this with your own hint)\n"
        "- (add as many as you want)\n"
    )
    print(f"  created {path.relative_to(ROOT)} (gitignored — edit it any time)")
    return path


# ── Modes ────────────────────────────────────────────────────────────


def run_init() -> int:
    print("TIG Swarm — initialise a new swarm")
    print("=" * 48)
    print(
        "You are the swarm OWNER. This wizard configures your swarm-wide\n"
        "settings (challenge, instance counts, timeout) and templates your\n"
        "URL into the docs Claude agents read.\n"
    )

    swarm_name = prompt("Swarm name (display only)", default="my-tig-swarm")
    owner_name = prompt("Your name (display only)", default=os.environ.get("USER", "owner"))

    challenge = prompt_choice(
        "Which TIG challenge will this swarm optimize?",
        list(CHALLENGES.keys()),
        default="vehicle_routing",
    )
    challenge_meta = CHALLENGES[challenge]
    print(f"  -> {challenge}, scoring direction = {challenge_meta['scoring_direction']}")

    print(
        f"\n{challenge} has 5 tracks. For each, choose how many instances to\n"
        f"benchmark per iteration. Lower numbers run faster but give noisier\n"
        f"scores. Default is {DEFAULT_INSTANCES_PER_TRACK}."
    )
    tracks: dict = {"seed": "test"}
    for key in challenge_meta["track_keys"]:
        tracks[key] = prompt_int(f"  instances for {key}", DEFAULT_INSTANCES_PER_TRACK, minimum=0)

    timeout = prompt_int("Per-instance solver timeout (seconds)", DEFAULT_TIMEOUT, minimum=1)

    stagnation_threshold = prompt_int(
        "Stagnation threshold (iterations without improvement before hints/inspiration)",
        2, minimum=1,
    )

    stagnation_limit = prompt_int(
        "Stagnation limit (iterations without improvement before trajectory reset, 0=disabled)",
        0, minimum=0,
    )

    print(
        "\nYour server URL is what agents POST to and what the dashboard\n"
        "lives at. Pick the form that matches how you're running it:\n"
        "  - http://localhost:8080         (local dev only)\n"
        "  - https://<your-tunnel>         (cloudflared / ngrok / tailscale funnel)\n"
        "  - https://<your-railway>.up.railway.app\n"
    )
    server_url = prompt("Server URL", default="http://localhost:8080")

    admin_key = prompt(
        "Admin key (used to push config and broadcast)",
        default="ads-2026",
    )

    cfg = {
        "swarm_name": swarm_name,
        "owner_name": owner_name,
        "server_url": server_url,
        "admin_key": admin_key,
        "challenge": challenge,
        "tracks": tracks,
        "timeout": timeout,
        "stagnation_threshold": stagnation_threshold,
        "stagnation_limit": stagnation_limit,
        "scoring_direction": challenge_meta["scoring_direction"],
    }

    algorithm_path = f"src/{challenge}/algorithm/mod.rs"
    cfg["algorithm_path"] = algorithm_path

    print("\nWriting files…")
    prior = read_prior_swarm_config()
    template_files(
        server_url,
        challenge=challenge,
        algorithm_path=algorithm_path,
        prior=prior,
    )
    write_challenge_md(challenge)
    write_swarm_config(cfg)
    test_json_dir = ROOT / "datasets" / challenge
    test_json_dir.mkdir(parents=True, exist_ok=True)
    (test_json_dir / "test.json").write_text(json.dumps(tracks, indent=2) + "\n")
    print(f"  wrote {(test_json_dir / 'test.json').relative_to(ROOT)}")

    print("\nPushing swarm config to server (best effort)…")
    push_config_to_server(server_url, admin_key, cfg)

    print(
        "\n── Tacit knowledge (optional) ──\n"
        "You can give your local Claude agent private strategy hints that\n"
        "other agents in the swarm never see. These are read when the agent\n"
        f"stagnates ({stagnation_threshold}+ iterations without improvement).\n"
        "When stagnating, the server randomly picks (50/50) between tacit\n"
        "knowledge and swarm inspiration for each iteration.\n"
        "Examples: 'Try simulated annealing with cooling schedule',\n"
        "          'Focus on the interaction_values matrix structure'\n"
    )
    tk_path = init_personal_tacit_knowledge()
    hints: list[str] = []
    while True:
        hint = input("Add a hint (or press Enter to skip): ").strip()
        if not hint:
            break
        hints.append(hint)
    if hints:
        lines = [
            "# Personal tacit knowledge\n",
            "Hints only **your local Claude agent** sees. Never sent to the server.\n",
            f"Read by your agent when stagnating (`my_runs_since_improvement >= {stagnation_threshold}`).\n",
            "\n## When stuck, try…\n",
        ]
        for h in hints:
            lines.append(f"- {h}\n")
        tk_path.write_text("\n".join(lines))
        print(f"  wrote {len(hints)} hint(s) to {tk_path.relative_to(ROOT)}")
    else:
        print(f"  no hints added (edit {tk_path.relative_to(ROOT)} any time)")

    print(
        "\nDone. Next steps:\n"
        "  1. (If not already) start the server:\n"
        "       cd server && pip install -r requirements.txt && \\\n"
        "         DATA_DIR=$(pwd)/../data uvicorn server:app --port 8080\n"
        "  2. Visit your server URL — the dashboard is served from /.\n"
        "  3. Share your URL with collaborators. They run:\n"
        f"       python setup.py join {server_url}\n"
        "  4. Each contributor (including you) opens Claude Code in this\n"
        "     directory and tells it to read CLAUDE.md.\n"
    )
    return 0


def run_join(server_url: str) -> int:
    print(f"TIG Swarm — joining {server_url}")
    print("=" * 48)

    prior = read_prior_swarm_config()
    # Pull live swarm config from the owner's server so we know which
    # challenge / algorithm path to template into the local files.
    challenge = None
    algorithm_path = None
    stagnation_threshold = 2
    try:
        with urllib.request.urlopen(f"{server_url.rstrip('/')}/api/swarm_config", timeout=4) as r:
            swarm = json.load(r)
        challenge = swarm.get("challenge")
        stagnation_threshold = swarm.get("stagnation_threshold", 2)
        if challenge:
            algorithm_path = f"src/{challenge}/algorithm/mod.rs"
    except Exception as e:
        print(f"  couldn't fetch swarm config from {server_url}: {e}")
        print("  CLAUDE.md / CHALLENGE.md will only have the URL templated; rerun this command once the server is up.")

    template_files(
        server_url,
        challenge=challenge,
        algorithm_path=algorithm_path,
        prior=prior,
    )
    if challenge:
        write_challenge_md(challenge)
    # Stash a minimal record so a future re-run can swap the URL/challenge
    # without leaving stale strings in the templated files.
    write_swarm_config(
        {
            "server_url": server_url,
            "role": "contributor",
            "challenge": challenge or (prior or {}).get("challenge"),
            "algorithm_path": algorithm_path or (prior or {}).get("algorithm_path"),
        }
    )

    print(
        "\n── Tacit knowledge (optional) ──\n"
        "You can give your local Claude agent private strategy hints that\n"
        "other agents in the swarm never see. These are read when the agent\n"
        f"stagnates ({stagnation_threshold}+ iterations without improvement).\n"
        "When stagnating, the server randomly picks (50/50) between tacit\n"
        "knowledge and swarm inspiration for each iteration.\n"
        "Examples: 'Try simulated annealing with cooling schedule',\n"
        "          'Focus on the interaction_values matrix structure'\n"
    )
    tk_path = init_personal_tacit_knowledge()
    hints: list[str] = []
    while True:
        hint = input("Add a hint (or press Enter to skip): ").strip()
        if not hint:
            break
        hints.append(hint)
    if hints:
        lines = [
            "# Personal tacit knowledge\n",
            "Hints only **your local Claude agent** sees. Never sent to the server.\n",
            f"Read by your agent when stagnating (`my_runs_since_improvement >= {stagnation_threshold}`).\n",
            "\n## When stuck, try…\n",
        ]
        for h in hints:
            lines.append(f"- {h}\n")
        tk_path.write_text("\n".join(lines))
        print(f"  wrote {len(hints)} hint(s) to {tk_path.relative_to(ROOT)}")
    else:
        print(f"  no hints added (edit {tk_path.relative_to(ROOT)} any time)")

    print(
        "\nDone. Open Claude Code in this directory and have it read\n"
        "CLAUDE.md to start contributing. Edit tacit_knowledge_personal.md\n"
        "any time with private hints — they only ever live on your machine.\n"
    )
    return 0


# ── Auto-detect public URL ──────────────────────────────────────────


def detect_public_url(port: int) -> str:
    """Try to find a publicly reachable URL for this machine."""
    import socket
    import subprocess as sp

    # Try to get the default-route IP (works on most Linux)
    try:
        result = sp.run(
            ["hostname", "-I"], capture_output=True, text=True, timeout=3
        )
        if result.returncode == 0:
            first_ip = result.stdout.strip().split()[0]
            # Check if it's a public IP (not 10.x, 172.16-31.x, 192.168.x)
            parts = first_ip.split(".")
            if parts[0] not in ("10", "127") and not (
                parts[0] == "172" and 16 <= int(parts[1]) <= 31
            ) and not (parts[0] == "192" and parts[1] == "168"):
                return f"http://{first_ip}:{port}"
    except Exception:
        pass

    # Fallback: try external service
    try:
        with urllib.request.urlopen("https://ifconfig.me", timeout=3) as r:
            ip = r.read().decode().strip()
            return f"http://{ip}:{port}"
    except Exception:
        pass

    return f"http://localhost:{port}"


# ── Start (automated owner setup) ──────────────────────────────────


def run_start() -> int:
    """Automated owner setup: prompts only for challenge, instances per
    track, and timeout. Auto-detects public URL, starts the server, and
    prints a shareable join link."""
    import subprocess as sp

    port = 8080
    print("TIG Swarm — automated setup")
    print("=" * 48)

    challenge = prompt_choice(
        "Which TIG challenge will this swarm optimize?",
        list(CHALLENGES.keys()),
        default="vehicle_routing",
    )
    challenge_meta = CHALLENGES[challenge]
    print(f"  -> {challenge}")

    print(
        f"\n{challenge} has 5 tracks. For each, choose how many instances to\n"
        f"benchmark per iteration. Default is {DEFAULT_INSTANCES_PER_TRACK}."
    )
    tracks: dict = {"seed": "test"}
    for key in challenge_meta["track_keys"]:
        tracks[key] = prompt_int(f"  instances for {key}", DEFAULT_INSTANCES_PER_TRACK, minimum=0)

    timeout = prompt_int("\nPer-instance solver timeout (seconds)", DEFAULT_TIMEOUT, minimum=1)

    stagnation_threshold = prompt_int(
        "Stagnation threshold (iterations without improvement before hints/inspiration)",
        2, minimum=1,
    )

    stagnation_limit = prompt_int(
        "Stagnation limit (iterations without improvement before trajectory reset, 0=disabled)",
        0, minimum=0,
    )

    # Tacit knowledge
    print(
        "\n── Tacit knowledge (optional) ──\n"
        "Give your local Claude agent private strategy hints.\n"
        f"These are read when stagnating ({stagnation_threshold}+ iterations without improvement).\n"
    )
    tk_path = init_personal_tacit_knowledge()
    hints: list[str] = []
    while True:
        hint = input("Add a hint (or press Enter to skip): ").strip()
        if not hint:
            break
        hints.append(hint)
    if hints:
        lines = [
            "# Personal tacit knowledge\n",
            "Hints only **your local Claude agent** sees. Never sent to the server.\n",
            f"Read by your agent when stagnating (`my_runs_since_improvement >= {stagnation_threshold}`).\n",
            "\n## When stuck, try…\n",
        ]
        for h in hints:
            lines.append(f"- {h}\n")
        tk_path.write_text("\n".join(lines))
        print(f"  wrote {len(hints)} hint(s) to {tk_path.relative_to(ROOT)}")

    # Ensure data directory exists
    data_dir = ROOT / "server" / "data"
    data_dir.mkdir(parents=True, exist_ok=True)

    # Start the server
    print("\nStarting server…")
    env = os.environ.copy()
    env["DATA_DIR"] = str(data_dir)
    server_proc = sp.Popen(
        [sys.executable, "-m", "uvicorn", "server:app", "--host", "0.0.0.0", "--port", str(port)],
        cwd=str(ROOT / "server"),
        env=env,
        stdout=sp.DEVNULL,
        stderr=sp.DEVNULL,
    )

    # Wait for server to be ready
    import time
    for _ in range(20):
        time.sleep(0.5)
        try:
            with urllib.request.urlopen(f"http://localhost:{port}/api/swarm_config", timeout=2):
                break
        except Exception:
            continue
    else:
        print("  error: server did not start within 10 seconds")
        server_proc.kill()
        return 1

    print(f"  server running (PID {server_proc.pid})")

    # Detect public URL
    server_url = detect_public_url(port)
    print(f"  detected URL: {server_url}")

    # Verify reachability
    try:
        with urllib.request.urlopen(f"{server_url}/api/swarm_config", timeout=3):
            pass
        print(f"  confirmed reachable")
    except Exception:
        print(f"  warning: {server_url} may not be reachable externally")
        print(f"  falling back to localhost — share via tunnel if needed")
        server_url = f"http://localhost:{port}"

    admin_key = "ads-2026"
    cfg = {
        "swarm_name": f"{challenge}-swarm",
        "owner_name": os.environ.get("USER", "owner"),
        "server_url": server_url,
        "admin_key": admin_key,
        "challenge": challenge,
        "tracks": tracks,
        "timeout": timeout,
        "stagnation_threshold": stagnation_threshold,
        "stagnation_limit": stagnation_limit,
        "scoring_direction": challenge_meta["scoring_direction"],
        "algorithm_path": f"src/{challenge}/algorithm/mod.rs",
    }

    print("\nWriting config…")
    prior = read_prior_swarm_config()
    template_files(
        server_url,
        challenge=challenge,
        algorithm_path=cfg["algorithm_path"],
        prior=prior,
    )
    write_challenge_md(challenge)
    write_swarm_config(cfg)
    test_json_dir = ROOT / "datasets" / challenge
    test_json_dir.mkdir(parents=True, exist_ok=True)
    (test_json_dir / "test.json").write_text(json.dumps(tracks, indent=2) + "\n")

    print("Pushing config to server…")
    push_config_to_server(server_url, admin_key, cfg)

    # Detect the git remote URL so the join instructions are correct for forks
    repo_url = "<this-repo-url>"
    try:
        result = sp.run(
            ["git", "remote", "get-url", "origin"],
            capture_output=True, text=True, timeout=3, cwd=str(ROOT),
        )
        if result.returncode == 0 and result.stdout.strip():
            repo_url = result.stdout.strip()
    except Exception:
        pass

    print("\n" + "=" * 48)
    print("SWARM IS LIVE")
    print("=" * 48)
    print(f"\n  Dashboard:  {server_url}/")
    print(f"  Challenge:  {challenge}")
    print(f"\n  Share this with your friends to join:\n")
    print(f"    git clone {repo_url}")
    print(f"    cd {Path(repo_url).stem.replace('.git', '') if repo_url != '<this-repo-url>' else 'tig-swarm-demo'}")
    print(f"    python setup.py join {server_url}")
    print(f"\n  Then tell Claude: 'Read CLAUDE.md and start contributing to the swarm.'")
    print(f"\n  Server PID: {server_proc.pid} (kill with: kill {server_proc.pid})")
    print()
    return 0


# ── Entrypoint ──────────────────────────────────────────────────────


def main() -> int:
    parser = argparse.ArgumentParser(prog="setup.py")
    sub = parser.add_subparsers(dest="mode", required=True)
    sub.add_parser("init", help="Owner: configure a new swarm (manual server setup).")
    sub.add_parser("start", help="Owner: configure + auto-start server + print join link.")
    join = sub.add_parser("join", help="Contributor: point this clone at a swarm URL.")
    join.add_argument("server_url", help="The swarm owner's server URL.")
    args = parser.parse_args()

    if args.mode == "init":
        return run_init()
    if args.mode == "start":
        return run_start()
    if args.mode == "join":
        return run_join(args.server_url)
    parser.print_help()
    return 1


if __name__ == "__main__":
    sys.exit(main())
