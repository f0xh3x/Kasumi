import "@fontsource/inter/400.css";
import "@fontsource/inter/600.css";
import "./settings.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  collapsible,
  editors,
  migrationOps,
  migrationOpsDisabledSpots,
  migrationOpsRemovedWidgets,
  migrationOpsV3,
  migrationOpsV4,
  migrationOpsV5,
  isPlaced,
  openSection,
  scope,
  shownSurfaces,
  surfaceIds,
  surfaceName,
  type Op,
} from "./editors";
import { $, glyph, row, segmented, toggleSwitch } from "./dom";
import { kasumiLogo } from "../quick-tiles";
import { areaOf, areas, pageAliases, pages, type Cfg, type Field, type Page, type Path, type Section } from "./schema";
import { renderNotes, renderSetup, type Host } from "./setup";

/** This window: settings, or the first-time setup / update notes (settings.rs). */
const VIEW = new URLSearchParams(location.search).get("view");
if (VIEW) document.body.classList.add(`view-${VIEW}`);

let cfg: Cfg = {};
/** Cards whose More is open (remembered; a card's key: page and title). */
const moreOpen = new Set<string>(JSON.parse(localStorage.getItem("settings-more") ?? "[]"));
const saveMore = () => localStorage.setItem("settings-more", JSON.stringify([...moreOpen]));

/** A setting's row with its tag (what it changes or needs) by its name. */
function fieldControl(f: Field): HTMLElement | null {
  const r = control(f);
  if (r && "tag" in f && f.tag) r.querySelector(".row-label")?.append($("span", "row-tag", f.tag));
  // Changed from the default: a dot that puts it back.
  const path = r && changedPath(f);
  if (path) {
    const dot = $("button", "changed-dot");
    const what = "default" in f ? String(f.default === true ? "on" : f.default === false ? "off" : f.default) : "";
    dot.title = `Changed. Click to put back the default${what ? ` (${what})` : ""}`;
    dot.setAttribute("aria-label", dot.title);
    dot.addEventListener("click", () => toDefault([path]));
    r.querySelector(".row-label")?.prepend(dot);
  }
  return r;
}

/** Each page's open tab (pages with tabs: the widgets'). */
const tabs: Record<string, string> = {};
const TAB_ORDER = ["Content", "Look", "Motion"];
/** A page's id now (older names lead to where their settings went), if it exists. */
const pageId = (want: string | null): string | null => {
  const id = want ? (pageAliases[want] ?? want) : null;
  return id && pages.some((p) => p.id === id) ? id : null;
};
let current = pageId(new URLSearchParams(location.search).get("page")) ?? pages[0].id;
// Opened on a tab (?tab=Look).
{
  const want = new URLSearchParams(location.search).get("tab");
  if (want) tabs[current] = want;
}
/** The sidebar's search: while it holds text, the content lists the settings matching it. */
let query = "";
/** Only what's changed from the defaults (the sidebar's Changed button). */
let changedOnly = false;
// Opened on a page from elsewhere (the bar menu's Edit this bar...): go there.
listen<string>("settings-page", (e) => {
  const id = pageId(e.payload);
  if (!id) return;
  current = id;
  render();
  document.querySelector(".content")?.scrollTo(0, 0);
});


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
/** A short note at the bottom; after a change, with Undo (it stays a little longer). */
function toast(text: string, error = false, undoable = false): void {
  const t = document.getElementById("toast")!;
  t.replaceChildren(text);
  if (undoable && history.length) {
    const b = $("button", "toast-undo", "Undo");
    b.addEventListener("click", () => undoLast());
    t.append(b);
  }
  t.classList.toggle("error", error);
  t.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => t.classList.remove("show"), error ? 4000 : undoable ? 5000 : 1200);
}

// --- Undo -------------------------------------------------------------------------

/** What each change replaced (newest last): Undo (or Ctrl+Z) puts it back. */
const history: { path: Path; before: unknown }[][] = [];

/** Takes back the last change. */
async function undoLast(): Promise<void> {
  const last = history.pop();
  if (!last) return;
  const ops = last.map((c) => ({ path: c.path, value: c.before === undefined ? null : c.before }));
  try {
    await invoke("config_write_many", { ops });
    last.forEach((c) => setIn(cfg, c.path, c.before));
    toast("Undone");
  } catch (e) {
    toast(String(e), true);
  }
  render();
}

document.addEventListener("keydown", (e) => {
  if (e.ctrlKey && e.key.toLowerCase() === "z" && !document.activeElement?.matches("input[type=text], input:not([type]), input[type=search], textarea")) {
    e.preventDefault();
    undoLast();
  }
});

// --- Changed from the default --------------------------------------------------------

/** Where a setting is stored, when it's set to something other than its default. */
function changedPath(f: Field): Path | null {
  if (!("default" in f) || !("path" in f)) return null;
  const path = typeof f.path === "function" ? f.path(cfg) : f.path;
  if (!path) return null;
  const v = getIn(cfg, path);
  return v !== undefined && JSON.stringify(v) !== JSON.stringify(f.default) ? path : null;
}

/** Puts settings back to their defaults (they're taken out of the config). */
const toDefault = (paths: Path[]) => saveMany(paths.map((path) => ({ path, value: null })));

/** Where the current page's computed fields resolve (null = hidden): fields that
 *  appear only with another setting (e.g. corner roundness) depend on it. */
function shownFields(): string {
  const page = pages.find((p) => p.id === current);
  if (!page) return "";
  return JSON.stringify(page.sections.flatMap((s) => s.fields.map((f) => ("path" in f && typeof f.path === "function" ? f.path(cfg) : 0))));
}

async function save(path: Path, value: unknown): Promise<void> {
  try {
    const was = structuredClone(getIn(cfg, path));
    await invoke("config_write", { path, value: value === undefined ? null : value });
    const before = shownFields();
    setIn(cfg, path, value);
    history.push([{ path, before: was }]);
    toast("Saved", false, true);
    // A setting that shows or hides others: redraw (not on every save: a slider
    // being dragged would lose its grip). The setup's own fields: always.
    if (VIEW || shownFields() !== before) render();
  } catch (e) {
    toast(String(e), true);
    render(); // show the stored value again
  }
}

async function saveMany(ops: Op[]): Promise<void> {
  if (!ops.length) return;
  try {
    const was = ops.map((o) => ({ path: o.path, before: structuredClone(getIn(cfg, o.path)) }));
    await invoke("config_write_many", { ops });
    ops.forEach((o) => setIn(cfg, o.path, o.value ?? undefined));
    // In reverse, so putting them back undoes later edits first.
    history.push(was.reverse());
    toast("Saved", false, true);
  } catch (e) {
    toast(String(e), true);
  }
  render(); // structured editors redraw from the new config
}

// --- Controls --------------------------------------------------------------------

const fieldRow = (f: { label: string; help?: string; short?: string }, control: HTMLElement) => {
  if (f.short === undefined) return row(f.label, f.help, control);
  // Home's cards: a short line (or none), the whole help in the tooltip.
  const r = row(f.label, f.short || undefined, control);
  if (f.help) r.title = f.help;
  return r;
};

function control(f: Field): HTMLElement | null {
  if (f.kind === "custom") {
    // "name:arg": one editor for several settings (e.g. osdPlace:volume).
    const [name, ...rest] = f.editor.split(":");
    const arg = rest.length ? rest.join(":") : undefined;
    const editor = editors[name];
    if (!editor) return null;
    return editor({
      arg,
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
        } else if (f.run === "setup") {
          await invoke("open_setup_window");
        } else if (f.run === "notes") {
          await invoke("open_notes_window", {});
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
      const values = f.values;
      const sw = values ? toggleSwitch(value === values[1], (on) => save(path, values[on ? 1 : 0])) : toggleSwitch(Boolean(value), (on) => save(path, on));
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
      // A widget's look: a tile each, with Kasumi's own picture of it (public/looks,
      // drawn by tools/look_tiles.py; a look without one shows its name only).
      if (path[0] === "widgets" && path[2] === "style") {
        const tiles = $("div", "look-tiles");
        tiles.setAttribute("role", "radiogroup");
        tiles.setAttribute("aria-label", f.label);
        for (const o of f.options) {
          const [name, rest] = o.label.split(/ \((.*)\)$/);
          const on = String(value) === o.value;
          const img = document.createElement("img");
          img.src = `looks/${path[1]}-${o.value || "classic"}.png`;
          img.alt = "";
          img.addEventListener("error", () => img.remove());
          const tile = $("button", `look-tile${on ? " on" : ""}`, $("span", "look-pic", img), $("b", "", name), ...(rest ? [$("small", "", rest)] : []));
          tile.setAttribute("role", "radio");
          tile.setAttribute("aria-checked", String(on));
          tile.addEventListener("click", () => {
            if (!on) save(path, o.value).then(() => render());
          });
          tiles.append(tile);
        }
        const r = row(f.label, f.help, $("span"));
        r.classList.add("look-row");
        r.append(tiles);
        return r;
      }
      // A few short choices: buttons side by side, their longer words in a tooltip.
      const short = f.options.map((o) => o.label.split(/ \((.*)\)$/));
      if (f.options.length <= 4 && short.reduce((n, [name]) => n + name.length, 0) <= 40) {
        const opts = f.options.map((o, i): [string, string, string?] => [o.value, short[i][0], short[i][1]]);
        return fieldRow(f, segmented(opts, String(value), f.label, (v) => save(path, v)));
      }
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
  // A place with several pages: its name over the page's.
  const area = areaOf(page.id);
  const content = $("main", "content");
  if (area.pages.length > 1) content.append($("div", "page-crumb", area.label));
  content.append($("h1", "page-title", area.pages.length > 1 ? page.label : area.label));
  // Tabs: the sections above them (no tab) always show, then the open tab's.
  const names = TAB_ORDER.filter((t) => page.sections.some((x) => x.tab === t && x.fields.some((f) => control(f))));
  const tab = names.includes(tabs[page.id]) ? tabs[page.id] : names[0];
  let tabBar: HTMLElement | null = null;
  if (names.length > 1) {
    tabBar = $("div", "tabs");
    tabBar.setAttribute("role", "tablist");
    for (const t of names) {
      const b = $("button", `tab${t === tab ? " active" : ""}`, t);
      b.setAttribute("role", "tab");
      b.setAttribute("aria-selected", String(t === tab));
      b.tabIndex = t === tab ? 0 : -1;
      b.addEventListener("click", () => {
        tabs[page.id] = t;
        render();
      });
      // Arrows go to the next tab (and keep the focus on the tabs).
      b.addEventListener("keydown", (e) => {
        const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
        if (!step) return;
        e.preventDefault();
        tabs[page.id] = names[(names.indexOf(t) + step + names.length) % names.length];
        render();
        (document.querySelector('.tabs [aria-selected="true"]') as HTMLElement | null)?.focus();
      });
      tabBar.append(b);
    }
  }
  let group: HTMLElement | null = null;
  let last: HTMLElement | null = null;
  for (let i = 0; i < page.sections.length; i++) {
    const s = page.sections[i];
    if (s.tab && s.tab !== tab) continue;
    if (s.tab && tabBar) {
      content.append(tabBar);
      tabBar = null;
    }
    // A pointer to where these settings live.
    if (s.link) {
      const go = $("button", "button", `${s.link[0]} ›`);
      go.addEventListener("click", () => goto(s.link![1], ""));
      content.append($("section", "card", $("h2", "card-title", s.title), ...(s.help ? [$("p", "card-help", s.help)] : []), go));
      group = null;
      continue;
    }
    // Widgets page: some sections belong to the Global view, some to a monitor's.
    if (s.scope === "global" && scope.monitor) continue;
    if (s.scope === "monitor" && !scope.monitor) continue;
    // One bar's settings: a card for each bar, the run of them together.
    if (s.perBar) {
      const run: Section[] = [];
      for (; i < page.sections.length && page.sections[i].perBar; i++) {
        const x = page.sections[i];
        if (!(x.scope === "global" && scope.monitor) && !(x.scope === "monitor" && !scope.monitor)) run.push(x);
      }
      i--;
      const keep = scope.surface;
      for (const id of surfaceIds(cfg)) {
        scope.surface = id;
        const card = barCard(id, run);
        if (card) content.append(card);
      }
      scope.surface = keep;
      group = null;
      continue;
    }
    // Expert settings fold under More (open once opened, until closed again).
    const shown = s.fields.filter((f) => !("more" in f && f.more));
    const expert = s.fields.filter((f) => "more" in f && f.more);
    let rows = shown.map(fieldControl).filter((r): r is HTMLElement => r !== null);
    const extra = expert.map(fieldControl).filter((r): r is HTMLElement => r !== null);
    if (extra.length) {
      const key = `${page.id}:${s.title}`;
      const isOpen = moreOpen.has(key);
      const more = $("button", `more-toggle${isOpen ? " open" : ""}`, glyph(0xe76c), isOpen ? "Less" : `More (${extra.length})`);
      more.setAttribute("aria-expanded", String(isOpen));
      more.addEventListener("click", () => {
        if (moreOpen.has(key)) moreOpen.delete(key);
        else moreOpen.add(key);
        saveMore();
        render();
      });
      rows = [...rows, more, ...(isOpen ? extra : [])];
    }
    if (!rows.length) {
      // None of its settings applies: say why, quietly.
      const why = s.note?.(cfg);
      if (why) content.append($("p", "quiet-note", glyph(0xe946), $("span", "", why)));
      continue;
    }
    if (s.collapsible && s.attach && last) {
      last.append(collapsible(s.title, $("div", "sub-body", ...rows), `${page.id}:${s.title}`, s.icon));
      continue;
    }
    if (s.collapsible) {
      // Consecutive collapsible sections share one card, one dropdown each.
      if (!group) {
        group = $("section", "card", $("h2", "card-title", s.group ?? "Widget settings"));
        content.append(group);
      }
      group.append(collapsible(s.title, $("div", "sub-body", ...rows), `${page.id}:${s.title}`, s.icon));
      continue;
    }
    group = null;
    // Cards side by side, the title over them.
    if (s.tiles) {
      if (s.title) content.append($("h2", "tiles-title", s.title));
      content.append($("div", "tiles", ...rows));
      continue;
    }
    // Untitled: just its content (a widget page's header, the widget cards).
    if (!s.title) {
      content.append(...rows);
      continue;
    }
    const title = $("h2", "card-title", s.title);
    const badge = s.badge?.(cfg);
    if (badge) title.append($("span", "card-badge", badge));
    if (s.legend) title.append($("span", "card-legend", ...s.legend.map(([code, text]) => $("span", "", glyph(code), text))));
    // Settings in it changed: all of them back to their defaults at once.
    const changed = s.fields.map(changedPath).filter((x): x is Path => x !== null);
    if (changed.length > 1) {
      const back = $("button", "card-reset", "Back to default");
      back.title = `Puts back the default of the ${changed.length} settings changed here`;
      back.addEventListener("click", () => toDefault(changed));
      title.append(back);
    }
    const card = $("section", "card", title);
    if (s.help) card.append($("p", "card-help", s.help));
    card.append(...rows);
    content.append(card);
    last = card;
  }
  return content;
}

/** One bar's card of `run` (per-bar sections, drawn with scope.surface on it): its
 *  name as the title; a bar that's off and has nothing to set here: none. */
function barCard(id: string, run: Section[]): HTMLElement | null {
  const name = surfaceName(id, cfg.surfaces?.[id]);
  const rows = run.flatMap((s) => s.fields.map(fieldControl).filter((r): r is HTMLElement => r !== null));
  const title = $("h2", "card-title", run.length === 1 && run[0].title ? `${name} · ${run[0].title}` : name);
  if (!rows.length) {
    if (!shownSurfaces(cfg).includes(id)) return null;
    const why = run.map((s) => s.note?.(cfg)).find((x) => x);
    return why ? $("p", "quiet-note", glyph(0xe946), $("span", "", `${name}: ${why}`)) : null;
  }
  const changed = run.flatMap((s) => s.fields.map(changedPath)).filter((x): x is Path => x !== null);
  if (changed.length > 1) {
    const back = $("button", "card-reset", "Back to default");
    back.title = `Puts back the default of the ${changed.length} settings changed here`;
    back.addEventListener("click", () => toDefault(changed));
    title.append(back);
  }
  const help = run.length === 1 ? run[0].help : undefined;
  return $("section", "card", title, ...(help ? [$("p", "card-help", help)] : []), ...rows);
}

/** The sidebar's search: every setting whose page, section, name or help has all the
 *  typed words, with its control (changeable here), under its page and section (a click
 *  there goes to it). */
function renderResults(q: string, onlyChanged = false): HTMLElement {
  const content = $("main", "content", $("h1", "page-title", onlyChanged ? "Changed from the defaults" : "Search"));
  const words = q.toLowerCase().split(/\s+/).filter(Boolean);
  const has = (...texts: (string | undefined)[]) => {
    const t = texts.filter(Boolean).join(" ").toLowerCase();
    return words.every((w) => t.includes(w));
  };
  let found = 0;
  for (const p of pages) {
    for (const s of p.sections) {
      // As on its page: the Widgets page's Global or monitor view.
      if (s.scope === "global" && scope.monitor) continue;
      if (s.scope === "monitor" && !scope.monitor) continue;
      const whole = has(p.label, s.title, s.help);
      const fields = onlyChanged ? s.fields.filter((f) => changedPath(f)) : s.fields.filter((f) => whole || ("label" in f && has(p.label, s.title, f.label, f.help)));
      const rows = fields.map(fieldControl).filter((r): r is HTMLElement => r !== null);
      if (!rows.length) continue;
      found++;
      const area = areaOf(p.id);
      const where = area.pages.length > 1 ? `${area.label} › ${p.label}` : area.label;
      const title = $("button", "card-title result-title", `${where} › ${s.title}`);
      title.addEventListener("click", () => {
        query = "";
        goto(p.id, s.collapsible ? s.title : "");
      });
      content.append($("section", "card", title, ...rows));
    }
  }
  if (!found) content.append($("p", "card-help", onlyChanged ? "Everything is as it came: nothing's changed from the defaults." : "No settings match."));
  return content;
}

/** Shows `page` with its `section` dropdown open and scrolled into view. */
function goto(page: string, section: string): void {
  // (The setup's widget chips: their settings are in the settings window.)
  if (VIEW) return;
  page = pageId(page) ?? page;
  current = page;
  if (section) openSection(`${page}:${section}`);
  render();
  const app = document.getElementById("app")!;
  const target = section && app.querySelector<HTMLElement>(`[data-key="${CSS.escape(`${page}:${section}`)}"]`);
  if (target) target.scrollIntoView({ block: "start" });
  else app.querySelector(".content")!.scrollTop = 0;
}

const host: Host = { cfg: () => cfg, control, saveMany, reload: load, render: () => render(), toast };

function render(): void {
  const app = document.getElementById("app")!;
  if (VIEW === "setup" || VIEW === "notes") {
    const keep = app.querySelector(".setup-page")?.scrollTop ?? 0;
    app.replaceChildren(VIEW === "setup" ? renderSetup(host) : renderNotes(host));
    app.querySelector(".setup-page")!.scrollTop = keep;
    return;
  }
  const scroll = app.querySelector(".content")?.scrollTop ?? 0;
  const nav = $("nav", "nav");
  // Search: typing redraws only the content, so the field keeps the focus.
  const search = $("input", "input nav-search") as HTMLInputElement;
  search.type = "search";
  search.placeholder = "Search settings";
  search.value = query;
  search.addEventListener("input", () => {
    query = search.value.trim();
    const next = query ? renderResults(query) : renderPage(pages.find((p) => p.id === current)!);
    app.querySelector(".content")!.replaceWith(next);
    nav.querySelectorAll("[data-page]").forEach((n) => n.classList.toggle("active", !query && n.getAttribute("data-page") === current));
    nav.querySelectorAll("[data-area]").forEach((n) => n.classList.toggle("active", !query && n.getAttribute("data-area") === areaOf(current).id));
  });
  search.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && search.value) {
      e.preventDefault();
      search.value = "";
      search.dispatchEvent(new Event("input"));
    }
  });
  nav.append(search);
  const open = (id: string) => {
    current = id;
    query = "";
    changedOnly = false;
    render();
    app.querySelector(".content")!.scrollTop = 0;
  };
  // The places; the open one shows its pages under it.
  const here = areaOf(current);
  for (const a of areas) {
    const icon = a.icon ? glyph(a.icon) : $("span", "icon nav-logo", kasumiLogo());
    const item = $("button", `nav-item${a === here && !query ? " active" : ""}`, icon, a.label);
    if (a === here && !query) item.setAttribute("aria-current", "true");
    item.setAttribute("data-area", a.id);
    item.addEventListener("click", () => open(a.pages[0]));
    nav.append(item);
    if (a !== here || a.pages.length < 2) continue;
    for (const id of a.pages) {
      const p = pages.find((x) => x.id === id);
      if (!p) continue;
      // Widgets: the ones on a bar (and the open one).
      if (id.startsWith("w-") && id !== current && !isPlaced(cfg, id.slice(2))) continue;
      const sub = $("button", `nav-sub${id === current && !query ? " active" : ""}`, p.label);
      if (id === current && !query) sub.setAttribute("aria-current", "page");
      sub.setAttribute("data-page", id);
      sub.addEventListener("click", () => open(id));
      nav.append(sub);
    }
  }
  // Only what's changed from the defaults.
  const onlyChanged = $("button", `nav-changed${changedOnly ? " active" : ""}`, glyph(0xe7c1), "Changed from the defaults");
  onlyChanged.setAttribute("aria-pressed", String(changedOnly));
  onlyChanged.addEventListener("click", () => {
    changedOnly = !changedOnly;
    query = "";
    render();
  });
  nav.insertBefore(onlyChanged, nav.children[1] ?? null);
  const page = changedOnly ? renderResults("", true) : query ? renderResults(query) : renderPage(pages.find((p) => p.id === current)!);
  app.replaceChildren(nav, page);
  page.scrollTop = scroll;
  // The sidebar's foot: the mist shape (Desktop asset.svg), the version in its lower-left gap.
  const foot = $("div", "nav-foot");
  foot.innerHTML =
    '<svg viewBox="0 0 188.17 40.54" aria-hidden="true"><path d="M179.95,24.1h-76.73c-2.12,0-3.83-1.72-3.83-3.83s1.72-3.83,3.83-3.83h23.32c4.54,0,8.22-3.68,8.22-8.22s-3.68-8.22-8.22-8.22H8.22C3.68,0,0,3.68,0,8.22s3.68,8.22,8.22,8.22h67.68c2.12,0,3.83,1.72,3.83,3.83s-1.72,3.83-3.83,3.83h-14.27c-4.54,0-8.22,3.68-8.22,8.22h0c0,4.54,3.68,8.22,8.22,8.22h118.33c4.54,0,8.22-3.68,8.22-8.22h0c0-4.54-3.68-8.22-8.22-8.22Z"/></svg>';
  nav.append(foot);
  invoke<string>("app_version").then((v) => foot.append($("div", "version", `v${v}`)));
}

async function load(): Promise<void> {
  try {
    cfg = await invoke<Cfg>("config_read");
    // Older configs move to the current format once: [bar] lists + [[notches]]
    // -> [layout], then per-widget notch tables -> [notch] side settings.
    let migrated = false;
    // Only writes that change something: a step can restate what's already there,
    // and every reload (each time this window gets the focus) would "migrate" again.
    const at = (o: any, path: (string | number)[]) => path.reduce((v, k) => (v == null ? undefined : v[k]), o);
    const same = (a: unknown, b: unknown) => JSON.stringify(a ?? null) === JSON.stringify(b ?? null);
    for (const step of [migrationOps, migrationOpsV3, migrationOpsV4, migrationOpsV5, migrationOpsDisabledSpots, migrationOpsRemovedWidgets]) {
      const ops = step(cfg).filter((op) => !same(at(cfg, op.path), op.value));
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

// Custom title bar: Windows controls in the window's own style, the logo by the name.
document.querySelector(".titlebar-title")!.prepend(kasumiLogo());
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
