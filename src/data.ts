import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// Mirrors the Rust data providers (src-tauri/src/providers). Values are pushed
// on change as "notch://data" events; `get_data` returns the latest snapshot.

export interface Temps {
  ok: boolean;
  cpu: number | null;
  gpu: number | null;
  cpuName?: string;
  gpuName?: string;
  cpuLoad?: number | null;
  cpuPower?: number | null;
  /** MHz */
  cpuClock?: number | null;
  gpuLoad?: number | null;
  gpuPower?: number | null;
  /** MB */
  vramUsed?: number | null;
  vramTotal?: number | null;
  /** GB */
  ramUsed?: number | null;
  ramAvailable?: number | null;
}

export interface Media {
  title: string;
  artist: string;
  playing: boolean;
  canPrevious: boolean;
  canNext: boolean;
  canPlayPause: boolean;
  canSeek: boolean;
  /** Position as of `updatedAt` (Unix ms); extrapolate while playing. */
  positionMs: number;
  durationMs: number;
  updatedAt: number;
  /** Album art as a data URL. */
  art: string | null;
  app: string;
}

export interface Weather {
  temp: number;
  feels: number;
  code: number;
  isDay: boolean;
  min: number;
  max: number;
  location: string;
  hourly: HourlyWeather[];
}

export interface HourlyWeather {
  /** Local time, e.g. "2026-10-07T16:00". */
  time: string;
  temp: number;
  code: number;
  isDay: boolean;
}

export interface TrayIcon {
  id: string;
  tip: string;
  exe: string;
  w: number;
  h: number;
  /** Base64 RGBA pixels. */
  rgba: string;
}

export interface Keyboard {
  /** ISO 639-2 code, e.g. "ENG", "LIT". */
  code: string;
  /** ISO 639-1 code, e.g. "EN", "LT". */
  code2: string;
  name: string;
}

export interface TaskApp {
  key: string;
  name: string;
  exe: string;
  aumid: string | null;
  /** Pinned (listed even when not running). */
  pinned: boolean;
  /** Its entry in the pin list (an app key or a dropped file's path). */
  pin: string | null;
}

export interface TaskWindow {
  hwnd: number;
  title: string;
  /** TaskApp key. */
  app: string;
  active: boolean;
  minimized: boolean;
  /** Monitor device name. */
  monitor: string;
  /** Z order: 0 = most recently in front. */
  z: number;
}

export interface Tasks {
  apps: TaskApp[];
  windows: TaskWindow[];
}

export interface SystemStatus {
  /** Null on PCs without a battery. */
  battery: { percent: number | null; plugged: boolean; charging: boolean; secondsLeft: number | null; saver: boolean } | null;
  volume: { level: number; muted: boolean } | null;
  /** Built-in panel brightness 0..100; null where it can't be controlled. */
  brightness: number | null;
  network: { connected: boolean; internet?: boolean; name: string; wifi: boolean } | null;
}

/** Claude Code usage from its local session files (see providers/claude.rs). */
/** A plan limit's use (the Claude account's, as /usage shows it). */
export interface ClaudeLimit {
  percent: number;
  /** ISO time it resets. */
  resetsAt: string | null;
}

export interface ClaudeUsage {
  buckets: { h: number; model: string; tokens: number; cacheRead: number; messages: number; sessions: string[] }[];
  window: { start: number; end: number; tokens: number; messages: number } | null;
  busy: boolean;
  now: number;
  /** From the Claude account ([widgets.claude] account); null: off. */
  account?: {
    session?: ClaudeLimit | null;
    week?: ClaudeLimit | null;
    weekOpus?: ClaudeLimit | null;
    weekSonnet?: ClaudeLimit | null;
    error?: "signed-out" | "expired" | "unavailable";
  } | null;
}

interface DataMap {
  system: SystemStatus | null;
  tasks: Tasks | null;
  taskIcons: Record<string, { w: number; h: number; rgba: string }> | null;
  temps: Temps | null;
  media: Media | null;
  weather: Weather | null;
  tray: TrayIcon[] | null;
  keyboard: Keyboard | null;
  focus: { title: string; app: string } | null;
  netspeed: { down: number; up: number } | null;
  /** Komorebi: per monitor ("DISPLAY4"), its workspaces and the focused one. */
  workspaces: { focused: number; monitors: { name: string; focused: number; workspaces: { name: string; windows: number }[] }[] } | null;
  disks: { id: string; label: string; total: number; free: number }[] | null;
  privacy: { mic: string[]; camera: string[]; location: string[] } | null;
  claude: ClaudeUsage | null;
  notifications: { count: number; unseen: number; items: { id: number; app: string; aumid: string; title: string; body: string; time: number }[] } | null;
  clipboard: { id: number; time: number; kind: "text" | "image"; text?: string; bytes?: number; thumb?: { w: number; h: number; rgba: string } | null }[] | null;
}

type Key = keyof DataMap;

const values = new Map<Key, unknown>();
const listeners = new Map<Key, Set<(v: never) => void>>();

function set(key: Key, value: unknown): void {
  values.set(key, value);
  listeners.get(key)?.forEach((cb) => cb(value as never));
}

/** Calls `cb` now (if a value is known) and on every change. Returns an unsubscribe function. */
export function subscribe<K extends Key>(key: K, cb: (value: DataMap[K]) => void): () => void {
  let set = listeners.get(key);
  if (!set) listeners.set(key, (set = new Set()));
  set.add(cb as (v: never) => void);
  if (values.has(key)) cb(values.get(key) as DataMap[K]);
  return () => set.delete(cb as (v: never) => void);
}

/** Keys whose events carry only the entries that changed (Rust: `publish_entries`). */
const MERGED: Key[] = ["taskIcons"];

export async function initData(): Promise<void> {
  await listen<{ key: Key; value: unknown }>("notch://data", (e) => {
    const { key, value } = e.payload;
    set(key, MERGED.includes(key) ? { ...(values.get(key) as object | undefined), ...(value as object) } : value);
  });
  const snapshot = await invoke<Partial<Record<Key, unknown>>>("get_data");
  for (const [key, value] of Object.entries(snapshot) as [Key, unknown][]) {
    // The snapshot is whole; entries that arrived while it was on its way are newer.
    if (MERGED.includes(key)) set(key, { ...(value as object), ...(values.get(key) as object | undefined) });
    else if (!values.has(key)) set(key, value);
  }
}
