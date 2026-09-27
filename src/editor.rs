//! The landscape editor.
//!
//! The 1983 original shipped with a level editor, which was extraordinary for a
//! 48K game and is the main reason people still talk about it. Leaving it out of
//! a redesign would be missing the point, so it is here: sculpt the cave with a
//! brush, drop emplacements where you like, save, and fly it.
//!
//! It edits a [`Track`] in place — the same type the campaign generator produces
//! and the same type the simulation consumes — so anything you can build is
//! automatically playable, and playtesting is a straight handoff with no export
//! step.

use crate::config::*;
use crate::level::{parse_pen, write_pen, Anchor, Spawn, SpawnKind, Track};
use crate::render::{blend_palette, Renderer};
use crate::save::{self, CUSTOM_LEVEL};
use crate::terrain::Surface;
use crate::theme;
use crate::util::fade;
use macroquad::prelude::*;

/// The narrowest gap the editor considers flyable. Anything tighter is
/// highlighted; it is a warning rather than a prohibition, because a deliberate
/// wall is a legitimate thing to build.
pub const MIN_PLAYABLE_GAP: f32 = 52.0;

const PAN_SPEED: f32 = 420.0;
const SCULPT_RATE: f32 = 150.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Ceiling,
    Floor,
    Place,
    Erase,
}

impl Tool {
    fn label(self) -> &'static str {
        match self {
            Tool::Ceiling => "SCULPT CEILING",
            Tool::Floor => "SCULPT FLOOR",
            Tool::Place => "PLACE",
            Tool::Erase => "ERASE",
        }
    }
}

/// What the editor wants the application to do next.
#[derive(Clone, Debug)]
pub enum Exit {
    Stay,
    Menu,
    /// Fly what is on screen.
    Playtest(Box<Track>),
}

pub struct Editor {
    pub track: Track,
    pub cam_x: f32,
    pub tool: Tool,
    pub brush: f32,
    /// Index into [`SpawnKind::PALETTE`].
    pub kind: usize,
    status: Option<(String, f32)>,
    pub dirty: bool,
    pub show_help: bool,
}

impl Editor {
    pub fn new(track: Track) -> Self {
        Editor {
            track,
            cam_x: 0.0,
            tool: Tool::Floor,
            brush: 26.0,
            kind: 0,
            status: None,
            dirty: false,
            show_help: true,
        }
    }

    /// Shows a transient message along the bottom of the editor.
    pub fn say(&mut self, message: impl Into<String>) {
        self.status = Some((message.into(), 3.0));
    }

    fn selected_kind(&self) -> SpawnKind {
        SpawnKind::PALETTE[self.kind % SpawnKind::PALETTE.len()]
    }

    pub fn update(&mut self, r: &Renderer, dt: f32) -> Exit {
        if let Some((_, t)) = self.status.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                self.status = None;
            }
        }

        if is_key_pressed(KeyCode::Escape) {
            return Exit::Menu;
        }
        if is_key_pressed(KeyCode::F5) {
            return Exit::Playtest(Box::new(self.track.clone()));
        }
        if is_key_pressed(KeyCode::H) {
            self.show_help = !self.show_help;
        }

        self.handle_files();
        self.handle_tool_selection();
        self.handle_pan(dt);
        self.handle_brush(r, dt);

        Exit::Stay
    }

    fn handle_files(&mut self) {
        let ctrl = is_key_down(KeyCode::LeftControl) || is_key_down(KeyCode::RightControl);
        if !ctrl {
            return;
        }
        if is_key_pressed(KeyCode::S) {
            match save::write_level(CUSTOM_LEVEL, &write_pen(&self.track)) {
                Ok(()) => {
                    self.dirty = false;
                    self.say(format!("saved to {CUSTOM_LEVEL}"));
                }
                Err(e) => self.say(e),
            }
        }
        if is_key_pressed(KeyCode::O) {
            match save::read_level(CUSTOM_LEVEL).and_then(|t| parse_pen(&t).map_err(|e| e.to_string())) {
                Ok(track) => {
                    self.track = track;
                    self.cam_x = 0.0;
                    self.dirty = false;
                    self.say(format!("loaded {CUSTOM_LEVEL}"));
                }
                Err(e) => self.say(e),
            }
        }
        if is_key_pressed(KeyCode::N) {
            // A fresh generated campaign, seeded off the current one so repeated
            // presses keep producing something new.
            let seed = self.track.seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            self.track = crate::level::generate_campaign(seed);
            self.cam_x = 0.0;
            self.dirty = false;
            self.say(format!("generated a new cave from seed {seed}"));
        }
        if is_key_pressed(KeyCode::B) {
            self.track = blank_track();
            self.cam_x = 0.0;
            self.dirty = true;
            self.say("blank canvas - sculpt it, then place a warhead");
        }
    }

    fn handle_tool_selection(&mut self) {
        for (key, tool) in [
            (KeyCode::Key1, Tool::Ceiling),
            (KeyCode::Key2, Tool::Floor),
            (KeyCode::Key3, Tool::Place),
            (KeyCode::Key4, Tool::Erase),
        ] {
            if is_key_pressed(key) {
                self.tool = tool;
            }
        }
        if is_key_pressed(KeyCode::Tab) {
            self.kind = (self.kind + 1) % SpawnKind::PALETTE.len();
            self.tool = Tool::Place;
        }
    }

    fn handle_pan(&mut self, dt: f32) {
        let mut pan = 0.0;
        if is_key_down(KeyCode::A) || is_key_down(KeyCode::Left) {
            pan -= 1.0;
        }
        if is_key_down(KeyCode::D) || is_key_down(KeyCode::Right) {
            pan += 1.0;
        }
        let fast = if is_key_down(KeyCode::LeftShift) { 3.0 } else { 1.0 };
        self.cam_x += pan * PAN_SPEED * fast * dt;

        if is_key_pressed(KeyCode::Home) {
            self.cam_x = 0.0;
        }
        if is_key_pressed(KeyCode::End) {
            self.cam_x = (self.track.world_len() - VIRTUAL_W).max(0.0);
        }
        self.cam_x = self
            .cam_x
            .clamp(0.0, (self.track.world_len() - VIRTUAL_W).max(0.0));
    }

    fn handle_brush(&mut self, r: &Renderer, dt: f32) {
        let (_, wheel) = mouse_wheel();
        if wheel != 0.0 {
            self.brush = (self.brush + wheel.signum() * 4.0).clamp(6.0, 90.0);
        }
        if is_key_pressed(KeyCode::LeftBracket) {
            self.brush = (self.brush - 4.0).max(6.0);
        }
        if is_key_pressed(KeyCode::RightBracket) {
            self.brush = (self.brush + 4.0).min(90.0);
        }

        let m = r.viewport.to_virtual(Vec2::from(mouse_position()));
        // Ignore clicks on the toolbar, or dragging a wall becomes impossible
        // near the top of the screen.
        if m.y < HUD_H {
            return;
        }
        let world = Vec2::new(self.cam_x + m.x, m.y);

        let left = is_mouse_button_down(MouseButton::Left);
        let right = is_mouse_button_down(MouseButton::Right);

        match self.tool {
            Tool::Ceiling | Tool::Floor => {
                let surface = if self.tool == Tool::Ceiling {
                    Surface::Ceiling
                } else {
                    Surface::Floor
                };
                // Left grows rock into the cave, right carves it away.
                let delta = if left {
                    SCULPT_RATE
                } else if right {
                    -SCULPT_RATE
                } else {
                    0.0
                };
                if delta != 0.0 {
                    self.track
                        .terrain
                        .sculpt(surface, world.x, self.brush, delta * dt);
                    self.dirty = true;
                }
            }
            Tool::Place => {
                if is_mouse_button_pressed(MouseButton::Left) {
                    self.place(world);
                }
                if is_mouse_button_pressed(MouseButton::Right) {
                    self.erase(world);
                }
            }
            Tool::Erase => {
                if is_mouse_button_pressed(MouseButton::Left) {
                    self.erase(world);
                }
            }
        }
    }

    fn place(&mut self, world: Vec2) {
        let kind = self.selected_kind();
        // Only one warhead makes sense; placing a second replaces the first.
        if kind == SpawnKind::Warhead {
            self.track.spawns.retain(|s| s.kind != SpawnKind::Warhead);
        }
        self.track.spawns.push(Spawn {
            kind,
            x: world.x,
            y: world.y,
        });
        self.dirty = true;
        self.say(format!("placed {}", kind.label()));
    }

    fn erase(&mut self, world: Vec2) {
        let mut best: Option<(usize, f32)> = None;
        for (i, s) in self.track.spawns.iter().enumerate() {
            let (x, y) = self.track.spawn_position(s);
            let d = Vec2::new(x, y).distance(world);
            if d < 14.0 && best.map(|(_, bd)| d < bd).unwrap_or(true) {
                best = Some((i, d));
            }
        }
        if let Some((i, _)) = best {
            let kind = self.track.spawns[i].kind;
            self.track.spawns.remove(i);
            self.dirty = true;
            self.say(format!("removed {}", kind.label()));
        }
    }

    // -----------------------------------------------------------------------
    // Drawing
    // -----------------------------------------------------------------------

    pub fn draw(&self, r: &Renderer) {
        let palette = blend_palette(&self.track, self.cam_x + VIRTUAL_W * 0.5);
        draw_rectangle(0.0, 0.0, VIRTUAL_W, VIRTUAL_H, palette.sky);
        r.draw_parallax(&self.track, self.cam_x, 0.0, &palette);
        r.draw_terrain(&self.track, self.cam_x, 0.0, &palette);

        self.draw_impassable(r);
        self.draw_spawns(r);
        self.draw_cursor(r);
        self.draw_toolbar(r);
        if self.show_help {
            self.draw_help(r);
        }
        self.draw_status(r);
    }

    /// Shades any column too tight to fly through.
    fn draw_impassable(&self, r: &Renderer) {
        let terrain = &self.track.terrain;
        let first = terrain.column_at(self.cam_x);
        let last = (terrain.column_at(self.cam_x + VIRTUAL_W) + 1).min(terrain.columns());
        for i in first..last {
            let gap = terrain.floor[i] - terrain.ceiling[i];
            if gap >= MIN_PLAYABLE_GAP {
                continue;
            }
            let x = i as f32 * COLUMN_W - self.cam_x;
            draw_rectangle(x, terrain.ceiling[i], COLUMN_W, gap.max(1.0), fade(theme::HUD_WARN, 0.45));
            r.glow_line(
                Vec2::new(x, terrain.ceiling[i]),
                Vec2::new(x, terrain.floor[i]),
                0.8,
                theme::HUD_WARN,
            );
        }
    }

    fn draw_spawns(&self, r: &Renderer) {
        for s in &self.track.spawns {
            let (x, y) = self.track.spawn_position(s);
            let sx = x - self.cam_x;
            if !(-30.0..=VIRTUAL_W + 30.0).contains(&sx) {
                continue;
            }
            let color = theme::enemy_color(s.kind);
            let pos = Vec2::new(sx, y);
            r.glow_circle(pos, 7.0, 0.9, color);
            r.glow_dot(pos, 1.6, color);
            // Anchored kinds get a tether to the surface they belong to, so it is
            // obvious they will move if you sculpt underneath them.
            match s.kind.anchor() {
                Anchor::Floor => r.glow_line(pos, pos + Vec2::new(0.0, 8.0), 0.5, fade(color, 0.6)),
                Anchor::Ceiling => r.glow_line(pos, pos - Vec2::new(0.0, 8.0), 0.5, fade(color, 0.6)),
                Anchor::Air => {}
            }
            r.text_centered(s.kind.as_str(), sx, y - 10.0, 5.0, fade(color, 0.8));
        }
    }

    fn draw_cursor(&self, r: &Renderer) {
        let m = r.viewport.to_virtual(Vec2::from(mouse_position()));
        match self.tool {
            Tool::Ceiling | Tool::Floor => {
                let color = if self.tool == Tool::Ceiling {
                    theme::SAM_BALLISTIC
                } else {
                    theme::RADAR
                };
                r.glow_circle(m, self.brush, 0.7, fade(color, 0.8));
                r.glow_line(
                    Vec2::new(m.x, HUD_H),
                    Vec2::new(m.x, VIRTUAL_H),
                    0.4,
                    fade(color, 0.25),
                );
            }
            Tool::Place => {
                let color = theme::enemy_color(self.selected_kind());
                r.glow_circle(m, 8.0, 0.8, color);
                r.text_centered(self.selected_kind().label(), m.x, m.y - 12.0, 5.5, color);
            }
            Tool::Erase => {
                r.glow_circle(m, 14.0, 0.8, theme::HUD_WARN);
                r.glow_line(m - Vec2::splat(5.0), m + Vec2::splat(5.0), 0.8, theme::HUD_WARN);
                r.glow_line(
                    m + Vec2::new(-5.0, 5.0),
                    m + Vec2::new(5.0, -5.0),
                    0.8,
                    theme::HUD_WARN,
                );
            }
        }
    }

    fn draw_toolbar(&self, r: &Renderer) {
        draw_rectangle(0.0, 0.0, VIRTUAL_W, HUD_H, theme::HUD_PANEL);
        draw_line(0.0, HUD_H, VIRTUAL_W, HUD_H, 0.6, fade(theme::HUD_TEXT, 0.25));

        r.text("EDITOR", 6.0, 9.0, 6.0, theme::HUD_DIM);
        r.text(self.tool.label(), 6.0, 18.0, 8.0, theme::HUD_TEXT);

        if self.tool == Tool::Place {
            r.text("PLACING", 108.0, 9.0, 5.5, theme::HUD_DIM);
            r.text(
                self.selected_kind().label(),
                108.0,
                18.0,
                7.5,
                theme::enemy_color(self.selected_kind()),
            );
        } else {
            r.text("BRUSH", 108.0, 9.0, 5.5, theme::HUD_DIM);
            r.text(&format!("{:.0}", self.brush), 108.0, 18.0, 7.5, theme::HUD_TEXT);
        }

        // Playability readout — the number that decides whether this is a level
        // or a wall. Only the route up to the warhead counts; a sealed dead end
        // behind the objective is scenery, not a fault.
        let bad = self.track.blocked_columns(MIN_PLAYABLE_GAP).len();
        r.text("ROUTE", 200.0, 9.0, 5.5, theme::HUD_DIM);
        if self.track.warhead_x().is_none() {
            r.text("NO TARGET", 200.0, 18.0, 7.5, theme::BOMB);
        } else if bad == 0 {
            r.text("CLEAR", 200.0, 18.0, 7.5, theme::HUD_GOOD);
        } else {
            r.text(&format!("{bad} BLOCKED"), 200.0, 18.0, 7.5, theme::HUD_WARN);
        }

        r.text("TIGHTEST", 276.0, 9.0, 5.5, theme::HUD_DIM);
        // Measured over the route, not the whole track — a sealed chamber behind
        // the objective is zero units wide on purpose, and reporting that would
        // mark every campaign the game generates as broken.
        let tightest = self.track.tightest_route_gap();
        r.text(
            &format!("{tightest:.0}px"),
            276.0,
            18.0,
            7.5,
            if tightest < MIN_PLAYABLE_GAP {
                theme::HUD_WARN
            } else {
                theme::HUD_TEXT
            },
        );

        r.text("LENGTH", 330.0, 9.0, 5.5, theme::HUD_DIM);
        r.text(
            &format!("{} COLS", self.track.terrain.columns()),
            330.0,
            18.0,
            7.5,
            theme::HUD_TEXT,
        );

        r.text_right(
            if self.dirty { "UNSAVED" } else { "SAVED" },
            VIRTUAL_W - 6.0,
            18.0,
            7.0,
            if self.dirty { theme::BOMB } else { theme::HUD_DIM },
        );
        r.text_right("H FOR KEYS", VIRTUAL_W - 6.0, 9.0, 5.5, theme::HUD_DIM);

        // Position along the track.
        let len = self.track.world_len().max(1.0);
        let t = (self.cam_x / len).clamp(0.0, 1.0);
        draw_rectangle(0.0, HUD_H + 1.0, VIRTUAL_W, 1.2, fade(theme::HUD_DIM, 0.3));
        draw_rectangle(
            t * (VIRTUAL_W - 40.0),
            HUD_H + 1.0,
            40.0,
            1.2,
            theme::HUD_TEXT,
        );
    }

    fn draw_help(&self, r: &Renderer) {
        const KEYS: &[(&str, &str)] = &[
            ("1 / 2", "sculpt ceiling / floor"),
            ("3 / 4", "place / erase objects"),
            ("TAB", "next object type"),
            ("LMB / RMB", "grow rock / carve rock"),
            ("WHEEL, [ ]", "brush size"),
            ("A D, HOME END", "pan (SHIFT for fast)"),
            ("CTRL+S / CTRL+O", "save / load levels/custom.pen"),
            ("CTRL+N / CTRL+B", "new generated cave / blank canvas"),
            ("F5", "playtest"),
            ("H", "hide this"),
            ("ESC", "back to menu"),
        ];
        let w = 176.0;
        let h = KEYS.len() as f32 * 9.0 + 14.0;
        let x = VIRTUAL_W - w - 6.0;
        let y = HUD_H + 8.0;
        draw_rectangle(x, y, w, h, Color::new(0.0, 0.02, 0.04, 0.82));
        draw_rectangle_lines(x, y, w, h, 0.8, fade(theme::HUD_TEXT, 0.3));
        let mut ty = y + 12.0;
        for (k, v) in KEYS {
            r.text(k, x + 6.0, ty, 6.0, theme::HUD_TEXT);
            r.text(v, x + 74.0, ty, 6.0, theme::HUD_DIM);
            ty += 9.0;
        }
    }

    fn draw_status(&self, r: &Renderer) {
        let Some((message, t)) = self.status.as_ref() else {
            return;
        };
        let alpha = (t / 3.0).clamp(0.0, 1.0).min(1.0);
        r.text_centered(
            message,
            VIRTUAL_W * 0.5,
            VIRTUAL_H - 10.0,
            7.0,
            fade(theme::HUD_TEXT, alpha),
        );
    }
}

/// An empty cave to build in from scratch: one wide-open zone with nothing in
/// it. The editor's blank canvas.
pub fn blank_track() -> Track {
    use crate::level::ZoneSpan;
    use crate::terrain::Terrain;

    const COLUMNS: usize = 420;
    Track {
        terrain: Terrain::flat(COLUMNS),
        zones: vec![ZoneSpan {
            name: "CUSTOM".to_string(),
            start_col: 0,
            end_col: COLUMNS,
            scroll: 120.0,
            palette: 0,
        }],
        spawns: Vec::new(),
        seed: 0,
        bunker_hold_x: (COLUMNS as f32 * COLUMN_W - VIRTUAL_W).max(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::generate_campaign;

    fn editor() -> Editor {
        Editor::new(generate_campaign(4321))
    }

    #[test]
    fn a_generated_campaign_opens_with_a_clear_route() {
        let e = editor();
        assert!(
            e.track.blocked_columns(MIN_PLAYABLE_GAP).is_empty(),
            "the generator should never hand the editor an unflyable route"
        );
    }

    #[test]
    fn sculpting_can_block_the_cave_and_the_editor_notices() {
        let mut e = editor();
        // Somewhere well before the warhead, so it counts as blocking the route.
        let x = 600.0;
        for _ in 0..80 {
            e.track.terrain.sculpt(Surface::Ceiling, x, 20.0, 8.0);
            e.track.terrain.sculpt(Surface::Floor, x, 20.0, 8.0);
        }
        assert!(
            !e.track.blocked_columns(MIN_PLAYABLE_GAP).is_empty(),
            "the warning must fire once the route is sealed"
        );
    }

    #[test]
    fn the_tightest_readout_ignores_the_sealed_dead_end() {
        let e = editor();
        // The whole-track figure is zero: the bunker ends in a wall.
        assert!(e.track.terrain.tightest_gap() < 1.0);
        // The figure the editor shows is about the route, and must be flyable.
        assert!(
            e.track.tightest_route_gap() >= MIN_PLAYABLE_GAP,
            "generated campaign reported an unflyable route gap of {}",
            e.track.tightest_route_gap()
        );
    }

    #[test]
    fn a_blank_canvas_is_empty_open_and_ready_to_build_in() {
        let t = blank_track();
        assert!(t.spawns.is_empty());
        assert_eq!(t.zones.len(), 1);
        assert!(t.tightest_route_gap() > MIN_PLAYABLE_GAP);
        assert!(t.blocked_columns(MIN_PLAYABLE_GAP).is_empty());
    }

    #[test]
    fn placing_and_erasing_round_trip() {
        let mut e = editor();
        let before = e.track.spawns.len();
        e.tool = Tool::Place;
        e.kind = 0; // silo
        e.place(Vec2::new(1000.0, 200.0));
        assert_eq!(e.track.spawns.len(), before + 1);
        assert!(e.dirty);

        // Erase snaps to the nearest object, and a silo is floor-anchored, so
        // aim at where it actually sits.
        let placed = *e.track.spawns.last().unwrap();
        let (x, y) = e.track.spawn_position(&placed);
        e.erase(Vec2::new(x, y));
        assert_eq!(e.track.spawns.len(), before);
    }

    #[test]
    fn erasing_empty_space_does_nothing() {
        let mut e = editor();
        let before = e.track.spawns.len();
        e.erase(Vec2::new(-9999.0, 0.0));
        assert_eq!(e.track.spawns.len(), before);
    }

    #[test]
    fn there_can_only_ever_be_one_warhead() {
        let mut e = editor();
        e.kind = SpawnKind::PALETTE
            .iter()
            .position(|k| *k == SpawnKind::Warhead)
            .unwrap();
        e.place(Vec2::new(500.0, 150.0));
        e.place(Vec2::new(900.0, 150.0));
        assert_eq!(
            e.track.spawns.iter().filter(|s| s.kind == SpawnKind::Warhead).count(),
            1
        );
    }

    #[test]
    fn an_edited_track_still_saves_and_reloads() {
        let mut e = editor();
        e.track.terrain.sculpt(Surface::Floor, 400.0, 30.0, 40.0);
        e.place(Vec2::new(420.0, 120.0));

        let text = write_pen(&e.track);
        let back = parse_pen(&text).expect("an edited track must round trip");
        assert_eq!(back.terrain.columns(), e.track.terrain.columns());
        assert_eq!(back.spawns.len(), e.track.spawns.len());
    }

    #[test]
    fn the_object_palette_cycles() {
        let mut e = editor();
        let first = e.selected_kind();
        for _ in 0..SpawnKind::PALETTE.len() {
            e.kind = (e.kind + 1) % SpawnKind::PALETTE.len();
        }
        assert_eq!(e.selected_kind(), first);
    }
}
