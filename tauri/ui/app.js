// Markdown Reader, Tauri front end. Rendering happens in Rust (mdreader-core);
// this file is the window: documents, the CodeMirror editor, the sandboxed
// preview frame, scroll sync, search, links and export.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const dialog = window.__TAURI__.dialog;
const appWindow = window.__TAURI__.window.getCurrentWindow();
const webview = window.__TAURI__.webview.getCurrentWebview();

const $ = (id) => document.getElementById(id);
const el = {
  tocBtn: $("btn-toc"), back: $("btn-back"), open: $("btn-open"), save: $("btn-save"),
  edit: $("btn-edit"), menuBtn: $("btn-menu"), menu: $("menu"),
  title: $("doc-title"), dir: $("doc-dir"),
  searchbar: $("searchbar"), search: $("search"), count: $("match-count"),
  banner: $("banner"), toc: $("toc"), tocList: $("toc-list"),
  editorPane: $("editor-pane"), editor: $("editor"), splitter: $("splitter"),
  work: $("work"), preview: $("preview"), toast: $("toast"), tabs: $("tabs"),
};

const APP = "Markdown Reader";
const MD_EXT = ["md", "markdown", "mdown", "mkd", "mkdn", "mdx"];
const FILTERS = [{ name: "Markdown", extensions: MD_EXT }, { name: "All files", extensions: ["*"] }];
const EMPTY =
  '<div class="empty-state"><p>Open a Markdown file with <b>Ctrl+O</b>, drop one here, ' +
  "or start a new one with <b>Ctrl+N</b>.</p></div>";

// ------------------------------------------------------------ preferences
// Window size and position are kept by the window-state plugin; these are
// the rest of the layout. localStorage lives in the app's WebView2 profile.

const prefs = { zoom: 1, sidebar: null, split: 0.5 };
try {
  Object.assign(prefs, JSON.parse(localStorage.getItem("mdreader-prefs") || "{}"));
} catch (_) {}
const savePrefs = () => {
  try {
    localStorage.setItem("mdreader-prefs", JSON.stringify(prefs));
  } catch (_) {}
};

// ------------------------------------------------------------------ state

// One per tab. `state` is the tab on screen; the others keep their editor
// state (text, undo history, cursor) and scroll positions until shown.
const newTab = () => ({
  path: null, // null: unsaved document
  name: null,
  dir: null,
  saved: "", // text as last read from / written to disk, with "\n" line breaks
  eol: "\n", // line break written back to disk: kept from the file as opened
  editing: false,
  toc: [],
  hasMath: false,
  hasMermaid: false,
  history: [], // Back, within this tab
  editorState: null, // CodeMirror state while the tab is in the background
  editorScroll: null,
  previewY: 0,
  changedOnDisk: false, // while in the background
  banner: false,
});
const tabs = [newTab()];
let state = tabs[0];

// ----------------------------------------------------------------- editor

const dark = matchMedia("(prefers-color-scheme: dark)");
const themeSlot = new CM.Compartment();
const editorTheme = () =>
  dark.matches ? CM.oneDark : CM.syntaxHighlighting(CM.defaultHighlightStyle, { fallback: true });

const extensions = () => [
  CM.lineNumbers(),
  CM.highlightActiveLineGutter(),
  CM.highlightSpecialChars(),
  CM.history(),
  CM.drawSelection(),
  CM.dropCursor(),
  CM.indentOnInput(),
  CM.bracketMatching(),
  CM.highlightActiveLine(),
  CM.highlightSelectionMatches(),
  CM.EditorView.lineWrapping,
  CM.markdown({ base: CM.markdownLanguage, codeLanguages: CM.languages }),
  CM.search(),
  CM.keymap.of([...CM.defaultKeymap, ...CM.historyKeymap, CM.indentWithTab]),
  themeSlot.of(editorTheme()),
  CM.EditorView.updateListener.of((u) => {
    if (u.docChanged) {
      updateTitle();
      scheduleRender();
    }
  }),
];

const view = new CM.EditorView({ parent: el.editor, state: CM.EditorState.create({ doc: "", extensions: extensions() }) });
const text = () => view.state.doc.toString();
const dirty = () => text() !== state.saved;

// CodeMirror keeps "\n" line breaks, so text from disk is compared in that
// form; a Windows (CRLF) file is written back with CRLF.
const toLF = (s) => s.replace(/\r\n?/g, "\n");
const eolOf = (s) => (s.includes("\r\n") ? "\r\n" : "\n");

// A fresh document: new undo history, cursor at the top.
function resetEditor(doc) {
  view.setState(CM.EditorState.create({ doc, extensions: extensions() }));
  view.scrollDOM.scrollTop = 0;
}

// Same document changed on disk: replace the text, keep the cursor nearby.
function replaceEditorText(doc) {
  const head = Math.min(view.state.selection.main.head, doc.length);
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: doc }, selection: { anchor: head } });
}

dark.addEventListener("change", () => {
  view.dispatch({ effects: themeSlot.reconfigure(editorTheme()) });
  reloadPreview(); // Mermaid draws diagrams for one theme
});

// ------------------------------------------------------- preview frame RPC

let frameReady;
let resolveReady;
let rpcSeq = 0;
const pending = new Map();
let libs = { katex: false, mermaid: false };

function call(cmd, ...args) {
  return frameReady.then(
    () =>
      new Promise((resolve, reject) => {
        const id = ++rpcSeq;
        pending.set(id, { resolve, reject });
        el.preview.contentWindow.postMessage({ id, cmd, args }, "*");
      })
  );
}

window.addEventListener("message", (e) => {
  if (e.source !== el.preview.contentWindow || !e.data) return;
  const m = e.data;
  switch (m.type) {
    case "ready":
      resolveReady();
      break;
    case "reply": {
      const p = pending.get(m.id);
      if (!p) break;
      pending.delete(m.id);
      if (m.error) p.reject(new Error(m.error));
      else p.resolve(m.result);
      break;
    }
    case "scroll":
      followPreview(m.line, m.f);
      break;
    case "image": // Linux: see putImage in core/src/runtime.js
      invoke("read_image", { url: String(m.url) })
        .catch(() => null)
        .then((bytes) => call("putImage", m.url, bytes))
        .catch(() => {});
      break;
    case "link":
      queued(() => followLink(m.href));
      break;
    case "key":
      handleKey(m);
      break;
  }
});

// The preview page serves one folder (relative images resolve against it), so
// documents in the same folder share it, and the libraries it has loaded.
let shellDir;

function ensureShell() {
  return shellDir === state.dir ? frameReady : loadShell();
}

// Load a fresh preview page for the current document's folder.
async function loadShell() {
  shellDir = state.dir;
  for (const p of pending.values()) p.reject(new Error("preview reloaded"));
  pending.clear();
  libs = { katex: false, mermaid: false };
  frameReady = new Promise((r) => (resolveReady = r));
  el.preview.srcdoc = await invoke("preview_page", { dir: state.dir });
  await frameReady;
  await call("setZoom", prefs.zoom);
}

async function reloadPreview() {
  await loadShell();
  await renderNow();
}

// ---------------------------------------------------------------- render

let renderTimer = 0;
let renderSeq = 0;
const abs = (p) => new URL(p, location.href).href;

function scheduleRender() {
  clearTimeout(renderTimer);
  renderTimer = setTimeout(renderNow, 120);
}

async function renderNow() {
  clearTimeout(renderTimer);
  const seq = ++renderSeq;
  const src = text();
  const r = await invoke("render_markdown", { text: src });
  if (seq !== renderSeq) return; // a newer render is on its way
  setToc(r.toc);
  state.hasMath = r.hasMath;
  state.hasMermaid = r.hasMermaid;
  try {
    const want = [];
    if (r.hasMath && !libs.katex) {
      want.push({ url: abs("vendor/katex.min.js") }, { url: abs("vendor/katex.css"), css: true });
      libs.katex = true;
    }
    if (r.hasMermaid && !libs.mermaid) {
      want.push({ url: abs("vendor/mermaid.min.js") });
      libs.mermaid = true;
    }
    if (want.length) await call("loadLibs", want);
    const body = !src.trim() && !state.path && !state.editing ? EMPTY : r.html;
    const drawn = call("update", body);
    if (state.editing) syncPreview();
    if (!el.searchbar.hidden && el.search.value) call("highlight", el.search.value);
    await drawn;
  } catch (e) {
    if (e.message !== "preview reloaded") console.error(e);
  }
}

// ------------------------------------------------------------- documents

let titleQueue = Promise.resolve();

function updateTitle() {
  const d = dirty();
  const name = state.path ? state.name : state.editing || d ? "Untitled" : APP;
  el.title.textContent = (d ? "• " : "") + name;
  // The folder is right-to-left so long paths lose their start, not their
  // end, to the ellipsis; the marks keep "/" and "\" where they belong.
  el.dir.textContent = state.dir ? `\u200e${state.dir}\u200e` : "";
  const title = name === APP ? APP : `${d ? "• " : ""}${name} — ${APP}`;
  // In order: on Windows two IPC calls can finish out of order, and the
  // startup title could then overwrite the document's.
  titleQueue = titleQueue.then(() => appWindow.setTitle(title)).catch(() => {});
  const label = el.tabs.querySelector(".tab.active .tab-name");
  if (label) label.textContent = tabLabel(state);
  el.save.hidden = !(state.editing || d);
  el.save.disabled = !d && !!state.path;
}

function toast(msg) {
  el.toast.textContent = msg;
  el.toast.hidden = false;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => (el.toast.hidden = true), 4000);
}

// Resolve unsaved changes before they would be lost. True means go ahead.
async function confirmDiscard() {
  if (!dirty()) return true;
  const answer = await dialog.message(
    `“${state.name || "Untitled"}” has unsaved changes. Changes you don't save are lost.`,
    { title: "Save changes?", kind: "warning", buttons: { yes: "Save", no: "Don't Save", cancel: "Cancel" } }
  );
  if (answer === "Yes" || answer === "Save") return save();
  return answer === "No" || answer === "Don't Save";
}

async function readDoc(path) {
  try {
    return await invoke("read_document", { path });
  } catch (e) {
    toast(`Could not open ${path}: ${e}`);
    return null;
  }
}

// Show `doc` in the current tab; `record` lets Back return to the one before.
async function showDoc(doc, { fragment = null, record = true } = {}) {
  if (record && state.path && state.path !== doc.path) state.history.push(state.path);
  const firstDoc = !state.path;
  Object.assign(state, {
    path: doc.path, name: doc.name, dir: doc.dir, saved: toLF(doc.text), eol: eolOf(doc.text), changedOnDisk: false,
  });
  resetEditor(state.saved);
  if (doc.lossy) toast("This file is not valid UTF-8; some characters were replaced");
  el.banner.hidden = true;
  el.back.hidden = state.history.length === 0;
  updateTitle();
  renderTabs();
  await ensureShell();
  await renderNow();
  if (firstDoc || record) applySidebarPref();
  syncWatches();
  if (fragment) call("scrollToAnchor", fragment);
  else call("setScrollY", 0).catch(() => {}); // the preview page may be the previous document's
}

// Replace the current tab's document (links, Back, Reload).
async function openPath(path, { fragment = null, record = true, force = false } = {}) {
  if (!force && !(await confirmDiscard())) return false;
  const doc = await readDoc(path);
  if (!doc) return false;
  await showDoc(doc, { fragment, record });
  return true;
}

const IS_WINDOWS = navigator.userAgent.includes("Windows");
const samePath = (a, b) => !!a && !!b && (IS_WINDOWS ? a.toLowerCase() === b.toLowerCase() : a === b);

// Open files in tabs of their own (or the tabs already showing them) and show
// the last. The others wait in the background: drawing a document can take a
// while (diagrams, maths), so only the one on screen is drawn.
async function openFiles(paths) {
  const blank = !state.path && !dirty() ? state : null; // e.g. the start-up window
  let target = null;
  for (const path of paths) {
    let tab = tabs.find((t) => samePath(t.path, path));
    if (!tab) {
      const doc = await readDoc(path);
      if (!doc) continue;
      tab = tabs.find((t) => samePath(t.path, doc.path)) || addTab(doc);
    }
    target = tab;
  }
  if (!target) return false;
  syncWatches();
  await switchTo(target);
  if (blank && blank !== target) tabs.splice(tabs.indexOf(blank), 1);
  renderTabs(); // new tabs, even when the one shown has not changed
  return true;
}

const openInTab = (path) => openFiles([path]);

async function chooseFile() {
  const picked = await dialog.open({ multiple: true, directory: false, filters: FILTERS });
  if (picked) await openFiles([picked].flat());
}

async function newDocument() {
  if (state.path || dirty()) await switchTo(addTab());
  Object.assign(state, { path: null, name: null, dir: null, saved: "", eol: "\n", history: [] });
  resetEditor("");
  el.back.hidden = true;
  el.banner.hidden = true;
  syncWatches();
  renderTabs();
  await ensureShell();
  setEditing(true);
}

// ------------------------------------------------------------------ tabs

const tabDirty = (t) => (t === state ? text() : t.editorState?.doc.toString() ?? t.saved) !== t.saved;
const tabLabel = (t) => (tabDirty(t) ? "• " : "") + (t.name || "Untitled");

// A new tab at the end, holding `doc` if given (shown later).
function addTab(doc = null) {
  const t = newTab();
  if (doc) {
    Object.assign(t, { path: doc.path, name: doc.name, dir: doc.dir, saved: toLF(doc.text), eol: eolOf(doc.text) });
    if (doc.lossy) toast(`${doc.name} is not valid UTF-8; some characters were replaced`);
  }
  tabs.push(t);
  return t;
}

// Keep the on-screen tab's editor and scroll positions while another is shown.
async function stashTab() {
  state.editorState = view.state;
  state.editorScroll = view.scrollSnapshot();
  state.banner = !el.banner.hidden;
  state.previewY = state.editing ? 0 : await call("getScrollY").catch(() => 0);
}

// Put `state`, the current tab, on screen.
async function showTab() {
  const t = state;
  view.setState(t.editorState || CM.EditorState.create({ doc: t.saved, extensions: extensions() }));
  t.editorState = null;
  view.dispatch({ effects: themeSlot.reconfigure(editorTheme()) }); // the theme may have changed meanwhile
  el.banner.hidden = !t.banner;
  el.back.hidden = t.history.length === 0;
  applyEditing();
  if (t.editorScroll) view.dispatch({ effects: t.editorScroll });
  updateTitle();
  renderTabs();
  await ensureShell();
  await renderNow();
  if (!t.editing) call("setScrollY", t.previewY).catch(() => {});
  applySidebarPref();
  if (!el.searchbar.hidden) refreshSearch(false);
  if (t.editing) view.focus();
  if (t.changedOnDisk) {
    t.changedOnDisk = false;
    checkDisk();
  }
}

async function switchTo(tab) {
  if (tab === state || !tabs.includes(tab)) return;
  await stashTab();
  state = tab;
  await showTab();
}

function stepTab(by) {
  const i = tabs.indexOf(state);
  return switchTo(tabs[(i + by + tabs.length) % tabs.length]);
}

// Closing the last tab closes the window, which asks about unsaved changes.
async function closeTab(tab = state) {
  if (tabs.length === 1) return appWindow.close();
  if (tabDirty(tab)) {
    await switchTo(tab); // show what would be lost
    if (!(await confirmDiscard())) return;
  }
  const i = tabs.indexOf(tab);
  tabs.splice(i, 1);
  syncWatches();
  if (tab === state) {
    state = tabs[Math.min(i, tabs.length - 1)];
    await showTab();
  } else {
    renderTabs();
  }
}

function renderTabs() {
  el.tabs.hidden = tabs.length < 2;
  el.tabs.replaceChildren(
    ...tabs.map((t, i) => {
      const tab = document.createElement("div");
      tab.className = t === state ? "tab active" : "tab";
      tab.setAttribute("role", "tab");
      tab.setAttribute("aria-selected", String(t === state));
      tab.dataset.index = i;
      tab.title = t.path || "Untitled";
      const name = document.createElement("span");
      name.className = "tab-name";
      name.textContent = tabLabel(t);
      const close = document.createElement("button");
      close.className = "tab-close";
      close.title = "Close (Ctrl+W)";
      close.setAttribute("aria-label", `Close ${t.name || "Untitled"}`);
      close.innerHTML = '<svg viewBox="0 0 20 20"><path d="M6 6l8 8M14 6l-8 8"/></svg>';
      tab.append(name, close);
      return tab;
    })
  );
  el.tabs.querySelector(".tab.active")?.scrollIntoView({ block: "nearest", inline: "nearest" });
}

const tabAt = (e) => tabs[e.target.closest(".tab")?.dataset.index];
el.tabs.addEventListener("click", (e) => {
  const t = tabAt(e);
  if (t) queued(() => (e.target.closest(".tab-close") ? closeTab(t) : switchTo(t)));
});
el.tabs.addEventListener("auxclick", (e) => {
  const t = tabAt(e);
  if (t && e.button === 1) queued(() => closeTab(t));
});
el.tabs.addEventListener(
  "wheel",
  (e) => {
    if (!e.deltaY || e.shiftKey) return;
    el.tabs.scrollLeft += e.deltaY;
    e.preventDefault();
  },
  { passive: false }
);
el.tabs.addEventListener("mousedown", (e) => {
  if (e.button === 1) e.preventDefault(); // no autoscroll on middle click
});

function syncWatches() {
  const paths = [...new Set(tabs.map((t) => t.path).filter(Boolean))];
  invoke("watch_files", { paths }).catch((e) => console.warn("watch:", e));
}

// Tab changes run one at a time, in order: each one swaps what is on screen.
let tabQueue = Promise.resolve();
function queued(fn) {
  tabQueue = tabQueue.then(fn).catch((e) => toast(String(e.message || e)));
  return tabQueue;
}

async function save() {
  if (!state.path) return saveAs();
  return writeTo(state.path);
}

async function saveAs() {
  let path = await dialog.save({ defaultPath: state.path || "Untitled.md", filters: FILTERS });
  if (!path) return false;
  if (!/\.[^\\/.]+$/.test(path)) path += ".md";
  const moved = path !== state.path;
  if (!(await writeTo(path))) return false;
  if (moved) {
    state.path = path;
    state.name = path.split(/[\\/]/).pop();
    state.dir = path.slice(0, path.length - state.name.length).replace(/[\\/]$/, "");
    syncWatches();
    updateTitle();
    renderTabs();
    await reloadPreview(); // relative images now resolve against the new folder
  }
  return true;
}

async function writeTo(path) {
  const t = text();
  try {
    await invoke("write_document", { path, text: state.eol === "\n" ? t : t.replace(/\n/g, state.eol) });
  } catch (e) {
    toast(`Could not save: ${e}`);
    return false;
  }
  state.saved = t;
  el.banner.hidden = true;
  updateTitle();
  return true;
}

// The file changed on disk: follow it, unless there are unsaved edits.
let diskTimer = 0;
listen("file-changed", (e) => {
  const tab = tabs.find((t) => t.path === e.payload);
  if (!tab) return;
  if (tab !== state) {
    tab.changedOnDisk = true; // looked at when the tab is shown
    return;
  }
  clearTimeout(diskTimer);
  diskTimer = setTimeout(checkDisk, 250);
});

async function checkDisk() {
  const tab = state;
  if (!tab.path) return;
  let doc;
  try {
    doc = await invoke("read_document", { path: tab.path });
  } catch (_) {
    return; // deleted or mid-write; a later event will follow
  }
  if (tab !== state) {
    tab.changedOnDisk = true; // switched away meanwhile
    return;
  }
  const t = toLF(doc.text);
  if (t === state.saved) return;
  if (dirty()) {
    el.banner.hidden = false;
    return;
  }
  state.saved = t;
  state.eol = eolOf(doc.text);
  replaceEditorText(t);
  updateTitle();
  renderNow();
}

$("banner-reload").addEventListener("click", () => openPath(state.path, { record: false, force: true }));

async function goBack() {
  const prev = state.history.pop();
  if (!prev) return;
  if (!(await openPath(prev, { record: false }))) state.history.push(prev);
  el.back.hidden = state.history.length === 0;
}

// ----------------------------------------------------------------- links

async function followLink(href) {
  let r;
  try {
    r = await invoke("open_link", { href, docPath: state.path });
  } catch (e) {
    toast(`Could not open link: ${e}`);
    return;
  }
  if (r.kind === "missing") toast(`${r.path} does not exist`);
  else if (r.kind === "markdown") {
    if (r.path === state.path) {
      if (r.fragment) call("scrollToAnchor", r.fragment);
    } else {
      // A document already open in another tab is shown there, not twice.
      const open = tabs.find((t) => samePath(t.path, r.path));
      if (open) {
        await switchTo(open);
        if (r.fragment) call("scrollToAnchor", r.fragment);
      } else {
        await openPath(r.path, { fragment: r.fragment });
      }
    }
  }
}

// ------------------------------------------------------------ edit mode

function setEditing(on) {
  state.editing = on;
  applyEditing();
  updateTitle();
  renderNow();
  if (!el.searchbar.hidden) refreshSearch(false);
  if (on) view.focus();
}

function applyEditing() {
  const on = state.editing;
  el.edit.setAttribute("aria-pressed", String(on));
  el.editorPane.hidden = !on;
  el.splitter.hidden = !on;
  applySplit();
}

function applySplit() {
  el.editorPane.style.width = `${(prefs.split * 100).toFixed(2)}%`;
}

el.splitter.addEventListener("pointerdown", (e) => {
  el.splitter.setPointerCapture(e.pointerId);
  el.splitter.classList.add("dragging");
  const box = el.work.getBoundingClientRect();
  const move = (ev) => {
    prefs.split = Math.min(0.85, Math.max(0.15, (ev.clientX - box.left) / box.width));
    applySplit();
  };
  const up = () => {
    el.splitter.classList.remove("dragging");
    el.splitter.removeEventListener("pointermove", move);
    savePrefs();
    syncPreview();
  };
  el.splitter.addEventListener("pointermove", move);
  el.splitter.addEventListener("pointerup", up, { once: true });
});

// ------------------------------------------------------------ scroll sync
// Same model as the GTK app (see core/src/runtime.js): at scroll progress f,
// the source line f of the way down one pane is placed at the same height
// in the other, so tops align at the start and ends align at the finish.

let ignoreEditorScrollUntil = 0;
let syncQueued = false;

view.scrollDOM.addEventListener(
  "scroll",
  () => {
    if (performance.now() < ignoreEditorScrollUntil || syncQueued) return;
    syncQueued = true;
    requestAnimationFrame(() => {
      syncQueued = false;
      syncPreview();
    });
  },
  { passive: true }
);

function syncPreview() {
  if (!state.editing) return;
  const sd = view.scrollDOM;
  const max = sd.scrollHeight - sd.clientHeight;
  const f = max > 0 ? Math.min(1, Math.max(0, sd.scrollTop / max)) : 0;
  const h = sd.getBoundingClientRect().top + f * sd.clientHeight - view.documentTop;
  const block = view.lineBlockAtHeight(h);
  const line = view.state.doc.lineAt(block.from).number;
  const frac = block.height > 0 ? Math.min(1, Math.max(0, (h - block.top) / block.height)) : 0;
  call("scrollToLine", line + frac, f, view.state.doc.lines).catch(() => {});
}

function followPreview(line, f) {
  if (!state.editing) return;
  const sd = view.scrollDOM;
  const doc = view.state.doc;
  const n = Math.min(doc.lines, Math.max(1, Math.floor(line)));
  const block = view.lineBlockAt(doc.line(n).from);
  const frac = line >= doc.lines + 1 ? 1 : line - Math.floor(line);
  const docTop = view.documentTop - sd.getBoundingClientRect().top + sd.scrollTop;
  const target = docTop + block.top + frac * block.height - f * sd.clientHeight;
  ignoreEditorScrollUntil = performance.now() + 250;
  sd.scrollTop = Math.min(sd.scrollHeight - sd.clientHeight, Math.max(0, target));
}

// -------------------------------------------------------------- contents

let shownToc = "[]";

function setToc(toc) {
  state.toc = toc;
  const key = JSON.stringify(toc);
  if (key === shownToc) return; // tabs share the list: compare with what it shows
  shownToc = key;
  el.tocList.replaceChildren();
  const min = Math.min(...toc.map((h) => h.level), 6);
  toc.forEach((h) => {
    const li = document.createElement("li");
    const b = document.createElement("button");
    b.textContent = h.text;
    b.title = h.text;
    b.style.paddingLeft = `${10 + 14 * (h.level - min)}px`;
    if (h.level === min) b.className = "top";
    b.addEventListener("click", () => {
      call("scrollToAnchor", h.anchor);
      if (state.editing) {
        const pos = view.state.doc.line(Math.min(h.line, view.state.doc.lines)).from;
        view.dispatch({ selection: { anchor: pos }, effects: CM.EditorView.scrollIntoView(pos, { y: "start" }) });
      }
    });
    li.append(b);
    el.tocList.append(li);
  });
}

function showSidebar(on) {
  el.toc.hidden = !on;
  el.tocBtn.setAttribute("aria-pressed", String(on));
}

// The user's last choice; without one, show it for documents with a few headings.
function applySidebarPref() {
  const n = state.toc.length;
  showSidebar(prefs.sidebar === null ? n >= 3 : prefs.sidebar && n > 0);
}

function toggleSidebar() {
  prefs.sidebar = el.toc.hidden;
  savePrefs();
  showSidebar(prefs.sidebar);
}

// ---------------------------------------------------------------- search
// Reading: steps through matches in the preview. Editing: steps through the
// editor (so the cursor lands on the text to change) and highlights the
// preview, which follows through scroll sync.

function openSearch() {
  el.searchbar.hidden = false;
  el.search.focus();
  el.search.select();
  if (el.search.value) refreshSearch(false);
}

function closeSearch() {
  el.searchbar.hidden = true;
  view.dispatch({ effects: CM.setSearchQuery.of(new CM.SearchQuery({ search: "" })) });
  call("highlight", "").catch(() => {});
  el.count.textContent = "";
  if (state.editing) view.focus(); // the match stays selected, ready to type over
}

function editorQuery() {
  return new CM.SearchQuery({ search: el.search.value, caseSensitive: false, literal: true });
}

function editorMatchLabel() {
  const q = editorQuery();
  if (!q.valid) return "";
  const sel = view.state.selection.main;
  let total = 0;
  let index = 0;
  const cursor = q.getCursor(view.state);
  for (let m = cursor.next(); !m.done; m = cursor.next()) {
    total++;
    if (m.value.from === sel.from && m.value.to === sel.to) index = total;
    if (total >= 10000) break;
  }
  el.search.classList.toggle("error", total === 0);
  if (!total) return "No matches";
  return index ? `${index} of ${total}` : `${total} matches`;
}

async function refreshSearch(jump) {
  const t = el.search.value;
  el.search.classList.remove("error");
  const shown = call("highlight", t).catch(() => 0);
  if (state.editing) {
    view.dispatch({ effects: CM.setSearchQuery.of(editorQuery()) });
    if (jump && t) {
      // Incremental: search from where the current match starts.
      const from = view.state.selection.main.from;
      view.dispatch({ selection: { anchor: from } });
      CM.findNext(view);
    }
    el.count.textContent = t ? editorMatchLabel() : "";
  } else {
    const n = await shown;
    if (!t) el.count.textContent = "";
    else if (jump && n) stepSearch(true);
    else {
      el.count.textContent = n ? `${n} matches` : "No matches";
      el.search.classList.toggle("error", !n);
    }
  }
}

async function stepSearch(forward) {
  if (!el.search.value) return;
  if (state.editing) {
    (forward ? CM.findNext : CM.findPrevious)(view);
    el.count.textContent = editorMatchLabel();
  } else {
    const r = await call("findStep", forward);
    el.count.textContent = r.total ? `${r.index} of ${r.total}` : "No matches";
    el.search.classList.toggle("error", !r.total);
  }
}

el.search.addEventListener("input", () => refreshSearch(true));
el.search.addEventListener("keydown", (e) => {
  if (e.key === "Enter") {
    e.preventDefault();
    stepSearch(!e.shiftKey);
  } else if (e.key === "Escape") {
    e.preventDefault();
    closeSearch();
  }
});
$("find-next").addEventListener("click", () => stepSearch(true));
$("find-prev").addEventListener("click", () => stepSearch(false));
$("find-close").addEventListener("click", closeSearch);

// ------------------------------------------------------ print and export

async function withLightPage(fn) {
  await call("setForceLight", true);
  try {
    return await fn();
  } finally {
    await call("setForceLight", false);
  }
}

// window.print() in the preview; WebView2's dialog includes "Save as PDF".
// The page title names the PDF and fills the page header.
async function printPreview() {
  document.title = (state.name || "Untitled").replace(/\.[^.]+$/, "");
  try {
    return await withLightPage(() => call("print"));
  } finally {
    document.title = APP;
  }
}

async function exportHtml() {
  const stem = (state.name || "Untitled").replace(/\.[^.]+$/, "");
  const dest = await dialog.save({ defaultPath: `${stem}.html`, filters: [{ name: "HTML page", extensions: ["html", "htm"] }] });
  if (dest) await exportHtmlTo(dest);
}

async function exportHtmlTo(dest) {
  const stem = (state.name || "Untitled").replace(/\.[^.]+$/, "");
  const body = await withLightPage(() => call("exportBody"));
  const title = state.toc[0]?.text || stem;
  try {
    await invoke("export_html", { dest, title, body, hasMath: state.hasMath, docDir: state.dir });
    toast(`Exported ${dest}`);
  } catch (e) {
    toast(`Export failed: ${e}`);
  }
}

// ------------------------------------------------------------------ zoom

function setZoom(z) {
  prefs.zoom = Math.round(Math.min(3, Math.max(0.5, z)) * 100) / 100;
  savePrefs();
  document.documentElement.style.setProperty("--editor-font-size", `${14 * prefs.zoom}px`);
  call("setZoom", prefs.zoom).catch(() => {});
}

// --------------------------------------------------------- menu & keys

async function about() {
  const version = await invoke("app_version");
  await dialog.message(
    `Markdown Reader ${version}\nRead, edit and preview Markdown files.\n\n` +
      "MIT licence. Includes KaTeX (© Khan Academy and contributors), Mermaid (© Knut Sveidqvist) " +
      "and CodeMirror (© Marijn Haverbeke and others), all MIT-licensed.",
    { title: "About Markdown Reader", kind: "info" }
  );
}

const actions = {
  new: newDocument,
  open: chooseFile,
  save,
  "save-as": saveAs,
  print: printPreview,
  "export-html": exportHtml,
  find: openSearch,
  "zoom-in": () => setZoom(prefs.zoom * 1.1),
  "zoom-out": () => setZoom(prefs.zoom / 1.1),
  "zoom-reset": () => setZoom(1),
  about,
  edit: () => setEditing(!state.editing),
  toc: toggleSidebar,
  back: goBack,
  close: () => closeTab(),
  "next-tab": () => stepTab(1),
  "prev-tab": () => stepTab(-1),
};

function run(name) {
  closeMenu();
  queued(() => actions[name]());
}

function closeMenu() {
  el.menu.hidden = true;
  el.menuBtn.setAttribute("aria-expanded", "false");
}

el.menuBtn.addEventListener("click", (e) => {
  e.stopPropagation();
  el.menu.hidden = !el.menu.hidden;
  el.menuBtn.setAttribute("aria-expanded", String(!el.menu.hidden));
});
el.menu.addEventListener("click", (e) => {
  const b = e.target.closest("button[data-action]");
  if (b) run(b.dataset.action);
});
document.addEventListener("click", (e) => {
  if (!el.menu.hidden && !e.target.closest(".menu-wrap")) closeMenu();
});
el.open.addEventListener("click", () => run("open"));
el.save.addEventListener("click", () => run("save"));
el.edit.addEventListener("click", () => run("edit"));
el.tocBtn.addEventListener("click", () => run("toc"));
el.back.addEventListener("click", () => run("back"));

// Returns true when the key was ours. `k` is a KeyboardEvent or a shortcut
// forwarded from the preview frame ({key, ctrl, shift, alt}).
function handleKey(k) {
  const ctrl = k.ctrl ?? (k.ctrlKey || k.metaKey);
  const shift = k.shift ?? k.shiftKey;
  const alt = k.alt ?? k.altKey;
  const key = k.key.length === 1 ? k.key.toLowerCase() : k.key;
  const map = ctrl
    ? {
        o: "open", n: "new", s: shift ? "save-as" : "save", e: "edit", f: "find", p: "print",
        w: "close", "=": "zoom-in", "+": "zoom-in", "-": "zoom-out", 0: "zoom-reset",
      }
    : alt
      ? { ArrowLeft: "back" }
      : { F9: "toc", F3: shift ? "find-prev" : "find-next" };
  if (ctrl && (key === "Tab" || key === "PageDown" || key === "PageUp")) {
    run(key === "PageUp" || (key === "Tab" && shift) ? "prev-tab" : "next-tab");
    return true;
  }
  if (ctrl && key === "g") {
    stepSearch(!shift);
    return true;
  }
  if (key === "Escape") {
    if (!el.menu.hidden) closeMenu();
    else if (!el.searchbar.hidden) closeSearch();
    return true;
  }
  if ((ctrl && key === "r") || key === "F5") return true; // never reload the app page
  const name = map[key];
  if (name === "find-next" || name === "find-prev") {
    stepSearch(name === "find-next");
    return true;
  }
  if (!name) return false;
  run(name);
  return true;
}

document.addEventListener("keydown", (e) => {
  if (handleKey(e)) e.preventDefault();
});

// The browser's own menu (Back, Reload, Print…) would act on the app page.
document.addEventListener("contextmenu", (e) => {
  if (!e.target.closest(".cm-editor, input")) e.preventDefault();
});

// ------------------------------------------------- window integration

// Ask about each tab with unsaved changes, showing it first.
appWindow.onCloseRequested((event) => {
  if (!tabs.some(tabDirty)) return;
  event.preventDefault();
  queued(async () => {
    for (const t of [...tabs]) {
      if (!tabDirty(t)) continue;
      await switchTo(t);
      if (!(await confirmDiscard())) return;
    }
    await appWindow.destroy();
  });
});

webview.onDragDropEvent((e) => {
  if (e.payload.type !== "drop") return;
  queued(async () => {
    for (const path of e.payload.paths) await openInTab(path);
  });
});

// A second launch (e.g. double-clicking another .md file) sends its file here.
listen("open-file", (e) => queued(() => openInTab(e.payload)));

// For tests driving a debug build (see devdrive in main.rs).
window.mdrApp = {
  view, call, openPath, setEditing, refreshSearch, stepSearch, openSearch, followLink, goBack,
  writeTo, exportHtmlTo, dirty, tabs, openInTab, openFiles, switchTo, closeTab, queued,
  get state() {
    return state;
  },
};

// ----------------------------------------------------------------- start

(async () => {
  document.documentElement.style.setProperty("--editor-font-size", `${14 * prefs.zoom}px`);
  applySplit();
  updateTitle();
  const file = await invoke("startup_file");
  if (!file || !(await openPath(file))) {
    await loadShell();
    await renderNow();
  }
})();
