import { invoke } from "@tauri-apps/api/core";
import { subscribe, type TrayIcon } from "../data";
import { el, icon, type PanelContext, type Widget } from "./index";

const CHEVRON_DOWN = String.fromCharCode(0xe70d);

/** Bar button that drops the tray panel (YASB's label_collapsed glyph). */
export function trayButton(): Widget {
  const root = el("widget tray-button", icon(CHEVRON_DOWN));
  const unsubscribe = subscribe("tray", (icons) => {
    root.title = icons?.length ? `${icons.length} tray icons` : "No tray icons";
  });
  return { el: root, dispose: unsubscribe };
}

/** RGBA from Rust → PNG data URL, cached per pixel payload. */
const urls = new Map<string, string>();
export function iconUrl(i: Pick<TrayIcon, "w" | "h" | "rgba">): string {
  const cached = urls.get(i.rgba);
  if (cached) return cached;
  const bytes = Uint8ClampedArray.from(atob(i.rgba), (c) => c.charCodeAt(0));
  const canvas = document.createElement("canvas");
  canvas.width = i.w;
  canvas.height = i.h;
  canvas.getContext("2d")!.putImageData(new ImageData(bytes, i.w, i.h), 0, 0);
  const url = canvas.toDataURL();
  if (urls.size > 200) urls.clear();
  urls.set(i.rgba, url);
  return url;
}

/** Grid of hosted notification-area icons; clicks go to the owning app. */
export function trayPanel(opts: Record<string, unknown>, ctx: PanelContext): Widget {
  const perRow = typeof opts.icons_per_row === "number" ? opts.icons_per_row : 5;
  const grid = el("tray-grid");
  grid.style.gridTemplateColumns = `repeat(${perRow}, 32px)`;
  const empty = el("tray-empty", "No tray icons");
  const root = el("tray-panel", grid, empty);

  const click = (id: string, button: "left" | "right" | "double", target: HTMLElement) => {
    const r = target.getBoundingClientRect();
    invoke("tray_click", { id, button, x: r.left + r.width / 2, y: r.top + r.height / 2 }).catch((e) =>
      console.error("tray_click", e),
    );
    if (button !== "double") ctx.close();
  };

  // Cells are kept per icon id and patched in place, so updates (animated icons,
  // changing tooltips) don't rebuild the grid or flicker.
  const cells = new Map<string, { cell: HTMLElement; img: HTMLImageElement; rgba: string }>();
  let lastCount = -1;
  const unsubscribe = subscribe("tray", (icons) => {
    const list = icons ?? [];
    const seen = new Set<string>();
    for (const i of list) {
      seen.add(i.id);
      let c = cells.get(i.id);
      if (!c) {
        const cell = el("tray-icon");
        const img = document.createElement("img");
        img.draggable = false;
        cell.append(img);
        cell.addEventListener("click", () => click(i.id, "left", cell));
        cell.addEventListener("dblclick", () => click(i.id, "double", cell));
        cell.addEventListener("contextmenu", (e) => {
          e.preventDefault();
          click(i.id, "right", cell);
        });
        c = { cell, img, rgba: "" };
        cells.set(i.id, c);
      }
      if (c.rgba !== i.rgba) {
        c.img.src = iconUrl(i);
        c.rgba = i.rgba;
      }
      c.cell.title = i.tip || i.exe;
      grid.append(c.cell); // keeps order; moving an existing node is cheap
    }
    for (const [id, c] of cells) {
      if (!seen.has(id)) {
        c.cell.remove();
        cells.delete(id);
      }
    }
    empty.hidden = list.length > 0;
    if (list.length !== lastCount) {
      lastCount = list.length;
      ctx.resize();
    }
  });
  return { el: root, dispose: unsubscribe };
}
