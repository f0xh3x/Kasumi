import { invoke } from "@tauri-apps/api/core";
import { el, icon, type PanelContext, type Widget } from "./index";
import { iconUrl } from "./tray";

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

const MAX_RESULTS = 8;
const g = (c: number) => String.fromCharCode(c);

const S = {
  open: false,
  query: "",
  selected: 0,
  apps: [] as App[],
  results: [] as App[],
  listeners: new Set<() => void>(),
  fields: new Set<HTMLInputElement>(),
};
const notify = () => S.listeners.forEach((f) => f());
const icons = new Map<string, string>();
const pending = new Set<string>();

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
        .slice(0, MAX_RESULTS)
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

export function launcher(_opts: Record<string, unknown>): Widget {
  const field = document.createElement("input");
  field.className = "launcher-field";
  bindField(field);
  const root = el("widget launcher-w", icon(g(0xe721)), field);
  root.title = "Search apps";
  root.addEventListener("click", (e) => {
    e.stopPropagation();
    if (!S.open) invoke("launcher_open").catch(console.error);
  });
  const sync = () => root.classList.toggle("searching", S.open);
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
  const root = el("launcher-panel", ...parts, list, hint);
  let rows = -1;
  const render = () => {
    hint.textContent = S.query.trim() ? (S.results.length ? "" : "No apps found") : "Type to search apps";
    hint.hidden = !hint.textContent;
    list.replaceChildren(
      ...S.results.map((a, i) => {
        const img = document.createElement("img");
        img.draggable = false;
        const url = icons.get(a.id);
        if (url) img.src = url;
        else if (!pending.has(a.id)) {
          pending.add(a.id);
          invoke<{ w: number; h: number; rgba: string }>("launcher_icon", { id: a.id })
            .then((i) => {
              icons.set(a.id, iconUrl(i));
              notify();
            })
            .catch(() => {});
        }
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
    },
  };
}
