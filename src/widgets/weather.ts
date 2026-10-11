import cloudy from "@fluentui/svg-icons/icons/weather_cloudy_20_regular.svg?raw";
import drizzle from "@fluentui/svg-icons/icons/weather_drizzle_20_regular.svg?raw";
import fog from "@fluentui/svg-icons/icons/weather_fog_20_regular.svg?raw";
import partlyDay from "@fluentui/svg-icons/icons/weather_partly_cloudy_day_20_regular.svg?raw";
import partlyNight from "@fluentui/svg-icons/icons/weather_partly_cloudy_night_20_regular.svg?raw";
import rain from "@fluentui/svg-icons/icons/weather_rain_20_regular.svg?raw";
import rainShowersDay from "@fluentui/svg-icons/icons/weather_rain_showers_day_20_regular.svg?raw";
import rainShowersNight from "@fluentui/svg-icons/icons/weather_rain_showers_night_20_regular.svg?raw";
import rainSnow from "@fluentui/svg-icons/icons/weather_rain_snow_20_regular.svg?raw";
import snow from "@fluentui/svg-icons/icons/weather_snow_20_regular.svg?raw";
import snowShowerDay from "@fluentui/svg-icons/icons/weather_snow_shower_day_20_regular.svg?raw";
import snowShowerNight from "@fluentui/svg-icons/icons/weather_snow_shower_night_20_regular.svg?raw";
import thunderstorm from "@fluentui/svg-icons/icons/weather_thunderstorm_20_regular.svg?raw";
import { subscribe, type Weather } from "../data";
import { el, Glyph, icon, setText, type Widget } from "./index";

type Visual = { glyph: string } | { svg: string };

/**
 * WMO weather code → icon. Clear sky uses the YASB glyphs (Segoe Fluent);
 * Segoe Fluent has no other weather glyphs, so the rest are Fluent UI System
 * Icons, drawn in the same style.
 */
function visual(code: number, day: boolean): Visual {
  if (code === 0) return { glyph: day ? Glyph.sunny : Glyph.clearNight };
  if (code === 1 || code === 2) return { svg: day ? partlyDay : partlyNight };
  if (code === 3) return { svg: cloudy };
  if (code === 45 || code === 48) return { svg: fog };
  if (code >= 51 && code <= 57) return { svg: drizzle };
  if (code === 66 || code === 67) return { svg: rainSnow };
  if (code >= 61 && code <= 65) return { svg: rain };
  if (code >= 80 && code <= 82) return { svg: day ? rainShowersDay : rainShowersNight };
  if (code >= 71 && code <= 77) return { svg: snow };
  if (code === 85 || code === 86) return { svg: day ? snowShowerDay : snowShowerNight };
  if (code >= 95) return { svg: thunderstorm };
  return { svg: cloudy };
}

const DESCRIPTIONS: [number[], string][] = [
  [[0], "Clear"],
  [[1], "Mostly clear"],
  [[2], "Partly cloudy"],
  [[3], "Overcast"],
  [[45, 48], "Fog"],
  [[51, 53, 55, 56, 57], "Drizzle"],
  [[61, 63, 65], "Rain"],
  [[66, 67], "Freezing rain"],
  [[71, 73, 75, 77], "Snow"],
  [[80, 81, 82], "Rain showers"],
  [[85, 86], "Snow showers"],
  [[95, 96, 99], "Thunderstorm"],
];

/** Condition icon element (Segoe glyph or Fluent SVG). */
export function weatherIcon(code: number, day: boolean, size = 16): HTMLElement {
  const v = visual(code, day);
  if ("glyph" in v) {
    const i = icon(v.glyph);
    i.style.fontSize = `${size}px`;
    return i;
  }
  const s = document.createElement("span");
  s.className = "svg-icon";
  s.innerHTML = v.svg;
  const svg = s.querySelector("svg");
  if (svg) {
    svg.style.width = `${size}px`;
    svg.style.height = `${size}px`;
  }
  return s;
}

const describe = (code: number) => DESCRIPTIONS.find(([codes]) => codes.includes(code))?.[1] ?? "";
const deg = (v: number) => `${Math.round(v)}°`;

/** Current temperature + condition icon from Open-Meteo; details in the tooltip. */
export function weather(opts: Record<string, unknown> = {}): Widget {
  const iconSlot = el("weather-icon");
  const temp = document.createElement("span");
  const root = el("widget weather", iconSlot, temp);
  root.hidden = true;

  const render = (w: Weather) => {
    iconSlot.replaceChildren(weatherIcon(w.code, w.isDay));
    setText(temp, deg(w.temp), opts._roll === true);
    root.title = [
      w.location && `${w.location}: ${describe(w.code)}`.trim(),
      `Feels like ${deg(w.feels)}`,
      `${deg(w.min)} / ${deg(w.max)}`,
    ]
      .filter(Boolean)
      .join("\n");
  };

  const unsubscribe = subscribe("weather", (w) => {
    root.hidden = !w;
    if (w) render(w);
  });
  return { el: root, dispose: unsubscribe };
}

/**
 * Weather panel: current conditions and the next hours (`hours`, default 8).
 */
export function weatherPanel(opts: Record<string, unknown>): Widget {
  const hours = typeof opts.hours === "number" ? opts.hours : 8;
  const nowIcon = el("wx-now-icon");
  const nowTemp = el("wx-now-temp");
  const cond = el("wx-cond");
  const feels = el("wx-detail");
  const range = el("wx-detail");
  const place = el("wx-place");
  const head = el("wx-head", nowIcon, nowTemp, el("wx-meta", cond, feels, range), place);
  const strip = el("wx-hours");
  const root = el("weather-panel", head, strip);

  const unsubscribe = subscribe("weather", (w) => {
    if (!w) {
      cond.textContent = "No weather data";
      return;
    }
    nowIcon.replaceChildren(weatherIcon(w.code, w.isDay, 26));
    nowTemp.textContent = deg(w.temp);
    cond.textContent = describe(w.code);
    feels.textContent = `Feels like ${deg(w.feels)}`;
    range.textContent = `${deg(w.min)} / ${deg(w.max)}`;
    place.textContent = w.location;
    strip.replaceChildren(
      ...(w.hourly ?? []).slice(0, hours + 1).map((h, i) =>
        el(
          "wx-hour",
          el("wx-hour-time", i === 0 ? "Now" : h.time.slice(11, 13)),
          weatherIcon(h.code, h.isDay, 16),
          el("wx-hour-temp", deg(h.temp)),
        ),
      ),
    );
  });
  return { el: root, dispose: unsubscribe };
}
