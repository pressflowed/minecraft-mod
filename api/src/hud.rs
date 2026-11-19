//! The powers panel (every power, its key, whether it's ready, the laser eyes' power), the flight
//! read-out and the Cataclysm's charge. The panel key (H) shows and hides the panel; hidden, a
//! one-line hint stays.

use crate::client::{self, CATACLYSM, CHARGE_TICKS, FLIGHT, LASERS, LOOP, PANEL, POWERS};
use crate::items::Holding;
use crate::weather::EVENTS;
use horizon_client::mod_api::{Corner, Corners, Host, HudBatch};
use std::time::Instant;

const LINE: f32 = 11.0;
const WIDTH: f32 = 190.0;
const PAD: f32 = 4.0;

fn width_of(text: &str) -> f32 {
    text.len() as f32 * 6.0
}

pub fn draw(host: &mut dyn Host, batch: &mut HudBatch, corners: &mut Corners) {
    // the screen's width, from where a box of no width goes in the top right corner
    let width = corners.place(Corner::TopRight, 0.0, 0.0).0 + 2.0;
    draw_items(host, batch, corners, width);
    draw_jets(host, batch, corners, width);
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

/// The jet's read-out (behind it; in the cockpit the instruments say it all), its keys, and the
/// jet picker.
fn draw_jets(host: &mut dyn Host, batch: &mut HudBatch, corners: &mut Corners, width: f32) {
    use crate::jet;
    use crate::jet_sim::{self, Mode};
    let (readout, picker, selected) = jet::readout();
    let key = |k: (&str, &str, &str)| client::key_label(host, k.0, k.2);
    if let Some(r) = readout {
        let mode = if r.mode == Mode::Direct { "DIRECT" } else { "MOUSE AIM" };
        let assist = if r.assist { "ASSIST" } else { "RAW" };
        let lines: Vec<(String, u32)> = if r.third {
            let mut l = vec![
                (format!("{}  {mode}  {assist}", r.name.to_uppercase()), 0x8AE0FF),
                (format!("SPD {:>3.0}  ALT {:>4.0}  M {:.2}  G {:>4.1}", r.speed, r.altitude, r.mach, r.g), 0xD8F8D8),
                (format!("THR {:>3.0}%{}  GEAR {}", r.throttle * 100.0, if r.burner > 0.3 { " AB" } else { "" }, if r.gear > 0.97 { "DOWN" } else if r.gear < 0.03 { "UP" } else { "..." }), 0xD8F8D8),
                (format!("GUN {}  MSL {}  BMB {}{}", r.rounds, r.missiles, r.bombs, match r.lock { 2 => "  LOCK", 1 => "  locking", _ => "" }), 0xFFD27A),
            ];
            if r.stalled {
                l.push(("STALL".to_owned(), 0xFF5040));
            }
            l
        } else {
            Vec::new()
        };
        let hint = format!("[{}] out  [{}] {}  [{}] assists  W/S throttle  Ctrl burner  Space brake", key(jet::JET_KEY), key(jet::MODE_KEY), if r.mode == Mode::Direct { "mouse aim" } else { "direct" }, key(jet::ASSIST_KEY));
        let w = lines.iter().map(|(t, _)| width_of(t)).fold(width_of(&hint), f32::max) + PAD * 2.0;
        let h = LINE * (lines.len() as f32 + 1.0) + 4.0;
        let (x, y) = corners.place(Corner::BottomLeft, w, h);
        host.fill(batch, [x, y, w, h], 0x9006_0A10);
        host.fill(batch, [x, y, w, 1.5], 0xFF8A_E0FF);
        let mut row = y + 3.0;
        for (text, color) in &lines {
            host.text(batch, text, [x + PAD, row], *color, true);
            row += LINE;
        }
        host.text(batch, &hint, [x + PAD, row], 0x9A9A9A, true);
    }
    if picker {
        let w = 250.0;
        let x = (width - w) / 2.0;
        let y = 24.0;
        let h = PAD * 2.0 + LINE * (jet_sim::COUNT as f32 * 2.0 + 2.6);
        host.fill(batch, [x, y, w, h], 0xC00A_0612);
        host.fill(batch, [x, y, w, 1.5], 0xFF8A_E0FF);
        host.text(batch, "JETS", [x + PAD, y + PAD], 0x8AE0FF, true);
        let help = "scroll or A/D";
        host.text(batch, help, [x + w - PAD - width_of(help), y + PAD], 0x8A8A8A, true);
        let mut row = y + PAD + LINE * 1.4;
        for (i, spec) in jet_sim::specs().iter().enumerate() {
            let chosen = i == selected;
            if chosen {
                host.fill(batch, [x + 2.0, row - 1.5, w - 4.0, LINE * 2.0 + 1.0], 0x6020_4060);
                host.fill(batch, [x + 2.0, row - 1.5, 2.0, LINE * 2.0 + 1.0], 0xFF8A_E0FF);
            }
            let name = format!("{}{}", if chosen { "> " } else { "  " }, spec.name);
            host.text(batch, &name, [x + PAD + 2.0, row], if chosen { 0xFFFFFF } else { 0xB8D8F0 }, true);
            // bars: speed, agility, size
            let top = (spec.afterburner / spec.drag_area.z / 60_000.0).clamp(0.2, 1.0) as f32;
            let agile = (spec.max_roll / 4.4) as f32;
            let size = (spec.span / 52.0).sqrt() as f32;
            for (k, (label, value, color)) in [("SPD", top, 0xFF7AD0FFu32), ("AGL", agile, 0xFF7AFF9A), ("SIZE", size, 0xFFFFC27A)].into_iter().enumerate() {
                let bx = x + 112.0 + k as f32 * 46.0;
                host.text(batch, label, [bx, row], 0x8A8A8A, true);
                host.fill(batch, [bx, row + 8.0, 40.0, 2.0], 0x40FF_FFFF);
                host.fill(batch, [bx, row + 8.0, 40.0 * value.clamp(0.05, 1.0), 2.0], color);
            }
            host.text(batch, spec.about, [x + PAD + 12.0, row + LINE], 0x9A9A9A, true);
            row += LINE * 2.0;
        }
        let release = format!("let go of [{}] to choose", key(jet::JET_KEY));
        host.text(batch, &release, [x + PAD, row + 2.0], 0x8A8A8A, true);
    }
}

/// The Storm Scepter's and the Gravity Gun's read-outs, the picker, and a passing message.
fn draw_items(host: &mut dyn Host, batch: &mut HudBatch, corners: &mut Corners, width: f32) {
    let (holding, selected, picker, aim, ready, size, gripping, toast) = {
        let c = client::state();
        let i = &c.items;
        (i.holding, i.selected, i.picker, i.aim, i.ready, i.size, i.gripping.is_some(), i.toast.clone())
    };
    let flight_key = client::key_label(host, POWERS[FLIGHT].key, POWERS[FLIGHT].default);
    let now = Instant::now();
    match holding {
        Holding::Scepter => {
            let event = &EVENTS[selected];
            let left = ready[selected].map_or(0.0, |r| r.saturating_duration_since(now).as_secs_f32());
            let range = match aim {
                Some((_, d)) => format!("{d:.0} m"),
                None => "no target".to_owned(),
            };
            let title = format!("STORM SCEPTER  {}", event.label.to_uppercase());
            let hint = format!("RMB summon   hold [{flight_key}] change   {range}");
            let w = width_of(&title).max(width_of(&hint)) + PAD * 2.0;
            let (x, y) = corners.place(Corner::BottomRight, w, LINE * 2.0 + 6.0);
            host.fill(batch, [x, y, w, LINE * 2.0 + 6.0], 0xB010_0818);
            host.fill(batch, [x, y, w, 1.5], 0xFF00_0000 | event.color);
            host.text(batch, &title, [x + PAD, y + 3.0], event.color, true);
            let (status, color) = if left > 0.0 { (format!("{left:.1}s"), 0xFF7070) } else { (String::new(), 0) };
            host.text(batch, &hint, [x + PAD, y + 3.0 + LINE], 0xB8B8B8, true);
            if !status.is_empty() {
                host.text(batch, &status, [x + w - PAD - width_of(&status), y + 3.0], color, true);
            }
        }
        Holding::Gun => {
            let title = format!("GRAVITY GUN  {size}x{size}x{size}");
            let hint = if gripping { "LMB fling   RMB drop" } else { "LMB grab   RMB size x2" };
            let w = width_of(&title).max(width_of(hint)) + PAD * 2.0;
            let (x, y) = corners.place(Corner::BottomRight, w, LINE * 2.0 + 6.0);
            host.fill(batch, [x, y, w, LINE * 2.0 + 6.0], 0xB010_0818);
            host.fill(batch, [x, y, w, 1.5], 0xFFFF_9A2A);
            host.text(batch, &title, [x + PAD, y + 3.0], 0xFF9A2A, true);
            host.text(batch, hint, [x + PAD, y + 3.0 + LINE], 0xB8B8B8, true);
        }
        Holding::Nothing => {}
    }
    if let Some((text, at)) = toast {
        let age = now.saturating_duration_since(at).as_secs_f32();
        if age < 2.0 {
            let w = width_of(&text) + PAD * 2.0;
            let (x, y) = corners.place(Corner::BottomRight, w, LINE + 2.0);
            host.fill(batch, [x, y, w, LINE + 2.0], 0x9000_0000);
            host.text(batch, &text, [x + PAD, y + 2.0], 0xFFE08A, true);
        }
    }
    if picker {
        // the picker: every event in a column in the middle of the screen, the chosen one lit
        let w = 230.0;
        let x = (width - w) / 2.0;
        let y = 24.0;
        let rows = EVENTS.len() as f32;
        let h = PAD * 2.0 + LINE * (rows + 3.2);
        host.fill(batch, [x, y, w, h], 0xC00A_0612);
        host.fill(batch, [x, y, w, 1.5], 0xFF6F_A8FF);
        host.text(batch, "STORM SCEPTER", [x + PAD, y + PAD], 0x9FD0FF, true);
        let help = "scroll or A/D";
        host.text(batch, help, [x + w - PAD - width_of(help), y + PAD], 0x8A8A8A, true);
        let mut row = y + PAD + LINE * 1.4;
        for (i, event) in EVENTS.iter().enumerate() {
            let chosen = i == selected;
            if chosen {
                host.fill(batch, [x + 2.0, row - 1.5, w - 4.0, LINE + 1.0], 0x7000_0000 | dim(event.color));
                host.fill(batch, [x + 2.0, row - 1.5, 2.0, LINE + 1.0], 0xFF00_0000 | event.color);
            }
            let left = ready[i].map_or(0.0, |r| r.saturating_duration_since(now).as_secs_f32());
            let mark = if chosen { "> " } else { "  " };
            let label = format!("{mark}{}", event.label);
            let color = if chosen { 0xFFFFFF } else { event.color };
            host.text(batch, &label, [x + PAD + 2.0, row], color, true);
            let status = if left > 0.0 { format!("{left:.0}s") } else { String::new() };
            if !status.is_empty() {
                host.text(batch, &status, [x + w - PAD - width_of(&status), row], 0xFF7070, true);
            }
            row += LINE;
        }
        let about = EVENTS[selected].about;
        host.fill(batch, [x + PAD, row + 2.0, w - PAD * 2.0, 1.0], 0x40FF_FFFF);
        host.text(batch, about, [x + PAD, row + 5.0], 0xD0D0D0, true);
        let release = format!("let go of [{flight_key}] to choose");
        host.text(batch, &release, [x + PAD, row + 5.0 + LINE], 0x8A8A8A, true);
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
    let rows = POWERS.len() as f32 + 1.2 + 4.6 + if lasers { 1.0 } else { 0.0 } + if flying { 1.0 } else { 0.0 };
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
    // the items, at the bottom
    host.fill(batch, [x + PAD, row + 2.0, WIDTH - PAD * 2.0, 1.0], 0x606F_A8FF);
    host.text(batch, "ITEMS  /superpowers give", [x + PAD, row + 4.0], 0x9FD0FF, true);
    row += LINE * 1.2;
    host.text(batch, "Storm Scepter: breeze rod", [x + PAD, row], 0xB8B8B8, true);
    row += LINE;
    host.text(batch, "Gravity Gun: echo shard", [x + PAD, row], 0xB8B8B8, true);
    row += LINE * 1.2;
    let jet_key = client::key_label(host, crate::jet::JET_KEY.0, crate::jet::JET_KEY.2);
    host.fill(batch, [x + PAD, row + 2.0, WIDTH - PAD * 2.0, 1.0], 0x608A_E0FF);
    host.text(batch, &format!("JETS  [{jet_key}] in/out, hold: choose"), [x + PAD, row + 4.0], 0x8AE0FF, true);
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
