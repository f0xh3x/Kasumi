import { invoke } from "@tauri-apps/api/core";
import { subscribe } from "../data";
import { el, icon, type PanelContext, type Widget } from "./index";
import { iconUrl } from "./tray";

const g = (c: number) => String.fromCharCode(c);

/** The window in front: its title, its app's name, or both. */
export function focus(opts: Record<string, unknown>): Widget {
  const show = opts.show === "app" || opts.show === "both" ? opts.show : "title";
  const maxWidth = typeof opts.max_width === "number" ? opts.max_width : 300;
  const root = el("widget focus-title");
  root.style.maxWidth = `${maxWidth}px`;
  root.hidden = true;
  const off = subscribe("focus", (f) => {
    root.hidden = !f;
    if (!f) return;
    root.textContent = show === "app" ? f.app : show === "both" && f.app ? `${f.app} — ${f.title}` : f.title;
    root.title = f.title;
  });
  return { el: root, dispose: off };
}

/** Bytes per second as "1.2 MB/s" (or bits: "9.6 Mb/s"). */
function rate(bytes: number, bits: boolean): string {
  let v = bits ? bytes * 8 : bytes;
  const units = bits ? ["b/s", "Kb/s", "Mb/s", "Gb/s"] : ["B/s", "KB/s", "MB/s", "GB/s"];
  let i = 0;
  while (v >= 1000 && i < units.length - 1) {
    v /= 1000;
    i++;
  }
  return `${v < 10 && i > 0 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

/** Live download / upload rate. */
export function netspeed(opts: Record<string, unknown>): Widget {
  const show = opts.show === "down" || opts.show === "up" ? opts.show : "both";
  const bits = opts.units === "bits";
  const part = (glyph: number) => {
    const value = el("netspeed-value");
    return { wrap: el("netspeed-part", icon(g(glyph)), value), value };
  };
  const down = part(0xe74b);
  const up = part(0xe74a);
  const root = el("widget netspeed", ...(show !== "up" ? [down.wrap] : []), ...(show !== "down" ? [up.wrap] : []));
  const off = subscribe("netspeed", (n) => {
    down.value.textContent = rate(n?.down ?? 0, bits);
    up.value.textContent = rate(n?.up ?? 0, bits);
  });
  return { el: root, dispose: off };
}

const size = (bytes: number) =>
  bytes >= 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : bytes >= 1e6 ? `${Math.round(bytes / 1e6)} MB` : `${Math.round(bytes / 1e3)} KB`;

/** Recycle bin: click opens it, right-click empties it (Windows asks first). */
export function recycle(opts: Record<string, unknown>): Widget {
  const count = el("recycle-count");
  const root = el("widget recycle clickable", icon(g(0xe74d)), ...(opts.count === true ? [count] : []));
  root.addEventListener("click", (e) => {
    e.stopPropagation();
    invoke("run_action", { action: "open", target: "shell:RecycleBinFolder" }).catch(console.error);
  });
  root.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    e.stopPropagation();
    invoke("recycle_empty").catch(console.error);
  });
  const off = subscribe("recycle", (r) => {
    const items = r?.items ?? 0;
    root.classList.toggle("empty", items === 0);
    count.textContent = items ? String(items) : "";
    root.title = items ? `Recycle bin: ${items} item${items === 1 ? "" : "s"}, ${size(r?.bytes ?? 0)} — right-click to empty` : "Recycle bin is empty";
  });
  return { el: root, dispose: off };
}

const DEVICES: [key: "mic" | "camera" | "location", glyph: number, label: string][] = [
  ["mic", 0xe720, "Microphone"],
  ["camera", 0xe714, "Camera"],
  ["location", 0xe81d, "Location"],
];

/** Microphone / camera / location in use: an icon each, only while some app uses it. */
export function privacy(opts: Record<string, unknown>): Widget {
  const watched = DEVICES.filter(([key]) => opts[key] !== false);
  const icons = new Map(watched.map(([key, glyph]) => [key, icon(g(glyph))]));
  const root = el("widget privacy", ...icons.values());
  root.hidden = true;
  const off = subscribe("privacy", (p) => {
    let any = false;
    const lines: string[] = [];
    for (const [key, , label] of watched) {
      const apps = p?.[key] ?? [];
      icons.get(key)!.hidden = apps.length === 0;
      if (apps.length) {
        any = true;
        lines.push(`${label}: ${apps.join(", ")}`);
      }
    }
    root.hidden = !any;
    root.title = lines.join("\n");
  });
  return { el: root, dispose: off };
}

/** Which apps use what. */
export function privacyPanel(_opts: Record<string, unknown>, ctx: PanelContext): Widget {
  const root = el("privacy-panel");
  const off = subscribe("privacy", (p) => {
    const rows = DEVICES.flatMap(([key, glyph, label]) =>
      (p?.[key] ?? []).map((app) => el("privacy-row", icon(g(glyph)), el("privacy-app", app), el("privacy-what", label))),
    );
    root.replaceChildren(...(rows.length ? rows : [el("privacy-none", "Nothing is using the microphone, camera or location.")]));
    ctx.resize();
  });
  return { el: root, dispose: off };
}

/** Clipboard history: the icon (+ count); its panel lists recent copies to paste again. */
export function clipboard(opts: Record<string, unknown>): Widget {
  const count = el("clip-count");
  const root = el("widget clipboard", icon(g(0xe77f)), ...(opts.count === true ? [count] : []));
  const off = subscribe("clipboard", (items) => {
    count.textContent = items?.length ? String(items.length) : "";
    root.title = "Clipboard history";
  });
  return { el: root, dispose: off };
}

export function clipboardPanel(_opts: Record<string, unknown>, ctx: PanelContext): Widget {
  const list = el("clip-list");
  const clear = el("clip-clear", "Clear");
  clear.addEventListener("click", (e) => {
    e.stopPropagation();
    invoke("clipboard_clear").catch(console.error);
  });
  const root = el("clip-panel", list, clear);
  const off = subscribe("clipboard", (items) => {
    const rows = (items ?? []).map((it) => {
      const body =
        it.kind === "image"
          ? it.thumb
            ? (() => {
                const img = document.createElement("img");
                img.src = iconUrl(it.thumb);
                img.draggable = false;
                return el("clip-image", img);
              })()
            : el("clip-text dim", "Image")
          : el("clip-text", it.text ?? "");
      const remove = el("clip-remove", icon(g(0xe711)));
      remove.title = "Remove";
      remove.addEventListener("click", (e) => {
        e.stopPropagation();
        invoke("clipboard_remove", { id: it.id }).catch(console.error);
      });
      const row = el("clip-row", body, remove);
      row.addEventListener("click", (e) => {
        e.stopPropagation();
        ctx.close();
        invoke("clipboard_paste", { id: it.id }).catch(console.error);
      });
      return row;
    });
    list.replaceChildren(...(rows.length ? rows : [el("clip-empty", "Copy something and it shows up here.")]));
    clear.hidden = !rows.length;
    ctx.resize();
  });
  return { el: root, dispose: off };
}
