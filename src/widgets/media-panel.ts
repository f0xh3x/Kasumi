import { invoke } from "@tauri-apps/api/core";
import { subscribe, type Media } from "../data";
import { g, el, Glyph, icon, type PanelContext, type Widget } from "./index";

const fmt = (ms: number) => {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
};

/**
 * Now-playing panel: album art, title / artist, a seekable progress bar and
 * the transport keys. Position is extrapolated between the player's updates.
 */
export function mediaPanel(_opts: Record<string, unknown>, _ctx: PanelContext): Widget {
  const art = el("media-art");
  const title = el("media-title");
  const artist = el("media-artist");

  const fill = el("seek-fill");
  const knob = el("seek-knob");
  const track = el("seek-track", fill, knob);
  const hit = el("seek-hit", track);
  const elapsed = el("seek-time");
  const total = el("seek-time");
  const seek = el("media-seek", elapsed, hit, total);

  const key = (glyph: string, action: string) => {
    const b = el("media-btn big", icon(glyph));
    b.addEventListener("click", () => {
      if (!b.classList.contains("disabled")) invoke("media_control", { action }).catch(console.error);
    });
    return b;
  };
  const prev = key(Glyph.prev, "previous");
  const playPause = key(Glyph.play, "play_pause");
  const next = key(Glyph.next, "next");
  const keys = el("media-keys", prev, playPause, next);

  const info = el("media-info", title, artist, seek, keys);
  const root = el("media-panel", art, info);

  let media: Media | null = null;
  let dragging: number | null = null; // fraction while the user drags

  const position = () => {
    if (!media) return 0;
    const drift = media.playing ? Date.now() - media.updatedAt : 0;
    return Math.min(media.durationMs, media.positionMs + Math.max(0, drift));
  };

  const paint = () => {
    if (!media) return;
    const dur = media.durationMs;
    const frac = dragging ?? (dur > 0 ? position() / dur : 0);
    seek.style.setProperty("--v", String(frac));
    elapsed.textContent = fmt(frac * dur);
    total.textContent = dur > 0 ? fmt(dur) : "";
  };

  const fraction = (e: PointerEvent) => {
    const r = track.getBoundingClientRect();
    return Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
  };
  hit.addEventListener("pointerdown", (e) => {
    if (!media?.canSeek) return;
    hit.setPointerCapture(e.pointerId);
    dragging = fraction(e);
    seek.classList.add("dragging");
    paint();
  });
  hit.addEventListener("pointermove", (e) => {
    if (dragging === null) return;
    dragging = fraction(e);
    paint();
  });
  const finish = (e: PointerEvent) => {
    if (dragging === null || !media) return;
    const target = fraction(e) * media.durationMs;
    // Show the new position right away; the player confirms on its next update.
    media = { ...media, positionMs: target, updatedAt: Date.now() };
    dragging = null;
    seek.classList.remove("dragging");
    paint();
    invoke("media_control", { action: "seek", positionMs: target }).catch(console.error);
  };
  hit.addEventListener("pointerup", finish);
  hit.addEventListener("pointercancel", () => {
    dragging = null;
    seek.classList.remove("dragging");
    paint();
  });

  const unsubscribe = subscribe("media", (m) => {
    media = m;
    if (!m) {
      title.textContent = "Nothing playing";
      artist.textContent = "";
      art.replaceChildren(icon(g(0xe8d6)));
      art.style.backgroundImage = "";
      seek.hidden = true;
      return;
    }
    title.textContent = m.title;
    title.title = m.title;
    artist.textContent = m.artist;
    if (m.art) {
      art.replaceChildren();
      art.style.backgroundImage = `url("${m.art}")`;
    } else {
      art.style.backgroundImage = "";
      art.replaceChildren(icon(g(0xe8d6))); // music note
    }
    seek.hidden = m.durationMs <= 0;
    seek.classList.toggle("disabled", !m.canSeek);
    playPause.firstElementChild!.textContent = m.playing ? Glyph.pause : Glyph.play;
    prev.classList.toggle("disabled", !m.canPrevious);
    next.classList.toggle("disabled", !m.canNext);
    playPause.classList.toggle("disabled", !m.canPlayPause);
    paint();
  });
  // The position moves between the player's updates; nobody sees it while the panel is shut.
  const timer = window.setInterval(() => {
    const panel = root.closest<HTMLElement>(".panel");
    if (media?.playing && dragging === null && panel && panel.style.visibility !== "hidden") paint();
  }, 500);

  return {
    el: root,
    dispose: () => {
      unsubscribe();
      clearInterval(timer);
    },
  };
}
