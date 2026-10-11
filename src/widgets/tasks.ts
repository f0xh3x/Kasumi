import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { subscribe, type TaskApp, type TaskWindow, type Tasks } from "../data";
import { iconUrl, g, el, type PanelContext, type Widget } from "./index";

/**
 * Open apps (the windows Alt+Tab shows), one button per app.
 * Click: front it (again: minimize; several windows: cycle). Middle click: close.
 * Right click: the app's windows, a new window, close all (the tasks panel).
 *
 * Options: `monitor` = "own" (windows on this monitor, default) | "all";
 * `labels` = "none" (default) | "active" (title of the window in front);
 * `icons` = "color" (default) | "mono" | "accent" (silhouettes in the accent colour);
 * `pinning` = true (default): pinned apps show first, even when not running, and
 * the right-click menu pins / unpins (`pins` = the pinned app keys; until set,
 * the Windows taskbar's pins).
 * Hovering an app with several windows pops up its windows to pick from.
 * Icons size with the surface (`--h`), so they grow as a dock opens.
 */

/** The open tasks panel (to tell whether a right click should close it). */
let panelRoot: HTMLElement | null = null;
const panelShowing = () => {
  const panel = panelRoot?.closest<HTMLElement>(".panel");
  return !!panel && panel.style.visibility !== "hidden" && panel.style.opacity !== "0";
};

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

const EASE = "cubic-bezier(0.2, 0.8, 0.2, 1)";

/** The pin list as written in the config (app keys / dropped files' paths). */
const pinEntries = (t: Tasks | null) => (t?.apps ?? []).filter((a) => a.pinned).map((a) => a.pin ?? a.key);
const savePins = (entries: string[]) => call("config_write", { path: ["widgets", "tasks", "pins"], value: entries });

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
  const showPinned = opts.pinning !== false && opts.pinned !== false;
  const device = typeof opts._device === "string" ? opts._device : "";
  const labels = opts.labels === "active";
  const root = el("widget tasks");
  if (opts.icons === "mono" || opts.icons === "accent") root.classList.add(`icons-${opts.icons}`);
  if (opts.dim_minimized === true) root.classList.add("dim-minimized");
  // Marker glide ([animation] marker_glide): when another app comes to the front,
  // its line flows there like a drop of liquid: the front edge runs ahead (the
  // line stretches and thins), the back edge follows and catches up at the new
  // app. Both edges ease in and out and arrive together: no overshoot.
  const glides = opts._glide === true;
  if (glides) root.classList.add("glides");
  let lastActive: string | null = null;
  let glideFrame = 0;
  let glideEnd: (() => void) | null = null;
  const glideMarker = (from: { b: HTMLElement; dots: HTMLElement }, to: { b: HTMLElement; dots: HTMLElement }) => {
    // Where a line's center sits, within the widget (offsets: unaffected by a dock's scaling).
    const spot = (x: { b: HTMLElement; dots: HTMLElement }) => ({
      x: x.b.offsetLeft + x.dots.offsetLeft + x.dots.offsetWidth / 2,
      y: x.b.offsetTop + x.dots.offsetTop,
    });
    glideEnd?.();
    const a = spot(from);
    const LINE = 10;
    const THICK = 3;
    const ease = (k: number) => (k < 0.5 ? 4 * k * k * k : 1 - (-2 * k + 2) ** 3 / 2);
    const part = (t: number, from: number) => ease(Math.min(1, Math.max(0, (t - from) / 0.68)));
    to.b.classList.add("marker-away");
    const m = el("task-marker");
    root.append(m);
    const start = performance.now();
    const z0 = spot(to);
    const dur = Math.min(420, 240 + Math.abs(z0.x - a.x) * 0.45);
    glideEnd = () => {
      cancelAnimationFrame(glideFrame);
      m.remove();
      to.b.classList.remove("marker-away");
      glideEnd = null;
    };
    const frame = (now: number) => {
      const t = Math.min(1, (now - start) / dur);
      // The target is read every frame: the line lands exactly where the app's own is.
      const z = spot(to);
      const right = z.x > a.x;
      const lead = part(t, 0);
      const trail = part(t, 0.32);
      const front = right ? [a.x + LINE / 2, z.x + LINE / 2] : [a.x - LINE / 2, z.x - LINE / 2];
      const back = right ? [a.x - LINE / 2, z.x - LINE / 2] : [a.x + LINE / 2, z.x + LINE / 2];
      const f = front[0] + (front[1] - front[0]) * lead;
      const bk = back[0] + (back[1] - back[0]) * trail;
      const left = Math.min(f, bk);
      const width = Math.max(THICK, Math.abs(f - bk));
      // Stretched, it thins (down to 2 px), staying on the same baseline.
      const thick = Math.max(2, Math.min(THICK, THICK * Math.sqrt(LINE / width)));
      m.style.width = `${width.toFixed(2)}px`;
      m.style.height = `${thick.toFixed(2)}px`;
      m.style.transform = `translate(${left.toFixed(2)}px, ${(z.y + THICK - thick).toFixed(2)}px)`;
      if (t < 1) glideFrame = requestAnimationFrame(frame);
      else glideEnd?.();
    };
    glideFrame = requestAnimationFrame(frame);
  };
  // Magnify ([animation] magnify): icons grow as the cursor passes over them.
  if (opts._magnify === true) {
    root.classList.add("magnify");
    root.addEventListener("mousemove", (e) => {
      for (const ico of root.querySelectorAll<HTMLElement>(".task > .ico")) {
        const r = ico.getBoundingClientRect();
        const d = Math.abs(e.clientX - (r.left + r.width / 2));
        ico.style.scale = (1 + 0.35 * Math.max(0, 1 - d / 70)).toFixed(3);
      }
    });
    root.addEventListener("mouseleave", () => {
      for (const ico of root.querySelectorAll<HTMLElement>(".task > .ico")) ico.style.scale = "";
    });
  }
  const buttons = new Map<string, { b: HTMLElement; img: HTMLImageElement; label: HTMLElement; dots: HTMLElement }>();
  // Dragging an app to a new place: it follows the pointer, the others slide aside.
  type Drag = {
    b: HTMLElement;
    x: number;
    moved: boolean;
    /** The buttons and where they were when the drag started. */
    items: HTMLElement[];
    homes: DOMRect[];
    from: number;
    to: number;
    /** How far the others slide to make room (the dragged button's width + gap). */
    room: number;
  };
  let drag: Drag | null = null;
  let justDragged = false;
  /** The order the user dragged apps into, until the next data catches up. */
  let localOrder: string[] | null = null;

  const startDrag = (d: Drag) => {
    d.moved = true;
    d.items = [...root.children] as HTMLElement[];
    d.homes = d.items.map((c) => c.getBoundingClientRect());
    d.from = d.to = d.items.indexOf(d.b);
    const me = d.homes[d.from];
    const next = d.homes[d.from + 1];
    const prev = d.homes[d.from - 1];
    const gap = next ? next.left - me.right : prev ? me.left - prev.right : 0;
    d.room = me.width + gap;
    d.b.classList.add("dragging");
    root.classList.add("reordering");
    clearTimeout(hover.openTimer);
  };

  const moveDrag = (d: Drag, dx: number) => {
    const me = d.homes[d.from];
    // Stays within the widget, give or take a little: enough to pass the first or
    // last app's middle, so the ends can be reached.
    const over = me.width / 2 + 6;
    dx = Math.max(d.homes[0].left - me.left - over, Math.min(d.homes[d.homes.length - 1].right - me.right + over, dx));
    d.b.style.transform = `translateX(${dx}px)`;
    const centre = me.left + me.width / 2 + dx;
    let to = d.from;
    d.items.forEach((c, j) => {
      if (c === d.b) return;
      const mid = d.homes[j].left + d.homes[j].width / 2;
      let shift = 0;
      if (j > d.from && centre > mid) {
        shift = -d.room;
        to++;
      } else if (j < d.from && centre < mid) {
        shift = d.room;
        to--;
      }
      c.style.transform = shift ? `translateX(${shift}px)` : "";
    });
    d.to = to;
  };

  const endDrag = (d: Drag) => {
    justDragged = true;
    window.setTimeout(() => (justDragged = false), 0);
    // Into its new place in one frame (the others are already there), then the
    // dragged one glides from where it was let go.
    const dropped = d.b.getBoundingClientRect().left;
    root.classList.remove("reordering");
    for (const c of d.items) c.style.transform = "";
    const others = d.items.filter((c) => c !== d.b && c.isConnected);
    root.insertBefore(d.b, others[d.to] ?? null);
    const dx = dropped - d.b.getBoundingClientRect().left;
    d.b.animate([{ transform: `translateX(${dx}px)` }, { transform: "none" }], { duration: 200, easing: EASE });
    d.b.classList.remove("dragging");
    if (d.to === d.from) return;
    // New order: what's shown here, then everything else as it was.
    const shown = ([...root.children] as HTMLElement[]).map((c) => c.dataset.key!);
    const all = (latest?.apps ?? []).map((x) => x.key);
    const order = [...shown, ...all.filter((k) => !shown.includes(k))];
    localOrder = order;
    call("task_reorder", { keys: order });
    // Pinned apps keep their new order across restarts.
    const pins = pinEntries(latest);
    const byKey = new Map((latest?.apps ?? []).filter((x) => x.pinned).map((x) => [x.key, x.pin ?? x.key]));
    const reordered = order.filter((k) => byKey.has(k)).map((k) => byKey.get(k)!);
    if (showPinned && JSON.stringify(reordered) !== JSON.stringify(pins)) savePins(reordered);
  };

  const draw = () => {
    const t = latest;
    if (!t) return;
    const mine = t.windows.filter((w) => !ownMonitor || !device || w.monitor === device);
    let nowActive: string | null = null;
    let apps = t.apps.filter((a) => (showPinned && a.pinned) || mine.some((w) => w.app === a.key));
    if (localOrder) {
      const o = localOrder;
      // The new order has arrived from the backend: stop overriding it.
      const keys = t.apps.map((a) => a.key);
      if (o.filter((k) => keys.includes(k)).join() === keys.filter((k) => o.includes(k)).join()) localOrder = null;
      else apps = [...apps].sort((p, q) => (o.indexOf(p.key) + 1 || 1e9) - (o.indexOf(q.key) + 1 || 1e9));
    }
    root.hidden = apps.length === 0;
    // Where each button is now, so moves can glide instead of jump (not while dragging).
    const was = drag?.moved ? null : new Map([...buttons].map(([k, x]) => [k, x.b.getBoundingClientRect().left]));
    const rootWas = root.getBoundingClientRect().left;
    const fresh: HTMLElement[] = [];
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
        b.dataset.key = a.key;
        b.addEventListener("pointerdown", (e) => {
          if (e.button !== 0) return;
          drag = { b, x: e.clientX, moved: false, items: [], homes: [], from: 0, to: 0, room: 0 };
          b.setPointerCapture(e.pointerId);
        });
        b.addEventListener("pointermove", (e) => {
          if (drag?.b !== b) return;
          const dx = e.clientX - drag.x;
          if (!drag.moved) {
            if (Math.abs(dx) < 6) return;
            startDrag(drag);
          }
          moveDrag(drag, dx);
        });
        const release = () => {
          if (drag?.b !== b) return;
          const d = drag;
          drag = null;
          if (d.moved) endDrag(d);
        };
        b.addEventListener("pointerup", release);
        b.addEventListener("pointercancel", release);
        b.addEventListener("click", (e) => {
          e.stopPropagation();
          if (justDragged) return;
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
          clearTimeout(hover.openTimer);
          // A second right click on the same app closes its menu.
          if (menuTarget.key === a.key && !hover.open && panelShowing()) {
            hover.close?.();
            return;
          }
          // A right click opens it for good (not a hover popup).
          hover.open = false;
          menuTarget.key = a.key;
          menuTarget.listeners.forEach((f) => f());
          // Ask the overlay to open this widget's panel (the tasks panel).
          root.dispatchEvent(new CustomEvent("notch:panel", { bubbles: true }));
        });
        x = { b, img, label, dots };
        buttons.set(a.key, x);
        fresh.push(b);
      }
      if (!drag?.moved && root.children[i] !== x.b) root.insertBefore(x.b, root.children[i] ?? null);
      const wins = mine.filter((w) => w.app === a.key);
      const active = wins.find((w) => w.active);
      if (active) nowActive = a.key;
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
      const dots = Math.min(3, wins.length);
      x.dots.replaceChildren(...Array.from({ length: dots }, () => el("task-dot")));
    });
    if (glides && lastActive && nowActive && lastActive !== nowActive) {
      const from = buttons.get(lastActive);
      const to = buttons.get(nowActive);
      if (from && to && from.b.isConnected) glideMarker(from, to);
    }
    lastActive = nowActive;
    if (!was) return;
    // Moved buttons glide to their new place; new ones grow in.
    const shift = root.getBoundingClientRect().left - rootWas;
    for (const [key, x] of buttons) {
      const before = was.get(key);
      if (before === undefined) continue;
      const dx = before - shift - x.b.getBoundingClientRect().left;
      if (Math.abs(dx) > 0.5) x.b.animate([{ transform: `translateX(${dx}px)` }, { transform: "none" }], { duration: 200, easing: EASE });
    }
    if (was.size) {
      for (const b of fresh) b.animate([{ opacity: 0, transform: "scale(0.6)" }, { opacity: 1, transform: "none" }], { duration: 180, easing: EASE });
    }
  };

  // Drop programs, shortcuts or links anywhere on the bar to pin them (the bar's
  // first tasks widget takes them). WebView2 handles the drop; the files go to
  // the backend, which reads their paths and sends them back as notch://drop.
  let dropOff: (() => void) | null = null;
  const dropListeners: [string, (e: DragEvent) => void][] = [];
  if (showPinned) {
    const mine = () => document.querySelector(".widget.tasks") === root;
    const hasFiles = (e: DragEvent) => !!e.dataTransfer && [...e.dataTransfer.types].includes("Files");
    const over = (e: DragEvent) => {
      if (!mine() || !hasFiles(e)) return;
      e.preventDefault();
      e.dataTransfer!.dropEffect = "link";
      root.classList.add("drop-target");
    };
    dropListeners.push(
      ["dragenter", over],
      ["dragover", over],
      ["dragleave", (e) => {
        // Left the page (not just moved between elements).
        if (!e.relatedTarget) root.classList.remove("drop-target");
      }],
      ["drop", (e) => {
        root.classList.remove("drop-target");
        if (!mine() || !hasFiles(e)) return;
        e.preventDefault();
        const webview = (window as unknown as { chrome?: { webview?: { postMessageWithAdditionalObjects?: (m: string, o: FileList) => void } } }).chrome?.webview;
        webview?.postMessageWithAdditionalObjects?.("statusnotch-drop", e.dataTransfer!.files);
      }],
    );
    for (const [type, f] of dropListeners) document.addEventListener(type, f as EventListener);
    getCurrentWebviewWindow()
      .listen<{ kind: "drop"; paths: string[] }>("notch://drop", (e) => {
        if (!mine()) return;
        const pins = pinEntries(latest);
        const added = e.payload.paths.filter((path) => !pins.some((x) => x.toLowerCase() === path.toLowerCase()));
        invoke("fe_log", { message: `tasks: pinning ${JSON.stringify(added)}` }).catch(() => {});
        if (added.length) savePins([...pins, ...added]);
      })
      .then((f) => (dropOff = f))
      .catch((e) => console.error("drop", e));
  }

  const off = [
    () => dropOff?.(),
    () => dropListeners.forEach(([type, f]) => document.removeEventListener(type, f as EventListener)),
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
export function tasksPanel(opts: Record<string, unknown>, ctx: PanelContext): Widget {
  const root = el("tasks-panel");
  panelRoot = root;
  const pinning = opts.pinning !== false && opts.pinned !== false;
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
      const close = el("tasks-close", g(0xe711));
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
      const a = el("tasks-action", el("icon", g(glyph)), label);
      a.addEventListener("click", () => {
        run();
        ctx.close();
      });
      return a;
    };
    // Pin / unpin: StatusNotch's own pin list (starts from the Windows taskbar's).
    const pins = pinEntries(t);
    const mine = app.pin ?? app.key;
    const pinAction = pinning
      ? [
          app.pinned
            ? action("Unpin", 0xe77a, () => savePins(pins.filter((k) => k !== mine)))
            : action("Pin", 0xe718, () => savePins([...pins, app.key])),
        ]
      : [];
    root.replaceChildren(
      head,
      ...rows,
      el("tasks-sep"),
      action(wins.length ? "New window" : "Open", 0xe710, () => call("task_launch", { appKey: app.key })),
      ...pinAction,
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
