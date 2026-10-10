import { subscribe, type Temps } from "../data";
import { el, type PanelContext, type Widget } from "./index";

const HISTORY = 60; // samples (one per update, ~1 s)
const SVG = "http://www.w3.org/2000/svg";

/** "AMD Ryzen 9 9955HX" → "Ryzen 9 9955HX", "NVIDIA GeForce RTX 5070 Laptop GPU" → "RTX 5070 Laptop". */
const shortName = (name: string | undefined) =>
  (name ?? "").replace(/^(AMD|Intel\(R\)|Intel|NVIDIA)\s+/i, "").replace(/^GeForce\s+/i, "").replace(/\s+GPU$/i, "");

const num = (v: number | null | undefined, digits = 0) => (v == null ? "–" : v.toFixed(digits));

function bar(): { root: HTMLElement; set: (pct: number | null | undefined) => void } {
  const fill = el("hw-bar-fill");
  const root = el("hw-bar", fill);
  return { root, set: (pct) => fill.style.setProperty("--v", String(Math.max(0, Math.min(100, pct ?? 0)) / 100)) };
}

function sparkline(): { root: SVGSVGElement; set: (values: number[]) => void } {
  const root = document.createElementNS(SVG, "svg");
  root.setAttribute("class", "hw-spark");
  root.setAttribute("viewBox", "0 0 100 24");
  root.setAttribute("preserveAspectRatio", "none");
  const line = document.createElementNS(SVG, "polyline");
  root.append(line);
  return {
    root,
    set: (values) => {
      if (values.length < 2) return line.setAttribute("points", "");
      const lo = Math.min(...values) - 2;
      const hi = Math.max(...values) + 2;
      const step = 100 / (HISTORY - 1);
      const x0 = 100 - step * (values.length - 1);
      line.setAttribute(
        "points",
        values.map((v, i) => `${(x0 + i * step).toFixed(1)},${(22 - ((v - lo) / (hi - lo)) * 20).toFixed(1)}`).join(" "),
      );
    },
  };
}

/** One CPU/GPU card: heading, big temperature + 60 s sparkline, load bar, detail line. */
function card(label: string) {
  const name = el("hw-name");
  const temp = el("hw-temp");
  const spark = sparkline();
  const load = bar();
  const loadText = el("hw-load-text");
  const detail = el("hw-detail");
  const root = el(
    "hw-card",
    el("hw-head", el("hw-label", label), name),
    el("hw-main", temp, spark.root),
    el("hw-row", el("hw-key", "Load"), load.root, loadText),
    detail,
  );
  const history: number[] = [];
  return {
    root,
    update(t: number | null | undefined, hw: string | undefined, loadPct: number | null | undefined, details: string, warn: number) {
      name.textContent = shortName(hw);
      temp.textContent = t == null ? "–" : `${Math.round(t)}°`;
      temp.classList.toggle("warn", t != null && t >= warn);
      if (t != null) {
        history.push(t);
        if (history.length > HISTORY) history.shift();
      }
      spark.set(history);
      load.set(loadPct);
      loadText.textContent = `${num(loadPct)}%`;
      detail.textContent = details;
    },
  };
}

/** CPU / GPU / memory details from LibreHardwareMonitor. */
export function tempsPanel(opts: Record<string, unknown>, _ctx: PanelContext): Widget {
  const warn = typeof opts.warn === "number" ? opts.warn : 90;
  const cpu = card("CPU");
  const gpu = card("GPU");
  const ram = bar();
  const ramText = el("hw-load-text wide");
  const offline = el("hw-offline", "LibreHardwareMonitor web server not reachable");
  const root = el(
    "temps-panel",
    el("hw-cards", cpu.root, gpu.root),
    el("hw-row hw-ram", el("hw-key", "Memory"), ram.root, ramText),
    offline,
  );

  const unsubscribe = subscribe("temps", (t: Temps | null) => {
    offline.hidden = !!t?.ok;
    if (!t?.ok) return;
    const ghz = t.cpuClock == null ? "" : ` · ${(t.cpuClock / 1000).toFixed(2)} GHz`;
    cpu.update(t.cpu, t.cpuName, t.cpuLoad, `${num(t.cpuPower)} W${ghz}`, warn);
    const vram =
      t.vramUsed == null || t.vramTotal == null
        ? ""
        : ` · VRAM ${(t.vramUsed / 1024).toFixed(1)} / ${(t.vramTotal / 1024).toFixed(1)} GB`;
    gpu.update(t.gpu, t.gpuName, t.gpuLoad, `${num(t.gpuPower)} W${vram}`, warn);
    const total = t.ramUsed != null && t.ramAvailable != null ? t.ramUsed + t.ramAvailable : null;
    ram.set(total ? ((t.ramUsed ?? 0) / total) * 100 : null);
    ramText.textContent = total ? `${num(t.ramUsed, 1)} / ${total.toFixed(1)} GB` : "–";
  });
  return { el: root, dispose: unsubscribe };
}
