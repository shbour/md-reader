// Installed into each preview page by the app (page markup cannot run
// scripts). The app calls these functions; the page reports scrolling back
// through the "mdr" script message handler.
window.mdr = (() => {
  const content = () => document.getElementById("content");
  const post = (msg) => {
    try {
      window.webkit.messageHandlers.mdr.postMessage(JSON.stringify(msg));
    } catch (_) {}
  };

  // ------------------------------------------------------------- math

  function typesetMath(root) {
    if (!window.katex) return;
    for (const el of root.querySelectorAll("[data-math-style]")) {
      const display = el.dataset.mathStyle === "display";
      const tex = el.textContent;
      let target = el;
      if (el.tagName === "PRE") {
        target = document.createElement("div");
        target.className = "math-display";
        if (el.dataset.sourcepos) target.dataset.sourcepos = el.dataset.sourcepos;
        el.replaceWith(target);
      } else if (display) {
        target = document.createElement("div");
        target.className = "math-display";
        if (el.dataset.sourcepos) target.dataset.sourcepos = el.dataset.sourcepos;
        el.replaceWith(target);
      }
      try {
        katex.render(tex, target, { displayMode: display, throwOnError: false });
      } catch (e) {
        target.textContent = tex;
        target.classList.add("math-error");
      }
    }
  }

  // ---------------------------------------------------------- mermaid

  let forceLight = false;
  let mermaidTheme = null;
  const mermaidCache = new Map();
  let seq = 0;

  const isDark = () =>
    !forceLight && matchMedia("(prefers-color-scheme: dark)").matches;

  function ensureMermaidTheme() {
    const theme = isDark() ? "dark" : "default";
    if (theme !== mermaidTheme) {
      mermaid.initialize({ startOnLoad: false, securityLevel: "strict", theme });
      mermaidTheme = theme;
      mermaidCache.clear();
    }
  }

  async function renderMermaid(root) {
    if (!window.mermaid) return;
    ensureMermaidTheme();
    for (const pre of [...root.querySelectorAll("pre.mermaid")]) {
      const src = pre.textContent;
      const div = document.createElement("div");
      div.className = "mermaid-diagram";
      div.dataset.src = src;
      if (pre.dataset.sourcepos) div.dataset.sourcepos = pre.dataset.sourcepos;
      // Diagrams are cached by source so typing elsewhere doesn't redraw them.
      let svg = mermaidCache.get(src);
      if (!svg) {
        try {
          svg = (await mermaid.render("mdr-mermaid-" + ++seq, src)).svg;
          mermaidCache.set(src, svg);
        } catch (e) {
          div.classList.add("mermaid-error");
          div.textContent = "Mermaid diagram error: " + ((e && e.message) || e);
        }
      }
      if (svg) div.innerHTML = svg;
      // The document may have been re-rendered while we waited.
      if (pre.isConnected) pre.replaceWith(div);
    }
    // mermaid leaves its scratch element behind when a diagram fails.
    document.querySelectorAll('body > [id^="dmdr-mermaid-"]').forEach((e) => e.remove());
  }

  function unrenderMermaid() {
    for (const d of document.querySelectorAll(".mermaid-diagram")) {
      const pre = document.createElement("pre");
      pre.className = "mermaid";
      pre.textContent = d.dataset.src;
      if (d.dataset.sourcepos) pre.dataset.sourcepos = d.dataset.sourcepos;
      d.replaceWith(pre);
    }
  }

  // ------------------------------------------------- scroll <-> source

  // Sync model: at scroll progress f (0 at the top, 1 at the end) each pane
  // looks at the point f of the way down its viewport. The source line
  // there is placed at the same height in the other pane. f = 0 aligns top
  // lines, f = 1 aligns the ends, and in between the line under that point
  // matches exactly, whatever the relative heights of the two panes.

  let totalLines = 1; // lines in the source, sent by the app

  // [line, y] pairs from every element's data-sourcepos, increasing in both.
  function points() {
    const raw = [[1, 0]];
    const sy = window.scrollY;
    for (const el of content().querySelectorAll("[data-sourcepos]")) {
      const m = /^(\d+):\d+-(\d+):\d+$/.exec(el.dataset.sourcepos);
      if (!m) continue;
      const r = el.getBoundingClientRect();
      if (!r.height) continue;
      raw.push([+m[1], r.top + sy]);
      raw.push([+m[2] + 1, r.bottom + sy]);
    }
    raw.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    const out = [];
    for (const p of raw) {
      const last = out[out.length - 1];
      if (last && (p[0] <= last[0] || p[1] < last[1])) continue;
      out.push(p);
    }
    const end = document.documentElement.scrollHeight;
    const last = out[out.length - 1];
    if (totalLines + 1 > last[0] && end >= last[1]) out.push([totalLines + 1, end]);
    return out;
  }

  function interpolate(pts, value, from, to) {
    let i = 0;
    while (i < pts.length - 1 && pts[i + 1][from] <= value) i++;
    const a = pts[i];
    const b = pts[i + 1];
    if (!b || b[from] === a[from]) return a[to];
    return a[to] + ((value - a[from]) / (b[from] - a[from])) * (b[to] - a[to]);
  }

  const maxScroll = () =>
    Math.max(0, document.documentElement.scrollHeight - window.innerHeight);

  let ignoreScrollUntil = 0;
  let editorTarget = null; // last position the editor asked for

  function scrollToLine(line, f, total) {
    if (total) totalLines = total;
    editorTarget = { line, f };
    const y = interpolate(points(), line, 0, 1) - f * window.innerHeight;
    ignoreScrollUntil = performance.now() + 250;
    window.scrollTo(0, Math.min(maxScroll(), Math.max(0, y)));
  }

  let raf = 0;
  window.addEventListener(
    "scroll",
    () => {
      if (performance.now() < ignoreScrollUntil) return;
      editorTarget = null;
      if (raf) return;
      raf = requestAnimationFrame(() => {
        raf = 0;
        const y = window.scrollY;
        const max = maxScroll();
        const f = max > 0 ? Math.min(1, Math.max(0, y / max)) : 0;
        post({ type: "scroll", line: interpolate(points(), y + f * window.innerHeight, 1, 0), f });
      });
    },
    { passive: true }
  );

  // ----------------------------------------------------------- search

  let findText = "";
  function highlight(text) {
    findText = text || "";
    if (!window.CSS || !CSS.highlights) return 0;
    CSS.highlights.delete("mdr-find");
    if (!findText) return 0;
    const needle = findText.toLowerCase();
    const ranges = [];
    const walker = document.createTreeWalker(content(), NodeFilter.SHOW_TEXT);
    for (let n; (n = walker.nextNode()); ) {
      const hay = n.data.toLowerCase();
      if (hay.length !== n.data.length) continue; // case-folding changed offsets
      for (let i = hay.indexOf(needle); i !== -1; i = hay.indexOf(needle, i + needle.length)) {
        const r = new Range();
        r.setStart(n, i);
        r.setEnd(n, i + needle.length);
        ranges.push(r);
      }
    }
    CSS.highlights.set("mdr-find", new Highlight(...ranges));
    return ranges.length;
  }

  // ------------------------------------------------------------ public

  let pending = Promise.resolve();

  // Call after the article's HTML changes.
  function after() {
    const root = content();
    typesetMath(root);
    if (findText) highlight(findText);
    pending = renderMermaid(root).then(() => {
      if (findText) highlight(findText);
      // Diagrams change the layout; keep the editor's position in view.
      if (editorTarget) scrollToLine(editorTarget.line, editorTarget.f);
    });
    return pending;
  }

  async function setForceLight(on) {
    await pending;
    forceLight = on;
    document.documentElement.classList.toggle("force-light", on);
    if (window.mermaid) {
      unrenderMermaid();
      pending = renderMermaid(content());
      await pending;
    }
  }

  async function exportBody() {
    await pending;
    return content().innerHTML;
  }

  // _lineY is only for the app's test hooks.
  const _lineY = (line) => interpolate(points(), line, 0, 1);
  return { after, scrollToLine, highlight, setForceLight, exportBody, _lineY };
})();
