// toa.sh console window logic — runs inside its OWN OS window (toa.html).
// Receives every log line via "slipstream://log" (Rust emits them globally),
// renders with timestamps + colors, copies to clipboard, and autosaves the
// whole buffer to a dated file next to the exe.

(function () {
  "use strict";

  const MAX_BUFFER = 2000;

  let buffer = [];
  let copiedTimer = null;
  let autosaveTimer = null;
  let dirty = false;

  const body = document.getElementById("body");
  const countEl = document.getElementById("count");

  const svgCopy = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>';
  const svgCheck = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>';
  const svgMin = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M5 12h14"/></svg>';
  const svgClose = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18 6 6 18M6 6l12 12"/></svg>';

  function setSvg(id, svg) {
    const el = document.getElementById(id);
    if (el) el.innerHTML = svg;
  }
  setSvg("copyBtn", svgCopy);
  setSvg("minBtn", svgMin);
  setSvg("closeBtn", svgClose);

  // ---------- rendering ----------
  function pushLog(level, message) {
    const entry = { time: new Date(), level, message: String(message) };
    buffer.push(entry);
    if (buffer.length > MAX_BUFFER) buffer.splice(0, buffer.length - MAX_BUFFER);

    const empty = body.querySelector(".empty");
    if (empty) empty.remove();

    const stick = body.scrollHeight - body.scrollTop - body.clientHeight < 30;
    body.appendChild(lineFor(entry));
    while (body.childNodes.length > MAX_BUFFER) body.removeChild(body.firstChild);
    if (stick) body.scrollTop = body.scrollHeight;

    countEl.textContent = buffer.length ? String(buffer.length) : "";
    scheduleAutosave();
  }

  function lineFor(entry) {
    const ts = new Date(entry.time.getTime() - entry.time.getTimezoneOffset() * 60000);
    const timeStr = ts.toISOString().slice(11, 23); // HH:MM:SS.mmm

    const line = document.createElement("div");
    line.className = "line";
    const t = document.createElement("span");
    t.className = "ts";
    t.textContent = "[" + timeStr + "]";
    const lv = document.createElement("span");
    lv.className = "lv " + entry.level;
    lv.textContent = entry.level;
    const msg = document.createElement("span");
    msg.textContent = entry.message;
    line.append(t, lv, msg);
    return line;
  }

  function renderBacklog() {
    body.textContent = "";
    for (const e of buffer) body.appendChild(lineFor(e));
    body.scrollTop = body.scrollHeight;
    countEl.textContent = buffer.length ? String(buffer.length) : "";
  }

  // ---------- copy ----------
  function serialize() {
    return buffer.map((l) =>
      l.time.toISOString() + "  [" + l.level.toUpperCase() + "]  " + l.message
    ).join("\n");
  }

  function copyLog() {
    const text = serialize();
    const flash = () => {
      const btn = document.getElementById("copyBtn");
      btn.innerHTML = svgCheck;
      btn.classList.add("copied");
      clearTimeout(copiedTimer);
      copiedTimer = setTimeout(() => { btn.innerHTML = svgCopy; btn.classList.remove("copied"); }, 1000);
    };
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(flash, () => fallbackCopy(text, flash));
    } else {
      fallbackCopy(text, flash);
    }
  }

  function fallbackCopy(text, done) {
    try {
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      document.execCommand("copy");
      document.body.removeChild(ta);
      done();
    } catch (_) {}
  }

  // ---------- autosave (one dated file per run) ----------
  function scheduleAutosave() {
    dirty = true;
    if (autosaveTimer) return;
    autosaveTimer = setTimeout(() => { autosaveTimer = null; autosave(); }, 1500);
  }

  function autosave() {
    if (autosaveTimer) { clearTimeout(autosaveTimer); autosaveTimer = null; }
    if (!dirty || !buffer.length) return;
    dirty = false;
    try {
      window.__TAURI__.core.invoke("save_toa_log", { content: serialize() }).catch(() => {});
    } catch (_) {}
  }

  // ---------- window controls ----------
  function appWin() {
    try { return window.__TAURI__.window.getCurrentWindow(); } catch (_) { return null; }
  }

  document.getElementById("minBtn").addEventListener("click", () => {
    const w = appWin();
    if (w) w.minimize(); else window.blur();
  });
  document.getElementById("closeBtn").addEventListener("click", () => {
    autosave();
    const w = appWin();
    if (w) w.hide(); else window.close();
  });
  document.getElementById("copyBtn").addEventListener("click", copyLog);

  // ---------- logging sources ----------
  function wireRustLogs() {
    try {
      window.__TAURI__.event.listen("slipstream://log", (event) => {
        try {
          const payload = event.payload || {};
          let msg = String(payload.message || "");
          const m = msg.match(/^\[js (log|info|warn|error)\] (.*)$/);
          let level = String(payload.level || "log").toLowerCase();
          if (m) { level = m[1]; msg = m[2]; }
          if (level.startsWith("err")) level = "error";
          else if (level !== "warn" && level !== "info" && level !== "log") level = "rust";
          pushLog(level, msg);
        } catch (_) {}
      });
    } catch (_) {}
  }

  function captureWindowErrors() {
    window.addEventListener("error", (e) => {
      pushLog("error", "window.onerror: " + e.message + " @ " + e.filename + ":" + e.lineno + ":" + e.colno);
    });
    window.addEventListener("unhandledrejection", (e) => {
      let r = e.reason;
      if (r && r.message) r = r.message;
      pushLog("error", "unhandledrejection: " + r);
    });
  }

  function loadBacklog() {
    try {
      window.__TAURI__.core.invoke("get_debug_log_backlog", {}).then((lines) => {
        const seq = [];
        for (const raw of lines || []) {
          const m = raw.match(/^\[(\d+)\] \[js (log|info|warn|error)\] (.*)$/);
          seq.push({ time: new Date(), level: m ? m[2] : "log", message: m ? m[3] : raw });
        }
        for (const e of seq) buffer.push(e);
        renderBacklog();
        pushLog("info", "toa.sh ready; console buffer autosaves next to the exe (dated)");
      }).catch(() => {});
    } catch (_) {}
  }

  // ---------- init ----------
  wireRustLogs();
  captureWindowErrors();
  loadBacklog();

  // Flush any buffered logs shortly after boot so early lines land on disk.
  setTimeout(autosave, 4000);
})();