import { invoke } from "@tauri-apps/api/core";
import { subscribe } from "../data";
import { el, type Widget } from "./index";

/**
 * Komorebi workspaces of this bar's monitor: dots (the focused one a wider
 * pill), numbers or names ([widgets.workspaces] show). Brighter ones have
 * windows; hide_empty leaves out empty ones but the focused. Click one to show
 * it; scroll over the widget to step through them. Hidden while komorebi isn't
 * running.
 */
export function workspaces(opts: Record<string, unknown>): Widget {
  // Our monitor as komorebi names it: "\\.\DISPLAY4" -> "DISPLAY4".
  const me = (typeof opts._device === "string" ? opts._device : "").replace(/^.*\\/, "");
  const show = opts.show === "numbers" || opts.show === "names" ? opts.show : "dots";
  const hideEmpty = opts.hide_empty === true;
  const root = el(`widget workspaces ws-${show}`);
  root.hidden = true;
  let monitor = -1;
  let focused = 0;
  let count = 0;
  const go = (workspace: number) => {
    if (monitor >= 0 && workspace !== focused) invoke("komorebi_focus", { monitor, workspace }).catch(console.error);
  };
  // Elements stay put between updates, so the focused pill can stretch across.
  const items: HTMLElement[] = [];
  const off = subscribe("workspaces", (k) => {
    const i = k ? k.monitors.findIndex((m) => m.name === me) : -1;
    // A single monitor komorebi names differently is still this one.
    monitor = i >= 0 ? i : k && k.monitors.length === 1 ? 0 : -1;
    const m = k && monitor >= 0 ? k.monitors[monitor] : null;
    root.hidden = !m;
    if (!m) return;
    focused = m.focused;
    count = m.workspaces.length;
    while (items.length < count) {
      const index = items.length;
      const b = el("ws");
      b.addEventListener("click", (e) => {
        e.stopPropagation();
        go(index);
      });
      items.push(b);
      root.append(b);
    }
    items.forEach((b, j) => {
      const w = m.workspaces[j];
      b.hidden = !w || (hideEmpty && w.windows === 0 && j !== focused);
      if (!w) return;
      b.classList.toggle("focused", j === focused);
      b.classList.toggle("busy", w.windows > 0);
      b.textContent = show === "numbers" ? String(j + 1) : show === "names" ? w.name : "";
      b.title = `${w.name}${w.windows ? ` · ${w.windows} window${w.windows === 1 ? "" : "s"}` : " · empty"}`;
    });
  });
  // Scroll: the next / previous workspace.
  root.addEventListener(
    "wheel",
    (e) => {
      if (monitor < 0 || !count) return;
      e.preventDefault();
      go((focused + (e.deltaY > 0 ? 1 : -1) + count) % count);
    },
    { passive: false },
  );
  return { el: root, dispose: off };
}
