const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let store = {
  games: [],
  settings: { blocklist: [], blocklist_off: [], boost_power_plan: true },
  scan_folders: [],
};
let debugLogging = false;
function setDebugLogging(on) {
  debugLogging = !!on;
  try { invoke("debug_log", { level: "set", message: "debug_logging=" + debugLogging }); } catch (_) {}
}

// ---------- frontend debug logging ----------
function jslog(level, message) {
  if (!debugLogging) return;
  try {
    invoke("debug_log", { level, message: String(message) });
  } catch (_) {}
}
window.addEventListener("error", (e) => {
  jslog("error", `window.onerror: ${e.message} @ ${e.filename}:${e.lineno}:${e.colno}`);
});
window.addEventListener("unhandledrejection", (e) => {
  jslog("error", `unhandledrejection: ${e.reason}`);
});
const origLog = console.log;
const origInfo = console.info;
const origWarn = console.warn;
const origError = console.error;
console.log = (...a) => { origLog(...a); jslog("log", a.map((x) => typeof x === "string" ? x : JSON.stringify(x)).join(" ")); };
console.info = (...a) => { origInfo(...a); jslog("info", a.map((x) => typeof x === "string" ? x : JSON.stringify(x)).join(" ")); };
console.warn = (...a) => { origWarn(...a); jslog("warn", a.map((x) => typeof x === "string" ? x : JSON.stringify(x)).join(" ")); };
console.error = (...a) => { origError(...a); jslog("error", a.map((x) => typeof x === "string" ? x : JSON.stringify(x)).join(" ")); };

jslog("info", "frontend boot: main.js loaded");

// ---------- modals: close helpers ----------
function closeModals() {
  document.querySelectorAll(".modal").forEach((m) => {
    m.hidden = true;
  });
}
document.querySelectorAll(".modal").forEach((m) => {
  m.addEventListener("pointerdown", (e) => {
    if (e.target === m) m.hidden = true;
  });
});
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") closeModals();
  const isF12 = e.key === "F12";
  const isQuote = e.key === "`" && (e.ctrlKey || e.metaKey);
  if (isF12 || isQuote) {
    e.preventDefault();
    invoke("toggle_toa").catch(() => {});
  }
});
document.addEventListener("contextmenu", (e) => e.preventDefault());

let status = {
  running: [],
  boosted: false,
  closed_processes: [],
};

// ---------- themes ----------
const THEMES = [
  { id: "slipstream", label: "Slipstream" },
  { id: "steam", label: "Steam" },
  { id: "epic", label: "Epic" },
  { id: "gog", label: "GOG" },
  { id: "xbox", label: "Xbox" },
  { id: "origin", label: "Origin" },
];

function currentTheme() {
  const t = store.settings.theme;
  return THEMES.some((x) => x.id === t) ? t : "steam";
}

function applyTheme(theme) {
  document.documentElement.dataset.theme = theme || "steam";
}

function renderThemeGrid() {
  const grid = document.getElementById("themeGrid");
  grid.innerHTML = "";
  const cur = currentTheme();
  for (const t of THEMES) {
    const card = document.createElement("div");
    card.className = "theme-card" + (t.id === cur ? " selected" : "");
    card.dataset.theme = t.id;
    card.innerHTML = `
      <div class="theme-swatch"><span class="t-bar"></span></div>
      <div class="theme-name">${escapeHtml(t.label)}</div>
      <span class="theme-check">Active</span>
    `;
    card.addEventListener("click", async () => {
      store.settings.theme = t.id;
      applyTheme(t.id);
      renderThemeGrid();
      try {
        store = await invoke("save_settings", { settings: store.settings });
        jslog("info", `theme set to ${t.id} and saved`);
      } catch (err) {
        jslog("error", `theme save failed: ${err}`);
      }
    });
    grid.appendChild(card);
  }
}

// ---------- tabs ----------
document.querySelectorAll(".tab").forEach((tab) => {
  tab.addEventListener("click", () => {
    document.querySelectorAll(".tab").forEach((t) => {
      t.classList.remove("active");
      t.setAttribute("aria-selected", "false");
    });
    tab.classList.add("active");
    tab.setAttribute("aria-selected", "true");
    document.querySelectorAll(".panel").forEach((p) => p.classList.remove("active"));
    document.getElementById(`panel-${tab.dataset.tab}`).classList.add("active");
  });
});

// ---------- settings sub-tabs ----------
document.querySelectorAll(".sub-tab").forEach((tab) => {
  tab.addEventListener("click", () => {
    document.querySelectorAll(".sub-tab").forEach((t) => {
      t.classList.remove("active");
      t.setAttribute("aria-selected", "false");
    });
    tab.classList.add("active");
    tab.setAttribute("aria-selected", "true");
    document.querySelectorAll(".sub-panel").forEach((p) => p.classList.remove("active"));
    document.getElementById(`sub-${tab.dataset.sub}`).classList.add("active");
  });
});

// ---------- status ----------
function setStatus() {
  const pill = document.getElementById("statusPill");
  const text = document.getElementById("statusText");
  pill.classList.toggle("active", status.boosted);
  text.textContent = status.boosted ? "Boosting" : "Idle";

  const strip = document.getElementById("actionStrip");
  const stripText = document.getElementById("actionStripText");
  if (status.boosted) {
    strip.hidden = false;
    const closed = status.closed_processes || [];
    stripText.textContent =
      closed.length > 0
        ? `Game mode active — closed: ${closed.join(", ")}`
        : "Game mode active";
    stripText.style.color = "var(--ok)";
  } else {
    strip.hidden = true;
  }
}

// ---------- window controls ----------
function wireWindowControls() {
  const appWin = window.__TAURI__ && window.__TAURI__.window && window.__TAURI__.window.getCurrentWindow();
  if (!appWin) return;
  document.getElementById("winMin").addEventListener("click", () => appWin.minimize());
  document.getElementById("winMax").addEventListener("click", () => appWin.toggleMaximize());
  document.getElementById("winClose").addEventListener("click", () => appWin.close());
}

// ---------- games ----------
function isRunning(id) {
  return status.running.includes(id);
}

function folderLabel(folder) {
  const parts = folder.replace(/[\\/]+$/, "").split(/[\\/]/);
  return parts[parts.length - 1] || folder;
}

function folderGameCount(folder) {
  const root = folder.replace(/[\\/]+$/, "").toLowerCase();
  return store.games.filter((g) => {
    const p = g.exe_path.replace(/[\\/]+$/, "").toLowerCase();
    return p === root || p.startsWith(root + "\\") || p.startsWith(root + "/");
  }).length;
}

function renderFolders() {
  const list = store.scan_folders || [];

  // Quick chips in the Games tab.
  const wrap = document.getElementById("scanFolderChips");
  if (!list.length) {
    wrap.hidden = true;
  } else {
    wrap.hidden = false;
    wrap.innerHTML = "";
    for (const folder of store.scan_folders) {
      const chip = document.createElement("div");
      chip.className = "chip";
      chip.innerHTML = `<span>${escapeHtml(folderLabel(folder))}</span>`;
      const rm = document.createElement("button");
      rm.textContent = "✕";
      rm.addEventListener("click", () => removeFolder(folder));
      chip.appendChild(rm);
      wrap.appendChild(chip);
    }
  }

  // Full overview in Settings.
  const grid = document.getElementById("folderGrid");
  const emptyHint = document.getElementById("folderEmptyHint");
  emptyHint.hidden = list.length > 0;
  grid.innerHTML = "";
  for (const folder of store.scan_folders) {
    const card = document.createElement("div");
    card.className = "folder-card";

    const drive = document.createElement("div");
    drive.className = "folder-drive";
    drive.textContent = driveLetter(folder) || "•";
    drive.title = folder;

    const info = document.createElement("div");
    info.className = "folder-info";
    info.innerHTML = `
      <div class="folder-name">${escapeHtml(folderLabel(folder))}</div>
      <div class="folder-path">${escapeHtml(folder)}</div>
      <div class="folder-count">${folderGameCount(folder)} ${folderGameCount(folder) === 1 ? "game" : "games"} found</div>
    `;

    const actions = document.createElement("div");
    actions.className = "folder-actions";

    const foldBtn = document.createElement("button");
    foldBtn.className = "btn ghost small";
    foldBtn.textContent = "Scan again";
    foldBtn.title = "Re-scan this folder for games";
    foldBtn.addEventListener("click", () => scanFolderForGames(folder));
    actions.appendChild(foldBtn);

    const rmBtn = document.createElement("button");
    rmBtn.className = "game-remove";
    rmBtn.textContent = "✕";
    rmBtn.title = "Remove this folder from the list";
    rmBtn.addEventListener("click", () => removeFolder(folder));
    actions.appendChild(rmBtn);

    card.appendChild(drive);
    card.appendChild(info);
    card.appendChild(actions);
    grid.appendChild(card);
  }
}

async function removeFolder(folder) {
  store = await invoke("remove_scan_folder", { folder });
  renderFolders();
}

function driveLetter(folder) {
  const m = folder.match(/^([A-Za-z]):/);
  return m ? m[1].toUpperCase() : "";
}

async function scanFolderForGames(folder) {
  try {
    const results = await invoke("scan_folder", { folder });
    jslog("info", `scan_folder(${folder}) -> ${results.length} candidates`);
    if (results.length) {
      showScanResults(results);
    } else {
      alert("No game executables found in that folder.");
    }
  } catch (err) {
    jslog("error", `scan_folder failed: ${err}`);
    alert(err);
  }
}

function renderGames() {
  const list = document.getElementById("gameList");
  const empty = document.getElementById("emptyState");
  list.innerHTML = "";
  empty.hidden = store.games.length > 0;

  for (const game of store.games) {
    const running = isRunning(game.id);

    const card = document.createElement("div");
    card.className = "game-card" + (running ? " running" : "");

    const iconBox = document.createElement("div");
    iconBox.className = "game-icon" + (running ? " running" : "");
    const img = document.createElement("img");
    img.alt = "";
    img.dataset.exe = game.exe_path;
    if (game.custom_icon) img.dataset.icon = game.custom_icon;
    iconBox.appendChild(img);

    const info = document.createElement("div");
    info.className = "game-info";
    info.innerHTML = `
      <div class="game-name">${escapeHtml(game.name)}</div>
      <div class="game-path">${escapeHtml(game.exe_path)}</div>
      <span class="game-source">${escapeHtml(game.source)}</span>
    `;

    const toggles = document.createElement("div");
    toggles.className = "quick-toggles";
    const toggleDefs = [
      { field: "boost_power", label: "Power plan", off: "Use default" },
      { field: "close_background", label: "Close apps", off: "Use default" },
      { field: "boost_priority", label: "High priority", off: "Off" },
    ];
    for (const t of toggleDefs) {
      const btn = document.createElement("button");
      btn.className = "qt-btn";
      btn.dataset.field = t.field;
      btn.textContent = t.label;
      btn.title = t.off;
      btn.addEventListener("click", async () => {
        const cur = game[t.field];
        const next = cur === true ? false : true;
        game[t.field] = next;
        renderGames();
        try {
          store = await invoke("set_game_override", {
            id: game.id,
            field: t.field,
            value: next,
          });
        } catch (err) {
          jslog("error", `set_game_override failed: ${err}`);
        }
      });
      toggles.appendChild(btn);
    }
    const qts = toggles.querySelectorAll(".qt-btn");
    qts.forEach((b) => {
      b.classList.toggle("on", game[b.dataset.field] === true);
      b.classList.toggle("off", game[b.dataset.field] === false);
    });
    info.appendChild(toggles);

    const actions = document.createElement("div");
    actions.className = "game-actions";

    const watchBtn = document.createElement("button");
    watchBtn.className = "btn watch-btn" + (game.watched ? " on" : "") + (running ? " running" : "");
    watchBtn.textContent = running ? "● Running" : game.watched ? "Watching" : "Watch";
    watchBtn.title = game.watched
      ? "Boost applies while this game is running"
      : "Start watching this game";
    watchBtn.addEventListener("click", async () => {
      store = await invoke("set_watched", { id: game.id, watched: !game.watched });
      renderGames();
    });
    actions.appendChild(watchBtn);

    const optionsBtn = document.createElement("button");
    optionsBtn.className = "btn ghost small";
    optionsBtn.textContent = "Options";
    optionsBtn.title = "Per-game boost settings";
    optionsBtn.addEventListener("click", () => openGameOptions(game));
    actions.appendChild(optionsBtn);

    const removeBtn = document.createElement("button");
    removeBtn.className = "game-remove";
    removeBtn.textContent = "✕";
    removeBtn.title = "Remove";
    removeBtn.addEventListener("click", async () => {
      store = await invoke("remove_game", { id: game.id });
      renderGames();
    });
    actions.appendChild(removeBtn);

    if (running) {
      const live = document.createElement("span");
      live.className = "live-tag";
      live.textContent = "running";
      info.appendChild(live);
    }

    card.appendChild(iconBox);
    card.appendChild(info);
    card.appendChild(actions);
    list.appendChild(card);
  }
  loadGameIcons();
}

const gameIconCache = new Map();
function loadGameIcons() {
  document.querySelectorAll(".game-icon img[data-exe]").forEach((img) => {
    const exe = img.dataset.exe;
    const icon = img.dataset.icon || "";
    delete img.dataset.exe;
    if (img.dataset.icon) delete img.dataset.icon;
    const key = icon ? exe + "\u0001" + icon : exe;
    if (gameIconCache.has(key)) {
      img.src = gameIconCache.get(key) || "";
      return;
    }
    invoke("get_game_icon", { exePath: exe, iconPath: icon || null })
      .then((url) => {
        gameIconCache.set(key, url || "");
        img.src = url || "";
      })
      .catch(() => {
        gameIconCache.set(key, "");
        img.src = "";
      });
  });
}

function escapeHtml(str) {
  const d = document.createElement("div");
  d.textContent = str;
  return d.innerHTML;
}

// Toggle item factory shared by the settings blocklist and the per-game
// keep-open list. `onChange(name, checked)` mutates state; the badge text
// (labelOn/labelOff) flips live as the slider moves.
function makeToggleItem(name, checked, labelOn, labelOff, onChange) {
  const label = document.createElement("label");
  label.className = "tlist-item" + (checked ? " on" : "");
  const box = document.createElement("input");
  box.type = "checkbox";
  box.checked = checked;
  const s = document.createElement("span");
  s.className = "switch";
  s.setAttribute("aria-hidden", "true");
  const nm = document.createElement("span");
  nm.className = "tlist-name";
  nm.title = name;
  nm.textContent = name;
  const st = document.createElement("span");
  st.className = "tlist-state";
  st.textContent = checked ? labelOn : labelOff;
  const refresh = () => {
    label.classList.toggle("on", box.checked);
    st.textContent = box.checked ? labelOn : labelOff;
  };
  box.addEventListener("change", () => {
    refresh();
    if (onChange) onChange(name, box.checked);
  });
  label.appendChild(box);
  label.appendChild(s);
  label.appendChild(nm);
  label.appendChild(st);
  return label;
}

// ---------- per-game options ----------
let activeGameId = null;
const gameOptionsModal = document.getElementById("gameOptionsModal");

function previewIcon() {
  const path = document.getElementById("optIconPath").value.trim();
  const img = document.getElementById("optIconPreview");
  if (!path) {
    img.hidden = true;
    img.removeAttribute("src");
    return;
  }
  invoke("get_game_icon", { exePath: "", iconPath: path })
    .then((url) => {
      if (url && path === document.getElementById("optIconPath").value.trim()) {
        img.src = url;
        img.hidden = false;
      }
    })
    .catch(() => { img.hidden = true; });
}

document.getElementById("optIconPick").addEventListener("click", async () => {
  const p = await invoke("pick_icon");
  if (!p) return;
  document.getElementById("optIconPath").value = p;
  previewIcon();
});

document.getElementById("optIconClear").addEventListener("click", () => {
  document.getElementById("optIconPath").value = "";
  previewIcon();
});

function openGameOptions(game) {
  try {
    activeGameId = game.id;
    document.getElementById("gameOptionsTitle").textContent = `${game.name} — boost options`;
    const nameInput = document.getElementById("optName");
    if (nameInput) nameInput.value = game.name;
    document.getElementById("optIconPath").value = game.custom_icon || "";
    previewIcon();
    const map = {
      boost_power: game.boost_power,
      boost_priority: game.boost_priority,
      close_background: game.close_background,
    };
    for (const [field, val] of Object.entries(map)) {
      const sel = document.querySelector(`#gameOptionsModal [data-field="${field}"]`);
      if (sel) sel.value = val === null || val === undefined ? "" : val ? "1" : "0";
    }
    const watcher = document.querySelector('#gameOptionsModal [data-field="watched"]');
    if (watcher) watcher.value = "keep";

    const keep = new Set((game.keep_open || []).map((k) => k.toLowerCase()));
    const keepList = document.getElementById("keepOpenList");
    keepList.innerHTML = "";
    for (const proc of [...new Set(store.settings.blocklist)]) {
      keepList.appendChild(makeToggleItem(
        proc,
        keep.has(proc.toLowerCase()),
        "Kept open",
        "Will close"
      ));
    }

    gameOptionsModal.hidden = false;
    jslog("info", `openGameOptions(${game.id}) modal hidden=${gameOptionsModal.hidden}`);
  } catch (err) {
    jslog("error", `openGameOptions threw: ${err}`);
  }
}

document.getElementById("gameOptionsClose").addEventListener("click", () => {
  jslog("info", "gameOptionsClose clicked, setting hidden=true");
  gameOptionsModal.hidden = true;
});

document.getElementById("gameOptionsSave").addEventListener("click", async () => {
  try {
    jslog("info", `gameOptionsSave clicked for id=${activeGameId}`);
    const keepOpen = [];
    document.querySelectorAll("#keepOpenList .tlist-item input:checked").forEach((el) => {
      const name = el.closest(".tlist-item").querySelector(".tlist-name").textContent;
      keepOpen.push(name);
    });
    store = await invoke("set_game_options", {
      id: activeGameId,
      name: document.getElementById("optName").value.trim(),
      watched: selValue(document.querySelector('#gameOptionsModal [data-field="watched"]')),
      boostPower: selValue(document.querySelector('#gameOptionsModal [data-field="boost_power"]')),
      boostPriority: selValue(document.querySelector('#gameOptionsModal [data-field="boost_priority"]')),
      closeBackground: selValue(document.querySelector('#gameOptionsModal [data-field="close_background"]')),
      keepOpen,
      customIcon: document.getElementById("optIconPath").value.trim(),
    });
    jslog("info", `set_game_options returned OK, keep_open=${JSON.stringify(keepOpen)}`);
    gameOptionsModal.hidden = true;
    renderGames();
    renderFolders();
  } catch (err) {
    jslog("error", `set_game_options failed: ${err}`);
    alert(err);
  }
});

function selValue(sel) {
  if (!sel) return null;
  const v = sel.value;
  if (v === "" || v === "keep") return null;
  return v === "1";
}

// ---------- steam picker ----------
const steamSelect = document.getElementById("steamSelect");

async function refreshSteamPicker() {
  const found = await invoke("scan_steam_games");
  const candidates = found.filter(
    (g) => !store.games.some(
      (existing) => existing.exe_path.toLowerCase() === g.exe_path.toLowerCase()
    )
  );
  window.__steamCandidates = candidates;

  steamSelect.innerHTML = "";
  const placeholder = document.createElement("option");
  placeholder.value = "";
  if (found.length === 0) {
    placeholder.textContent = "No Steam games detected";
    placeholder.disabled = true;
  } else if (candidates.length === 0) {
    placeholder.textContent = "All detected Steam games are already added";
    placeholder.disabled = true;
  } else {
    placeholder.textContent = `Add a Steam game (${candidates.length})…`;
    placeholder.disabled = true;
  }
  placeholder.selected = true;
  steamSelect.appendChild(placeholder);

  for (const g of candidates) {
    const opt = document.createElement("option");
    opt.value = g.exe_path;
    opt.textContent = g.name;
    steamSelect.appendChild(opt);
  }
}

steamSelect.addEventListener("change", async () => {
  const hit = window.__steamCandidates.find(
    (g) => g.exe_path === steamSelect.value
  );
  if (!hit) return;
  store = await invoke("save_game", { game: hit });
  renderGames();
  await refreshSteamPicker();
});

document.getElementById("refreshSteamBtn").addEventListener("click", () => {
  steamSelect.innerHTML = '<option value="" disabled>Detecting…</option>';
  refreshSteamPicker();
});

// ---------- folder scanning ----------
const scanResultsModal = document.getElementById("scanResultsModal");

document.getElementById("scanFolderBtn").addEventListener("click", async () => {
  const folder = await invoke("pick_folder");
  if (!folder) return;
  store = await invoke("add_scan_folder", { folder });
  renderFolders();
  await scanFolderForGames(folder);
});

document.getElementById("addFolderBtn").addEventListener("click", async () => {
  const folder = await invoke("pick_folder");
  if (!folder) return;
  store = await invoke("add_scan_folder", { folder });
  renderFolders();
  jslog("info", `library folder added: ${folder}`);
});

function showScanResults(games) {
  const list = document.getElementById("scanResultsList");
  list.innerHTML = "";
  for (const g of games) {
    const hit = store.games.some(
      (existing) => existing.exe_path.toLowerCase() === g.exe_path.toLowerCase()
    );
    if (hit) continue;
    const item = document.createElement("div");
    item.className = "picker-item scan-item";
    item.innerHTML = `<span><strong>${escapeHtml(g.name)}</strong><br><span class="scan-exe">${escapeHtml(g.exe_path)}</span></span><button class="btn ghost small">Add</button>`;
    item.querySelector("button").addEventListener("click", async () => {
      store = await invoke("save_game", { game: g });
      renderGames();
      renderFolders();
      item.querySelector("button").textContent = "Added";
      item.querySelector("button").disabled = true;
    });
    list.appendChild(item);
  }
  if (!list.children.length) {
    const none = document.createElement("div");
    none.className = "picker-item";
    none.textContent = "Everything in this folder is already added.";
    list.appendChild(none);
  }
  scanResultsModal.hidden = false;
}

document.getElementById("scanResultsClose").addEventListener("click", () => {
  scanResultsModal.hidden = true;
});

// ---------- add game modal ----------
const addGameModal = document.getElementById("addGameModal");
document.getElementById("addCustomBtn").addEventListener("click", () => {
  document.getElementById("gameNameInput").value = "";
  document.getElementById("gameExeInput").value = "";
  document.getElementById("gameArgsInput").value = "";
  addGameModal.hidden = false;
});
document.getElementById("cancelAddGame").addEventListener("click", () => {
  addGameModal.hidden = true;
});
document.getElementById("browseExeBtn").addEventListener("click", async () => {
  const path = await invoke("pick_exe");
  if (path) {
    document.getElementById("gameExeInput").value = path;
    if (!document.getElementById("gameNameInput").value) {
      const base = path.split(/[\\/]/).pop().replace(/\.exe$/i, "");
      document.getElementById("gameNameInput").value = base;
    }
  }
});
document.getElementById("confirmAddGame").addEventListener("click", async () => {
  const name = document.getElementById("gameNameInput").value.trim();
  const exe = document.getElementById("gameExeInput").value.trim();
  const args = document.getElementById("gameArgsInput").value.trim();
  if (!name || !exe) {
    alert("Name and executable are both required.");
    return;
  }
  store = await invoke("save_game", {
    game: {
      id: "", name, exe_path: exe, args, source: "custom",
      watched: false, boost_power: null, boost_priority: null, close_background: null,
    },
  });
  addGameModal.hidden = true;
  renderGames();
});

// ---------- settings ----------
function saveBlocklistState() {
  store = invoke("save_settings", { settings: store.settings })
    .then((s) => { store = s; })
    .catch((e) => jslog("error", `blocklist toggle save failed: ${e}`));
}

function renderBlocklist() {
  const wrap = document.getElementById("blocklist");
  const suspended = new Set((store.settings.blocklist_off || []).map((s) => s.toLowerCase()));
  wrap.innerHTML = "";
  const all = [...new Set([
    ...(store.settings.blocklist || []),
    ...(store.settings.blocklist_off || []),
  ])].sort((a, b) => a.toLowerCase().localeCompare(b.toLowerCase()));

  const setProcState = (proc, closeIt) => {
    const procLow = proc.toLowerCase();
    const on = store.settings.blocklist.filter((p) => p.toLowerCase() !== procLow);
    const off = (store.settings.blocklist_off || []).filter((p) => p.toLowerCase() !== procLow);
    if (closeIt) {
      on.push(proc);
    } else {
      off.push(proc);
    }
    store.settings.blocklist = on;
    store.settings.blocklist_off = off;
    saveBlocklistState();
    renderBlocklist();
  };

  for (const proc of all) {
    const enabled = !suspended.has(proc.toLowerCase());
    wrap.appendChild(makeToggleItem(
      proc,
      enabled,
      "Will close",
      "Kept open",
      (_n, checked) => setProcState(proc, checked)
    ));
  }
}

document.getElementById("addProcessBtn").addEventListener("click", () => {
  const input = document.getElementById("newProcessInput");
  const val = input.value.trim();
  if (!val) return;
  const normalized = val.toLowerCase().endsWith(".exe") ? val : `${val}.exe`;
  if (!store.settings.blocklist.some((p) => p.toLowerCase() === normalized.toLowerCase())) {
    store.settings.blocklist.push(normalized);
  }
  input.value = "";
  renderBlocklist();
});

const pickerModal = document.getElementById("pickerModal");
let pickerProcs = [];
document.getElementById("pickRunningBtn").addEventListener("click", async () => {
  pickerProcs = await invoke("list_running_processes");
  document.getElementById("pickerFilter").value = "";
  renderPicker(pickerProcs);
  pickerModal.hidden = false;
});
document.getElementById("pickerClose").addEventListener("click", () => {
  pickerModal.hidden = true;
});
document.getElementById("pickerFilter").addEventListener("input", (e) => {
  const q = e.target.value.toLowerCase();
  renderPicker(pickerProcs.filter((p) => p.toLowerCase().includes(q)));
});

function renderPicker(procs) {
  const list = document.getElementById("pickerList");
  list.innerHTML = "";
  for (const p of procs) {
    const item = document.createElement("div");
    item.className = "picker-item";
    item.textContent = p;
    item.addEventListener("click", () => {
      if (!store.settings.blocklist.some((b) => b.toLowerCase() === p.toLowerCase())) {
        store.settings.blocklist.push(p);
        renderBlocklist();
      }
      pickerModal.hidden = true;
    });
    list.appendChild(item);
  }
}

document.getElementById("saveSettingsBtn").addEventListener("click", async () => {
  store.settings.boost_power_plan = document.getElementById("togglePowerPlan").checked;
  store.settings.boost_priority = document.getElementById("togglePriority").checked;
  store.settings.close_background = document.getElementById("toggleClose").checked;
  setDebugLogging(document.getElementById("toggleDebugLogging").checked);
  store.settings.debug_logging = debugLogging;
  store = await invoke("save_settings", { settings: store.settings });
  renderBlocklist();
});

document.getElementById("copyLogPathBtn").addEventListener("click", async () => {
  const path = await invoke("get_debug_log_path");
  await navigator.clipboard.writeText(path);
  document.getElementById("copyLogPathBtn").textContent = "Copied";
  setTimeout(() => {
    document.getElementById("copyLogPathBtn").textContent = "Copy log path";
  }, 1500);
});

document.getElementById("restoreDefaultsBtn").addEventListener("click", async () => {
  try {
    const defaults = await invoke("default_blocklist");
    store.settings.blocklist_off = (store.settings.blocklist_off || []).filter(
      (p) => !defaults.some((d) => d.toLowerCase() === p.toLowerCase())
    );
    for (const proc of defaults) {
      if (!store.settings.blocklist.some((p) => p.toLowerCase() === proc.toLowerCase())) {
        store.settings.blocklist.push(proc);
      }
    }
    renderBlocklist();
    jslog("info", `restore defaults: blocklist now ${JSON.stringify(store.settings.blocklist)}`);
  } catch (err) {
    jslog("error", `restore defaults failed: ${err}`);
    alert(err);
  }
});

// ---------- windows tweaks ----------
let tweakCache = { tweaks: [], elevated: false };

function renderTweaks() {
  const { tweaks, elevated } = tweakCache;
  const boostList = document.getElementById("boostTweakList");
  const permList = document.getElementById("permanentTweakList");
  boostList.innerHTML = "";
  permList.innerHTML = "";

  const boostTweaks = tweaks.filter((t) => t.group === "boost");
  const permTweaks = tweaks.filter((t) => t.group === "permanent");

  for (const t of boostTweaks) {
    const row = makeToggleItem(t.label, t.appliedOrOn, "On", "Off", async (_n, on) => {
      if (t.admin && !elevated) {
        await requestElevation();
        renderTweaks();
        return;
      }
      try {
        tweakCache.tweaks = await invoke("set_boost_tweak", { id: t.id, on });
      } catch (err) {
        jslog("error", `set_boost_tweak(${t.id}) failed: ${err}`);
        await refreshTweaksData();
      }
      // Re-render from backend truth so the switch always reflects saved state.
      renderTweaks();
    });
    row.classList.add("tweak-row");
    if (t.admin) {
      row.classList.add("admin");
      if (!elevated) {
        row.classList.add("needs-admin");
        row.title = "Requires admin — toggle to request elevation";
      } else {
        row.title = "Requires admin — applied because Slipstream is elevated";
      }
    }
    const help = document.createElement("span");
    help.className = "tweak-help";
    help.textContent = t.help;
    const wrap = document.createElement("div");
    wrap.className = "tweak-wrap";
    wrap.appendChild(row);
    wrap.appendChild(help);
    boostList.appendChild(wrap);
  }

  if (!boostTweaks.length) {
    boostList.innerHTML = '<p class="section-hint" style="margin:0">No boost tweaks.</p>';
  }

  for (const t of permTweaks) {
    const row = document.createElement("div");
    row.className = "tweak-perm" + (t.admin ? " admin" : "");
    const head = document.createElement("div");
    head.className = "tweak-head";

    const label = document.createElement("div");
    label.className = "tweak-title";
    label.innerHTML = `
      ${escapeHtml(t.label)}
      ${t.admin ? '<span class="sh-icon">🛡</span>' : ""}
      ${t.reboot ? '<span class="reboot-chip">reboot</span>' : ""}
    `;
    if (t.tip) {
      const tip = document.createElement("span");
      tip.className = "tweak-info";
      tip.textContent = "?";
      tip.setAttribute("role", "button");
      tip.setAttribute("aria-label", "What do I lose?");
      tip.dataset.tip = t.tip;
      label.appendChild(tip);
    }

    const status = document.createElement("span");
    status.className = "tweak-state" + (t.appliedOrOn ? " applied" : "");
    status.textContent = t.appliedOrOn ? "Applied" : "Not applied";

    const act = document.createElement("div");
    act.className = "tweak-actions";
    const btn = document.createElement("button");
    btn.className = "btn small " + (t.appliedOrOn ? "ghost" : "primary");
    btn.textContent = t.appliedOrOn ? "Revert" : "Apply";
    if (t.admin && !elevated) {
      row.classList.add("needs-admin");
      row.title = "Requires admin — apply will prompt for elevation";
    }
    btn.addEventListener("click", async () => {
      try {
        if (t.appliedOrOn) {
          tweakCache.tweaks = await invoke("revert_permanent_tweak", { id: t.id });
        } else {
          tweakCache.tweaks = await invoke("apply_permanent_tweak", { id: t.id });
        }
      } catch (err) {
        jslog("error", `permanent tweak ${t.id} failed: ${err}`);
        alert(`Could not ${t.appliedOrOn ? "revert" : "apply"} "${t.label}". Reason: ${err}`);
        await refreshTweaksData();
      }
      renderTweaks();
    });
    act.appendChild(btn);
    head.appendChild(label);
    head.appendChild(status);
    head.appendChild(act);

    const help = document.createElement("div");
    help.className = "tweak-help";
    help.textContent = t.help;

    row.appendChild(head);
    row.appendChild(help);
    permList.appendChild(row);
  }
  jslog("info", `renderTweaks: ${boostTweaks.length} boost, ${permTweaks.length} permanent, elevated=${elevated}`);
}

async function requestElevation() {
  const shouldExit = await invoke("request_elevation");
  if (shouldExit) {
    try {
      window.close();
    } catch (_) {
      /* relaunched copy already owns the window title */
    }
  }
  return !shouldExit;
}

async function refreshTweaksData() {
  try {
    const [tweaks, elevated] = await invoke("get_tweaks");
    tweakCache = { tweaks, elevated };
  } catch (err) {
    jslog("error", `get_tweaks failed: ${err}`);
  }
  return tweakCache;
}

async function refreshTweaks() {
  await refreshTweaksData();
  renderTweaks();
}
const detectBanner = document.getElementById("detectBanner");
let detectCandidates = [];
let detectSuppressed = new Set();

async function refreshDetect() {
  try {
    const candidates = await invoke("detect_game_processes");
    const fresh = candidates.filter(
      (c) => !detectSuppressed.has(c.exe_path.toLowerCase()) &&
             !store.games.some((g) => g.exe_path.toLowerCase() === c.exe_path.toLowerCase())
    );
    detectCandidates = fresh;
    if (fresh.length) {
      const first = fresh[0];
      document.getElementById("detectTitle").textContent = "Looks like a game is running";
      const stem = first.name.replace(/\.exe$/i, "");
      document.getElementById("detectSub").textContent = `${stem} — turn on game mode?`;
      detectBanner.hidden = false;
    } else {
      detectBanner.hidden = true;
    }
  } catch (err) {
    jslog("error", `refreshDetect failed: ${err}`);
  }
}

document.getElementById("detectAddBtn").addEventListener("click", async () => {
  const game = detectCandidates[0];
  if (!game) {
    detectBanner.hidden = true;
    return;
  }
  try {
    const entry = {
      ...game,
      watched: true,
      boost_power: true,
      close_background: true,
    };
    store = await invoke("save_game", { game: entry });
    detectSuppressed.add(game.exe_path.toLowerCase());
    detectBanner.hidden = true;
    renderGames();
    refreshDetect();
    jslog("info", `game mode enabled for ${game.name}`);
  } catch (err) {
    jslog("error", `detectAdd failed: ${err}`);
    alert(err);
  }
});

document.getElementById("detectDismissBtn").addEventListener("click", () => {
  if (detectCandidates[0]) {
    detectSuppressed.add(detectCandidates[0].exe_path.toLowerCase());
    const name = detectCandidates[0].name;
    if (!store.settings.dismissed_games.some((n) => n.toLowerCase() === name.toLowerCase())) {
      store.settings.dismissed_games.push(name);
      invoke("save_settings", { settings: store.settings }).then(
        () => jslog("info", `dismissed ${name}`),
        (e) => jslog("error", `dismiss save failed: ${e}`)
      );
    }
  }
  detectBanner.hidden = true;
  refreshDetect().catch(() => {});
});

// ---------- boot ----------
async function boot() {
  store = await invoke("get_store");
  document.getElementById("toggleDebugLogging").checked = !!store.settings.debug_logging;
  setDebugLogging(store.settings.debug_logging);
  document.getElementById("togglePowerPlan").checked = store.settings.boost_power_plan;
  document.getElementById("togglePriority").checked = store.settings.boost_priority;
  document.getElementById("toggleClose").checked = store.settings.close_background;
  renderBlocklist();
  renderFolders();
  renderGames();
  renderThemeGrid();
  applyTheme(currentTheme());
  setStatus();
  wireWindowControls();
  refreshTweaks();

  document.getElementById("toaBtn").addEventListener("click", () => {
    invoke("toggle_toa").catch((e) => jslog("error", `toggle_toa failed: ${e}`));
  });

  invoke("get_debug_log_path").then(
    (p) => { document.getElementById("logPath").textContent = p; },
    () => {}
  );

  refreshSteamPicker().catch((e) => jslog("error", `refreshSteamPicker failed: ${e}`));
  refreshDetect().catch((e) => jslog("error", `refreshDetect at boot failed: ${e}`));
  setInterval(() => {
    refreshDetect().catch((e) => jslog("error", `refreshDetect poll failed: ${e}`));
  }, 5000);

  try {
    const initial = await invoke("get_status");
    status = { ...status, ...initial };
    setStatus();
    renderGames();
  } catch (e) {
    jslog("error", `get_status failed at boot: ${e}`);
  }
}

listen("slipstream://status", (event) => {
  status = { ...status, ...event.payload };
  setStatus();
  renderGames();
});

boot();