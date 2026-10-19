"use strict";
// The weather and calendar parts of the settings panel, the two widgets that go online. settings.js
// puts `html()` in the panel, calls `fill()` after each time the panel is drawn and `wire()` once.
{
const O = Oriel;
const { el, esc } = O;
const { invoke } = window.__TAURI__.core;

const MAX_CALENDARS = 5, MAX_NAME = 40; // as in settings.rs
// Theme tokens a calendar can have (themes.css). Each theme paints them differently, so they have numbers, not color names.
const COLORS = [["s1", "Color 1"], ["s2", "Color 2"], ["gpu", "Color 3"], ["mem", "Color 4"], ["cpu", "Color 5"]];
const DAYS = [[7, "7 days"], [14, "14 days"]];

const UNITS = [["auto", "Auto"], ["c", "°C"], ["f", "°F"]];

let api = null;        // {save, S, drawer} from settings.js
let found = [];        // the last place search's results
let armTimer = 0;

const options = (list, value) => list.map(([v, label]) => `<option value="${v}"${String(v) === String(value) ? " selected" : ""}>${esc(label)}</option>`).join("");

function html(s) {
  return `
      <section data-sk="weather forecast temperature rain online internet open-meteo">
        <h3>Weather</h3>
        <p class="note" data-s="weather online internet open-meteo privacy">The weather widget asks Open-Meteo for the forecast at the chosen place's coordinates, every 30 minutes or so while it's on a page. Nothing else is sent. Add the widget in Layout.</p>
        <div class="stack" data-s="weather place location city town search coordinates">
          <span>Place</span>
          <div class="placeline" data-place-line></div>
          <div class="wsearch">${O.findBox("Search for a place", "Search for a city")}<button class="btn" type="button" data-place-search>Search</button></div>
          <div class="places" data-places aria-live="polite"></div>
          <span class="hint">Searching asks Open-Meteo's place search for the words you typed, once, when you press Enter.</span>
        </div>
        <div class="field" data-s="units temperature celsius fahrenheit"><span>Temperature</span>
          <span class="seg" role="group" aria-label="Temperature units" data-seg="weather_units">
            ${UNITS.map(([v, label]) => `<button type="button" data-v="${v}" aria-pressed="${s.weather_units === v}">${label}</button>`).join("")}
          </span></div>
      </section>

      <section data-sk="calendar ics ical events agenda google outlook apple online internet">
        <h3>Calendar</h3>
        <p class="note" data-s="calendar online internet ics privacy link secret">The calendar widget fetches each link below every 15 minutes or so while it's on a page. Nothing else is sent. Treat a link like a password: anyone who has it can read that calendar. Oriel keeps it only in the settings file and shows it only here.</p>
        <p class="note" data-s="calendar link ics google outlook apple icloud secret address publish where find"><b>Google:</b> calendar settings, Integrate calendar, Secret address in iCal format. <b>Outlook:</b> Settings, Calendar, Shared calendars, Publish a calendar, ICS link. <b>Apple:</b> share the calendar as a public calendar and copy its link.</p>
        <div class="crows" data-cals data-s="calendar link ics url name color colour add remove"></div>
        <div class="actions" data-s="calendar add link ics"><button class="btn" type="button" data-add-cal>Add a calendar</button></div>
        <div class="field" data-s="calendar days ahead look next week"><label for="s-caldays">Look ahead</label>
          <select id="s-caldays" data-set="calendar_days" data-num>${options(DAYS, s.calendar_days)}</select></div>
      </section>`;
}

const root = () => api.drawer();
const q = sel => root().querySelector(sel);

// ---- weather -------------------------------------------------------------------
function fillPlace() {
  const p = api.S().weather_place;
  q("[data-place-line]").innerHTML = p
    ? `<b>${esc(p.name)}</b><button class="btn link" type="button" data-place-clear>Remove</button>`
    : `<span class="note">None chosen yet</span>`;
}

function showPlaces(html) {
  q("[data-places]").innerHTML = html;
}

async function search() {
  const input = q(".wsearch input");
  const words = input.value.trim();
  if (words.length < 2) return;
  showPlaces(`<p class="note">Searching…</p>`);
  try {
    found = await invoke("search_places", { query: words });
    showPlaces(found.length
      ? found.map((p, i) => `<button type="button" class="place" data-place="${i}"><b>${esc(p.name)}</b><small>${esc(p.detail)}</small></button>`).join("")
      : `<p class="note">No places match “${esc(words)}”.</p>`);
  } catch (e) {
    showPlaces(`<p class="note bad">Couldn't search: ${esc(e)}</p>`);
  }
}

// ---- calendars -----------------------------------------------------------------
function row(c, i) {
  const swatches = [["", "Automatic"], ...COLORS].map(([v, name]) =>
    `<button type="button" class="sw${v ? "" : " auto"}" data-swatch="${v}" style="--c:var(--${v || "muted"})" aria-pressed="${c.color === v}" title="${name}" aria-label="${name} color"></button>`).join("");
  return `
    <div class="crow" data-cal="${i}">
      <div class="ctop">
        <input type="text" class="cname" value="${esc(c.name)}" maxlength="${MAX_NAME}" placeholder="Calendar ${i + 1}" spellcheck="false" autocomplete="off" aria-label="Name of calendar ${i + 1}">
        <span class="swatches" role="group" aria-label="Color of calendar ${i + 1}">${swatches}</span>
        <button type="button" class="pbtn del" data-del-cal title="Remove this calendar" aria-label="Remove calendar ${i + 1}">${DEL}<span class="ask">Remove?</span></button>
      </div>
      <div class="curl">
        <input type="text" class="curl-in mask" value="${esc(c.url)}" spellcheck="false" autocomplete="off" placeholder="https:// or webcal:// link" aria-label="Link of calendar ${i + 1}">
        <button type="button" class="btn link" data-show-url aria-pressed="false">Show</button>
      </div>
      <p class="note bad" data-bad hidden>That isn't a link Oriel can fetch: it has to start with https:// or webcal://.</p>
    </div>`;
}
const DEL = `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 7l10 10M17 7 7 17"/></svg>`;

function fillCals() {
  const list = api.S().calendars;
  q("[data-cals]").innerHTML = list.map(row).join("");
  const add = q("[data-add-cal]");
  add.disabled = list.length >= MAX_CALENDARS;
  add.title = add.disabled ? `Up to ${MAX_CALENDARS} calendars` : "";
}

/** What's in the rows now, in order. */
function rows() {
  return [...root().querySelectorAll("[data-cal]")].map(r => ({
    name: r.querySelector(".cname").value,
    color: r.querySelector("[data-swatch][aria-pressed='true']")?.dataset.swatch ?? "",
    url: r.querySelector(".curl-in").value,
  }));
}

/** The backend empties a link it can't fetch; say so next to what was typed, which stays as it was. */
function checkLinks() {
  const saved = api.S().calendars;
  root().querySelectorAll("[data-cal]").forEach((r, i) => {
    const typed = r.querySelector(".curl-in").value.trim();
    r.querySelector("[data-bad]").hidden = !typed || !!saved[i]?.url;
  });
}

async function saveCals() {
  await api.save({ calendars: rows() });
  checkLinks();
}
let timer = 0;
const saveSoon = () => {
  clearTimeout(timer);
  timer = setTimeout(saveCals, 500);
};

function disarm() {
  clearTimeout(armTimer);
  root().querySelectorAll(".armed").forEach(b => {
    b.classList.remove("armed");
    b.title = "Remove this calendar";
  });
}

function wire() {
  const d = root();
  d.addEventListener("click", e => {
    const t = e.target;
    if (t.closest("[data-place-search]")) return search();
    const pick = t.closest("[data-place]");
    if (pick) {
      const p = found[Number(pick.dataset.place)];
      if (!p) return;
      found = [];
      showPlaces("");
      q(".wsearch input").value = "";
      api.save({ weather_place: { name: p.name, lat: p.lat, lon: p.lon } }).then(fillPlace); // the backend may have tidied it
      return fillPlace();
    }
    if (t.closest("[data-place-clear]")) {
      api.save({ weather_place: null });
      return fillPlace();
    }
    const seg = t.closest('[data-seg="weather_units"] button');
    if (seg) {
      seg.parentElement.querySelectorAll("button").forEach(b => b.setAttribute("aria-pressed", String(b === seg)));
      return; // settings.js saves it
    }

    const row = t.closest("[data-cal]");
    if (t.closest("[data-add-cal]")) {
      const list = api.S().calendars;
      if (list.length >= MAX_CALENDARS) return;
      api.save({ calendars: [...rows(), { name: "", color: "", url: "" }] }).then(() => {
        fillCals();
        const inputs = root().querySelectorAll(".curl-in");
        inputs[inputs.length - 1]?.focus();
      });
      return;
    }
    if (!row) return;
    const swatch = t.closest("[data-swatch]");
    if (swatch) {
      row.querySelectorAll("[data-swatch]").forEach(b => b.setAttribute("aria-pressed", String(b === swatch)));
      return saveCals();
    }
    if (t.closest("[data-show-url]")) {
      const input = row.querySelector(".curl-in");
      const shown = input.classList.toggle("mask") === false;
      t.closest("[data-show-url]").textContent = shown ? "Hide" : "Show";
      t.closest("[data-show-url]").setAttribute("aria-pressed", String(shown));
      return;
    }
    const del = t.closest("[data-del-cal]");
    if (del) {
      // A calendar that has a link takes a second click, so the link isn't lost by accident.
      if (row.querySelector(".curl-in").value.trim() && !del.classList.contains("armed")) {
        disarm();
        del.classList.add("armed");
        del.title = "Click again to remove this calendar and its link";
        armTimer = setTimeout(disarm, 4000);
        return;
      }
      const i = Number(row.dataset.cal);
      api.save({ calendars: rows().filter((_, k) => k !== i) }).then(fillCals);
    }
  });
  d.addEventListener("input", e => {
    if (e.target.matches(".cname")) saveSoon();
  });
  d.addEventListener("change", e => {
    if (e.target.matches(".curl-in")) saveCals(); // when the field is left, not on every key
  });
  d.addEventListener("keydown", e => {
    if (e.key !== "Enter") return;
    if (e.target.matches(".wsearch input")) {
      e.preventDefault();
      search();
    } else if (e.target.matches(".curl-in, .cname")) {
      e.target.blur();
    }
  });
}

O.onlineSettings = {
  html,
  /** Called each time the panel has been drawn. */
  fill() {
    found = [];
    fillPlace();
    fillCals();
  },
  /** Called once, with settings.js's `save`, `S` (the current settings) and `drawer`. */
  wire(hooks) {
    api = hooks;
    wire();
  },
};
}
