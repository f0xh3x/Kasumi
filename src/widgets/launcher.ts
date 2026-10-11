import { invoke } from "@tauri-apps/api/core";
import { POWER_ACTIONS } from "../power-actions";
import { appIconNow, appIcon, confirmClick, g, el, icon, type PanelContext, type Widget } from "./index";

/**
 * The app launcher. The widget is a search icon that morphs into a text field
 * while the launcher is open; the panel (dropping out of its notch / bar) lists
 * the matching apps. Without the widget, the hotkey opens the panel with a field
 * of its own (`_ownField`). ↑/↓ choose, Enter launches, Esc closes; the launcher
 * also closes when focus goes elsewhere.
 */

interface App {
  id: string;
  name: string;
}


const S = {
  open: false,
  query: "",
  selected: 0,
  apps: [] as App[],
  results: [] as App[],
  /** How many results show ([widgets.launcher] results). */
  max: 8,
  listeners: new Set<() => void>(),
  fields: new Set<HTMLInputElement>(),
  /** Closed, but still showing in the dock while it collapses (launcherHold). */
  hold: false,
};
const notify = () => S.listeners.forEach((f) => f());

/** Higher is better; -1 = no match. */
function score(name: string, q: string): number {
  const n = name.toLowerCase();
  if (n.startsWith(q)) return 100 - n.length * 0.01;
  const words = n.split(/[\s\-_.()]+/).filter(Boolean);
  if (words.some((w) => w.startsWith(q))) return 80 - n.length * 0.01;
  // Initials, e.g. "vsc" -> "Visual Studio Code".
  if (words.map((w) => w[0]).join("").startsWith(q)) return 70;
  const i = n.indexOf(q);
  return i >= 0 ? 60 - i * 0.1 : -1;
}

function search(): void {
  const q = S.query.trim().toLowerCase();
  S.results = q
    ? S.apps
        .map((a) => [a, score(a.name, q)] as const)
        .filter(([, s]) => s >= 0)
        .sort((a, b) => b[1] - a[1])
        .slice(0, S.max)
        .map(([a]) => a)
    : [];
  S.selected = 0;
  notify();
}

async function loadApps(): Promise<void> {
  try {
    S.apps = await invoke<App[]>("launcher_apps");
    search();
  } catch (e) {
    console.error("launcher_apps", e);
  }
}

function close(refocus: boolean): void {
  if (S.open) invoke("launcher_close", { refocus }).catch(console.error);
}

function launch(app: App | undefined): void {
  if (!app) return;
  invoke("launcher_launch", { id: app.id }).catch(console.error);
  close(false);
}

/** The signed-in user (name, account picture), asked for once. */
let user: Promise<{ name: string; picture: string | null }> | null = null;
const userInfo = () =>
  (user ??= invoke<{ name: string; picture: string | null }>("launcher_user").catch(() => ({ name: "", picture: null })));

const bottomEdge = () => document.documentElement.classList.contains("edge-bottom");

function bindField(input: HTMLInputElement): void {
  input.type = "text";
  input.spellcheck = false;
  input.autocomplete = "off";
  input.placeholder = "Search apps";
  S.fields.add(input);
  input.addEventListener("input", () => {
    S.query = input.value;
    for (const f of S.fields) if (f !== input) f.value = input.value;
    search();
  });
  input.addEventListener("keydown", (e) => {
    // On a bottom dock the list grows upwards from the field: up moves away from it.
    const away = bottomEdge() ? "ArrowUp" : "ArrowDown";
    const toward = bottomEdge() ? "ArrowDown" : "ArrowUp";
    if (e.key === away || e.key === toward) {
      e.preventDefault();
      const n = S.results.length;
      if (n) S.selected = (S.selected + (e.key === away ? 1 : n - 1)) % n;
      notify();
    } else if (e.key === "Enter") {
      e.preventDefault();
      launch(S.results[S.selected]);
    } else if (e.key === "Escape") {
      e.preventDefault();
      close(true);
    }
  });
}

/**
 * Keeps a closed launcher showing in its notch (the dock's other items still
 * aside), so a dock that's closing too shrinks in one motion instead of first
 * growing back to its items' size. The overlay lets go once the dock has closed.
 */
export function launcherHold(on: boolean): void {
  if (S.hold === on) return;
  S.hold = on;
  notify();
}

/** Called by the overlay when Rust opens or closes the launcher on it. */
export function launcherSetOpen(open: boolean): void {
  S.open = open;
  S.query = "";
  for (const f of S.fields) f.value = "";
  search();
  if (open) {
    void loadApps();
    // Focus whichever field is showing (the widget's, else the panel's own).
    window.setTimeout(() => {
      const field = [...S.fields].find((f) => f.offsetParent !== null) ?? [...S.fields][0];
      field?.focus();
    }, 60);
  }
  notify();
}

// Clicking another app (or anywhere outside) closes the launcher.
window.addEventListener("blur", () => close(false));

/** Results shown, from the widget's options (3–20, default 8). */
function setMax(opts: Record<string, unknown>): void {
  const n = Number(opts.results);
  if (Number.isFinite(n) && n > 0) S.max = Math.min(20, Math.max(3, Math.round(n)));
}

export function launcher(opts: Record<string, unknown>): Widget {
  setMax(opts);
  const field = document.createElement("input");
  field.className = "launcher-field";
  bindField(field);
  const root = el("widget launcher-w", icon(g(0xe721)), field);
  root.title = "Search apps";
  root.addEventListener("click", (e) => {
    e.stopPropagation();
    if (!S.open) invoke("launcher_open").catch(console.error);
  });
  const sync = () => root.classList.toggle("searching", S.open || S.hold);
  S.listeners.add(sync);
  sync();
  return {
    el: root,
    // Rust opens the panel (it pins the bar and gives the overlay focus first).
    panelTrigger: "event",
    dispose: () => {
      S.listeners.delete(sync);
      S.fields.delete(field);
    },
  };
}

export function launcherPanel(opts: Record<string, unknown>, ctx: PanelContext): Widget {
  setMax(opts);
  const list = el("launcher-list");
  const hint = el("launcher-hint");
  const parts: HTMLElement[] = [];
  let own: HTMLInputElement | null = null;
  if (opts._ownField) {
    own = document.createElement("input");
    own.className = "launcher-own-field";
    bindField(own);
    parts.push(el("launcher-own", icon(g(0xe721)), own));
  }
  // The row at the end away from the bar, like a start menu's: the user (button
  // to the account page, [widgets.launcher] user) and the power buttons
  // ([widgets.launcher] power, aligned by power_align); restart / shut down /
  // sign out need a second click, which the note asks for.
  const chosen = Array.isArray(opts.power) ? (opts.power as unknown[]).filter((p): p is string => typeof p === "string" && p in POWER_ACTIONS) : [];
  const showUser = opts.user === true;
  const align = opts.power_align === "left" || opts.power_align === "center" ? opts.power_align : "right";
  const disposers: (() => void)[] = [];
  const note = el("launcher-power-note");
  const buttons = el("launcher-power-buttons");
  const power = el(`launcher-power align-${align}`);
  if (showUser) {
    const pic = el("launcher-user-pic", icon(g(0xe77b)));
    const name = el("launcher-user-name");
    const user = el("launcher-user clickable", pic, name);
    user.title = "Your account";
    user.addEventListener("click", () => {
      invoke("run_action", { action: "open", target: "ms-settings:yourinfo" }).catch(console.error);
      close(false);
    });
    userInfo().then((u) => {
      name.textContent = u.name;
      if (u.picture) {
        const img = document.createElement("img");
        img.src = u.picture;
        img.draggable = false;
        pic.replaceChildren(img);
      }
    });
    power.append(user);
  }
  power.append(note, buttons, el("launcher-power-tail"));
  for (const id of Object.keys(POWER_ACTIONS).filter((p) => chosen.includes(p))) {
    const [label, glyph, confirm] = POWER_ACTIONS[id];
    const b = el("launcher-power-button clickable", icon(g(glyph)));
    b.title = label;
    disposers.push(
      confirmClick(
        b,
        confirm,
        () => {
          invoke("run_action", { action: id, target: null }).catch(console.error);
          close(false);
        },
        (on) => (note.textContent = on ? `Click again to ${label.toLowerCase()}` : ""),
      ),
    );
    buttons.append(b);
  }
  power.hidden = !chosen.length && !showUser;
  const root = el("launcher-panel", ...parts, list, hint, power);
  // [widgets.launcher] panel_width.
  if (typeof opts.panel_width === "number") root.style.width = `${Math.min(700, Math.max(240, opts.panel_width))}px`;
  let rows = -1;
  const render = () => {
    hint.textContent = S.query.trim() && !S.results.length ? "No apps found" : "";
    hint.hidden = !hint.textContent;
    list.replaceChildren(
      ...S.results.map((a, i) => {
        const img = document.createElement("img");
        img.draggable = false;
        const url = appIconNow(a.id);
        if (url) img.src = url;
        else if (url === undefined) appIcon(a.id).then((u) => u && notify());
        const row = el(`launcher-row${i === S.selected ? " selected" : ""}`, img, el("launcher-name", a.name));
        row.addEventListener("mousemove", () => {
          if (S.selected !== i) {
            S.selected = i;
            notify();
          }
        });
        row.addEventListener("click", () => launch(a));
        return row;
      }),
    );
    if (S.results.length !== rows) {
      rows = S.results.length;
      ctx.resize();
    }
  };
  S.listeners.add(render);
  render();
  return {
    el: root,
    dispose: () => {
      S.listeners.delete(render);
      if (own) S.fields.delete(own);
      disposers.forEach((d) => d());
    },
  };
}
