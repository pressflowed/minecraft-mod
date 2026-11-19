//! Superpowers: twelve super powers with big visuals, Superflight with barrel rolls and loops, a
//! 40-chunk Cataclysm, the Storm Scepter and the Gravity Gun, five fully simulated jets, a keybind
//! panel, and aurora night skies.
//!
//! A native mod (`horizon-mod.toml`): the launcher builds this crate against the pinned Horizon
//! and loads the `impl Hooks` below. Three mixins patch the game: the camera's shake is freed to
//! turn all the way round, and the effects and the jets draw on the GPU once the world is drawn.
//!
//! - [`host`]: the game as the rest of the mod calls it.
//! - [`client`]: keys, cooldowns, the effects being drawn, Superflight, camera feel, sounds.
//! - [`server`]: what the powers do to the world, in each level's tick.
//! - [`render`] and [`shader`]: what the two effect passes draw (a lens pass and a light pass).
//! - [`gfx`]: what the `draw` mixin asks each frame.
//! - [`hud`]: the panel, the read-outs and the pickers.
//! - [`items`] (the window) and [`storms`] (the world): the Storm Scepter's thirteen events and
//!   the Gravity Gun; [`weather`] holds what both sides work out alike.
//! - The jets: [`jet_sim`] (the flight model), [`jet_mesh`] (the models and cockpit),
//!   [`jet_shader`] (drawing them), [`jet`] (flying one, in the window) and [`jet_server`]
//!   (weapons in the world), talking over [`jet_net`].

mod client;
mod flight;
pub mod gfx;
mod host;
mod hud;
mod items;
mod jet;
mod jet_mesh;
mod jet_net;
mod jet_server;
mod jet_shader;
mod jet_sim;
mod net;
mod render;
mod server;
mod shader;
mod storms;
mod weather;

use host::Host;
use mod_api::{
    CameraDistanceEvent, CameraPoseEvent, CameraShakeEvent, ClientHost, ClientTickPostEvent,
    CommandEvent, ComputeFovEvent, EntityJoinLevelEvent, EntityLeaveLevelEvent, Flow,
    GuiRenderPostEvent, Hooks, HostSide, IPayloadContext, IPayloadHandler, KeyMappingPreEvent,
    KeyRegistration, LevelTickPostEvent, LivingFallEvent, LoopContext, MouseScrollingEvent,
    MovementInputEvent, PayloadBytes, PayloadHandleError, PayloadId, Registrar,
    RegistrationContext, RenderFramePreEvent, RenderHandEvent, RenderPlayerPreEvent,
    ScreenClosingEvent, ScreenOpening, ScreenOpeningEvent, ServerHost, ServerTickEvent,
    ServerTickPhase,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The mod's id, as `horizon-mod.toml` names it: its keys and its message channels.
pub const ID: &str = "superpowers";
pub const NAME: &str = "Superpowers";
pub const VERSION: &str = "0.7.0";

/// The game's keys held now, under their old names.
static HELD: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());

/// The sprint key held (Superflight's boost, a jet's afterburner).
static SPRINT: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct Mod;

/// The powers' messages, from a window to the server.
struct PowerMessages;

impl IPayloadHandler for PowerMessages {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), PayloadHandleError> {
        let Ok(player) = context.player() else {
            return Ok(());
        };
        if let Ok(host) = context.server_host() {
            let mut host = Host::server(host);
            if host.on(ID) {
                server::message(&mut host, player.uuid(), &payload.0);
            }
        }
        Ok(())
    }
}

/// The jets' messages, both ways.
struct JetMessages;

impl IPayloadHandler for JetMessages {
    fn handle(&self, payload: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), PayloadHandleError> {
        if let Ok(host) = context.client_host() {
            let mut host = Host::client(host);
            jet::message(&mut host, &payload.0);
            return Ok(());
        }
        let Ok(player) = context.player() else {
            return Ok(());
        };
        if let Ok(host) = context.server_host() {
            let mut host = Host::server(host);
            if host.on(ID) {
                jet_server::message(&mut host, player.uuid(), &payload.0);
            }
        }
        Ok(())
    }
}

/// The game's own keys, under the names the mod's key handling knows them by.
fn old_name(mapping: &str) -> Option<&'static str> {
    Some(match mapping {
        "key.attack" => "key.mouse.left",
        "key.use" => "key.mouse.right",
        "key.pickItem" => "key.mouse.middle",
        "key.left" => "key.keyboard.a",
        "key.right" => "key.keyboard.d",
        "key.forward" => "key.keyboard.w",
        "key.back" => "key.keyboard.s",
        _ => return None,
    })
}

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
        let category = "key.categories.superpowers";
        for (key, label, default) in client::all_keys() {
            r.register_key_mapping(KeyRegistration::new(key, default).label(label).category(category));
        }
        let mut handlers = r.payload_handlers();
        let mut registrar = handlers.registrar("1");
        registrar.play_to_server(PayloadId(net::CHANNEL.into()), Arc::new(PowerMessages));
        registrar.play_bidirectional(PayloadId(jet_net::CHANNEL.into()), Arc::new(JetMessages), Some(Arc::new(JetMessages)));
    }

    fn client_tick_post(&self, host: &mut dyn ClientHost, _event: &mut ClientTickPostEvent) {
        let on = host.on(ID);
        gfx::ENABLED.store(on, Ordering::Relaxed);
        if !on || !host.playing() {
            return;
        }
        let mut host = Host::client(host);
        client::poll_keys(&mut host);
        if let Some(camera) = host.camera() {
            client::tick(&mut host, camera);
        }
    }

    fn render_frame_pre(&self, host: &mut dyn ClientHost, _event: &mut RenderFramePreEvent) {
        if host.on(ID) && host.playing() {
            client::frame(&mut Host::client(host));
        }
    }

    /// The camera rides the jet, every frame, smoothly; Superflight's loops go all the way round.
    fn camera_pose(&self, host: &dyn ClientHost, event: &mut CameraPoseEvent<'_>) {
        if !host.on(ID) {
            return;
        }
        let mut pose = (*event.position, event.yaw, event.pitch);
        let third = !host.camera_type().is_first_person();
        if jet::camera_pose(event.partial, third, &mut pose) || client::camera_pose(event.partial, &mut pose) {
            *event.position = pose.0;
            event.yaw = pose.1;
            event.pitch = pose.2;
        }
    }

    fn camera_shake(&self, host: &dyn ClientHost, event: &mut CameraShakeEvent<'_>) {
        if host.on(ID) {
            client::shake(event.shake, event.partial);
        }
    }

    fn compute_fov(&self, host: &dyn ClientHost, event: &mut ComputeFovEvent) {
        if host.on(ID) {
            // past the shake's share, the roll turns the picture, which is shown wider to fit
            let picture = gfx::split_roll(client::roll(event.partial)).1;
            event.degrees = gfx::turn_picture(picture, client::fov(event.degrees));
        } else {
            gfx::turn_picture(0.0, event.degrees);
        }
    }

    fn camera_distance(&self, host: &dyn ClientHost, event: &mut CameraDistanceEvent<'_>) {
        if host.on(ID) {
            jet::camera_distance(&mut event.distance);
        }
    }

    fn movement_input(&self, host: &dyn ClientHost, event: &mut MovementInputEvent) {
        if host.on(ID) {
            SPRINT.store(event.sprint, Ordering::Relaxed);
            client::movement(&mut event.forward, &mut event.strafe, &mut event.jump, &mut event.sneak, event.sprint);
        }
    }

    /// The game's keys: the mouse buttons fire what the player holds or flies; A and D roll.
    fn key_mapping_pre(&self, host: &mut dyn ClientHost, event: &mut KeyMappingPreEvent<'_>) -> Flow {
        if !host.on(ID) {
            return Flow::Continue;
        }
        let Some(name) = old_name(event.mapping) else {
            return Flow::Continue;
        };
        // a held key repeats its press: only the first is a press
        let first = {
            let mut held = HELD.lock().unwrap_or_else(|p| p.into_inner());
            let was = held.contains(&name);
            held.retain(|n| *n != name);
            if event.pressed {
                held.push(name);
            }
            event.pressed != was
        };
        let mut host = Host::client(host);
        let claimed = first && client::key(&mut host, name, event.pressed);
        // a held item or a jet keeps the buttons it uses from the game
        let button = match event.mapping {
            "key.attack" => Some(0),
            "key.use" => Some(1),
            "key.pickItem" => Some(2),
            _ => None,
        };
        let keep = button.is_some_and(|b| jet::claims(b) || items::claims(b) || (b == 1 && client::lasers_on()));
        if keep || claimed && button.is_some() { Flow::Cancel } else { Flow::Continue }
    }

    fn mouse_scrolling(&self, host: &mut dyn ClientHost, event: &mut MouseScrollingEvent) -> Flow {
        if !host.on(ID) {
            return Flow::Continue;
        }
        let mut host = Host::client(host);
        if jet::scroll(&mut host, event.scroll[1]) || items::scroll(&mut host, event.scroll[1]) {
            return Flow::Cancel;
        }
        Flow::Continue
    }

    fn screen_opening(&self, _host: &dyn ClientHost, _event: &mut ScreenOpeningEvent<'_>) -> Option<ScreenOpening> {
        client::screen_open(true);
        None
    }

    fn screen_closing(&self, _host: &dyn ClientHost, _event: &mut ScreenClosingEvent<'_>) {
        client::screen_open(false);
    }

    fn gui_render_post(&self, host: &dyn ClientHost, event: &mut GuiRenderPostEvent<'_>) {
        if host.on(ID) && !event.gui_hidden {
            hud::draw(host, event.batch);
        }
    }

    /// In a jet the player sits inside it: their body isn't drawn, nor their hand.
    fn render_player_pre(&self, host: &dyn ClientHost, event: &mut RenderPlayerPreEvent<'_>) -> Flow {
        if host.on(ID) && event.who.local && jet::hides_player() {
            return Flow::Cancel;
        }
        Flow::Continue
    }

    fn render_hand(&self, host: &dyn ClientHost, _event: &mut RenderHandEvent<'_, '_>) -> Flow {
        if host.on(ID) && jet::hides_player() {
            return Flow::Cancel;
        }
        Flow::Continue
    }

    /// `/superpowers give`: the Storm Scepter and the Gravity Gun.
    fn command(&self, host: &mut dyn ClientHost, event: &mut CommandEvent<'_>) -> Flow {
        if !host.on(ID) {
            return Flow::Continue;
        }
        let words: Vec<&str> = event.command.split_whitespace().collect();
        if words.first() != Some(&ID) {
            return Flow::Continue;
        }
        if words.get(1) == Some(&"give") {
            let mut host = Host::client(host);
            if let Some(camera) = host.camera() {
                let dir = client::look(&camera);
                client::send(&mut host, net::Act::Give, &camera.dimension, camera.position, dir, 1);
            }
        }
        Flow::Cancel
    }

    fn server_tick(&self, _host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        if matches!(event.phase, ServerTickPhase::ServerTickStart { .. }) {
            server::tick();
        }
    }

    fn level_tick_post(&self, host: HostSide, event: &mut LevelTickPostEvent) {
        let HostSide::Server(host) = host else {
            return;
        };
        if !host.on(ID) {
            return;
        }
        let Some(dimension) = event.level.dimension().id(&*host) else {
            return;
        };
        host::SERVER_LEVEL.with(|l| l.set(Some(event.level)));
        server::level_tick(&mut Host::server(host), &dimension);
        host::SERVER_LEVEL.with(|l| l.set(None));
    }

    /// No fall damage while flying, nor for a while after landing, a dash or a Cataclysm.
    fn living_fall(&self, _cx: &dyn LoopContext, event: &mut LivingFallEvent<'_>) -> Flow {
        if event.kind == "minecraft:player" && client::fall_protected() {
            event.distance = 0.0;
            event.multiplier = 0.0;
            return Flow::Cancel;
        }
        Flow::Continue
    }

    fn entity_join_level(&self, _host: &mut dyn ServerHost, event: &mut EntityJoinLevelEvent<'_>) -> Flow {
        server::joined(event.dimension, event.entity, event.kind);
        Flow::Continue
    }

    fn entity_leave_level(&self, _host: &mut dyn ServerHost, event: &mut EntityLeaveLevelEvent<'_>) {
        server::left(event.entity);
    }
}

/// The sprint key, as the movement hook last said.
pub fn sprinting() -> bool {
    SPRINT.load(Ordering::Relaxed)
}
