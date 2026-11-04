// The context menu editor ([context_menu] layout): the menu drawn as it shows for one
// kind of click (file, folder, drive, several items, empty space, desktop), and beside
// it the entry clicked: its name, icon, whether and where it shows, and the rest of its
// options. Drag an entry by its handle to reorder it, or onto a folder to put it
// inside; folders and Windows' submenus (Send to, Open with...) open in place. Plus
// your own folders, commands and separators.
//
// The layout is one list for every kind of click (each entry can say where it shows).
// Windows' entries Kasumi hasn't placed come where the layout says "rest"; editing
// one of those places them all, in the order they show.
import { invoke } from "@tauri-apps/api/core";
import { $, glyph, iconButton, input, row as field, select, toggleSwitch } from "./dom";
import { ICONS, type EditorCtx } from "./editors";

interface MenuNode {
  match?: string[];
  kasumi?: string;
  folder?: string;
  items?: MenuNode[];
  name?: string;
  icon?: string;
  cmd?: string;
  args?: string;
  hide?: boolean;
  separator?: boolean;
  rest?: boolean;
  custom?: boolean;
  shift?: boolean;
  where?: string;
}

interface Seen {
  text: string;
  verb: string;
  sub?: boolean;
  glyph?: string;
  bitmap?: { w: number; h: number; rgba: string };
}

interface Info {
  seen: Record<string, Seen[]>;
  default: MenuNode[];
}

const KINDS: [string, string][] = [
  ["file", "File"],
  ["folder", "Folder"],
  ["drive", "Drive"],
  ["multiple", "Several items"],
  ["background", "Folder background"],
  ["desktop", "Desktop"],
];
/** The tabs: each kind of Explorer click, and Kasumi's bar menu and launcher menu (each
 *  its own layout). */
const TABS: [string, string][] = [...KINDS, ["bar", "Kasumi bar"], ["launcher", "Launcher apps"]];
/** Kasumi's own menus: a layout of their own ([bar_menu] / [launcher_menu] layout). */
const OWN: Record<string, string> = { bar: "bar_menu", launcher: "launcher_menu" };
const SELECTION = ["file", "folder", "drive", "multiple"];
const SPACE = ["background", "desktop"];

/** Kasumi's own entries: their names, icons, and the clicks they show for. */
const KASUMI: Record<string, { label: string; glyph: string; kinds?: string[] }> = {
  "view.view": { label: "View", glyph: "E8A9", kinds: SPACE },
  "view.sort": { label: "Sort by", glyph: "E8CB", kinds: SPACE },
  "view.refresh": { label: "Refresh", glyph: "E72C", kinds: SPACE },
  "terminal.cmd": { label: "Command Prompt", glyph: "E756" },
  "terminal.powershell": { label: "PowerShell", glyph: "E756" },
  "terminal.wt": { label: "Windows Terminal", glyph: "E756" },
  "file_manage.copy_path": { label: "Copy path", glyph: "E8C8", kinds: SELECTION },
  "file_manage.select": { label: "Select", glyph: "E8B3" },
  "file_manage.show_hide": { label: "Show / Hide", glyph: "E7B3" },
  "file_manage.attributes": { label: "Attributes", glyph: "E946", kinds: ["file", "folder"] },
  "file_manage.new_folder": { label: "New folder (date and time)", glyph: "E8F4", kinds: SPACE },
  "file_manage.new_file": { label: "New file", glyph: "E8A5", kinds: SPACE },
  "file_manage.folder_options": { label: "Folder options", glyph: "E713", kinds: ["file", "folder", "drive", "multiple", "background"] },
  "go_to.folders": { label: "Folders", glyph: "E8B7" },
  "go_to.control_panel": { label: "Control Panel", glyph: "E713" },
  "go_to.all_control_panel": { label: "All Control Panel items", glyph: "E713" },
  "go_to.run": { label: "Run", glyph: "E768" },
  "go_to.settings": { label: "Settings", glyph: "E713" },
};
const GROUPS: Record<string, string[]> = {};
for (const id of Object.keys(KASUMI)) (GROUPS[id.split(".")[0]] ??= []).push(id);

/** `text:Pin to*` / `verb:copyaspath` against a Windows entry (as layout.rs does). */
function matches(m: string, e: Seen): boolean {
  if (m.startsWith("in:")) return false;
  const i = m.indexOf(":");
  const [kind, pat] = i < 0 ? ["text", m] : [m.slice(0, i), m.slice(i + 1)];
  const hay = (kind === "verb" ? e.verb : e.text).toLowerCase();
  if (!hay) return false;
  const p = pat.toLowerCase();
  return p.endsWith("*") ? hay.startsWith(p.slice(0, -1)) : hay === p;
}

const keyOf = (e: Seen) => (e.verb ? `verb:${e.verb}` : `text:${e.text}`);
/** An entry lifted out of its submenu: `in:<submenu>|<entry>`. */
const liftedOf = (n: MenuNode): [string, string] | null => {
  const m = n.match?.[0];
  if (!m?.startsWith("in:")) return null;
  const i = m.indexOf("|");
  return i < 0 ? null : [m.slice(3, i), m.slice(i + 1)];
};
const applies = (n: MenuNode, kind: string) => !n.where || n.where.split("|").some((w) => w.trim() === kind);
const hexOf = (ch: string) => ch.codePointAt(0)!.toString(16).toUpperCase();

/** A node as saved: no empty or default fields (a folder keeps its list). */
function clean(n: MenuNode): MenuNode {
  const out: any = {};
  for (const [k, v] of Object.entries(n)) {
    if (v === "" || v === false || v == null || (Array.isArray(v) && v.length === 0)) continue;
    out[k] = k === "items" ? (v as MenuNode[]).map(clean) : v;
  }
  if (n.folder) out.items = (n.items ?? []).map(clean);
  return out;
}

/** Kasumi's groups ("file_manage") as their entries, so each can be edited. */
function expandGroups(list: MenuNode[]) {
  for (let i = list.length - 1; i >= 0; i--) {
    const n = list[i];
    if (n.kasumi && GROUPS[n.kasumi]) list.splice(i, 1, ...GROUPS[n.kasumi].map((k) => ({ kasumi: k })));
    else if (n.folder) expandGroups((n.items ??= []));
  }
}

/** The editor's place across redraws: the kind of click shown, folders opened. */
const state: { info?: Info; kind: string; open: Set<string>; sel: string | null } = { kind: "file", open: new Set(), sel: null };

type Owner = () => [MenuNode, MenuNode[]] | null;

interface Row {
  node: MenuNode;
  /** The list it's in (for one of the rest's entries: the rest's). */
  list: MenuNode[];
  /** A Windows entry still in "rest" (not placed by the layout yet). */
  entry?: Seen;
  /** The Windows entries it stands for, here. */
  hits?: Seen[];
  depth: number;
  /** What its matches are against: this click's entries, or a submenu's. */
  seen: Seen[];
  /** In a Windows submenu: that submenu's own node (placed first if needed), its key. */
  owner?: Owner;
  ownerKey?: string;
  /** Has entries under it (a folder, or a Windows submenu Kasumi has seen). */
  parent?: string;
  /** Hidden here (for this kind of click, or everywhere). */
  off?: boolean;
}

export const contextMenuEditor = (ctx: EditorCtx): HTMLElement => {
  const box = $("div", "cm-editor", $("p", "card-help", "Loading…"));
  let layout: MenuNode[] = [];
  /** Explorer's menu's layout, and the bar menu's ([bar_menu] layout): `layout` is the
   *  one on the tab shown. */
  let mainLayout: MenuNode[] = [];
  let barLayout: MenuNode[] = [{ rest: true }];
  let launcherLayout: MenuNode[] = [{ rest: true }];
  const pick = () => (layout = state.kind === "bar" ? barLayout : state.kind === "launcher" ? launcherLayout : mainLayout);
  let seenAll: Record<string, Seen[]> = {};
  let drag: Row | null = null;

  const save = () => {
    if (OWN[state.kind]) return ctx.save([OWN[state.kind], "layout"], layout.map(clean)).catch((e) => ctx.toast(String(e), true));
    const hasCustom = ((ctx.cfg as any).context_menu?.custom ?? []).length > 0;
    const value = mainLayout.map(clean);
    const done = hasCustom
      ? ctx.saveMany([
          { path: ["context_menu", "layout"], value },
          { path: ["context_menu", "custom"], value: null },
        ])
      : ctx.save(["context_menu", "layout"], value);
    return done.catch((e) => ctx.toast(String(e), true));
  };

  /** The Windows entries a list names (placed or hidden), folders' included. */
  const claimedIn = (list: MenuNode[], seen: Seen[]) => {
    const used = new Set<number>();
    const walk = (l: MenuNode[]) => {
      for (const n of l) {
        n.match?.forEach((m) => seen.forEach((e, i) => matches(m, e) && used.add(i)));
        if (n.folder && n.items) walk(n.items);
      }
    };
    walk(list);
    return used;
  };

  const findList = (root: MenuNode[], node: MenuNode): MenuNode[] | null => {
    if (root.includes(node)) return root;
    for (const n of root) {
      const inner = n.folder && n.items ? findList(n.items, node) : null;
      if (inner) return inner;
    }
    return null;
  };

  const findKey = (root: MenuNode[], key: string): [MenuNode, MenuNode[]] | null => {
    for (const n of root) {
      if (n.match?.length === 1 && n.match[0] === key) return [n, root];
      const inner = n.folder && n.items ? findKey(n.items, key) : null;
      if (inner) return inner;
    }
    return null;
  };

  /** Places the rest's entries (in `root`) as their own, in the order they show. */
  const materialize = (root: MenuNode[], seen: Seen[]) => {
    const rest = (l: MenuNode[]): [MenuNode[], number] | null => {
      for (let i = 0; i < l.length; i++) {
        if (l[i].rest) return [l, i];
        const inner = l[i].folder && l[i].items ? rest(l[i].items!) : null;
        if (inner) return inner;
      }
      return null;
    };
    const at = rest(root);
    if (!at) return;
    const used = claimedIn(root, seen);
    at[0].splice(at[1], 0, ...seen.filter((_, j) => !used.has(j)).map((e) => ({ match: [keyOf(e)] })));
  };

  /** A row's node and its list, placing what it needs first (the rest's entries; a
   *  Windows submenu's own node). */
  const resolve = (row: Row): [MenuNode, MenuNode[]] | null => {
    let root = layout;
    if (row.owner) {
      const o = row.owner();
      if (!o) return null;
      const items = (o[0].items ??= []);
      if (!items.some((n) => n.rest)) items.push({ rest: true });
      root = items;
    }
    if (row.entry) {
      materialize(root, row.seen);
      return findKey(root, keyOf(row.entry));
    }
    const list = findList(root, row.node);
    return list ? [row.node, list] : null;
  };

  const contains = (outer: MenuNode, inner: MenuNode): boolean => outer === inner || (outer.items ?? []).some((c) => contains(c, inner));

  // --- The rows ---------------------------------------------------------------------

  const rows = (list: MenuNode[], seen: Seen[], used: Set<number>, depth: number, out: Row[], owner?: Owner, ownerKey?: string) => {
    const kind = state.kind;
    for (const n of list) {
      const off = !!n.hide || (!owner && !applies(n, kind));
      if (n.rest) {
        seen.forEach((e, i) => {
          if (used.has(i)) return;
          const row: Row = { node: n, list, entry: e, depth, seen, owner, ownerKey };
          out.push(row);
          if (!owner) subRows(e, row, depth + 1, out);
        });
      } else if (n.folder) {
        const row: Row = { node: n, list, depth, seen, owner, ownerKey, parent: `folder:${n.folder}`, off };
        out.push(row);
        if (state.open.has(row.parent!)) rows(n.items ?? [], seen, used, depth + 1, out, owner, ownerKey);
      } else if (n.kasumi) {
        const k = KASUMI[n.kasumi];
        if (!k?.kinds || k.kinds.includes(kind)) out.push({ node: n, list, depth, seen, owner, ownerKey, off });
      } else if (liftedOf(n)) {
        const [parent, child] = liftedOf(n)!;
        const hits = (seenAll[`sub:${parent}`] ?? []).filter((e) => keyOf(e) === child);
        if (hits.length) out.push({ node: n, list, hits, depth, seen, owner, ownerKey, off });
      } else if (n.match?.length) {
        const hits = seen.filter((e) => n.match!.some((m) => matches(m, e)));
        if (!hits.length) continue;
        const row: Row = { node: n, list, hits, depth, seen, owner, ownerKey, off };
        out.push(row);
        if (!owner) subRows(hits[0], row, depth + 1, out, n);
      } else {
        out.push({ node: n, list, depth, seen, owner, ownerKey, off });
      }
    }
  };

  /** A Windows submenu's entries (as Kasumi has seen them), under its row. */
  const subRows = (e: Seen, parentRow: Row, depth: number, out: Row[], node?: MenuNode) => {
    const subSeen = seenAll[`sub:${keyOf(e)}`];
    if (!subSeen?.length) return;
    parentRow.parent = `sub:${keyOf(e)}`;
    if (!state.open.has(parentRow.parent)) return;
    const items = node?.items ?? [];
    const list = items.some((r) => r.rest) ? items : [...items, { rest: true }];
    // Ones lifted out of it (into the menu or a folder) aren't in it.
    const used = claimedIn(list, subSeen);
    const lifted = new Set<string>();
    const walk = (l: MenuNode[]) =>
      l.forEach((n) => {
        const f = liftedOf(n);
        if (f && f[0] === keyOf(e)) lifted.add(f[1]);
        if (n.items) walk(n.items);
      });
    walk(layout);
    subSeen.forEach((se, i) => lifted.has(keyOf(se)) && used.add(i));
    rows(list, subSeen, used, depth, out, () => resolve(parentRow), keyOf(e));
  };

  // --- Editing ----------------------------------------------------------------------

  const commit = () => {
    render();
    save();
  };

  const edit = (row: Row, f: (node: MenuNode, list: MenuNode[]) => void) => () => {
    const r = resolve(row);
    if (!r) return;
    f(r[0], r[1]);
    commit();
  };

  type Zone = "before" | "after" | "into";
  const zoneOf = (e: DragEvent, el: HTMLElement, row: Row): Zone => {
    const r = el.getBoundingClientRect();
    const y = (e.clientY - r.top) / r.height;
    if (row.node.folder && !row.entry && y > 0.25 && y < 0.75) return "into";
    return y < 0.5 ? "before" : "after";
  };
  const canDrop = (src: Row | null, dst: Row, zone: Zone) => {
    if (!src || src === dst) return false;
    // Out of a submenu (into the menu or a folder); back into its own one; nothing
    // else goes into a submenu.
    if (src.ownerKey !== dst.ownerKey) {
      const out = !!src.ownerKey && !dst.ownerKey;
      const back = !src.ownerKey && !!dst.ownerKey && liftedOf(src.node)?.[0] === dst.ownerKey;
      if (!out && !back) return false;
    }
    if (zone === "into" && (!dst.node.folder || dst.entry)) return false;
    if (!src.entry && !dst.entry && contains(src.node, dst.node)) return false;
    return true;
  };
  const drop = (src: Row, dst: Row, zone: Zone) => {
    // Back into its submenu: it's one of its entries again.
    const lifted = liftedOf(src.node);
    if (!src.ownerKey && lifted && dst.ownerKey === lifted[0]) {
      const s = resolve(src);
      if (s) s[1].splice(s[1].indexOf(s[0]), 1);
      commit();
      return;
    }
    // Out of its submenu: taken out by name (`in:`), keeping its name and icon here.
    if (src.ownerKey && !dst.ownerKey) {
      const child = src.entry ? keyOf(src.entry) : src.node.match?.[0];
      if (!child) return;
      const node: MenuNode = { match: [`in:${src.ownerKey}|${child}`] };
      if (!src.entry) {
        const s = resolve(src);
        if (s) {
          if (s[0].name) node.name = s[0].name;
          if (s[0].icon) node.icon = s[0].icon;
          s[1].splice(s[1].indexOf(s[0]), 1);
        }
      }
      const d = resolve(dst);
      if (!d) return;
      const [dn, dl] = d;
      if (zone === "into") {
        (dn.items ??= []).push(node);
        if (dn.folder) state.open.add(`folder:${dn.folder}`);
      } else {
        dl.splice(dl.indexOf(dn) + (zone === "after" ? 1 : 0), 0, node);
      }
      commit();
      return;
    }
    const s = resolve(src);
    const d = resolve(dst);
    if (!s || !d) return;
    const [sn, sl] = s;
    const [dn, dl] = d;
    if (sn === dn || contains(sn, dn)) return;
    sl.splice(sl.indexOf(sn), 1);
    if (zone === "into") {
      (dn.items ??= []).push(sn);
      if (dn.folder) state.open.add(`folder:${dn.folder}`);
    } else {
      dl.splice(dl.indexOf(dn) + (zone === "after" ? 1 : 0), 0, sn);
    }
    commit();
  };

  /** The icon button: the entry's icon now (or empty), a grid to choose another, its
   *  own (Default) or none. */
  /** An entry's icon as the menu shows it (its own, Kasumi's, Windows', or none). */
  const iconFace = (row: Row): HTMLElement | null => {
    const n = row.node;
    const own = row.entry ?? row.hits?.[0];
    const set = row.entry ? undefined : n.icon;
    if (set === "0") return null;
    if (set) return $("span", "icon", String.fromCodePoint(parseInt(set, 16)));
    if (n.kasumi && KASUMI[n.kasumi]) return $("span", "icon", String.fromCodePoint(parseInt(KASUMI[n.kasumi].glyph, 16)));
    if (own?.bitmap) {
      const c = document.createElement("canvas");
      c.width = own.bitmap.w;
      c.height = own.bitmap.h;
      const px = Uint8ClampedArray.from(atob(own.bitmap.rgba), (ch) => ch.charCodeAt(0));
      c.getContext("2d")!.putImageData(new ImageData(px, own.bitmap.w, own.bitmap.h), 0, 0);
      c.className = "cm-bitmap";
      return c;
    }
    if (own?.glyph) return $("span", "icon", String.fromCodePoint(parseInt(own.glyph, 16)));
    return null;
  };

  const iconControl = (row: Row): [HTMLElement, HTMLElement] => {
    const btn = $("button", "icon-picker cm-icon") as HTMLButtonElement;
    btn.title = "Its icon";
    const face = iconFace(row);
    if (face) btn.append(face);
    const grid = $("div", "icon-grid");
    grid.hidden = true;
    const pick = (value: string | null) => () => {
      grid.hidden = true;
      edit(row, (node) => {
        if (value === null) delete node.icon;
        else node.icon = value;
      })();
    };
    const word = (text: string, value: string | null) => {
      const b = $("button", "icon-cell cm-word", text);
      b.addEventListener("click", pick(value));
      return b;
    };
    grid.append(word("Default", null), word("None", "0"));
    for (const [code, name] of ICONS) {
      const cell = $("button", "icon-cell", glyph(code));
      cell.title = name;
      cell.addEventListener("click", pick(hexOf(String.fromCharCode(code))));
      grid.append(cell);
    }
    btn.addEventListener("click", () => {
      const open = grid.hidden;
      document.querySelectorAll<HTMLElement>(".icon-grid").forEach((g) => (g.hidden = true));
      grid.hidden = !open;
    });
    return [btn, grid];
  };

  /** A row's key, to find it again after a redraw (the one selected). */
  const rowKey = (row: Row): string => {
    const n = row.node;
    if (row.entry) return `e:${row.ownerKey ?? ""}:${keyOf(row.entry)}`;
    if (n.folder) return `f:${n.folder}`;
    if (n.kasumi) return `k:${n.kasumi}`;
    if (n.cmd) return `c:${n.name ?? ""}|${n.cmd}`;
    if (n.match?.length) return `m:${row.ownerKey ?? ""}:${n.match.join(",")}`;
    if (n.separator) return `s:${row.depth}:${row.list.indexOf(n)}`;
    return "";
  };

  /** What an entry says in the menu. */
  const labelOf = (row: Row): string => {
    const n = row.node;
    if (row.entry) return n.name || row.entry.text;
    if (n.folder) return n.folder;
    if (n.kasumi) return n.name || (KASUMI[n.kasumi]?.label ?? n.kasumi);
    if (n.cmd) return n.name || n.cmd;
    return n.name || (row.hits ?? []).map((e) => e.text).join(", ");
  };

  /** One of the menu's entries: drag it by its handle, click it to change it. */
  const rowEl = (row: Row): HTMLElement => {
    const n = row.node;
    const isSep = !!n.separator && !row.entry;
    const key = rowKey(row);
    const el = $("div", `cm-row${row.off ? " hidden-row" : ""}${n.folder ? " folder-row" : ""}${key && key === state.sel ? " selected" : ""}`);
    el.style.paddingLeft = `${6 + row.depth * 18}px`;
    el.tabIndex = 0;
    el.setAttribute("role", "option");
    el.setAttribute("aria-selected", String(key === state.sel));

    const handle = $("span", "cm-handle", glyph(0xe76f));
    handle.title = "Drag to move (onto a folder to put it inside)";
    handle.draggable = true;
    handle.addEventListener("dragstart", (e) => {
      drag = row;
      e.dataTransfer!.effectAllowed = "move";
      e.dataTransfer!.setData("text/plain", "");
      e.dataTransfer!.setDragImage(el, 12, 12);
      el.classList.add("dragging");
    });
    handle.addEventListener("dragend", () => {
      drag = null;
      el.classList.remove("dragging");
    });
    const clear = () => el.classList.remove("drop-before", "drop-after", "drop-into");
    el.addEventListener("dragover", (e) => {
      const zone = zoneOf(e, el, row);
      clear();
      if (!canDrop(drag, row, zone)) return;
      e.preventDefault();
      el.classList.add(`drop-${zone}`);
    });
    el.addEventListener("dragleave", clear);
    el.addEventListener("drop", (e) => {
      e.preventDefault();
      clear();
      const zone = zoneOf(e, el, row);
      if (drag && canDrop(drag, row, zone)) drop(drag, row, zone);
    });
    el.append(handle);

    if (isSep) {
      el.append($("div", "sep-line"));
    } else {
      el.append($("span", "cm-face", ...(iconFace(row) ? [iconFace(row)!] : [])), $("span", "cm-label", labelOf(row)));
      const tag = n.folder ? "Folder" : n.kasumi ? "Kasumi" : n.cmd ? "Command" : "";
      if (tag) el.append($("span", "cm-tag", tag));
      if (n.shift) el.append($("span", "cm-tag", "Shift"));
    }
    // Folders and submenus open and close in place.
    if (row.parent) {
      const open = state.open.has(row.parent);
      const chev = iconButton(open ? 0xe70d : 0xe76c, open ? "Close" : "Open", () => {
        open ? state.open.delete(row.parent!) : state.open.add(row.parent!);
        render();
      });
      chev.classList.add("cm-chevron");
      el.append(chev);
    }
    const pick = () => {
      state.sel = key;
      render();
    };
    el.addEventListener("click", pick);
    el.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        pick();
      }
    });
    return el;
  };

  /** The clicked entry's options: name, whether and where it shows, icon, what it runs,
   *  Shift only, delete or reset. */
  const inspector = (row: Row | undefined): HTMLElement => {
    const box = $("div", "cm-inspector");
    if (!row) {
      box.append($("p", "card-help", "Click an entry in the menu to change its name, icon and where it shows."));
      return box;
    }
    const n = row.node;
    const isSep = !!n.separator && !row.entry;
    box.append($("div", "cm-inspector-title", isSep ? "Separator" : labelOf(row)));
    const here = KINDS.find(([k]) => k === state.kind)?.[1] ?? "";
    const own = !!(row.owner || OWN[state.kind]);
    if (!isSep) {
      const text = input(row.entry ? "" : n.folder || n.name || "", labelOf(row));
      text.addEventListener("change", () =>
        edit(row, (node) => {
          if (node.folder) node.folder = text.value || "Folder";
          else node.name = text.value;
        })(),
      );
      // Empty: Windows' own name (it shows in the field, greyed).
      box.append(field("Name", undefined, text));
    }
    const all = KINDS.map(([k]) => k);
    /** The kinds of click it shows for (hidden: none). */
    const kindsOf = (node: MenuNode): Set<string> => (node.hide ? new Set() : new Set(node.where ? node.where.split("|") : all));
    /** Shows for `set`: all of them is no `where`; none of them is hidden. */
    const setKinds = (node: MenuNode, set: Set<string>) => {
      node.hide = set.size === 0;
      node.where = set.size === 0 || set.size === all.length ? "" : all.filter((k) => set.has(k)).join("|");
    };
    // Shown here: for this kind of click (a submenu's or Kasumi's own menus: at all).
    const shown = toggleSwitch(!row.off, () =>
      edit(row, (node) => {
        if (own) {
          node.hide = !node.hide;
          return;
        }
        const set = kindsOf(node);
        if (row.off) set.add(state.kind);
        else set.delete(state.kind);
        setKinds(node, set);
      })(),
    );
    box.append(field(own ? "Show it" : `Show it on ${here.toLowerCase()}`, undefined, shown.el));
    // Where it shows, every kind of click at once.
    if (!own) {
      const chips = $("div", "cm-kinds");
      const on_ = kindsOf(n);
      for (const [k, label] of KINDS) {
        const on = on_.has(k);
        const chip = $("button", `cm-kind${on ? " on" : ""}`, label);
        chip.setAttribute("aria-pressed", String(on));
        chip.addEventListener(
          "click",
          edit(row, (node) => {
            const set = kindsOf(node);
            if (set.has(k)) set.delete(k);
            else set.add(k);
            setKinds(node, set);
          }),
        );
        chips.append(chip);
      }
      const none = on_.size === 0 ? [$("div", "card-help", "Shows nowhere: pick where it should.")] : [];
      box.append($("div", "cm-section", $("div", "cm-section-title", "Shows for"), chips, ...none));
    }
    if (!isSep) {
      const [iconBtn, iconGrid] = iconControl(row);
      box.append(field("Icon", undefined, iconBtn), iconGrid);
    }
    if (n.cmd) {
      const cmd = input(n.cmd, "Program (notepad.exe, code, C:\\...)");
      cmd.addEventListener("change", () => edit(row, (node) => (node.cmd = cmd.value))());
      const args = input(n.args ?? "", 'Arguments: {path}, {paths}, {dir} ("{path}")');
      args.addEventListener("change", () => edit(row, (node) => (node.args = args.value))());
      box.append($("div", "cm-section", $("div", "cm-section-title", "Runs"), cmd, args));
    }
    if ((n.folder || n.cmd) && !row.owner) {
      const shift = toggleSwitch(!!n.shift, (v) => edit(row, (node) => (node.shift = v))());
      box.append(field("Only while Shift is held", undefined, shift.el));
    }
    if (!row.entry && !n.rest) {
      const windows = !!n.match && !n.folder;
      const label = windows ? "Reset (Windows' own name and place)" : n.folder ? "Delete folder (its entries move out)" : "Delete";
      const del = $("button", "button cm-delete", label) as HTMLButtonElement;
      let armed: number | undefined;
      del.addEventListener("click", () => {
        if (armed === undefined) {
          del.classList.add("armed");
          del.textContent = windows ? "Click again to reset" : "Click again to delete";
          armed = window.setTimeout(() => {
            armed = undefined;
            del.classList.remove("armed");
            del.textContent = label;
          }, 3000);
          return;
        }
        window.clearTimeout(armed);
        state.sel = null;
        edit(row, (node, list) => {
          list.splice(list.indexOf(node), 1, ...(node.folder ? (node.items ?? []) : []));
        })();
      });
      box.append($("div", "cm-section", del));
    }
    return box;
  };

  const render = () => {
    const seen = seenAll[state.kind] ?? [];
    const list: Row[] = [];
    rows(layout, seen, claimedIn(layout, seen), 0, list);

    const tabs = $("div", "cm-tabs");
    for (const [k, label] of TABS) {
      const b = $("button", `cm-tab${k === state.kind ? " on" : ""}`, label) as HTMLButtonElement;
      b.addEventListener("click", () => {
        state.kind = k;
        pick();
        render();
      });
      tabs.append(b);
    }

    const table = $("div", "cm-rows");
    if (state.kind === "bar") {
      table.append($("p", "card-help", "The menu you get by right-clicking a bar's empty space. Power is hidden at first: its eye shows it."));
    } else if (state.kind === "launcher") {
      table.append(
        $(
          "p",
          "card-help",
          "The menu you get by right-clicking an app in the launcher: Windows' own entries for it (as in Start: Pin to Start, Run as administrator, Open file location, Uninstall…) and Kasumi's Pin to dock. Windows' entries show here once you've right-clicked an app.",
        ),
      );
    } else if (!seen.length) {
      table.append($("p", "card-help", "Kasumi hasn't seen this kind of menu yet: right-click one once (with Kasumi's menu on), then come back."));
    }
    table.setAttribute("role", "listbox");
    table.setAttribute("aria-label", "The menu");
    for (const row of list) table.append(rowEl(row));
    const selected = list.find((r) => state.sel && rowKey(r) === state.sel);

    const add = (n: MenuNode) => {
      layout.push(n);
      commit();
    };
    const missing = Object.keys(KASUMI).filter((id) => {
      const has = (l: MenuNode[]): boolean => l.some((n) => n.kasumi === id || (!!n.folder && has(n.items ?? [])));
      return !has(layout);
    });
    const addKasumi = select([["", "Add Kasumi's entry…"], ...missing.map((id) => [id, KASUMI[id].label] as [string, string])], "");
    addKasumi.hidden = !missing.length || !!OWN[state.kind];
    addKasumi.addEventListener("change", () => addKasumi.value && add({ kasumi: addKasumi.value }));
    const bFolder = $("button", "button", "New folder");
    const bCmd = $("button", "button", "New command");
    const bSep = $("button", "button", "Separator");
    const bReset = $("button", "button", "Reset to Kasumi's default");
    bFolder.addEventListener("click", () => {
      const name = "New folder";
      state.open.add(`folder:${name}`);
      add({ folder: name, icon: "E8B7", items: [] });
    });
    bCmd.addEventListener("click", () => add({ name: "New command", cmd: "notepad.exe", args: '"{path}"', icon: "E756" }));
    bSep.addEventListener("click", () => add({ separator: true }));
    bReset.addEventListener("click", async () => {
      if (!confirm("Go back to Kasumi's default menu? Your changes and your own entries go.")) return;
      await ctx.saveMany(
        OWN[state.kind]
          ? [{ path: [OWN[state.kind], "layout"], value: null }]
          : [
              { path: ["context_menu", "layout"], value: null },
              { path: ["context_menu", "custom"], value: null },
            ],
      );
    });

    box.replaceChildren(
      tabs,
      $("div", "cm-split", table, inspector(selected)),
      $("div", "button-row", bFolder, bCmd, bSep, addKasumi, bReset),
      $(
        "p",
        "card-help",
        "The menu as it opens for the kind of click picked above. Drag an entry by its handle to move it, or onto a folder to put it inside; click it to change it. Hidden entries stay here, struck through, to bring back. New entries come at the end. Changes apply to the next menu you open. Kasumi learns Windows' entries (and their submenus') as you use the menu: right-click a .zip or an image, or open a submenu, once to see its entries here.",
      ),
    );
  };

  /** From what's saved (Kasumi's default without a layout of your own). */
  const setup = (info: Info) => {
    seenAll = info.seen ?? {};
    const cm = (ctx.cfg as any).context_menu ?? {};
    const bm = (ctx.cfg as any).bar_menu ?? {};
    barLayout = structuredClone(bm.layout?.length ? bm.layout : bm.power ? [{ rest: true }] : [{ rest: true }, { match: ["verb:kasumi.power"], hide: true }]);
    const lm = (ctx.cfg as any).launcher_menu ?? {};
    launcherLayout = structuredClone(lm.layout?.length ? lm.layout : [{ rest: true }]);
    layout = mainLayout = structuredClone(cm.layout?.length ? cm.layout : info.default);
    // Your own entries (`custom`) become part of the layout where it said `custom`.
    const custom: MenuNode[] = structuredClone(cm.custom ?? []);
    const at = layout.findIndex((n) => n.custom);
    if (at >= 0) layout.splice(at, 1, ...custom);
    else layout.push(...custom);
    expandGroups(layout);
    if (!state.open.size) for (const n of layout) if (n.folder) state.open.add(`folder:${n.folder}`);
    pick();
    render();
  };

  if (state.info) setup(state.info);
  invoke<Info>("context_menu_info")
    .then((info) => {
      const changed = JSON.stringify(info) !== JSON.stringify(state.info);
      state.info = info;
      if (changed) setup(info);
    })
    .catch((e) => box.replaceChildren($("p", "card-help", `Couldn't load the menu: ${e}`)));
  return box;
};
