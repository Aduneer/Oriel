"use strict";
{
const { register, el, refs, setHTML, setText } = Oriel;
const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);

const ICONS = {
  on: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 9.5h3.5L12 5.5v13l-4.5-4H4Z"/><path d="M15.5 9a4.2 4.2 0 0 1 0 6M18.2 6.5a8 8 0 0 1 0 11"/></svg>`,
  off: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 9.5h3.5L12 5.5v13l-4.5-4H4Z"/><path d="m16 9.5 5 5m0-5-5 5"/></svg>`,
};

// Programs as Windows names their files, as people know them. Anything else shows under its own name.
const NAMES = {
  chrome: "Chrome", msedge: "Edge", firefox: "Firefox", brave: "Brave", opera: "Opera", vivaldi: "Vivaldi",
  spotify: "Spotify", discord: "Discord", steam: "Steam", steamwebhelper: "Steam", teams: "Teams", "ms-teams": "Teams",
  zoom: "Zoom", slack: "Slack", vlc: "VLC", obs64: "OBS", applemusic: "Apple Music", itunes: "iTunes",
  "microsoft.media.player": "Media Player", wmplayer: "Windows Media Player", explorer: "Explorer",
};
const label = a => (a.pid === 0 ? "System sounds" : a.title || NAMES[a.exe.toLowerCase()] || a.exe || `Process ${a.pid}`);

const HOLD_MS = 1500; // after a change, the slider wins over samples until Windows has caught up
const SEND_MS = 50;   // while dragging, at most this often

register("volume", {
  name: "Volume mixer",
  description: "Your speakers' or headphones' volume and each app's, with mute",
  color: "accent",
  create(ctx) {
    const t = refs(el(`
      <article class="tile vol" style="--hue:var(--accent)">
        <header><h2>Volume</h2><span class="sub" data-k="device"></span>
          ${ctx.go("ms-settings:apps-volume", "Open Windows' volume mixer")}</header>
        <div class="vmaster" data-k="top"></div>
        <div class="vlist" data-k="list"></div>
        <p class="vnone" data-k="empty" hidden>No apps are using sound</p>
        <p class="vnone" data-k="none" hidden>No output device</p>
      </article>`));

    const SLIDER = `<input type="range" min="0" max="100" step="1" data-k="slider">`;
    const MUTE = `<button type="button" class="mute" data-k="mute"></button>`;

    /** A slider with its mute button: an app's row, or the device's big one when `pid` is null. */
    function row(pid) {
      const r = refs(el(pid == null
        ? `<div class="vrow master"><div class="stat"><span class="big" data-k="val"></span>${MUTE}</div>${SLIDER}</div>`
        : `<div class="vrow">${MUTE}<span class="vname" data-k="name"></span>${SLIDER}<span class="vval" data-k="val"></span></div>`));
      Object.assign(r, { pid, label: "", value: -1, muted: null, hold: 0, dragging: false, pending: null, timer: 0 });

      const set = args => invoke("set_volume", { pid, ...args }).catch(e => console.warn(e));
      // Leading and trailing: the first move goes out at once, the last one always arrives.
      const send = () => {
        if (r.pending == null) return void (r.timer = 0);
        set({ volume: r.pending });
        r.pending = null;
        r.timer = setTimeout(send, SEND_MS);
      };
      r.slider.addEventListener("input", () => {
        r.hold = Date.now() + HOLD_MS;
        show(r, +r.slider.value, r.muted);
        r.pending = +r.slider.value;
        if (!r.timer) send();
      });
      r.slider.addEventListener("pointerdown", () => {
        r.dragging = true;
        addEventListener("pointerup", () => {
          r.dragging = false;
          r.hold = Date.now() + HOLD_MS;
        }, { once: true });
      });
      r.mute.addEventListener("click", () => {
        r.hold = Date.now() + HOLD_MS;
        show(r, r.value, !r.muted);
        set({ muted: r.muted });
      });
      return r;
    }

    function show(r, value, muted) {
      if (r.value !== value) {
        r.value = value;
        if (r.slider.value !== String(value)) r.slider.value = value;
        r.slider.style.setProperty("--v", value);
        if (r.pid == null) setHTML(r.val, `${value}<small>%</small>`);
        else setText(r.val, String(value));
      }
      if (r.muted !== muted) {
        r.muted = muted;
        r.root.classList.toggle("muted", muted);
        setHTML(r.mute, muted ? ICONS.off : ICONS.on);
        r.mute.setAttribute("aria-pressed", String(muted));
      }
      r.mute.title = `${muted ? "Unmute" : "Mute"} ${r.label}`;
      r.mute.setAttribute("aria-label", r.mute.title);
    }

    /** A sample's values, unless the user is mid-change on this row. */
    function sync(r, value, muted) {
      if (!r.dragging && Date.now() >= r.hold) show(r, value, muted);
    }

    function name(r, text, title = text) {
      r.label = text;
      if (r.name) {
        setText(r.name, text);
        r.name.title = title;
      }
      r.slider.setAttribute("aria-label", `${text} volume`);
    }

    const master = row(null);
    t.top.append(master.root);
    const apps = new Map(); // pid -> row
    let order = "";         // the pids shown, so rows only move when apps come or go
    let missing = 0;        // samples in a row without the mixer (the first read takes a tick)

    return {
      root: t.root,
      update(s) {
        if (!s.procs) return; // history samples leave out the process list and the mixer
        const a = s.audio;
        missing = a ? 0 : missing + 1;
        t.top.hidden = t.list.hidden = !a;
        t.none.hidden = !!a || missing < 2;
        if (!a) {
          t.empty.hidden = true;
          setText(t.device, "");
          return;
        }
        setText(t.device, a.device);
        t.device.title = a.device;
        name(master, a.device || "Output");
        sync(master, a.volume, a.muted);

        const list = a.apps.map(x => ({ ...x, label: label(x) }));
        list.sort((x, y) => (x.pid === 0) - (y.pid === 0) || x.label.localeCompare(y.label));
        const key = list.map(x => x.pid).join(",");
        if (key !== order) {
          order = key;
          const keep = new Set(list.map(x => x.pid));
          for (const [pid, r] of apps) if (!keep.has(pid)) { r.root.remove(); apps.delete(pid); }
          for (const x of list) {
            if (!apps.has(x.pid)) apps.set(x.pid, row(x.pid));
            t.list.append(apps.get(x.pid).root); // appending an existing row moves it into order
          }
        }
        for (const x of list) {
          const r = apps.get(x.pid);
          name(r, x.label, x.pid && x.exe ? `${x.label} (${x.exe}.exe, PID ${x.pid})` : x.label);
          r.root.classList.toggle("idle", !x.active);
          sync(r, x.volume, x.muted);
        }
        t.empty.hidden = list.length > 0;
      },
    };
  },
});
}
