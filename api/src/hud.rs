//! The powers panel (every power, its key, whether it's ready, the laser eyes' power), the flight
//! read-out and the Cataclysm's charge. The panel key (H) shows and hides the panel; hidden, a
//! one-line hint stays.

use crate::client::{self, CATACLYSM, CHARGE_TICKS, FLIGHT, LASERS, LOOP, PANEL, POWERS};
use horizon_client::mod_api::{Corner, Corners, Host, HudBatch};
use std::time::Instant;

const LINE: f32 = 11.0;
const WIDTH: f32 = 190.0;
const PAD: f32 = 4.0;

fn width_of(text: &str) -> f32 {
    text.len() as f32 * 6.0
}

pub fn draw(host: &mut dyn Host, batch: &mut HudBatch, corners: &mut Corners) {
    let (panel, lasers, laser_power, ready, flying, charge) = {
        let c = client::state();
        (c.panel, c.lasers, c.laser_power, c.ready, c.flying, c.charge)
    };
    let toggle = client::key_label(host, PANEL.0, PANEL.2);
    if panel {
        draw_panel(host, batch, corners, &toggle, lasers, laser_power, &ready, flying.is_some());
    } else {
        let hint = format!("[{toggle}] Superpowers");
        let w = width_of(&hint) + PAD * 2.0;
        let (x, y) = corners.place(Corner::TopLeft, w, LINE + 2.0);
        host.fill(batch, [x, y, w, LINE + 2.0], 0x6000_0000);
        host.text(batch, &hint, [x + PAD, y + 2.0], 0xC8C8C8, true);
    }
    if let Some(f) = flying {
        let text = format!("FLYING {:>3.0} b/s  BANK {:>4.0}", f.speed * 20.0, f.bank + f.barrel * f.barrel_dir);
        let w = width_of(&text) + PAD * 2.0;
        let (x, y) = corners.place(Corner::TopRight, w, LINE + 2.0);
        host.fill(batch, [x, y, w, LINE + 2.0], 0x7000_0000);
        host.text(batch, &text, [x + PAD, y + 2.0], 0xA0E8FF, true);
    }
    if charge > 0 {
        let full = (charge as f32 / CHARGE_TICKS as f32).min(1.0);
        let w = 180.0;
        let (x, y) = corners.place(Corner::TopRight, w + PAD * 2.0, LINE + 10.0);
        host.fill(batch, [x, y, w + PAD * 2.0, LINE + 10.0], 0xC000_0000);
        host.fill(batch, [x + PAD, y + 3.0, w * full, 5.0], 0xFFFF_4020);
        let text = if full >= 1.0 { "CATACLYSM" } else { "CHARGING CATACLYSM..." };
        host.text(batch, text, [x + PAD, y + 10.0], 0xFF6A3A, true);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_panel(
    host: &mut dyn Host,
    batch: &mut HudBatch,
    corners: &mut Corners,
    toggle: &str,
    lasers: bool,
    laser_power: u8,
    ready: &[Option<Instant>; 13],
    flying: bool,
) {
    let rows = POWERS.len() as f32 + 1.2 + if lasers { 1.0 } else { 0.0 } + if flying { 1.0 } else { 0.0 };
    let height = PAD * 2.0 + LINE * (rows + 1.6);
    let (x, y) = corners.place(Corner::TopLeft, WIDTH, height);
    // a dark glass panel with a gold rim along its top
    host.fill(batch, [x, y, WIDTH, height], 0xA010_0818);
    host.fill(batch, [x, y, WIDTH, 1.5], 0xFFFF_C23D);
    host.text(batch, "SUPERPOWERS", [x + PAD, y + PAD], 0xFFC23D, true);
    let hide = format!("[{toggle}] hide");
    host.text(batch, &hide, [x + WIDTH - PAD - width_of(&hide), y + PAD], 0x8A8A8A, true);
    let now = Instant::now();
    let mut row = y + PAD + LINE * 1.5;
    let mut mega_header = false;
    for (index, power) in POWERS.iter().enumerate() {
        if power.mega && !mega_header {
            mega_header = true;
            host.fill(batch, [x + PAD, row + 2.0, WIDTH - PAD * 2.0, 1.0], 0x60FF_6A3A);
            host.text(batch, "MEGA POWERS  x5", [x + PAD, row + 4.0], 0xFF6A3A, true);
            row += LINE * 1.2;
        }
        let key = client::key_label(host, power.key, power.default);
        // the key in a little box of the power's colour
        let key_w = width_of(&key).max(6.0) + 4.0;
        host.fill(batch, [x + PAD, row - 1.0, key_w, LINE - 1.0], 0xFF00_0000 | dim(power.color));
        host.text(batch, &key, [x + PAD + 2.0, row], 0xFFFFFF, true);
        host.text(batch, power.label, [x + PAD + key_w + 4.0, row], power.color, true);
        let left = ready[index].map_or(0.0, |r| r.saturating_duration_since(now).as_secs_f32());
        let (status, color) = match index {
            LASERS if lasers => (format!("ON  P{laser_power}"), 0xFF5050),
            LASERS => ("off".to_owned(), 0x9A9A9A),
            FLIGHT if flying => ("FLYING".to_owned(), 0x8AE0FF),
            FLIGHT => ("off".to_owned(), 0x9A9A9A),
            _ if left > 0.0 => (format!("{left:.1}s"), 0xFF7070),
            _ => ("READY".to_owned(), 0x6CFF8A),
        };
        host.text(batch, &status, [x + WIDTH - PAD - width_of(&status), row], color, true);
        // a bar under the line that fills as the power comes back
        if left > 0.0 && index != LASERS && index != FLIGHT {
            let full = (1.0 - left / power.cooldown).clamp(0.0, 1.0);
            let bar = WIDTH - PAD * 2.0;
            host.fill(batch, [x + PAD, row + LINE - 2.0, bar, 1.0], 0x40FF_FFFF);
            host.fill(batch, [x + PAD, row + LINE - 2.0, bar * full, 1.0], 0xFF00_0000 | power.color);
        }
        row += LINE;
        if index == LASERS && lasers {
            host.text(batch, "power", [x + PAD + 8.0, row], 0xB0B0B0, true);
            for pip in 0..5u8 {
                let lit = pip < laser_power;
                let color = if lit { 0xFF00_0000 | beam_color(pip + 1) } else { 0x50FF_FFFF };
                host.fill(batch, [x + PAD + 44.0 + f32::from(pip) * 11.0, row + 1.0, 9.0, 6.0], color);
            }
            host.text(batch, "RMB +", [x + WIDTH - PAD - 30.0, row], 0xB0B0B0, true);
            row += LINE;
        }
        if index == FLIGHT && flying {
            let loop_key = client::key_label(host, LOOP.0, LOOP.2);
            let hint = format!("A/D bank, 2x tap: roll, {loop_key}: loop");
            host.text(batch, &hint, [x + PAD + 8.0, row], 0xB0B0B0, true);
            row += LINE;
        }
        let _ = CATACLYSM;
    }
}

/// A darker shade of `rgb`, for the key boxes.
fn dim(rgb: u32) -> u32 {
    let r = (rgb >> 16 & 0xFF) * 2 / 5;
    let g = (rgb >> 8 & 0xFF) * 2 / 5;
    let b = (rgb & 0xFF) * 2 / 5;
    r << 16 | g << 8 | b
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
