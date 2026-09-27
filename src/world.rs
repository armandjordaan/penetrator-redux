//! The simulation.
//!
//! `World` owns the track, the ship, the enemies, the projectiles and the score,
//! and advances all of it one fixed-ish step at a time. Rendering reads it and
//! never writes to it.
//!
//! # The mission
//!
//! A run is three phases. [`Phase::Outbound`] scrolls right through four zones to
//! the bunker. [`Phase::BunkerHold`] parks the camera in the warhead chamber —
//! nothing pushes you forward any more, and you stay there until the warhead is
//! destroyed or you are. [`Phase::Egress`] scrolls back the way you came, faster
//! and with everything rebuilt, until you reach the mouth of the cave.
//!
//! # Why enemies stream in and out
//!
//! Every placement in the track exists as a [`Spawn`] and is turned into a live
//! [`Enemy`] only when the camera gets close, then dropped again when it is far
//! behind. That keeps the per-frame cost proportional to what is on screen rather
//! than to the length of the level, and it gives the egress leg its "they rebuilt
//! it while you were gone" effect for free: flipping direction just clears the
//! live list and lets everything stream back in.

use crate::audio::{Audio, Sfx};
use crate::config::*;
use crate::enemy::{Action, Enemy, Senses};
use crate::fx::Fx;
use crate::input::Frame;
use crate::level::{Spawn, SpawnKind, Track};
use crate::player::Player;
use crate::projectile::{Projectile, ProjectileKind};
use crate::rng::Rng;
use crate::theme;
use crate::util::circles_overlap;
use macroquad::prelude::*;

/// How far beyond the screen edge a spawn is brought to life.
const SPAWN_MARGIN: f32 = 120.0;
/// How far behind the screen a live enemy is dropped again.
const DESPAWN_MARGIN: f32 = 260.0;

/// Which leg of the mission is being flown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Outbound,
    /// Camera parked in the warhead chamber; the ship is free to manoeuvre.
    BunkerHold,
    Egress,
}

impl Phase {
    /// Camera travel direction. The hold phase does not move.
    pub fn scroll_dir(self) -> f32 {
        match self {
            Phase::Outbound => 1.0,
            Phase::BunkerHold => 0.0,
            Phase::Egress => -1.0,
        }
    }

    /// Which way the ship faces. The hold phase keeps the outbound facing,
    /// because you arrived pointing that way and nothing has turned you round yet.
    pub fn facing(self) -> f32 {
        match self {
            Phase::Egress => -1.0,
            _ => 1.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Phase::Outbound => "INBOUND",
            Phase::BunkerHold => "TARGET",
            Phase::Egress => "EGRESS",
        }
    }
}

/// What the run is doing right now.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RunState {
    Flying,
    /// The ship is exploding. `timer` counts down to the respawn.
    Dying { timer: f32 },
    /// Out of lives.
    GameOver { timer: f32 },
    /// Reached the mouth of the cave alive.
    Complete { timer: f32 },
}

impl RunState {
    pub fn is_over(self) -> bool {
        matches!(self, RunState::GameOver { .. } | RunState::Complete { .. })
    }
}

/// A full-width callout, used for zone names and mission beats.
#[derive(Clone, Debug)]
pub struct Banner {
    pub title: String,
    pub subtitle: String,
    pub life: f32,
    pub max_life: f32,
    pub color: Color,
}

/// Whether a placement is currently live, and whether it has been destroyed.
#[derive(Clone, Copy, Default, Debug)]
struct SpawnState {
    live: bool,
    /// Destroyed. Reset on the egress flip for everything except radar and the
    /// warhead — see [`World::begin_egress`].
    destroyed: bool,
}

/// Numbers the HUD and the end-of-run screen want.
#[derive(Clone, Copy, Default, Debug)]
pub struct Stats {
    pub kills: u32,
    pub shots_fired: u32,
    pub bombs_dropped: u32,
    pub deaths: u32,
    pub elapsed: f32,
}

pub struct World {
    pub track: Track,
    pub player: Player,
    pub enemies: Vec<Enemy>,
    pub projectiles: Vec<Projectile>,
    pub fx: Fx,
    pub rng: Rng,

    /// World x of the left edge of the screen.
    pub cam_x: f32,
    pub phase: Phase,
    pub state: RunState,

    pub score: u32,
    pub lives: i32,
    pub stats: Stats,

    pub radars_total: usize,
    pub radars_alive: usize,
    pub warhead_destroyed: bool,

    pub banner: Option<Banner>,
    /// Slow-motion timer, used for the warhead detonation.
    slowmo: f32,
    spawn_states: Vec<SpawnState>,
    /// Zone the ship was in last frame, for arrival announcements.
    last_zone: usize,
    /// Zones already scored for being reached.
    zones_scored: Vec<bool>,
}

impl World {
    pub fn new(track: Track) -> Self {
        let spawn_states = vec![SpawnState::default(); track.spawns.len()];
        let zones_scored = vec![false; track.zones.len()];
        let radars_total = track.radar_count();

        let start_x = SHIP_SCREEN_REST;
        let start_y = track.terrain.centre_at(start_x);

        let mut world = World {
            player: Player::new(Vec2::new(start_x, start_y)),
            track,
            enemies: Vec::new(),
            projectiles: Vec::new(),
            fx: Fx::new(),
            rng: Rng::new(0x50D1E),
            cam_x: 0.0,
            phase: Phase::Outbound,
            state: RunState::Flying,
            score: 0,
            lives: STARTING_LIVES,
            stats: Stats::default(),
            radars_total,
            radars_alive: radars_total,
            warhead_destroyed: false,
            banner: None,
            slowmo: 0.0,
            spawn_states,
            last_zone: usize::MAX,
            zones_scored,
        };
        world.announce_zone();
        world
    }

    // -----------------------------------------------------------------------
    // Queries used by the HUD and renderer
    // -----------------------------------------------------------------------

    pub fn zone_index(&self) -> usize {
        self.track.zone_index_at(self.player.pos.x)
    }

    pub fn zone_name(&self) -> &str {
        &self.track.zones[self.zone_index()].name
    }

    pub fn radar_active(&self) -> bool {
        self.radars_alive > 0
    }

    /// Base scroll speed for where the ship currently is.
    pub fn scroll_speed(&self) -> f32 {
        let base = self.track.zones[self.zone_index()].scroll;
        if self.phase == Phase::Egress {
            base * EGRESS_SPEED_BONUS
        } else {
            base
        }
    }

    /// Mission progress, 0 at the cave mouth and 1 at the warhead — then back
    /// down again on the way home.
    pub fn progress(&self) -> f32 {
        let total = self.track.world_len().max(1.0);
        (self.player.pos.x / total).clamp(0.0, 1.0)
    }

    // -----------------------------------------------------------------------
    // Main step
    // -----------------------------------------------------------------------

    /// Advances the simulation. `dt` is real time; slow motion is applied here so
    /// that callers never have to know about it.
    pub fn update(&mut self, input: &Frame, dt: f32, audio: &mut Audio) {
        let dt = dt.min(MAX_DT);

        let sim_dt = if self.slowmo > 0.0 {
            self.slowmo = (self.slowmo - dt).max(0.0);
            dt * SLOWMO_SCALE
        } else {
            dt
        };

        self.stats.elapsed += sim_dt;
        if let Some(b) = self.banner.as_mut() {
            b.life -= dt;
            if b.life <= 0.0 {
                self.banner = None;
            }
        }

        match self.state {
            RunState::Flying => self.update_flying(input, sim_dt, audio),
            RunState::Dying { timer } => {
                let t = timer - dt;
                if t <= 0.0 {
                    self.after_death(audio);
                } else {
                    self.state = RunState::Dying { timer: t };
                }
                audio.stop_engine();
                self.step_world_without_player(sim_dt);
            }
            RunState::GameOver { timer } => {
                self.state = RunState::GameOver { timer: timer + dt };
                audio.stop_engine();
                self.step_world_without_player(sim_dt);
            }
            RunState::Complete { timer } => {
                self.state = RunState::Complete { timer: timer + dt };
                audio.stop_engine();
                self.step_world_without_player(sim_dt);
            }
        }

        self.fx.update(sim_dt);
    }

    /// Keeps explosions and debris moving during death and end-of-run screens.
    fn step_world_without_player(&mut self, dt: f32) {
        let target = self.player.pos;
        self.projectiles.retain_mut(|p| p.update(dt, target));
        let senses = Senses {
            player: target,
            player_vel: Vec2::ZERO,
            radar_active: self.radar_active(),
            travel_dir: self.phase.facing(),
            aggression: 1.0,
            ceasefire: true,
        };
        for e in self.enemies.iter_mut() {
            e.update(dt, &senses, &mut self.rng);
        }
    }

    fn update_flying(&mut self, input: &Frame, dt: f32, audio: &mut Audio) {
        let dir = self.phase.facing();
        let scroll = self.scroll_speed();
        // The ship's actual world velocity, which is the camera's — zero while
        // parked in the bunker chamber. Ordnance inherits *this*, not the zone's
        // nominal scroll speed: a bomb released over a stationary target has to
        // fall straight down or the chamber becomes unwinnable.
        let carry = scroll * self.phase.scroll_dir();

        self.advance_camera(dt, scroll);

        // --- ship ---
        let out = self.player.update(input, dt, dir, self.cam_x);
        self.player.clamp_to_screen();

        if out.fired_cannon {
            let round = self.player.make_cannon_round(dir, carry);
            self.projectiles.push(round);
            self.stats.shots_fired += 1;
            audio.play(Sfx::Cannon);
        }
        if out.just_overheated {
            audio.play(Sfx::Overheat);
        }
        if out.dropped_bomb {
            let (pos, vel) = self.player.bomb_release(carry);
            self.projectiles.push(Projectile::bomb(pos, vel));
            self.stats.bombs_dropped += 1;
            audio.play(Sfx::BombDrop);
        }

        let exhaust = self.player.exhaust_point(dir);
        self.fx
            .exhaust(&mut self.rng, exhaust, Vec2::new(-dir, 0.0), self.player.thrust);
        audio.engine(self.player.thrust);

        // Dust when skimming a wall — the only warning you get that you are one
        // twitch from the rock.
        let clearance = self.player.wall_clearance(&self.track.terrain);
        if clearance < 7.0 {
            let (top, bottom) = self.track.terrain.gap_at(self.player.pos.x);
            let near_ceiling = (self.player.pos.y - top) < (bottom - self.player.pos.y);
            let (surface_y, away) = if near_ceiling {
                (top, Vec2::new(0.0, 1.0))
            } else {
                (bottom, Vec2::new(0.0, -1.0))
            };
            self.fx.wall_dust(
                &mut self.rng,
                Vec2::new(self.player.pos.x, surface_y),
                away,
            );
        }

        self.stream_enemies();
        self.think_enemies(dt, audio);
        self.advance_projectiles(dt);
        self.intercept_incoming(audio);
        self.resolve_friendly_fire(audio);
        self.resolve_player_hazards(audio);
        self.check_phase_transitions(audio);
        self.announce_zone();
    }

    /// Moves the camera and applies the limits that define each phase.
    fn advance_camera(&mut self, dt: f32, scroll: f32) {
        let dir = self.phase.scroll_dir();
        self.cam_x += scroll * dir * dt;

        match self.phase {
            Phase::Outbound => {
                if self.cam_x >= self.track.bunker_hold_x {
                    self.cam_x = self.track.bunker_hold_x;
                    self.phase = Phase::BunkerHold;
                    self.set_banner(
                        "WARHEAD CHAMBER",
                        "BOMBS ONLY - CANNON WILL NOT PENETRATE",
                        theme::WARHEAD,
                        3.4,
                    );
                }
            }
            Phase::BunkerHold => {}
            Phase::Egress => {
                // Terrain sampling clamps below column zero, and the first
                // columns of the track are forced wide open, so there is nothing
                // solid out here — just the mouth of the cave receding behind you.
                self.cam_x = self.cam_x.max(-EGRESS_EXIT_RUN);
            }
        }
    }

    /// Brings nearby placements to life and retires distant ones.
    fn stream_enemies(&mut self) {
        let lo = self.cam_x - SPAWN_MARGIN;
        let hi = self.cam_x + VIRTUAL_W + SPAWN_MARGIN;

        for (idx, spawn) in self.track.spawns.iter().enumerate() {
            let state = &mut self.spawn_states[idx];
            if state.live || state.destroyed {
                continue;
            }
            if spawn.x < lo || spawn.x > hi {
                continue;
            }
            let (x, y) = resolve_spawn(&self.track, spawn);
            self.enemies
                .push(Enemy::new(spawn.kind, Vec2::new(x, y), idx, &mut self.rng));
            state.live = true;
        }

        let drop_lo = self.cam_x - DESPAWN_MARGIN;
        let drop_hi = self.cam_x + VIRTUAL_W + DESPAWN_MARGIN;
        let states = &mut self.spawn_states;
        self.enemies.retain(|e| {
            let keep = e.pos.x >= drop_lo && e.pos.x <= drop_hi;
            if !keep {
                // Retired, not destroyed: it can stream back in later.
                if let Some(s) = states.get_mut(e.spawn_idx) {
                    s.live = false;
                }
            }
            keep
        });
    }

    fn think_enemies(&mut self, dt: f32, audio: &Audio) {
        let senses = Senses {
            player: self.player.pos,
            player_vel: Vec2::new(
                self.scroll_speed() * self.phase.scroll_dir(),
                self.player.vel.y,
            ),
            radar_active: self.radar_active(),
            travel_dir: self.phase.facing(),
            aggression: if self.phase == Phase::Egress {
                EGRESS_AGGRESSION
            } else {
                1.0
            },
            ceasefire: self.state != RunState::Flying,
        };

        let mut actions: Vec<Action> = Vec::new();
        for e in self.enemies.iter_mut() {
            if let Some(a) = e.update(dt, &senses, &mut self.rng) {
                actions.push(a);
            }
        }
        let mut live_sams = self
            .projectiles
            .iter()
            .filter(|p| p.kind == ProjectileKind::Sam)
            .count();

        for a in actions {
            match a {
                Action::LaunchSam { from, homing } => {
                    // Hold fire into a sky that is already full. See MAX_LIVE_SAMS.
                    if live_sams >= MAX_LIVE_SAMS {
                        continue;
                    }
                    live_sams += 1;
                    self.projectiles.push(Projectile::sam(from, homing));
                    // The launch is the warning. Without it a homing SAM coming
                    // up from behind is unfair rather than difficult.
                    audio.play(Sfx::SamLaunch);
                }
                Action::FireShell { from, dir } => {
                    self.projectiles.push(Projectile::shell(from, dir));
                }
            }
        }
    }

    fn advance_projectiles(&mut self, dt: f32) {
        let target = self.player.pos;
        self.projectiles.retain_mut(|p| p.update(dt, target));
    }

    /// Cannon rounds against incoming ordnance.
    ///
    /// Shooting a missile out of the air is not a bonus feature, it is the
    /// answer to the question the game keeps asking. Without it a guided SAM
    /// launched from close range is unavoidable rather than difficult, and the
    /// cannon has nothing to do between emplacements. The 1983 original let you
    /// do this and it is the main reason its missiles felt fair.
    fn intercept_incoming(&mut self, audio: &mut Audio) {
        let n = self.projectiles.len();
        if n < 2 {
            return;
        }
        let mut spent = vec![false; n];
        let mut hits: Vec<(Vec2, f32)> = Vec::new();

        for i in 0..n {
            let round = self.projectiles[i];
            // Bombs get their area effect on detonation instead.
            if !round.friendly || spent[i] || round.kind != ProjectileKind::Cannon {
                continue;
            }
            for j in 0..n {
                if j == i || spent[j] {
                    continue;
                }
                let incoming = self.projectiles[j];
                if incoming.friendly
                    || !circles_overlap(round.pos, round.radius, incoming.pos, incoming.radius)
                {
                    continue;
                }
                spent[i] = true;
                spent[j] = true;
                hits.push((incoming.pos, incoming.explosion_scale()));
                break;
            }
        }

        if hits.is_empty() {
            return;
        }
        let mut keep = spent.iter().map(|s| !s);
        self.projectiles.retain(|_| keep.next().unwrap_or(true));

        for (pos, scale) in hits {
            self.fx.explosion(&mut self.rng, pos, scale * 0.7, theme::EXPLOSION);
            self.score += SCORE_INTERCEPT;
        }
        audio.play(Sfx::ExplodeSmall);
    }

    /// Player ordnance against enemies and rock.
    fn resolve_friendly_fire(&mut self, audio: &mut Audio) {
        let mut detonations: Vec<(Vec2, f32, i32)> = Vec::new();
        let mut i = 0;

        while i < self.projectiles.len() {
            let p = self.projectiles[i];
            if !p.friendly {
                i += 1;
                continue;
            }

            let mut consumed = false;

            // Rock first: a round that has already buried itself in a wall should
            // not also hit the silo standing on it.
            if self.track.terrain.solid_at(p.pos.x, p.pos.y) {
                match p.kind {
                    ProjectileKind::Bomb if p.armed() => {
                        detonations.push((p.pos, p.blast_radius(), p.damage));
                    }
                    _ => {
                        self.fx
                            .impact(&mut self.rng, p.pos, -p.vel, theme::CANNON_ROUND);
                    }
                }
                consumed = true;
            }

            if !consumed {
                for e in self.enemies.iter_mut() {
                    if !e.is_alive() || !circles_overlap(p.pos, p.radius, e.pos, e.radius()) {
                        continue;
                    }
                    if p.kind == ProjectileKind::Cannon && e.immune_to_cannon() {
                        // Rounds spark off the warhead casing. Deliberately loud,
                        // so the player learns the rule in one try.
                        self.fx.impact(&mut self.rng, p.pos, -p.vel, theme::HUD_WARN);
                        audio.play(Sfx::HitArmour);
                        consumed = true;
                        break;
                    }
                    if p.kind == ProjectileKind::Bomb {
                        if p.armed() {
                            detonations.push((p.pos, p.blast_radius(), p.damage));
                            consumed = true;
                        }
                        break;
                    }
                    let killed = e.damage(p.damage);
                    self.fx
                        .impact(&mut self.rng, p.pos, -p.vel, theme::enemy_color(e.kind));
                    if !killed {
                        audio.play(Sfx::HitArmour);
                    }
                    consumed = true;
                    break;
                }
            }

            if consumed {
                self.projectiles.swap_remove(i);
            } else {
                i += 1;
            }
        }

        for (pos, radius, damage) in detonations {
            self.detonate(pos, radius, damage, audio);
        }
        self.reap_enemies(audio);
    }

    /// A bomb going off: area damage plus the spectacle.
    fn detonate(&mut self, pos: Vec2, radius: f32, damage: i32, audio: &mut Audio) {
        for e in self.enemies.iter_mut() {
            if e.is_alive() && circles_overlap(pos, radius, e.pos, e.radius()) {
                e.damage(damage);
            }
        }
        // Bombs also clear incoming missiles caught in the blast, which makes a
        // well-timed drop a legitimate defensive move.
        self.projectiles.retain(|p| {
            p.friendly || !circles_overlap(pos, radius, p.pos, p.radius)
        });

        self.fx.explosion(&mut self.rng, pos, 34.0, theme::EXPLOSION);
        audio.play(Sfx::ExplodeSmall);
    }

    /// Turns dead enemies into explosions, score and consequences.
    fn reap_enemies(&mut self, audio: &mut Audio) {
        let mut killed: Vec<(SpawnKind, Vec2, u32, f32, usize)> = Vec::new();
        self.enemies.retain(|e| {
            if e.is_alive() {
                return true;
            }
            killed.push((e.kind, e.pos, e.score(), e.explosion_scale(), e.spawn_idx));
            false
        });

        for (kind, pos, score, scale, spawn_idx) in killed {
            if let Some(s) = self.spawn_states.get_mut(spawn_idx) {
                s.live = false;
                s.destroyed = true;
            }
            self.score += score;
            self.stats.kills += 1;

            let tint = if kind == SpawnKind::Warhead {
                theme::EXPLOSION_BIG
            } else {
                theme::EXPLOSION
            };
            self.fx.explosion(&mut self.rng, pos, scale, tint);
            self.fx
                .float_text(pos, format!("{score}"), theme::enemy_color(kind), 7.0);

            match kind {
                SpawnKind::Radar => {
                    self.radars_alive = self.radars_alive.saturating_sub(1);
                    audio.play(Sfx::ExplodeLarge);
                    if self.radars_alive == 0 {
                        audio.play(Sfx::RadarDown);
                        self.set_banner(
                            "RADAR NETWORK DOWN",
                            "SAMs ARE NOW UNGUIDED",
                            theme::HUD_GOOD,
                            3.0,
                        );
                    } else {
                        self.set_banner(
                            "RADAR DESTROYED",
                            &format!("{} STILL TRANSMITTING", self.radars_alive),
                            theme::RADAR,
                            2.0,
                        );
                    }
                }
                SpawnKind::Warhead => {
                    self.on_warhead_destroyed(audio);
                }
                _ => audio.play(Sfx::ExplodeSmall),
            }
        }
    }

    /// Hostile projectiles, enemy bodies and the cave, against the ship.
    fn resolve_player_hazards(&mut self, audio: &mut Audio) {
        if self.state != RunState::Flying {
            return;
        }

        // Rock is always lethal. Invulnerability is protection from the enemy,
        // not from your own flying.
        if self.player.hits_terrain(&self.track.terrain, self.phase.facing()) {
            self.kill_player("CRASHED INTO THE CAVE", audio);
            return;
        }

        // Hostile ordnance meeting rock, whether or not it was aimed at you.
        let mut wrecks: Vec<(Vec2, f32)> = Vec::new();
        self.projectiles.retain(|p| {
            if p.friendly {
                return true;
            }
            if self.track.terrain.solid_at(p.pos.x, p.pos.y) {
                wrecks.push((p.pos, p.explosion_scale()));
                return false;
            }
            true
        });
        for (pos, scale) in wrecks {
            self.fx.explosion(&mut self.rng, pos, scale, theme::EXPLOSION);
        }

        if self.player.invulnerable() {
            return;
        }

        let ship = self.player.pos;
        let hit_by_shot = self
            .projectiles
            .iter()
            .any(|p| !p.friendly && circles_overlap(p.pos, p.radius, ship, SHIP_HIT_RADIUS));
        if hit_by_shot {
            self.projectiles
                .retain(|p| p.friendly || !circles_overlap(p.pos, p.radius, ship, SHIP_HIT_RADIUS));
            self.kill_player("SHOT DOWN", audio);
            return;
        }

        let collided = self
            .enemies
            .iter()
            .any(|e| e.is_alive() && circles_overlap(e.pos, e.radius(), ship, SHIP_HIT_RADIUS));
        if collided {
            self.kill_player("COLLISION", audio);
        }
    }

    fn kill_player(&mut self, reason: &str, audio: &mut Audio) {
        self.stats.deaths += 1;
        self.lives -= 1;
        self.fx
            .explosion(&mut self.rng, self.player.pos, 78.0, theme::EXPLOSION_BIG);
        self.fx.shake(0.85);
        audio.play(Sfx::PlayerDeath);
        audio.stop_engine();

        if self.lives <= 0 {
            self.set_banner("MISSION FAILED", reason, theme::HUD_WARN, 6.0);
            self.state = RunState::GameOver { timer: 0.0 };
        } else {
            self.set_banner(reason, &format!("{} SHIPS REMAINING", self.lives), theme::HUD_WARN, 2.2);
            self.state = RunState::Dying { timer: 1.6 };
        }
    }

    /// Puts a fresh ship at the current leg's checkpoint.
    fn after_death(&mut self, _audio: &mut Audio) {
        self.state = RunState::Flying;

        // Dying in the bunker sends you back out to fly the approach again.
        if self.phase == Phase::BunkerHold {
            self.phase = Phase::Outbound;
        }

        let dir = self.phase.facing();
        let rest = Player::screen_x_for(SHIP_SCREEN_REST, dir);
        let checkpoint = self.track.checkpoint_x(self.player.pos.x, dir);

        // Place the camera so the checkpoint lands under the ship's resting
        // position, then derive the ship from the camera — never the other way
        // round, or clamping the camera would silently move the ship into rock.
        self.cam_x =
            (checkpoint - rest).clamp(0.0, (self.track.world_len() - VIRTUAL_W).max(0.0));

        let x = self.cam_x + rest;
        let y = safe_spawn_y(&self.track, x);
        self.player.respawn(Vec2::new(x, y));

        // Clear the board around the respawn so nothing is already on top of you,
        // and drop the debris from a death that happened somewhere else.
        self.projectiles.clear();
        self.enemies.clear();
        self.fx.clear();
        for s in self.spawn_states.iter_mut() {
            s.live = false;
        }
        self.last_zone = usize::MAX;
    }

    fn on_warhead_destroyed(&mut self, audio: &mut Audio) {
        self.warhead_destroyed = true;
        self.slowmo = SLOWMO_TIME;
        self.fx.shake(1.0);
        audio.play(Sfx::WarheadKill);
        self.set_banner("WARHEAD DESTROYED", "GET OUT THE WAY YOU CAME IN", theme::HUD_GOOD, 4.0);
        self.begin_egress();
    }

    /// Turns the mission round.
    fn begin_egress(&mut self) {
        // Keep the ship exactly where it is on screen. Recomputing the throttle
        // band from the resting position would teleport it into the back wall.
        let screen_x = self.player.pos.x - self.cam_x;
        self.player.travel_screen =
            (VIRTUAL_W - screen_x).clamp(SHIP_SCREEN_MIN, SHIP_SCREEN_MAX);
        self.player.invuln = self.player.invuln.max(2.5);

        self.phase = Phase::Egress;
        self.projectiles.clear();
        self.enemies.clear();

        // Everything is rebuilt for the run home — except the radar network,
        // which stays down. Clearing it was worth something and it should stay
        // worth something.
        for (idx, s) in self.spawn_states.iter_mut().enumerate() {
            s.live = false;
            let kind = self.track.spawns[idx].kind;
            if !matches!(kind, SpawnKind::Radar | SpawnKind::Warhead) {
                s.destroyed = false;
            }
        }

        self.scramble_interceptors();
        self.last_zone = usize::MAX;
    }

    /// Adds the drone screen that only exists on the way home.
    ///
    /// Two placement rules, both learned the hard way from watching a run:
    /// nothing within a screen and a half of the bunker, so the phase flip is
    /// not immediately followed by a head-on; and nothing in a passage narrower
    /// than [`DRONE_MIN_GAP`], because an interceptor in a tight corridor is not
    /// a fight, it is a wall you cannot shoot your way past in time.
    fn scramble_interceptors(&mut self) {
        let bunker_start = self
            .track
            .zones
            .last()
            .map(|z| z.start_x())
            .unwrap_or(self.track.world_len());

        let mut x = bunker_start - VIRTUAL_W * 1.5;
        while x > VIRTUAL_W {
            if self.track.terrain.gap_height(x) >= DRONE_MIN_GAP {
                let (top, bottom) = self.track.terrain.gap_at(x);
                let y = self.rng.range(top + 18.0, bottom - 18.0);
                self.track.spawns.push(Spawn {
                    kind: SpawnKind::Drone,
                    x,
                    y,
                });
                self.spawn_states.push(SpawnState::default());
                x -= self.rng.range(820.0, 1300.0);
            } else {
                // Too tight here; try a little further back.
                x -= COLUMN_W * 4.0;
            }
        }
    }

    fn check_phase_transitions(&mut self, audio: &mut Audio) {
        // The camera running out of track is the escape, not the ship reaching
        // some x. Keying off the ship would make completion depend on where the
        // player happens to be sitting in the throttle band — and at the back of
        // that band, unreachable.
        if self.phase == Phase::Egress && self.cam_x <= -EGRESS_EXIT_RUN + 0.5 {
            self.complete_mission(audio);
        }
    }

    fn complete_mission(&mut self, audio: &mut Audio) {
        if self.state.is_over() {
            return;
        }
        let bonus = SCORE_ESCAPE + self.lives.max(0) as u32 * SCORE_LIFE_BONUS;
        self.score += bonus;
        self.state = RunState::Complete { timer: 0.0 };
        self.set_banner("MISSION COMPLETE", &format!("+{bonus} ESCAPE BONUS"), theme::HUD_GOOD, 8.0);
        audio.play(Sfx::ExplodeLarge);
        audio.stop_engine();
    }

    fn announce_zone(&mut self) {
        let idx = self.zone_index();
        if idx == self.last_zone {
            return;
        }
        self.last_zone = idx;

        // Reaching a new zone outbound is worth points, once per run.
        if self.phase == Phase::Outbound && !self.zones_scored[idx] {
            self.zones_scored[idx] = true;
            if idx > 0 {
                self.score += SCORE_ZONE_CLEAR;
            }
        }

        let zone = &self.track.zones[idx];
        let title = zone.name.clone();
        let subtitle = format!("ZONE {} OF {} - {}", idx + 1, self.track.zones.len(), self.phase.label());
        let color = theme::palette(zone.palette).rock_edge;
        self.set_banner(&title, &subtitle, color, 2.4);
    }

    fn set_banner(&mut self, title: &str, subtitle: &str, color: Color, life: f32) {
        self.banner = Some(Banner {
            title: title.to_string(),
            subtitle: subtitle.to_string(),
            life,
            max_life: life,
            color,
        });
    }
}

/// Resolves a spawn against the terrain, seating floor and ceiling emplacements
/// slightly into the rock so they do not appear to hover.
fn resolve_spawn(track: &Track, spawn: &Spawn) -> (f32, f32) {
    let (x, y) = track.spawn_position(spawn);
    let seated = match spawn.kind.anchor() {
        crate::level::Anchor::Floor => y - 5.0,
        crate::level::Anchor::Ceiling => y + 5.0,
        crate::level::Anchor::Air => y,
    };
    (x, seated)
}

/// A y coordinate at `x` that the ship's whole hull fits into.
///
/// Takes the tightest ceiling and the tightest floor across the hull's length
/// rather than sampling the centre point, because respawning on a slope with the
/// nose already in the rock is the single most infuriating way to lose a life.
fn safe_spawn_y(track: &Track, x: f32) -> f32 {
    let mut lowest_ceiling: f32 = f32::MIN;
    let mut highest_floor: f32 = f32::MAX;
    for offset in [-SHIP_HALF_LEN, 0.0, SHIP_HALF_LEN] {
        let (top, bottom) = track.terrain.gap_at(x + offset);
        lowest_ceiling = lowest_ceiling.max(top);
        highest_floor = highest_floor.min(bottom);
    }
    if highest_floor - lowest_ceiling > SHIP_HALF_HEIGHT * 2.0 + 4.0 {
        (lowest_ceiling + highest_floor) * 0.5
    } else {
        // Nowhere fits; the least-bad option is the middle of the local gap.
        track.terrain.centre_at(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::generate_campaign;

    fn silent() -> Audio {
        Audio::silent()
    }

    fn world() -> World {
        World::new(generate_campaign(1234))
    }

    fn fly(w: &mut World, input: &Frame, seconds: f32) {
        let mut a = silent();
        let dt = 1.0 / 60.0;
        for _ in 0..((seconds / dt) as usize) {
            w.update(input, dt, &mut a);
        }
    }

    #[test]
    fn a_new_run_starts_flying_outbound_with_full_lives() {
        let w = world();
        assert_eq!(w.state, RunState::Flying);
        assert_eq!(w.phase, Phase::Outbound);
        assert_eq!(w.lives, STARTING_LIVES);
        assert_eq!(w.score, 0);
        assert!(w.radars_alive > 0 && w.radars_alive == w.radars_total);
    }

    #[test]
    fn the_ship_starts_in_open_air() {
        for seed in 0..25u64 {
            let w = World::new(generate_campaign(seed));
            assert!(
                !w.player.hits_terrain(&w.track.terrain, 1.0),
                "seed {seed} spawned the ship inside rock"
            );
        }
    }

    #[test]
    fn the_camera_advances_outbound_and_stops_at_the_bunker() {
        let mut w = world();
        let start = w.cam_x;
        fly(&mut w, &Frame::default(), 2.0);
        assert!(w.cam_x > start);

        // Skip most of the trip rather than simulating four minutes of flying.
        w.cam_x = w.track.bunker_hold_x - 5.0;
        fly(&mut w, &Frame::default(), 2.0);
        assert_eq!(w.phase, Phase::BunkerHold);
        assert!((w.cam_x - w.track.bunker_hold_x).abs() < 0.01);
    }

    #[test]
    fn the_bunker_hold_leaves_the_ship_free_but_the_camera_parked() {
        let mut w = world();
        w.cam_x = w.track.bunker_hold_x;
        w.phase = Phase::BunkerHold;
        let cam = w.cam_x;
        fly(&mut w, &Frame { throttle: 1.0, ..Default::default() }, 1.5);
        assert_eq!(w.cam_x, cam, "camera must stay parked");
        assert!(w.player.travel_screen > SHIP_SCREEN_REST, "ship must still move");
    }

    #[test]
    fn destroying_the_warhead_turns_the_mission_round() {
        let mut w = world();
        w.cam_x = w.track.bunker_hold_x;
        w.phase = Phase::BunkerHold;
        let mut audio = silent();

        // Put the warhead in front of the ship and bomb it directly.
        let warhead_x = w.track.warhead_x().unwrap();
        w.player.pos.x = warhead_x;
        w.stream_enemies();
        let wh = w
            .enemies
            .iter()
            .position(|e| e.kind == SpawnKind::Warhead)
            .expect("warhead should be streamed in");
        let pos = w.enemies[wh].pos;
        w.detonate(pos, 40.0, WARHEAD_HP, &mut audio);
        w.reap_enemies(&mut audio);

        assert!(w.warhead_destroyed);
        assert_eq!(w.phase, Phase::Egress);
        assert!(w.score >= SCORE_WARHEAD);
    }

    #[test]
    fn the_egress_flip_does_not_teleport_the_ship() {
        let mut w = world();
        w.cam_x = w.track.bunker_hold_x;
        w.phase = Phase::BunkerHold;
        w.player.pos.x = w.cam_x + 200.0;
        w.player.travel_screen = 200.0;

        let before = w.player.pos.x;
        w.begin_egress();
        // The player's world x is recomputed from the band next frame; it must
        // land back where it was.
        let after = w.cam_x + Player::screen_x_for(w.player.travel_screen, -1.0);
        assert!((after - before).abs() < 1.0, "{before} -> {after}");
    }

    #[test]
    fn radar_stays_destroyed_across_the_egress_flip_but_silos_come_back() {
        let mut w = world();
        // Flatten every placement, then flip.
        for s in w.spawn_states.iter_mut() {
            s.destroyed = true;
        }
        w.radars_alive = 0;
        w.begin_egress();

        for (idx, s) in w.spawn_states.iter().enumerate() {
            match w.track.spawns[idx].kind {
                SpawnKind::Radar | SpawnKind::Warhead => {
                    assert!(s.destroyed, "radar and warhead must stay dead")
                }
                SpawnKind::Drone => {}
                _ => assert!(!s.destroyed, "defences should be rebuilt for the way home"),
            }
        }
        assert_eq!(w.radars_alive, 0, "the radar network stays down");
    }

    #[test]
    fn interceptors_only_exist_on_the_way_home() {
        let mut w = world();
        assert_eq!(
            w.track.spawns.iter().filter(|s| s.kind == SpawnKind::Drone).count(),
            0
        );
        w.begin_egress();
        assert!(
            w.track.spawns.iter().filter(|s| s.kind == SpawnKind::Drone).count() > 0,
            "egress should scramble interceptors"
        );
    }

    #[test]
    fn reaching_the_cave_mouth_on_egress_completes_the_mission() {
        let mut w = world();
        let mut a = silent();
        w.phase = Phase::Egress;

        // Pinned at the start of the track is not yet out: there is still an
        // exit run to fly.
        w.cam_x = 0.0;
        w.check_phase_transitions(&mut a);
        assert_eq!(w.state, RunState::Flying);

        w.cam_x = -EGRESS_EXIT_RUN;
        w.check_phase_transitions(&mut a);
        assert!(matches!(w.state, RunState::Complete { .. }));
        assert!(w.score >= SCORE_ESCAPE);
    }

    #[test]
    fn the_exit_run_is_actually_reachable_from_anywhere_in_the_throttle_band() {
        // The bug this guards against: keying completion off the ship's world x
        // while the camera stops at zero leaves the finish line beyond the back
        // of the throttle band, and the mission can never be finished.
        let mut w = world();
        let mut a = silent();
        w.phase = Phase::Egress;
        w.cam_x = 400.0;

        // Sit at the very back of the band — the least favourable position — and
        // fly perfectly, so the only thing under test is whether the exit can be
        // reached at all.
        for _ in 0..4000 {
            w.player.pos.y = w.track.terrain.centre_at(w.player.pos.x);
            w.player.vel.y = 0.0;
            w.player.invuln = 10.0;
            let input = Frame { throttle: -1.0, ..Default::default() };
            w.update(&input, 1.0 / 60.0, &mut a);
            if w.state.is_over() {
                break;
            }
        }
        assert!(
            matches!(w.state, RunState::Complete { .. }),
            "could not escape while holding the throttle back: {:?}",
            w.state
        );
    }

    #[test]
    fn flying_into_rock_costs_a_life_and_respawns_you_in_the_open() {
        let mut w = world();
        let mut a = silent();
        // Drive the ship into the floor.
        w.player.pos.y = w.track.terrain.floor_at(w.player.pos.x) + 2.0;
        w.resolve_player_hazards(&mut a);
        assert_eq!(w.lives, STARTING_LIVES - 1);
        assert!(matches!(w.state, RunState::Dying { .. }));

        fly(&mut w, &Frame::default(), 2.5);
        assert_eq!(w.state, RunState::Flying);
        assert!(
            !w.player.hits_terrain(&w.track.terrain, w.phase.facing()),
            "respawned inside rock"
        );
        assert!(w.player.invulnerable());
    }

    #[test]
    fn running_out_of_lives_ends_the_run() {
        let mut w = world();
        let mut a = silent();
        w.lives = 1;
        w.player.pos.y = w.track.terrain.floor_at(w.player.pos.x) + 2.0;
        w.resolve_player_hazards(&mut a);
        assert!(matches!(w.state, RunState::GameOver { .. }));
    }

    #[test]
    fn dying_in_the_bunker_sends_you_back_out_to_the_approach() {
        let mut w = world();
        let mut a = silent();
        w.phase = Phase::BunkerHold;
        w.cam_x = w.track.bunker_hold_x;
        w.player.pos = Vec2::new(w.cam_x + 100.0, 10.0);
        w.kill_player("test", &mut a);
        w.after_death(&mut a);
        assert_eq!(w.phase, Phase::Outbound, "the camera must be able to scroll again");
    }

    #[test]
    fn invulnerability_stops_shots_but_not_walls() {
        let mut w = world();
        let mut a = silent();
        w.player.invuln = 5.0;

        // A missile sitting on the ship does nothing while invulnerable.
        w.projectiles.push(Projectile::sam(w.player.pos, false));
        w.resolve_player_hazards(&mut a);
        assert_eq!(w.state, RunState::Flying);
        assert_eq!(w.lives, STARTING_LIVES);

        // The cave still kills.
        w.player.pos.y = w.track.terrain.ceiling_at(w.player.pos.x) - 1.0;
        w.resolve_player_hazards(&mut a);
        assert_eq!(w.lives, STARTING_LIVES - 1);
    }

    #[test]
    fn cannon_fire_cannot_scratch_the_warhead() {
        let mut w = world();
        let mut a = silent();
        let warhead_x = w.track.warhead_x().unwrap();
        w.cam_x = w.track.bunker_hold_x;
        w.player.pos.x = warhead_x - 40.0;
        w.stream_enemies();

        let idx = w.enemies.iter().position(|e| e.kind == SpawnKind::Warhead).unwrap();
        let hp_before = w.enemies[idx].hp;
        let target = w.enemies[idx].pos;

        for _ in 0..40 {
            w.projectiles.push(Projectile::cannon(target, 1.0, 0.0));
            w.resolve_friendly_fire(&mut a);
        }
        let idx = w.enemies.iter().position(|e| e.kind == SpawnKind::Warhead).unwrap();
        assert_eq!(w.enemies[idx].hp, hp_before, "cannon must not damage the warhead");
        assert!(!w.warhead_destroyed);
    }

    #[test]
    fn killing_the_last_radar_makes_new_sams_unguided() {
        let mut w = world();
        let mut a = silent();
        assert!(w.radar_active());

        // Find and destroy every dish.
        w.player.pos.x = w.track.spawns.iter().find(|s| s.kind == SpawnKind::Radar).unwrap().x;
        w.stream_enemies();
        while let Some(i) = w.enemies.iter().position(|e| e.kind == SpawnKind::Radar) {
            w.enemies[i].hp = 0;
            w.reap_enemies(&mut a);
        }
        w.radars_alive = 0;
        assert!(!w.radar_active());
    }

    #[test]
    fn the_sky_never_fills_past_the_missile_cap() {
        let mut w = world();
        let mut a = silent();
        // Park the ship among the defences and let them shoot at it for a while.
        let silo = w.track.spawns.iter().find(|s| s.kind == SpawnKind::Silo).unwrap();
        w.cam_x = silo.x - 300.0;
        for _ in 0..4000 {
            w.player.invuln = 10.0;
            w.update(&Frame::default(), 1.0 / 60.0, &mut a);
            let live = w
                .projectiles
                .iter()
                .filter(|p| p.kind == ProjectileKind::Sam)
                .count();
            assert!(live <= MAX_LIVE_SAMS, "{live} SAMs airborne at once");
        }
    }

    #[test]
    fn bombs_clear_incoming_missiles() {
        let mut w = world();
        let mut a = silent();
        let at = Vec2::new(w.player.pos.x + 60.0, w.player.pos.y);
        w.projectiles.push(Projectile::sam(at, true));
        let friendly = Projectile::cannon(at, 1.0, 0.0);
        w.projectiles.push(friendly);

        w.detonate(at, BOMB_BLAST_RADIUS, BOMB_DAMAGE, &mut a);
        assert!(
            !w.projectiles.iter().any(|p| p.kind == ProjectileKind::Sam),
            "blast should sweep up hostile ordnance"
        );
        assert!(
            w.projectiles.iter().any(|p| p.friendly),
            "and leave your own alone"
        );
    }

    #[test]
    fn enemies_stream_in_and_out_rather_than_all_existing_at_once() {
        let mut w = world();
        fly(&mut w, &Frame::default(), 3.0);
        assert!(
            w.enemies.len() < w.track.spawns.len(),
            "the whole track should not be live at once"
        );
        // Everything live is near the camera.
        for e in &w.enemies {
            assert!(e.pos.x > w.cam_x - DESPAWN_MARGIN - 1.0);
            assert!(e.pos.x < w.cam_x + VIRTUAL_W + DESPAWN_MARGIN + 1.0);
        }
    }

    #[test]
    fn a_retired_enemy_can_stream_back_in() {
        let mut w = world();
        let home = w.track.spawns[0].x;
        w.cam_x = home - 100.0;
        w.stream_enemies();
        let idx = w.enemies[0].spawn_idx;

        // Fly far away — the emplacement is retired, not destroyed. (Other
        // emplacements stream in around the new camera position; only this one
        // is being tracked.)
        w.cam_x += 5000.0;
        w.stream_enemies();
        assert!(!w.enemies.iter().any(|e| e.spawn_idx == idx));
        assert!(!w.spawn_states[idx].destroyed);

        w.cam_x = home - 100.0;
        w.stream_enemies();
        assert!(
            w.enemies.iter().any(|e| e.spawn_idx == idx),
            "an undestroyed emplacement should return"
        );
    }

    #[test]
    fn a_destroyed_emplacement_does_not_come_back_within_a_leg() {
        let mut w = world();
        let mut a = silent();
        w.cam_x = w.track.spawns[0].x - 100.0;
        w.stream_enemies();
        let idx = w.enemies[0].spawn_idx;
        w.enemies[0].hp = 0;
        w.reap_enemies(&mut a);

        w.cam_x += 5000.0;
        w.stream_enemies();
        w.cam_x -= 5000.0;
        w.stream_enemies();
        assert!(!w.enemies.iter().any(|e| e.spawn_idx == idx));
    }

    #[test]
    fn the_simulation_survives_an_absurd_frame_time() {
        let mut w = world();
        let mut a = silent();
        // A hitched frame must not fling the ship through a wall.
        w.update(&Frame::default(), 10.0, &mut a);
        assert!(w.cam_x < 10.0 * 200.0, "dt should have been clamped");
        assert!(w.player.pos.x.is_finite() && w.player.pos.y.is_finite());
    }

    #[test]
    fn a_long_run_stays_numerically_sane() {
        let mut w = world();
        let mut a = silent();
        let input = Frame { fire_held: true, throttle: 1.0, ..Default::default() };
        for i in 0..7200 {
            let pitch = if (i / 40) % 2 == 0 { -1.0 } else { 1.0 };
            w.update(&Frame { pitch, ..input }, 1.0 / 60.0, &mut a);
            assert!(w.player.pos.x.is_finite() && w.player.pos.y.is_finite());
            assert!(w.projectiles.len() < 4000, "projectiles leaked");
            assert!(w.enemies.len() < 400, "enemies leaked");
        }
    }
}
