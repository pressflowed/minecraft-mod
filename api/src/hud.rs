//! The powers panel: every power with its key, whether it's ready, and the laser eyes'
//! power. The panel key (H by default) shows and hides it; hidden, a one-line hint stays.

use crate::client::{self, LASERS, PANEL, POWERS};
use horizon_client::mod_api::{Corner, Corners, Host, HudBatch};
use std::time::Instant;

const LINE: f32 = 11.0;
const WIDTH: f32 = 168.0;
const PAD: f32 = 4.0;

pub fn draw(host: &mut dyn Host, batch: &mut HudBatch, corners: &mut Corners) {
    let (panel, lasers, laser_power, ready) = {
        let state = client::state();
        (state.panel, state.lasers, state.laser_power, state.ready)
    };
    let corner = if host.corner_shown(Corner::TopLeft) { Corner::TopLeft } else { Corner::BottomLeft };
    let toggle = client::key_label(host, PANEL.0, PANEL.2);
    if !panel {
        let hint = format!("[{toggle}] Superpowers");
        let w = hint.len() as f32 * 6.0 + PAD * 2.0;
        let (x, y) = corners.place(corner, w, LINE + 2.0);
        host.fill(batch, [x, y, w, LINE + 2.0], 0x6000_0000);
        host.text(batch, &hint, [x + PAD, y + 2.0], 0xC8C8C8, true);
        return;
    }
    let rows = POWERS.len() as f32 + if lasers { 1.0 } else { 0.0 };
    let height = PAD * 2.0 + LINE * (rows + 1.6);
    let (x, y) = corners.place(corner, WIDTH, height);
    // A dark glass panel with a gold rim along its top.
    host.fill(batch, [x, y, WIDTH, height], 0xA010_0818);
    host.fill(batch, [x, y, WIDTH, 1.5], 0xFFFFC23D);
    host.text(batch, "SUPERPOWERS", [x + PAD, y + PAD], 0xFFC23D, true);
    let hide = format!("[{toggle}] hide");
    host.text(batch, &hide, [x + WIDTH - PAD - hide.len() as f32 * 6.0, y + PAD], 0x8A8A8A, true);

    let now = Instant::now();
    let mut row = y + PAD + LINE * 1.5;
    for (index, power) in POWERS.iter().enumerate() {
        let key = client::key_label(host, power.key, power.default);
        // The key in a little box of the power's colour.
        let key_w = key.len().max(1) as f32 * 6.0 + 4.0;
        host.fill(batch, [x + PAD, row - 1.0, key_w, LINE - 1.0], 0xFF00_0000 | dim(power.color));
        host.text(batch, &key, [x + PAD + 2.0, row], 0xFFFFFF, true);
        host.text(batch, power.label, [x + PAD + key_w + 4.0, row], power.color, true);

        let left = ready[index].map_or(0.0, |r| r.saturating_duration_since(now).as_secs_f32());
        let (status, color) = if index == LASERS {
            if lasers { (format!("ON  P{laser_power}"), 0xFF5050) } else { ("off".to_owned(), 0x9A9A9A) }
        } else if left > 0.0 {
            (format!("{left:.1}s"), 0xFF7070)
        } else {
            ("READY".to_owned(), 0x6CFF8A)
        };
        host.text(batch, &status, [x + WIDTH - PAD - status.len() as f32 * 6.0, row], color, true);
        // A bar under the line that fills as the power comes back.
        if left > 0.0 && index != LASERS {
            let full = (1.0 - left / power.cooldown).clamp(0.0, 1.0);
            let bar_w = WIDTH - PAD * 2.0;
            host.fill(batch, [x + PAD, row + LINE - 2.0, bar_w, 1.0], 0x40FF_FFFF);
            host.fill(batch, [x + PAD, row + LINE - 2.0, bar_w * full, 1.0], 0xFF00_0000 | power.color);
        }
        row += LINE;
        // The laser eyes' power, five pips, and how to raise it.
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
