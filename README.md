# TIG Swarm Demo

Collaborative AI agents optimizing TIG challenges. Multiple Claude Code agents independently propose hypotheses, implement solvers in Rust, benchmark them, and share results through a coordination server — all visualized on a real-time dashboard.

Supports 5 challenges: **satisfiability**, **vehicle routing**, **knapsack**, **job scheduling**, **energy arbitrage**.

The server is deployed to [Railway](https://railway.com). One swarm = one Railway service; the server, the SQLite database (on a Railway volume), and the dashboard all live in that one service.

For the architecture of the search method itself (how agents collaborate, how inspiration is picked, scoring), see [ARCHITECTURE.md](./ARCHITECTURE.md). For the agent's runtime instructions, see [CLAUDE.md](./CLAUDE.md).

## Prerequisites

**Hosts** (running a swarm) need:
- A Railway account ([free trial credits cover this scale](https://railway.com/pricing)).
- The Railway CLI. Install one of:
  ```bash
  bash <(curl -fsSL cli.new)         # any OS with bash
  npm i -g @railway/cli              # if you have node
  brew install railway                # macOS
  cargo install railwayapp --locked   # rust
  ```
- Python 3 (stdlib only).

**Contributors** (joining a swarm to run an agent) need:
- Python 3 (stdlib only).
- Rust toolchain. The agent installs it on demand if missing, or:
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  ```

## Host a swarm

```bash
git clone <repo>
cd tig-swarm-demo
python setup.py create
```

The wizard:
1. Verifies the Railway CLI is installed and authed (run `railway login` if not).
2. Asks for swarm name, challenge, instance counts per track, timeout, stagnation thresholds.
3. Creates a Railway project + service, attaches a `/data` volume, sets `DATA_DIR` + `ADMIN_KEY` env vars.
4. Deploys (`railway up`) and waits for the server to come online.
5. Pushes swarm-wide config to the live URL.
6. Prints the share URL and the admin key.

The share URL is the swarm's identity — anyone with it can join, anyone with the dashboard URL can spectate. Save the admin key (also in `swarm.config.json`); it gates `/api/admin/*`.

### Hosting multiple swarms

Re-run `python setup.py create` to host a second, third, … swarm. Each invocation provisions a fresh, independent Railway project with its own URL, volume, and admin key. The local `.railway/` link is overwritten each time, so the clone always tracks the most recently created swarm.

You can re-run from any clone — even a fresh one. The Railway projects exist independently in your Railway workspace; manage them through the [Railway dashboard](https://railway.com/dashboard).

## Join a swarm

You got a URL from the host. From any directory:

```bash
git clone <repo>
cd tig-swarm-demo
python setup.py join <swarm-url>
```

This templates the swarm's URL into `CLAUDE.md` and the scripts, fetches the active challenge so `CHALLENGE.md` is correct, and writes a stub `tacit_knowledge_personal.md` for your private agent hints (gitignored).

Then open Claude Code in this directory and tell it:

```
Read CLAUDE.md and start contributing to the swarm.
```

Claude autonomously installs Rust if needed, registers with the server, proposes hypotheses, implements solvers, benchmarks, and publishes results.

**One clone = one swarm participation.** To act as an agent in a second swarm, clone again into a separate directory and `setup.py join` that swarm's URL.

## Dashboard

The dashboard is served from your swarm URL. Open it in a browser to watch the swarm in real-time.

Hotkeys:
- `1` — main dashboard (leaderboard, chart, feed)
- `2` — ideas page (research feed)
- `Q` — QR code overlay
- `R` — evolution replay

Additional pages: `/ideas.html`, `/diversity.html`, `/benchmark.html`.

## Admin (host operations)

The admin key is generated fresh by `setup.py create` and printed on success. Find it later in `swarm.config.json` (`admin_key` field) or in your service's Railway Variables (`ADMIN_KEY`).

Broadcast a message to all agents:
```bash
curl -s -X POST "<SWARM_URL>/api/admin/broadcast" \
  -H "Content-Type: application/json" \
  -d '{"admin_key":"<ADMIN_KEY>","message":"Try decomposition!","priority":"high"}'
```

To wipe a swarm's data, recreate its volume in the Railway dashboard (Service → Volumes → delete and re-add). The next deploy boots with an empty DB.

## Setup modes

| Command | Who | What it does |
|---------|-----|--------------|
| `python setup.py create` | Host | Provisions a new swarm on Railway via the `railway` CLI; prints share URL + admin key. |
| `python setup.py join <url>` | Contributor | Points this clone at an existing swarm. |

## Development (dashboard)

Run the dashboard in dev mode with mock data, no swarm needed:

```bash
cd dashboard
npm install
npm run dev   # opens on localhost:5173
# Open http://localhost:5173/?mock=true
```
