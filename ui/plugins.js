"use strict";
// Widgets from the plugin folder (%APPDATA%\Oriel\widgets). Only does anything when the
// setting is on. Each plugin is a script that calls Oriel.register, like the built-in widgets;
// this file loads them one at a time and keeps a broken one from taking the dashboard with it.
{
const O = Oriel;
const { el } = O;
const { invoke } = window.__TAURI__.core;

const LOAD_TIMEOUT_MS = 5000;
const real = O.register;
let current = null; // the file being loaded, so a register call can be traced to it

const P = (O.plugins = {
  loaded: 0,       // widgets registered from the folder
  problems: [],    // [{file, message}], shown in Settings
  load: () => (started ??= run()),
});
let started = null;

const say = e => String(e instanceof Error ? e.message : e).slice(0, 300);
function problem(file, message) {
  P.problems.push({ file, message });
  console.warn(`plugin ${file}: ${message}`);
}

// ---- registering ---------------------------------------------------------------------
O.register = (id, def) => {
  if (!current) return real(id, def);
  const file = current;
  if (typeof id !== "string" || !/^[a-z0-9-]{1,32}$/.test(id)) return problem(file, "a widget id is lowercase letters, digits and dashes, up to 32 characters");
  if (!def || typeof def.create !== "function") return problem(file, `${id}: needs a create(ctx) function`);
  const clash = O.widgets.get(id);
  if (clash) return problem(file, clash.plugin ? `id "${id}" is already used by ${clash.plugin}, which was loaded first` : `id "${id}" belongs to a built-in widget, which wins`);
  P.loaded++;
  real(id, {
    ...def,
    name: `${def.name || id} (plugin)`,
    description: `${def.description || ""} Plugin: ${file}.`.trim(),
    color: /^[a-z0-9-]+$/.test(def.color || "") ? def.color : "muted",
    plugin: file,
    create: guarded(id, file, def),
  });
};

/** create() and update() as the dashboard calls them, with a plugin's mistakes turned into an error tile. */
function guarded(id, file, def) {
  return ctx => {
    let w;
    try {
      w = def.create(ctx);
      if (!w || !(w.root instanceof Element)) throw new Error("create() must return {root, update}, with root a DOM element");
      if (w.update != null && typeof w.update !== "function") throw new Error("update must be a function");
    } catch (e) {
      problem(file, `${id}: create() failed: ${say(e)}`);
      return { root: errorTile(file, `create() failed: ${say(e)}`), update() {} };
    }
    let broken = false;
    return {
      root: w.root,
      update(sample) {
        if (broken || !w.update) return;
        try {
          w.update(sample);
        } catch (e) {
          broken = true; // once is enough; the tile says what happened
          problem(file, `${id}: update() failed: ${say(e)}`);
          const tile = errorTile(file, `update() failed: ${say(e)}`);
          w.root.removeAttribute("style");
          w.root.className = tile.className;
          w.root.replaceChildren(...tile.childNodes);
        }
      },
    };
  };
}

function errorTile(file, message) {
  const tile = el(`<article class="tile plugin-error"><header><h2>Plugin error</h2></header><p class="file"></p><p class="msg"></p></article>`);
  tile.querySelector(".file").textContent = file;
  tile.querySelector(".msg").textContent = message;
  return tile;
}

// ---- loading -------------------------------------------------------------------------
async function run() {
  if (!O.state.settings.plugins) return;
  let files;
  try {
    files = await invoke("list_plugins");
  } catch (e) {
    return problem("widgets folder", say(e));
  }
  for (const f of files) {
    if (f.css) await tag("link", { rel: "stylesheet", href: f.css }, f.file);
    current = f.file;
    await tag("script", { src: f.url, crossorigin: "anonymous" }, f.file, true);
    current = null;
  }
}

/** Add a <link> or <script> and wait for it. Never rejects: a plugin that fails is a problem to report. */
function tag(name, attrs, file, isScript = false) {
  return new Promise(done => {
    const node = Object.assign(document.createElement(name), attrs);
    // An error thrown while the script runs top to bottom (it all happens before `load`).
    const onError = e => {
      if (isScript && e.filename === attrs.src) problem(file, `${e.message} (line ${e.lineno})`);
    };
    const finish = failure => {
      clearTimeout(timer);
      removeEventListener("error", onError);
      if (failure) problem(file, failure);
      done();
    };
    const timer = setTimeout(() => finish("took too long to load"), LOAD_TIMEOUT_MS);
    addEventListener("error", onError);
    node.onload = () => finish();
    node.onerror = () => finish(`couldn't load ${name === "link" ? "the style sheet" : "the file"} (too big, or not a plain .js or .css file?)`);
    document.head.appendChild(node);
  });
}
}
