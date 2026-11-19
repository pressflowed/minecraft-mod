//! Superpowers: twelve super powers with big visuals, Superflight with barrel rolls and loops, a
//! 40-chunk Cataclysm, a keybind panel, and aurora night skies.
//!
//! A trait mod (Horizon's docs/native-mods.md, "Trait mods"): the launcher links this crate into
//! the game and calls [`hooks`]. Every hook first checks that the player left the mod on.
//!
//! - [`client`]: keys, cooldowns, the effects being drawn, Superflight, camera feel, sounds.
//! - [`server`]: what the powers do to the world, in each level's tick.
//! - [`render`] and [`shader`]: two GPU passes after the world is drawn, a lens pass (black
//!   holes, shock ripples, frost) and a light pass (everything that glows, and the aurora).
//! - [`hud`]: the panel, the flight read-out and the Cataclysm's charge.
//! - [`items`] (the window) and [`storms`] (the world): the Storm Scepter's thirteen events and
//!   the Gravity Gun; [`weather`] holds what both sides work out alike.

mod client;
mod flight;
mod hud;
mod items;
mod net;
mod render;
mod server;
mod shader;
mod storms;
mod weather;

use horizon_client::mod_api::{
    CameraShakeEvent, CommandEvent, CommandsEvent, ComputeFovEvent, EntityJoinLevelEvent,
    EntityLeaveLevelEvent, Flow, FrameEvent, Hooks, Host, HudEvent, InteractionKeyEvent, KeyEvent,
    LevelTickPostEvent, LivingFallEvent, ModsEvent, MouseScrollingEvent, MovementInputEvent,
    NetFromClientEvent, RenderHooks, ScreenClosingEvent, ScreenOpeningEvent, TickEvent,
};
use std::sync::atomic::{AtomicBool, Ordering};

/// The mod's id, as `horizon-mixin.toml` names it: its keys' lines and its message channel.
pub const ID: &str = "superpowers";
pub const NAME: &str = "Superpowers";
pub const VERSION: &str = "0.5.0";

/// The sprint key held (the movement hook doesn't tell): Superflight's boost.
static SPRINT: AtomicBool = AtomicBool::new(false);

/// The launcher calls this to get the mod.
pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Superpowers {
        renderer: render::Renderer::new(),
    })
}

struct Superpowers {
    renderer: render::Renderer,
}

impl Hooks for Superpowers {
    fn render(&self) -> Option<&dyn RenderHooks> {
        Some(&self.renderer)
    }

    fn mods(&self, _host: &mut dyn Host, event: &mut ModsEvent) {
        event.mods.push(client::mod_row());
    }

    /// Every tick in every game: the window's has a camera, a server's has none.
    fn tick(&self, host: &mut dyn Host, _event: &mut TickEvent) {
        if !host.on(ID) {
            return;
        }
        match host.camera() {
            Some(camera) => client::tick(host, camera),
            None => server::tick(),
        }
    }

    fn frame(&self, host: &mut dyn Host, _event: &mut FrameEvent) {
        if host.on(ID) {
            client::frame(host);
        }
    }

    fn key(&self, host: &mut dyn Host, event: &mut KeyEvent) {
        if !host.on(ID) {
            return;
        }
        if event.key == "key.keyboard.left.control" || event.key == host.mod_key(ID, "sprint", "key.keyboard.left.control") {
            SPRINT.store(event.pressed, Ordering::Relaxed);
        }
        client::key(host, event.key, event.pressed);
    }

    /// While the laser eyes are on, the use key fires them instead of using the held item; the
    /// Storm Scepter and the Gravity Gun take the buttons they use.
    fn interaction_key(&self, host: &mut dyn Host, event: &mut InteractionKeyEvent) -> Flow {
        if host.on(ID) && (event.button == 1 && client::lasers_on() || items::claims(event.button)) {
            event.swing_hand = false;
            return Flow::Cancel;
        }
        Flow::Continue
    }

    /// The mouse wheel chooses in the Storm Scepter's picker instead of the hotbar slot.
    fn mouse_scrolling(&self, host: &mut dyn Host, event: &mut MouseScrollingEvent) -> Flow {
        if host.on(ID) && items::scroll(host, event.scroll[1]) {
            return Flow::Cancel;
        }
        Flow::Continue
    }

    fn commands(&self, host: &mut dyn Host, event: &mut CommandsEvent) {
        if host.on(ID) {
            event.commands.push(ID);
        }
    }

    /// `/superpowers give`: the Storm Scepter and the Gravity Gun.
    fn command(&self, host: &mut dyn Host, event: &mut CommandEvent) -> Flow {
        if !host.on(ID) {
            return Flow::Continue;
        }
        let words: Vec<&str> = event.command.split_whitespace().collect();
        if words.first() != Some(&ID) {
            return Flow::Continue;
        }
        if words.get(1) == Some(&"give") {
            if let Some(camera) = host.camera() {
                client::send(host, net::Act::Give, &camera.dimension, camera.position, client::look(&camera), 1);
            }
        }
        Flow::Cancel
    }

    fn movement_input(&self, host: &dyn Host, event: &mut MovementInputEvent) {
        if host.on(ID) {
            let sprint = SPRINT.load(Ordering::Relaxed);
            client::movement(&mut event.forward, &mut event.strafe, &mut event.jump, &mut event.sneak, sprint);
        }
    }

    fn screen_opening(&self, _host: &dyn Host, _event: &mut ScreenOpeningEvent) {
        client::screen_open(true);
    }

    fn screen_closing(&self, _host: &dyn Host, _event: &mut ScreenClosingEvent) {
        client::screen_open(false);
    }

    fn hud(&self, host: &mut dyn Host, event: &mut HudEvent) {
        if host.on(ID) {
            hud::draw(host, event.batch, event.corners);
        }
    }

    fn camera_shake(&self, host: &dyn Host, event: &mut CameraShakeEvent) {
        if host.on(ID) {
            client::shake(event.shake);
        }
    }

    fn compute_fov(&self, host: &dyn Host, event: &mut ComputeFovEvent) {
        if host.on(ID) {
            event.degrees = client::fov(event.degrees);
        }
    }

    /// No fall damage while flying, nor for a while after landing, a dash or a Cataclysm.
    fn living_fall(&self, event: &mut LivingFallEvent) -> Flow {
        if event.kind == "minecraft:player" && client::fall_protected() {
            event.distance = 0.0;
            event.multiplier = 0.0;
            return Flow::Cancel;
        }
        Flow::Continue
    }

    fn net_from_client(&self, host: &mut dyn Host, event: &mut NetFromClientEvent) {
        if host.on(ID) && event.channel == net::CHANNEL {
            server::message(host, event.player, event.data);
        }
    }

    fn level_tick_post(&self, host: &mut dyn Host, event: &mut LevelTickPostEvent) {
        if host.on(ID) {
            server::level_tick(host, event.dimension);
        }
    }

    fn entity_join_level(&self, _host: &mut dyn Host, event: &mut EntityJoinLevelEvent) -> Flow {
        server::joined(event.dimension, event.entity, event.kind);
        Flow::Continue
    }

    fn entity_leave_level(&self, _host: &mut dyn Host, event: &mut EntityLeaveLevelEvent) {
        server::left(event.entity);
    }
}
