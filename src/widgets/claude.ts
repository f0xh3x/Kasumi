import { subscribe, type ClaudeUsage } from "../data";
import { meter, el, type PanelContext, type Widget } from "./index";

/** 1234 -> "1.2k", 3_400_000 -> "3.4M". */
export function tokens(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)}B`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(n >= 1e7 ? 0 : 1)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(n >= 1e4 ? 0 : 1)}k`;
  return String(n);
}

const duration = (secs: number) => {
  const m = Math.max(0, Math.round(secs / 60));
  return m >= 60 ? `${Math.floor(m / 60)}h ${m % 60}m` : `${m}m`;
};

const num = (v: unknown, d: number) => (typeof v === "number" && Number.isFinite(v) ? v : d);

/** Local midnight `days` days ago, in unix seconds. */
const dayStart = (days: number) => {
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  d.setDate(d.getDate() - days);
  return d.getTime() / 1000;
};

interface Totals {
  tokens: number;
  cacheRead: number;
  messages: number;
  sessions: Set<string>;
  models: Map<string, number>;
}

function totals(u: ClaudeUsage | null, from: number, to = Infinity): Totals {
  const t: Totals = { tokens: 0, cacheRead: 0, messages: 0, sessions: new Set(), models: new Map() };
  for (const b of u?.buckets ?? []) {
    if (b.h < from || b.h >= to) continue;
    t.tokens += b.tokens;
    t.cacheRead += b.cacheRead;
    t.messages += b.messages;
    b.sessions.forEach((s) => t.sessions.add(s));
    t.models.set(b.model, (t.models.get(b.model) ?? 0) + b.tokens);
  }
  return t;
}

/** The share shown by the indicator (0..1): of the limit, else (5-hour window) of the time. */
function share(u: ClaudeUsage | null, opts: Record<string, unknown>): number {
  if (opts.indicator_for === "week") {
    const limit = num(opts.weekly_limit, 0);
    return limit > 0 ? totals(u, dayStart(6)).tokens / limit : 0;
  }
  const w = u?.window;
  if (!w) return 0;
  const limit = num(opts.window_limit, 0);
  return limit > 0 ? w.tokens / limit : (Date.now() / 1000 - w.start) / (w.end - w.start);
}

/** Claude Code usage: text, an indicator and a "working" dot, each optional. */
export function claude(opts: Record<string, unknown>): Widget {
  const kind = opts.indicator === "none" || opts.indicator === "circle" ? opts.indicator : "bar";
  const ind = kind === "none" ? null : meter(kind, num(opts.bar_length, 40));
  const text = el("claude-text");
  const dot = el("claude-dot");
  const showText = opts.text !== false;
  const parts: HTMLElement[] = [];
  if (opts.working !== false) parts.push(dot);
  if (ind && opts.indicator_side !== "right") parts.push(ind.el);
  if (showText) parts.push(text);
  if (ind && opts.indicator_side === "right") parts.push(ind.el);
  const root = el("widget claude", ...parts);
  let latest: ClaudeUsage | null = null;

  const draw = () => {
    const u = latest;
    const now = Date.now() / 1000;
    const w = u?.window && now < u.window.end ? u.window : null;
    const bits: string[] = [];
    if (opts.show_window !== false) bits.push(w ? tokens(w.tokens) : "0");
    if (opts.show_reset !== false && w) bits.push(duration(w.end - now));
    if (opts.show_today === true) bits.push(`today ${tokens(totals(u, dayStart(0)).tokens)}`);
    if (opts.show_week_left === true) {
      const limit = num(opts.weekly_limit, 0);
      const used = totals(u, dayStart(6)).tokens;
      bits.push(limit > 0 ? `${tokens(Math.max(0, limit - used))} left` : `week ${tokens(used)}`);
    }
    text.textContent = bits.join(" · ");
    text.hidden = !bits.length;
    const v = share(u, opts);
    ind?.set(v);
    root.classList.toggle("high", v >= 0.9);
    dot.classList.toggle("busy", !!u?.busy);
    root.title = w ? `Claude Code: ${tokens(w.tokens)} tokens in this 5-hour window, resets in ${duration(w.end - now)}` : "Claude Code: no 5-hour window running";
  };
  const off = subscribe("claude", (u) => {
    latest = u;
    draw();
  });
  // The countdown moves on between updates.
  const timer = window.setInterval(draw, 30_000);
  return { el: root, dispose: () => (off(), clearInterval(timer)) };
}

/** The 5-hour window, today (per model), and the last 7 days. */
export function claudePanel(opts: Record<string, unknown>, ctx: PanelContext): Widget {
  const root = el("claude-panel");
  const row = (label: string, value: string, dim = false) => el(`claude-row${dim ? " dim" : ""}`, el("claude-label", label), el("claude-value", value));
  const draw = (u: ClaudeUsage | null) => {
    const now = Date.now() / 1000;
    const w = u?.window && now < u.window.end ? u.window : null;
    const wl = num(opts.window_limit, 0);
    const kl = num(opts.weekly_limit, 0);
    const sections: HTMLElement[] = [];
    const head = (t: string) => el("claude-head", t);

    sections.push(head("5-hour window"));
    if (w) {
      const resets = new Date(w.end * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
      sections.push(
        row("Used", wl > 0 ? `${tokens(w.tokens)} of ${tokens(wl)}` : tokens(w.tokens)),
        ...(wl > 0 ? [row("Left", tokens(Math.max(0, wl - w.tokens)))] : []),
        row("Resets", `${resets} (in ${duration(w.end - now)})`),
        row("Messages", String(w.messages), true),
      );
    } else sections.push(row("Not running", "starts with your next message", true));

    const today = totals(u, dayStart(0));
    sections.push(head("Today"), row("Tokens", tokens(today.tokens)), row("Messages", String(today.messages), true), row("Sessions", String(today.sessions.size), true));
    for (const [model, t] of [...today.models].sort((a, b) => b[1] - a[1])) sections.push(row(model.replace(/^claude-/, ""), tokens(t), true));
    if (today.cacheRead) sections.push(row("Cache reads", tokens(today.cacheRead), true));

    if (opts.weekly !== false) {
      const week = totals(u, dayStart(6));
      sections.push(head("Last 7 days"), row("Tokens", kl > 0 ? `${tokens(week.tokens)} of ${tokens(kl)}` : tokens(week.tokens)));
      if (kl > 0) sections.push(row("Left", tokens(Math.max(0, kl - week.tokens))));
      const days = Array.from({ length: 7 }, (_, i) => totals(u, dayStart(6 - i), dayStart(5 - i)).tokens);
      const max = Math.max(1, ...days);
      const chart = el(
        "claude-chart",
        ...days.map((t, i) => {
          const d = new Date((dayStart(6 - i) + 3600) * 1000);
          const col = el("claude-col", el("claude-col-bar"), el("claude-col-day", d.toLocaleDateString([], { weekday: "narrow" })));
          (col.firstElementChild as HTMLElement).style.height = `${Math.max(2, (t / max) * 48)}px`;
          col.title = `${d.toLocaleDateString([], { weekday: "long" })}: ${tokens(t)}`;
          return col;
        }),
      );
      sections.push(chart);
    }
    root.replaceChildren(...sections);
    ctx.resize();
  };
  const off = subscribe("claude", draw);
  return { el: root, dispose: off };
}
