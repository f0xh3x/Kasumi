import { invoke } from "@tauri-apps/api/core";
import type { Widget } from "./index";

const pad = (n: number) => String(n).padStart(2, "0");
const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** strftime subset: %H %I %M %S %p %d %m %Y %y %a %b %%. */
export function strftime(fmt: string, d: Date): string {
  return fmt.replace(/%([HIMSpdmYyab%])/g, (_, c: string) => {
    switch (c) {
      case "H": return pad(d.getHours());
      case "I": return pad(d.getHours() % 12 || 12);
      case "M": return pad(d.getMinutes());
      case "S": return pad(d.getSeconds());
      case "p": return d.getHours() < 12 ? "AM" : "PM";
      case "d": return pad(d.getDate());
      case "m": return pad(d.getMonth() + 1);
      case "Y": return String(d.getFullYear());
      case "y": return pad(d.getFullYear() % 100);
      case "a": return DAYS[d.getDay()];
      case "b": return MONTHS[d.getMonth()];
      default: return "%";
    }
  });
}

export function clock(opts: Record<string, unknown>): Widget {
  let format = typeof opts.format === "string" ? opts.format : "%H:%M";
  if (format === "system") {
    format = "%H:%M";
    // Windows' own short time format (Region settings).
    invoke<string>("system_time_format").then((f) => {
      format = f;
      el.textContent = strftime(format, new Date());
    });
  }
  const el = document.createElement("div");
  el.className = "widget clock";
  let timer = 0;
  const tick = () => {
    const now = new Date();
    el.textContent = strftime(format, now);
    // Re-align to the next whole second so the minute flips on time.
    timer = window.setTimeout(tick, 1000 - now.getMilliseconds() + 5);
  };
  tick();
  return { el, dispose: () => clearTimeout(timer) };
}
