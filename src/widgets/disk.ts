import { invoke } from "@tauri-apps/api/core";
import { subscribe } from "../data";
import { el, formatBytes, meter, type Widget } from "./index";

/**
 * Disk usage, per chosen fixed drive ([widgets.disk] disks; all of them when none
 * are chosen): its letter (letters), how full it is as a bar or ring (indicator:
 * bar | circle | none, bar_length) and as text (show: percent | used | free |
 * none). Over 90% full it takes the warning colour. A click opens This PC.
 */
export function disk(opts: Record<string, unknown>): Widget {
  const chosen = Array.isArray(opts.disks) ? (opts.disks as unknown[]).filter((d): d is string => typeof d === "string") : [];
  const kind = opts.indicator === "circle" || opts.indicator === "none" ? opts.indicator : "bar";
  const show = opts.show === "used" || opts.show === "free" || opts.show === "none" ? opts.show : "percent";
  const letters = opts.letters !== false;
  const length = typeof opts.bar_length === "number" ? opts.bar_length : 30;
  const root = el("widget disk clickable");
  root.addEventListener("click", (e) => {
    e.stopPropagation();
    invoke("run_action", { action: "open", target: "shell:MyComputerFolder" }).catch(console.error);
  });
  let built = "";
  let rows: { row: HTMLElement; meter: ReturnType<typeof meter> | null; text: HTMLElement }[] = [];
  const off = subscribe("disks", (list) => {
    const disks = (list ?? []).filter((d) => !chosen.length || chosen.includes(d.id));
    const key = disks.map((d) => d.id).join();
    if (key !== built) {
      built = key;
      rows = disks.map((d) => {
        const m = kind === "none" ? null : meter(kind, length);
        const text = el("disk-text");
        const row = el("disk-item", ...(letters ? [el("disk-letter", d.id)] : []), ...(m ? [m.el] : []), ...(show === "none" ? [] : [text]));
        return { row, meter: m, text };
      });
      root.replaceChildren(...rows.map((r) => r.row));
    }
    disks.forEach((d, i) => {
      const { row, meter: m, text } = rows[i];
      const used = d.total - d.free;
      const share = d.total ? used / d.total : 0;
      m?.set(share);
      text.textContent =
        show === "used" ? `${formatBytes(used)} / ${formatBytes(d.total)}` : show === "free" ? `${formatBytes(d.free)} free` : `${Math.round(share * 100)}%`;
      row.classList.toggle("high", share >= 0.9);
      row.title = `${d.id}${d.label ? ` ${d.label}` : ""}: ${formatBytes(used)} of ${formatBytes(d.total)} used, ${formatBytes(d.free)} free`;
    });
  });
  return { el: root, dispose: off };
}
