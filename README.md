# Slipstream

[![License: GPL-3.0](https://img.shields.io/github/license/ChaosCodify/slipstream)](https://github.com/ChaosCodify/slipstream/blob/main/LICENSE)

**Open-source, telemetry-free Razer Cortex alternative for Windows.** A local
"boost & watch" utility: it switches your PC to High Performance when you're
playing a game and closes the background junk (Discord, browsers, overlays)
that steals your frames — then restores your normal power plan when you're
done. No account, no telemetry, no bundled bloat. Built as a Tauri v2 app.

## Slipstream vs. Razer Cortex

| | Razer Cortex | Slipstream |
|---|---|---|
| Requires account | Yes | No |
| Telemetry | Yes | None — the whole source is here |
| Open source | No | Yes (GPL-3.0) |
| Launches/touches the game process | Yes | No — it only watches |
| Extra bundled apps / bloat | Yes | No |
| Works offline | No | Yes |

Slipstream is **not** a launcher and is **not** Razer. It never launches your
game or pokes its process, so it won't trip anti-cheat — it just boosts the
system around your game. It's the "boost" part of Cortex, done locally and
auditable, with one restore toggle and no account anywhere.

## Why Tauri instead of a `.ps1`?

Because it needs a persisted game list and a UI to manage the process
blocklist. The actual boost logic underneath is exactly the kind of thing
you'd put in a script: `taskkill` and `powercfg`. You can read every line of
what it does in `src-tauri/src/main.rs` — no bundled binary blobs, no update
checker, and no network code anywhere in this project (grep for `reqwest`,
`http`, or a URL — there isn't one).

## What it does

1. Scans your Steam library (`libraryfolders.vdf` + each app's `.acf`
   manifest, with a registry fallback for non-default installs) and proposes
   install-folder executables in a dropdown — you can also add any `.exe`
   manually.
2. You mark games as **Watch**. Slipstream then polls the process table in
   the background (every 2s). When a watched game's exe is running, it
   applies the boost: captures your current Windows power plan, switches to
   High Performance, and force-closes whatever's in your blocklist (Discord,
   browsers, overlays — fully editable).
3. When the game exits, your original power plan is restored automatically.
   A manual "Restore now" path exists in the backend too, for odd crashes.

## What it deliberately does NOT do

- It does **not launch** your game. It only watches for it to be running —
  that's the whole point. You start the game yourself (Steam, Epic, etc.)
  and Slipstream boosts the system around it. It never opens or pokes the
  game's process, so it won't trip anti-cheat the way a launch+priority
  wrapper might.
- It does not relaunch the apps it closed. Closing is one-way; that's a
  feature, not a bug.
- It does not touch Easy Anti-Cheat, suspend threads, or do anything at the
  kernel level. It's a userland process manager. Given what's actually wrong
  with Tokon's PC port right now, don't expect this to fully fix the stutter
  — it'll get you a real but partial improvement.
- No auto-updater, no crash reporter, no analytics SDK of any kind.

## Installing

Grab the latest release from the [Releases](https://github.com/ChaosCodify/slipstream/releases)
page — you get a portable exe, a NSIS setup, and an MSI. The portable exe
runs straight from a folder or USB stick, no install wizard required.

## Building it (Windows only)

You'll need, on the Windows machine you actually want to run this on:

```powershell
# 1. Rust toolchain
winget install Rustlang.Rustup

# 2. Tauri CLI
cargo install tauri-cli --version "^2"

# 3. From the project root:
cd slipstream
cargo tauri build
```

That produces an installer under `src-tauri/target/release/bundle/` (both
NSIS `.exe` and `.msi` are configured). For quick iteration without a full
build, `cargo tauri dev` runs it live.

No `npm install`, no `node_modules`, no bundler — the frontend is three plain
files (`src/index.html`, `style.css`, `main.js`) served directly, so there's
nothing to build on the JS side at all.

## Editing the blocklist defaults

The starting blocklist lives in `Settings::default()` near the top of
`src-tauri/src/main.rs` — edit that array before building if you want
different defaults baked in, or just manage it from the Settings tab at
runtime (it's saved to a local `store.json` in the app's data folder,
nothing else touches it).

## License & attribution

Licensed under the [GNU GPL v3](LICENSE). Copyright stays with the author.

**Not affiliated with Razer.** "Razer Cortex" and "Razer" are trademarks of
Razer Inc. This is an independent, community-built alternative and is in no
way endorsed by or connected to Razer.