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
 * `labels` = "none" (default) | "active" (title of the window in front);
 * `icons` = "color" (default) | "mono" | "accent" (silhouettes in the accent colour);
 * `pinned` = true (default): Windows taskbar pins show even when not running.
 * Hovering an app with several windows pops up its windows to pick from.
 * Icons size with the surface (`--h`), so they grow as a dock opens.
 */

/** The app the tasks panel is about (set on right click / hover). */
const menuTarget = { key: "", listeners: new Set<() => void>() };

// Hover popup: opened over an app with several windows, closed shortly after the
// cursor leaves both the app and the popup.
const hover = { openTimer: 0, closeTimer: 0, open: false, close: null as null | (() => void) };
const HOVER_OPEN_MS = 450;
const HOVER_CLOSE_MS = 350;
function scheduleHoverClose(): void {
  clearTimeout(hover.closeTimer);
  hover.closeTimer = window.setTimeout(() => {
    if (hover.open) {
      hover.open = false;
      hover.close?.();
    }
  }, HOVER_CLOSE_MS);
}

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
  const showPinned = opts.pinned !== false;
  const device = typeof opts._device === "string" ? opts._device : "";
  const labels = opts.labels === "active";
  const root = el("widget tasks");
  if (opts.icons === "mono" || opts.icons === "accent") root.classList.add(`icons-${opts.icons}`);
  const buttons = new Map<string, { b: HTMLElement; img: HTMLImageElement; label: HTMLElement; dots: HTMLElement }>();

  const draw = () => {
    const t = latest;
    if (!t) return;
    const mine = t.windows.filter((w) => !ownMonitor || !device || w.monitor === device);
    const apps = t.apps.filter((a) => (showPinned && a.pinned) || mine.some((w) => w.app === a.key));
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
        const b = el("task", el("ico", img, el("ico-mask")), label, dots);
        b.addEventListener("click", (e) => {
          e.stopPropagation();
          const wins = (latest?.windows ?? []).filter((w) => w.app === a.key && (!ownMonitor || !device || w.monitor === device));
          const next = nextWindow(wins);
          if (next) call("task_activate", { hwndId: next.hwnd, toggle: next.toggle });
          // A pinned app that isn't running: start it.
          else call("task_launch", { appKey: a.key });
        });
        b.addEventListener("mouseenter", () => {
          clearTimeout(hover.closeTimer);
          clearTimeout(hover.openTimer);
          hover.openTimer = window.setTimeout(() => {
            const wins = (latest?.windows ?? []).filter((w) => w.app === a.key && (!ownMonitor || !device || w.monitor === device));
            if (wins.length < 2) return;
            menuTarget.key = a.key;
            menuTarget.listeners.forEach((f) => f());
            hover.open = true;
            root.dispatchEvent(new CustomEvent("notch:panel", { bubbles: true }));
          }, HOVER_OPEN_MS);
        });
        b.addEventListener("mouseleave", () => {
          clearTimeout(hover.openTimer);
          scheduleHoverClose();
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
          // A right click opens it for good (not a hover popup).
          clearTimeout(hover.openTimer);
          hover.open = false;
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
      x.b.classList.toggle("idle", wins.length === 0);
      x.b.classList.toggle("minimized", wins.length > 0 && wins.every((w) => w.minimized));
      x.b.title = wins.length > 1 ? "" : wins.length ? wins[0].title : a.name;
      const icon = icons[a.key];
      const url = icon ? iconUrl(icon) : "";
      if (x.img.src !== url) {
        if (url) x.img.src = url;
        else x.img.removeAttribute("src");
        // The accent style paints the icon's silhouette through a mask.
        (x.img.parentElement as HTMLElement).style.setProperty("--icon", url ? `url("${url}")` : "none");
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

/**
 * A window title without the app's name at its end ("Docs - File Explorer" ->
 * "Docs"); a folder path shows just the folder ("C:\work\Stand Up" -> "Stand Up").
 */
function shortTitle(title: string, app: string): string {
  let t = title;
  for (const sep of [" - ", " — ", " – "]) {
    if (t.toLowerCase().endsWith(`${sep}${app}`.toLowerCase())) t = t.slice(0, t.length - sep.length - app.length);
  }
  if (/^[a-z]:\\/i.test(t) || t.startsWith("\\\\")) {
    const parts = t.split("\\").filter(Boolean);
    t = parts[parts.length - 1] ?? t;
  }
  return t.trim() || title;
}

/** Right-click (and hover) panel: the app's windows, a new window, close all. */
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
      const title = el("tasks-title", shortTitle(w.title, app.name));
      title.title = w.title;
      const row = el(`tasks-row${w.active ? " active" : ""}`, title, close);
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
  hover.close = ctx.close;
  root.addEventListener("mouseenter", () => clearTimeout(hover.closeTimer));
  root.addEventListener("mouseleave", () => {
    if (hover.open) scheduleHoverClose();
  });
  draw();
  return {
    el: root,
    dispose: () => {
      menuTarget.listeners.delete(draw);
      off();
    },
  };
}
