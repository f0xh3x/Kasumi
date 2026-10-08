import { subscribe } from "../data";
import { el, Glyph, icon, type Widget } from "./index";

/** CPU · GPU temperatures from LibreHardwareMonitor. `warn` (°C) highlights hot readings. */
export function temps(opts: Record<string, unknown>): Widget {
  const warn = typeof opts.warn === "number" ? opts.warn : 90;
  const reading = (label: string) => {
    const value = document.createElement("span");
    value.className = "temp-value";
    const wrap = el("temp-reading", icon(Glyph.temp), value);
    wrap.title = label;
    return { wrap, value };
  };
  const cpu = reading("CPU");
  const gpu = reading("GPU");
  const root = el("widget temps", cpu.wrap, el("sep", "·"), gpu.wrap);

  const show = (r: { wrap: HTMLElement; value: HTMLElement }, v: number | null | undefined) => {
    r.value.textContent = v == null ? "–" : `${Math.round(v)}°`;
    r.wrap.classList.toggle("warn", v != null && v >= warn);
  };
  const unsubscribe = subscribe("temps", (t) => {
    show(cpu, t?.cpu);
    show(gpu, t?.gpu);
    root.classList.toggle("offline", !t?.ok);
    root.title = t?.ok ? "" : "LibreHardwareMonitor web server not reachable";
  });
  return { el: root, dispose: unsubscribe };
}
