import { invoke } from "@tauri-apps/api/core";
import { subscribe } from "../data";
import { el, Glyph, icon, type Widget } from "./index";

/**
 * Now playing (Windows media session): previous / play-pause / next, then
 * "title — artist". Hidden when nothing is playing. `max_width` (px) caps the text;
 * with `scroll`, text that doesn't fit scrolls instead of ending in "…".
 */
/** Title scrolling speed, px per second. */
const SCROLL_SPEED = 32;

export function media(opts: Record<string, unknown>): Widget {
  const maxWidth = typeof opts.max_width === "number" ? opts.max_width : 240;
  const scroll = opts.scroll === true;

  const button = (glyph: string, action: string) => {
    const b = el("media-btn", icon(glyph));
    b.addEventListener("click", (e) => {
      e.stopPropagation();
      if (b.classList.contains("disabled")) return;
      invoke("media_control", { action }).catch((err) => console.error("media_control", err));
    });
    return b;
  };
  const prev = button(Glyph.prev, "previous");
  const playPause = button(Glyph.play, "play_pause");
  const next = button(Glyph.next, "next");
  // The label, plus a second copy that follows it round while scrolling.
  const label = document.createElement("span");
  const echo = document.createElement("span");
  echo.className = "media-echo";
  const track = document.createElement("span");
  track.className = "media-track";
  track.append(label, echo);
  const text = el("media-text", track);
  // Where a notch copy glides to/from (see Overlay.glide).
  text.dataset.glide = "";
  text.style.maxWidth = `${maxWidth}px`;
  // Fixed: always the full title width, so the notch keeps one size whatever plays.
  if (opts.fixed_width === true) text.style.width = `${maxWidth}px`;
  const root = el("widget media", el("media-controls", prev, playPause, next), text);
  // Playback keys beside the title ([widgets.media] keys): off | always | open
  // (only while the bar is open) | closed (only while it's closed). The media panel
  // always has them. In a closed notch they can't be clicked (click-through).
  const keys = opts.keys === true ? "always" : typeof opts.keys === "string" ? opts.keys : "off";
  if (keys === "off") root.classList.add("compact");
  else if (keys === "open" || keys === "closed") root.classList.add(`keys-${keys}`);
  if (opts.fixed_width === true) root.classList.add("fixed");
  root.hidden = true;

  let marquee: Animation | null = null;
  const fit = () => {
    marquee?.cancel();
    marquee = null;
    const over = scroll && label.offsetWidth > text.clientWidth + 1;
    text.classList.toggle("scrolling", over);
    if (!over) return;
    const distance = label.offsetWidth + echo.offsetLeft - label.offsetLeft - label.offsetWidth;
    const hold = 1800;
    const total = hold + (distance / SCROLL_SPEED) * 1000;
    marquee = track.animate(
      [
        { transform: "translateX(0)" },
        { transform: "translateX(0)", offset: hold / total },
        { transform: `translateX(${-distance}px)` },
      ],
      { duration: total, iterations: Infinity },
    );
    // Pinned to the document's clock: the notch's and the bar's copies (same text,
    // same timing) scroll in step, so the hand-over between them stays invisible.
    marquee.startTime = 0;
  };
  const resized = new ResizeObserver(fit);
  if (scroll) resized.observe(text);

  const unsubscribe = subscribe("media", (m) => {
    root.hidden = !m;
    if (!m) return;
    const line = m.artist ? `${m.title} — ${m.artist}` : m.title;
    if (line !== label.textContent) {
      label.textContent = echo.textContent = line;
      if (scroll) fit();
    }
    root.title = m.artist ? `${m.title}\n${m.artist}` : m.title;
    playPause.firstElementChild!.textContent = m.playing ? Glyph.pause : Glyph.play;
    prev.classList.toggle("disabled", !m.canPrevious);
    next.classList.toggle("disabled", !m.canNext);
    playPause.classList.toggle("disabled", !m.canPlayPause);
  });
  return {
    el: root,
    dispose: () => {
      unsubscribe();
      resized.disconnect();
      marquee?.cancel();
    },
  };
}
