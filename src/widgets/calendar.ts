import type { Widget } from "./index";

const MONTHS = [
  "January", "February", "March", "April", "May", "June",
  "July", "August", "September", "October", "November", "December",
];

/** ISO-8601 week number. */
function isoWeek(d: Date): number {
  const t = new Date(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()));
  const day = t.getUTCDay() || 7;
  t.setUTCDate(t.getUTCDate() + 4 - day);
  const yearStart = Date.UTC(t.getUTCFullYear(), 0, 1);
  return Math.ceil(((t.getTime() - yearStart) / 86400000 + 1) / 7);
}

function cell(text: string, className: string): HTMLDivElement {
  const c = document.createElement("div");
  c.className = className;
  c.textContent = text;
  return c;
}

/** Month view, Monday first, with ISO week numbers. */
export function calendar(): Widget {
  const el = document.createElement("div");
  el.className = "panel-calendar";
  let shownDay = -1;
  let timer = 0;

  const render = () => {
    const today = new Date();
    shownDay = today.getDate();
    el.replaceChildren();
    el.append(cell(`${MONTHS[today.getMonth()]} ${today.getFullYear()}`, "cal-title"));

    const grid = document.createElement("div");
    grid.className = "cal-grid";
    grid.append(cell("", "cal-wk"));
    for (const d of ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"]) grid.append(cell(d, "cal-dow"));

    const first = new Date(today.getFullYear(), today.getMonth(), 1);
    const start = new Date(first);
    start.setDate(1 - ((first.getDay() + 6) % 7));
    for (let week = 0; week < 6; week++) {
      const rowStart = new Date(start);
      rowStart.setDate(start.getDate() + week * 7);
      grid.append(cell(String(isoWeek(rowStart)), "cal-wk"));
      for (let i = 0; i < 7; i++) {
        const d = new Date(rowStart);
        d.setDate(rowStart.getDate() + i);
        let cls = "cal-day";
        if (d.getMonth() !== today.getMonth()) cls += " other";
        if (d.toDateString() === today.toDateString()) cls += " today";
        grid.append(cell(String(d.getDate()), cls));
      }
    }
    el.append(grid);
  };

  render();
  timer = window.setInterval(() => {
    if (new Date().getDate() !== shownDay) render();
  }, 60_000);
  return { el, dispose: () => clearInterval(timer) };
}
