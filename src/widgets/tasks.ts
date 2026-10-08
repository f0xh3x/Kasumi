import { invoke } from "@tauri-apps/api/core";
import { subscribe, type TaskApp, type TaskWindow, type Tasks } from "../data";
import { el, type PanelContext, type Widget } from "./index";
import { iconUrl } from "./tray";

/**
 * Open apps (the windows Alt+Tab shows), one button per app.
 * Click: front it (again: minimize; several windows: cycle). Middle click: close.
 * Right click: the app's windows, a new window, close all (the tasks panel).
 *
 * Options: `monitor` = "own" (windows on this monitor, default) | "all";
 * `labels` = "none" (default) | "active" (title of the window in front).
 * Icons size with the surface (`--h`), so they grow as a dock opens.
 */

/** The app the tasks panel is about (set on right click). */
const menuTarget = { key: "", listeners: new Set<() => void>() };

let latest: Tasks | null = null;
let icons: Record<string, { w: number; h: number; rgba: string }> = {};

const call = (cmd: string, args: Record<string, unknown>) => invoke(cmd, args).catch((e) => console.error(cmd, e));

/** The window a click on an app should front: cycle when one of its windows is in front. */
function nextWindow(wins: TaskWindow[]): { hwnd: number; toggle: boolean } | null {
  if (!wins.length) return null;
  if (wins.length === 1) return { hwnd: wins[0].hwnd, toggle: true };
  const i = wins.findIndex((w) => w.active);
  if (i < 0) return { hwnd: [...wins].sort((a, b) => a.z - b.z)[0].hwnd, toggle: false };
  return { hwnd: wins[(i + 1) % wins.length].hwnd, toggle: false };
}

export function tasks(opts: Record<string, unknown>): Widget {
  const ownMonitor = opts.monitor !== "all";
  const device = typeof opts._device === "string" ? opts._device : "";
  const labels = opts.labels === "active";
  const root = el("widget tasks");
  const buttons = new Map<string, { b: HTMLElement; img: HTMLImageElement; label: HTMLElement; dots: HTMLElement }>();

  const draw = () => {
    const t = latest;
    if (!t) return;
    const mine = t.windows.filter((w) => !ownMonitor || !device || w.monitor === device);
    const apps = t.apps.filter((a) => mine.some((w) => w.app === a.key));
    root.hidden = apps.length === 0;
    for (const [key, x] of buttons) {
      if (!apps.some((a) => a.key === key)) {
        x.b.remove();
        buttons.delete(key);
      }
    }
    apps.forEach((a, i) => {
      let x = buttons.get(a.key);
      if (!x) {
        const img = document.createElement("img");
        img.draggable = false;
        const label = el("task-label");
        const dots = el("task-dots");
        const b = el("task", img, label, dots);
        b.addEventListener("click", (e) => {
          e.stopPropagation();
          const wins = (latest?.windows ?? []).filter((w) => w.app === a.key && (!ownMonitor || !device || w.monitor === device));
          const next = nextWindow(wins);
          if (next) call("task_activate", { hwndId: next.hwnd, toggle: next.toggle });
        });
        b.addEventListener("auxclick", (e) => {
          if (e.button !== 1) return;
          e.preventDefault();
          e.stopPropagation();
          const wins = (latest?.windows ?? []).filter((w) => w.app === a.key && (!ownMonitor || !device || w.monitor === device));
          const front = [...wins].sort((p, q) => p.z - q.z)[0];
          if (front) call("task_close", { hwndId: front.hwnd });
        });
        b.addEventListener("contextmenu", (e) => {
          e.preventDefault();
          e.stopPropagation();
          menuTarget.key = a.key;
          menuTarget.listeners.forEach((f) => f());
          // Ask the overlay to open this widget's panel (the tasks panel).
          root.dispatchEvent(new CustomEvent("notch:panel", { bubbles: true }));
        });
        x = { b, img, label, dots };
        buttons.set(a.key, x);
      }
      if (root.children[i] !== x.b) root.insertBefore(x.b, root.children[i] ?? null);
      const wins = mine.filter((w) => w.app === a.key);
      const active = wins.find((w) => w.active);
      x.b.classList.toggle("active", !!active);
      x.b.classList.toggle("minimized", wins.every((w) => w.minimized));
      x.b.title = wins.map((w) => w.title).join("\n");
      const icon = icons[a.key];
      const url = icon ? iconUrl(icon) : "";
      if (x.img.src !== url) {
        if (url) x.img.src = url;
        else x.img.removeAttribute("src");
      }
      x.img.classList.toggle("missing", !url);
      x.label.textContent = labels && active ? active.title : "";
      x.label.hidden = !(labels && active);
      x.dots.replaceChildren(...wins.slice(0, 3).map(() => el("task-dot")));
    });
  };

  const off = [
    subscribe("tasks", (t) => {
      latest = t;
      draw();
    }),
    subscribe("taskIcons", (i) => {
      icons = i ?? {};
      draw();
    }),
  ];
  return { el: root, dispose: () => off.forEach((f) => f()), panelTrigger: "event" };
}

/** Right-click panel: the app's windows, a new window, close all. */
export function tasksPanel(_opts: Record<string, unknown>, ctx: PanelContext): Widget {
  const root = el("tasks-panel");
  const draw = () => {
    const t = latest;
    const app: TaskApp | undefined = t?.apps.find((a) => a.key === menuTarget.key);
    if (!t || !app) {
      root.replaceChildren(el("tasks-empty", "No windows"));
      return;
    }
    const wins = t.windows.filter((w) => w.app === app.key);
    const icon = icons[app.key];
    const head = el("tasks-head");
    if (icon) {
      const img = document.createElement("img");
      img.src = iconUrl(icon);
      head.append(img);
    }
    head.append(el("tasks-name", app.name));
    const rows = wins.map((w) => {
      const close = el("tasks-close", String.fromCharCode(0xe711));
      close.title = "Close window";
      close.addEventListener("click", (e) => {
        e.stopPropagation();
        call("task_close", { hwndId: w.hwnd });
      });
      const row = el(`tasks-row${w.active ? " active" : ""}`, el("tasks-title", w.title), close);
      row.addEventListener("click", () => {
        call("task_activate", { hwndId: w.hwnd, toggle: false });
        ctx.close();
      });
      return row;
    });
    const action = (label: string, glyph: number, run: () => void) => {
      const a = el("tasks-action", el("icon", String.fromCharCode(glyph)), label);
      a.addEventListener("click", () => {
        run();
        ctx.close();
      });
      return a;
    };
    root.replaceChildren(
      head,
      ...rows,
      el("tasks-sep"),
      action("New window", 0xe710, () => call("task_launch", { appKey: app.key })),
      ...(wins.length
        ? [action(wins.length > 1 ? "Close all windows" : "Close window", 0xe711, () => wins.forEach((w) => call("task_close", { hwndId: w.hwnd })))]
        : []),
    );
    ctx.resize();
  };
  menuTarget.listeners.add(draw);
  const off = subscribe("tasks", () => draw());
  draw();
  return {
    el: root,
    dispose: () => {
      menuTarget.listeners.delete(draw);
      off();
    },
  };
}
