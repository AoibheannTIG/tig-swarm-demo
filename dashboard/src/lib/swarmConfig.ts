// Live swarm config (active challenge, scoring direction, swarm name).
// Fetched once at page load and refreshed when the server broadcasts a
// `swarm_config_updated` event over the WebSocket. Every panel that
// renders challenge-specific content (labels, the active visualization,
// score-direction-aware deltas) should consult getSwarmConfig() rather
// than hardcoding "vehicle_routing" assumptions.

export type ScoringDirection = "min" | "max";

export interface SwarmConfig {
  challenge: string;
  scoring_direction: ScoringDirection;
  tracks: Record<string, number | string>;
  timeout: number;
  swarm_name: string;
  owner_name: string;
}

const FALLBACK: SwarmConfig = {
  challenge: "vehicle_routing",
  scoring_direction: "min",
  tracks: {},
  timeout: 30,
  swarm_name: "",
  owner_name: "",
};

let current: SwarmConfig = FALLBACK;
let listeners: Array<(cfg: SwarmConfig) => void> = [];

export function getSwarmConfig(): SwarmConfig {
  return current;
}

export function getDirection(): ScoringDirection {
  return current.scoring_direction;
}

export function isMin(): boolean {
  return current.scoring_direction === "min";
}

export function isMax(): boolean {
  return current.scoring_direction === "max";
}

// Returns true if `candidate` beats `prior` in the active direction.
// Used by panels that compare two scores (e.g. "is this experiment a new
// best for the agent?").
export function isBetter(candidate: number, prior: number): boolean {
  return current.scoring_direction === "max"
    ? candidate > prior
    : candidate < prior;
}

// Score label for stats / leaderboard headers.
export function scoreLabel(): string {
  if (current.challenge === "vehicle_routing") return "DISTANCE";
  if (current.challenge === "satisfiability") return "QUALITY";
  if (current.challenge === "knapsack") return "VALUE";
  if (current.challenge === "job_scheduling") return "MAKESPAN";
  if (current.challenge === "energy_arbitrage") return "PROFIT";
  return "SCORE";
}

export async function loadSwarmConfig(apiBase: string): Promise<SwarmConfig> {
  try {
    const r = await fetch(`${apiBase}/api/swarm_config`);
    if (!r.ok) throw new Error(`HTTP ${r.status}`);
    const data = await r.json();
    current = {
      challenge: data.challenge ?? FALLBACK.challenge,
      scoring_direction: data.scoring_direction === "max" ? "max" : "min",
      tracks: data.tracks ?? {},
      timeout: typeof data.timeout === "number" ? data.timeout : 30,
      swarm_name: data.swarm_name ?? "",
      owner_name: data.owner_name ?? "",
    };
    notify();
  } catch (e) {
    console.warn("loadSwarmConfig: falling back to defaults", e);
  }
  return current;
}

export function onSwarmConfigChange(fn: (cfg: SwarmConfig) => void): () => void {
  listeners.push(fn);
  return () => {
    listeners = listeners.filter((l) => l !== fn);
  };
}

function notify(): void {
  for (const l of listeners) {
    try {
      l(current);
    } catch (e) {
      console.error("swarm config listener error", e);
    }
  }
}

// Wire up to the WebSocket so dashboards see the owner switching the
// active challenge mid-experiment without a manual refresh. Re-fetches
// the full config (the WS event carries only a subset).
export function handleWsEvent(apiBase: string, msg: any): void {
  if (msg && msg.type === "swarm_config_updated") {
    void loadSwarmConfig(apiBase);
  }
}
