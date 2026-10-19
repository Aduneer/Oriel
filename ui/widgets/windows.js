"use strict";
{
const { register, el, refs, esc, setHTML, setText } = Oriel;
const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);

const NOTE_MS = 4000;

// Programs as Windows names their files, as people know them. Anything else shows under its own name.
const NAMES = {
  chrome: "Chrome", msedge: "Edge", firefox: "Firefox", brave: "Brave", opera: "Opera", vivaldi: "Vivaldi",
  code: "VS Code", devenv: "Visual Studio", notepad: "Notepad", "notepad++": "Notepad++", explorer: "Explorer",
  windowsterminal: "Terminal", powershell: "PowerShell", cmd: "Command Prompt",
  spotify: "Spotify", discord: "Discord", steam: "Steam", teams: "Teams", "ms-teams": "Teams", slack: "Slack",
  zoom: "Zoom", vlc: "VLC", obs64: "OBS", winword: "Word", excel: "Excel", powerpnt: "PowerPoint", outlook: "Outlook",
  onenote: "OneNote", mspaint: "Paint", taskmgr: "Task Manager", mmc: "Management Console",
  applicationframehost: "Windows app", systemsettings: "Settings",
};
const appName = w => NAMES[w.exe.toLowerCase()] || w.exe || "Window";

/** The text with the search match marked. */
function markHTML(text, needle) {
  const i = needle ? text.toLowerCase().indexOf(needle) : -1;
  if (i < 0) return esc(text);
  const j = i + needle.length;
  return `${esc(text.slice(0, i))}<mark>${esc(text.slice(i, j))}</mark>${esc(text.slice(j))}`;
}

/** A hue from 0–359 that stays the same for a program, so its letter keeps its color. */
function hue(name) {
  let h = 0;
  for (const c of name.toLowerCase()) h = (h * 31 + c.charCodeAt(0)) % 360;
  return h;
}

const CLOSE_ICON = `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18"/></svg>`;

register("windows", {
  name: "Open windows",
  description: "The windows that are open, like the taskbar. Click one to bring it to the front.",
  color: "accent",
  create() {
    const t = refs(el(`
      <article class="tile wins" style="--hue:var(--accent)">
        <header><h2>Windows</h2><span class="sub" data-k="sub"></span>
          ${Oriel.findBox("Search windows", "Search")}</header>
        <div class="wlist" data-k="list"></div>
      </article>`));
    const q = t.root.querySelector(".find input");
    let wins = null;  // newest list from the backend: {hwnd, pid, exe, title, minimized, front}
    let needle = "";
    let note = null;  // {text, until}: why a click didn't work

    const hit = w => !needle || `${appName(w)} ${w.exe} ${w.title}`.toLowerCase().includes(needle);

    function rowHTML(w) {
      const name = appName(w);
      const cls = `${w.front ? " front" : ""}${w.minimized ? " min" : ""}`;
      const label = `${name}: ${w.title}`;
      return `<div class="wrow${cls}" data-h="${w.hwnd}" data-p="${w.pid}">` +
        `<button type="button" class="wmain" title="${esc(`${label}${w.exe ? ` (${w.exe}.exe)` : ""}${w.minimized ? ", minimized" : ""}`)}" aria-label="${esc(`Switch to ${label}`)}">` +
        `<span class="wicon" style="--h:${hue(w.exe || name)}" aria-hidden="true">${esc([...name][0].toUpperCase())}</span>` +
        `<span class="wtext"><span class="wapp">${markHTML(name, needle)}${w.minimized ? `<small>minimized</small>` : ""}</span>` +
        `<span class="wtitle">${markHTML(w.title, needle)}</span></span></button>` +
        `<button type="button" class="wclose" title="Close this window" aria-label="${esc(`Close ${label}`)}">${CLOSE_ICON}</button></div>`;
    }

    function refresh() {
      if (note && note.until < Date.now()) note = null;
      const shown = (wins || []).filter(hit);
      setText(t.sub, note ? note.text : !wins ? "" : needle ? `${shown.length} of ${wins.length}` : `${wins.length} open`);
      t.sub.classList.toggle("bad", !!note);
      if (!wins) return;
      setHTML(t.list, shown.length
        ? shown.map(rowHTML).join("")
        : `<p class="none">${wins.length ? `No window matches “${esc(q.value.trim())}”` : "No windows are open"}</p>`);
    }

    /** Ask for `cmd` on the row's window, and say why not if Windows won't. */
    async function act(cmd, row, refused) {
      const args = { hwnd: Number(row.dataset.h), pid: Number(row.dataset.p) };
      try {
        await invoke(cmd, args);
      } catch (e) {
        console.warn(e);
        note = { text: typeof e === "string" && e.includes("gone") ? "That window has closed" : refused, until: Date.now() + NOTE_MS };
        refresh();
      }
    }

    t.list.addEventListener("click", e => {
      const row = e.target.closest(".wrow");
      if (!row) return;
      if (e.target.closest(".wclose")) act("close_window", row, "Windows wouldn't close that window");
      else if (e.target.closest(".wmain")) act("focus_window", row, "Windows wouldn't bring that window to the front");
    });
    // Middle-click closes, like a browser tab (and some taskbars). Stop it starting autoscroll.
    t.list.addEventListener("mousedown", e => { if (e.button === 1 && e.target.closest(".wmain")) e.preventDefault(); });
    t.list.addEventListener("auxclick", e => {
      const row = e.button === 1 && e.target.closest(".wmain") && e.target.closest(".wrow");
      if (row) act("close_window", row, "Windows wouldn't close that window");
    });

    q.addEventListener("input", () => {
      needle = q.value.trim().toLowerCase();
      t.list.scrollTop = 0;
      refresh();
    });
    q.addEventListener("keydown", e => {
      if (e.key !== "Escape") return;
      e.stopPropagation(); // or Escape also closes the settings
      if (!q.value) return q.blur();
      e.preventDefault();
      q.value = "";
      q.dispatchEvent(new Event("input"));
    });

    return {
      root: t.root,
      update(s) {
        if (!s.procs) return; // history samples leave the window list out
        wins = s.windows ?? wins; // not sent until the first read
        refresh();
      },
    };
  },
});
}
