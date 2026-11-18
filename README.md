# Superpowers

A native mod for [Horizon](https://github.com/horizon-rust/horizon), the Rust rewrite of Minecraft: Java Edition
26.3. It adds **six super powers**, each with its own key and its own GPU-drawn effects, a **keybind panel**, and
**aurora night skies**.

![Super Punch, Laser Eyes, Thunder Strike, Frost Nova, Gravity Well and Hyper Dash](docs/preview-powers.png)

*Top: Super Punch, Laser Eyes (power 5), Thunder Strike. Bottom: Frost Nova, Gravity Well, Hyper Dash. These were
drawn by the mod's own code on a software GPU, over a stand-in scene of grass and stone pillars, with each power
fired through the mod's real key handler. In the game they draw over your world.*

## The powers

| Key | Power | What it does | What you see |
| --- | --- | --- | --- |
| **Z** | **Super Punch** | Shatters a cone of blocks 16 long in front of you; part of it flies off as debris. Mobs in it take big damage and are thrown. | Rings of shock flying out from your fist, a white flash, a shockwave racing 34 blocks across the ground, heavy screen shake. |
| **G** | **Laser Eyes** | Toggles the beams. **Each right-click raises the power, 1 to 5, then back to 1.** They burn through blocks and set mobs on fire, faster and wider at higher power. | Two beams from your eyes with white-hot cores, red, orange, pink, violet, then cyan as the power rises; a flaring, molten spot where they hit. |
| **V** | **Thunder Strike** | Calls a bolt onto what you look at, then six more around it. | Jagged bolts that flicker and strike again, a blue shockwave, a flash. |
| **B** | **Frost Nova** | Freezes water to ice and lava to obsidian, and lays snow in a circle of 14 blocks. Mobs are frozen, slowed and hurt. | Frost spreading over the ground, sparkling ice crystals, an icy shock ring, frosted screen edges. |
| **N** | **Gravity Well** | A black hole for 5 seconds: it pulls in mobs and items and tears blocks out of the ground, then implodes. | A black event horizon, **gravitational lensing** that bends the world behind it, a swirling accretion disk with a photon ring, then a violet shockwave. |
| **J** | **Hyper Dash** | Launches you forward; mobs in your path are struck and knocked aside. | A sonic boom, a rainbow trail, speed lines, and a wider field of view. |
| **H** | **Powers panel** | Shows and hides the panel: every power, its key, whether it's ready (with a cooldown bar), and the laser eyes' power. | |

Rebind any key in **Options > Mods > Superpowers**. While the laser eyes are on, right-click raises their power
instead of using the held item. Keys do nothing while a screen (chat, inventory) is open.

At night in the Overworld the sky also gets **aurora curtains**, **shooting stars** and **twinkling stars**
([preview](docs/preview-aurora.png), [shooting star](docs/preview-shooting-star.png)).

> [!WARNING]
> **Super Punch, Laser Eyes, Thunder Strike, Frost Nova and Gravity Well change the world for good.** Try them in a
> throwaway world or back yours up first. They never break bedrock, the bottom five layers of the world, chests,
> spawners or command blocks.

## Requirements

- The Horizon launcher (0.2.37 or newer) and an instance set up from your own Minecraft: Java Edition 26.3.
- This mod is pinned to Horizon commit `e57b1d34bb0dcf6bf081c19fe6d2e7f69b60cdf4`, the same one Orbital Laser
  pins, so the two build into one game.
- On Linux: a C compiler, `pkg-config`, and the ALSA and libudev development files (the launcher tells you the
  command if any are missing). Windows needs nothing extra.

## Install

1. **Close the game.**
2. In the launcher, open the **Mods** page and choose **Add from GitHub**. Paste a link to one exact commit
   (`https://github.com/pressflowed/minecraft-mod/commit/<sha>`). The launcher reads GitHub without signing in, so
   **the repository must be public**.
3. On the **Native build of {instance}** row, press **Build**, read the warning, and choose
   **I trust these mods: build**. Native mods aren't sandboxed: everything this one does is in [`api/src`](api/src).
4. Play. Press **H** for the panel.

Command-line alternative:

```bash
horizon-launcher mods add-github https://github.com/pressflowed/minecraft-mod/commit/<sha> --instance "<instance>" --trust-native-code
```

```bash
horizon-launcher native-build --instance "<instance>" --trust-native-code
```

## If the build fails

The Mods page shows the error, and the launcher keeps the full output in the instance's `native/build.log`. How the
mod was checked before release:

- Its Rust compiles against a stand-in for Horizon's mod API at the pinned commit (the same hook events, `Host`
  helpers and `wgpu` 30.0.1), and its unit tests pass.
- An end-to-end test drives every power through the mod's own hooks: key press, message to the server, then the
  block edits, commands and mob pushes the server makes.
- The shaders pass wgpu's validator (`naga`) and translate for Vulkan, Metal and DirectX.

It hasn't been built inside the real game yet, so if something doesn't compile, the build log says exactly what.

## How it works

- `horizon-mixin.toml`: the bundle. Its `[api]` package is a trait mod; it needs no injections.
- [`api/src/client.rs`](api/src/client.rs): keys, cooldowns, the effects being drawn, camera shake, field of view,
  sounds, and the message each power sends to the server.
- [`api/src/server.rs`](api/src/server.rs): what the powers do to the world. Messages wait for their dimension's
  level tick, then become one bulk block edit (`edit_blocks`, only blocks a tool mines, plants, glass and wool),
  falling-block debris, commands (`damage`, `summon lightning_bolt`, `effect`) and mob pushes.
- [`api/src/render.rs`](api/src/render.rs) and [`api/src/shader.rs`](api/src/shader.rs): two full-screen passes once
  the world is drawn. The lens pass bends a copy of the picture round black holes, ripples it along shock waves and
  frosts it. The light pass adds everything that glows. Both rebuild each pixel's position from the world's depth,
  so effects light the ground and hide behind hills.
- [`api/src/hud.rs`](api/src/hud.rs): the panel.
