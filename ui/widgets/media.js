"use strict";
{
const { register, el, refs, setHTML, setText, meter } = Oriel;
const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);

const ICONS = {
  previous: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 5v14M19 5.5 9 12l10 6.5Z"/></svg>`,
  next: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M18 5v14M5 5.5 15 12 5 18.5Z"/></svg>`,
  play: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 5v14l11-7Z"/></svg>`,
  pause: `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 5h3v14H7ZM14 5h3v14h-3Z"/></svg>`,
  note: `<svg class="note" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 18V5l11-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="17" cy="16" r="3"/></svg>`,
};

// Players as Windows identifies them, as people know them. Anything else shows under its own name.
const APPS = {
  spotify: "Spotify", spotifymusic: "Spotify", chrome: "Chrome", msedge: "Edge", firefox: "Firefox",
  zunemusic: "Media Player", zunevideo: "Films & TV", vlc: "VLC", applemusicwin: "Apple Music", itunes: "iTunes",
  tidal: "TIDAL", deezer: "Deezer", musicbee: "MusicBee", foobar2000: "foobar2000", brave: "Brave", opera: "Opera",
};
/** "Spotify.exe", "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic" or "Helium.LYMXGRRS252…" to a name. */
function appName(id = "") {
  let s = id.replace(/\.exe$/i, "");
  if (/^[0-9A-F]{16}$/.test(s)) return "Firefox"; // Firefox's id is a hash of the folder it's installed in
  if (s.includes("!")) s = s.split("!")[0].split("_")[0]; // Store app: Publisher.Name_hash!App
  // Chromium browsers add a hash per profile: Helium.LYMXGRRS…, Chrome._crx_…
  const parts = s.split(".").filter(p => p && !/^[0-9A-Z]{12,}$/.test(p) && !p.startsWith("_crx_"));
  for (const p of [s, ...parts]) {
    const known = APPS[p.toLowerCase()];
    if (known) return known;
  }
  return parts.at(-1) ?? "";
}

/** 83 -> "1:23", 3725 -> "1:02:05" */
function clock(secs) {
  const s = Math.max(0, Math.floor(secs));
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), ss = String(s % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

register("media", {
  name: "Now playing",
  description: "What's playing in Spotify, a browser or any app in Windows' media controls, with play/pause and skip",
  color: "accent",
  size: { w: 1.1 },
  create() {
    const t = refs(el(`
      <article class="tile media" style="--hue:var(--accent)">
        <header><h2>Now playing</h2><span class="sub" data-k="app"></span></header>
        <div class="np" data-k="np" hidden>
          <div class="art">${ICONS.note}<img data-k="art" alt="" hidden></div>
          <div class="info">
            <div class="title" data-k="title"></div>
            <div class="by" data-k="by"></div>
            <div class="time" data-k="time"><span data-k="pos"></span><div class="meter" data-k="bar"><i></i></div><span data-k="len"></span></div>
            <div class="ctl">
              <button type="button" data-media="previous" data-k="prev" title="Previous" aria-label="Previous">${ICONS.previous}</button>
              <button type="button" data-media="toggle" data-k="toggle" class="play"></button>
              <button type="button" data-media="next" data-k="next" title="Next" aria-label="Next">${ICONS.next}</button>
            </div>
          </div>
        </div>
        <p class="idle" data-k="idle">${ICONS.note}<span>Nothing playing</span>
          <small>Play something in Spotify, a browser or Media Player</small></p>
      </article>`));
    let m = null;     // what the backend last said
    let art = 0;      // which cover is showing (see `art` in media.rs)
    let guess = null; // {playing, until}: after a click, until the player confirms it

    function showArt(id) {
      art = id;
      if (!id) {
        t.art.hidden = true;
        t.art.removeAttribute("src");
        return;
      }
      invoke("media_art").then(url => {
        if (art !== id) return; // the track changed meanwhile
        const ok = typeof url === "string" && url.startsWith("data:image/");
        if (ok) t.art.src = url;
        t.art.hidden = !ok;
      }, e => console.warn(e));
    }

    function showToggle(playing) {
      setHTML(t.toggle, playing ? ICONS.pause : ICONS.play);
      t.toggle.title = playing ? "Pause" : "Play";
      t.toggle.setAttribute("aria-label", t.toggle.title);
    }

    t.root.addEventListener("click", e => {
      const b = e.target.closest("[data-media]");
      if (!b || b.disabled || !m) return;
      if (b.dataset.media === "toggle") {
        guess = { playing: !m.playing, until: Date.now() + 2000 };
        showToggle(guess.playing);
      }
      invoke("media_control", { action: b.dataset.media }).catch(e => console.warn(e));
    });

    return {
      root: t.root,
      update(s) {
        if (!s.procs) return; // history samples leave out the process list and media
        m = s.media?.title ? s.media : null;
        t.np.hidden = !m;
        t.idle.hidden = !!m;
        setText(t.app, m ? appName(m.app) : "");
        if (!m) {
          if (art) showArt(0);
          return;
        }
        setText(t.title, m.title);
        setText(t.by, [m.artist, m.album].filter(Boolean).join(" · "));
        t.title.title = m.title;
        t.by.title = t.by.textContent;
        if (m.art !== art) showArt(m.art);

        if (guess && (guess.playing === m.playing || Date.now() > guess.until)) guess = null;
        showToggle(guess ? guess.playing : m.playing);
        t.toggle.disabled = !m.can_toggle;
        t.prev.disabled = !m.can_prev;
        t.next.disabled = !m.can_next;

        // Players report the position now and then; move it along by the time since.
        t.time.hidden = m.duration == null;
        if (m.duration != null) {
          const pos = Math.min(m.duration, m.position + (m.playing ? Math.max(0, s.t - m.at) : 0));
          setText(t.pos, clock(pos));
          setText(t.len, clock(m.duration));
          meter(t.bar, (pos / m.duration) * 100);
        }
      },
    };
  },
});
}
