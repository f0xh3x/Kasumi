// The context menu editor ([context_menu] layout): the menu as it shows for one kind
// of click (file, folder, drive, several items, empty space, desktop), and for each
// entry: move it, hide it, rename it, give it an icon, put it in a folder, show it
// only for some kinds of click. Plus your own folders, commands and separators.
//
// The layout is one list for every kind of click (each entry can say where it shows).
// Windows' entries Kasumi hasn't placed come where the layout says "rest"; editing
// one of those places them all, in the order they show.
import { invoke } from "@tauri-apps/api/core";
import { $, iconButton, input, select, toggleSwitch } from "./dom";
import { iconPicker, type EditorCtx } from "./editors";

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
}

const KINDS: [string, string][] = [
  ["file", "File"],
  ["folder", "Folder"],
  ["drive", "Drive"],
  ["multiple", "Several items"],
  ["background", "Folder background"],
  ["desktop", "Desktop"],
];

const BUILTIN: Record<string, string> = {
  view: "View, Sort by, Refresh",
  terminal: "Command Prompt, PowerShell, Windows Terminal",
  file_manage: "Copy path, Select, Show / Hide, Attributes, New, Folder options",
  go_to: "Folders, Control Panel, Run, Settings pages",
};

/** `text:Pin to*` / `verb:copyaspath` against a Windows entry (as layout.rs does). */
function matches(m: string, e: Seen): boolean {
  const i = m.indexOf(":");
  const [kind, pat] = i < 0 ? ["text", m] : [m.slice(0, i), m.slice(i + 1)];
  const hay = (kind === "verb" ? e.verb : e.text).toLowerCase();
  if (!hay) return false;
  const p = pat.toLowerCase();
  return p.endsWith("*") ? hay.startsWith(p.slice(0, -1)) : hay === p;
}

const keyOf = (e: Seen) => (e.verb ? `verb:${e.verb}` : `text:${e.text}`);
const applies = (n: MenuNode, kind: string) => !n.where || n.where.split("|").some((w) => w.trim() === kind);
const hexOf = (ch: string) => ch.codePointAt(0)!.toString(16).toUpperCase();
const charOf = (hex?: string) => (hex ? String.fromCodePoint(parseInt(hex, 16) || 0xe8b7) : "");

/** A node as saved: no empty or default fields. */
function clean(n: MenuNode): MenuNode {
  const out: any = {};
  for (const [k, v] of Object.entries(n)) {
    if (v === "" || v === false || v == null || (Array.isArray(v) && v.length === 0 && k !== "items")) continue;
    out[k] = k === "items" ? (v as MenuNode[]).map(clean) : v;
  }
  if (n.folder) out.items = (n.items ?? []).map(clean);
  return out;
}

interface Row {
  node: MenuNode;
  list: MenuNode[];
  /** A Windows entry still in "rest" (not placed by the layout yet). */
  entry?: Seen;
  depth: number;
}

export const contextMenuEditor = (ctx: EditorCtx): HTMLElement => {
  const box = $("div", "cm-editor", $("p", "card-help", "Loading…"));
  let layout: MenuNode[] = [];
  let seenAll: Record<string, Seen[]> = {};
  let kind = "file";

  const save = () =>
    ctx
      .saveMany([
        { path: ["context_menu", "layout"], value: layout.map(clean) },
        { path: ["context_menu", "custom"], value: null },
      ])
      .catch((e) => ctx.toast(String(e), true));

  /** Every Windows entry the layout names (placed or hidden) — "rest" has the others. */
  const claimed = (seen: Seen[]) => {
    const used = new Set<number>();
    const walk = (list: MenuNode[]) => {
      for (const n of list) {
        n.match?.forEach((m) => seen.forEach((e, i) => matches(m, e) && used.add(i)));
        if (n.items) walk(n.items);
      }
    };
    walk(layout);
    return used;
  };

  /** Places the rest's entries for this kind of click, so they can be edited. */
  const materialize = (seen: Seen[]) => {
    const used = claimed(seen);
    const find = (list: MenuNode[]): [MenuNode[], number] | null => {
      for (let i = 0; i < list.length; i++) {
        if (list[i].rest) return [list, i];
        const inner = list[i].items && find(list[i].items!);
        if (inner) return inner;
      }
      return null;
    };
    const at = find(layout);
    if (!at) return;
    const [list, i] = at;
    const add = seen.filter((_, j) => !used.has(j)).map((e) => ({ match: [keyOf(e)] }));
    list.splice(i, 0, ...add);
  };

  /** The rows for this kind of click: what the menu shows, hidden ones too. */
  const rows = (list: MenuNode[], seen: Seen[], used: Set<number>, depth: number, out: Row[]) => {
    for (const n of list) {
      if (!applies(n, kind)) continue;
      if (n.rest) {
        seen.forEach((e, i) => !used.has(i) && out.push({ node: n, list, entry: e, depth }));
      } else if (n.folder) {
        out.push({ node: n, list, depth });
        rows(n.items ?? [], seen, used, depth + 1, out);
      } else if (n.kasumi) {
        if (n.kasumi !== "view" || kind === "background" || kind === "desktop") out.push({ node: n, list, depth });
      } else if (n.match?.length) {
        if (seen.some((e) => n.match!.some((m) => matches(m, e)))) out.push({ node: n, list, depth });
      } else {
        out.push({ node: n, list, depth });
      }
    }
  };

  /** Does a node show a row for this kind of click (for moving past it)? */
  const shows = (n: MenuNode, seen: Seen[], used: Set<number>) => {
    const r: Row[] = [];
    rows([n], seen, used, 0, r);
    return r.length > 0;
  };

  /** A row's node, placing the rest first if it's one of the rest's entries. */
  const resolve = (row: Row, seen: Seen[]): [MenuNode, MenuNode[]] | null => {
    if (!row.entry) return [row.node, row.list];
    materialize(seen);
    const key = keyOf(row.entry);
    let hit: [MenuNode, MenuNode[]] | null = null;
    const walk = (list: MenuNode[]) => {
      for (const n of list) {
        if (!hit && n.match?.length === 1 && n.match[0] === key) hit = [n, list];
        if (n.items) walk(n.items);
      }
    };
    walk(layout);
    return hit;
  };

  const folders = () => {
    const out: [MenuNode, string][] = [];
    const walk = (list: MenuNode[], prefix: string) => {
      for (const n of list)
        if (n.folder) {
          out.push([n, prefix + n.folder]);
          walk(n.items ?? [], `${prefix}${n.folder} › `);
        }
    };
    walk(layout, "");
    return out;
  };

  const contains = (outer: MenuNode, inner: MenuNode): boolean =>
    outer === inner || (outer.items ?? []).some((c) => contains(c, inner));

  const restIndex = () => layout.findIndex((n) => n.rest);

  const render = () => {
    const seen = seenAll[kind] ?? [];
    const used = claimed(seen);
    const list: Row[] = [];
    rows(layout, seen, used, 0, list);

    const tabs = $("div", "cm-tabs");
    for (const [k, label] of KINDS) {
      const b = $("button", `cm-tab${k === kind ? " on" : ""}`, label) as HTMLButtonElement;
      b.addEventListener("click", () => {
        kind = k;
        render();
      });
      tabs.append(b);
    }

    const table = $("div", "cm-rows");
    if (!seen.length) {
      table.append($("p", "card-help", "Kasumi hasn't seen this kind of menu yet: right-click one once (with Kasumi's menu on), then come back."));
    }
    for (const row of list) table.append(rowEl(row, seen));

    const add = (n: MenuNode) => () => {
      const i = restIndex();
      layout.splice(i < 0 ? layout.length : i + 1, 0, n);
      render();
      save();
    };
    const buttons = $(
      "div",
      "button-row",
      $("button", "button", "New folder"),
      $("button", "button", "New command"),
      $("button", "button", "Separator"),
      $("button", "button", "Reset to Kasumi's default"),
    );
    const [bFolder, bCmd, bSep, bReset] = [...buttons.children] as HTMLButtonElement[];
    bFolder.addEventListener("click", add({ folder: "New folder", icon: "E8B7", items: [] }));
    bCmd.addEventListener("click", add({ name: "New command", cmd: "notepad.exe", args: '"{path}"', icon: "E756" }));
    bSep.addEventListener("click", add({ separator: true }));
    bReset.addEventListener("click", async () => {
      if (!confirm("Go back to Kasumi's default menu? Your changes and your own entries go.")) return;
      await ctx.saveMany([
        { path: ["context_menu", "layout"], value: null },
        { path: ["context_menu", "custom"], value: null },
      ]);
      await load(true);
    });

    box.replaceChildren(
      tabs,
      table,
      buttons,
      $(
        "p",
        "card-help",
        "Changes apply to the next menu you open. Entries show for every kind of click unless you limit them (⋯). Kasumi learns Windows' entries as you use the menu: right-click a .zip or an image once to see the entries only they have.",
      ),
    );
  };

  const rowEl = (row: Row, seen: Seen[]): HTMLElement => {
    const n = row.node;
    const el = $("div", `cm-row${n.hide ? " hidden-row" : ""}${n.folder ? " folder-row" : ""}`);
    el.style.paddingLeft = `${row.depth * 22}px`;
    const edit = (f: (node: MenuNode, list: MenuNode[]) => void) => () => {
      const r = resolve(row, seen);
      if (!r) return;
      f(r[0], r[1]);
      render();
      save();
    };

    if (n.separator && !row.entry) {
      el.append($("div", "sep-line"));
    } else {
      const label = row.entry
        ? row.entry.text
        : n.folder
          ? n.folder
          : n.kasumi
            ? `Kasumi: ${BUILTIN[n.kasumi] ?? n.kasumi}`
            : n.cmd
              ? n.name || n.cmd
              : n.name ||
                seen
                  .filter((e) => n.match!.some((m) => matches(m, e)))
                  .map((e) => e.text)
                  .join(", ");
      const [iconBtn, iconGrid] = iconPicker(charOf(n.icon), (ch) => edit((node) => (node.icon = hexOf(ch)))());
      const text = input(n.folder || n.name || "", label);
      text.title = row.entry || n.match ? `Windows' name: ${label}` : "";
      text.addEventListener("change", () =>
        edit((node) => {
          if (node.folder !== undefined && node.folder !== "") node.folder = text.value || "Folder";
          else node.name = text.value;
        })(),
      );
      if (n.kasumi) text.disabled = true;
      const tag = $(
        "span",
        "cm-tag",
        n.folder ? "Folder" : n.kasumi ? "Kasumi" : n.cmd ? "Command" : (row.entry ?? seen.find((e) => n.match!.some((m) => matches(m, e))))?.sub ? "Windows ›" : "Windows",
      );
      el.append(iconBtn, text, tag);
      if (n.shift) el.append($("span", "cm-tag", "Shift"));
      el.append(iconGrid);
    }

    // Move up / down among what this kind of click shows.
    const move = (dir: -1 | 1) =>
      edit((node, list) => {
        const vis = list.filter((x) => shows(x, seen, claimed(seen)));
        const i = vis.indexOf(node);
        const next = vis[i + dir];
        if (!next) return;
        list.splice(list.indexOf(node), 1);
        list.splice(list.indexOf(next) + (dir > 0 ? 1 : 0), 0, node);
      });
    const into = select(
      [
        ["", "Move to…"],
        ["@top", "The menu itself"],
        ...folders()
          .filter(([f]) => !contains(n, f))
          .map(([, path], i) => [String(i), path] as [string, string]),
      ],
      "",
    );
    into.title = "Move into a folder";
    into.addEventListener("change", () => {
      const all = folders().filter(([f]) => !contains(n, f));
      const pick = into.value;
      edit((node, list) => {
        list.splice(list.indexOf(node), 1);
        if (pick === "@top") {
          const i = restIndex();
          layout.splice(i < 0 ? layout.length : i + 1, 0, node);
        } else {
          const f = all[Number(pick)]?.[0];
          if (f) (f.items ??= []).push(node);
          else list.push(node);
        }
      })();
    });

    const details = $("div", "cm-details");
    details.hidden = true;
    const actions = $(
      "div",
      "menu-row-actions",
      iconButton(0xe70e, "Move up", move(-1)),
      iconButton(0xe70d, "Move down", move(1)),
      iconButton(n.hide ? 0xed1a : 0xe7b3, n.hide ? "Show" : "Hide", edit((node) => (node.hide = !node.hide))),
      into,
      iconButton(0xe712, "More", () => (details.hidden = !details.hidden)),
      iconButton(0xe74d, n.match && !n.folder ? "Back to Windows' place" : "Remove", edit((node, list) => {
        const i = list.indexOf(node);
        list.splice(i, 1, ...(node.folder ? (node.items ?? []) : []));
      })),
    );
    el.append(actions);

    // Where it shows; Shift; a command's program and arguments.
    const where = $("div", "cm-where", $("span", "row-help", "Shows for:"));
    const on = new Set((n.where || KINDS.map(([k]) => k).join("|")).split("|"));
    for (const [k, label] of KINDS) {
      const b = $("button", `cm-tab small${on.has(k) ? " on" : ""}`, label) as HTMLButtonElement;
      b.addEventListener("click", () =>
        edit((node) => {
          const set = new Set((node.where || KINDS.map(([x]) => x).join("|")).split("|"));
          set.has(k) ? set.delete(k) : set.add(k);
          node.where = set.size === KINDS.length ? "" : [...set].join("|");
        })(),
      );
      where.append(b);
    }
    details.append(where);
    if (n.folder || n.cmd) {
      const shift = toggleSwitch(!!n.shift, (v) => edit((node) => (node.shift = v))());
      details.append($("div", "cm-where", $("span", "row-help", "Only with Shift held"), shift.el));
    }
    if (n.cmd) {
      const cmd = input(n.cmd, "Program (notepad.exe, code, C:\\...)");
      cmd.addEventListener("change", () => edit((node) => (node.cmd = cmd.value))());
      const args = input(n.args ?? "", 'Arguments: {path}, {paths}, {dir} ("{path}")');
      args.addEventListener("change", () => edit((node) => (node.args = args.value))());
      details.append($("div", "cm-where", cmd, args));
    }
    return $("div", "cm-item", el, details);
  };

  const load = async (fresh = false) => {
    try {
      const info = await invoke<{ seen: Record<string, Seen[]>; default: MenuNode[] }>("context_menu_info");
      seenAll = info.seen ?? {};
      const cm = fresh ? {} : (ctx.cfg as any).context_menu ?? {};
      layout = structuredClone(cm.layout?.length ? cm.layout : info.default);
      // Your own entries (`custom`) become part of the layout where it said `custom`.
      const custom: MenuNode[] = structuredClone(cm.custom ?? []);
      const at = layout.findIndex((n) => n.custom);
      if (at >= 0) layout.splice(at, 1, ...custom);
      else layout.push(...custom);
      render();
    } catch (e) {
      box.replaceChildren($("p", "card-help", `Couldn't load the menu: ${e}`));
    }
  };
  load();
  return box;
};
