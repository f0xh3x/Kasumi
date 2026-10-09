// DOM helpers shared by the settings window's pages (main.ts) and its
// structured editors (editors.ts).

export const $ = <K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, ...kids: (Node | string)[]) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  e.append(...kids);
  return e;
};

export const glyph = (code: number) => $("span", "icon", String.fromCharCode(code));

export function input(value: string, placeholder = "", cls = "input"): HTMLInputElement {
  const i = $("input", cls) as HTMLInputElement;
  i.value = value;
  i.placeholder = placeholder;
  return i;
}

export function select(options: [string, string][], value: string): HTMLSelectElement {
  const s = $("select", "select") as HTMLSelectElement;
  for (const [v, label] of options) s.append(new Option(label, v));
  s.value = value;
  return s;
}

export function iconButton(code: number, title: string, onClick: () => void): HTMLButtonElement {
  const b = $("button", "icon-button", glyph(code)) as HTMLButtonElement;
  b.title = title;
  b.addEventListener("click", onClick);
  return b;
}

/** An on/off switch; `set` changes it without calling `onChange`. */
export function toggleSwitch(on: boolean, onChange: (on: boolean) => void) {
  const el = $("button", "switch") as HTMLButtonElement;
  el.setAttribute("role", "switch");
  const set = (v: boolean) => {
    on = v;
    el.classList.toggle("on", v);
    el.setAttribute("aria-checked", String(v));
  };
  set(on);
  el.addEventListener("click", () => {
    set(!on);
    onChange(on);
  });
  return { el, set };
}

/** A settings row: label (and help) on the left, its control on the right. */
export function row(label: string, help: string | undefined, control: HTMLElement): HTMLElement {
  const text = $("div", "row-text", $("div", "row-label", label));
  if (help) text.append($("div", "row-help", help));
  return $("div", "row", text, $("div", "row-control", control));
}
