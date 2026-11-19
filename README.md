# Superpowers

A native mod for [Horizon](https://github.com/horizon-rust/horizon), the Rust rewrite of Minecraft: Java Edition
26.3. It adds twelve super powers, five of them mega powers, plus elytra-style **Superflight** with barrel rolls and
loops. On top of that there's the **CATACLYSM**, a crater 40 chunks across, two items (the **Storm Scepter**, which
calls down 13 disasters, and a **Gravity Gun**), a keybind panel and aurora night skies. Built on Horizon commit `e57b1d3`, the build Orbital Laser pins, so the two run in one game, and for
launchers that read native mods as `horizon-mixin.toml` bundles.

![Every power, drawn by the mod's own shaders](docs/preview-powers.png)

*Drawn on a software GPU over a stand-in scene (the grass and stone pillars are fake) by this mod's effects as
they were first built for the newer mod API. This build draws the same effects with its own GPU pass, so the
look is close but not identical. In the game they draw over your world.*

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

## Items

![The Storm Scepter's events and the Gravity Gun, drawn by the mod's own shaders](docs/preview-storms.png)

*Drawn on a software GPU over a flat stand-in scene. In the game they draw over your world, and the blocks they
throw, the craters, the water and the lava are real.*

Type **`/superpowers give`** in chat to get both. They are a breeze rod and an echo shard with new names: any breeze
rod or echo shard works.

### Storm Scepter (breeze rod)

| Control | Does |
| --- | --- |
| Right-click | Calls the chosen event down wherever your cursor points, as far as the world is drawn (up to 1024 blocks). A ring on the ground shows where and how big. |
| Hold **R** | Opens the picker (R doesn't toggle flight while you hold the scepter). **Scroll** or **A / D** to choose, let go of R to keep it. |

| Event | What it does to the world |
| --- | --- |
| **EF1 Tornado** | A rope funnel wanders across the land for 20 s, stripping leaves, grass and flowers. |
| **EF2 Tornado** | Snaps trees and tears up wooden buildings and glass. |
| **EF3 Tornado** | Also scours the soil from its path. |
| **EF4 Tornado** | A wide funnel that flattens anything wooden and scours deeper. |
| **EF5 Tornado** | 38 blocks wide; grinds its path down to bare stone. Lightning flickers in its wall cloud. Every tornado flings debris blocks and sucks in mobs. |
| **Hurricane** | A spiral cloud deck 280 blocks across with a calm eye. Rain, lightning, wind that strips the land and throws mobs, and a storm surge that floods low ground. |
| **Tsunami** | A 28-block wave 150 wide rises at the cursor and rolls at **you**, sweeping away trees, buildings and mobs, and breaks just short of you. It leaves real water behind. |
| **Flood** | Rain, then the water rises 9 blocks over a 48-block radius, filling the low ground and leaving a lake. |
| **Solar Flare** | A column of the sun's fire reaches the ground. Water boils away, snow and ice melt, sand turns to glass, rock melts to magma and fires start. |
| **Nuke** | A flash, a fireball, a shockwave and a mushroom cloud 150 blocks tall. It leaves a crater 88 blocks across, flattens and burns the land out to 150 blocks, and the sound arrives late. |
| **Meteor** | A fireball streaks in from the sky, blasts a 34-block crater with ejecta flying out, and leaves a glowing meteorite of ancient debris and magma at the bottom. |
| **Earthquake** | The ground splits along jagged faults up to 90 blocks long and 26 deep, lava at the bottom of the widest, while blocks tumble and the camera shakes hard. |
| **Volcano** | A mountain 42 blocks tall and 80 across rises out of the ground, its crater fills with lava that spills down its sides, and it erupts lava bombs under a column of ash with lightning in it. |

### Gravity Gun (echo shard)

| Control | Does |
| --- | --- |
| Right-click | Doubles the size of what it picks: 1, 2, 4, 8 blocks across, then back to 1. A glowing box shows the cube. |
| Left-click | Rips that cube out of the world. The blocks float in front of you and follow your aim. |
| Left-click again | Flings them where you look. They hurt what they hit and land as blocks. |
| Right-click while holding | Lets them drop. |

## Superflight

| Control | Does |
| --- | --- |
| Mouse | Steers: you fly where you look. |
| W / S | Thrust and airbrake. |
| Sprint (Ctrl) + W | Boost (about 60 blocks a second). |
| A / D | Bank and slide sideways; turning banks you too. |
| Double-tap A or D | A full barrel roll that way, corkscrewing round your line of flight. |
| **X** | A loop: the camera and your flight go up, over and round. |
| Space / Sneak | Climb and sink. |
| R | Land (stop flying). |

You take no fall damage while flying, nor for ten seconds after. The view widens with speed, and speed lines stream
past when you boost. The camera rolls and loops all the way round: the mod carries one small patch to the game (the
`[[injection]]` in `horizon-mixin.toml`), because the game otherwise limits a mod's camera turn to 10 degrees.

> [!WARNING]
> **The powers change the world for good.** Use a throwaway world or back yours up. They never break bedrock or the
> bottom five layers of the world. The small powers only break blocks a tool can mine (plus plants, glass and wool);
> the Cataclysm wipes out everything in its crater. It only changes chunks that are loaded, so beyond your render
> distance the land stays as it was. The Storm Scepter's events are just as permanent: floods, tsunamis and
> volcanoes leave water and lava that flows.

## Install

1. In the launcher's **Mods** page, remove any older **superpowers**.
2. Choose **Add from GitHub** and paste a commit link (`https://github.com/pressflowed/minecraft-mod/commit/<sha>`).
   The repository must be public.
3. Press **Build**, then **I trust these mods: build**. Native mods run unsandboxed; all of this one's code is in
   [`api/src`](api/src).
4. Play. Press **H** for the panel. Type `/time set midnight` to see the aurora.

## How it was checked

- The code type-checks against Horizon's own source at commit `e57b1d3`, the build it is made for, and its
  unit tests pass (the flight physics, the bolt scatter, the cast message, the key layout).
- The shaders pass wgpu's validator and translate for Vulkan, Metal and DirectX. The storm visuals were drawn on a
  software GPU over a stand-in scene to tune them.
- It hasn't run inside the game yet. If the build fails, the Mods page and `native/build.log` say why. If something
  looks or feels wrong in play, it's quick to tune.

## Layout

| File | What |
| --- | --- |
| `horizon-mixin.toml` | The bundle: the pinned build, the `[api]` crate, and the camera patch. |
| `mixins/unbounded_shake.rs` | The camera patch. |
| `api/src/lib.rs` | The hooks. |
| `api/src/client.rs` | Every power's key and cooldown; effects, Superflight, camera feel and sounds. |
| `api/src/flight.rs` | Vector maths for flight. |
| `api/src/server.rs` | What each power does to the world, in each level's tick. |
| `api/src/render.rs`, `api/src/shader.rs` | The GPU passes and their WGSL. |
| `api/src/hud.rs` | The panel, the flight read-out and the Cataclysm's charge. |
