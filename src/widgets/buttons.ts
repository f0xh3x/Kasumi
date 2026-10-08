import { invoke } from "@tauri-apps/api/core";
import { el, icon, type Widget } from "./index";

/**
 * A row of icon buttons with the same actions as menu items (`items`: icon,
 * label, action, target, confirm). Several can be placed: `buttons`,
 * `buttons#2`, ... each with its own items. Icons size with the surface.
 */
interface Item {
  icon?: string;
  label?: string;
  action?: string;
  target?: string;
  confirm?: boolean;
}

export function buttons(opts: Record<string, unknown>): Widget {
  const items = (Array.isArray(opts.items) ? opts.items : []) as Item[];
  const root = el("widget buttons");
  for (const it of items) {
    if (!it.action) continue;
    const b = el("btn", icon(it.icon ?? String.fromCharCode(0xe8b7)));
    b.title = it.label ?? it.action;
    let armed = 0;
    b.addEventListener("click", (e) => {
      e.stopPropagation();
      if (it.confirm && !armed) {
        // Destructive actions take a second click within 3 s.
        b.classList.add("armed");
        b.title = `Click again: ${it.label ?? it.action}`;
        armed = window.setTimeout(() => {
          armed = 0;
          b.classList.remove("armed");
          b.title = it.label ?? it.action ?? "";
        }, 3000);
        return;
      }
      clearTimeout(armed);
      armed = 0;
      b.classList.remove("armed");
      invoke("run_action", { action: it.action, target: it.target ?? null }).catch(console.error);
    });
    root.append(b);
  }
  root.hidden = !root.children.length;
  return { el: root };
}
