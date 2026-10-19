"use strict";
{
const { register, el, refs, setHTML, setText, esc } = Oriel;
const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);

const REFRESH = `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20 12a8 8 0 1 1-2.6-5.9M20 4v4.5h-4.5"/></svg>`;
// Colors for calendars that weren't given one, by position: theme tokens, so they suit every theme.
const DEFAULTS = ["s1", "s2", "gpu", "mem", "cpu"];

// ---- dates ---------------------------------------------------------------------
// A day is a whole number: days since 1970-01-01 of the date on this PC's calendar. All-day events
// come with their days like that; timed ones are turned into the day they start on.
const dayOf = ms => {
  const d = new Date(ms);
  return Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()) / 864e5;
};
const dayName = n => new Date(n * 864e5).toLocaleDateString([], { weekday: "short", day: "numeric", month: "short", timeZone: "UTC" });
const weekdayName = n => new Date(n * 864e5).toLocaleDateString([], { weekday: "long", timeZone: "UTC" });
const clock = ms => new Date(ms).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });

/** "in 25 min", "in 2 h": how long until `ms`, for the next event. */
function until(ms, now) {
  const m = Math.max(1, Math.round((ms - now) / 6e4));
  if (m < 60) return `in ${m} min`;
  const h = Math.round(m / 60);
  return h < 12 ? `in ${h} h` : "";
}

register("calendar", {
  name: "Calendar",
  description: "Upcoming events from the calendar links (.ics) you add in Settings: Google, Outlook, Apple and others",
  color: "s2",
  size: { w: 1.1 },
  create() {
    const t = refs(el(`
      <article class="tile cal" style="--hue:var(--s2)">
        <header><h2>Calendar</h2><span class="sub" data-k="sub"></span>
          <span class="chip warn" data-k="stale" hidden>Out of date</span>
          <button class="go" type="button" data-k="refresh" title="Refresh" aria-label="Refresh the calendar">${REFRESH}</button></header>
        <div class="agenda" data-k="list"></div>
        <p class="wfoot" data-k="foot"></p>
        <div class="empty" data-k="msg" hidden></div>
      </article>`));

    let rev = null;     // which version of the data we have (see `feeds` in collect.rs)
    let data = null;    // {feeds: [{name, color, error}], events: [...]}, or null
    let st = null;      // what the last sample said about it: {rev, at, error, busy}
    let drawn = "";

    t.refresh.addEventListener("click", () => invoke("refresh_feed", { name: "calendar" }).catch(e => console.warn(e)));
    t.root.addEventListener("click", e => {
      if (e.target.closest("[data-retry]")) invoke("refresh_feed", { name: "calendar" }).catch(err => console.warn(err));
    });

    function message(html) {
      t.list.hidden = t.foot.hidden = true;
      t.msg.hidden = false;
      setHTML(t.msg, html);
    }

    // Names and colors are the settings': the data has one entry per calendar that has a link, in the same order.
    const linked = () => Oriel.state.settings.calendars.filter(c => c.url);
    const feedName = (cals, i) => cals[i]?.name || `Calendar ${i + 1}`;
    const colorOf = (cals, i) => `var(--${cals[i]?.color || DEFAULTS[i % DEFAULTS.length]})`;

    function draw() {
      const days = lookAhead();
      const cals = linked();
      const feeds = data?.feeds ?? [];
      const broken = feeds.map((f, i) => (f.error ? `${feedName(cals, i)}: ${f.error}` : "")).filter(Boolean);
      t.stale.hidden = !(broken.length && data);
      t.stale.title = broken.length ? `Couldn't update: ${broken.join(". ")}` : "";
      setText(t.sub, `Next ${days} days`);

      if (!data) {
        return message(st?.error
          ? `<p>Couldn't load the calendar<br><small>${esc(st.error)}</small></p><button type="button" data-retry>Try again</button>`
          : `<p>Loading the calendar…</p>`);
      }
      t.msg.hidden = true;
      t.list.hidden = t.foot.hidden = false;

      const now = Date.now();
      const today = dayOf(now);
      const last = today + days - 1;
      const groups = new Map(); // day -> {allDay: [], timed: []}
      for (let d = today; d <= last; d++) groups.set(d, { allDay: [], timed: [] });
      for (const e of data.events) {
        if (e.all_day) {
          // On every day it covers (`end` is the day after the last).
          for (let d = Math.max(e.start, today); d < Math.min(e.end, last + 1); d++) {
            groups.get(d).allDay.push({ e, nth: d - e.start + 1, of: e.end - e.start });
          }
          continue;
        }
        const [start, end] = [e.start * 1000, e.end * 1000];
        // What's over is no longer coming up. A moment with no length lingers for ten minutes.
        if (end > start ? end <= now : start + 6e5 <= now) continue;
        const d = Math.max(dayOf(start), today); // one that began yesterday and is still going counts as today's
        if (d <= last) groups.get(d).timed.push({ e, start, end });
      }
      for (const g of groups.values()) g.timed.sort((a, b) => a.start - b.start);

      // The one on now, and the next one to come.
      let next = null;
      for (const g of groups.values()) {
        next = g.timed.find(x => x.start > now);
        if (next) break;
      }

      const html = [];
      const any = [...groups.values()].some(g => g.allDay.length || g.timed.length);
      for (const [d, g] of any ? groups : []) {
        const busy = g.allDay.length || g.timed.length;
        if (!busy && d !== today) continue;
        const name = d === today ? "Today" : d === today + 1 ? "Tomorrow" : weekdayName(d);
        const rest = d === today || d === today + 1 ? dayName(d) : dayName(d).replace(/^[^ ]+ ?/, "");
        html.push(`<section class="day"><h3>${esc(name)}<small>${esc(rest)}</small></h3>`);
        if (g.allDay.length) {
          html.push(`<div class="ads">${g.allDay.map(({ e, nth, of }) => {
            const days = of > 1 ? ` · day ${nth} of ${of}` : "";
            return `<span class="ad" style="--c:${colorOf(cals, e.cal)}" title="${esc(feedName(cals, e.cal))}"><i></i>${esc(e.title || "(No title)")}${days}</span>`;
          }).join("")}</div>`);
        }
        for (const x of g.timed) {
          const { e } = x;
          const ongoing = x.start <= now && x.end > now;
          const cls = ongoing ? " now" : x === next ? " next" : "";
          const tag = ongoing ? `<span class="chip">Now</span>` : x === next && until(x.start, now) ? `<span class="rel">${until(x.start, now)}</span>` : "";
          const sameDay = dayOf(x.end) === dayOf(x.start);
          const endText = x.end > x.start ? `–${sameDay ? clock(x.end) : `${new Date(x.end).toLocaleDateString([], { weekday: "short" })} ${clock(x.end)}`}` : "";
          const startText = x.start <= now && dayOf(x.start) < d ? "Since " + new Date(x.start).toLocaleDateString([], { weekday: "short" }) : clock(x.start);
          html.push(`
            <div class="ev${cls}" style="--c:${colorOf(cals, e.cal)}" title="${esc(feedName(cals, e.cal))}">
              <span class="tm"><b>${esc(startText)}</b><small>${esc(endText)}</small></span>
              <span class="tx"><span class="ti">${esc(e.title || "(No title)")}</span>${e.location ? `<span class="lo">${esc(e.location)}</span>` : ""}</span>
              ${tag}
            </div>`);
        }
        if (!busy) html.push(`<p class="none">Nothing on today</p>`);
        html.push(`</section>`);
      }
      if (!any) html.push(`<p class="none wide">Nothing in the next ${days} days</p>`);

      // Scrolling is kept across the refreshes that happen every minute.
      const top = t.list.scrollTop;
      setHTML(t.list, html.join(""));
      t.list.scrollTop = top;

      const legend = feeds.length > 1 ? feeds.map((_, i) => `<span class="lg" style="--c:${colorOf(cals, i)}"><i></i>${esc(feedName(cals, i))}</span>`).join("") : "";
      const bad = broken.length ? `<span class="bad">Couldn't update ${esc(broken.join(" · "))}</span>` : "";
      setHTML(t.foot, legend + bad);
      t.foot.hidden = !(legend || bad);
    }

    const lookAhead = () => Oriel.state.settings.calendar_days || 7;
    const hasLinks = () => linked().length > 0;

    return {
      root: t.root,
      update(s) {
        if (!s.procs) return; // history samples leave out the feeds
        st = s.feeds?.calendar ?? null;
        if (!hasLinks()) {
          rev = data = null;
          drawn = "";
          t.stale.hidden = true;
          t.refresh.hidden = true;
          setText(t.sub, "");
          return message(`<p>Add a calendar link to see what's coming up.</p><button type="button" data-open-settings>Open settings</button>`);
        }
        t.refresh.hidden = false;
        if (!st) { // the backend hasn't started on it yet
          setText(t.sub, `Next ${lookAhead()} days`);
          return message(`<p>Loading the calendar…</p>`);
        }
        if (st.rev !== rev) {
          const mine = (rev = st.rev);
          invoke("calendar").then(v => {
            if (rev !== mine) return; // a newer one is on its way
            data = v;
            drawn = "";
            if (st) draw();
          }, e => console.warn(e));
        }
        // Again each minute, as events begin and end; otherwise only when something changed.
        const look = JSON.stringify(linked().map(c => [c.name, c.color])); // renamed or recolored in Settings
        const key = [st.rev, st.error, lookAhead(), look, !!data, Math.floor(Date.now() / 60000)].join("|");
        if (key === drawn) return;
        drawn = key;
        draw();
      },
    };
  },
});
}
