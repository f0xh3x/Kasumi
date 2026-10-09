import "@fontsource/inter/400.css";
import "@fontsource/inter/600.css";
import "./settings.css";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  collapsible,
  editors,
  migrationOps,
  migrationOpsDisabledSpots,
  migrationOpsV3,
  migrationOpsV4,
  migrationOpsV5,
  openSection,
  scope,
  type Op,
} from "./editors";
import { $, glyph, row, toggleSwitch } from "./dom";
import { pages, type Cfg, type Field, type Page, type Path } from "./schema";

let cfg: Cfg = {};
let current = pages[0].id;


function getIn(obj: any, path: Path): unknown {
  return path.reduce((o, k) => (o == null ? undefined : o[k as any]), obj);
}

function setIn(obj: any, path: Path, value: unknown): void {
  let o = obj;
  path.slice(0, -1).forEach((k, i) => {
    if (o[k as any] == null) o[k as any] = typeof path[i + 1] === "number" ? [] : {};
    o = o[k as any];
  });
  const last = path[path.length - 1] as any;
  if (value === undefined) delete o[last];
  else o[last] = value;
}

// --- Saving --------------------------------------------------------------------

let toastTimer = 0;
function toast(text: string, error = false): void {
  const t = document.getElementById("toast")!;
  t.textContent = text;
  t.classList.toggle("error", error);
  t.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => t.classList.remove("show"), error ? 4000 : 1200);
}

/** Where the current page's computed fields resolve (null = hidden): fields that
 *  appear only with another setting (e.g. corner roundness) depend on it. */
function shownFields(): string {
  const page = pages.find((p) => p.id === current);
  if (!page) return "";
  return JSON.stringify(page.sections.flatMap((s) => s.fields.map((f) => ("path" in f && typeof f.path === "function" ? f.path(cfg) : 0))));
}

async function save(path: Path, value: unknown): Promise<void> {
  try {
    await invoke("config_write", { path, value: value === undefined ? null : value });
    const before = shownFields();
    setIn(cfg, path, value);
    toast("Saved");
    // A setting that shows or hides others: redraw (not on every save: a slider
    // being dragged would lose its grip).
    if (shownFields() !== before) render();
  } catch (e) {
    toast(String(e), true);
    render(); // show the stored value again
  }
}

async function saveMany(ops: Op[]): Promise<void> {
  if (!ops.length) return;
  try {
    await invoke("config_write_many", { ops });
    ops.forEach((o) => setIn(cfg, o.path, o.value ?? undefined));
    toast("Saved");
  } catch (e) {
    toast(String(e), true);
  }
  render(); // structured editors redraw from the new config
}

// --- Controls --------------------------------------------------------------------

const fieldRow = (f: { label: string; help?: string }, control: HTMLElement) => row(f.label, f.help, control);

function control(f: Field): HTMLElement | null {
  if (f.kind === "custom") {
    const editor = editors[f.editor];
    if (!editor) return null;
    return editor({
      cfg,
      save: async (path, value) => {
        await save(path, value === null ? undefined : value);
        render();
      },
      saveMany,
      toast,
      rerender: () => render(),
      reload: load,
      goto,
    });
  }
  if (f.kind === "autostart") {
    const sw = toggleSwitch(false, async (on) => {
      try {
        await invoke("autostart_set", { enabled: on });
        toast(on ? "Starts with Windows" : "Won't start with Windows");
      } catch (e) {
        toast(String(e), true);
      }
    });
    invoke<boolean>("autostart_get").then((on) => sw.set(on));
    return fieldRow(f, sw.el);
  }
  if (f.kind === "action") {
    const b = $("button", "button", f.button);
    let armed = false;
    b.addEventListener("click", async () => {
      if (f.confirm && !armed) {
        armed = true;
        b.textContent = f.confirm;
        b.classList.add("danger");
        setTimeout(() => {
          armed = false;
          b.textContent = f.button;
          b.classList.remove("danger");
        }, 3000);
        return;
      }
      try {
        if (f.run === "reset") {
          await invoke("config_reset");
          await load();
          toast("Reset to defaults");
        } else {
          await invoke("run_action", { action: f.run });
        }
      } catch (e) {
        toast(String(e), true);
      }
    });
    return fieldRow(f, b);
  }

  const path = typeof f.path === "function" ? f.path(cfg) : f.path;
  if (!path) return null;
  const stored = getIn(cfg, path);
  const value = stored === undefined ? f.default : stored;

  switch (f.kind) {
    case "toggle": {
      const sw = toggleSwitch(Boolean(value), (on) => save(path, on));
      return fieldRow(f, sw.el);
    }
    case "slider": {
      const input = $("input", "slider") as HTMLInputElement;
      Object.assign(input, { type: "range", min: String(f.min), max: String(f.max), step: String(f.step) });
      input.value = String(value);
      const out = $("span", "slider-value");
      const show = () => (out.textContent = `${Number(input.value)}${f.unit ?? ""}`);
      const fill = () => input.style.setProperty("--fill", `${((Number(input.value) - f.min) / (f.max - f.min)) * 100}%`);
      show();
      fill();
      input.addEventListener("input", () => {
        show();
        fill();
      });
      input.addEventListener("change", () => save(path, Number(input.value)));
      return fieldRow(f, $("div", "slider-wrap", input, out));
    }
    case "number": {
      const input = $("input", "input number") as HTMLInputElement;
      input.type = "number";
      if (f.min !== undefined) input.min = String(f.min);
      if (f.max !== undefined) input.max = String(f.max);
      if (f.step !== undefined) input.step = String(f.step);
      input.value = String(value);
      input.addEventListener("change", () => {
        const n = Number(input.value);
        if (Number.isFinite(n)) save(path, n);
      });
      const wrap = $("div", "number-wrap", input);
      if (f.unit) wrap.append($("span", "unit", f.unit));
      return fieldRow(f, wrap);
    }
    case "text": {
      const input = $("input", "input") as HTMLInputElement;
      input.value = String(value ?? "");
      if (f.placeholder) input.placeholder = f.placeholder;
      input.addEventListener("change", () => save(path, input.value));
      return fieldRow(f, input);
    }
    case "list": {
      const input = $("input", "input") as HTMLInputElement;
      input.value = Array.isArray(value) ? value.join(", ") : "";
      if (f.placeholder) input.placeholder = f.placeholder;
      input.addEventListener("change", () =>
        save(
          path,
          input.value
            .split(",")
            .map((s) => s.trim())
            .filter(Boolean),
        ),
      );
      return fieldRow(f, input);
    }
    case "select": {
      const select = $("select", "select") as HTMLSelectElement;
      for (const o of f.options) select.append(new Option(o.label, o.value));
      select.value = String(value);
      select.addEventListener("change", () => save(path, select.value));
      return fieldRow(f, select);
    }
    case "color": {
      const input = $("input", "color") as HTMLInputElement;
      input.type = "color";
      input.value = String(value);
      const hex = $("span", "color-hex", String(value));
      input.addEventListener("input", () => (hex.textContent = input.value));
      input.addEventListener("change", () => save(path, input.value));
      return fieldRow(f, $("label", "color-wrap", input, hex));
    }
  }
}


// --- Pages -----------------------------------------------------------------------

function renderPage(page: Page): HTMLElement {
  const content = $("main", "content", $("h1", "page-title", page.label));
  let group: HTMLElement | null = null;
  let last: HTMLElement | null = null;
  for (const s of page.sections) {
    // Widgets page: some sections belong to the Global view, some to a monitor's.
    if (s.scope === "global" && scope.monitor) continue;
    if (s.scope === "monitor" && !scope.monitor) continue;
    const rows = s.fields.map(control).filter((r): r is HTMLElement => r !== null);
    if (!rows.length) continue;
    if (s.collapsible && s.attach && last) {
      last.append(collapsible(s.title, $("div", "sub-body", ...rows), `${page.id}:${s.title}`, s.icon));
      continue;
    }
    if (s.collapsible) {
      // Consecutive collapsible sections share one card, one dropdown each.
      if (!group) {
        group = $("section", "card", $("h2", "card-title", "Widget settings"));
        content.append(group);
      }
      group.append(collapsible(s.title, $("div", "sub-body", ...rows), `${page.id}:${s.title}`, s.icon));
      continue;
    }
    group = null;
    const title = $("h2", "card-title", s.title);
    const badge = s.badge?.(cfg);
    if (badge) title.append($("span", "card-badge", badge));
    const card = $("section", "card", title);
    if (s.help) card.append($("p", "card-help", s.help));
    card.append(...rows);
    content.append(card);
    last = card;
  }
  return content;
}

/** Shows `page` with its `section` dropdown open and scrolled into view. */
function goto(page: string, section: string): void {
  current = page;
  if (section) openSection(`${page}:${section}`);
  render();
  const app = document.getElementById("app")!;
  const target = section && app.querySelector<HTMLElement>(`[data-key="${CSS.escape(`${page}:${section}`)}"]`);
  if (target) target.scrollIntoView({ block: "start" });
  else app.querySelector(".content")!.scrollTop = 0;
}

function render(): void {
  const app = document.getElementById("app")!;
  const scroll = app.querySelector(".content")?.scrollTop ?? 0;
  const nav = $("nav", "nav");
  for (const p of pages) {
    const item = $("button", `nav-item${p.id === current ? " active" : ""}`, glyph(p.icon), p.label);
    item.addEventListener("click", () => {
      current = p.id;
      render();
      app.querySelector(".content")!.scrollTop = 0;
    });
    nav.append(item);
  }
  const page = renderPage(pages.find((p) => p.id === current)!);
  app.replaceChildren(nav, page);
  page.scrollTop = scroll;
  invoke<string>("app_version").then((v) => nav.append($("div", "version", `v${v}`)));
}

async function load(): Promise<void> {
  try {
    cfg = await invoke<Cfg>("config_read");
    // Older configs move to the current format once: [bar] lists + [[notches]]
    // -> [layout], then per-widget notch tables -> [notch] side settings.
    let migrated = false;
    for (const step of [migrationOps, migrationOpsV3, migrationOpsV4, migrationOpsV5, migrationOpsDisabledSpots]) {
      const ops = step(cfg);
      if (!ops.length) continue;
      await invoke("config_write_many", { ops });
      cfg = await invoke<Cfg>("config_read");
      migrated = true;
    }
    if (migrated) toast("Updated your config to the new layout format");
  } catch (e) {
    toast(`Couldn't read config: ${e}`, true);
  }
  render();
}

// Custom title bar: Windows controls in the window's own style.
const win = getCurrentWindow();
document.getElementById("tb-min")!.addEventListener("click", () => win.minimize());
document.getElementById("tb-max")!.addEventListener("click", () => win.toggleMaximize());
document.getElementById("tb-close")!.addEventListener("click", () => win.close());
const syncMax = async () => {
  const max = await win.isMaximized();
  document.getElementById("tb-max")!.firstElementChild!.textContent = String.fromCharCode(max ? 0xe923 : 0xe922);
  document.body.classList.toggle("maximized", max);
};
win.onResized(syncMax);
syncMax();

document.addEventListener("contextmenu", (e) => e.preventDefault());
// Pick up edits made elsewhere (e.g. the config file in an editor).
getCurrentWindow().onFocusChanged(({ payload: focused }) => {
  if (focused && !document.activeElement?.matches("input, select")) load();
});
load();
