//! The powers panel (every power, its key, whether it's ready), the flight instruments, and the
//! Cataclysm's charge. The panel key (H) shows and hides the panel; hidden, a hint stays.

use crate::client;
use crate::powers::{self, Power};
use mod_api::{ClientHost, DrawContext};
use std::time::Instant;

const LINE: f32 = 11.0;
const PAD: f32 = 4.0;

/// The key a power is bound to, as the panel shows it: `Z`, `RMB`, `F5`.
fn key_label(host: &dyn ClientHost, key: &str, default: &str) -> String {
    let bound = host.mod_key(crate::ID, key).map(|(k, _)| k).unwrap_or_else(|| default.to_owned());
    match bound.as_str() {
        "key.mouse.left" => "LMB".to_owned(),
        "key.mouse.right" => "RMB".to_owned(),
        "key.mouse.middle" => "MMB".to_owned(),
        _ => bound.rsplit('.').next().unwrap_or("?").to_ascii_uppercase(),
    }
}

/// The beams' colour at each power, `0xRRGGBB`: red, orange, pink, violet, then cyan.
pub fn beam_color(power: u8) -> u32 {
    match power {
        1 => 0xFF2A2A,
        2 => 0xFF7A1A,
        3 => 0xFF3DB4,
        4 => 0xA45CFF,
        _ => 0x3DF2FF,
    }
}

fn dim(rgb: u32) -> u32 {
    let f = |shift: u32| ((rgb >> shift & 0xFF) * 2 / 5) << shift;
    f(16) | f(8) | f(0)
}

pub fn draw(host: &dyn ClientHost, g: &mut dyn DrawContext) {
    if !host.playing() {
        return;
    }
    let (panel, lasers, laser_power, ready, charge, flight) = {
        let c = client::state();
        let flight = c.flight.map(|f| (f.speed(), c.view.2));
        (c.panel, c.lasers, c.laser_power, c.ready, c.charge, flight)
    };
    let [width, height] = g.size();
    let toggle = key_label(host, powers::PANEL.0, powers::PANEL.2);
    if panel {
        draw_panel(host, g, &toggle, lasers, laser_power, &ready, flight.is_some());
    } else {
        let hint = format!("[{toggle}] Superpowers");
        let w = g.text_width(&hint) + PAD * 2.0;
        g.fill([2.0, 2.0, w, LINE + 2.0], 0x6000_0000);
        g.text(&hint, [2.0 + PAD, 4.0], 0xFFC8_C8C8, true);
    }
    if let Some((speed, roll)) = flight {
        draw_flight(host, g, width, height, speed, roll);
    }
    if charge > 0 {
        let full = (charge as f32 / powers::CATACLYSM_CHARGE as f32).min(1.0);
        let w = 180.0;
        let (x, y) = ((width - w) / 2.0, height / 2.0 + 24.0);
        g.fill([x - 1.0, y - 1.0, w + 2.0, 7.0], 0xC000_0000);
        g.fill([x, y, w * full, 5.0], 0xFFFF_4020);
        let text = if full >= 1.0 { "CATACLYSM" } else { "CHARGING CATACLYSM... keep holding" };
        let tw = g.text_width(text);
        g.text(text, [(width - tw) / 2.0, y + 9.0], 0xFFFF_6A3A, true);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_panel(host: &dyn ClientHost, g: &mut dyn DrawContext, toggle: &str, lasers: bool, laser_power: u8, ready: &[Option<Instant>; 13], flying: bool) {
    let width = 190.0;
    let rows = powers::ALL.len() as f32 + 1.0 + if lasers { 1.0 } else { 0.0 } + if flying { 1.0 } else { 0.0 };
    let height = PAD * 2.0 + LINE * (rows + 1.6);
    let (x, y) = (2.0, 2.0);
    g.fill([x, y, width, height], 0xA010_0818);
    g.fill([x, y, width, 1.5], 0xFFFF_C23D);
    g.text("SUPERPOWERS", [x + PAD, y + PAD], 0xFFFF_C23D, true);
    let hide = format!("[{toggle}] hide");
    let hw = g.text_width(&hide);
    g.text(&hide, [x + width - PAD - hw, y + PAD], 0xFF8A_8A8A, true);
    let now = Instant::now();
    let mut row = y + PAD + LINE * 1.5;
    let mut mega_header = false;
    for info in powers::ALL.iter() {
        if info.mega && !mega_header {
            mega_header = true;
            g.fill([x + PAD, row + 2.0, width - PAD * 2.0, 1.0], 0x60FF_6A3A);
            g.text("MEGA POWERS  x5", [x + PAD, row + 4.0], 0xFFFF_6A3A, true);
            row += LINE + 2.0;
        }
        let key = key_label(host, info.key, info.default);
        let key_w = g.text_width(&key) + 4.0;
        g.fill([x + PAD, row - 1.0, key_w, LINE - 1.0], 0xFF00_0000 | dim(info.color));
        g.text(&key, [x + PAD + 2.0, row], 0xFFFF_FFFF, true);
        g.text(info.label, [x + PAD + key_w + 4.0, row], 0xFF00_0000 | info.color, true);
        let slot = info.power as usize;
        let left = ready[slot].map_or(0.0, |r| r.saturating_duration_since(now).as_secs_f32());
        let (status, color) = match info.power {
            Power::Lasers if lasers => (format!("ON  P{laser_power}"), 0xFFFF_5050),
            Power::Lasers => ("off".to_owned(), 0xFF9A_9A9A),
            Power::Flight if flying => ("FLYING".to_owned(), 0xFF8A_E0FF),
            Power::Flight => ("off".to_owned(), 0xFF9A_9A9A),
            _ if left > 0.0 => (format!("{left:.1}s"), 0xFFFF_7070),
            _ => ("READY".to_owned(), 0xFF6C_FF8A),
        };
        let sw = g.text_width(&status);
        g.text(&status, [x + width - PAD - sw, row], color, true);
        if left > 0.0 && info.power != Power::Lasers && info.power != Power::Flight {
            let full = (1.0 - left / info.cooldown).clamp(0.0, 1.0);
            let bar = width - PAD * 2.0;
            g.fill([x + PAD, row + LINE - 2.0, bar, 1.0], 0x40FF_FFFF);
            g.fill([x + PAD, row + LINE - 2.0, bar * full, 1.0], 0xFF00_0000 | info.color);
        }
        row += LINE;
        if info.power == Power::Lasers && lasers {
            g.text("power", [x + PAD + 8.0, row], 0xFFB0_B0B0, true);
            for pip in 0..5u8 {
                let color = if pip < laser_power { 0xFF00_0000 | beam_color(pip + 1) } else { 0x50FF_FFFF };
                g.fill([x + PAD + 44.0 + f32::from(pip) * 11.0, row + 1.0, 9.0, 6.0], color);
            }
            g.text("RMB +", [x + width - PAD - g.text_width("RMB +"), row], 0xFFB0_B0B0, true);
            row += LINE;
        }
        if info.power == Power::Flight && flying {
            let loop_key = key_label(host, powers::LOOP.0, powers::LOOP.2);
            let hint = format!("A/D roll, 2x tap: barrel roll, {loop_key}: loop");
            g.text(&hint, [x + PAD + 8.0, row], 0xFFB0_B0B0, true);
            row += LINE;
        }
    }
}

/// While flying: speed, height, and a horizon line that tilts with the roll.
fn draw_flight(host: &dyn ClientHost, g: &mut dyn DrawContext, width: f32, height: f32, speed: f64, roll: f64) {
    let altitude = host
        .local_player()
        .and_then(|p| p.entity().position(host))
        .map_or(0.0, |p| p[1]);
    let (cx, cy) = (width / 2.0, height / 2.0);
    // the horizon line, turned by the roll
    let (s, c) = (-roll.to_radians() as f32).sin_cos();
    let half = 40.0;
    let thick = 0.75;
    let p = |x: f32, y: f32| [cx + x * c - y * s, cy + x * s + y * c];
    for (a, b) in [(-half, -10.0), (10.0, half)] {
        g.quad([p(a, -thick), p(a, thick), p(b, thick), p(b, -thick)], 0xB0A0_E8FF);
    }
    let text = format!("{:>4.0} b/s   ALT {:>4.0}   ROLL {:>4.0}", speed * 20.0, altitude, roll);
    let tw = g.text_width(&text);
    g.fill([cx - tw / 2.0 - PAD, height - 70.0, tw + PAD * 2.0, LINE + 2.0], 0x7000_0000);
    g.text(&text, [cx - tw / 2.0, height - 68.0], 0xFFA0_E8FF, true);
}
