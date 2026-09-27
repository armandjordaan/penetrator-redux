//! The head-up display and the full-screen overlays.
//!
//! The HUD answers four questions, and it is laid out in the order you need them
//! answered: how am I doing (score, ships), where am I (zone, phase, progress),
//! what can I shoot with (bombs, cannon heat), and what is about to kill me
//! (radar state, missile lock).
//!
//! It lives in the top 22 virtual units plus a 4-unit strip at the bottom, and
//! the cave generator is forbidden from putting rock in that space — so the HUD
//! never hides a wall.

use crate::config::*;
use crate::input::CONTROL_HELP;
use crate::projectile::ProjectileKind;
use crate::render::{phase_color, Renderer};
use crate::theme;
use crate::util::fade;
use crate::world::{Phase, RunState, World};
use macroquad::prelude::*;

/// Draws the in-flight HUD.
pub fn draw(r: &Renderer, world: &World) {
    draw_top_bar(r, world);
    draw_progress(r, world);
    draw_lock_warning(r, world);
    draw_banner(r, world);
}

fn draw_top_bar(r: &Renderer, world: &World) {
    draw_rectangle(0.0, 0.0, VIRTUAL_W, HUD_H, theme::HUD_PANEL);
    draw_line(0.0, HUD_H, VIRTUAL_W, HUD_H, 0.6, fade(theme::HUD_TEXT, 0.25));

    // --- score and ships ---
    r.text("SCORE", 6.0, 8.0, 5.5, theme::HUD_DIM);
    r.text(&format!("{:07}", world.score), 6.0, 18.0, 9.0, theme::HUD_TEXT);

    let ships_x = 62.0;
    r.text("SHIPS", ships_x, 8.0, 5.5, theme::HUD_DIM);
    for i in 0..STARTING_LIVES.max(world.lives) {
        let x = ships_x + i as f32 * 8.0;
        let lit = i < world.lives;
        let color = if lit {
            theme::SHIP
        } else {
            fade(theme::HUD_DIM, 0.35)
        };
        // A tiny copy of the ship silhouette, so the icon means something.
        let pts = [
            Vec2::new(x + 6.0, 14.0),
            Vec2::new(x + 0.0, 11.0),
            Vec2::new(x + 1.0, 14.0),
            Vec2::new(x + 0.0, 17.0),
        ];
        r.glow_path(&pts, true, 0.6, color);
    }

    // --- where am I ---
    let zone = world.zone_name();
    r.text_centered(zone, VIRTUAL_W * 0.5, 10.0, 7.5, theme::HUD_TEXT);
    r.text_centered(
        world.phase.label(),
        VIRTUAL_W * 0.5,
        19.0,
        6.0,
        phase_color(world.phase),
    );

    // --- radar ---
    let radar_x = 300.0;
    let (radar_label, radar_color) = if world.radar_active() {
        ("TRACKING", theme::HUD_WARN)
    } else {
        ("JAMMED", theme::HUD_GOOD)
    };
    r.text("RADAR", radar_x, 8.0, 5.5, theme::HUD_DIM);
    r.text(radar_label, radar_x, 18.0, 7.0, radar_color);
    // Remaining dishes as pips.
    for i in 0..world.radars_total.min(14) {
        let lit = i < world.radars_alive;
        let x = radar_x + 40.0 + i as f32 * 3.2;
        draw_rectangle(
            x,
            12.0,
            2.0,
            6.0,
            if lit {
                theme::HUD_WARN
            } else {
                fade(theme::HUD_DIM, 0.3)
            },
        );
    }

    // --- weapons ---
    let arm_x = 396.0;
    r.text("BOMBS", arm_x, 8.0, 5.5, theme::HUD_DIM);
    let bomb_color = if world.player.bombs == 0 {
        theme::HUD_WARN
    } else {
        theme::BOMB
    };
    r.text(&format!("{:02}", world.player.bombs), arm_x, 18.0, 9.0, bomb_color);

    // Cannon heat. Red and labelled when locked out — an overheated gun that
    // simply stops firing reads as a bug.
    let heat_x = arm_x + 22.0;
    let heat_w = 52.0;
    r.text("CANNON", heat_x, 8.0, 5.5, theme::HUD_DIM);
    draw_rectangle(heat_x, 12.0, heat_w, 5.0, fade(theme::HUD_DIM, 0.25));
    let heat_color = if world.player.overheated {
        theme::HUD_WARN
    } else if world.player.heat > 0.7 {
        theme::BOMB
    } else {
        theme::HUD_GOOD
    };
    draw_rectangle(heat_x, 12.0, heat_w * world.player.heat.clamp(0.0, 1.0), 5.0, heat_color);
    draw_rectangle_lines(heat_x, 12.0, heat_w, 5.0, 0.6, fade(theme::HUD_TEXT, 0.3));
    if world.player.overheated {
        r.text_centered("OVERHEAT", heat_x + heat_w * 0.5, 16.4, 4.5, BLACK);
    }
}

/// The strip along the bottom: how far through the mission you are, with a tick
/// per zone and a marker that turns round when you do.
fn draw_progress(r: &Renderer, world: &World) {
    let y = VIRTUAL_H - 3.5;
    let (x0, x1) = (8.0, VIRTUAL_W - 8.0);
    let span = x1 - x0;

    draw_line(x0, y, x1, y, 1.0, fade(theme::HUD_DIM, 0.35));

    let len = world.track.world_len().max(1.0);
    for zone in &world.track.zones {
        let t = (zone.start_x() / len).clamp(0.0, 1.0);
        let x = x0 + span * t;
        draw_line(x, y - 2.0, x, y + 2.0, 0.8, fade(theme::HUD_DIM, 0.55));
    }

    let t = world.progress();
    let marker = x0 + span * t;
    let travelled_from = if world.phase == Phase::Egress { x1 } else { x0 };
    draw_line(
        travelled_from,
        y,
        marker,
        y,
        1.4,
        fade(phase_color(world.phase), 0.85),
    );
    let dir = world.phase.facing();
    r.glow_path(
        &[
            Vec2::new(marker + 3.0 * dir, y),
            Vec2::new(marker - 2.0 * dir, y - 2.5),
            Vec2::new(marker - 2.0 * dir, y + 2.5),
        ],
        true,
        0.7,
        phase_color(world.phase),
    );
}

/// Flashes when a guided missile is actually tracking you.
///
/// It only appears for homing SAMs that are closing, which is the difference
/// between a warning and a nuisance: an unguided missile crossing the screen is
/// not a threat and should not shout.
fn draw_lock_warning(r: &Renderer, world: &World) {
    if world.state != RunState::Flying {
        return;
    }
    let ship = world.player.pos;
    let threat = world.projectiles.iter().any(|p| {
        p.kind == ProjectileKind::Sam
            && p.homing
            && p.pos.distance(ship) < 150.0
            // Closing, rather than merely nearby.
            && (ship - p.pos).dot(p.vel) > 0.0
    });
    if !threat {
        return;
    }
    let blink = ((r.clock * 9.0).sin() * 0.5 + 0.5).powf(0.6);
    r.text_centered(
        "MISSILE LOCK",
        VIRTUAL_W * 0.5,
        HUD_H + 14.0,
        9.0,
        fade(theme::HUD_WARN, 0.35 + blink * 0.65),
    );
}

/// The mission callouts.
fn draw_banner(r: &Renderer, world: &World) {
    // Once the run is over the summary screen owns the middle of the display and
    // says the same thing in its own words. Leaving the callout up draws
    // "MISSION FAILED" twice, overlapping, straight through the statistics.
    if world.state.is_over() {
        return;
    }
    let Some(b) = world.banner.as_ref() else {
        return;
    };
    // Fade in over the first fifth, hold, fade out over the last third.
    let t = (b.life / b.max_life).clamp(0.0, 1.0);
    let alpha = (t * 3.0).min(1.0).min((1.0 - t) * 5.0 + 0.35).clamp(0.0, 1.0);

    let cy = VIRTUAL_H * 0.38;
    draw_rectangle(
        0.0,
        cy - 16.0,
        VIRTUAL_W,
        34.0,
        fade(Color::new(0.0, 0.0, 0.0, 0.55), alpha),
    );
    draw_line(0.0, cy - 16.0, VIRTUAL_W, cy - 16.0, 0.7, fade(b.color, alpha * 0.7));
    draw_line(0.0, cy + 18.0, VIRTUAL_W, cy + 18.0, 0.7, fade(b.color, alpha * 0.7));

    r.text_centered(&b.title, VIRTUAL_W * 0.5, cy - 2.0, 14.0, fade(b.color, alpha));
    r.text_centered(
        &b.subtitle,
        VIRTUAL_W * 0.5,
        cy + 11.0,
        6.5,
        fade(theme::HUD_TEXT, alpha * 0.85),
    );
}

// ---------------------------------------------------------------------------
// Overlays
// ---------------------------------------------------------------------------

/// Dims the playfield behind a modal screen.
pub fn dim(alpha: f32) {
    draw_rectangle(0.0, 0.0, VIRTUAL_W, VIRTUAL_H, Color::new(0.0, 0.02, 0.04, alpha));
}

pub fn draw_pause(r: &Renderer) {
    dim(0.72);
    r.text_centered("PAUSED", VIRTUAL_W * 0.5, 66.0, 24.0, theme::HUD_TEXT);

    let mut y = 96.0;
    for (keys, what) in CONTROL_HELP {
        r.text_right(keys, VIRTUAL_W * 0.5 - 10.0, y, 7.0, theme::HUD_TEXT);
        r.text(what, VIRTUAL_W * 0.5 + 10.0, y, 7.0, theme::HUD_DIM);
        y += 11.0;
    }
    r.text_centered(
        "P or ESC to resume     R to abandon the run",
        VIRTUAL_W * 0.5,
        VIRTUAL_H - 22.0,
        6.5,
        theme::HUD_DIM,
    );
}

/// The end-of-run summary, used for both outcomes.
pub fn draw_run_over(r: &Renderer, world: &World, high_score: u32) {
    let complete = matches!(world.state, RunState::Complete { .. });
    dim(0.78);

    let (title, color) = if complete {
        ("MISSION COMPLETE", theme::HUD_GOOD)
    } else {
        ("MISSION FAILED", theme::HUD_WARN)
    };
    r.text_centered(title, VIRTUAL_W * 0.5, 56.0, 22.0, color);
    r.text_centered(
        if complete {
            "The warhead is gone and so are you."
        } else {
            "The cave keeps what it catches."
        },
        VIRTUAL_W * 0.5,
        70.0,
        6.5,
        theme::HUD_DIM,
    );

    let s = &world.stats;
    let rows: [(&str, String); 6] = [
        ("SCORE", format!("{:07}", world.score)),
        ("BEST", format!("{high_score:07}")),
        ("TARGETS DESTROYED", format!("{}", s.kills)),
        ("SHIPS LOST", format!("{}", s.deaths)),
        ("ROUNDS FIRED", format!("{}", s.shots_fired)),
        ("FLIGHT TIME", format_time(s.elapsed)),
    ];
    let mut y = 100.0;
    for (label, value) in rows {
        r.text_right(label, VIRTUAL_W * 0.5 - 8.0, y, 7.0, theme::HUD_DIM);
        r.text(&value, VIRTUAL_W * 0.5 + 8.0, y, 7.0, theme::HUD_TEXT);
        y += 12.0;
    }

    if world.score >= high_score && world.score > 0 {
        r.text_centered("NEW BEST", VIRTUAL_W * 0.5, y + 6.0, 9.0, theme::BOMB);
    }

    r.text_centered(
        "ENTER to fly again     ESC for the menu",
        VIRTUAL_W * 0.5,
        VIRTUAL_H - 20.0,
        7.0,
        theme::HUD_TEXT,
    );
}

/// Formats seconds as `m:ss`.
pub fn format_time(seconds: f32) -> String {
    let total = seconds.max(0.0) as u32;
    format!("{}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_formatting_pads_seconds() {
        assert_eq!(format_time(0.0), "0:00");
        assert_eq!(format_time(9.7), "0:09");
        assert_eq!(format_time(65.0), "1:05");
        assert_eq!(format_time(600.0), "10:00");
    }

    #[test]
    fn time_formatting_survives_nonsense_input() {
        assert_eq!(format_time(-5.0), "0:00");
        assert_eq!(format_time(f32::NAN), "0:00");
    }
}
