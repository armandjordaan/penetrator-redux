//! The ship.
//!
//! Two things about the flight model are worth knowing before reading the code.
//!
//! First, the ship does not control its own x. The camera travels at the zone's
//! scroll speed and the throttle slides the ship forward and back *within the
//! screen*, between [`SHIP_SCREEN_MIN`] and [`SHIP_SCREEN_MAX`]. This is how the
//! 1983 original worked and it is still the right answer: it guarantees the
//! player can always see what is coming, and it makes "how far ahead do I sit"
//! a real risk/reward decision rather than a speed run.
//!
//! Second, that band is measured *along the direction of travel*, not in screen
//! space. On the way home the ship flies left, and pushing the throttle forward
//! still means "further into the unknown". Everything that has a handedness in
//! this module takes a `dir` of +1 or -1 and the rest of the game never has to
//! think about which way round it is.

use crate::config::*;
use crate::input::Frame;
use crate::projectile::Projectile;
use crate::terrain::Terrain;
use crate::util::{approach, damp};
use macroquad::prelude::*;

/// What the ship did this frame that the world needs to act on.
#[derive(Clone, Copy, Default, Debug)]
pub struct Output {
    pub fired_cannon: bool,
    pub dropped_bomb: bool,
    /// True on the frame the cannon locks out, so the world can play the alarm
    /// exactly once instead of every frame it stays hot.
    pub just_overheated: bool,
}

pub struct Player {
    pub pos: Vec2,
    pub vel: Vec2,
    /// Position within the throttle band, measured along the travel direction.
    pub travel_screen: f32,
    /// Cannon heat, 0..1. At 1.0 the gun locks out until it cools to
    /// [`HEAT_RESUME`].
    pub heat: f32,
    pub overheated: bool,
    pub bombs: i32,
    gun_cd: f32,
    bomb_cd: f32,
    bomb_regen: f32,
    /// Seconds of post-respawn immunity remaining.
    pub invuln: f32,
    /// Visual roll, follows vertical velocity.
    pub tilt: f32,
    /// 0..1, drives exhaust and engine noise.
    pub thrust: f32,
}

impl Player {
    pub fn new(pos: Vec2) -> Self {
        Player {
            pos,
            vel: Vec2::ZERO,
            travel_screen: SHIP_SCREEN_REST,
            heat: 0.0,
            overheated: false,
            bombs: BOMB_MAX,
            gun_cd: 0.0,
            bomb_cd: 0.0,
            bomb_regen: 0.0,
            invuln: RESPAWN_INVULN,
            tilt: 0.0,
            thrust: 0.5,
        }
    }

    /// Puts the ship back at a checkpoint. Bombs and heat are restored: a
    /// respawn should hand you a working aircraft.
    pub fn respawn(&mut self, pos: Vec2) {
        self.pos = pos;
        self.vel = Vec2::ZERO;
        self.travel_screen = SHIP_SCREEN_REST;
        self.heat = 0.0;
        self.overheated = false;
        self.bombs = BOMB_MAX;
        self.gun_cd = 0.0;
        self.bomb_cd = 0.0;
        self.bomb_regen = 0.0;
        self.invuln = RESPAWN_INVULN;
        self.tilt = 0.0;
    }

    pub fn invulnerable(&self) -> bool {
        self.invuln > 0.0
    }

    /// Converts a position within the travel band into a screen x.
    pub fn screen_x_for(travel_screen: f32, dir: f32) -> f32 {
        if dir >= 0.0 {
            travel_screen
        } else {
            VIRTUAL_W - travel_screen
        }
    }

    pub fn screen_x(&self, dir: f32) -> f32 {
        Self::screen_x_for(self.travel_screen, dir)
    }

    /// Advances the ship. `dir` is +1 outbound and -1 on the way home; `cam_x` is
    /// the world x of the left edge of the screen, which together with the
    /// throttle band determines where the ship actually is.
    pub fn update(&mut self, input: &Frame, dt: f32, dir: f32, cam_x: f32) -> Output {
        let mut out = Output::default();

        self.invuln = (self.invuln - dt).max(0.0);
        self.gun_cd = (self.gun_cd - dt).max(0.0);
        self.bomb_cd = (self.bomb_cd - dt).max(0.0);

        // --- vertical ---
        if input.pitch != 0.0 {
            self.vel.y += input.pitch * SHIP_ACCEL_Y * dt;
        } else {
            self.vel.y = damp(self.vel.y, SHIP_DRAG_Y, dt);
        }
        self.vel.y = self.vel.y.clamp(-SHIP_MAX_VY, SHIP_MAX_VY);
        self.pos.y += self.vel.y * dt;

        // --- throttle ---
        if input.throttle != 0.0 {
            self.travel_screen += input.throttle * THROTTLE_SPEED * dt;
        } else {
            // A gentle pull back to the resting position. Slow enough that you
            // can hold a forward posture by feel, firm enough that letting go
            // eventually returns you to a safe viewing distance.
            self.travel_screen =
                approach(self.travel_screen, SHIP_SCREEN_REST, THROTTLE_RECENTER * dt);
        }
        self.travel_screen = self.travel_screen.clamp(SHIP_SCREEN_MIN, SHIP_SCREEN_MAX);
        self.pos.x = cam_x + self.screen_x(dir);

        // --- cosmetics ---
        self.tilt = approach(
            self.tilt,
            (self.vel.y / SHIP_MAX_VY) * 0.42,
            6.0 * dt,
        );
        self.thrust = approach(self.thrust, 0.45 + 0.55 * input.throttle.max(0.0), 3.0 * dt);

        // --- heat ---
        let was_overheated = self.overheated;
        self.heat = (self.heat - HEAT_COOL * dt).max(0.0);
        if self.overheated && self.heat <= HEAT_RESUME {
            self.overheated = false;
        }

        // --- weapons ---
        if input.fire_held && !self.overheated && self.gun_cd <= 0.0 {
            self.gun_cd = CANNON_COOLDOWN;
            self.heat += HEAT_PER_SHOT;
            out.fired_cannon = true;
            if self.heat >= 1.0 {
                self.heat = 1.0;
                self.overheated = true;
            }
        }
        out.just_overheated = self.overheated && !was_overheated;

        self.bomb_regen += dt;
        while self.bomb_regen >= BOMB_REGEN && self.bombs < BOMB_MAX {
            self.bomb_regen -= BOMB_REGEN;
            self.bombs += 1;
        }
        if self.bombs >= BOMB_MAX {
            self.bomb_regen = 0.0;
        }

        if input.bomb_pressed && self.bomb_cd <= 0.0 && self.bombs > 0 {
            self.bomb_cd = BOMB_COOLDOWN;
            self.bombs -= 1;
            out.dropped_bomb = true;
        }

        out
    }

    /// Where a cannon round leaves the ship.
    pub fn muzzle(&self, dir: f32) -> Vec2 {
        self.pos + Vec2::new(SHIP_HALF_LEN * dir, 0.0)
    }

    /// Where a bomb leaves the ship, and with what velocity.
    ///
    /// `carry` is the ship's own horizontal world velocity. A bomb keeps it, so
    /// it lands where the ship looked like it was going to put it — and when the
    /// ship is not moving horizontally at all, it falls straight down.
    pub fn bomb_release(&self, carry: f32) -> (Vec2, Vec2) {
        (
            self.pos + Vec2::new(0.0, SHIP_HALF_HEIGHT),
            Vec2::new(carry, self.vel.y * 0.35 + 30.0),
        )
    }

    /// Where the exhaust comes out.
    pub fn exhaust_point(&self, dir: f32) -> Vec2 {
        self.pos - Vec2::new(SHIP_HALF_LEN * dir, 0.0)
    }

    /// The points tested against the cave.
    ///
    /// Five samples along the hull rather than one at the centre: a nose-first
    /// clip into a stalactite should kill you at the moment the nose touches it,
    /// which is what the player sees, not half a ship's length later.
    pub fn hull_points(&self, dir: f32) -> [Vec2; 5] {
        let nose = SHIP_HALF_LEN * dir;
        [
            self.pos + Vec2::new(nose, 0.0),
            self.pos + Vec2::new(nose * 0.35, -SHIP_HALF_HEIGHT),
            self.pos + Vec2::new(nose * 0.35, SHIP_HALF_HEIGHT),
            self.pos + Vec2::new(-nose * 0.85, -SHIP_HALF_HEIGHT * 0.6),
            self.pos + Vec2::new(-nose * 0.85, SHIP_HALF_HEIGHT * 0.6),
        ]
    }

    /// True if any part of the hull is inside rock.
    pub fn hits_terrain(&self, terrain: &Terrain, dir: f32) -> bool {
        self.hull_points(dir)
            .iter()
            .any(|p| terrain.solid_at(p.x, p.y))
    }

    /// Distance from the nearest cave surface, used for the "flying too close"
    /// dust and for the proximity readout on the HUD.
    pub fn wall_clearance(&self, terrain: &Terrain) -> f32 {
        let (top, bottom) = terrain.gap_at(self.pos.x);
        ((self.pos.y - SHIP_HALF_HEIGHT) - top).min(bottom - (self.pos.y + SHIP_HALF_HEIGHT))
    }

    /// Keeps the ship on screen vertically. Called after the world knows the
    /// terrain, so it never fights the cave for control of the ship.
    pub fn clamp_to_screen(&mut self) {
        let lo = HUD_H + SHIP_HALF_HEIGHT;
        let hi = VIRTUAL_H - SHIP_HALF_HEIGHT;
        if self.pos.y < lo {
            self.pos.y = lo;
            self.vel.y = self.vel.y.max(0.0);
        } else if self.pos.y > hi {
            self.pos.y = hi;
            self.vel.y = self.vel.y.min(0.0);
        }
    }

    /// Builds the cannon round for this frame's shot. Rounds pick up half the
    /// ship's own speed, which keeps their reach roughly constant relative to the
    /// ship rather than shrinking as the cave gets faster.
    pub fn make_cannon_round(&self, dir: f32, carry: f32) -> Projectile {
        Projectile::cannon(self.muzzle(dir), dir, carry * 0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level_flight() -> Frame {
        Frame::default()
    }

    fn step(p: &mut Player, input: &Frame, seconds: f32, dir: f32) -> Output {
        let mut last = Output::default();
        let dt = 1.0 / 120.0;
        let steps = (seconds / dt) as usize;
        for _ in 0..steps {
            last = p.update(input, dt, dir, 0.0);
        }
        last
    }

    #[test]
    fn climbing_and_diving_are_symmetric() {
        let mut up = Player::new(Vec2::new(0.0, 135.0));
        let mut down = Player::new(Vec2::new(0.0, 135.0));
        step(&mut up, &Frame { pitch: -1.0, ..level_flight() }, 0.5, 1.0);
        step(&mut down, &Frame { pitch: 1.0, ..level_flight() }, 0.5, 1.0);
        assert!(up.pos.y < 135.0 && down.pos.y > 135.0);
        assert!(((135.0 - up.pos.y) - (down.pos.y - 135.0)).abs() < 0.5);
    }

    #[test]
    fn vertical_speed_is_capped() {
        let mut p = Player::new(Vec2::new(0.0, 135.0));
        step(&mut p, &Frame { pitch: 1.0, ..level_flight() }, 10.0, 1.0);
        assert!(p.vel.y <= SHIP_MAX_VY + 0.01);
    }

    #[test]
    fn the_throttle_band_is_respected_in_both_directions() {
        for dir in [1.0f32, -1.0] {
            let mut p = Player::new(Vec2::ZERO);
            step(&mut p, &Frame { throttle: 1.0, ..level_flight() }, 20.0, dir);
            assert!(p.travel_screen <= SHIP_SCREEN_MAX + 0.01);
            step(&mut p, &Frame { throttle: -1.0, ..level_flight() }, 20.0, dir);
            assert!(p.travel_screen >= SHIP_SCREEN_MIN - 0.01);
        }
    }

    #[test]
    fn throttling_forward_moves_the_right_way_on_screen_for_each_leg() {
        let mut out = Player::new(Vec2::ZERO);
        step(&mut out, &Frame { throttle: 1.0, ..level_flight() }, 1.0, 1.0);
        assert!(out.screen_x(1.0) > SHIP_SCREEN_REST, "outbound: forward is right");

        let mut home = Player::new(Vec2::ZERO);
        step(&mut home, &Frame { throttle: 1.0, ..level_flight() }, 1.0, -1.0);
        assert!(
            home.screen_x(-1.0) < VIRTUAL_W - SHIP_SCREEN_REST,
            "egress: forward is left"
        );
    }

    #[test]
    fn releasing_the_throttle_drifts_back_to_the_rest_position() {
        let mut p = Player::new(Vec2::ZERO);
        step(&mut p, &Frame { throttle: 1.0, ..level_flight() }, 4.0, 1.0);
        assert!(p.travel_screen > SHIP_SCREEN_REST);
        step(&mut p, &level_flight(), 30.0, 1.0);
        assert!((p.travel_screen - SHIP_SCREEN_REST).abs() < 0.5);
    }

    #[test]
    fn holding_the_trigger_overheats_and_then_recovers() {
        let mut p = Player::new(Vec2::ZERO);
        let firing = Frame { fire_held: true, ..level_flight() };
        let mut shots = 0;
        for _ in 0..1200 {
            if p.update(&firing, 1.0 / 120.0, 1.0, 0.0).fired_cannon {
                shots += 1;
            }
        }
        assert!(p.overheated, "sustained fire should overheat");
        assert!(shots > 0);

        // Cooling off restores the gun.
        step(&mut p, &level_flight(), 4.0, 1.0);
        assert!(!p.overheated);
        assert!(p.update(&firing, 1.0 / 120.0, 1.0, 0.0).fired_cannon);
    }

    #[test]
    fn overheat_is_announced_once_per_lockout_not_once_per_frame() {
        let mut p = Player::new(Vec2::ZERO);
        let firing = Frame { fire_held: true, ..level_flight() };

        // With the current tuning the gun gains heat at about 0.37/s net of
        // cooling, so it locks out around 2.7s and is not back under the resume
        // threshold until roughly 4.2s. A four-second window therefore contains
        // exactly one lockout — and would contain many if the flag latched per
        // frame instead of per transition.
        let announcements = (0..480)
            .filter(|_| p.update(&firing, 1.0 / 120.0, 1.0, 0.0).just_overheated)
            .count();
        assert_eq!(announcements, 1, "the alarm must not repeat while locked out");
        assert!(p.overheated);
    }

    #[test]
    fn bombs_are_spent_and_regenerate() {
        let mut p = Player::new(Vec2::ZERO);
        let drop = Frame { bomb_pressed: true, ..level_flight() };
        // The cooldown means one drop per press-and-wait.
        for _ in 0..BOMB_MAX {
            p.update(&drop, 1.0 / 120.0, 1.0, 0.0);
            step(&mut p, &level_flight(), BOMB_COOLDOWN * 1.1, 1.0);
        }
        // Regen during those waits tops some back up; the point is that dropping
        // costs and waiting refills.
        let after_dropping = p.bombs;
        assert!(after_dropping < BOMB_MAX);
        step(&mut p, &level_flight(), BOMB_REGEN * BOMB_MAX as f32 * 1.2, 1.0);
        assert_eq!(p.bombs, BOMB_MAX, "bombs should cap at the maximum");
    }

    #[test]
    fn an_empty_bomb_bay_drops_nothing() {
        let mut p = Player::new(Vec2::ZERO);
        p.bombs = 0;
        let drop = Frame { bomb_pressed: true, ..level_flight() };
        assert!(!p.update(&drop, 1.0 / 120.0, 1.0, 0.0).dropped_bomb);
    }

    #[test]
    fn respawning_restores_a_working_aircraft() {
        let mut p = Player::new(Vec2::ZERO);
        p.bombs = 0;
        p.heat = 1.0;
        p.overheated = true;
        p.vel = Vec2::new(50.0, 50.0);
        p.respawn(Vec2::new(10.0, 20.0));
        assert_eq!(p.bombs, BOMB_MAX);
        assert!(!p.overheated);
        assert_eq!(p.vel, Vec2::ZERO);
        assert!(p.invulnerable());
    }

    #[test]
    fn the_hull_extends_ahead_of_the_ship_in_the_travel_direction() {
        let p = Player::new(Vec2::new(100.0, 100.0));
        let out = p.hull_points(1.0);
        assert!(out.iter().any(|q| q.x > 100.0 + SHIP_HALF_LEN - 0.01));
        let home = p.hull_points(-1.0);
        assert!(home.iter().any(|q| q.x < 100.0 - SHIP_HALF_LEN + 0.01));
    }

    #[test]
    fn the_ship_cannot_leave_the_playfield() {
        let mut p = Player::new(Vec2::new(0.0, 0.0));
        p.vel.y = -500.0;
        p.clamp_to_screen();
        assert!(p.pos.y >= HUD_H);
        assert!(p.vel.y >= 0.0, "clamping should kill the velocity into the wall");

        p.pos.y = VIRTUAL_H + 50.0;
        p.vel.y = 500.0;
        p.clamp_to_screen();
        assert!(p.pos.y <= VIRTUAL_H);
        assert!(p.vel.y <= 0.0);
    }

    #[test]
    fn terrain_collision_uses_the_whole_hull() {
        let terrain = Terrain {
            // A cave that is open on the left and pinched on the right.
            ceiling: vec![20.0, 20.0, 20.0, 130.0],
            floor: vec![250.0, 250.0, 250.0, 250.0],
        };
        let mut p = Player::new(Vec2::new(COLUMN_W * 3.0 - SHIP_HALF_LEN + 1.0, 100.0));
        assert!(p.hits_terrain(&terrain, 1.0), "nose should clip the overhang");
        // Facing the other way, the same position is clear.
        p.pos.x = COLUMN_W * 1.5;
        assert!(!p.hits_terrain(&terrain, 1.0));
    }
}
