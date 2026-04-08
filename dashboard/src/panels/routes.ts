import * as d3 from "d3";
import type { Panel, WSMessage, RouteData, AllRouteData, RoutePoint } from "../types";
import { getRouteColor } from "../lib/colors";

interface DrawOptions {
  ghost?: boolean;
  animate?: boolean;
}

const routeLine = d3.line<RoutePoint>()
  .x((d) => d.x)
  .y((d) => d.y)
  .curve(d3.curveCatmullRom.alpha(0.5));

function fullPath(data: RouteData, route: { path: RoutePoint[] }): RoutePoint[] {
  const depot = { x: data.depot.x, y: data.depot.y, customer_id: -1 };
  return [depot, ...route.path, depot];
}

export class RoutesPanel implements Panel {
  private svg!: any;
  private routeGroup!: any;
  private ghostGroup!: any;
  private customerGroup!: any;
  private depotGroup!: any;
  private scoreEl!: HTMLElement;
  private deltaEl!: HTMLElement;
  private instanceLabelEl!: HTMLElement;
  private navEl!: HTMLElement;

  private allInstances: AllRouteData = {};
  private currentIndex = 0;
  private currentRouteData: RouteData | null = null;
  private ghostTimeout: ReturnType<typeof setTimeout> | null = null;

  private get instanceKeys(): string[] {
    return Object.keys(this.allInstances).sort();
  }

  init(container: HTMLElement) {
    container.innerHTML = `
      <div class="panel-inner routes-panel">
        <div class="panel-label">ROUTES</div>
        <svg id="routes-svg"></svg>
        <div class="routes-nav" id="routes-nav" style="display:none">
          <button class="routes-nav-btn" id="routes-prev">&lsaquo;</button>
          <span class="routes-instance-label" id="routes-instance-label"></span>
          <button class="routes-nav-btn" id="routes-next">&rsaquo;</button>
        </div>
        <div class="routes-score">
          <div class="routes-score-label">TOTAL DISTANCE</div>
          <div class="routes-score-value" id="routes-score">---</div>
        </div>
        <div class="routes-delta" id="routes-delta"></div>
      </div>
    `;

    this.scoreEl = document.getElementById("routes-score")!;
    this.deltaEl = document.getElementById("routes-delta")!;
    this.instanceLabelEl = document.getElementById("routes-instance-label")!;
    this.navEl = document.getElementById("routes-nav")!;

    document.getElementById("routes-prev")!.addEventListener("click", () => this.navigate(-1));
    document.getElementById("routes-next")!.addEventListener("click", () => this.navigate(1));

    this.svg = d3.select("#routes-svg");
    this.svg
      .attr("viewBox", "0 0 1000 1000")
      .attr("preserveAspectRatio", "xMidYMid meet");

    const defs = this.svg.append("defs");
    const filter = defs.append("filter").attr("id", "route-glow");
    filter.append("feGaussianBlur").attr("stdDeviation", "1.5").attr("result", "blur");
    const merge = filter.append("feMerge");
    merge.append("feMergeNode").attr("in", "blur");
    merge.append("feMergeNode").attr("in", "SourceGraphic");

    const radGrad = defs.append("radialGradient").attr("id", "shockwave-grad");
    radGrad.append("stop").attr("offset", "0%").attr("stop-color", "#00e5ff").attr("stop-opacity", "0");
    radGrad.append("stop").attr("offset", "70%").attr("stop-color", "#00e5ff").attr("stop-opacity", "0.2");
    radGrad.append("stop").attr("offset", "100%").attr("stop-color", "#00e5ff").attr("stop-opacity", "0");

    this.ghostGroup = this.svg.append("g").attr("class", "ghost-routes");
    this.routeGroup = this.svg.append("g").attr("class", "routes");
    this.customerGroup = this.svg.append("g").attr("class", "customers");
    this.depotGroup = this.svg.append("g").attr("class", "depot");

    setInterval(() => {
      if (this.instanceKeys.length > 1) {
        this.navigate(1);
      }
    }, 8000);
  }

  private navigate(delta: number) {
    const keys = this.instanceKeys;
    if (keys.length === 0) return;
    this.currentIndex = (this.currentIndex + delta + keys.length) % keys.length;
    this.updateInstanceLabel();
    this.drawRoutes(this.allInstances[keys[this.currentIndex]]);
  }

  private updateInstanceLabel() {
    const keys = this.instanceKeys;
    if (keys.length <= 1) {
      this.navEl.style.display = "none";
      return;
    }
    this.navEl.style.display = "flex";
    const key = keys[this.currentIndex];
    // Format: "n_nodes=100/0.txt" -> "100 nodes #1"
    const match = key.match(/n_nodes=(\d+)\/(\d+)/);
    const label = match
      ? `${match[1]} nodes #${parseInt(match[2]) + 1}`
      : key;
    this.instanceLabelEl.textContent = `${label}  (${this.currentIndex + 1}/${keys.length})`;
  }

  handleMessage(msg: WSMessage) {
    if (msg.type === "new_global_best" && msg.route_data) {
      this.handleNewBest(msg.route_data, msg.score, msg.improvement_pct);
    }
    if (msg.type === "stats_update" && msg.best_score && !this.currentRouteData) {
      this.scoreEl.textContent = msg.best_score.toFixed(1);
    }
  }

  private handleNewBest(rawData: AllRouteData, score: number, improvementPct: number) {
    this.allInstances = rawData;
    const keys = this.instanceKeys;

    if (this.currentIndex >= keys.length) {
      this.currentIndex = 0;
    }

    const data = rawData[keys[this.currentIndex]];
    this.updateInstanceLabel();
    this.animateNewBest(data, score, improvementPct);
  }

  private animateNewBest(data: RouteData, score: number, improvementPct: number) {
    const oldData = this.currentRouteData;
    this.currentRouteData = data;

    // Move current routes to ghost layer
    if (oldData) {
      if (this.ghostTimeout) clearTimeout(this.ghostTimeout);
      this.ghostGroup.selectAll("*").remove();
      this.drawRoutes(oldData, { ghost: true, target: this.ghostGroup });
      this.ghostGroup.transition().duration(600).style("opacity", 0.06);
      this.ghostTimeout = setTimeout(() => this.ghostGroup.selectAll("*").remove(), 12000);
    }

    this.drawRoutes(data, { animate: true });
    this.shockwave(data.depot.x, data.depot.y);

    this.scoreEl.textContent = score.toFixed(1);

    this.deltaEl.textContent = `+${improvementPct.toFixed(1)}% improvement`;
    this.deltaEl.style.opacity = "1";
    this.deltaEl.style.transform = "translateY(0)";
    setTimeout(() => {
      this.deltaEl.style.transition = "opacity 1s ease, transform 1s ease";
      this.deltaEl.style.opacity = "0";
      this.deltaEl.style.transform = "translateY(-10px)";
      setTimeout(() => { this.deltaEl.style.transition = ""; }, 1000);
    }, 2500);
  }

  private drawRoutes(
    data: RouteData,
    opts?: DrawOptions & { target?: any },
  ) {
    const { ghost = false, animate = false, target } = opts ?? {};
    const group = target ?? this.routeGroup;

    if (!target) {
      this.routeGroup.selectAll("*").remove();
      this.customerGroup.selectAll("*").remove();
      this.depotGroup.selectAll("*").remove();
    }

    data.routes.forEach((route, i) => {
      const path = fullPath(data, route);
      const color = getRouteColor(i);

      // Glow trail (skip for ghost)
      if (!ghost) {
        const glow = group.append("path")
          .datum(path)
          .attr("d", routeLine as any)
          .attr("fill", "none")
          .attr("stroke", color)
          .attr("stroke-width", 20)
          .attr("filter", "url(#route-glow)");

        if (animate) {
          glow.attr("stroke-opacity", 0)
            .transition().delay(i * 100).duration(800)
            .attr("stroke-opacity", 0.1);
        } else {
          glow.attr("stroke-opacity", 0.1);
        }
      }

      // Main path
      const mainPath = group.append("path")
        .datum(path)
        .attr("d", routeLine as any)
        .attr("fill", "none")
        .attr("stroke", color)
        .attr("stroke-width", ghost ? 5 : 8)
        .attr("stroke-opacity", ghost ? 0.3 : 0.85)
        .attr("class", ghost ? "" : "route-flowing");

      if (animate) {
        const node = mainPath.node()!;
        const totalLength = node.getTotalLength();
        mainPath
          .attr("stroke-dasharray", `${totalLength}`)
          .attr("stroke-dashoffset", totalLength)
          .transition()
          .delay(i * 100)
          .duration(1200)
          .ease(d3.easeCubicInOut)
          .attr("stroke-dashoffset", 0)
          .on("end", function (this: SVGPathElement) {
            d3.select(this)
              .attr("stroke-dasharray", "20 8")
              .attr("stroke-dashoffset", 0);
          });
      } else {
        mainPath.attr("stroke-dasharray", ghost ? "none" : "20 8");
      }

      // Customers (skip for ghost)
      if (!ghost) {
        route.path.forEach((pt) => {
          const circle = this.customerGroup.append("circle")
            .attr("cx", pt.x)
            .attr("cy", pt.y)
            .attr("fill", color)
            .attr("opacity", 0.7);

          if (animate) {
            circle.attr("r", 0)
              .transition().delay(i * 100 + 400).duration(300)
              .attr("r", 10);
          } else {
            circle.attr("r", 10);
          }
        });
      }
    });

    // Depot (skip for ghost)
    if (!ghost) {
      const depotSize = 25;
      const depot = this.depotGroup.append("rect")
        .attr("x", data.depot.x - depotSize / 2)
        .attr("y", data.depot.y - depotSize / 2)
        .attr("width", depotSize)
        .attr("height", depotSize)
        .attr("fill", "#fff")
        .attr("transform", `rotate(45, ${data.depot.x}, ${data.depot.y})`)
        .attr("class", "depot-pulse");

      if (animate) {
        depot.attr("opacity", 0).transition().duration(400).attr("opacity", 0.9);
      } else {
        depot.attr("opacity", 0.9);
      }
    }
  }

  private shockwave(cx: number, cy: number) {
    const ring = this.svg.append("circle")
      .attr("cx", cx)
      .attr("cy", cy)
      .attr("r", 20)
      .attr("fill", "none")
      .attr("stroke", "#00e5ff")
      .attr("stroke-width", 5)
      .attr("stroke-opacity", 0.4);

    ring.transition()
      .duration(1000)
      .ease(d3.easeCubicOut)
      .attr("r", 400)
      .attr("stroke-opacity", 0)
      .attr("stroke-width", 1)
      .remove();
  }
}
