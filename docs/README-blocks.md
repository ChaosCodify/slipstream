# Slipstream — README content blocks

Ready-to-paste blocks for the README / About panel. Grab what you want.

---

## 1. About description (repo About panel, right side under the repo name)

```
Open-source, telemetry-free Razer Cortex alternative for Windows. Boosts games without launching or touching the game process.
```

## 2. Topics (About panel → "Topics")

```
razer-cortex-alternative game-booster windows tauri foss open-source no-telemetry gaming-performance rust game-launcher
```

## 3. Tagline / intro paragraph

```
**Open-source, telemetry-free Razer Cortex alternative for Windows.** A local
"boost & watch" utility: it switches your PC to High Performance while you're
playing a game and closes the background junk (Discord, browsers, overlays)
that steals your frames — then restores your normal power plan when you're
done. No account, no telemetry, no bundled bloat. Built with Tauri (Rust).
```

## 4. Comparison table

```md
## Slipstream vs. Razer Cortex

|                      | Razer Cortex        | Slipstream                     |
|----------------------|---------------------|--------------------------------|
| Requires account     | Yes                 | No                             |
| Telemetry            | Yes                 | None — check the source        |
| Open source          | No                  | Yes (GPL-3.0)                  |
| Launches or touches the game process | Yes | No — it only watches    |
| Extra bundled apps / bloat | Yes            | No                             |
| Works offline        | No                  | Yes                            |
```

## 5. Disclaimer (recommended, put somewhere visible)

```
**Not affiliated with Razer.** "Razer Cortex" and "Razer" are trademarks of
Razer Inc. This is an independent, community-built alternative and is in no
way endorsed by or connected to Razer.
```

## 6. License badge (top of README)

```md
[![License: GPL-3.0](https://img.shields.io/github/license/ChaosCodify/slipstream)](https://github.com/ChaosCodify/slipstream/blob/main/LICENSE)
```

---

Notes:

- Slipstream repo already has the **description** and **topics** above set.
  Edit them anytime in Settings/About if you change your mind.
- The **LICENSE** file (GPL-3.0) is already committed — GitHub auto-detects it
  on the repo page.
- The app exits immediately on this machine (exit code 0, no window), so a
  real UI screenshot couldn't be captured — add one in `src/index.html` state
  or a render whenever you have one. A screenshot near the top of the README
  is the single biggest CTR lever for a "game booster" search comparison.