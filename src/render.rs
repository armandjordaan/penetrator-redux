//! Drawing.
//!
//! Everything is drawn in the 480x270 virtual canvas and mapped onto the window
//! by a single [`Viewport`], which letterboxes rather than stretching. That one
//! decision removes resolution from the rest of the codebase entirely: gameplay
//! code, HUD layout and the editor all work in the same fixed units, and the game
//! looks identical at 720p and 4K.
//!
//! There is no shader in here. The neon look is built out of ordinary primitives
//! drawn two or three times at decreasing width and increasing alpha, which is
//! cheap, portable to any backend macroquad supports, and — usefully — degrades
//! to a plain vector look rather than to nothing if a machine cannot manage it.

use crate::config::*;
use crate::fx::{Fx, ParticleKind};
use crate::level::{SpawnKind, Track};
use crate::projectile::ProjectileKind;
use crate::theme::{self, Palette};
use crate::util::{fade, mix, remap};
use crate::world::{Phase, RunState, World};
use macroquad::prelude::*;

/// The mapping from the virtual canvas to real window pixels.
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Real pixels per virtual unit.
    pub scale: f32,
}

impl Viewport {
    /// Fits the virtual canvas inside `(screen_w, screen_h)`, preserving aspect.
    pub fn fit(screen_w: f32, screen_h: f32) -> Viewport {
        let screen_w = screen_w.max(1.0);
        let screen_h = screen_h.max(1.0);
        // A window wider than the canvas is height-limited and gets bars at the
        // sides; a taller one is width-limited and gets them top and bottom.
        let scale = if screen_w / screen_h > ASPECT {
            screen_h / VIRTUAL_H
        } else {
            screen_w / VIRTUAL_W
        };
        let w = VIRTUAL_W * scale;
        let h = VIRTUAL_H * scale;
        Viewport {
            x: ((screen_w - w) * 0.5).floor(),
            y: ((screen_h - h) * 0.5).floor(),
            w,
            h,
            scale,
        }
    }

    /// Converts a real window position (such as the mouse) into virtual units.
    /// Used by the editor; clamped so a cursor on the letterbox bar still maps
    /// to a sensible point rather than off the canvas.
    pub fn to_virtual(self, p: Vec2) -> Vec2 {
        Vec2::new(
            ((p.x - self.x) / self.w * VIRTUAL_W).clamp(0.0, VIRTUAL_W),
            ((p.y - self.y) / self.h * VIRTUAL_H).clamp(0.0, VIRTUAL_H),
        )
    }
}

pub struct Renderer {
    pub viewport: Viewport,
    /// Scanlines and vignette. Toggled with F1.
    pub crt: bool,
    pub debug: bool,
    /// Free-running clock for idle animation.
    pub clock: f32,
}

impl Renderer {
    pub fn new() -> Self {
        Renderer {
            viewport: Viewport::fit(screen_width(), screen_height()),
            crt: true,
            debug: false,
            clock: 0.0,
        }
    }

    /// Clears the window and switches into virtual-canvas space.
    pub fn begin(&mut self, dt: f32) {
        self.clock += dt;
        self.viewport = Viewport::fit(screen_width(), screen_height());

        clear_background(BLACK);
        set_camera(&Self::camera(self.viewport));
    }

    /// Builds the camera that maps the virtual canvas onto the viewport.
    ///
    /// The `zoom.y` negation is not cosmetic and not optional.
    /// `Camera2D::from_display_rect` hands back a camera with a *negative*
    /// `zoom.y`, because it is written for render targets, which are themselves
    /// stored bottom-up. When a camera has no render target macroquad applies
    /// its own inversion on top:
    ///
    /// ```text
    /// let invert_y = if self.render_target.is_some() { 1.0 } else { -1.0 };
    /// ```
    ///
    /// Drawing straight to the screen, as this game does, those two negatives
    /// cancel and the entire world renders upside down — cave, ship, HUD and
    /// all. Negating here undoes the one baked into `from_display_rect` so the
    /// surviving inversion is macroquad's, and virtual y = 0 lands at the top of
    /// the screen where every coordinate in this codebase assumes it is.
    ///
    /// Kept separate from `begin` so it can be tested without a window; the
    /// matrix is pure arithmetic.
    pub fn camera(viewport: Viewport) -> Camera2D {
        let mut cam = Camera2D::from_display_rect(Rect::new(0.0, 0.0, VIRTUAL_W, VIRTUAL_H));
        cam.zoom.y = -cam.zoom.y;
        cam.viewport = Some((
            viewport.x as i32,
            viewport.y as i32,
            viewport.w as i32,
            viewport.h as i32,
        ));
        cam
    }

    pub fn end(&self) {
        set_default_camera();
    }

    // -----------------------------------------------------------------------
    // Primitives
    // -----------------------------------------------------------------------

    /// A line with a soft halo around it. Three passes: a wide dim one for the
    /// bloom, a medium one for the core, and the line itself at full strength.
    pub fn glow_line(&self, a: Vec2, b: Vec2, width: f32, color: Color) {
        draw_line(a.x, a.y, b.x, b.y, width * 4.5, fade(color, 0.07));
        draw_line(a.x, a.y, b.x, b.y, width * 2.2, fade(color, 0.20));
        draw_line(a.x, a.y, b.x, b.y, width, color);
    }

    pub fn glow_circle(&self, c: Vec2, r: f32, width: f32, color: Color) {
        draw_circle_lines(c.x, c.y, r, width * 3.5, fade(color, 0.09));
        draw_circle_lines(c.x, c.y, r, width, color);
    }

    pub fn glow_dot(&self, c: Vec2, r: f32, color: Color) {
        draw_circle(c.x, c.y, r * 2.6, fade(color, 0.10));
        draw_circle(c.x, c.y, r, color);
    }

    /// Draws a path with glow. Closes the loop when `closed`.
    pub fn glow_path(&self, points: &[Vec2], closed: bool, width: f32, color: Color) {
        if points.len() < 2 {
            return;
        }
        for pair in points.windows(2) {
            self.glow_line(pair[0], pair[1], width, color);
        }
        if closed {
            self.glow_line(points[points.len() - 1], points[0], width, color);
        }
    }

    /// Fills a convex polygon by fanning triangles from the first vertex.
    pub fn fill_poly(&self, points: &[Vec2], color: Color) {
        for i in 1..points.len().saturating_sub(1) {
            draw_triangle(points[0], points[i], points[i + 1], color);
        }
    }

    /// Text sized in virtual units.
    ///
    /// The font is rasterised at the size it will actually occupy on screen and
    /// then scaled back into virtual units, so text stays sharp at any window
    /// size instead of being a blurry upscale of a 6px glyph.
    pub fn text(&self, s: &str, x: f32, y: f32, size: f32, color: Color) -> f32 {
        let raster = (size * self.viewport.scale).clamp(9.0, 160.0).round() as u16;
        let scale = size / raster as f32;
        let d = draw_text_ex(
            s,
            x,
            y,
            TextParams {
                font: None,
                font_size: raster,
                font_scale: scale,
                font_scale_aspect: 1.0,
                rotation: 0.0,
                color,
            },
        );
        d.width
    }

    pub fn text_width(&self, s: &str, size: f32) -> f32 {
        let raster = (size * self.viewport.scale).clamp(9.0, 160.0).round() as u16;
        let scale = size / raster as f32;
        measure_text(s, None, raster, scale).width
    }

    /// Text centred on `cx`.
    pub fn text_centered(&self, s: &str, cx: f32, y: f32, size: f32, color: Color) {
        let w = self.text_width(s, size);
        self.text(s, cx - w * 0.5, y, size, color);
    }

    /// Text ending at `right`.
    pub fn text_right(&self, s: &str, right: f32, y: f32, size: f32, color: Color) {
        let w = self.text_width(s, size);
        self.text(s, right - w, y, size, color);
    }

    // -----------------------------------------------------------------------
    // The world
    // -----------------------------------------------------------------------

    pub fn draw_world(&self, world: &World) {
        let shake = world.fx.shake_offset();
        let cam_x = world.cam_x + shake.x;
        let cam_y = shake.y;
        let palette = blend_palette(&world.track, world.cam_x + VIRTUAL_W * 0.5);

        draw_rectangle(0.0, 0.0, VIRTUAL_W, VIRTUAL_H, palette.sky);
        self.draw_parallax(&world.track, cam_x, cam_y, &palette);
        self.draw_terrain(&world.track, cam_x, cam_y, &palette);
        self.draw_enemies(world, cam_x, cam_y);
        self.draw_projectiles(world, cam_x, cam_y);
        self.draw_particles(&world.fx, cam_x, cam_y);
        if world.state == RunState::Flying {
            self.draw_ship(world, cam_x, cam_y);
        }
        self.draw_shockwaves(&world.fx, cam_x, cam_y);
        self.draw_float_text(&world.fx, cam_x, cam_y);

        // A brief whiteout on the very biggest hits. Tied to trauma so it fires
        // for the warhead and for losing a ship, and for nothing smaller.
        let trauma = world.fx.trauma();
        if trauma > 0.55 {
            let flash = remap(trauma, 0.55, 1.0, 0.0, 0.30);
            draw_rectangle(0.0, 0.0, VIRTUAL_W, VIRTUAL_H, fade(WHITE, flash));
        }
    }

    /// Two silhouettes of the same cave, drawn further away and flatter. Sharing
    /// the terrain data means the background always belongs to the foreground,
    /// which a separately generated backdrop never quite manages.
    pub fn draw_parallax(&self, track: &Track, cam_x: f32, cam_y: f32, palette: &Palette) {
        for (depth, factor) in [(0.35f32, 0.55f32), (0.6, 0.30)] {
            let tint = fade(palette.backdrop, 0.55 * depth + 0.2);
            let step = 16.0;
            let mid = VIRTUAL_H * 0.5;
            let mut sx = 0.0;
            while sx < VIRTUAL_W {
                let wx0 = cam_x * depth + sx;
                let wx1 = cam_x * depth + sx + step;
                let c0 = mix_toward(track.terrain.ceiling_at(wx0), mid, 1.0 - factor);
                let c1 = mix_toward(track.terrain.ceiling_at(wx1), mid, 1.0 - factor);
                let f0 = mix_toward(track.terrain.floor_at(wx0), mid, 1.0 - factor);
                let f1 = mix_toward(track.terrain.floor_at(wx1), mid, 1.0 - factor);

                let y = cam_y;
                draw_triangle(
                    Vec2::new(sx, y),
                    Vec2::new(sx + step, y),
                    Vec2::new(sx + step, c1 + y),
                    tint,
                );
                draw_triangle(
                    Vec2::new(sx, y),
                    Vec2::new(sx + step, c1 + y),
                    Vec2::new(sx, c0 + y),
                    tint,
                );
                draw_triangle(
                    Vec2::new(sx, f0 + y),
                    Vec2::new(sx + step, f1 + y),
                    Vec2::new(sx + step, VIRTUAL_H + y),
                    tint,
                );
                draw_triangle(
                    Vec2::new(sx, f0 + y),
                    Vec2::new(sx + step, VIRTUAL_H + y),
                    Vec2::new(sx, VIRTUAL_H + y),
                    tint,
                );
                sx += step;
            }
        }
    }

    pub fn draw_terrain(&self, track: &Track, cam_x: f32, cam_y: f32, palette: &Palette) {
        let terrain = &track.terrain;
        if terrain.is_empty() {
            return;
        }
        // One extra column each side so the fill never shows a seam at the edge.
        let first = ((cam_x / COLUMN_W).floor() as i32 - 1).max(0) as usize;
        let last = (((cam_x + VIRTUAL_W) / COLUMN_W).ceil() as i32 + 1)
            .max(0)
            .min(terrain.columns() as i32 - 1) as usize;
        if last <= first {
            return;
        }

        let sx = |i: usize| i as f32 * COLUMN_W - cam_x;

        for i in first..last {
            let (x0, x1) = (sx(i), sx(i + 1));
            let (c0, c1) = (terrain.ceiling[i] + cam_y, terrain.ceiling[i + 1] + cam_y);
            let (f0, f1) = (terrain.floor[i] + cam_y, terrain.floor[i + 1] + cam_y);

            draw_triangle(
                Vec2::new(x0, -8.0),
                Vec2::new(x1, -8.0),
                Vec2::new(x1, c1),
                palette.rock_fill,
            );
            draw_triangle(
                Vec2::new(x0, -8.0),
                Vec2::new(x1, c1),
                Vec2::new(x0, c0),
                palette.rock_fill,
            );
            draw_triangle(
                Vec2::new(x0, f0),
                Vec2::new(x1, f1),
                Vec2::new(x1, VIRTUAL_H + 8.0),
                palette.rock_fill,
            );
            draw_triangle(
                Vec2::new(x0, f0),
                Vec2::new(x1, VIRTUAL_H + 8.0),
                Vec2::new(x0, VIRTUAL_H + 8.0),
                palette.rock_fill,
            );
        }

        // The glowing edges go on afterwards so the fill never covers them.
        for i in first..last {
            let (x0, x1) = (sx(i), sx(i + 1));
            self.glow_line(
                Vec2::new(x0, terrain.ceiling[i] + cam_y),
                Vec2::new(x1, terrain.ceiling[i + 1] + cam_y),
                1.0,
                palette.rock_edge,
            );
            self.glow_line(
                Vec2::new(x0, terrain.floor[i] + cam_y),
                Vec2::new(x1, terrain.floor[i + 1] + cam_y),
                1.0,
                palette.rock_edge,
            );
        }
    }

    fn draw_ship(&self, world: &World, cam_x: f32, cam_y: f32) {
        let p = &world.player;
        let dir = world.phase.facing();
        let pos = Vec2::new(p.pos.x - cam_x, p.pos.y + cam_y);

        // Blink while invulnerable, but always leave the ship visible enough to
        // fly: alpha floors at 0.35 rather than going to zero.
        let alpha = if p.invulnerable() {
            0.35 + 0.65 * ((self.clock * 22.0).sin() * 0.5 + 0.5)
        } else {
            1.0
        };
        let color = fade(
            if p.invulnerable() {
                theme::SHIP_INVULN
            } else {
                theme::SHIP
            },
            alpha,
        );

        // Hull outline in ship space, nose at +x.
        let hull = [
            Vec2::new(10.0, 0.0),
            Vec2::new(2.0, -4.5),
            Vec2::new(-7.0, -3.5),
            Vec2::new(-9.0, 0.0),
            Vec2::new(-7.0, 3.5),
            Vec2::new(2.0, 4.5),
        ];
        let wing = [
            Vec2::new(-1.0, -1.5),
            Vec2::new(-6.0, -8.5),
            Vec2::new(-9.0, -8.0),
            Vec2::new(-5.0, -1.0),
        ];
        let wing2 = [
            Vec2::new(-1.0, 1.5),
            Vec2::new(-6.0, 8.5),
            Vec2::new(-9.0, 8.0),
            Vec2::new(-5.0, 1.0),
        ];

        let to_world = |v: Vec2| {
            // Roll first, then mirror for the direction of travel.
            let (s, c) = p.tilt.sin_cos();
            let r = Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c);
            pos + Vec2::new(r.x * dir, r.y)
        };

        let hull_pts: Vec<Vec2> = hull.iter().map(|v| to_world(*v)).collect();
        self.fill_poly(&hull_pts, Color::new(0.02, 0.08, 0.12, 0.9));
        self.glow_path(&hull_pts, true, 1.1, color);

        for w in [&wing, &wing2] {
            let pts: Vec<Vec2> = w.iter().map(|v| to_world(*v)).collect();
            self.glow_path(&pts, true, 0.8, fade(color, 0.85));
        }

        // Canopy.
        self.glow_dot(to_world(Vec2::new(3.0, -0.5)), 1.2, fade(WHITE, alpha));
    }

    fn draw_enemies(&self, world: &World, cam_x: f32, cam_y: f32) {
        for e in &world.enemies {
            let pos = Vec2::new(e.pos.x - cam_x, e.pos.y + cam_y);
            let base = theme::enemy_color(e.kind);
            // Flash white on damage so hits always register visually, even when
            // the thing being hit is small and busy.
            let color = mix(base, WHITE, e.hit_flash);

            match e.kind {
                SpawnKind::Silo => {
                    let body = [
                        Vec2::new(-7.0, 6.0),
                        Vec2::new(-4.0, -5.0),
                        Vec2::new(4.0, -5.0),
                        Vec2::new(7.0, 6.0),
                    ];
                    let pts: Vec<Vec2> = body.iter().map(|v| pos + *v).collect();
                    self.fill_poly(&pts, Color::new(0.08, 0.02, 0.02, 0.85));
                    self.glow_path(&pts, true, 1.0, color);
                    // Open hatch, aimed at the sky.
                    self.glow_line(
                        pos + Vec2::new(-2.5, -5.0),
                        pos + Vec2::new(-2.5, -9.0),
                        0.9,
                        color,
                    );
                    self.glow_line(
                        pos + Vec2::new(2.5, -5.0),
                        pos + Vec2::new(2.5, -9.0),
                        0.9,
                        color,
                    );
                }
                SpawnKind::Radar => {
                    let mast = [
                        Vec2::new(-5.0, 7.0),
                        Vec2::new(-2.0, -2.0),
                        Vec2::new(2.0, -2.0),
                        Vec2::new(5.0, 7.0),
                    ];
                    let pts: Vec<Vec2> = mast.iter().map(|v| pos + *v).collect();
                    self.fill_poly(&pts, Color::new(0.02, 0.09, 0.04, 0.85));
                    self.glow_path(&pts, true, 1.0, color);

                    // The dish sweeps; a stopped dish would read as broken.
                    let sweep = (e.anim * 1.6).sin() * 0.9;
                    let dish_dir = Vec2::new(sweep.sin(), -sweep.cos());
                    let centre = pos + Vec2::new(0.0, -4.0);
                    let across = Vec2::new(-dish_dir.y, dish_dir.x);
                    self.glow_line(
                        centre + across * 6.0 - dish_dir * 1.5,
                        centre - across * 6.0 - dish_dir * 1.5,
                        1.0,
                        color,
                    );
                    self.glow_line(centre, centre + dish_dir * 5.0, 0.9, color);
                    // Emission arcs, so an active radar is legible at a glance.
                    let pulse = (e.anim * 3.0).sin() * 0.5 + 0.5;
                    self.glow_circle(centre + dish_dir * 6.0, 3.0 + pulse * 4.0, 0.6, fade(color, 0.5 * (1.0 - pulse)));
                }
                SpawnKind::Turret => {
                    let body = [
                        Vec2::new(-6.0, -6.0),
                        Vec2::new(6.0, -6.0),
                        Vec2::new(4.0, 3.0),
                        Vec2::new(-4.0, 3.0),
                    ];
                    let pts: Vec<Vec2> = body.iter().map(|v| pos + *v).collect();
                    self.fill_poly(&pts, Color::new(0.10, 0.05, 0.01, 0.85));
                    self.glow_path(&pts, true, 1.0, color);

                    let aim = (world.player.pos - e.pos).normalize_or_zero();
                    self.glow_line(pos, pos + aim * 8.0, 1.4, color);
                }
                SpawnKind::Mine => {
                    self.glow_circle(pos, MINE_RADIUS, 1.0, color);
                    for k in 0..6 {
                        let a = k as f32 * std::f32::consts::TAU / 6.0 + e.anim * 0.6;
                        let d = Vec2::new(a.cos(), a.sin());
                        self.glow_line(pos + d * MINE_RADIUS, pos + d * (MINE_RADIUS + 3.5), 0.8, color);
                    }
                    let blink = ((e.anim * 4.0).sin() * 0.5 + 0.5).powi(3);
                    self.glow_dot(pos, 1.4, fade(WHITE, blink));
                }
                SpawnKind::Drone => {
                    let facing = e.vel.normalize_or_zero();
                    let (s, c) = facing.y.atan2(facing.x).sin_cos();
                    let rot = |v: Vec2| pos + Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c);
                    let body = [
                        rot(Vec2::new(7.0, 0.0)),
                        rot(Vec2::new(-2.0, -5.0)),
                        rot(Vec2::new(-5.0, 0.0)),
                        rot(Vec2::new(-2.0, 5.0)),
                    ];
                    self.fill_poly(&body, Color::new(0.07, 0.03, 0.10, 0.85));
                    self.glow_path(&body, true, 1.0, color);
                    self.glow_dot(rot(Vec2::new(2.0, 0.0)), 1.0, fade(WHITE, 0.8));
                }
                SpawnKind::Warhead => {
                    let pulse = (e.anim * 2.2).sin() * 0.5 + 0.5;
                    let body = [
                        Vec2::new(0.0, -15.0),
                        Vec2::new(7.0, -6.0),
                        Vec2::new(7.0, 10.0),
                        Vec2::new(-7.0, 10.0),
                        Vec2::new(-7.0, -6.0),
                    ];
                    let pts: Vec<Vec2> = body.iter().map(|v| pos + *v).collect();
                    self.fill_poly(&pts, Color::new(0.12, 0.02, 0.06, 0.9));
                    self.glow_path(&pts, true, 1.3, color);
                    // Fins.
                    self.glow_line(pos + Vec2::new(-7.0, 4.0), pos + Vec2::new(-12.0, 10.0), 1.0, color);
                    self.glow_line(pos + Vec2::new(7.0, 4.0), pos + Vec2::new(12.0, 10.0), 1.0, color);
                    // Core, pulsing on a timer that says "do something about this".
                    self.glow_dot(pos + Vec2::new(0.0, 0.0), 2.0 + pulse * 2.5, fade(WHITE, 0.35 + pulse * 0.5));
                    self.glow_circle(pos, 11.0 + pulse * 3.0, 0.7, fade(color, 0.4 * (1.0 - pulse)));

                    // Damage readout, because "am I getting anywhere" matters here.
                    let frac = e.hp as f32 / e.max_hp as f32;
                    let w = 26.0;
                    draw_rectangle(pos.x - w * 0.5, pos.y + 16.0, w, 2.0, fade(theme::HUD_DIM, 0.5));
                    draw_rectangle(pos.x - w * 0.5, pos.y + 16.0, w * frac, 2.0, color);
                }
            }
        }
    }

    fn draw_projectiles(&self, world: &World, cam_x: f32, cam_y: f32) {
        for p in &world.projectiles {
            let pos = Vec2::new(p.pos.x - cam_x, p.pos.y + cam_y);
            match p.kind {
                ProjectileKind::Cannon => {
                    let dir = p.vel.normalize_or_zero();
                    self.glow_line(pos - dir * 4.0, pos + dir * 2.5, 1.3, theme::CANNON_ROUND);
                }
                ProjectileKind::Bomb => {
                    let dir = p.vel.normalize_or_zero();
                    self.glow_line(pos - dir * 3.5, pos + dir * 3.0, 2.0, theme::BOMB);
                    self.glow_dot(pos, 1.4, theme::BOMB);
                }
                ProjectileKind::Sam => {
                    // Colour encodes the thing the player most needs to know:
                    // red is tracking you, orange is not.
                    let tint = if p.homing {
                        theme::SAM_TRACKING
                    } else {
                        theme::SAM_BALLISTIC
                    };
                    let dir = p.vel.normalize_or_zero();
                    self.glow_line(pos - dir * 5.0, pos + dir * 3.0, 1.5, tint);
                    let flare = 0.6 + 0.4 * (self.clock * 40.0).sin();
                    self.glow_dot(pos - dir * 6.0, 1.6 * flare, fade(theme::EXPLOSION, 0.9));
                    if p.homing {
                        self.glow_circle(pos, 5.0, 0.5, fade(tint, 0.35));
                    }
                }
                ProjectileKind::Shell => {
                    self.glow_dot(pos, 2.0, theme::SHELL);
                }
            }
        }
    }

    fn draw_particles(&self, fx: &Fx, cam_x: f32, cam_y: f32) {
        for p in &fx.particles {
            let pos = Vec2::new(p.pos.x - cam_x, p.pos.y + cam_y);
            let color = Fx::particle_color(p);
            match p.kind {
                ParticleKind::Spark | ParticleKind::Exhaust => {
                    draw_circle(pos.x, pos.y, p.size, color);
                }
                ParticleKind::Smoke => {
                    draw_circle(pos.x, pos.y, p.size, color);
                }
                ParticleKind::Debris => {
                    // A short streak in the direction of travel reads better than
                    // a dot at these sizes.
                    let d = p.vel.normalize_or_zero() * p.size * 1.6;
                    draw_line(pos.x - d.x, pos.y - d.y, pos.x + d.x, pos.y + d.y, p.size, color);
                }
            }
        }
    }

    fn draw_shockwaves(&self, fx: &Fx, cam_x: f32, cam_y: f32) {
        for w in &fx.waves {
            let t = (w.life / w.max_life).clamp(0.0, 1.0);
            let pos = Vec2::new(w.pos.x - cam_x, w.pos.y + cam_y);
            self.glow_circle(pos, w.radius, 1.0 + t * 1.5, fade(w.color, t * 0.8));
        }
    }

    fn draw_float_text(&self, fx: &Fx, cam_x: f32, cam_y: f32) {
        for t in &fx.texts {
            let a = (t.life / t.max_life).clamp(0.0, 1.0);
            self.text_centered(
                &t.text,
                t.pos.x - cam_x,
                t.pos.y + cam_y,
                t.size,
                fade(t.color, a),
            );
        }
    }

    // -----------------------------------------------------------------------
    // Post
    // -----------------------------------------------------------------------

    /// Scanlines, vignette and a slow rolling brightness band.
    ///
    /// Drawn in virtual space, so the scanlines scale up with the window and stay
    /// chunky instead of dissolving into a grey haze at high resolutions — which
    /// is the whole point of them.
    pub fn draw_crt(&self) {
        if !self.crt {
            return;
        }
        let line = Color::new(0.0, 0.0, 0.0, 0.20);
        let mut y = 0.0;
        while y < VIRTUAL_H {
            draw_rectangle(0.0, y, VIRTUAL_W, 1.0, line);
            y += 2.0;
        }

        // Rolling band.
        let band_y = (self.clock * 26.0) % (VIRTUAL_H + 90.0) - 45.0;
        draw_rectangle(0.0, band_y, VIRTUAL_W, 26.0, Color::new(0.55, 0.75, 1.0, 0.018));

        // Vignette: concentric frames of increasing alpha toward the edge.
        for i in 0..16 {
            let t = i as f32 / 15.0;
            let inset = t * 26.0;
            draw_rectangle_lines(
                inset,
                inset,
                VIRTUAL_W - inset * 2.0,
                VIRTUAL_H - inset * 2.0,
                2.0,
                Color::new(0.0, 0.0, 0.0, 0.05 * (1.0 - t)),
            );
        }
    }

    /// Frame-time and entity counts. F3.
    pub fn draw_debug(&self, world: &World) {
        if !self.debug {
            return;
        }
        let lines = [
            format!("fps {:>3}  dt {:>5.1}ms", get_fps(), get_frame_time() * 1000.0),
            format!("cam {:>7.1}  ship {:>7.1},{:>5.1}", world.cam_x, world.player.pos.x, world.player.pos.y),
            format!("phase {:?}  state {:?}", world.phase, world.state),
            format!("enemies {:>3}  shots {:>3}  parts {:>4}", world.enemies.len(), world.projectiles.len(), world.fx.particles.len()),
            format!("zone {} ({}/{})", world.zone_name(), world.zone_index() + 1, world.track.zones.len()),
            format!("radar {}/{}  seed {}", world.radars_alive, world.radars_total, world.track.seed),
        ];
        let h = lines.len() as f32 * 8.0 + 6.0;
        draw_rectangle(2.0, VIRTUAL_H - h - 2.0, 210.0, h, Color::new(0.0, 0.0, 0.0, 0.65));
        for (i, l) in lines.iter().enumerate() {
            self.text(l, 5.0, VIRTUAL_H - h + 6.0 + i as f32 * 8.0, 6.5, theme::HUD_GOOD);
        }
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

/// Pulls a height toward the middle of the screen, flattening the parallax
/// layers so they read as distant.
fn mix_toward(value: f32, target: f32, amount: f32) -> f32 {
    value + (target - value) * amount
}

/// The palette at a world position, cross-faded across zone boundaries so the
/// colour scheme changes over a screen width rather than in a single frame.
pub fn blend_palette(track: &Track, x: f32) -> Palette {
    if track.zones.is_empty() {
        return theme::palette(0);
    }
    let idx = track.zone_index_at(x);
    let zone = &track.zones[idx];
    let here = theme::palette(zone.palette);

    // How far into the fade zone at each end of this span.
    const FADE: f32 = 220.0;
    let from_start = x - zone.start_x();
    let to_end = zone.end_x() - x;

    if from_start < FADE && idx > 0 {
        let prev = theme::palette(track.zones[idx - 1].palette);
        let t = remap(from_start, 0.0, FADE, 0.5, 1.0);
        blend(prev, here, t)
    } else if to_end < FADE && idx + 1 < track.zones.len() {
        let next = theme::palette(track.zones[idx + 1].palette);
        let t = remap(to_end, 0.0, FADE, 0.5, 1.0);
        blend(next, here, t)
    } else {
        here
    }
}

fn blend(a: Palette, b: Palette, t: f32) -> Palette {
    Palette {
        rock_edge: mix(a.rock_edge, b.rock_edge, t),
        rock_fill: mix(a.rock_fill, b.rock_fill, t),
        backdrop: mix(a.backdrop, b.backdrop, t),
        sky: mix(a.sky, b.sky, t),
    }
}

/// The colour a phase should be drawn in on the HUD.
pub fn phase_color(phase: Phase) -> Color {
    match phase {
        Phase::Outbound => theme::HUD_TEXT,
        Phase::BunkerHold => theme::WARHEAD,
        Phase::Egress => theme::HUD_GOOD,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::generate_campaign;

    #[test]
    fn the_viewport_preserves_aspect_and_centres() {
        // Wider than 16:9: bars on the left and right.
        let v = Viewport::fit(1920.0, 1080.0);
        assert!((v.w / v.h - ASPECT).abs() < 1e-3);
        assert!((v.x).abs() < 1.0 && (v.y).abs() < 1.0);

        let wide = Viewport::fit(2400.0, 1080.0);
        assert!((wide.w / wide.h - ASPECT).abs() < 1e-3);
        assert!(wide.x > 0.0, "should letterbox horizontally");
        assert!((wide.h - 1080.0).abs() < 1.0);

        let tall = Viewport::fit(1000.0, 1000.0);
        assert!(tall.y > 0.0, "should letterbox vertically");
        assert!((tall.w - 1000.0).abs() < 1.0);
    }

    #[test]
    fn the_viewport_survives_a_degenerate_window() {
        let v = Viewport::fit(0.0, 0.0);
        assert!(v.scale.is_finite() && v.scale > 0.0);
        assert!(v.w.is_finite() && v.h.is_finite());
    }

    #[test]
    fn mouse_mapping_is_the_inverse_of_the_viewport() {
        let v = Viewport::fit(1600.0, 900.0);
        for (vx, vy) in [(0.0, 0.0), (VIRTUAL_W, VIRTUAL_H), (240.0, 135.0)] {
            let screen = Vec2::new(v.x + vx * v.scale, v.y + vy * v.scale);
            let back = v.to_virtual(screen);
            assert!((back.x - vx).abs() < 0.01 && (back.y - vy).abs() < 0.01);
        }
    }

    #[test]
    fn mouse_mapping_clamps_to_the_canvas() {
        let v = Viewport::fit(2400.0, 1080.0);
        let off = v.to_virtual(Vec2::new(-500.0, -500.0));
        assert_eq!(off, Vec2::new(0.0, 0.0));
        let far = v.to_virtual(Vec2::new(9999.0, 9999.0));
        assert_eq!(far, Vec2::new(VIRTUAL_W, VIRTUAL_H));
    }

    /// Projects a virtual-canvas point through the camera into normalised device
    /// coordinates, where y = +1 is the top of the screen and y = -1 the bottom.
    fn to_ndc(cam: &Camera2D, p: Vec2) -> Vec2 {
        let clip = cam.matrix() * Vec4::new(p.x, p.y, 0.0, 1.0);
        Vec2::new(clip.x, clip.y)
    }

    #[test]
    fn the_canvas_is_not_upside_down() {
        // This is the regression test for a bug that shipped once already.
        // `Camera2D::from_display_rect` is written for render targets and comes
        // back with a negative zoom.y; macroquad then applies its own inversion
        // for cameras with no render target. Left alone, the two cancel and the
        // whole game renders upside down — which no other test in this project
        // can see, because the matrix is the only place it exists.
        let cam = Renderer::camera(Viewport::fit(1920.0, 1080.0));

        let top = to_ndc(&cam, Vec2::new(VIRTUAL_W * 0.5, 0.0));
        let bottom = to_ndc(&cam, Vec2::new(VIRTUAL_W * 0.5, VIRTUAL_H));
        assert!(
            top.y > 0.9,
            "virtual y=0 must land at the top of the screen, got NDC y={}",
            top.y
        );
        assert!(
            bottom.y < -0.9,
            "virtual y={VIRTUAL_H} must land at the bottom, got NDC y={}",
            bottom.y
        );
    }

    #[test]
    fn the_canvas_is_not_mirrored_either() {
        let cam = Renderer::camera(Viewport::fit(1920.0, 1080.0));
        let left = to_ndc(&cam, Vec2::new(0.0, VIRTUAL_H * 0.5));
        let right = to_ndc(&cam, Vec2::new(VIRTUAL_W, VIRTUAL_H * 0.5));
        assert!(left.x < -0.9, "virtual x=0 should be screen-left");
        assert!(right.x > 0.9, "virtual x={VIRTUAL_W} should be screen-right");
    }

    #[test]
    fn the_canvas_centre_maps_to_the_centre_of_the_screen() {
        let cam = Renderer::camera(Viewport::fit(1600.0, 900.0));
        let mid = to_ndc(&cam, Vec2::new(VIRTUAL_W * 0.5, VIRTUAL_H * 0.5));
        assert!(mid.x.abs() < 1e-5 && mid.y.abs() < 1e-5, "centre drifted: {mid:?}");
    }

    #[test]
    fn the_camera_and_the_mouse_mapping_agree_about_which_way_is_up() {
        // The editor maps the mouse with its own arithmetic. If that ever
        // disagrees with the render, sculpting the ceiling would move the floor.
        let viewport = Viewport::fit(1600.0, 900.0);
        let cam = Renderer::camera(viewport);

        // A point near the top of the window should map to a small virtual y...
        let near_top = viewport.to_virtual(Vec2::new(
            viewport.x + viewport.w * 0.5,
            viewport.y + viewport.h * 0.1,
        ));
        assert!(near_top.y < VIRTUAL_H * 0.5);
        // ...and that virtual y should project back to the upper half of NDC.
        assert!(to_ndc(&cam, near_top).y > 0.0);
    }

    #[test]
    fn palette_blending_is_continuous_across_a_zone_seam() {
        let track = generate_campaign(77);
        let seam = track.zones[1].start_x();
        let before = blend_palette(&track, seam - 1.0);
        let after = blend_palette(&track, seam + 1.0);
        // The two sides of a boundary must be near-identical, or the transition
        // pops in a single frame.
        assert!((before.rock_edge.r - after.rock_edge.r).abs() < 0.05);
        assert!((before.rock_edge.g - after.rock_edge.g).abs() < 0.05);
        assert!((before.rock_edge.b - after.rock_edge.b).abs() < 0.05);
    }

    #[test]
    fn palette_blending_handles_the_ends_of_the_track() {
        let track = generate_campaign(78);
        let _ = blend_palette(&track, -5000.0);
        let _ = blend_palette(&track, track.world_len() + 5000.0);
    }

    #[test]
    fn palette_blending_survives_an_empty_track() {
        let _ = blend_palette(&Track::default(), 0.0);
    }
}
