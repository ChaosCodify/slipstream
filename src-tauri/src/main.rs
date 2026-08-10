// Slipstream — local, telemetry-free game boost & watch utility.
// Everything here runs on-device. No network calls exist anywhere in this
// binary. The only external processes it ever shells out to are Windows'
// own `powercfg`, `taskkill`, and `powershell`.
//
// Anti-cheat note: powercfg and taskkill close/switch unrelated system
// resources, so the game process itself is never touched. The only action
// that opens a handle to the *game's* process is the per-game "High
// priority" setting, which is why it is opt-in *per game* and off by
// default — games with aggressive kernel anti-cheat (e.g. Tokon/EAC) can
// simply leave it off while other games use it.
//
// Debug logging: this binary logs nearly everything to a text file so
// bugs can be chased after the fact. The log lands in `logs/` at the
// project root when it can find one (walks up from the exe looking for
// `src-tauri/Cargo.toml`), otherwise in `%LOCALAPPDATA%\slipstream\logs\`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use sysinfo::System;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl};
use uuid::Uuid;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x08000000;

// ---------- debug logging ----------

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();
static DEBUG_LOGGING: AtomicBool = AtomicBool::new(false);

fn log_dir() -> PathBuf {
    // Save the log right next to the exe, wherever that exe happens to live
    // (release install folder, target/debug for dev, etc.). The app is
    // installed with admin rights, so it owns its own folder in the common
    // case; if the exe folder is read-only we fall back to the user's
    // local app data.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.to_path_buf();
        }
    }
    if let Some(ad) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(ad).join("slipstream").join("logs");
    }
    std::env::temp_dir()
}

fn log_file() -> PathBuf {
    LOG_PATH
        .get_or_init(|| {
            let dir = log_dir();
            fs::create_dir_all(&dir).ok();
            dir.join("slipstream-debug.log")
        })
        .clone()
}

fn log_msg(msg: &str) {
    // Debug logging is opt-in (setting `debug_logging`, false by default).
    // When off, nothing is written and nothing is mirrored to the toa console.
    if !DEBUG_LOGGING.load(Ordering::Relaxed) {
        return;
    }
    let path = log_file();
    let line = format!("[{}] {}\n", now_stamp(), msg);
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }
    // Mirror every log line to the frontend so the toa.sh debug console can
    // show it live (no-op until the app is running and the handle is set).
    if let Some(app) = APP_HANDLE.get() {
        let payload = serde_json::json!({ "level": "rust", "message": msg });
        let _ = app.emit("slipstream://log", payload);
    }
    #[cfg(debug_assertions)]
    eprintln!("{}", line.trim_end());
}

fn now_stamp() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{ms}")
}

macro_rules! dlog {
    ($($arg:tt)*) => { log_msg(&format!($($arg)*)) };
}

// ---------- data model ----------

#[derive(Serialize, Deserialize, Clone, Debug)]
struct GameEntry {
    id: String,
    name: String,
    exe_path: String,
    args: String,            // unused for now, reserved
    source: String,          // "steam" | "custom"
    #[serde(default)]
    watched: bool,           // watch this game and boost while it runs
    #[serde(default)]
    boost_power: Option<bool>,      // None = use global setting
    #[serde(default)]
    boost_priority: Option<bool>,   // None = use global setting
    #[serde(default)]
    close_background: Option<bool>, // None = use global setting
    #[serde(default)]
    keep_open: Vec<String>,         // blocklisted exes to NOT close for this game
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Settings {
    blocklist: Vec<String>,
    // Blocklisted entries the user has toggled off but kept around for
    // convenience. Only `blocklist` is force-closed by the watcher.
    #[serde(default)]
    blocklist_off: Vec<String>,
    #[serde(default = "default_true")]
    boost_power_plan: bool,   // global default for per-game boost_power
    #[serde(default)]
    boost_priority: bool,     // global default for per-game boost_priority (safe off)
    #[serde(default = "default_true")]
    close_background: bool,   // global default for per-game close_background
    #[serde(default = "default_theme")]
    theme: String,            // UI theme: "steam" | "epic" | "gog" | "xbox" | "origin" | "slipstream"
    // Ids of the per-boost Windows tweaks the user has enabled. They are
    // applied while a watched game is boosting and reverted when it exits.
    #[serde(default)]
    boost_tweaks: Vec<String>,
    // Ids of the "permanent"/one-time tweaks that have been applied to the
    // system (B-group). Tracked so the UI can show an "applied" state and
    // offer an explicit Revert button.
    #[serde(default)]
    applied_tweaks: Vec<String>,
    #[serde(default)]
    dismissed_games: Vec<String>, // game names the user told us not to nag about
    #[serde(default)]
    debug_logging: bool, // opt-in: write the debug log + show toa console mirror
}

fn default_true() -> bool {
    true
}

fn default_theme() -> String {
    "slipstream".into()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            blocklist: vec![
                "Discord.exe".into(),
                "GameBar.exe".into(),
                "GameBarPresenceWriter.exe".into(),
                "GeForceOverlay.exe".into(),
                "NVIDIA Share.exe".into(),
                "RTSS.exe".into(),
                "MSIAfterburner.exe".into(),
                "chrome.exe".into(),
                "firefox.exe".into(),
                "Spotify.exe".into(),
                "EpicGamesLauncher.exe".into(),
                "OneDrive.exe".into(),
                "SteamWebHelper.exe".into(),
            ],
            blocklist_off: vec![],
            boost_power_plan: true,
            boost_priority: false,
            close_background: true,
            theme: default_theme(),
            boost_tweaks: vec![],
            applied_tweaks: vec![],
            dismissed_games: vec![],
            debug_logging: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct Store {
    games: Vec<GameEntry>,
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    scan_folders: Vec<String>, // extra folders to scan for exes
}

// ---------- runtime state ----------

#[derive(Default)]
struct RuntimeState {
    running_game_ids: Vec<String>,
    prior_power_scheme: Option<String>,
    closed_processes: Vec<String>,
    // Per-boost tweaks applied this session (so restore can revert exactly
    // what a launch applied). fso needs the exe list to remove its layer value.
    boost_tweaks_applied: Vec<String>,
    boost_exes: Vec<String>,
}

type SharedState = Arc<Mutex<RuntimeState>>;

// ---------- storage ----------

fn store_path(app: &AppHandle) -> PathBuf {
    let dir = app
        .path()
        .app_data_dir()
        .expect("no app data dir available");
    fs::create_dir_all(&dir).ok();
    dir.join("store.json")
}

fn load_store(app: &AppHandle) -> Store {
    let path = store_path(app);
    let mut store = Store::default();
    if let Ok(raw) = fs::read_to_string(&path) {
        if let Ok(parsed) = serde_json::from_str::<Store>(&raw) {
            store = parsed;
        } else {
            dlog!("store.json at {} failed to parse; using defaults", path.display());
        }
    }
    DEBUG_LOGGING.store(store.settings.debug_logging, Ordering::Relaxed);
    store
}

fn write_store(app: &AppHandle, store: &Store) -> Result<(), String> {
    let path = store_path(app);
    let raw = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    fs::write(path, raw).map_err(|e| e.to_string())
}

// ---------- system / process helpers ----------

fn hidden_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

fn capture_active_scheme() -> Option<String> {
    let output = hidden_command("powercfg")
        .args(["/getactivescheme"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    // "Power Scheme GUID: <guid>  (Name)"
    let guid = text
        .split("GUID:")
        .nth(1)?
        .trim()
        .split_whitespace()
        .next()
        .map(|s| s.to_string());
    dlog!("capture_active_scheme -> {:?} (raw: {})", guid, text.trim());
    guid
}

fn set_scheme(guid_or_alias: &str) {
    let status = hidden_command("powercfg")
        .args(["/s", guid_or_alias])
        .status();
    dlog!("set_scheme({guid_or_alias}) -> {:?}", status.map(|s| s.success()));
}

fn running_process_names() -> HashSet<String> {
    let mut sys = System::new_all();
    sys.refresh_all();
    sys.processes()
        .values()
        .map(|p| p.name().to_string_lossy().to_lowercase())
        .collect()
}

// ---------- game-like process detection ----------
//
// Heuristic "is this a game?" so Slipstream can offer Game Mode without the
// user having to add anything. Two signals:
//   1. The exe lives under a known store install root (Steam/GOG/Epic/…).
//   2. The process stem matches a small curated allowlist of common games.
// We also skip obvious non-games that live in store folders (uninstallers,
// launchers, redists, …).

fn game_install_root_marks() -> Vec<&'static str> {
    vec![
        "steamapps\\common",
        "gog games",
        "epic games",
        "xboxgames",
        "xbox games",
        "battlenet",
        "proton",
    ]
}

const GAME_STEM_HINTS: &[&str] = &[
    "cyberpunk2077", "rdr2", "eldenring", "darktide", "valheim",
    "hollowknight", "terraria", "stardew", "dota2", "cs2", "csgo",
    "valorant", "league", "rocketleague", "warframe", "fortnite",
    "hunt", "tarkov", "destiny2", "gta5", "gta_launcher", "gtav",
    "witcher3", "witcher", "skyrim", "fallout4", "fallout76", "no man's",
];

fn looks_like_game(name: &str, exe: Option<&std::path::Path>) -> bool {
    let stem = exe_file_stem(name);
    // Skip obvious non-games regardless of location.
    let noisy = [
        "unins", "uninstall", "setup", "install", "redist", "crash",
        "crashpad", "elev", "bootstrapper", "updater", "update", "vc_redist",
        "cor_email", "backend", "server", "webhelper", "overlay",
        "steam", "epic", "goggalaxy", "origin", "ubisoft", "discord",
    ];
    if noisy.iter().any(|n| stem.contains(n)) {
        return false;
    }
    // Known game exe name.
    if GAME_STEM_HINTS.iter().any(|h| *h == stem) {
        return true;
    }
    // Lives under a store install root.
    if let Some(p) = exe {
        let lp = p.to_string_lossy().to_lowercase().replace('/', "\\");
        if game_install_root_marks().iter().any(|m| lp.contains(m)) {
            return true;
        }
    }
    false
}

#[allow(clippy::type_complexity)]
fn running_game_processes() -> Vec<(String, Option<PathBuf>)> {
    let mut sys = System::new_all();
    sys.refresh_all();
    sys.processes()
        .values()
        .map(|p| (p.name().to_string_lossy().to_string(), p.exe().map(|x| x.to_path_buf())))
        .collect()
}

fn close_blocklisted(blocklist: &[String], procs: &HashSet<String>, quiet: bool) -> Vec<String> {
    let mut closed = vec![];
    for name in blocklist {
        if procs.contains(&name.to_lowercase()) {
            // taskkill /IM kills every process sharing that image name, which
            // matters for multi-process apps like Discord or Chrome.
            let status = hidden_command("taskkill")
                .args(["/IM", name, "/F", "/T"])
                .status();
            let ok = status.map(|s| s.success()).unwrap_or(false);
            if !quiet {
                dlog!("taskkill /IM {} /F /T -> {}", name, ok);
            }
            if ok {
                closed.push(name.clone());
            }
        } else if !quiet {
            dlog!("blocklisted {} not running, skip", name.to_lowercase());
        }
    }
    closed
}

fn set_priority_high_by_name(process_stem: &str) {
    // The one action that reaches into the game's own process. Per-game opt-in.
    let script = format!(
        "$p = Get-Process -Name '{}' -ErrorAction SilentlyContinue; foreach ($proc in $p) {{ $proc.PriorityClass = 'High' }}",
        process_stem
    );
    let status = hidden_command("powershell")
        .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
        .status();
    dlog!("set_priority_high_by_name({process_stem}) -> {:?}", status.map(|s| s.success()));
}

fn exe_file_name(path: &str) -> String {
    PathBuf::from(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase()
}

fn exe_file_stem(path: &str) -> String {
    PathBuf::from(path)
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase()
}

// ---------- windows tweaks ----------
//
// A → per-boost tweaks (belong to the `boost_tweaks` session). They are
// applied when a watched game starts boosting and reverted when it exits, so
// the machine goes back to its ordinary state.
//
// B → permanent tweaks. Applied once (one-time), the "applied" state is
// tracked in store.json so the UI can offer an explicit Revert. Several need
// a reboot to actually take effect and are purely optional.
//
// Each tweak is a pile of registry/service/bcdedit commands. Admin tweaks
// (HKLM/bcdedit/sc) re-run through a UAC "runas" helper when the app itself
// isn't elevated.

#[derive(Clone, Copy, Debug)]
struct TweakDef {
    id: &'static str,
    label: &'static str,
    help: &'static str,
    group: &'static str, // "boost" | "permanent"
    admin: bool,
    reboot: bool,
}

fn tweak_defs() -> Vec<TweakDef> {
    vec![
        TweakDef {
            id: "fso",
            label: "Disable fullscreen optimizations",
            help: "Prevents the Windows Game Bar layer from double-buffering the game window (fixes a common FPS hit in DX11 games).",
            group: "boost", admin: false, reboot: false,
        },
        TweakDef {
            id: "dvr",
            label: "Game Bar / Game DVR off",
            help: "Kills background recording & the Game Bar app while playing (false caption capturing).",
            group: "boost", admin: false, reboot: false,
        },
        TweakDef {
            id: "bgapps",
            label: "Background apps off",
            help: "Disables background execution of Windows Store apps that resume in the background.",
            group: "boost", admin: false, reboot: false,
        },
        TweakDef {
            id: "vfx",
            label: "Visual effects: best performance",
            help: "Sets Windows visual effects to the 'best performance' preset (no shadows/animations).",
            group: "boost", admin: false, reboot: false,
        },
        TweakDef {
            id: "netthrottle",
            label: "NetworkThrottlingIndex off",
            help: "Lifts Windows' multimedia network throttling cap (0xffffffff). Only helps if you're network-bound.",
            group: "boost", admin: true, reboot: false,
        },
        TweakDef {
            id: "sysresp",
            label: "SystemResponsiveness 10",
            help: "MMCSS responsiveness — Windows rounds SystemResponsiveness /10, so min useful value is 10 (of 1000 units).",
            group: "boost", admin: true, reboot: false,
        },
        TweakDef {
            id: "pwrthrottle",
            label: "Power throttling off",
            help: "Disables Windows dynamic power throttling for the whole machine.",
            group: "boost", admin: true, reboot: false,
        },
        TweakDef {
            id: "hags",
            label: "Hardware-accelerated GPU scheduling",
            help: "Toggles the Windows HAGS feature. Affects frame timing & VRR — results vary by GPU/driver. Reboot applies.",
            group: "permanent", admin: true, reboot: true,
        },
        TweakDef {
            id: "dynamictick",
            label: "Dynamic tick off",
            help: "bcdedit disabledynamictick. Trades idle power for timer responsiveness. Widely claimed to cut input lag — results vary by hardware; benchmark before/after.",
            group: "permanent", admin: true, reboot: true,
        },
        TweakDef {
            id: "hpet",
            label: "HPET off (useplatformclock)",
            help: "Removes the High Precision Event Timer from the boot entry. Timing results are inconsistent — some report FPS drops alongside smoother pacing. YMMV; benchmark before/after.",
            group: "permanent", admin: true, reboot: true,
        },
        TweakDef {
            id: "priority",
            label: "Win32PrioritySeparation 0x1A",
            help: "Makes Windows prefer the foreground process's thread scheduling. One of the most-misunderstood tweaks — little solid evidence, results vary; benchmark before/after. Revert restores the default 0x02.",
            group: "permanent", admin: true, reboot: false,
        },
        TweakDef {
            id: "memint",
            label: "Memory integrity (VBS) off",
            help: "Disables Hypervisor-Enforced Code Integrity (memory integrity). Only turn it off if a driver you need can't load under HVCI — hover '?' for exactly what you lose. Reboot applies.",
            group: "permanent", admin: true, reboot: true,
        },
        TweakDef {
            id: "diag",
            label: "DiagTrack service off",
            help: "Stops the Connected User Experiences & Telemetry service.",
            group: "permanent", admin: true, reboot: false,
        },
        TweakDef {
            id: "sysmain",
            label: "SysMain (Superfetch) off",
            help: "Stops SysMain. Usually pointless on SSD, occasionally helps HDD reads — benchmark first.",
            group: "permanent", admin: true, reboot: false,
        },
        TweakDef {
            id: "mpo",
            label: "Disable Multiplane Overlay (MPO)",
            help: "Stops the GPU from compositing windows as separate hardware planes. A troubleshooting tweak for flicker, black flashes, or stutter — especially with G-Sync/FreeSync. Apply only if you're actually seeing those symptoms. Reboot applies.",
            group: "permanent", admin: true, reboot: true,
        },
        TweakDef {
            id: "corepark",
            label: "Disable core parking",
            help: "Keeps all CPU cores awake during light load so bursty games don't wait for a parked core to wake. Changes the currently active power plan — apply after the plan swaps to High Performance. Revert restores the default 5%.",
            group: "permanent", admin: true, reboot: false,
        },
    ]
}

type Cmd = (String, Vec<String>);

fn reg_cmd(key: &str, name: &str, dword: &str) -> Cmd {
    (
        "reg".into(),
        vec![
            "add".into(),
            key.into(),
            "/v".into(),
            name.into(),
            "/t".into(),
            "REG_DWORD".into(),
            "/d".into(),
            dword.into(),
            "/f".into(),
        ],
    )
}

fn reg_del_value(key: &str, name: &str) -> Cmd {
    (
        "reg".into(),
        vec![
            "delete".into(),
            key.into(),
            "/v".into(),
            name.into(),
            "/f".into(),
        ],
    )
}

fn cmd1(program: &str, args: Vec<&str>) -> Cmd {
    (
        program.to_string(),
        args.into_iter().map(|s| s.to_string()).collect(),
    )
}

/// Apply command lines for a tweak. `exes` is filled for `fso` (the A1
/// per-game override lives in AppCompatFlags\Layers).
fn apply_cmds(id: &str, exes: &[String]) -> Vec<Cmd> {
    match id {
        "fso" => exes
            .iter()
            .map(|exe| {
                let t = vec![
                    "add".to_string(),
                    r"HKCU\Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers".into(),
                    "/v".into(),
                    exe.to_string(),
                    "/t".into(),
                    "REG_SZ".into(),
                    "/d".into(),
                    "~ DISABLEDXMAXIMIZEDWINDOWEDMODE".into(),
                    "/f".into(),
                ];
                ("reg".into(), t)
            })
            .collect(),
        "dvr" => vec![
            reg_cmd(r"HKCU\System\GameConfigStore", "GameDVR_Enabled", "0"),
            reg_cmd(r"HKCU\Software\Microsoft\Windows\CurrentVersion\GameDVR", "AppCaptureEnabled", "0"),
        ],
        "bgapps" => vec![reg_cmd(
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\BackgroundAccessApplications",
            "GlobalUserDisabled",
            "1",
        )],
        "vfx" => vec![reg_cmd(
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\VisualEffects",
            "VisualFXSetting",
            "2",
        )],
        "netthrottle" => vec![reg_cmd(
            r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Multimedia\SystemProfile",
            "NetworkThrottlingIndex",
            "0xffffffff",
        )],
        "sysresp" => vec![reg_cmd(
            r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Multimedia\SystemProfile",
            "SystemResponsiveness",
            "10",
        )],
        "pwrthrottle" => vec![reg_cmd(
            r"HKLM\SYSTEM\CurrentControlSet\Control\Power\PowerThrottling",
            "PowerThrottlingOff",
            "1",
        )],
        "hags" => vec![reg_cmd(
            r"HKLM\SYSTEM\CurrentControlSet\Control\GraphicsDrivers",
            "HwSchMode",
            "2",
        )],
        "dynamictick" => vec![cmd1("bcdedit", vec!["/set", "disabledynamictick", "yes"])],
        "hpet" => vec![cmd1("bcdedit", vec!["/set", "useplatformclock", "false"])],
        "priority" => vec![reg_cmd(
            r"HKLM\SYSTEM\CurrentControlSet\Control\PriorityControl",
            "Win32PrioritySeparation",
            "0x1a",
        )],
        "memint" => vec![reg_cmd(
            r"HKLM\SYSTEM\CurrentControlSet\Control\DeviceGuard\Scenarios\HypervisorEnforcedCodeIntegrity",
            "Enabled",
            "0",
        )],
        "diag" => vec![
            cmd1("sc.exe", vec!["config", "DiagTrack", "start=", "disabled"]),
            cmd1("powershell", vec!["-NoProfile", "-Command", "Stop-Service -Name DiagTrack -Force -ErrorAction SilentlyContinue"]),
        ],
        "sysmain" => vec![
            cmd1("sc.exe", vec!["config", "SysMain", "start=", "disabled"]),
            cmd1("powershell", vec!["-NoProfile", "-Command", "Stop-Service -Name SysMain -Force -ErrorAction SilentlyContinue"]),
        ],
        "mpo" => vec![reg_cmd(
            r"HKLM\SOFTWARE\Microsoft\Windows\Dwm",
            "OverlayTestMode",
            "5",
        )],
        "corepark" => vec![
            cmd1("powercfg", vec!["-setacvalueindex", "SCHEME_CURRENT", "54533251-82be-4824-96c1-47b60b740d00", "0cc5b647-c1df-4637-891a-dec35c318583", "100"]),
            cmd1("powercfg", vec!["-S", "SCHEME_CURRENT"]),
        ],
        _ => vec![],
    }
}

fn revert_cmds(id: &str, exes: &[String]) -> Vec<Cmd> {
    match id {
        "fso" => exes
            .iter()
            .map(|exe| {
                (
                    "reg".into(),
                    vec![
                        "delete".to_string(),
                        r"HKCU\Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers".into(),
                        "/v".into(),
                        exe.to_string(),
                        "/f".into(),
                    ],
                )
            })
            .collect(),
        "dvr" => vec![
            reg_cmd(r"HKCU\System\GameConfigStore", "GameDVR_Enabled", "1"),
            reg_cmd(r"HKCU\Software\Microsoft\Windows\CurrentVersion\GameDVR", "AppCaptureEnabled", "1"),
        ],
        "bgapps" => vec![reg_del_value(
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\BackgroundAccessApplications",
            "GlobalUserDisabled",
        )],
        "vfx" => vec![reg_cmd(
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\VisualEffects",
            "VisualFXSetting",
            "0",
        )],
        "netthrottle" => vec![reg_cmd(
            r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Multimedia\SystemProfile",
            "NetworkThrottlingIndex",
            "10",
        )],
        "sysresp" => vec![reg_cmd(
            r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Multimedia\SystemProfile",
            "SystemResponsiveness",
            "20",
        )],
        "pwrthrottle" => vec![reg_del_value(
            r"HKLM\SYSTEM\CurrentControlSet\Control\Power\PowerThrottling",
            "PowerThrottlingOff",
        )],
        "hags" => vec![reg_cmd(
            r"HKLM\SYSTEM\CurrentControlSet\Control\GraphicsDrivers",
            "HwSchMode",
            "1",
        )],
        "dynamictick" => vec![cmd1("bcdedit", vec!["/set", "disabledynamictick", "no"])],
        "hpet" => vec![cmd1("bcdedit", vec!["/set", "useplatformclock", "true"])],
        "priority" => vec![reg_cmd(
            r"HKLM\SYSTEM\CurrentControlSet\Control\PriorityControl",
            "Win32PrioritySeparation",
            "0x02",
        )],
        "memint" => vec![reg_cmd(
            r"HKLM\SYSTEM\CurrentControlSet\Control\DeviceGuard\Scenarios\HypervisorEnforcedCodeIntegrity",
            "Enabled",
            "1",
        )],
        "diag" => vec![
            cmd1("sc.exe", vec!["config", "DiagTrack", "start=", "demand"]),
            cmd1("powershell", vec!["-NoProfile", "-Command", "Start-Service -Name DiagTrack -ErrorAction SilentlyContinue"]),
        ],
        "sysmain" => vec![
            cmd1("sc.exe", vec!["config", "SysMain", "start=", "auto"]),
            cmd1("powershell", vec!["-NoProfile", "-Command", "Start-Service -Name SysMain -ErrorAction SilentlyContinue"]),
        ],
        "mpo" => vec![reg_del_value(
            r"HKLM\SOFTWARE\Microsoft\Windows\Dwm",
            "OverlayTestMode",
        )],
        "corepark" => vec![
            cmd1("powercfg", vec!["-setacvalueindex", "SCHEME_CURRENT", "54533251-82be-4824-96c1-47b60b740d00", "0cc5b647-c1df-4637-891a-dec35c318583", "5"]),
            cmd1("powercfg", vec!["-S", "SCHEME_CURRENT"]),
        ],
        _ => vec![],
    }
}

fn run_cmds_here(cmds: &[Cmd]) -> bool {
    let mut ok = true;
    for (prog, args) in cmds {
        let status = hidden_command(prog).args(args).status();
        let code = status.ok().and_then(|s| s.code());
        // sc.exe exit 1062 = service has not been started (stop on an already
        // stopped service), 1056 = service already running (start on a running
        // one). Both mean the service is already in the state we wanted.
        if code == Some(0) {
            dlog!("tweak cmd ok: {prog} {:?}", args);
        } else if prog == "sc.exe" && matches!(code, Some(1056) | Some(1062)) {
            dlog!("tweak cmd acceptable state: {prog} {:?} (exit {code:?})", args);
        } else {
            dlog!("tweak cmd FAILED: {prog} {:?} (exit {code:?})", args);
            ok = false;
        }
    }
    ok
}

/// Run every command for one tweak, honouring its admin flag: if the app is
/// not elevated, re-launch the whole command sub-process *as admin* (UAC)
/// so the HKLM/bcdedit/sc ops still land.
fn run_tweak_cmds(cmds: &[Cmd], admin: bool) -> bool {
    if cmds.is_empty() {
        return true;
    }
    if !admin || is_elevated() {
        return run_cmds_here(cmds);
    }
    // Build a temporary .cmd1 that contains the lines, then run it via
    // runas so one UAC dialog applies the whole tweak. `--russian-doll`
    // isn't needed; we pass the file to cmd.exe elevated.
    let mut buf = String::new();
    buf.push_str("@echo off\r\n");
    for (prog, args) in cmds {
        buf.push_str(prog);
        for a in args {
            buf.push(' ');
            if a.contains(' ') || a.contains('"') {
                buf.push('"');
                buf.push_str(&a.replace('"', "\""));
                buf.push('"');
            } else {
                buf.push_str(a);
            }
        }
        buf.push_str("\r\n");
    }
    let inside = std::env::temp_dir().join(format!("ss-tweak-{}.cmd", Uuid::new_v4().simple()));
    if fs::write(&inside, &buf).is_err() {
        dlog!("failed to write tweak script {}", inside.display());
        return false;
    }
    let script = format!(
        "Start-Process -FilePath 'cmd.exe' -ArgumentList '/c', \"\"\"{}\"\"\" -Verb RunAs -Wait -WindowStyle Hidden",
        inside.display().to_string().replace('\'', "''")
    );
    let out = hidden_command("powershell")
        .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
        .output();
    let ok = out.map(|o| o.status.success()).unwrap_or(false);
    let _ = fs::remove_file(&inside);
    dlog!("tweaks elevated: {} ok={}", inside.display(), ok);

    // Writing the tweaks via cmd.exe exits 0 even on many failures, so double
    // check by… doing nothing else. Good enough for a runas.
    ok
}

// Apply-revert-side state for the watcher: the boost group is session-scoped.
fn boost_tweak_ids(settings: &Settings) -> Vec<String> {
    let defs = tweak_defs();
    let ids: HashSet<String> = settings.boost_tweaks.iter().cloned().collect();
    defs.into_iter()
        .filter(|t| t.group == "boost" && ids.contains(t.id))
        .map(|t| t.id.to_string())
        .collect()
}

/// Apply every enabled per-boost tweak. Returns the tweak ids actually applied
/// (admin tweaks are skipped when the process is not elevated, because the
/// auto-runas spark at game-start would be annoying — user toggles those
/// deliberately in Settings).
fn apply_enabled_boost_tweaks(settings: &Settings, exes: &[String]) -> Vec<String> {
    let elevated = is_elevated();
    let mut applied = vec![];
    for id in boost_tweak_ids(settings) {
        let def = tweak_defs().into_iter().find(|t| t.id == id);
        let Some(def) = def else { continue };
        let cmds = apply_cmds(&id, exes);
        if def.admin && !elevated {
            dlog!("boost tweak {id} needs admin, skipping (not elevated)");
            continue;
        }
        if run_tweak_cmds(&cmds, def.admin) {
            applied.push(id);
        }
    }
    dlog!("boost tweaks applied this launch: {applied:?}");
    applied
}

fn revert_applied_boost_tweaks(applied: &[String], exes: &[String]) {
    let elevated = is_elevated();
    for id in applied {
        let def = tweak_defs().into_iter().find(|t| t.id == id);
        let Some(def) = def else { continue };
        let cmds = revert_cmds(id, exes);
        if def.admin && !elevated {
            dlog!("boost tweak {id} admin revert skipped (not elevated)");
            continue;
        }
        run_tweak_cmds(&cmds, def.admin);
    }
}

// ---------- game icons ----------

fn icon_cache_dir(app: &AppHandle) -> PathBuf {
    let dir = app.path().app_data_dir().expect("no app data dir").join("icons");
    fs::create_dir_all(&dir).ok();
    dir
}

fn icon_cache_file(app: &AppHandle, exe: &str) -> PathBuf {
    let name = format!("{:x}.png", exe_hash(exe));
    icon_cache_dir(app).join(name)
}

fn exe_hash(exe: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in exe.to_lowercase().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Returns the exe's embedded icon as a data: URL (PNG). Extracts to a cache
/// once, then reads from disk.
#[tauri::command]
fn get_game_icon(app: AppHandle, exe_path: String) -> Option<String> {
    let cached = icon_cache_file(&app, &exe_path);
    if !cached.exists() {
        let out = cached.display().to_string();
        let exe_cmd = exe_path.replace('\'', "''");
        let script = format!(
            "Add-Type -AssemblyName System.Drawing; $exe=\"{}\"; try {{ $i=[System.Drawing.Icon]::ExtractAssociatedIcon($exe); if($i){{ $bmp=$i.ToBitmap(); $bmp.Save(\"{}\",[System.Drawing.Imaging.ImageFormat]::Png); $null=$i.Dispose(); $null=$bmp.Dispose() }} }} catch {{ }}",
            exe_cmd, out
        );
        let st = hidden_command("powershell")
            .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
            .status();
        dlog!(
            "get_game_icon({}) extract -> {:?} exists={}",
            exe_path,
            st.map(|s| s.success()),
            cached.exists()
        );
    }
    if let Ok(bytes) = fs::read(&cached) {
        if bytes.is_empty() {
            return None;
        }
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        return Some(format!("data:image/png;base64,{b64}"));
    }
    None
}

fn steam_root() -> Option<PathBuf> {
    for candidate in [
        r"C:\Program Files (x86)\Steam",
        r"C:\Program Files\Steam",
    ] {
        let p = PathBuf::from(candidate);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn steam_root_from_registry() -> Option<PathBuf> {
    let output = hidden_command("reg")
        .args([
            "query",
            r"HKCU\Software\Valve\Steam",
            "/v",
            "SteamPath",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find_map(|line| {
            let line = line.trim();
            let idx = line.rfind("REG_SZ").or_else(|| line.rfind("REG_EXPAND_SZ"))?;
            let val = line[idx..].split_whitespace().nth(1)?;
            let p = PathBuf::from(val.replace('/', "\\"));
            p.exists().then_some(p)
        })
}

fn parse_vdf_string_values(raw: &str, key: &str) -> Vec<String> {
    // Minimal VDF scraper: pulls every `"key"   "value"` pair for a given key.
    let needle = format!("\"{}\"", key);
    raw.lines()
        .filter_map(|line| {
            let line = line.trim();
            if !line.starts_with(&needle) {
                return None;
            }
            let rest = &line[needle.len()..];
            let parts: Vec<&str> = rest.splitn(3, '"').collect();
            // rest looks like:    "value"
            parts.get(1).map(|v| v.to_string())
        })
        .collect()
}

fn library_folders(steam_path: &PathBuf) -> Vec<PathBuf> {
    let mut libs = vec![steam_path.clone()];
    let vdf_path = steam_path
        .join("steamapps")
        .join("libraryfolders.vdf");
    if let Ok(raw) = fs::read_to_string(&vdf_path) {
        for p in parse_vdf_string_values(&raw, "path") {
            let pb = PathBuf::from(p.replace("\\\\", "\\"));
            if pb.exists() && !libs.contains(&pb) {
                libs.push(pb);
            }
        }
    }
    libs
}

fn find_exe_in_dir(dir: &PathBuf, hint: &str, depth: u8) -> Option<PathBuf> {
    if depth == 0 {
        return None;
    }
    let entries = fs::read_dir(dir).ok()?;
    let mut best: Option<(PathBuf, i32)> = None;
    let hint_lower = hint.to_lowercase();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(
                name.to_lowercase().as_str(),
                "_commonredist" | "redistributables" | "crashreportclient" | "engine"
            ) {
                continue;
            }
            if let Some(found) = find_exe_in_dir(&path, hint, depth - 1) {
                let score = score_exe(&found, &hint_lower);
                if best.as_ref().map(|(_, s)| score > *s).unwrap_or(true) {
                    best = Some((found, score));
                }
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("exe") {
            let score = score_exe(&path, &hint_lower);
            if best.as_ref().map(|(_, s)| score > *s).unwrap_or(true) {
                best = Some((path, score));
            }
        }
    }
    best.map(|(p, _)| p)
}

fn find_exes_in_dir(dir: &PathBuf, depth: u8) -> Vec<PathBuf> {
    // All plausible game-running exes, used by the folder scanner.
    if depth == 0 {
        return vec![];
    }
    let mut out = vec![];
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(
                name.to_lowercase().as_str(),
                "_commonredist" | "redistributables" | "crashreportclient" | "engine" | "sdk" | "tools"
            ) {
                continue;
            }
            out.extend(find_exes_in_dir(&path, depth - 1));
        } else if path.extension().and_then(|e| e.to_str()) == Some("exe") {
            let score = score_exe(&path, "");
            if score > -50 {
                out.push(path);
            }
        }
    }
    out
}

fn score_exe(path: &PathBuf, hint_lower: &str) -> i32 {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mut score = 0;
    if stem.contains("uninstall") || stem.contains("crashpad") || stem.contains("redist") {
        score -= 100;
    }
    if stem.contains("launcher") && hint_lower.is_empty() {
        score -= 15;
    }
    if hint_lower.contains(&stem) || stem.contains(hint_lower) {
        score += 50;
    }
    // shallower paths score higher (root install exe over some tool subfolder exe)
    score -= path.components().count() as i32;
    score
}

// ---------- commands ----------

#[tauri::command]
async fn scan_steam_games() -> Vec<GameEntry> {
    dlog!("cmd scan_steam_games");
    let root = steam_root().or_else(steam_root_from_registry);
    let Some(root) = root else {
        dlog!("scan_steam_games: no steam root found");
        return vec![];
    };
    dlog!("scan_steam_games: root={}", root.display());
    let mut out = vec![];
    for lib in library_folders(&root) {
        let steamapps = lib.join("steamapps");
        let Ok(entries) = fs::read_dir(&steamapps) else {
            dlog!("scan_steam_games: no steamapps in {}", lib.display());
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("acf") {
                continue;
            }
            let Ok(raw) = fs::read_to_string(&path) else {
                continue;
            };
            let name = parse_vdf_string_values(&raw, "name").into_iter().next();
            let installdir = parse_vdf_string_values(&raw, "installdir")
                .into_iter()
                .next();
            if let (Some(name), Some(installdir)) = (name, installdir) {
                if name.to_lowercase().contains("steamworks common redistributables") {
                    continue;
                }
                let game_dir = steamapps.join("common").join(&installdir);
                if !game_dir.exists() {
                    continue;
                }
                let exe = find_exe_in_dir(&game_dir, &name, 4)
                    .or_else(|| find_exe_in_dir(&game_dir, &installdir, 4));
                if let Some(exe) = exe {
                    out.push(new_game_entry(exe.to_string_lossy().to_string(), &name, "steam"));
                }
            }
        }
    }
    dlog!("scan_steam_games: found {} games", out.len());
    out
}

fn new_game_entry(exe_path: String, name: &str, source: &str) -> GameEntry {
    GameEntry {
        id: Uuid::new_v4().to_string(),
        name: name.trim().into(),
        exe_path,
        args: String::new(),
        source: source.into(),
        watched: false,
        boost_power: None,
        boost_priority: None,
        close_background: None,
        keep_open: vec![],
    }
}

#[tauri::command]
async fn scan_folder(folder: String) -> Vec<GameEntry> {
    dlog!("cmd scan_folder: {folder}");
    let dir = PathBuf::from(&folder);
    if !dir.is_dir() {
        dlog!("scan_folder: not a directory");
        return vec![];
    }
    let hint = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let exes = find_exes_in_dir(&dir, 3);
    // Prefer the exe that best matches the folder name.
    let mut scored: Vec<(i32, PathBuf)> = exes
        .iter()
        .cloned()
        .map(|p| (score_exe(&p, &hint.to_lowercase()), p))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    // Dedupe by file stem so we don't offer launcher + game exe of the same game.
    // Name each candidate after its own exe stem, not the folder, so the
    // list actually tells you what game each entry is.
    let mut seen = HashSet::new();
    let mut out = vec![];
    for (_, path) in scored {
        let stem = exe_file_stem(&path.to_string_lossy());
        if seen.insert(stem.clone()) {
            out.push(new_game_entry(
                path.to_string_lossy().to_string(),
                &stem,
                "custom",
            ));
        }
    }
    out.truncate(20);
    dlog!("scan_folder: {} candidates", out.len());
    out
}

#[tauri::command]
fn get_store(app: AppHandle) -> Store {
    let s = load_store(&app);
    dlog!("cmd get_store: {} games", s.games.len());
    s
}

#[tauri::command]
fn add_scan_folder(app: AppHandle, folder: String) -> Result<Store, String> {
    dlog!("cmd add_scan_folder: {folder}");
    let mut store = load_store(&app);
    if !store.scan_folders.iter().any(|f| f.eq_ignore_ascii_case(&folder)) {
        store.scan_folders.push(folder);
    }
    write_store(&app, &store)?;
    Ok(store)
}

#[tauri::command]
fn remove_scan_folder(app: AppHandle, folder: String) -> Result<Store, String> {
    dlog!("cmd remove_scan_folder: {folder}");
    let mut store = load_store(&app);
    store.scan_folders.retain(|f| !f.eq_ignore_ascii_case(&folder));
    write_store(&app, &store)?;
    Ok(store)
}

#[tauri::command]
fn save_game(app: AppHandle, mut game: GameEntry) -> Result<Store, String> {
    dlog!("cmd save_game: {} @ {}", game.name, game.exe_path);
    let mut store = load_store(&app);
    if game.id.is_empty() {
        game.id = Uuid::new_v4().to_string();
    }
    if let Some(existing) = store.games.iter_mut().find(|g| g.id == game.id) {
        *existing = game;
    } else {
        store.games.push(game);
    }
    write_store(&app, &store)?;
    Ok(store)
}

#[tauri::command]
fn remove_game(app: AppHandle, id: String) -> Result<Store, String> {
    dlog!("cmd remove_game: {id}");
    let mut store = load_store(&app);
    store.games.retain(|g| g.id != id);
    write_store(&app, &store)?;
    Ok(store)
}

#[tauri::command]
fn save_settings(app: AppHandle, settings: Settings) -> Result<Store, String> {
    dlog!(
        "cmd save_settings: power={} priority={} close={} theme={} blocklist={:?}",
        settings.boost_power_plan,
        settings.boost_priority,
        settings.close_background,
        settings.theme,
        settings.blocklist
    );
    let mut store = load_store(&app);
    store.settings = settings;
    DEBUG_LOGGING.store(store.settings.debug_logging, Ordering::Relaxed);
    write_store(&app, &store)?;
    Ok(store)
}

#[tauri::command]
fn set_watched(app: AppHandle, id: String, watched: bool) -> Result<Store, String> {
    dlog!("cmd set_watched: id={id} watched={watched}");
    let mut store = load_store(&app);
    if let Some(g) = store.games.iter_mut().find(|g| g.id == id) {
        g.watched = watched;
    }
    write_store(&app, &store)?;
    Ok(store)
}

#[tauri::command]
fn set_game_options(
    app: AppHandle,
    id: String,
    watched: Option<bool>,
    boost_power: Option<bool>,
    boost_priority: Option<bool>,
    close_background: Option<bool>,
    keep_open: Option<Vec<String>>,
) -> Result<Store, String> {
    dlog!(
        "cmd set_game_options: id={id} watched={watched:?} power={boost_power:?} priority={boost_priority:?} close={close_background:?} keep_open={keep_open:?}"
    );
    let mut store = load_store(&app);
    if let Some(g) = store.games.iter_mut().find(|g| g.id == id) {
        if let Some(v) = watched {
            g.watched = v;
        }
        if let Some(v) = boost_power {
            g.boost_power = Some(v);
        }
        if let Some(v) = boost_priority {
            g.boost_priority = Some(v);
        }
        if let Some(v) = close_background {
            g.close_background = Some(v);
        }
        if let Some(v) = keep_open {
            g.keep_open = v;
        }
    }
    write_store(&app, &store)?;
    Ok(store)
}

#[tauri::command]
fn default_blocklist() -> Vec<String> {
    Settings::default().blocklist
}

#[tauri::command]
fn set_game_override(
    app: AppHandle,
    id: String,
    field: String,
    value: Option<bool>,
) -> Result<Store, String> {
    dlog!("cmd set_game_override: id={id} field={field} value={value:?}");
    let mut store = load_store(&app);
    if let Some(g) = store.games.iter_mut().find(|g| g.id == id) {
        match field.as_str() {
            "boost_power" => g.boost_power = value,
            "boost_priority" => g.boost_priority = value,
            "close_background" => g.close_background = value,
            _ => {}
        }
    }
    write_store(&app, &store)?;
    Ok(store)
}

#[tauri::command]
async fn list_running_processes() -> Vec<String> {
    dlog!("cmd list_running_processes");
    let mut names = running_process_names();
    let mut v: Vec<String> = names.drain().collect();
    v.sort();
    dlog!("cmd list_running_processes: {} procs", v.len());
    v
}

// ---------- frontend debug log bridge ----------

#[tauri::command]
fn debug_log(level: String, message: String) {
    log_msg(&format!("[js {level}] {message}"));
}

#[tauri::command]
fn get_debug_log_path() -> String {
    log_file().display().to_string()
}

/// Open (or toggle) the toa.sh debug console as a real secondary OS window.
/// The first call lazily creates the "toa" webview (toa.html); afterwards it
/// simply shows/hides that same window so its buffer survives toggling.
/// Returns whether the console is now visible.
#[tauri::command]
fn toggle_toa(app: AppHandle) -> Result<bool, String> {
    if let Some(win) = app.get_webview_window("toa") {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
            return Ok(false);
        }
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(true);
    }
    // Place it bottom-right of the primary monitor, like a small HUD window.
    let (w, h) = (420.0_f64, 280.0_f64);
    let (x, y) = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| {
            let size = m.size();
            (
                (size.width as f64 - w - 24.0).max(0.0),
                (size.height as f64 - h - 48.0).max(0.0),
            )
        })
        .unwrap_or((200.0, 200.0));
    let win = tauri::WebviewWindowBuilder::new(&app, "toa", WebviewUrl::App("toa.html".into()))
        .title("toa.sh")
        .inner_size(w, h)
        .min_inner_size(300.0, 160.0)
        .position(x, y)
        .resizable(true)
        .decorations(false)
        .build()
        .map_err(|e| e.to_string())?;
    let _ = win.show();
    let _ = win.set_focus();
    Ok(true)
}

/// Save the toa.sh console buffer to a dated file in the exe's own folder
/// (the same place the main debug log goes). One file per run: the stamp is
/// captured when the app starts, so filenames look like
/// `slipstream-console-20260809-013456.log`. Returns the full path written.
#[tauri::command]
fn save_toa_log(content: String) -> String {
    let path = log_dir().join(format!("slipstream-console-{}.log", session_stamp()));
    let _ = fs::write(&path, content);
    dlog!("toa console autosaved -> {}", path.display());
    path.display().to_string()
}

static SESSION_STAMP: OnceLock<String> = OnceLock::new();

fn session_stamp() -> String {
    SESSION_STAMP
        .get_or_init(|| {
            use chrono::Local;
            Local::now().format("%Y%m%d-%H%M%S").to_string()
        })
        .clone()
}

/// Return the most recent `limit` raw log lines from the debug log file,
/// newest first, so the frontend debug console can pre-fill its backlog.
#[tauri::command]
fn get_debug_log_backlog(limit: Option<usize>) -> Vec<String> {
    let max = limit.unwrap_or(300).min(2000);
    let path = log_file();
    let Ok(text) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    let mut lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines.truncate(max);
    let out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    dlog!("cmd get_debug_log_backlog: {} lines", out.len());
    out
}

#[tauri::command]
fn detect_game_processes(app: AppHandle) -> Vec<GameEntry> {
    dlog!("cmd detect_game_processes");
    let store = load_store(&app);
    let known_paths: HashSet<String> = store
        .games
        .iter()
        .map(|g| g.exe_path.to_lowercase())
        .collect();
    let dismissed: HashSet<String> = store
        .settings
        .dismissed_games
        .iter()
        .map(|s| s.to_lowercase())
        .collect();

    let procs = running_game_processes();
    let mut out: Vec<GameEntry> = vec![];
    let mut seen_names: HashSet<String> = HashSet::new();
    for (name, exe) in procs {
        if !looks_like_game(&name, exe.as_deref()) {
            continue;
        }
        let key = name.to_lowercase();
        if dismissed.contains(&key) || seen_names.contains(&key) {
            continue;
        }
        let path = exe
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| name.clone());
        if known_paths.contains(&path.to_lowercase()) {
            continue;
        }
        seen_names.insert(key.clone());
        out.push(new_game_entry(path, &name, "detected"));
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    dlog!("detect_game_processes: {} candidates", out.len());
    out
}

// ---------- tweak commands ----------

#[derive(Serialize, Clone, Debug)]
struct TweakView {
    id: String,
    label: String,
    help: String,
    group: String, // "boost" | "permanent"
    admin: bool,
    reboot: bool,
    #[serde(rename = "appliedOrOn")]
    applied_or_on: bool, // boost: user toggled it on; permanent: applied on the system
    tip: String, // longer explainer shown in a "?" mouseover (empty = none)
}

fn tweak_tip(id: &str) -> String {
    match id {
        "memint" => "Memory integrity (HVCI) runs the kernel behind a hypervisor and refuses to load any driver that doesn't pass hypervisor-based code-integrity checks. What you LOSE by turning it off:\n\n• Kernel drivers are no longer gated. A buggy, unsigned, or hijacked driver loads again — and if that driver is ever exploited, it gets full kernel control with nothing left to block it. HVCI is precisely the thing that would have stopped that.\n• The 'Core isolation' / VBS hardening that many security features lean on quietly goes away.\n\nWhat you gain: old or non-compliant drivers (legacy audio, GPU, anti-cheat, overclocking tools) start again — that's the usual only reason to turn it off. Perf gain on a modern CPU is negligible. Revert it any time; reboot applies.".into(),
        _ => String::new(),
    }
}

fn tweaks_view(settings: &Settings) -> Vec<TweakView> {
    let on: HashSet<String> = settings.boost_tweaks.iter().cloned().collect();
    let applied: HashSet<String> = settings.applied_tweaks.iter().cloned().collect();
    tweak_defs()
        .into_iter()
        .map(|t| TweakView {
            id: t.id.to_string(),
            label: t.label.to_string(),
            help: t.help.to_string(),
            group: t.group.to_string(),
            admin: t.admin,
            reboot: t.reboot,
            applied_or_on: if t.group == "boost" {
                on.contains(t.id)
            } else {
                applied.contains(t.id)
            },
            tip: tweak_tip(t.id),
        })
        .collect()
}

#[tauri::command]
fn get_tweaks(app: AppHandle) -> (Vec<TweakView>, bool) {
    let s = load_store(&app);
    dlog!("cmd get_tweaks: {} boost, {} applied", s.settings.boost_tweaks.len(), s.settings.applied_tweaks.len());
    (tweaks_view(&s.settings), is_elevated())
}

/// Flip a per-boost tweak on/off (session-scoped configuration).
#[tauri::command]
fn set_boost_tweak(app: AppHandle, id: String, on: bool) -> Result<Vec<TweakView>, String> {
    dlog!("cmd set_boost_tweak: {id} on={on}");
    let mut store = load_store(&app);
    if on {
        if !store.settings.boost_tweaks.iter().any(|x| *x == id) {
            store.settings.boost_tweaks.push(id);
        }
    } else {
        store.settings.boost_tweaks.retain(|x| *x != id);
    }
    write_store(&app, &store)?;
    Ok(tweaks_view(&store.settings))
}

/// Apply a permanent tweak to the system right now (elevates via UAC if the
/// app is not already elevated), then track it as applied.
#[tauri::command]
fn apply_permanent_tweak(app: AppHandle, id: String) -> Result<Vec<TweakView>, String> {
    dlog!("cmd apply_permanent_tweak: {id}");
    let mut store = load_store(&app);
    let def = tweak_defs().into_iter().find(|t| t.id == id);
    let Some(def) = def else {
        return Err(format!("unknown tweak {id}"));
    };
    if def.group != "permanent" {
        return Err(format!("{id} is not a permanent tweak"));
    }
    let cmds = apply_cmds(&id, &[]);
    if !run_tweak_cmds(&cmds, def.admin) {
        return Err(format!("{id} failed to apply (permission or command error)"));
    }
    if !store.settings.applied_tweaks.iter().any(|t| *t == id) {
        store.settings.applied_tweaks.push(id);
    }
    write_store(&app, &store)?;
    Ok(tweaks_view(&store.settings))
}

/// Revert a permanent tweak to stock (elevated via UAC if needed).
#[tauri::command]
fn revert_permanent_tweak(app: AppHandle, id: String) -> Result<Vec<TweakView>, String> {
    dlog!("cmd revert_permanent_tweak: {id}");
    let mut store = load_store(&app);
    let def = tweak_defs().into_iter().find(|t| t.id == id);
    let Some(def) = def else {
        return Err(format!("unknown tweak {id}"));
    };
    let cmds = revert_cmds(&id, &[]);
    if !run_tweak_cmds(&cmds, def.admin) {
        return Err(format!("{id} failed to revert (permission denied or command error)"));
    }
    store.settings.applied_tweaks.retain(|x| *x != id);
    write_store(&app, &store)?;
    Ok(tweaks_view(&store.settings))
}

// ---------- status ----------

#[derive(Serialize, Clone, Default)]
struct StatusPayload {
    running: Vec<String>, // ids of games currently running & watched
    boosted: bool,
    closed_processes: Vec<String>,
}

fn emit_status(app: &AppHandle, state: &SharedState) {
    let s = state.lock().unwrap();
    let payload = StatusPayload {
        running: s.running_game_ids.clone(),
        boosted: s.boosted(),
        closed_processes: s.closed_processes.clone(),
    };
    let _ = app.emit("slipstream://status", payload);
}

impl RuntimeState {
    fn boosted(&self) -> bool {
        self.prior_power_scheme.is_some()
            || !self.closed_processes.is_empty()
            || !self.boost_tweaks_applied.is_empty()
    }
}

fn watcher_tick(app: &AppHandle, state: &SharedState) {
    let store = load_store(app);
    let watched: Vec<GameEntry> = store.games.iter().filter(|g| g.watched).cloned().collect();

    if watched.is_empty() {
        let was_active = state.lock().unwrap().boosted();
        if was_active {
            dlog!("watcher: no watched games, restoring");
            do_restore(app, state);
        }
        return;
    }

    let procs = running_process_names();
    let running: Vec<GameEntry> = watched
        .iter()
        .filter(|g| procs.contains(&exe_file_name(&g.exe_path)))
        .cloned()
        .collect();

    let (already_boosted, last_running) = {
        let s = state.lock().unwrap();
        (s.boosted(), s.running_game_ids.clone())
    };

    let running_ids: Vec<String> = running.iter().map(|g| g.id.clone()).collect();
    let fresh_ids: Vec<String> = running_ids
        .iter()
        .filter(|id| !last_running.contains(id))
        .cloned()
        .collect();

    dlog!(
        "watcher tick: watched={} running_ids={:?} last={:?} boosted={}",
        watched.len(),
        running_ids,
        last_running,
        already_boosted
    );

    if running.is_empty() && already_boosted {
        dlog!("watcher: games exited, restoring");
        do_restore(app, state);
        return;
    }

    // Effective blocklist: strip any process the running games asked to keep open.
    let kept: HashSet<String> = running
        .iter()
        .flat_map(|g| g.keep_open.iter().cloned())
        .collect();
    let effective: Vec<String> = store
        .settings
        .blocklist
        .iter()
        .filter(|b| !kept.iter().any(|k| k.eq_ignore_ascii_case(b)))
        .cloned()
        .collect();
    let want_close = running
        .iter()
        .any(|g| g.close_background.unwrap_or(store.settings.close_background));

    if !running.is_empty() && !already_boosted {
        // ---- fresh boost: apply every effect the running games want ----
        let want_power = running
            .iter()
            .any(|g| g.boost_power.unwrap_or(store.settings.boost_power_plan));

        dlog!("watcher: engaging boost (power={want_power} close={want_close})");
        let prior_scheme = if want_power {
            let prior = capture_active_scheme();
            set_scheme("SCHEME_MIN");
            prior
        } else {
            None
        };

        dlog!("watcher: closing blocklist -> {:?}", effective);
        let closed = if want_close {
            close_blocklisted(&effective, &procs, false)
        } else {
            vec![]
        };
        for g in &running {
            if g.boost_priority.unwrap_or(store.settings.boost_priority) {
                set_priority_high_by_name(&exe_file_stem(&g.exe_path));
            }
        }

        // Apply the user's per-boost Windows tweaks (A-group); fso gets each
        // running game's exe so its AppCompatFlags entry targets the game.
        let exes: Vec<String> = running.iter().map(|g| g.exe_path.clone()).collect();
        let boost_applied = apply_enabled_boost_tweaks(&store.settings, &exes);

        {
            let mut s = state.lock().unwrap();
            s.running_game_ids = running_ids.clone();
            s.prior_power_scheme = prior_scheme;
            s.closed_processes = closed;
            s.boost_tweaks_applied = boost_applied;
            s.boost_exes = exes;
        }
        emit_status(app, state);
        return;
    }

    // Already boosting: keep closed processes dead even if they restarted,
    // apply per-game priority to anything that just appeared, and keep the
    // running list in sync. Uses the same process snapshot from this tick.
    if already_boosted {
        if want_close {
            let fresh_closed = close_blocklisted(&effective, &procs, true);
            if !fresh_closed.is_empty() {
                let mut s = state.lock().unwrap();
                for c in &fresh_closed {
                    if !s.closed_processes.contains(c) {
                        s.closed_processes.push(c.clone());
                    }
                }
                drop(s);
                emit_status(app, state);
            }
        }
        for g in &running {
            if fresh_ids.contains(&g.id)
                && g.boost_priority.unwrap_or(store.settings.boost_priority)
            {
                set_priority_high_by_name(&exe_file_stem(&g.exe_path));
            }
        }
    }

    if running_ids != last_running {
        let mut s = state.lock().unwrap();
        s.running_game_ids = running_ids.clone();
        let boosted_self = s.boosted();
        drop(s);
        if boosted_self {
            emit_status(app, state);
        }
    }
}

fn do_restore(app: &AppHandle, state: &SharedState) {
    let (prior, boost_applied, boost_exes) = {
        let mut s = state.lock().unwrap();
        let prior = s.prior_power_scheme.take();
        s.closed_processes.clear();
        s.running_game_ids.clear();
        let ba = std::mem::take(&mut s.boost_tweaks_applied);
        let be = std::mem::take(&mut s.boost_exes);
        (prior, ba, be)
    };
    if let Some(guid) = prior {
        dlog!("restoring power scheme {guid}");
        set_scheme(&guid);
    }
    if !boost_applied.is_empty() {
        dlog!("restoring boost tweaks: {boost_applied:?}");
        revert_applied_boost_tweaks(&boost_applied, &boost_exes);
    }
    emit_status(app, state);
}

#[tauri::command]
fn restore_now(app: AppHandle, state: State<SharedState>) -> Result<StatusPayload, String> {
    dlog!("cmd restore_now");
    do_restore(&app, state.inner());
    let s = state.lock().unwrap();
    Ok(StatusPayload {
        running: s.running_game_ids.clone(),
        boosted: s.boosted(),
        closed_processes: s.closed_processes.clone(),
    })
}

#[tauri::command]
fn get_status(state: State<SharedState>) -> StatusPayload {
    let s = state.lock().unwrap();
    StatusPayload {
        running: s.running_game_ids.clone(),
        boosted: s.boosted(),
        closed_processes: s.closed_processes.clone(),
    }
}

#[tauri::command]
async fn pick_exe(app: AppHandle) -> Option<String> {
    dlog!("cmd pick_exe");
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .add_filter("Executable", &["exe"])
        .blocking_pick_file()
        .map(|p| p.to_string())
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<String> {
    dlog!("cmd pick_folder");
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .blocking_pick_folder()
        .map(|p| p.to_string())
}

// ---------- admin elevation ----------

/// Public true: whether the current process runs with an elevated token.
fn is_elevated() -> bool {
    let script = r#"(New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)"#;
    let out = hidden_command("powershell")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-WindowStyle")
        .arg("Hidden")
        .arg("-Command")
        .arg(script)
        .output();
    match out {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout).to_lowercase();
            let ok = text.contains("true");
            dlog!("elevation check -> {}", if ok { "yes" } else { "no" });
            ok
        }
        Err(e) => {
            dlog!("elevation check failed: {e}");
            false
        }
    }
}

/// If not already elevated, relaunch this same exe with a UAC prompt and then
/// shut down the unelevated instance. Returns true when we should exit.
fn request_elevation_if_needed() -> bool {
    if is_elevated() {
        return false;
    }
    dlog!("not elevated – relaunching with RunAs (UAC prompt)");
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    let script = format!(
        "Start-Process -FilePath '{}' -Verb RunAs -ArgumentList '--slipstream-elevated'",
        exe.replace('\'', "''")
    );
    let status = hidden_command("powershell")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-WindowStyle")
        .arg("Hidden")
        .arg("-Command")
        .arg(&script)
        .status();
    match status {
        // UAC approved and the elevated copy was launched – hand over.
        Ok(s) if s.success() => {
            dlog!("UAC approved, elevating");
            true
        }
        // User declined UAC – keep running this (unelevated) instance so the
        // app still opens; power/close actions will just fail gracefully.
        _ => {
            dlog!("UAC declined (or failed) – continuing unelevated");
            false
        }
    }
}

/// Trigger a UAC relaunch so the whole app becomes elevated. If the relaunch
/// was approved, tells the current (unelevated) frontend to close itself and
/// also schedules this process to exit so the elevated copy takes over alone.
#[tauri::command]
fn request_elevation(_app: AppHandle) -> bool {
    dlog!("cmd request_elevation");
    if request_elevation_if_needed() {
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(800));
            std::process::exit(0);
        });
        true
    } else {
        false
    }
}

#[tauri::command]
fn get_ui_state(app: AppHandle) -> (bool, String) {
    let _ = app;
    dlog!("cmd get_ui_state");
    (is_elevated(), log_file().display().to_string())
}

fn main() {
    // Panic hook so crashes are captured in the debug log too.
    std::panic::set_hook(Box::new(|info| {
        let msg = info.payload().downcast_ref::<&str>().map(|s| s.to_string());
        let location = info.location().map(|l| l.to_string());
        log_msg(&format!(
            "PANIC: {:?} at {:?}",
            msg.or_else(|| info.payload().downcast_ref::<String>().cloned()),
            location
        ));
    }));

    dlog!("slipstream starting; log file: {}", log_file().display());
    dlog!("exe: {}", std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default());

    // If we're not running as admin (taskkill/powercfg need it), relaunch via
    // a UAC prompt and hand over. The elevated copy re-enters main() and
    // passes the is_elevated check. Skip in debug so `cargo tauri dev` (and
    // test runs) don't trigger UAC — release bundles are the real users.
    if cfg!(not(debug_assertions)) && request_elevation_if_needed() {
        std::process::exit(0);
    }

    let state: SharedState = Arc::new(Mutex::new(RuntimeState::default()));
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage::<SharedState>(state.clone())
        .invoke_handler(tauri::generate_handler![
            scan_steam_games,
            scan_folder,
            get_store,
            add_scan_folder,
            remove_scan_folder,
            save_game,
            remove_game,
            save_settings,
            set_watched,
            set_game_options,
            set_game_override,
            default_blocklist,
            get_tweaks,
            set_boost_tweak,
            apply_permanent_tweak,
            revert_permanent_tweak,
            get_game_icon,
            request_elevation,
            get_ui_state,
            list_running_processes,
            restore_now,
            get_status,
            pick_exe,
            pick_folder,
            debug_log,
            get_debug_log_path,
            get_debug_log_backlog,
            save_toa_log,
            detect_game_processes,
            toggle_toa,
        ])
        .setup(move |app| {
            let _ = APP_HANDLE.set(app.handle().clone());
            let app = app.handle().clone();
            let state = state.clone();
            // Closing the main window must terminate the whole app (including
            // the hidden/closed toa.sh console window), not leave it running.
            if let Some(win) = app.get_webview_window("main") {
                win.on_window_event(|e| {
                    if matches!(e, tauri::WindowEvent::CloseRequested { .. }) {
                        std::process::exit(0);
                    }
                });
            }
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_millis(2000));
                watcher_tick(&app, &state);
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running slipstream");
}