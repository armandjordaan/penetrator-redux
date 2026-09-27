//! Everything in flight that is not a ship: cannon rounds, bombs, SAMs and
//! turret shells.
//!
//! One struct covers all four. They differ in how they move and what they hit,
//! not in what they are made of, and a single flat `Vec<Projectile>` is far
//! easier to reason about (and to iterate for collision) than four parallel
//! lists.

use crate::config::*;
use crate::util::{lerp, turn_toward};
use macroquad::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProjectileKind {
    /// Player cannon. Fast, straight, cheap, stopped by rock.
    Cannon,
    /// Player bomb. Falls under gravity and detonates with a blast radius.
    Bomb,
    /// Surface-to-air missile from a silo. Homes only while a radar stands.
    Sam,
    /// Turret shell. Aimed once at launch, then ballistic.
    Shell,
}

#[derive(Clone, Copy, Debug)]
pub struct Projectile {
    pub kind: ProjectileKind,
    pub pos: Vec2,
    pub vel: Vec2,
    /// Seconds left before it fizzles out on its own.
    pub life: f32,
    /// True for player ordnance. Decides who it can hurt.
    pub friendly: bool,
    pub radius: f32,
    pub damage: i32,
    /// Facing, for drawing. Kept separate from velocity so a homing missile can
    /// point where it is turning to rather than where it is going.
    pub angle: f32,
    /// SAMs only: whether this missile got a radar lock at launch.
    pub homing: bool,
    /// SAMs only: seconds of guidance left before the motor burns out.
    pub guidance: f32,
    /// Bombs only: a brief arming delay so one cannot detonate on the ship that
    /// dropped it.
    pub arm_timer: f32,
    /// Accumulator for emitting trail particles at a fixed rate.
    pub trail_timer: f32,
}

impl Projectile {
    pub fn cannon(pos: Vec2, dir: f32, inherited_vx: f32) -> Self {
        let vel = Vec2::new(CANNON_SPEED * dir + inherited_vx, 0.0);
        Projectile {
            kind: ProjectileKind::Cannon,
            pos,
            vel,
            life: CANNON_RANGE / CANNON_SPEED,
            friendly: true,
            radius: 1.6,
            damage: CANNON_DAMAGE,
            angle: if dir >= 0.0 { 0.0 } else { std::f32::consts::PI },
            homing: false,
            guidance: 0.0,
            arm_timer: 0.0,
            trail_timer: 0.0,
        }
    }

    pub fn bomb(pos: Vec2, vel: Vec2) -> Self {
        Projectile {
            kind: ProjectileKind::Bomb,
            pos,
            vel,
            life: 6.0,
            friendly: true,
            radius: 2.4,
            damage: BOMB_DAMAGE,
            angle: std::f32::consts::FRAC_PI_2,
            homing: false,
            guidance: 0.0,
            // Long enough that a bomb dropped while skimming the floor still
            // clears the ship, short enough to be invisible in play.
            arm_timer: 0.06,
            trail_timer: 0.0,
        }
    }

    pub fn sam(pos: Vec2, homing: bool) -> Self {
        let speed = if homing { SAM_HOMING_SPEED } else { SAM_SPEED };
        Projectile {
            kind: ProjectileKind::Sam,
            pos,
            // Straight up out of the silo; homing kicks in once it is clear.
            vel: Vec2::new(0.0, -speed),
            life: SAM_LIFETIME,
            friendly: false,
            radius: SAM_RADIUS,
            damage: 1,
            angle: -std::f32::consts::FRAC_PI_2,
            homing,
            guidance: if homing { SAM_GUIDANCE_TIME } else { 0.0 },
            arm_timer: 0.0,
            trail_timer: 0.0,
        }
    }

    pub fn shell(pos: Vec2, dir: Vec2) -> Self {
        let v = dir.normalize_or_zero() * TURRET_SHELL_SPEED;
        Projectile {
            kind: ProjectileKind::Shell,
            pos,
            vel: v,
            life: 4.0,
            friendly: false,
            radius: 2.2,
            damage: 1,
            angle: v.y.atan2(v.x),
            homing: false,
            guidance: 0.0,
            arm_timer: 0.0,
            trail_timer: 0.0,
        }
    }

    /// Advances one step. `target` is the ship's position, used only by homing
    /// SAMs. Returns `false` when the projectile has expired.
    pub fn update(&mut self, dt: f32, target: Vec2) -> bool {
        self.life -= dt;
        self.arm_timer = (self.arm_timer - dt).max(0.0);
        self.trail_timer += dt;

        match self.kind {
            ProjectileKind::Cannon | ProjectileKind::Shell => {}
            ProjectileKind::Bomb => {
                self.vel.y += BOMB_GRAVITY * dt;
                self.angle = self.vel.y.atan2(self.vel.x);
            }
            ProjectileKind::Sam => {
                self.guidance = (self.guidance - dt).max(0.0);
                if self.homing && self.guidance <= 0.0 {
                    // Motor burnout. It keeps its heading and its speed; it just
                    // stops caring where you went.
                    self.homing = false;
                }

                // Boost phase: it leaves the tube slowly and accelerates. This is
                // the player's warning.
                let age = SAM_LIFETIME - self.life;
                let cruise = if self.homing {
                    SAM_HOMING_SPEED
                } else {
                    SAM_SPEED
                };
                let speed = lerp(
                    SAM_LAUNCH_SPEED,
                    cruise,
                    (age / SAM_BOOST_TIME).clamp(0.0, 1.0),
                );

                let heading = if self.homing {
                    // A finite turn rate is what makes homing missiles beatable:
                    // fly straight at one and it cannot come round in time.
                    let to_target = target - self.pos;
                    let desired = to_target.y.atan2(to_target.x);
                    let current = self.vel.y.atan2(self.vel.x);
                    turn_toward(current, desired, SAM_TURN_RATE * dt)
                } else {
                    self.vel.y.atan2(self.vel.x)
                };
                self.vel = Vec2::new(heading.cos(), heading.sin()) * speed;
                self.angle = heading;
            }
        }

        self.pos += self.vel * dt;
        self.life > 0.0
    }

    /// True once a bomb may detonate.
    pub fn armed(&self) -> bool {
        self.arm_timer <= 0.0
    }

    /// Blast radius on detonation. Only bombs have area effect; everything else
    /// is a point hit.
    pub fn blast_radius(&self) -> f32 {
        match self.kind {
            ProjectileKind::Bomb => BOMB_BLAST_RADIUS,
            _ => 0.0,
        }
    }

    /// How big an explosion this makes when it dies against something.
    pub fn explosion_scale(&self) -> f32 {
        match self.kind {
            ProjectileKind::Cannon => 0.0,
            ProjectileKind::Bomb => 34.0,
            ProjectileKind::Sam => 16.0,
            ProjectileKind::Shell => 12.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOWHERE: Vec2 = Vec2::new(10_000.0, 10_000.0);

    #[test]
    fn cannon_rounds_expire_at_their_stated_range() {
        let mut p = Projectile::cannon(Vec2::ZERO, 1.0, 0.0);
        let dt = 1.0 / 240.0;
        let mut alive = true;
        while alive {
            alive = p.update(dt, NOWHERE);
        }
        // Travelled distance should match CANNON_RANGE within a frame's worth.
        let travelled = p.pos.x;
        assert!(
            (travelled - CANNON_RANGE).abs() < CANNON_SPEED * dt * 2.0,
            "travelled {travelled}, expected about {CANNON_RANGE}"
        );
    }

    #[test]
    fn cannon_inherits_ship_speed_but_still_goes_the_right_way() {
        let forward = Projectile::cannon(Vec2::ZERO, 1.0, 60.0);
        assert!(forward.vel.x > CANNON_SPEED);
        let backward = Projectile::cannon(Vec2::ZERO, -1.0, -60.0);
        assert!(backward.vel.x < -CANNON_SPEED);
    }

    #[test]
    fn bombs_arm_shortly_after_release() {
        let mut b = Projectile::bomb(Vec2::ZERO, Vec2::ZERO);
        assert!(!b.armed(), "a bomb must not be live in the bomb bay");
        b.update(0.1, NOWHERE);
        assert!(b.armed());
    }

    #[test]
    fn bombs_fall() {
        let mut b = Projectile::bomb(Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0));
        for _ in 0..30 {
            b.update(1.0 / 60.0, NOWHERE);
        }
        assert!(b.pos.y > 0.0, "bomb did not fall");
        assert!(b.pos.x > 0.0, "bomb lost its forward throw");
    }

    #[test]
    fn a_homing_sam_closes_on_its_target() {
        let target = Vec2::new(300.0, 60.0);
        let mut m = Projectile::sam(Vec2::new(300.0, 240.0), true);
        let start = m.pos.distance(target);
        for _ in 0..60 {
            m.update(1.0 / 60.0, target);
        }
        assert!(m.pos.distance(target) < start, "homing SAM is not closing");
    }

    #[test]
    fn a_homing_sam_turns_no_faster_than_its_limit() {
        let mut m = Projectile::sam(Vec2::new(0.0, 200.0), true);
        let dt = 1.0 / 60.0;
        // Put the target behind it, forcing the hardest possible turn.
        let target = Vec2::new(0.0, 400.0);
        let before = m.vel.y.atan2(m.vel.x);
        m.update(dt, target);
        let after = m.vel.y.atan2(m.vel.x);
        let turned = crate::util::wrap_angle(after - before).abs();
        assert!(
            turned <= SAM_TURN_RATE * dt + 1e-3,
            "turned {turned} rad in one step, limit is {}",
            SAM_TURN_RATE * dt
        );
    }

    #[test]
    fn a_sam_leaves_the_tube_slowly_and_then_accelerates() {
        let mut m = Projectile::sam(Vec2::new(0.0, 200.0), false);
        m.update(1.0 / 120.0, NOWHERE);
        let launch = m.vel.length();
        assert!(
            launch < SAM_SPEED * 0.6,
            "a missile must not appear at full speed: {launch}"
        );
        for _ in 0..120 {
            m.update(1.0 / 120.0, NOWHERE);
        }
        assert!((m.vel.length() - SAM_SPEED).abs() < 1.0, "it should reach cruise");
    }

    #[test]
    fn a_dumb_sam_flies_straight_up_and_is_dodgeable() {
        let mut m = Projectile::sam(Vec2::new(100.0, 200.0), false);
        let target = Vec2::new(400.0, 100.0);
        for _ in 0..40 {
            m.update(1.0 / 60.0, target);
        }
        assert!((m.pos.x - 100.0).abs() < 0.001, "unguided SAM should not steer");
        assert!(m.pos.y < 200.0);
    }

    #[test]
    fn sams_time_out_rather_than_orbiting_forever() {
        let mut m = Projectile::sam(Vec2::ZERO, true);
        let target = Vec2::new(50.0, 50.0);
        let mut steps = 0;
        while m.update(1.0 / 60.0, target) {
            steps += 1;
            assert!(steps < 10_000, "SAM never expired");
        }
    }

    #[test]
    fn shells_are_aimed_once_and_then_fly_straight() {
        let mut s = Projectile::shell(Vec2::ZERO, Vec2::new(1.0, 1.0));
        let v0 = s.vel;
        s.update(0.5, Vec2::new(-500.0, -500.0));
        assert_eq!(s.vel, v0, "shells must not steer");
        assert!((s.vel.length() - TURRET_SHELL_SPEED).abs() < 0.01);
    }

    #[test]
    fn a_shell_fired_at_nothing_does_not_produce_nan() {
        let s = Projectile::shell(Vec2::ZERO, Vec2::ZERO);
        assert!(s.vel.x.is_finite() && s.vel.y.is_finite());
    }

    #[test]
    fn only_bombs_have_a_blast_radius() {
        assert!(Projectile::bomb(Vec2::ZERO, Vec2::ZERO).blast_radius() > 0.0);
        assert_eq!(Projectile::cannon(Vec2::ZERO, 1.0, 0.0).blast_radius(), 0.0);
        assert_eq!(Projectile::sam(Vec2::ZERO, true).blast_radius(), 0.0);
    }
}
