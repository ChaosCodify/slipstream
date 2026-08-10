[![License: GPL-3.0](https://img.shields.io/github/license/ChaosCodify/slipstream)](https://github.com/ChaosCodify/slipstream/blob/main/LICENSE)

## TL;DR

Agent Coded Software: You've been warned

You pick games to **Watch**. When a watched game is running, Slipstream
maxes your power plan and closes whatever background junk you told it
to close. When the game closes, it puts everything back.

It never launches the game and never touches the game's process
directly — it only watches from outside. That's on purpose: touching
the game process is the kind of thing that gets flagged by anti-cheat,
so this just doesn't do that.

No internet calls anywhere in this thing. Go check `main.rs` yourself
if you don't believe me.

Windows only. Fork it, break it, fix it — that's the point.

You can tag me in posts with your screenshots and ill add some to this:
<img width="1100" height="760" alt="image" src="https://github.com/user-attachments/assets/877ef486-6e21-4406-91c5-85eeef579990" />

---

## License

GPL-3.0 — full text in [LICENSE](LICENSE).

**Not affiliated with Razer.** "Razer Cortex" and "Razer" are trademarks of
Razer Inc. This is an independent, community-built alternative and is in no
way endorsed by or connected to Razer.

---

# Slipstream — README content blocks

Ready-to-paste blocks for the README / About panel. Grab what you want.



## 1. About description (repo About panel, right side under the repo name)


Open-source, telemetry-free Razer Cortex alternative for Windows. Boosts games without launching or touching the game process.


## 2. Topics (About panel → "Topics")


razer-cortex-alternative game-booster windows tauri foss open-source no-telemetry gaming-performance rust game-launcher


## 3. Tagline / intro paragraph


**Open-source, telemetry-free Razer Cortex alternative for Windows.** A local
"boost & watch" utility: it switches your PC to High Performance while you're
playing a game and closes the background junk (Discord, browsers, overlays)
that steals your frames — then restores your normal power plan when you're
done. No account, no telemetry, no bundled bloat. Built with Tauri (Rust).


## 4. Comparison table

## Slipstream vs. Razer Cortex

|                      | Razer Cortex        | Slipstream                     |
|----------------------|---------------------|--------------------------------|
| Requires account     | Yes                 | No                             |
| Telemetry            | Yes                 | None — check the source        |
| Open source          | No                  | Yes (GPL-3.0)                  |
| Launches or touches the game process | Yes | No — it only watches    |
| Extra bundled apps / bloat | Yes            | No                             |
| Works offline        | No                  | Yes                            |


## 5. Disclaimer (recommended, put somewhere visible)

**Not affiliated with Razer.** "Razer Cortex" and "Razer" are trademarks of
Razer Inc. This is an independent, community-built alternative and is in no
way endorsed by or connected to Razer.





---

Notes:

- Slipstream repo already has the **description** and **topics** above set.
  Edit them anytime in Settings/About if you change your mind.
- The **LICENSE** file (GPL-3.0) is already committed — GitHub auto-detects it
  on the repo page (the badge above renders from it).
- A screenshot sits near the top of the README — it's the single biggest CTR
  lever for a "game booster" search comparison.
