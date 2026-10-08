import { invoke } from "@tauri-apps/api/core";
import { icon, type PanelContext, type Widget } from "./index";

interface MenuItem {
  label?: string;
  icon?: string;
  action?: string;
  target?: string;
  confirm?: boolean;
  separator?: boolean;
}

const CONFIRM_MS = 3000;

/** Hamburger dropdown: items from `[widgets.menu] items`, actions run in Rust. */
export function menu(opts: Record<string, unknown>, ctx: PanelContext): Widget {
  const items = Array.isArray(opts.items) ? (opts.items as MenuItem[]) : [];
  const el = document.createElement("div");
  el.className = "menu-list";
  el.dataset.align = "icon";
  const timers: number[] = [];

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

    let armed = 0;
    row.addEventListener("click", () => {
      // Destructive items need a second click while armed.
      if (item.confirm && !armed) {
        row.classList.add("armed");
        label.textContent = `Click again to ${(item.label ?? "confirm").toLowerCase()}`;
        armed = window.setTimeout(() => {
          armed = 0;
          row.classList.remove("armed");
          label.textContent = item.label ?? "";
        }, CONFIRM_MS);
        timers.push(armed);
        return;
      }
      clearTimeout(armed);
      armed = 0;
      row.classList.remove("armed");
      label.textContent = item.label ?? "";
      invoke("run_action", { action: item.action, target: item.target ?? null }).catch((e) =>
        console.error("run_action", e),
      );
      ctx.close();
    });
    el.append(row);
  }

  return { el, dispose: () => timers.forEach(clearTimeout) };
}
