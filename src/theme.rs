//! The look: one palette per zone, plus the fixed colours everything else uses.
//!
//! The redesign's whole visual idea is a neon vector display seen through a CRT.
//! That constrains the palette hard — every colour has to survive being drawn as
//! a thin glowing line on black, which rules out anything dark or desaturated.
//! Zones are distinguished by hue rotation rather than by brightness, so no zone
//! is easier to read than another.

use macroquad::prelude::*;

/// The colours that vary from zone to zone.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// The glowing edge of the cave.
    pub rock_edge: Color,
    /// The filled body of the rock.
    pub rock_fill: Color,
    /// Parallax silhouettes behind the playfield.
    pub backdrop: Color,
    /// Wash behind everything.
    pub sky: Color,
}

/// Indexed by `ZoneSpan::palette`. Out-of-range indices wrap, so a hand-edited
/// level with `palette 99` gets a colour scheme rather than a panic.
pub const PALETTES: [Palette; 5] = [
    // APPROACH — cool teal, the friendliest the game ever looks.
    Palette {
        rock_edge: Color::new(0.29, 0.94, 0.83, 1.0),
        rock_fill: Color::new(0.03, 0.10, 0.13, 1.0),
        backdrop: Color::new(0.05, 0.16, 0.20, 1.0),
        sky: Color::new(0.012, 0.030, 0.045, 1.0),
    },
    // RAVINE — shifting toward violet as the cave tightens.
    Palette {
        rock_edge: Color::new(0.51, 0.72, 1.00, 1.0),
        rock_fill: Color::new(0.04, 0.07, 0.15, 1.0),
        backdrop: Color::new(0.07, 0.11, 0.24, 1.0),
        sky: Color::new(0.015, 0.020, 0.048, 1.0),
    },
    // THE TEETH — magenta. Hostile, and meant to read that way.
    Palette {
        rock_edge: Color::new(0.92, 0.45, 0.95, 1.0),
        rock_fill: Color::new(0.10, 0.04, 0.13, 1.0),
        backdrop: Color::new(0.17, 0.06, 0.20, 1.0),
        sky: Color::new(0.035, 0.012, 0.042, 1.0),
    },
    // DEEP CUT — furnace orange.
    Palette {
        rock_edge: Color::new(1.00, 0.58, 0.26, 1.0),
        rock_fill: Color::new(0.12, 0.05, 0.02, 1.0),
        backdrop: Color::new(0.20, 0.09, 0.03, 1.0),
        sky: Color::new(0.042, 0.018, 0.008, 1.0),
    },
    // BUNKER — sickly green under artificial light.
    Palette {
        rock_edge: Color::new(0.62, 1.00, 0.42, 1.0),
        rock_fill: Color::new(0.05, 0.11, 0.04, 1.0),
        backdrop: Color::new(0.08, 0.18, 0.07, 1.0),
        sky: Color::new(0.014, 0.038, 0.012, 1.0),
    },
];

pub fn palette(index: usize) -> Palette {
    PALETTES[index % PALETTES.len()]
}

// --- fixed colours -------------------------------------------------------

pub const SHIP: Color = Color::new(0.55, 0.95, 1.00, 1.0);
pub const SHIP_INVULN: Color = Color::new(1.00, 1.00, 1.00, 1.0);
pub const EXHAUST: Color = Color::new(0.40, 0.85, 1.00, 1.0);

pub const CANNON_ROUND: Color = Color::new(1.00, 0.95, 0.55, 1.0);
pub const BOMB: Color = Color::new(1.00, 0.72, 0.30, 1.0);
pub const SAM_TRACKING: Color = Color::new(1.00, 0.30, 0.35, 1.0);
pub const SAM_BALLISTIC: Color = Color::new(1.00, 0.62, 0.25, 1.0);
pub const SHELL: Color = Color::new(1.00, 0.45, 0.70, 1.0);

pub const SILO: Color = Color::new(1.00, 0.42, 0.42, 1.0);
pub const RADAR: Color = Color::new(0.45, 1.00, 0.62, 1.0);
pub const TURRET: Color = Color::new(1.00, 0.66, 0.35, 1.0);
pub const MINE: Color = Color::new(0.98, 0.90, 0.36, 1.0);
pub const DRONE: Color = Color::new(0.85, 0.55, 1.00, 1.0);
pub const WARHEAD: Color = Color::new(1.00, 0.28, 0.55, 1.0);

pub const HUD_TEXT: Color = Color::new(0.80, 0.94, 1.00, 1.0);
pub const HUD_DIM: Color = Color::new(0.42, 0.56, 0.66, 1.0);
pub const HUD_WARN: Color = Color::new(1.00, 0.45, 0.35, 1.0);
pub const HUD_GOOD: Color = Color::new(0.45, 1.00, 0.62, 1.0);
pub const HUD_PANEL: Color = Color::new(0.02, 0.05, 0.08, 0.72);

pub const EXPLOSION: Color = Color::new(1.00, 0.72, 0.32, 1.0);
pub const EXPLOSION_BIG: Color = Color::new(1.00, 0.90, 0.60, 1.0);

/// Colour for an enemy of a given kind.
pub fn enemy_color(kind: crate::level::SpawnKind) -> Color {
    use crate::level::SpawnKind as K;
    match kind {
        K::Silo => SILO,
        K::Radar => RADAR,
        K::Turret => TURRET,
        K::Mine => MINE,
        K::Drone => DRONE,
        K::Warhead => WARHEAD,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_index_wraps_instead_of_panicking() {
        // Hand-edited levels are allowed to be wrong; they are not allowed to crash.
        let a = palette(0);
        let b = palette(PALETTES.len());
        assert_eq!(a.rock_edge.r, b.rock_edge.r);
        let _ = palette(usize::MAX);
    }

    #[test]
    fn every_palette_reads_as_neon_on_black() {
        for (i, p) in PALETTES.iter().enumerate() {
            let edge_luma = 0.299 * p.rock_edge.r + 0.587 * p.rock_edge.g + 0.114 * p.rock_edge.b;
            let fill_luma = 0.299 * p.rock_fill.r + 0.587 * p.rock_fill.g + 0.114 * p.rock_fill.b;
            assert!(edge_luma > 0.45, "palette {i} edge is too dim to glow");
            assert!(fill_luma < 0.15, "palette {i} fill is too bright to sit behind");
            assert!(edge_luma - fill_luma > 0.4, "palette {i} lacks contrast");
        }
    }
}
