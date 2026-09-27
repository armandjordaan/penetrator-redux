//! Input is read once per frame into an abstract action set.
//!
//! Nothing in the simulation ever asks "is W held". It asks for `climb`. That
//! indirection costs one struct and buys two things: every control is rebindable
//! in one place, and the physics can be driven from a synthetic [`Frame`] in a
//! test without a window open.

use macroquad::prelude::*;

/// One frame of player intent, already resolved from whatever keys are down.
#[derive(Clone, Copy, Debug, Default)]
pub struct Frame {
    /// -1 climbing, +1 diving, 0 level. Screen coordinates, so positive is down.
    pub pitch: f32,
    /// -1 braking, +1 accelerating along the direction of travel.
    pub throttle: f32,
    pub fire_held: bool,
    pub bomb_pressed: bool,

    // Shell / menu actions.
    pub confirm: bool,
    pub cancel: bool,
    pub menu_prev: bool,
    pub menu_next: bool,
    pub pause: bool,
    pub toggle_mute: bool,
    pub toggle_crt: bool,
    pub toggle_debug: bool,
    pub toggle_fullscreen: bool,
    pub restart: bool,
}

/// Reads the keyboard. Held state comes from `is_key_down`; one-shot actions come
/// from `is_key_pressed` so holding a key never repeats a menu selection.
pub fn read() -> Frame {
    let up = is_key_down(KeyCode::Up) || is_key_down(KeyCode::W);
    let down = is_key_down(KeyCode::Down) || is_key_down(KeyCode::S);
    let back = is_key_down(KeyCode::Left) || is_key_down(KeyCode::A);
    let fwd = is_key_down(KeyCode::Right) || is_key_down(KeyCode::D);

    Frame {
        // Opposing keys cancel rather than one winning arbitrarily.
        pitch: (down as i32 - up as i32) as f32,
        throttle: (fwd as i32 - back as i32) as f32,

        fire_held: is_key_down(KeyCode::Space) || is_key_down(KeyCode::Z),
        bomb_pressed: is_key_pressed(KeyCode::LeftShift)
            || is_key_pressed(KeyCode::RightShift)
            || is_key_pressed(KeyCode::X)
            || is_key_pressed(KeyCode::B),

        confirm: is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Space),
        cancel: is_key_pressed(KeyCode::Escape),
        menu_prev: is_key_pressed(KeyCode::Up) || is_key_pressed(KeyCode::W),
        menu_next: is_key_pressed(KeyCode::Down) || is_key_pressed(KeyCode::S),

        pause: is_key_pressed(KeyCode::P) || is_key_pressed(KeyCode::Escape),
        toggle_mute: is_key_pressed(KeyCode::M),
        toggle_crt: is_key_pressed(KeyCode::F1),
        toggle_debug: is_key_pressed(KeyCode::F3),
        toggle_fullscreen: is_key_pressed(KeyCode::F11),
        restart: is_key_pressed(KeyCode::R),
    }
}

/// The control list shown on the help screen. Kept next to [`read`] so the two
/// cannot drift apart.
pub const CONTROL_HELP: &[(&str, &str)] = &[
    ("W / S  or  UP / DOWN", "Climb and dive"),
    ("A / D  or  LEFT / RIGHT", "Throttle back and forward"),
    ("SPACE  or  Z", "Cannon (watch the heat bar)"),
    ("SHIFT / X / B", "Drop bomb"),
    ("P  or  ESC", "Pause"),
    ("M", "Mute"),
    ("F1", "CRT filter on / off"),
    ("F3", "Debug overlay"),
    ("F11", "Fullscreen"),
];
