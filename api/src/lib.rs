//! Superpowers: six super powers with big visuals, a keybind panel, and aurora night skies.
//!
//! A trait mod (Horizon's docs/native-mods.md, "Trait mods"): the launcher links this crate into
//! the game and calls [`hooks`]. Every hook first checks that the player left the mod on.
//!
//! - [`client`]: the window's half. Keys, cooldowns, the effects being drawn, camera shake,
//!   field of view and sounds; each power tells the server what to do.
//! - [`server`]: the world's half. It shatters, burns and freezes blocks, throws debris, calls
//!   lightning, pulls things into black holes and hurts and pushes mobs.
//! - [`render`] and [`shader`]: two GPU passes after the world is drawn, a lens pass (black
//!   holes, shock ripples, frost) and a light pass (everything that glows, and the aurora).
//! - [`hud`]: the panel listing every power, its key and its cooldown.

mod client;
mod hud;
mod net;
mod render;
mod server;
mod shader;

use horizon_client::mod_api::{
    CameraShakeEvent, ComputeFovEvent, EntityJoinLevelEvent, EntityLeaveLevelEvent, Flow,
    FrameEvent, Hooks, Host, HudEvent, InteractionKeyEvent, KeyEvent, LevelTickPostEvent,
    ModsEvent, NetFromClientEvent, RenderHooks, ScreenClosingEvent, ScreenOpeningEvent, TickEvent,
};

/// The mod's id, as `horizon-mixin.toml` names it: its keys' lines and its message channel.
pub const ID: &str = "superpowers";
pub const NAME: &str = "Superpowers";
pub const VERSION: &str = "0.2.0";

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

    /// The mod's row on Options > Mods, with its keys, which the player rebinds there.
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
        if host.on(ID) {
            client::key(host, event.key, event.pressed);
        }
    }

    /// While the laser eyes are on, the use key fires them instead of using the held item.
    fn interaction_key(&self, host: &mut dyn Host, event: &mut InteractionKeyEvent) -> Flow {
        if host.on(ID) && event.button == 1 && client::lasers_on() {
            event.swing_hand = false;
            return Flow::Cancel;
        }
        Flow::Continue
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
