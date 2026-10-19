"use strict";
// An example Oriel plugin: the time in a few cities.
// Copy world-clock.js and world-clock.css into %APPDATA%\Oriel\widgets, turn on
// "Load widgets from the plugins folder" in Settings, restart Oriel, then add
// "World clock" in Settings → Layout.
{
const { register, el, refs, setText, esc } = Oriel;

// Change these to your own. Names are IANA time zones: https://en.wikipedia.org/wiki/List_of_tz_database_time_zones
const CITIES = [
  ["London", "Europe/London"],
  ["New York", "America/New_York"],
  ["Tokyo", "Asia/Tokyo"],
];

register("world-clock", {
  name: "World clock",
  description: "The time in a few cities",
  color: "s2",          // a theme color for the dot in the layout editor
  size: { w: 1, h: 1 }, // width within its row, and the row's relative height
  create() {
    const t = refs(el(`
      <article class="tile world-clock" style="--hue:var(--s2)">
        <header><h2>World clock</h2></header>
        <div class="cities">
          ${CITIES.map(([name], i) => `<div><span class="label">${esc(name)}</span><span class="mid" data-k="c${i}">–</span></div>`).join("")}
        </div>
      </article>`));
    const formats = CITIES.map(([, zone]) => new Intl.DateTimeFormat([], { timeZone: zone, hour: "2-digit", minute: "2-digit" }));
    return {
      root: t.root,
      // Called with each new sample, about once a second. The time comes from the clock, not the sample.
      update() {
        const now = new Date();
        formats.forEach((f, i) => setText(t[`c${i}`], f.format(now)));
      },
    };
  },
});
}
