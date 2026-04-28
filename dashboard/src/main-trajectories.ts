import "./style.css";
import { initParticles } from "./lib/particles";
import { TrajectoriesPanel } from "./panels/trajectories";

const params = new URLSearchParams(window.location.search);
const wsProtocol = window.location.protocol === "https:" ? "wss:" : "ws:";
const wsUrl = params.get("ws") || `${wsProtocol}//${window.location.host}/ws/dashboard`;

function getApiUrl(): string {
  const explicit = params.get("api");
  if (explicit) return explicit;
  return wsUrl
    .replace("ws://", "http://")
    .replace("wss://", "https://")
    .replace("/ws/dashboard", "");
}

const canvas = document.getElementById("particleCanvas") as HTMLCanvasElement;
initParticles(canvas);

const panel = new TrajectoriesPanel();
panel.init(document.getElementById("panel-trajectories")!, getApiUrl());

document.addEventListener("keydown", (e) => {
  if (e.key === "1") window.location.href = "/";
  if (e.key === "2") window.location.href = "/ideas.html";
  if (e.key === "3") window.location.href = "/diversity.html";
  if (e.key === "4") window.location.href = "/benchmark.html";
});
