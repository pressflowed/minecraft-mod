//! Superpowers: twelve super powers with big visuals, elytra-style Superflight with barrel rolls
//! and loops, a 40-chunk Cataclysm, a keybind panel, and aurora night skies.
//!
//! - [`client`]: keys, cooldowns, the effects being drawn, Superflight, camera feel, sounds.
//! - [`server`]: what the powers do to the world, in each level's tick.
//! - [`draw`]: every effect as quads of the mod's own shaders (`assets/superpowers/shaders`).
//! - [`hud`]: the powers panel and the flight instruments.

mod client;
mod draw;
mod flight;
mod hud;
mod net;
mod powers;
mod server;

use mod_api::{
    CameraPoseEvent, CameraShakeEvent, ClientHost, ClientResourcesReloadedEvent, ClientSetupEvent,
    ClientTickPostEvent, ComputeFovEvent, Flow, GuiRenderPostEvent, Hooks, HostSide,
    IPayloadContext, KeyMappingPreEvent, KeyRegistration, LevelTickPostEvent, LivingFallEvent,
    LoopContext, MovementInputEvent, PayloadId, RegisterRenderPipelinesEvent, Registrar,
    RegistrationContext, RenderFramePreEvent, ServerHost, ServerTickEvent, ServerTickPhase,
    SubmitCustomGeometryEvent, ToServer,
};
use std::sync::OnceLock;

pub const ID: &str = "superpowers";
pub const NAME: &str = "Superpowers";
pub const VERSION: &str = "0.3.0";

/// For previews outside the game: the client's state and this frame's quads.
#[doc(hidden)]
pub mod preview {
    pub use crate::client::{Beam, Effect, Kind, state};
    pub use crate::draw::quads_for;
    pub use crate::powers::bolts;
}

/// The cast message's sender, made at registration.
static CAST: OnceLock<ToServer<net::Cast>> = OnceLock::new();

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
        let category = "key.categories.superpowers";
        for info in powers::ALL.iter() {
            r.register_key_mapping(KeyRegistration::new(info.key, info.default).label(info.label).category(category));
        }
        for (key, label, default) in [powers::PANEL, powers::LOOP] {
            r.register_key_mapping(KeyRegistration::new(key, default).label(label).category(category));
        }
        let token = r.payload_handlers().registrar(net::VERSION).play_to_server_typed(
            PayloadId(net::CAST.into()),
            |cast: net::Cast, cx: &mut dyn IPayloadContext| {
                let player = cx.player().map_err(|e| format!("{e:?}"))?;
                server::receive(player.uuid(), cast);
                Ok(())
            },
        );
        let _ = CAST.set(token);
    }

    fn register_render_pipelines(&self, _cx: &RegistrationContext, event: &mut RegisterRenderPipelinesEvent<'_>) {
        draw::register(event.pipelines);
    }

    fn client_setup(&self, host: &mut dyn ClientHost, _event: &mut ClientSetupEvent) {
        draw::resolve(host);
    }

    fn client_resources_reloaded(&self, host: &mut dyn ClientHost, _event: &mut ClientResourcesReloadedEvent) {
        draw::resolve(host);
    }

    fn client_tick_post(&self, host: &mut dyn ClientHost, _event: &mut ClientTickPostEvent) {
        client::tick(host);
    }

    fn render_frame_pre(&self, host: &mut dyn ClientHost, _event: &mut RenderFramePreEvent) {
        if client::flying() {
            client::flight_frame(host);
        }
    }

    fn camera_pose(&self, _host: &dyn ClientHost, event: &mut CameraPoseEvent<'_>) {
        if let Some((yaw, pitch)) = client::flight_view() {
            event.yaw = yaw;
            event.pitch = pitch;
        }
    }

    fn camera_shake(&self, _host: &dyn ClientHost, event: &mut CameraShakeEvent<'_>) {
        client::shake(event.shake);
    }

    fn compute_fov(&self, _host: &dyn ClientHost, event: &mut ComputeFovEvent) {
        event.degrees = client::fov(event.degrees);
    }

    fn movement_input(&self, _host: &dyn ClientHost, event: &mut MovementInputEvent) {
        client::movement(&mut event.forward, &mut event.strafe, &mut event.jump, &mut event.sneak, event.sprint);
    }

    /// While the laser eyes are on, the use key raises their power instead of using the held
    /// item; while flying, a double tap left or right does a barrel roll.
    fn key_mapping_pre(&self, _host: &mut dyn ClientHost, event: &mut KeyMappingPreEvent<'_>) -> Flow {
        if !event.pressed {
            return Flow::Continue;
        }
        if event.mapping == "key.use" && client::use_key() {
            return Flow::Cancel;
        }
        client::tap(event.mapping);
        Flow::Continue
    }

    fn submit_custom_geometry(&self, host: &dyn ClientHost, event: &mut SubmitCustomGeometryEvent<'_>) {
        draw::world(host, event.view, event.draw);
    }

    fn gui_render_post(&self, host: &dyn ClientHost, event: &mut GuiRenderPostEvent<'_>) {
        if !event.gui_hidden {
            hud::draw(host, event.batch);
        }
    }

    fn server_tick(&self, _host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        if matches!(event.phase, ServerTickPhase::ServerTickStart { .. }) {
            server::server_tick();
        }
    }

    fn level_tick_post(&self, host: HostSide, event: &mut LevelTickPostEvent) {
        if let HostSide::Server(host) = host {
            server::level_tick(host, event.level);
        }
    }

    /// No fall damage while flying, nor for a while after landing or a Cataclysm.
    fn living_fall(&self, _cx: &dyn LoopContext, event: &mut LivingFallEvent<'_>) -> Flow {
        if server::fall_protected(event.entity) {
            event.distance = 0.0;
            event.multiplier = 0.0;
            return Flow::Cancel;
        }
        Flow::Continue
    }
}
