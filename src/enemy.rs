//! The opposition.
//!
//! All six enemy types share one struct. They are distinguished by their
//! [`SpawnKind`] and by the branch they take in [`Enemy::update`], which returns
//! an [`Action`] rather than mutating the world directly — the enemy decides
//! *what* it wants to do, and the world decides how that becomes a projectile.
//! That split is what keeps the borrow checker out of the way and the AI testable
//! without a world to put it in.

use crate::config::*;
use crate::level::SpawnKind;
use crate::rng::Rng;
use crate::util::damp;
use macroquad::prelude::*;

/// What an enemy wants the world to do on its behalf this frame.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    /// Launch a SAM from `from`. `homing` reflects the radar state at launch —
    /// destroying the last dish does not turn missiles already in the air dumb.
    LaunchSam { from: Vec2, homing: bool },
    /// Fire an aimed shell from `from` in direction `dir`.
    FireShell { from: Vec2, dir: Vec2 },
}

/// Everything the AI is allowed to know about the world.
#[derive(Clone, Copy, Debug)]
pub struct Senses {
    pub player: Vec2,
    pub player_vel: Vec2,
    /// True while at least one radar dish stands.
    pub radar_active: bool,
    /// Which way the ship is travelling: +1 outbound, -1 on the way home.
    pub travel_dir: f32,
    /// Fire-rate multiplier. 1.0 outbound, higher on the way home.
    pub aggression: f32,
    /// True while the player is dead or the mission is over; enemies hold fire.
    pub ceasefire: bool,
}

#[derive(Clone, Debug)]
pub struct Enemy {
    pub kind: SpawnKind,
    pub pos: Vec2,
    pub vel: Vec2,
    pub hp: i32,
    pub max_hp: i32,
    /// Counts down to the next shot.
    pub fire_timer: f32,
    /// Free-running phase for dish rotation, mine bob and warhead pulse.
    pub anim: f32,
    /// Decays after taking damage; drives the white flash.
    pub hit_flash: f32,
    /// Index into `Track::spawns` this enemy came from, so the world knows what
    /// to mark destroyed.
    pub spawn_idx: usize,
    /// Where the mine started, so it drifts around a point instead of wandering off.
    home: Vec2,
}

impl Enemy {
    pub fn new(kind: SpawnKind, pos: Vec2, spawn_idx: usize, rng: &mut Rng) -> Self {
        let hp = match kind {
            SpawnKind::Silo => SILO_HP,
            SpawnKind::Radar => RADAR_HP,
            SpawnKind::Turret => TURRET_HP,
            SpawnKind::Mine => MINE_HP,
            SpawnKind::Drone => DRONE_HP,
            SpawnKind::Warhead => WARHEAD_HP,
        };
        Enemy {
            kind,
            pos,
            vel: Vec2::ZERO,
            hp,
            max_hp: hp,
            // Stagger the opening salvo so a cluster of silos does not fire in
            // lockstep, which reads as scripted rather than defended. Drones
            // reuse this field as a break-off timer and must start at zero, or
            // the first thing they do is run away.
            fire_timer: if kind == SpawnKind::Drone {
                0.0
            } else {
                rng.range(0.4, 1.9)
            },
            anim: rng.range(0.0, std::f32::consts::TAU),
            hit_flash: 0.0,
            spawn_idx,
            home: pos,
        }
    }

    /// Collision radius.
    pub fn radius(&self) -> f32 {
        match self.kind {
            SpawnKind::Silo => 8.0,
            SpawnKind::Radar => 8.5,
            SpawnKind::Turret => 7.5,
            SpawnKind::Mine => MINE_RADIUS,
            SpawnKind::Drone => DRONE_RADIUS,
            SpawnKind::Warhead => WARHEAD_RADIUS,
        }
    }

    /// Points for killing it.
    pub fn score(&self) -> u32 {
        match self.kind {
            SpawnKind::Silo => SCORE_SILO,
            SpawnKind::Radar => SCORE_RADAR,
            SpawnKind::Turret => SCORE_TURRET,
            SpawnKind::Mine => SCORE_MINE,
            SpawnKind::Drone => SCORE_DRONE,
            SpawnKind::Warhead => SCORE_WARHEAD,
        }
    }

    /// The warhead is armoured against cannon fire. Only a bomb will do it — the
    /// single rule that turns the bunker from a shooting gallery into a problem.
    pub fn immune_to_cannon(&self) -> bool {
        self.kind == SpawnKind::Warhead
    }

    /// How big an explosion this leaves behind.
    pub fn explosion_scale(&self) -> f32 {
        match self.kind {
            SpawnKind::Warhead => 150.0,
            SpawnKind::Radar => 44.0,
            SpawnKind::Silo => 38.0,
            SpawnKind::Turret => 34.0,
            SpawnKind::Drone => 30.0,
            SpawnKind::Mine => 26.0,
        }
    }

    pub fn is_alive(&self) -> bool {
        self.hp > 0
    }

    /// Applies damage and returns true if this killed it.
    pub fn damage(&mut self, amount: i32) -> bool {
        if self.hp <= 0 {
            return false;
        }
        self.hp -= amount;
        self.hit_flash = 1.0;
        self.hp <= 0
    }

    /// One step of behaviour.
    pub fn update(&mut self, dt: f32, senses: &Senses, rng: &mut Rng) -> Option<Action> {
        self.anim += dt;
        self.hit_flash = (self.hit_flash - dt * 4.0).max(0.0);

        match self.kind {
            SpawnKind::Silo => self.update_silo(dt, senses, rng),
            SpawnKind::Turret => self.update_turret(dt, senses, rng),
            SpawnKind::Mine => {
                self.update_mine(dt);
                None
            }
            SpawnKind::Drone => {
                self.update_drone(dt, senses);
                None
            }
            SpawnKind::Radar | SpawnKind::Warhead => None,
        }
    }

    /// Whether the ship is somewhere this emplacement is allowed to shoot at.
    ///
    /// The envelope is asymmetric on purpose: an emplacement engages a ship that
    /// is *approaching* it, and stops shortly after the ship has gone past. Two
    /// reasons, and the second is the important one. It is how a real launch
    /// envelope works — but more than that, the player's cannon only fires
    /// forward, so a missile launched from behind has no answer at all. Firing
    /// only at approaching targets is what keeps every shot in the game
    /// something the player could have done something about.
    fn engagement_envelope(&self, senses: &Senses, range: f32) -> bool {
        // How far ahead of the ship this emplacement still is, along the
        // direction of travel. Positive means not yet passed.
        let ahead = (self.pos.x - senses.player.x) * senses.travel_dir;
        ahead > -TRAILING_FIRE_MARGIN && ahead < range
    }

    fn update_silo(&mut self, dt: f32, senses: &Senses, rng: &mut Rng) -> Option<Action> {
        if senses.ceasefire || !self.engagement_envelope(senses, SILO_TRIGGER_RANGE) {
            return None;
        }
        self.fire_timer -= dt * senses.aggression;
        if self.fire_timer > 0.0 {
            return None;
        }
        self.fire_timer = SILO_FIRE_INTERVAL * rng.range(0.8, 1.25);
        Some(Action::LaunchSam {
            from: self.pos + Vec2::new(0.0, -6.0),
            homing: senses.radar_active,
        })
    }

    fn update_turret(&mut self, dt: f32, senses: &Senses, rng: &mut Rng) -> Option<Action> {
        if senses.ceasefire || !self.engagement_envelope(senses, TURRET_TRIGGER_RANGE) {
            return None;
        }
        self.fire_timer -= dt * senses.aggression;
        if self.fire_timer > 0.0 {
            return None;
        }
        self.fire_timer = TURRET_FIRE_INTERVAL * rng.range(0.85, 1.2);

        let muzzle = self.pos + Vec2::new(0.0, 7.0);
        Some(Action::FireShell {
            from: muzzle,
            dir: lead_target(muzzle, senses.player, senses.player_vel, TURRET_SHELL_SPEED),
        })
    }

    fn update_mine(&mut self, dt: f32) {
        // A slow lissajous around the spawn point: enough movement to spoil a
        // memorised line, not enough to feel like it is chasing you.
        let drift = Vec2::new(
            (self.anim * 0.7).sin() * MINE_DRIFT,
            (self.anim * 1.1).cos() * MINE_DRIFT * 0.6,
        );
        let target = self.home + drift;
        self.vel = (target - self.pos) * 3.0;
        self.pos += self.vel * dt;
    }

    /// Interceptors fly attack runs rather than ramming.
    ///
    /// A drone that simply steers at the ship until one of them stops existing
    /// is a homing mine, and an unfair one: it arrives head-on at combined
    /// speed, there is no room to go round it, and dying to it puts you back at
    /// a checkpoint where it does the same thing again. Pressing in to
    /// [`DRONE_STANDOFF`] and then breaking away for [`DRONE_BREAK_TIME`] keeps
    /// the pressure on while leaving the player a window to shoot — which is
    /// what makes it an enemy instead of an obstacle.
    ///
    /// Contact is still fatal. It just has to be earned.
    fn update_drone(&mut self, dt: f32, senses: &Senses) {
        let to_player = senses.player - self.pos;
        let heading = to_player.normalize_or_zero();

        self.fire_timer -= dt;
        if self.fire_timer <= 0.0 && to_player.length() < DRONE_STANDOFF {
            self.fire_timer = DRONE_BREAK_TIME;
        }
        let breaking = self.fire_timer > 0.0;

        let desired = if senses.ceasefire {
            Vec2::ZERO
        } else if breaking {
            -heading * DRONE_SPEED
        } else {
            heading * DRONE_SPEED
        };

        // Ease into the desired heading so drones bank rather than snap.
        self.vel = Vec2::new(
            damp(self.vel.x - desired.x, 0.02, dt) + desired.x,
            damp(self.vel.y - desired.y, 0.02, dt) + desired.y,
        );
        self.pos += self.vel * dt;
    }
}

/// First-order intercept: where to aim so a shot at `speed` meets a target moving
/// at `target_vel`.
///
/// The exact solution is a quadratic; this iterates the flight time twice
/// instead, which converges fast enough for a game and — importantly — cannot
/// produce a NaN when the target is unreachable. A turret that misses is fine. A
/// turret that fires a NaN takes the whole simulation with it.
pub fn lead_target(from: Vec2, target: Vec2, target_vel: Vec2, speed: f32) -> Vec2 {
    if speed <= 0.0 {
        return Vec2::new(0.0, 1.0);
    }
    let mut aim = target;
    for _ in 0..2 {
        let t = from.distance(aim) / speed;
        aim = target + target_vel * t.clamp(0.0, 3.0);
    }
    let dir = aim - from;
    if dir.length_squared() < 1e-6 {
        Vec2::new(0.0, 1.0)
    } else {
        dir.normalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn senses(player: Vec2) -> Senses {
        Senses {
            player,
            player_vel: Vec2::ZERO,
            radar_active: true,
            travel_dir: 1.0,
            aggression: 1.0,
            ceasefire: false,
        }
    }

    fn silo_at(x: f32) -> Enemy {
        Enemy::new(SpawnKind::Silo, Vec2::new(x, 200.0), 0, &mut Rng::new(1))
    }

    /// A ship approaching an emplacement at `silo_x` from the left.
    fn approaching(silo_x: f32, distance: f32) -> Senses {
        senses(Vec2::new(silo_x - distance, 100.0))
    }

    #[test]
    fn a_silo_holds_fire_until_the_player_is_in_range() {
        let mut s = silo_at(1000.0);
        let mut rng = Rng::new(2);
        let far = approaching(1000.0, SILO_TRIGGER_RANGE + 50.0);
        for _ in 0..600 {
            assert_eq!(s.update(1.0 / 60.0, &far, &mut rng), None);
        }
    }

    #[test]
    fn a_silo_eventually_fires_once_you_are_close() {
        let mut s = silo_at(1000.0);
        let mut rng = Rng::new(3);
        let near = approaching(1000.0, 120.0);
        let fired = (0..600).any(|_| s.update(1.0 / 60.0, &near, &mut rng).is_some());
        assert!(fired, "silo never launched");
    }

    #[test]
    fn sam_guidance_matches_the_radar_state_at_launch() {
        let mut rng = Rng::new(4);
        for radar_active in [true, false] {
            let mut s = silo_at(1000.0);
            let mut sense = approaching(1000.0, 120.0);
            sense.radar_active = radar_active;
            let action = (0..600).find_map(|_| s.update(1.0 / 60.0, &sense, &mut rng));
            match action {
                Some(Action::LaunchSam { homing, .. }) => assert_eq!(homing, radar_active),
                other => panic!("expected a launch, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_ceasefire_silences_everything() {
        let mut rng = Rng::new(5);
        let mut sense = approaching(1000.0, 80.0);
        sense.ceasefire = true;
        let mut silo = silo_at(1000.0);
        let mut turret = Enemy::new(SpawnKind::Turret, Vec2::new(1000.0, 40.0), 0, &mut rng);
        for _ in 0..900 {
            assert!(silo.update(1.0 / 60.0, &sense, &mut rng).is_none());
            assert!(turret.update(1.0 / 60.0, &sense, &mut rng).is_none());
        }
    }

    #[test]
    fn aggression_makes_enemies_fire_sooner() {
        // Same seeds either side, so the only variable is the aggression dial.
        let count_shots = |aggression: f32| {
            let mut s = Enemy::new(SpawnKind::Silo, Vec2::new(1000.0, 200.0), 0, &mut Rng::new(9));
            let mut sense = approaching(1000.0, 120.0);
            sense.aggression = aggression;
            let mut r = Rng::new(9);
            (0..900)
                .filter(|_| s.update(1.0 / 60.0, &sense, &mut r).is_some())
                .count()
        };
        assert!(count_shots(2.0) > count_shots(1.0));
    }

    #[test]
    fn emplacements_shoot_at_what_is_coming_not_at_what_has_gone() {
        let mut rng = Rng::new(77);
        let silo_x = 1000.0;

        // Approaching, in range: engaged.
        let mut approach = silo_at(silo_x);
        assert!(
            (0..900).any(|_| approach
                .update(1.0 / 60.0, &approaching(silo_x, 150.0), &mut rng)
                .is_some()),
            "a silo should engage an approaching ship"
        );

        // Well past it: silent, because a forward-firing cannon could never
        // answer a missile launched from behind.
        let mut departed = silo_at(silo_x);
        let behind = senses(Vec2::new(silo_x + 150.0, 100.0));
        for _ in 0..900 {
            assert!(departed.update(1.0 / 60.0, &behind, &mut rng).is_none());
        }

        // And the same position is "approaching" again on the way home.
        let mut homeward = silo_at(silo_x);
        let mut going_back = senses(Vec2::new(silo_x + 150.0, 100.0));
        going_back.travel_dir = -1.0;
        assert!(
            (0..900).any(|_| homeward.update(1.0 / 60.0, &going_back, &mut rng).is_some()),
            "the envelope must flip with the direction of travel"
        );
    }

    #[test]
    fn drones_close_on_the_player() {
        let mut rng = Rng::new(7);
        let mut d = Enemy::new(SpawnKind::Drone, Vec2::new(0.0, 100.0), 0, &mut rng);
        let target = Vec2::new(400.0, 150.0);
        let start = d.pos.distance(target);
        for _ in 0..120 {
            d.update(1.0 / 60.0, &senses(target), &mut rng);
        }
        assert!(d.pos.distance(target) < start);
    }

    #[test]
    fn drones_break_off_instead_of_ramming() {
        let mut rng = Rng::new(12);
        let target = Vec2::new(240.0, 135.0);
        let mut d = Enemy::new(SpawnKind::Drone, Vec2::new(40.0, 135.0), 0, &mut rng);

        let mut closest = f32::MAX;
        let mut broke_off = false;
        let mut previous = d.pos.distance(target);
        for _ in 0..1200 {
            d.update(1.0 / 60.0, &senses(target), &mut rng);
            let range = d.pos.distance(target);
            closest = closest.min(range);
            // Opening the range while still nearby is a break-off.
            if range > previous && range < DRONE_STANDOFF * 2.0 {
                broke_off = true;
            }
            previous = range;
        }
        assert!(closest < DRONE_STANDOFF + 5.0, "the drone should press its attack");
        assert!(broke_off, "the drone should break off rather than ram");
    }

    #[test]
    fn mines_stay_near_where_they_were_placed() {
        let mut rng = Rng::new(8);
        let home = Vec2::new(100.0, 120.0);
        let mut m = Enemy::new(SpawnKind::Mine, home, 0, &mut rng);
        for _ in 0..1200 {
            m.update(1.0 / 60.0, &senses(Vec2::new(400.0, 40.0)), &mut rng);
            assert!(
                m.pos.distance(home) < MINE_DRIFT * 2.0,
                "mine wandered to {:?}",
                m.pos
            );
        }
    }

    #[test]
    fn damage_kills_exactly_once() {
        let mut rng = Rng::new(10);
        let mut e = Enemy::new(SpawnKind::Silo, Vec2::ZERO, 0, &mut rng);
        assert!(!e.damage(1), "should survive the first hit");
        assert!(e.damage(1), "should die on the second");
        assert!(!e.damage(5), "a corpse cannot die again");
        assert!(!e.is_alive());
    }

    #[test]
    fn the_warhead_shrugs_off_cannon_fire() {
        let mut rng = Rng::new(11);
        let w = Enemy::new(SpawnKind::Warhead, Vec2::ZERO, 0, &mut rng);
        assert!(w.immune_to_cannon());
        // And nothing else is, or the rule would not read as special.
        for kind in [
            SpawnKind::Silo,
            SpawnKind::Radar,
            SpawnKind::Turret,
            SpawnKind::Mine,
            SpawnKind::Drone,
        ] {
            assert!(!Enemy::new(kind, Vec2::ZERO, 0, &mut rng).immune_to_cannon());
        }
    }

    #[test]
    fn lead_aims_ahead_of_a_moving_target() {
        let from = Vec2::new(0.0, 0.0);
        let target = Vec2::new(100.0, 0.0);
        let straight = lead_target(from, target, Vec2::ZERO, 100.0);
        let leading = lead_target(from, target, Vec2::new(0.0, -50.0), 100.0);
        assert!((straight.y).abs() < 1e-4);
        assert!(leading.y < -0.1, "should aim above a climbing target");
    }

    #[test]
    fn lead_never_returns_nan() {
        let cases = [
            (Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, 100.0),
            (Vec2::ZERO, Vec2::new(10.0, 10.0), Vec2::new(1e9, 1e9), 100.0),
            (Vec2::ZERO, Vec2::new(10.0, 10.0), Vec2::ZERO, 0.0),
        ];
        for (from, target, vel, speed) in cases {
            let d = lead_target(from, target, vel, speed);
            assert!(d.x.is_finite() && d.y.is_finite(), "NaN from {from:?} {target:?}");
            assert!((d.length() - 1.0).abs() < 1e-3, "direction not normalised");
        }
    }
}
