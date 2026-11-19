//! The game as the rest of the mod uses it: one [`Host`] with the calls the powers, the items, the
//! storms and the jets make, answered by the window's [`ClientHost`] or the server's
//! [`ServerHost`]. A call the side can't answer returns nothing.
//!
//! Keys work by name: the mod's own keys are polled each tick and reported as
//! `superpowers:<key>`; the game's mouse and movement keys come through `key_mapping_pre` and are
//! reported under their old names (`key.mouse.left`, `key.keyboard.a`...).

use crate::ID;
pub use mod_api::{BlockRun as NativeBlockRun, Heightmap as NativeHeightmap, Shake as NativeShake};
use mod_api::{ClientHost, Compound, EntityRef, Hand, ServerHost, SoundPlay, SoundSource};

/// The camera, as the mod reads it.
#[derive(Clone, Debug)]
pub struct NativeCamera {
    pub position: [f64; 3],
    pub eye_height: f32,
    pub partial_tick: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub dimension: String,
}

/// Where a ray from the eyes meets a block.
#[derive(Clone, Debug)]
pub struct NativeHit {
    pub block_x: i32,
    pub block_y: i32,
    pub block_z: i32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    #[allow(dead_code)]
    pub block: String,
}

/// A player on the server.
#[derive(Clone, Debug)]
pub struct NativeServerPlayer {
    pub uuid: u128,
    pub name: String,
    pub dimension: String,
    pub position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
}

/// A bulk edit, its palette and filters written as commands write blocks and tags.
#[derive(Clone, Debug, Default)]
pub struct NativeBlockEdit {
    pub palette: Vec<String>,
    pub filters: Vec<String>,
    pub runs: Vec<NativeBlockRun>,
}

/// Which side a call is made from.
pub enum Side<'a> {
    Client(&'a mut dyn ClientHost),
    /// The window, to read only (drawing the HUD).
    View(&'a dyn ClientHost),
    Server(&'a mut dyn ServerHost),
}

/// The game, for the rest of the mod.
pub struct Host<'a> {
    side: Side<'a>,
}

impl<'a> Host<'a> {
    pub fn client(host: &'a mut dyn ClientHost) -> Self {
        Self { side: Side::Client(host) }
    }

    pub fn view(host: &'a dyn ClientHost) -> Self {
        Self { side: Side::View(host) }
    }

    pub fn server(host: &'a mut dyn ServerHost) -> Self {
        Self { side: Side::Server(host) }
    }

    fn client_ref(&self) -> Option<&dyn ClientHost> {
        match &self.side {
            Side::Client(h) => Some(&**h),
            Side::View(h) => Some(*h),
            Side::Server(_) => None,
        }
    }

    fn client_mut(&mut self) -> Option<&mut dyn ClientHost> {
        match &mut self.side {
            Side::Client(h) => Some(&mut **h),
            Side::View(_) | Side::Server(_) => None,
        }
    }

    fn server_ref(&self) -> Option<&dyn ServerHost> {
        match &self.side {
            Side::Server(h) => Some(&**h),
            Side::Client(_) | Side::View(_) => None,
        }
    }

    fn server_mut(&mut self) -> Option<&mut dyn ServerHost> {
        match &mut self.side {
            Side::Server(h) => Some(&mut **h),
            Side::Client(_) | Side::View(_) => None,
        }
    }

    fn any(&self) -> &dyn mod_api::Host {
        match &self.side {
            Side::Client(h) => &**h,
            Side::View(h) => *h,
            Side::Server(h) => &**h,
        }
    }

    // ---- both sides ---------------------------------------------------------------------------

    /// Whether the player left the mod on.
    pub fn on(&self, id: &str) -> bool {
        self.any().on(id)
    }

    /// The block at `pos` (`minecraft:stone`), air where nothing is loaded: in the window's world
    /// on the client, the level being ticked on the server.
    pub fn block(&self, pos: [i32; 3]) -> String {
        let host = self.any();
        let level = match &self.side {
            Side::Client(_) | Side::View(_) => self.client_ref().and_then(|h| h.local_player().and_then(|p| p.level(h))),
            Side::Server(_) => SERVER_LEVEL.with(|l| l.get()),
        };
        level
            .and_then(|l| l.block(host, pos))
            .map(|k| k.id(host))
            .unwrap_or_else(|| "minecraft:air".to_owned())
    }

    // ---- the window -----------------------------------------------------------------------------

    pub fn camera(&self) -> Option<NativeCamera> {
        let h = self.client_ref()?;
        let c = h.camera()?;
        let dimension = h.local_player().and_then(|p| p.dimension(h)).map(|d| d.to_string()).unwrap_or_else(|| "minecraft:overworld".to_owned());
        Some(NativeCamera { position: c.position, eye_height: c.eye_height, partial_tick: c.partial_tick, yaw: c.yaw, pitch: c.pitch, dimension })
    }

    pub fn raycast(&self, reach: f32) -> Option<NativeHit> {
        let hit = self.client_ref()?.raycast(reach)?;
        Some(NativeHit { block_x: hit.block_x, block_y: hit.block_y, block_z: hit.block_z, x: hit.x, y: hit.y, z: hit.z, block: hit.block.id(self.any()) })
    }

    pub fn block_collision(&self, pos: [i32; 3]) -> Vec<[f64; 6]> {
        self.client_ref().map(|h| h.block_collision(pos)).unwrap_or_default()
    }

    /// The name a mod key reports under (see the module's notes).
    pub fn mod_key(&self, _mod_id: &str, key: &str, _default: &str) -> String {
        format!("{ID}:{key}")
    }

    /// The key a mod key is bound to now (`key.keyboard.z`), for labels.
    pub fn bound_key(&self, mod_id: &str, key: &str, default: &str) -> String {
        self.client_ref().and_then(|h| h.mod_key(mod_id, key)).map(|(k, _)| k).unwrap_or_else(|| default.to_owned())
    }

    pub fn is_down(&self, key: &str) -> bool {
        self.client_ref().is_some_and(|h| h.is_down(ID, key))
    }

    pub fn consume_click(&mut self, key: &str) -> bool {
        self.client_mut().is_some_and(|h| h.consume_click(ID, key))
    }

    pub fn play_local_sound(&mut self, name: &str, at: [f64; 3], volume: f32, pitch: f32, delay: i32) -> bool {
        let Some(h) = self.client_ref() else {
            return false;
        };
        let mut play = SoundPlay::at(name, SoundSource::Players, at);
        play.volume = volume;
        play.pitch = pitch.clamp(0.5, 2.0);
        h.play_local_sound(&play, delay.max(0) as u32).is_some()
    }

    /// The player's velocity now (blocks a tick).
    pub fn player_velocity(&self) -> Option<[f64; 3]> {
        let h = self.client_ref()?;
        h.local_player()?.entity().velocity(h)
    }

    pub fn push_player(&mut self, kick: [f32; 3]) {
        if let Some(h) = self.client_mut() {
            h.push_player(kick);
        }
    }

    pub fn net_send_to_server(&mut self, channel: &str, data: &[u8]) -> Result<(), String> {
        let h = self.client_mut().ok_or("not a client")?;
        h.send_payload_to_server(&mod_api::PayloadId(channel.to_owned()), mod_api::PayloadBytes(data.to_vec()))
            .map(|_| ())
            .map_err(|e| format!("{e:?}"))
    }

    pub fn day_time(&self) -> i64 {
        let Some(h) = self.client_ref() else {
            return 6000;
        };
        h.local_player().and_then(|p| p.level(h)).and_then(|l| l.day_time(h)).unwrap_or(6000)
    }

    /// The item in the main hand (`minecraft:breeze_rod`), its count, and whether it is plain.
    pub fn held(&self) -> Option<(String, u32, bool)> {
        let h = self.client_ref()?;
        let living = h.living(h.local_player()?.entity())?;
        let stack = living.hand_item(h, Hand::Main)?;
        Some((stack.item().id(h), stack.count(), true))
    }

    // ---- the server -------------------------------------------------------------------------------

    pub fn world_height(&self) -> (i32, i32) {
        let Some(h) = self.server_ref() else {
            return (-64, 384);
        };
        SERVER_LEVEL
            .with(|l| l.get())
            .and_then(|l| l.height(h))
            .unwrap_or((-64, 384))
    }

    pub fn edit_blocks(&mut self, dimension: &str, edit: &NativeBlockEdit) -> Result<u64, String> {
        let h = self.server_mut().ok_or("not a server")?;
        let mut palette = Vec::with_capacity(edit.palette.len());
        for name in &edit.palette {
            palette.push(h.block_state_id(name).ok_or_else(|| format!("{name}: not a block"))?);
        }
        let mut filters = Vec::with_capacity(edit.filters.len());
        for f in &edit.filters {
            filters.push(predicate(&*h, f).ok_or_else(|| format!("{f}: not a block or tag"))?);
        }
        h.edit_blocks(dimension, &mod_api::BlockEdit { palette, filters, runs: edit.runs.clone() })
    }

    pub fn surface_heights(&self, dimension: &str, columns: &[(i32, i32)], map: NativeHeightmap) -> Result<Vec<Option<i32>>, String> {
        self.server_ref().ok_or("not a server")?.surface_heights(dimension, columns, map)
    }

    pub fn run_command(&mut self, line: &str) -> bool {
        self.server_mut().is_some_and(|h| h.commands().run_command(line))
    }

    pub fn server_players(&self) -> Vec<NativeServerPlayer> {
        let Some(h) = self.server_ref() else {
            return Vec::new();
        };
        h.players()
            .server_players()
            .into_iter()
            .filter_map(|p| {
                let e = p.entity();
                let rotation = e.rotation(h).unwrap_or([0.0, 0.0]);
                Some(NativeServerPlayer {
                    uuid: p.uuid(),
                    name: p.name(h).unwrap_or_default(),
                    dimension: p.dimension(h)?.to_string(),
                    position: e.position(h)?,
                    yaw: rotation[0],
                    pitch: rotation[1],
                })
            })
            .collect()
    }

    pub fn entity(&self, id: u32) -> Option<EntityRef> {
        self.server_ref()?.entity(id)
    }

    pub fn entity_position(&self, e: EntityRef) -> Option<[f64; 3]> {
        e.position(self.any())
    }

    pub fn entity_set_velocity(&mut self, e: EntityRef, v: [f64; 3]) -> bool {
        self.server_mut().is_some_and(|h| e.set_velocity(h, v))
    }

    /// A push added to a mob's velocity (blocks a tick).
    pub fn push_mob(&mut self, _dimension: &str, id: u32, v: [f64; 3]) -> bool {
        let Some(h) = self.server_mut() else {
            return false;
        };
        match h.entity(id) {
            Some(e) => e.push(h, v),
            None => false,
        }
    }

    pub fn summon(&mut self, dimension: &str, kind: &str, pos: [f64; 3], nbt: Compound) -> Option<u32> {
        self.server_mut()?.summon(dimension, kind, pos, nbt)
    }

    pub fn net_send_to_player(&mut self, uuid: u128, channel: &str, data: &[u8]) -> Result<(), String> {
        let h = self.server_mut().ok_or("not a server")?;
        let player = h.player(uuid).ok_or("no such player")?;
        player
            .send_payload(h, &mod_api::PayloadId(channel.to_owned()), mod_api::PayloadBytes(data.to_vec()))
            .map(|_| ())
            .map_err(|e| format!("{e:?}"))
    }
}

std::thread_local! {
    /// The level the server is ticking now (for block reads and heights).
    pub static SERVER_LEVEL: std::cell::Cell<Option<mod_api::LevelRef>> = const { std::cell::Cell::new(None) };
}

/// A filter as a command writes it: `#minecraft:leaves` or `minecraft:sand`.
fn predicate(h: &dyn ServerHost, text: &str) -> Option<mod_api::StatePredicate> {
    use mod_api::HostBlockStates;
    let states = h.block_states();
    if let Some(tag) = text.strip_prefix('#') {
        let key = h.tags().key(&mod_api::Identifier::parse(tag).ok()?)?;
        mod_api::StatePredicate::tag(&states, h.tags(), key, &[]).ok()
    } else {
        let block = h.block_key(text)?;
        mod_api::StatePredicate::block(&states, block, &[]).ok()
    }
}
