# Superpowers

A native mod for [Horizon](https://github.com/horizon-rust/horizon), the Rust rewrite of Minecraft: Java Edition
26.3. It adds twelve super powers, five of them mega powers, plus elytra-style **Superflight** with barrel rolls and
loops. On top of that there's the **CATACLYSM**, a crater 40 chunks across, along with a keybind panel and aurora night
skies. Built for **Horizon Nightly 2026-11-18**.

![Every power, drawn by the mod's own shaders](docs/preview-powers.png)

*Each picture is the mod's own quads and shaders, drawn the way the game draws them, on a software GPU over a
stand-in scene (the grass and stone pillars are fake). In the game they draw over your world.*

## Powers

All keys can be rebound in Horizon's Controls, under **Superpowers**. **H** shows and hides the panel, which lists
every key, cooldown and the laser power.

| Key | Power | What it does |
| --- | --- | --- |
| **Z** | Super Punch | Shatters a cone of blocks 16 long. Debris flies, mobs are hurt and thrown, and a shockwave races across the ground. |
| **G** | Laser Eyes | Toggles the beams. **Each right-click raises the power from 1 to 5**, then wraps back to 1. They burn through blocks and set mobs alight. |
| **V** | Thunder Strike | Seven lightning bolts strike around your target. |
| **B** | Frost Nova | Water freezes, lava turns to obsidian and snow settles within 14 blocks. Mobs are frozen and slowed. |
| **N** | Gravity Well | A black hole pulls in mobs, items and torn-up blocks for 5 seconds, then implodes. |
| **J** | Hyper Dash | Launches you forward, with a rainbow trail and a sonic boom. |
| **R** | **Superflight** | Fly freely, steering with the mouse, as with an elytra but with no gliding (see below). |

### Mega powers (×5)

| Key | Power | What it does |
| --- | --- | --- |
| **Y** | Titan Fist | Super Punch ×5: a cone 80 blocks long and a shockwave 170 blocks out. |
| **U** | Omega Beam | Laser Eyes ×5: a giant beam that carves a tunnel 200 blocks long for 4 seconds, following your aim. |
| **I** | Wrath of the Gods | Thunder ×5: 36 bolts across a circle 92 blocks wide, each leaving a crater. |
| **O** | Absolute Zero | Frost ×5: freezes 70 blocks round you, raises ice spikes and adds a blizzard of frost. |
| **K** | Singularity | Gravity Well ×5: a black hole that pulls from 120 blocks away, eats a ball of land 48 blocks across, then explodes. |
| **M** (hold 2 s) | **CATACLYSM** | Hold to charge, aim where you look, and let it build. It blasts a crater **640 blocks (40 chunks) across** and up to 90 deep, wiping out everything in it. Let go early to call it off. |

## Superflight

| Control | Does |
| --- | --- |
| Mouse | Steers the nose in your own frame: upside down, pulling up still pulls up. |
| W / S | Thrust and airbrake. |
| Sprint + W | Boost (about 60 blocks a second). |
| A / D | Roll. |
| Double-tap A or D | A full barrel roll that way. |
| **X** | A loop. |
| Space / Sneak | Climb and sink. |
| R | Land (stop flying). |

You take no fall damage while flying, nor for ten seconds after landing. Contrails stream from your wingtips, and the
view widens with speed. The HUD shows your speed, height and roll on a horizon line. The camera rolls all the way round:
the mod carries one small patch to the game (a `[[native.mixin]]` in `horizon-mod.toml`), because the game otherwise
limits a mod's camera roll to 10 degrees.

> [!WARNING]
> **The powers change the world for good.** Use a throwaway world or back yours up. They never break bedrock or the
> bottom five layers of the world. The small powers only break blocks a tool can mine (plus plants, glass and wool);
> the Cataclysm wipes out everything in its crater. It only changes chunks that are loaded, so beyond your render
> distance the land stays as it was.

## Install

1. **Use Horizon Nightly 2026-11-18.** Native mods are built against the exact game build they pin, and 11-18 is the
   build this mod is made for. Install it in the launcher and move your instance onto it (**Rebuild on Horizon
   Nightly**), or make a new instance on it.
2. If the instance still has an **older Superpowers** (the one with `horizon-mixin.toml`), remove it. Turn off mods
   pinned to other builds, such as the Orbital Laser version pinned to Horizon `e57b1d3`, or keep them in a different
   instance.
3. On the **Mods** page, choose **Add from GitHub** and paste a commit link
   (`https://github.com/pressflowed/minecraft-mod/commit/<sha>`). The repository must be public.
4. Press **Build**, then **I trust these mods: build**. Native mods run unsandboxed; all of this one's code is in
   [`src/`](src).
5. Play. Press **H** for the panel. Type `/time set midnight` to see the aurora.

## How it was checked

- The code compiles against the real mod SDK of Nightly 2026-11-18, and its unit tests pass. They cover the flight
  physics (barrel rolls and loops come back level, steering works in the flyer's frame), the crater geometry, the
  cast message and the key layout.
- All 13 shaders are wrapped in the game's own prelude and entry points. They pass wgpu's validator and translate for
  Vulkan, Metal and DirectX.
- The previews above were drawn from the mod's real quads with the game's own blending and depth rules.
- It hasn't run inside the game yet. If the build fails, the Mods page and `native/build.log` say why. If something
  looks or feels wrong in play (the direction of a roll, how strong an effect is), it's quick to tune.

## Layout

| File | What |
| --- | --- |
| `horizon-mod.toml` | The manifest: the pinned build, and the camera-roll patch. |
| `src/lib.rs` | The hooks. |
| `src/powers.rs` | Every power's key, name and cooldown. |
| `src/client.rs` | Keys, cooldowns, effects, Superflight, camera feel and sounds. |
| `src/flight.rs` | The flight physics. |
| `src/server.rs` | What each power does to the world, in each level's tick. |
| `src/draw.rs` | Every effect as shader quads. |
| `src/hud.rs` | The panel and the flight instruments. |
| `assets/superpowers/shaders/pipeline/*.wgsl` | The shaders. |
| `src/mixins/unbounded_shake.rs` | The camera-roll patch. |
