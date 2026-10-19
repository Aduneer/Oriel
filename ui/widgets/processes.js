"use strict";
{
const { register, el, refs, esc, setHTML, setText, fmt } = Oriel;

const ROW_REM = 2.05; // keep in step with `.procs .r` in style.css
const COLUMNS = {
  name: { dir: 1, cmp: (a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }) },
  cpu: { dir: -1, cmp: (a, b) => a.cpu - b.cpu || a.mem - b.mem },
  mem: { dir: -1, cmp: (a, b) => a.mem - b.mem || a.cpu - b.cpu },
};
const SORT_KEY = "oriel.processes.sort";
const VIEW_KEY = "oriel.processes.view";
const TREE_ICON = `<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 4v5a2 2 0 0 0 2 2h3M4 9v8a2 2 0 0 0 2 2h3M13 5h7M13 11h7M13 19h7"/></svg>`;
const NOTE_MS = 5000;
const REFUSED = "It runs as administrator or as part of Windows, and Oriel only has your rights.";

// Sort order and view are per-screen conveniences, so they live in the page's storage, not the settings file.
function loadSort() {
  try {
    const v = JSON.parse(localStorage.getItem(SORT_KEY));
    if (COLUMNS[v?.by] && (v.dir === 1 || v.dir === -1)) return v;
  } catch {}
  return { by: "cpu", dir: -1 };
}
function loadTree() {
  try { return localStorage.getItem(VIEW_KEY) === "tree"; } catch { return false; }
}
function store(key, value) {
  try { localStorage.setItem(key, value); } catch {}
}

/** The text with the search match marked. */
function markHTML(text, needle) {
  const i = needle ? text.toLowerCase().indexOf(needle) : -1;
  if (i < 0) return esc(text);
  const j = i + needle.length;
  return `${esc(text.slice(0, i))}<mark>${esc(text.slice(i, j))}</mark>${esc(text.slice(j))}`;
}

register("processes", {
  name: "Processes",
  description: "Every running app, grouped by name or as a tree. Search, sort, and right-click to end one.",
  color: "muted",
  size: { w: 1.2 },
  create(ctx) {
    const t = refs(el(`
      <article class="tile procs" style="--hue:var(--muted)">
        <header><h2>Processes</h2><span class="sub" data-k="sub"></span>
          <button class="toggle" type="button" data-k="view">${TREE_ICON}</button>
          ${Oriel.findBox("Search processes", "Search")}
          ${ctx.go("task-manager", "Open Task Manager")}</header>
        <div class="cols" data-k="head">
          <button type="button" data-sort="name">Name</button>
          <button type="button" data-sort="cpu">CPU</button>
          <button type="button" data-sort="mem">Memory</button>
        </div>
        <div class="list" data-k="list"><div data-k="rows"></div></div>
        <div class="pmenu" data-k="menu" role="menu" hidden>
          <div class="who" data-k="who"></div>
          <button type="button" role="menuitem" data-k="end"></button>
        </div>
      </article>`));
    const q = t.root.querySelector(".find input");
    let procs = [];          // newest list from the backend: {pid, ppid, name, cpu, mem}
    let rows = [];           // what the list shows, top to bottom
    let shown = 0;           // processes matching the search
    let needle = "";
    let sort = loadSort();
    let tree = loadTree();
    const opened = new Set(); // group names expanded in the grouped view
    const closed = new Set(); // pids collapsed in the tree view
    let queued = false;
    let box = "";
    let seen = 0;            // time of the sample the list shows (see end_processes in main.rs)
    let picked = null;       // {key, name, pids, seen} while the right-click menu is open
    let note = null;         // {text, title, bad, until}: what came of ending something

    const order = () => {
      const c = COLUMNS[sort.by];
      return (a, b) => c.cmp(a, b) * sort.dir || a.pid - b.pid;
    };
    // Digits also find a process by its id.
    const pidHit = p => /^\d+$/.test(needle) && String(p.pid).startsWith(needle);
    const hit = p => p.name.toLowerCase().includes(needle) || pidHit(p);

    /** Grouped by name, like Task Manager; a group opens up into its processes. */
    function groupRows() {
      const by = new Map();
      for (const p of procs) {
        let g = by.get(p.name);
        if (!g) by.set(p.name, (g = { name: p.name, pid: 0, cpu: 0, mem: 0, all: [], kids: null }));
        g.cpu += p.cpu;
        g.mem += p.mem;
        g.all.push(p);
      }
      let groups = [...by.values()];
      shown = procs.length;
      if (needle) {
        shown = 0;
        groups = groups.filter(g => {
          // A name match shows the whole group; an id match opens it to just that process.
          g.kids = g.name.toLowerCase().includes(needle) ? null : g.all.filter(pidHit);
          shown += g.kids ? g.kids.length : g.all.length;
          return !g.kids || g.kids.length;
        });
      }
      const cmp = order();
      groups.sort(cmp);
      const out = [];
      for (const g of groups) {
        const many = g.all.length > 1;
        const open = many && (g.kids ? true : opened.has(g.name));
        out.push({ key: `g:${g.name}`, name: g.name, cpu: g.cpu, mem: g.mem, depth: 0, kids: many, open, toggle: many && !g.kids, badge: many ? `×${g.all.length}` : "" });
        if (!open) continue;
        const kids = (g.kids || g.all).slice().sort(cmp);
        kids.forEach((p, i) => out.push({ key: `p:${p.pid}`, pid: p.pid, cpu: p.cpu, mem: p.mem, depth: 1, lines: [], last: i === kids.length - 1 }));
      }
      return out;
    }

    /** Parents and children, like btop. A collapsed branch shows its totals. */
    function treeRows() {
      const nodes = new Map(procs.map(p => [p.pid, { ...p, own: p, kids: [], n: 0 }]));
      const roots = [];
      for (const n of nodes.values()) (nodes.get(n.ppid)?.kids ?? roots).push(n);
      for (const pid of closed) if (!nodes.has(pid)) closed.delete(pid);
      // Totals for every branch, and which branches lead to a search match.
      const total = n => {
        n.cpu = n.own.cpu;
        n.mem = n.own.mem;
        n.hit = !!needle && hit(n);
        n.keep = !needle || n.hit;
        for (const k of n.kids) {
          total(k);
          n.cpu += k.cpu;
          n.mem += k.mem;
          n.n += 1 + k.n;
          n.keep ||= k.keep;
        }
      };
      roots.forEach(total);
      shown = needle ? [...nodes.values()].filter(n => n.hit).length : procs.length;
      const cmp = order();
      const out = [];
      const walk = (list, depth, lines) => {
        list = list.filter(n => n.keep).sort(cmp);
        list.forEach((n, i) => {
          const last = i === list.length - 1;
          const kids = n.kids.some(k => k.keep);
          const open = kids && (!!needle || !closed.has(n.pid));
          const whole = kids && !open; // collapsed: show the branch's totals
          out.push({
            key: `p:${n.pid}`, pid: n.pid, name: n.name, depth, lines, last, kids, open,
            toggle: kids && !needle, ctx: needle && !n.hit,
            cpu: whole || !kids ? n.cpu : n.own.cpu, mem: whole || !kids ? n.mem : n.own.mem,
            badge: whole ? `+${n.n}` : "",
          });
          if (open) walk(n.kids, depth + 1, depth ? [...lines, !last] : []);
        });
      };
      walk(roots, 0, []);
      return out;
    }

    function rowHTML(r) {
      const guides = r.depth ? r.lines.map(v => `<i class="g${v ? " v" : ""}"></i>`).join("") + `<i class="g ${r.last ? "l" : "t"}"></i>` : "";
      const twig = r.kids ? `<i class="tw c${r.open ? " open" : ""}"></i>` : `<i class="tw${r.depth ? " leaf" : ""}"></i>`;
      const label = r.name != null ? markHTML(r.name, needle) : `<span class="pid">PID ${markHTML(String(r.pid), needle)}</span>`;
      const title = r.name != null && r.pid ? `${r.name} · PID ${r.pid}` : r.name ?? `PID ${r.pid}`;
      return `<div class="r${r.toggle ? " x" : ""}${r.ctx ? " ctx" : ""}${picked?.key === r.key ? " picked" : ""}" data-k="${esc(r.key)}" title="${esc(title)}">` +
        `<span class="n">${guides}${twig}<span class="nm">${label}</span>${r.badge ? `<span class="count">${r.badge}</span>` : ""}</span>` +
        `<span>${r.cpu.toFixed(1)}%</span><span>${fmt.bytes(r.mem)}</span></div>`;
    }

    // Only the rows in view are in the DOM; a spacer keeps the scrollbar honest.
    function draw() {
      queued = false;
      const rowH = Oriel.fontPx * ROW_REM;
      const first = Math.floor(t.list.scrollTop / rowH);
      const count = Math.ceil(t.list.clientHeight / rowH) + 1;
      const size = `${rows.length * rowH}px ${first * rowH}px`;
      if (size !== box) {
        box = size;
        t.rows.style.height = `${rows.length * rowH}px`;
        t.rows.style.paddingTop = `${first * rowH}px`;
      }
      if (!rows.length) {
        return setHTML(t.rows, needle && procs.length ? `<p class="none">Nothing running matches “${esc(q.value.trim())}”</p>` : "");
      }
      setHTML(t.rows, rows.slice(first, first + count).map(rowHTML).join(""));
    }

    function refresh() {
      rows = tree ? treeRows() : groupRows();
      draw();
      if (note && note.until < Date.now()) note = null;
      setText(t.sub, note ? note.text : needle ? `${shown} of ${procs.length}` : procs.length ? `${procs.length} running` : "");
      t.sub.classList.toggle("bad", !!note?.bad);
      t.sub.title = note ? note.title || note.text : "";
    }

    // ---- ending processes: right-click a row, then "End" ----
    /** The processes a row stands for: a whole group, or one process. */
    function target(key) {
      if (key.startsWith("g:")) {
        const name = key.slice(2);
        const pids = procs.filter(p => p.name === name).map(p => p.pid);
        return pids.length ? { key, name, pids } : null;
      }
      const p = procs.find(p => p.pid === Number(key.slice(2)));
      return p ? { key, name: p.name, pids: [p.pid] } : null;
    }

    function openMenu(pick, x, y) {
      picked = { ...pick, seen };
      const n = pick.pids.length;
      setText(t.who, n > 1 ? `${pick.name} · ${n} processes` : `${pick.name} · PID ${pick.pids[0]}`);
      t.end.textContent = n > 1 ? `End all ${n}` : "End process";
      t.end.disabled = false;
      t.menu.hidden = false;
      // Next to the pointer, kept inside the tile.
      const tile = t.root.getBoundingClientRect(), m = t.menu.getBoundingClientRect();
      t.menu.style.left = `${Math.max(4, Math.min(x - tile.left, tile.width - m.width - 4))}px`;
      t.menu.style.top = `${Math.max(4, Math.min(y - tile.top, tile.height - m.height - 4))}px`;
      t.end.focus();
      draw();
    }

    function closeMenu() {
      if (!picked) return;
      picked = null;
      t.menu.hidden = true;
      draw();
    }

    function result(pick, r) {
      const { name } = pick, n = pick.pids.length;
      if (!r) return { text: `Couldn't end ${name}`, bad: true };
      if (r.own) return { text: "That's Oriel: quit it from the tray icon", bad: true };
      if (r.denied && !r.ended) return { text: `Windows refused to end ${name}`, title: REFUSED, bad: true };
      if (r.denied) return { text: `Ended ${r.ended} of ${n} ${name}; Windows refused ${r.denied}`, title: REFUSED, bad: true };
      if (!r.ended) return { text: `${name} had already exited` };
      return { text: n > 1 ? `Ended ${r.ended} ${name} processes` : `Ended ${name}` };
    }

    t.rows.addEventListener("contextmenu", e => {
      const pick = target(e.target.closest(".r")?.dataset.k ?? "");
      if (pick) openMenu(pick, e.clientX, e.clientY);
    });
    t.end.addEventListener("click", async () => {
      const pick = picked;
      if (!pick) return;
      t.end.disabled = true;
      const r = await window.__TAURI__.core.invoke("end_processes", { pids: pick.pids, seen: pick.seen })
        .catch(e => (console.warn(e), null));
      closeMenu();
      note = { ...result(pick, r), until: Date.now() + NOTE_MS };
      refresh();
    });
    t.menu.addEventListener("keydown", e => {
      if (e.key !== "Escape") return;
      e.stopPropagation(); // or Escape also closes the settings
      closeMenu();
    });
    // Anything else closes it: a click elsewhere, scrolling, leaving the window.
    document.addEventListener("pointerdown", e => { if (!t.menu.contains(e.target)) closeMenu(); }, true);
    addEventListener("blur", closeMenu);

    function syncHead() {
      for (const b of t.head.querySelectorAll("[data-sort]")) {
        const on = b.dataset.sort === sort.by;
        b.classList.toggle("on", on);
        b.dataset.dir = on ? sort.dir : "";
        b.title = on ? "Click to reverse" : `Sort by ${b.textContent.toLowerCase()}`;
      }
      t.view.setAttribute("aria-pressed", tree);
      t.view.title = tree ? "Showing a tree. Click to group by name." : "Grouped by name. Click to show a tree.";
      t.view.setAttribute("aria-label", "Tree view");
    }

    t.head.addEventListener("click", e => {
      const key = e.target.closest("[data-sort]")?.dataset.sort;
      if (!key) return;
      sort = sort.by === key ? { by: key, dir: -sort.dir } : { by: key, dir: COLUMNS[key].dir };
      store(SORT_KEY, JSON.stringify(sort));
      syncHead();
      t.list.scrollTop = 0;
      refresh();
    });
    t.view.addEventListener("click", () => {
      tree = !tree;
      store(VIEW_KEY, tree ? "tree" : "group");
      syncHead();
      t.list.scrollTop = 0;
      refresh();
    });
    t.rows.addEventListener("click", e => {
      const k = e.target.closest(".r.x")?.dataset.k;
      if (!k) return;
      const [set, id] = k.startsWith("g:") ? [opened, k.slice(2)] : [closed, Number(k.slice(2))];
      if (!set.delete(id)) set.add(id);
      refresh();
    });
    q.addEventListener("input", () => {
      needle = q.value.trim().toLowerCase();
      t.list.scrollTop = 0;
      refresh();
    });
    q.addEventListener("keydown", e => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      if (!q.value) return q.blur();
      e.preventDefault();
      q.value = "";
      q.dispatchEvent(new Event("input"));
    });
    t.list.addEventListener("scroll", () => {
      closeMenu();
      if (!queued) { queued = true; requestAnimationFrame(draw); }
    }, { passive: true });
    new ResizeObserver(() => draw()).observe(t.list);
    syncHead();

    return {
      root: t.root,
      update(s) {
        if (!s.procs) return; // history samples leave the list out
        procs = s.procs;
        seen = s.t;
        refresh();
      },
    };
  },
});
}
