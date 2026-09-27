//! The shell: menus, screen transitions, and the frame loop that drives the
//! simulation.
//!
//! `App` owns everything that outlives a single run — the audio bank, the
//! renderer, the save file — and hands the current screen a slice of each frame.
//! A [`World`] exists only while a mission is being flown; abandoning a run drops
//! it, which is also how the game guarantees no state leaks between attempts.

use crate::audio::{Audio, Sfx};
use crate::config::*;
use crate::editor::{Editor, Exit};
use crate::hud;
use crate::input::{self, Frame, CONTROL_HELP};
use crate::level::{generate_campaign, parse_pen, SpawnKind, Track};
use crate::render::Renderer;
use crate::save::{self, Save};
use crate::theme;
use crate::util::fade;
use crate::world::{RunState, World};
use macroquad::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Screen {
    Menu,
    Help,
    Playing,
    Paused,
    RunOver,
    Editing,
}

/// A menu entry. Kept as data so the list can be filtered — the custom-level
/// entry only appears when there is a custom level to fly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MenuItem {
    NewRun,
    SameCave,
    CustomLevel,
    Editor,
    Help,
    Quit,
}

impl MenuItem {
    fn label(self) -> &'static str {
        match self {
            MenuItem::NewRun => "NEW SORTIE",
            MenuItem::SameCave => "FLY THE SAME CAVE",
            MenuItem::CustomLevel => "FLY A CUSTOM LEVEL",
            MenuItem::Editor => "LANDSCAPE EDITOR",
            MenuItem::Help => "BRIEFING",
            MenuItem::Quit => "QUIT",
        }
    }

    fn blurb(self) -> &'static str {
        match self {
            MenuItem::NewRun => "A freshly generated cave system",
            MenuItem::SameCave => "The last seed, for another attempt at it",
            MenuItem::CustomLevel => "What you built in the editor, or the shipped example",
            MenuItem::Editor => "Sculpt your own cave and fly it",
            MenuItem::Help => "Controls, targets, and how to survive",
            MenuItem::Quit => "",
        }
    }
}

pub struct App {
    screen: Screen,
    menu_index: usize,
    world: Option<World>,
    editor: Option<Editor>,
    save: Save,
    audio: Audio,
    renderer: Renderer,
    /// Backdrop for the menu: a real track, slowly scrolling past.
    attract: Track,
    attract_cam: f32,
    clock: f32,
    fullscreen: bool,
    pub quit: bool,
    /// Set when a run ends, so the score is only filed once.
    run_filed: bool,
    notice: Option<(String, f32)>,
}

impl App {
    pub async fn new() -> App {
        let save = Save::load();
        let mut audio = Audio::load().await;
        audio.muted = save.muted;

        let mut renderer = Renderer::new();
        renderer.crt = save.crt;

        let attract_seed = fresh_seed();
        App {
            screen: Screen::Menu,
            menu_index: 0,
            world: None,
            editor: None,
            save,
            audio,
            renderer,
            attract: generate_campaign(attract_seed),
            attract_cam: 0.0,
            clock: 0.0,
            fullscreen: false,
            quit: false,
            run_filed: false,
            notice: None,
        }
    }

    fn menu_items(&self) -> Vec<MenuItem> {
        let mut items = vec![MenuItem::NewRun];
        if self.save.last_seed != 0 {
            items.push(MenuItem::SameCave);
        }
        if save::playable_level().is_some() {
            items.push(MenuItem::CustomLevel);
        }
        items.extend([MenuItem::Editor, MenuItem::Help, MenuItem::Quit]);
        items
    }

    fn notify(&mut self, message: impl Into<String>) {
        self.notice = Some((message.into(), 4.0));
    }

    // -----------------------------------------------------------------------
    // Frame
    // -----------------------------------------------------------------------

    pub fn frame(&mut self) {
        // Real frame time, clamped once here so nothing downstream has to.
        let dt = get_frame_time().clamp(0.0, 0.25);
        self.clock += dt;
        if let Some((_, t)) = self.notice.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                self.notice = None;
            }
        }

        let input = input::read();
        self.handle_global(&input);

        self.renderer.begin(dt);

        match self.screen {
            Screen::Menu => {
                self.update_menu(&input, dt);
                self.draw_menu();
            }
            Screen::Help => {
                if input.cancel || input.confirm {
                    self.screen = Screen::Menu;
                    self.audio.play(Sfx::UiConfirm);
                }
                self.draw_attract(dt);
                self.draw_help();
            }
            Screen::Playing => {
                self.update_playing(&input, dt);
                self.draw_playing();
            }
            Screen::Paused => {
                self.update_paused(&input);
                self.draw_playing();
                hud::draw_pause(&self.renderer);
            }
            Screen::RunOver => {
                self.update_run_over(&input);
                self.draw_playing();
                if let Some(w) = self.world.as_ref() {
                    hud::draw_run_over(&self.renderer, w, self.save.best());
                }
            }
            Screen::Editing => {
                self.update_editor(dt);
                if let Some(e) = self.editor.as_ref() {
                    e.draw(&self.renderer);
                }
            }
        }

        self.draw_notice();
        self.renderer.draw_crt();
        if let Some(w) = self.world.as_ref() {
            self.renderer.draw_debug(w);
        }
        self.renderer.end();
    }

    /// Keys that work on every screen.
    fn handle_global(&mut self, input: &Frame) {
        if input.toggle_crt {
            self.renderer.crt = !self.renderer.crt;
            self.save.crt = self.renderer.crt;
            self.save.save();
        }
        if input.toggle_debug {
            self.renderer.debug = !self.renderer.debug;
        }
        if input.toggle_mute {
            self.audio.toggle_mute();
            self.save.muted = self.audio.muted;
            self.save.save();
        }
        if input.toggle_fullscreen {
            self.fullscreen = !self.fullscreen;
            set_fullscreen(self.fullscreen);
        }
    }

    // -----------------------------------------------------------------------
    // Menu
    // -----------------------------------------------------------------------

    fn update_menu(&mut self, input: &Frame, dt: f32) {
        self.attract_cam += 26.0 * dt;
        let span = (self.attract.world_len() - VIRTUAL_W).max(1.0);
        if self.attract_cam > span {
            self.attract_cam = 0.0;
        }

        let items = self.menu_items();
        if input.menu_next {
            self.menu_index = (self.menu_index + 1) % items.len();
            self.audio.play(Sfx::UiMove);
        }
        if input.menu_prev {
            self.menu_index = (self.menu_index + items.len() - 1) % items.len();
            self.audio.play(Sfx::UiMove);
        }
        if !input.confirm {
            return;
        }

        self.audio.play(Sfx::UiConfirm);
        match items[self.menu_index.min(items.len() - 1)] {
            MenuItem::NewRun => self.start_run(fresh_seed()),
            MenuItem::SameCave => {
                let seed = self.save.last_seed;
                self.start_run(seed);
            }
            MenuItem::CustomLevel => self.start_custom(),
            MenuItem::Editor => {
                let seed = fresh_seed();
                self.editor = Some(Editor::new(generate_campaign(seed)));
                self.screen = Screen::Editing;
            }
            MenuItem::Help => self.screen = Screen::Help,
            MenuItem::Quit => self.quit = true,
        }
    }

    fn start_run(&mut self, seed: u64) {
        self.save.last_seed = seed;
        self.save.save();
        self.world = Some(World::new(generate_campaign(seed)));
        self.run_filed = false;
        self.screen = Screen::Playing;
    }

    fn start_track(&mut self, track: Track) {
        self.world = Some(World::new(track));
        self.run_filed = false;
        self.screen = Screen::Playing;
    }

    fn start_custom(&mut self) {
        let Some(path) = save::playable_level() else {
            self.notify("no custom level found in levels/");
            return;
        };
        match save::read_level(path).and_then(|t| parse_pen(&t).map_err(|e| e.to_string())) {
            Ok(track) => {
                if track.warhead_x().is_none() {
                    self.notify("that level has no warhead — place one in the editor");
                    return;
                }
                self.start_track(track);
            }
            Err(e) => self.notify(e),
        }
    }

    // -----------------------------------------------------------------------
    // Playing
    // -----------------------------------------------------------------------

    fn update_playing(&mut self, input: &Frame, dt: f32) {
        if input.pause {
            self.screen = Screen::Paused;
            self.audio.stop_engine();
            self.audio.play(Sfx::UiConfirm);
            return;
        }

        let Some(world) = self.world.as_mut() else {
            self.screen = Screen::Menu;
            return;
        };
        world.update(input, dt, &mut self.audio);

        // Give the player a moment to watch the wreckage before the summary.
        let settled = match world.state {
            RunState::GameOver { timer } | RunState::Complete { timer } => timer > 2.0,
            _ => false,
        };
        if settled {
            if !self.run_filed {
                self.run_filed = true;
                self.save.record(world.score);
                self.save.save();
            }
            self.screen = Screen::RunOver;
        }
    }

    fn update_paused(&mut self, input: &Frame) {
        if input.pause || input.confirm {
            self.screen = Screen::Playing;
            self.audio.play(Sfx::UiConfirm);
        }
        if input.restart {
            // Abandoning counts as a finished run, so the score still stands.
            if let Some(w) = self.world.as_ref() {
                if !self.run_filed {
                    self.save.record(w.score);
                    self.save.save();
                }
            }
            self.world = None;
            self.audio.stop_engine();
            self.screen = Screen::Menu;
        }
    }

    fn update_run_over(&mut self, input: &Frame) {
        if input.confirm {
            let seed = self.save.last_seed;
            self.audio.play(Sfx::UiConfirm);
            if seed != 0 {
                self.start_run(seed);
            } else {
                self.start_run(fresh_seed());
            }
        } else if input.cancel {
            self.world = None;
            self.screen = Screen::Menu;
            self.audio.play(Sfx::UiConfirm);
        }
    }

    fn draw_playing(&self) {
        if let Some(w) = self.world.as_ref() {
            self.renderer.draw_world(w);
            hud::draw(&self.renderer, w);
        }
    }

    // -----------------------------------------------------------------------
    // Editor
    // -----------------------------------------------------------------------

    fn update_editor(&mut self, dt: f32) {
        let Some(editor) = self.editor.as_mut() else {
            self.screen = Screen::Menu;
            return;
        };
        match editor.update(&self.renderer, dt) {
            Exit::Stay => {}
            Exit::Menu => {
                self.screen = Screen::Menu;
            }
            Exit::Playtest(track) => {
                if track.warhead_x().is_none() {
                    editor.say("place a warhead before playtesting");
                } else {
                    self.start_track(*track);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Menu drawing
    // -----------------------------------------------------------------------

    fn draw_attract(&self, _dt: f32) {
        let palette = crate::render::blend_palette(&self.attract, self.attract_cam + VIRTUAL_W * 0.5);
        draw_rectangle(0.0, 0.0, VIRTUAL_W, VIRTUAL_H, palette.sky);
        self.renderer
            .draw_parallax(&self.attract, self.attract_cam, 0.0, &palette);
        self.renderer
            .draw_terrain(&self.attract, self.attract_cam, 0.0, &palette);
        hud::dim(0.55);
    }

    fn draw_menu(&self) {
        self.draw_attract(0.0);
        let r = &self.renderer;

        // Title, with a second copy offset behind it for a chromatic ghost.
        let title = "PENETRATOR";
        let ghost = (self.clock * 1.7).sin() * 0.8;
        r.text_centered(title, VIRTUAL_W * 0.5 + ghost, 60.0, 34.0, fade(theme::WARHEAD, 0.35));
        r.text_centered(title, VIRTUAL_W * 0.5 - ghost, 60.0, 34.0, fade(theme::SHIP, 0.35));
        r.text_centered(title, VIRTUAL_W * 0.5, 60.0, 34.0, theme::HUD_TEXT);
        r.text_centered(
            "R E D U X",
            VIRTUAL_W * 0.5,
            74.0,
            9.0,
            fade(theme::HUD_DIM, 0.9),
        );
        r.text_centered(
            "after the 1983 ZX Spectrum original",
            VIRTUAL_W * 0.5,
            86.0,
            6.0,
            fade(theme::HUD_DIM, 0.7),
        );

        let items = self.menu_items();
        let mut y = 116.0;
        for (i, item) in items.iter().enumerate() {
            let selected = i == self.menu_index.min(items.len() - 1);
            let color = if selected {
                theme::HUD_TEXT
            } else {
                fade(theme::HUD_DIM, 0.85)
            };
            if selected {
                let pulse = 0.55 + 0.45 * (self.clock * 5.0).sin();
                r.glow_path(
                    &[
                        Vec2::new(VIRTUAL_W * 0.5 - 96.0, y - 4.0),
                        Vec2::new(VIRTUAL_W * 0.5 - 90.0, y - 7.0),
                        Vec2::new(VIRTUAL_W * 0.5 - 90.0, y - 1.0),
                    ],
                    true,
                    0.7,
                    fade(theme::SHIP, pulse),
                );
            }
            r.text_centered(item.label(), VIRTUAL_W * 0.5, y, 10.0, color);
            if selected {
                r.text_centered(
                    item.blurb(),
                    VIRTUAL_W * 0.5,
                    y + 9.0,
                    6.0,
                    fade(theme::HUD_DIM, 0.8),
                );
            }
            y += if selected { 22.0 } else { 14.0 };
        }

        r.text(
            &format!("BEST {:07}", self.save.best()),
            8.0,
            VIRTUAL_H - 10.0,
            7.0,
            theme::HUD_DIM,
        );
        r.text_right(
            "UP/DOWN to choose    ENTER to commit",
            VIRTUAL_W - 8.0,
            VIRTUAL_H - 10.0,
            7.0,
            theme::HUD_DIM,
        );
    }

    fn draw_help(&self) {
        let r = &self.renderer;
        r.text_centered("BRIEFING", VIRTUAL_W * 0.5, 30.0, 18.0, theme::HUD_TEXT);

        r.text(
            "Fly the cave to the bunker, bomb the warhead, and fly back out.",
            18.0,
            48.0,
            7.0,
            theme::HUD_TEXT,
        );
        r.text(
            "The way home is faster, and they rebuild everything except the radar.",
            18.0,
            58.0,
            7.0,
            theme::HUD_DIM,
        );

        // Controls, left column.
        r.text("CONTROLS", 18.0, 78.0, 8.0, theme::HUD_GOOD);
        let mut y = 90.0;
        for (keys, what) in CONTROL_HELP {
            r.text(keys, 18.0, y, 6.2, theme::HUD_TEXT);
            r.text(what, 128.0, y, 6.2, theme::HUD_DIM);
            y += 9.0;
        }

        // Threats, right column.
        r.text("WHAT IS SHOOTING AT YOU", 252.0, 78.0, 8.0, theme::HUD_GOOD);
        let threats: [(SpawnKind, &str); 6] = [
            (SpawnKind::Radar, "Guides every SAM. Kill these first."),
            (SpawnKind::Silo, "Launches SAMs. Two cannon hits."),
            (SpawnKind::Turret, "Ceiling gun. Leads its shots."),
            (SpawnKind::Mine, "Drifts. Does not shoot. Still fatal."),
            (SpawnKind::Drone, "Scrambles on the way home."),
            (SpawnKind::Warhead, "The target. Bombs only."),
        ];
        let mut y = 90.0;
        for (kind, note) in threats {
            let color = theme::enemy_color(kind);
            r.glow_dot(Vec2::new(256.0, y - 2.0), 2.0, color);
            r.text(kind.label(), 264.0, y, 6.2, color);
            r.text(note, 264.0, y + 7.0, 5.6, theme::HUD_DIM);
            y += 17.0;
        }

        r.text_centered(
            "Red missiles are tracking you. Orange ones are not.",
            VIRTUAL_W * 0.5,
            VIRTUAL_H - 26.0,
            7.0,
            theme::HUD_WARN,
        );
        r.text_centered(
            "ENTER or ESC to go back",
            VIRTUAL_W * 0.5,
            VIRTUAL_H - 12.0,
            6.5,
            theme::HUD_DIM,
        );
    }

    fn draw_notice(&self) {
        let Some((message, t)) = self.notice.as_ref() else {
            return;
        };
        let alpha = (t / 4.0).clamp(0.0, 1.0);
        let w = self.renderer.text_width(message, 7.0) + 16.0;
        let x = (VIRTUAL_W - w) * 0.5;
        let y = VIRTUAL_H - 46.0;
        draw_rectangle(x, y - 10.0, w, 16.0, fade(Color::new(0.1, 0.02, 0.02, 0.9), alpha));
        draw_rectangle_lines(x, y - 10.0, w, 16.0, 0.8, fade(theme::HUD_WARN, alpha));
        self.renderer
            .text_centered(message, VIRTUAL_W * 0.5, y, 7.0, fade(theme::HUD_TEXT, alpha));
    }
}

/// A seed from the wall clock.
///
/// `macroquad::rand` is deliberately not used anywhere in this game; the only
/// entropy the whole codebase needs is this one number, and everything after it
/// is reproducible from it.
pub fn fresh_seed() -> u64 {
    let now = macroquad::miniquad::date::now();
    // Microseconds since the epoch, mixed so that adjacent launches do not
    // produce adjacent-looking caves.
    let micros = (now * 1_000_000.0) as u64;
    micros
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .rotate_left(31)
        .wrapping_add(micros)
}
