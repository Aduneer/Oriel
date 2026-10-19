"use strict";
{
const { register, el, refs, setHTML, setText, esc } = Oriel;
const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);

// ---- weather codes -------------------------------------------------------------
// WMO codes, as Open-Meteo sends them. Icons are drawn here, so nothing is loaded from anywhere.
const svg = (inner, cls = "") => `<svg class="wi ${cls}" viewBox="0 0 24 24" aria-hidden="true">${inner}</svg>`;
const SUN = `<g class="sun"><circle cx="12" cy="12" r="4"/><path d="M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6 7 7M17 17l1.4 1.4M5.6 18.4 7 17M17 7l1.4-1.4"/></g>`;
const MOON = `<path class="sun" d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5Z"/>`;
const CLOUD = `<path d="M7.5 18a4.5 4.5 0 0 1-.6-8.96A6 6 0 0 1 18.4 10.3 3.9 3.9 0 0 1 17.5 18Z"/>`;
const high = inner => `<g transform="translate(0 -3.5)">${inner}</g>`;
// A small sun or moon behind a cloud
const SUN_BEHIND = `<g class="sun"><circle cx="8.5" cy="8.5" r="2.8"/><path d="M8.5 2.6v1.3M2.6 8.5h1.3M4.3 4.3l.9.9M12.7 4.3l-.9.9"/></g>`;
const MOON_BEHIND = `<path class="sun" d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5Z" transform="translate(1.5 1.5) scale(.5)" stroke-width="3.4"/>`;
// In front of the sun or moon, and hiding what's behind it
const CLOUD_FRONT = `<g class="occl" transform="translate(3.6 4) scale(.78)" stroke-width="2.2">${CLOUD}</g>`;
const DRIZZLE = `<path d="M8 19v1.2M12 19.8V21M16 19v1.2"/>`;
const RAIN = `<path d="M8 17.5 7 20.5M12.5 17.5l-1 3M17 17.5l-1 3"/>`;
const SNOW = `<g fill="currentColor" stroke="none"><circle cx="8" cy="18.6" r="1"/><circle cx="12" cy="20.6" r="1"/><circle cx="16" cy="18.6" r="1"/></g>`;
const BOLT = `<path d="m12.8 14.3-2.6 3.6h3.2l-1.6 3.7"/>`;
const FOG = `<path d="M5 8.5h14M3 12.5h18M6 16.5h12M9 20.5h6"/>`;

const WX = {
  clear:   (day) => svg(day ? SUN : MOON),
  partly:  (day) => svg((day ? SUN_BEHIND : MOON_BEHIND) + CLOUD_FRONT),
  cloud:   () => svg(CLOUD),
  fog:     () => svg(FOG),
  drizzle: () => svg(high(CLOUD) + DRIZZLE),
  rain:    () => svg(high(CLOUD) + RAIN),
  snow:    () => svg(high(CLOUD) + SNOW),
  storm:   () => svg(high(CLOUD) + BOLT),
};
// code: [text, icon]
const CODES = {
  0: ["Clear", "clear"], 1: ["Mostly clear", "clear"], 2: ["Partly cloudy", "partly"], 3: ["Overcast", "cloud"],
  45: ["Fog", "fog"], 48: ["Freezing fog", "fog"],
  51: ["Light drizzle", "drizzle"], 53: ["Drizzle", "drizzle"], 55: ["Heavy drizzle", "drizzle"],
  56: ["Freezing drizzle", "drizzle"], 57: ["Freezing drizzle", "drizzle"],
  61: ["Light rain", "rain"], 63: ["Rain", "rain"], 65: ["Heavy rain", "rain"],
  66: ["Freezing rain", "rain"], 67: ["Freezing rain", "rain"],
  71: ["Light snow", "snow"], 73: ["Snow", "snow"], 75: ["Heavy snow", "snow"], 77: ["Snow grains", "snow"],
  80: ["Light showers", "rain"], 81: ["Showers", "rain"], 82: ["Heavy showers", "rain"],
  85: ["Snow showers", "snow"], 86: ["Heavy snow showers", "snow"],
  95: ["Thunderstorm", "storm"], 96: ["Thunderstorm, hail", "storm"], 99: ["Thunderstorm, hail", "storm"],
};
const word = code => (CODES[code] ?? ["Unknown", "cloud"])[0];
const icon = (code, day = true) => WX[(CODES[code] ?? [0, "cloud"])[1]](day);

const REFRESH = `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20 12a8 8 0 1 1-2.6-5.9M20 4v4.5h-4.5"/></svg>`;

// ---- units and times -----------------------------------------------------------
// °F where people use it: the United States and a few others, by the language's region.
const FAHRENHEIT = new Set(["US", "BS", "BZ", "KY", "PW", "LR", "FM", "MH", "GU", "PR", "VI", "MP", "AS"]);
function regionUsesF() {
  try {
    const loc = new Intl.Locale(navigator.language);
    return FAHRENHEIT.has(loc.region ?? loc.maximize().region);
  } catch { return false; }
}
const useF = () => {
  const u = Oriel.state.settings.weather_units;
  return u === "f" || (u !== "c" && regionUsesF());
};
/** A temperature in °C as the number to show. */
const deg = (c, f) => Math.round(f ? (c * 9) / 5 + 32 : c);

/** "2026-10-19T14:00" as a Date whose local fields are those numbers (the place's wall clock, not this PC's). */
const wall = iso => {
  const [d, t = "00:00"] = iso.split("T");
  const [y, m, day] = d.split("-").map(Number);
  const [h, min] = t.split(":").map(Number);
  return new Date(y, m - 1, day, h, min || 0);
};
const fmtHour = d => d.toLocaleTimeString([], { hour: "numeric" });
const fmtClock = d => d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
const fmtDay = d => d.toLocaleDateString([], { weekday: "short" });

register("weather", {
  name: "Weather",
  description: "The forecast for a place you choose, from Open-Meteo: now, the next hours and the next days",
  color: "accent",
  size: { w: 1.2 },
  create() {
    const t = refs(el(`
      <article class="tile weather" style="--hue:var(--accent)">
        <header><h2>Weather</h2><span class="sub" data-k="place"></span>
          <span class="chip warn" data-k="stale" hidden>Out of date</span>
          <button class="go" type="button" data-k="refresh" title="Refresh" aria-label="Refresh the forecast">${REFRESH}</button></header>
        <div class="wnow" data-k="now">
          <div class="stat"><span class="big" data-k="temp">–</span><span class="wbig" data-k="icon"></span></div>
          <div class="wtext"><div class="wcond" data-k="cond"></div><div class="whl" data-k="hl"></div></div>
          <div class="facts" data-k="facts"></div>
        </div>
        <div class="wstrip hours" data-k="hours" aria-label="Next hours"></div>
        <div class="wstrip days" data-k="days" aria-label="Next days"></div>
        <p class="wfoot" data-k="foot"></p>
        <div class="empty" data-k="msg" hidden></div>
      </article>`));

    let rev = null;     // which version of the data we have (see `feeds` in collect.rs)
    let data = null;    // the forecast, or null
    let st = null;      // what the last sample said about it: {rev, at, error, busy}
    let place = null;   // the place chosen in Settings
    let drawn = "";

    // A tile with room for less shows less: the days go first, then the hours.
    new ResizeObserver(() => {
      const rem = t.root.clientHeight / Oriel.fontPx;
      t.root.classList.toggle("tall", rem >= 37);
      t.root.classList.toggle("short", rem < 28);
      t.root.classList.toggle("tiny", rem < 18);
    }).observe(t.root);

    t.refresh.addEventListener("click", () => invoke("refresh_feed", { name: "weather" }).catch(e => console.warn(e)));
    t.root.addEventListener("click", e => {
      if (e.target.closest("[data-retry]")) invoke("refresh_feed", { name: "weather" }).catch(err => console.warn(err));
    });

    function message(html) {
      t.now.hidden = t.hours.hidden = t.days.hidden = true;
      t.foot.hidden = true;
      t.msg.hidden = false;
      setHTML(t.msg, html);
    }

    function draw() {
      const f = useF();
      const unit = f ? "F" : "C";
      setText(t.place, place.name);
      t.stale.hidden = !(st.error && data);
      t.stale.title = st.error ? `Couldn't update: ${st.error}` : "";
      t.refresh.classList.toggle("on", !!st.error);

      if (!data) {
        t.refresh.hidden = false;
        return message(st.error
          ? `<p>Couldn't load the forecast<br><small>${esc(st.error)}</small></p><button type="button" data-retry>Try again</button>`
          : `<p>Loading the forecast…</p>`);
      }
      t.msg.hidden = true;
      t.now.hidden = t.hours.hidden = t.days.hidden = t.foot.hidden = false;

      // The data is as of when it was fetched, in the place's own time; move it along by the time since.
      const here = new Date(wall(data.now.time).getTime() + Math.max(0, Date.now() - st.at * 1000));
      here.setMinutes(0, 0, 0);
      const hourStart = here.getTime();
      const now = data.now;
      const today = data.days[0];
      setHTML(t.temp, `${deg(now.temp, f)}<small>°${unit}</small>`);
      setHTML(t.icon, icon(now.code, now.day));
      setText(t.cond, word(now.code));
      setText(t.hl, today ? `High ${deg(today.hi, f)}° · Low ${deg(today.lo, f)}°` : "");
      const wind = now.wind == null ? "" : f ? `${Math.round(now.wind * 0.6214)} mph` : `${Math.round(now.wind)} km/h`;
      setHTML(t.facts, Oriel.facts([
        now.feels != null && Math.abs(now.feels - now.temp) >= 1 ? `Feels ${deg(now.feels, f)}°` : "",
        now.humidity != null ? `Humidity ${Math.round(now.humidity)}%` : "",
        wind ? `Wind ${wind}` : "",
        today?.sunrise ? `Sunrise ${fmtClock(wall(`2000-01-01T${today.sunrise}`))}` : "",
        today?.sunset ? `Sunset ${fmtClock(wall(`2000-01-01T${today.sunset}`))}` : "",
      ].map(x => x && esc(x))));

      const rain = r => (r != null && r >= 20 ? `${r}%` : "");
      const hours = data.hours.filter(h => wall(h.time).getTime() >= hourStart);
      setHTML(t.hours, hours.map(h => `
        <div class="c"><span class="l">${esc(fmtHour(wall(h.time)))}</span>${icon(h.code, h.day)}<b>${deg(h.temp, f)}°</b><small>${rain(h.rain)}</small></div>`).join(""));
      setHTML(t.days, data.days.map((d, i) => `
        <div class="c"><span class="l">${i ? esc(fmtDay(wall(d.date))) : "Today"}</span>${icon(d.code)}<b>${deg(d.hi, f)}°</b><span class="lo">${deg(d.lo, f)}°</span><small>${rain(d.rain)}</small></div>`).join(""));

      setText(t.foot, st.error
        ? `Couldn't update (${st.error}). Showing the forecast from ${fmtClock(new Date(st.at * 1000))}.`
        : st.at ? `Updated ${fmtClock(new Date(st.at * 1000))} · Open-Meteo` : "");
      t.foot.classList.toggle("bad", !!st.error);
    }

    return {
      root: t.root,
      update(s) {
        if (!s.procs) return; // history samples leave out the feeds
        place = Oriel.state.settings.weather_place;
        st = s.feeds?.weather ?? null;
        if (!place) {
          rev = data = null;
          drawn = "";
          t.stale.hidden = true;
          t.refresh.hidden = true;
          setText(t.place, "");
          return message(`<p>Choose a place to see its weather.</p><button type="button" data-open-settings>Open settings</button>`);
        }
        t.refresh.hidden = false;
        if (!st) { // the backend hasn't started on it yet
          setText(t.place, place.name);
          return message(`<p>Loading the forecast…</p>`);
        }
        if (st.rev !== rev) {
          const mine = (rev = st.rev);
          invoke("weather").then(f => {
            if (rev !== mine) return; // a newer one is on its way
            data = f;
            drawn = "";
            if (st && place) draw();
          }, e => console.warn(e));
        }
        // Again each minute, as the hours move on; otherwise only when something changed.
        const key = [st.rev, st.error, st.at, useF(), place.name, !!data, Math.floor(Date.now() / 60000)].join("|");
        if (key === drawn) return;
        drawn = key;
        draw();
      },
    };
  },
});
}
