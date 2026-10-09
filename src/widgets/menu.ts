import { invoke } from "@tauri-apps/api/core";
import { confirmClick, icon, type PanelContext, type Widget } from "./index";

interface MenuItem {
  label?: string;
  icon?: string;
  action?: string;
  target?: string;
  confirm?: boolean;
  separator?: boolean;
}

/** Hamburger dropdown: items from `[widgets.menu] items`, actions run in Rust. */
export function menu(opts: Record<string, unknown>, ctx: PanelContext): Widget {
  const items = Array.isArray(opts.items) ? (opts.items as MenuItem[]) : [];
  const el = document.createElement("div");
  el.className = "menu-list";
  el.dataset.align = "icon";
  const disposers: (() => void)[] = [];

  for (const item of items) {
    if (item.separator) {
      el.append(Object.assign(document.createElement("div"), { className: "menu-sep" }));
      continue;
    }
    const row = document.createElement("div");
    row.className = "menu-item";
    const label = document.createElement("span");
    label.className = "menu-label";
    label.textContent = item.label ?? item.action ?? "?";
    row.append(icon(item.icon ?? ""), label);

    // Destructive items need a second click while armed.
    disposers.push(
      confirmClick(
        row,
        !!item.confirm,
        () => {
          invoke("run_action", { action: item.action, target: item.target ?? null }).catch((e) => console.error("run_action", e));
          ctx.close();
        },
        (on) => (label.textContent = on ? `Click again to ${(item.label ?? "confirm").toLowerCase()}` : (item.label ?? "")),
      ),
    );
    el.append(row);
  }

  return { el, dispose: () => disposers.forEach((d) => d()) };
}
