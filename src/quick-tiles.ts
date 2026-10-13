/** Shortcut tiles the quick settings panel can show: id -> [label, glyph, what it opens]. */
export const QUICK_TILES: Record<string, [label: string, glyph: number, target: string]> = {
  wifi: ["Wi-Fi", 0xe701, "ms-settings:network-wifi"],
  bluetooth: ["Bluetooth", 0xe702, "ms-settings:bluetooth"],
  display: ["Display", 0xe7f4, "ms-settings:display"],
  sound: ["Sound", 0xe767, "ms-settings:sound"],
  power: ["Power", 0xe7e8, "ms-settings:powersleep"],
  settings: ["Settings", 0xe713, "ms-settings:"],
  nightlight: ["Night light", 0xe708, "ms-settings:nightlight"],
  network: ["Network", 0xe774, "ms-settings:network"],
  personalization: ["Personalize", 0xe771, "ms-settings:personalization"],
  notifications: ["Notifications", 0xea8f, "ms-settings:notifications"],
  update: ["Update", 0xe895, "ms-settings:windowsupdate"],
  storage: ["Storage", 0xeda2, "ms-settings:storagesense"],
  apps: ["Apps", 0xe71d, "ms-settings:appsfeatures"],
  taskmgr: ["Task Manager", 0xe9d9, "taskmgr.exe"],
  explorer: ["Explorer", 0xe838, "explorer.exe"],
  // Kasumi's own settings (glyph 0: its logo).
  kasumi: ["Kasumi", 0, "kasumi:settings"],
};

/** Kasumi's logo (app-icon.svg) in the text colour, glyph-sized: a rounded square, the notch cut out. */
export function kasumiLogo(): HTMLSpanElement {
  const s = document.createElement("span");
  s.className = "icon kasumi-logo";
  s.innerHTML =
    '<svg viewBox="0 0 512 512" width="1em" height="1em" aria-hidden="true"><mask id="kasumi-notch"><rect width="512" height="512" fill="#fff"/>' +
    '<path d="M96 0 C122 0 136 14 136 40 L136 96 C136 124 152 140 180 140 L332 140 C360 140 376 124 376 96 L376 40 C376 14 390 0 416 0 Z" fill="#000"/></mask>' +
    '<rect width="512" height="512" rx="112" fill="currentColor" mask="url(#kasumi-notch)"/></svg>';
  return s;
}

/** At most this many tiles (one row). */
export const MAX_TILES = 6;

/** The tiles shown when none are chosen. */
export const DEFAULT_TILES = ["wifi", "bluetooth", "display", "sound", "power", "settings"];
