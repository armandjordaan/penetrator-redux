//! Particles, shockwaves, floating text and screen shake.
//!
//! None of this affects the simulation — nothing here can kill you and nothing
//! here is consulted by collision. It exists purely so that hitting something
//! feels like hitting something.

use crate::config::{SHAKE_MAX_OFFSET, TRAUMA_DECAY};
use crate::rng::Rng;
use crate::util::{damp, fade};
use macroquad::prelude::*;

/// Hard ceiling on live particles. A long game with a lot of explosions would
/// otherwise creep upward forever; when the budget is spent the oldest particles
/// are retired, which is invisible because they are also the faintest.
const MAX_PARTICLES: usize = 1200;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParticleKind {
    /// Bright, fast, short-lived. Impacts.
    Spark,
    /// Slow, expanding, fading. Fills the space an explosion left.
    Smoke,
    /// Tumbling fragments that fall under gravity.
    Debris,
    /// Engine exhaust.
    Exhaust,
}

#[derive(Clone, Copy, Debug)]
pub struct Particle {
    pub pos: Vec2,
    pub vel: Vec2,
    pub life: f32,
    pub max_life: f32,
    pub size: f32,
    pub color: Color,
    pub kind: ParticleKind,
    pub spin: f32,
    pub angle: f32,
}

impl Particle {
    /// 1.0 when just born, 0.0 at death.
    pub fn t(&self) -> f32 {
        (self.life / self.max_life).clamp(0.0, 1.0)
    }
}

/// The expanding ring an explosion pushes out ahead of itself.
#[derive(Clone, Copy, Debug)]
pub struct Shockwave {
    pub pos: Vec2,
    pub radius: f32,
    pub max_radius: f32,
    pub life: f32,
    pub max_life: f32,
    pub color: Color,
}

/// Score popups and mission callouts that drift up off the playfield.
#[derive(Clone, Debug)]
pub struct FloatText {
    pub pos: Vec2,
    pub text: String,
    pub life: f32,
    pub max_life: f32,
    pub color: Color,
    pub size: f32,
}

/// All the cosmetic state, plus the camera trauma accumulator.
#[derive(Default)]
pub struct Fx {
    pub particles: Vec<Particle>,
    pub waves: Vec<Shockwave>,
    pub texts: Vec<FloatText>,
    /// 0..1. Shake magnitude is trauma *squared*, so small knocks stay subtle and
    /// big ones dominate — the standard trick, and it is the right one.
    trauma: f32,
    clock: f32,
}

impl Fx {
    pub fn new() -> Self {
        Fx::default()
    }

    pub fn clear(&mut self) {
        self.particles.clear();
        self.waves.clear();
        self.texts.clear();
        self.trauma = 0.0;
    }

    pub fn update(&mut self, dt: f32) {
        self.clock += dt;
        self.trauma = (self.trauma - TRAUMA_DECAY * dt).max(0.0);

        for p in self.particles.iter_mut() {
            p.life -= dt;
            p.pos += p.vel * dt;
            p.angle += p.spin * dt;
            match p.kind {
                ParticleKind::Spark => {
                    p.vel = Vec2::new(damp(p.vel.x, 0.02, dt), damp(p.vel.y, 0.02, dt));
                }
                ParticleKind::Smoke => {
                    p.vel = Vec2::new(damp(p.vel.x, 0.05, dt), damp(p.vel.y, 0.05, dt) - 6.0 * dt);
                    p.size += 26.0 * dt;
                }
                ParticleKind::Debris => {
                    p.vel.y += 210.0 * dt;
                    p.vel.x = damp(p.vel.x, 0.6, dt);
                }
                ParticleKind::Exhaust => {
                    p.vel = Vec2::new(damp(p.vel.x, 0.08, dt), damp(p.vel.y, 0.08, dt));
                    p.size = (p.size - 7.0 * dt).max(0.0);
                }
            }
        }
        self.particles.retain(|p| p.life > 0.0);

        for w in self.waves.iter_mut() {
            w.life -= dt;
            let t = 1.0 - (w.life / w.max_life).clamp(0.0, 1.0);
            // Ease out, so the ring leaps away and then settles.
            w.radius = w.max_radius * (1.0 - (1.0 - t).powi(3));
        }
        self.waves.retain(|w| w.life > 0.0);

        for t in self.texts.iter_mut() {
            t.life -= dt;
            t.pos.y -= 22.0 * dt;
        }
        self.texts.retain(|t| t.life > 0.0);
    }

    /// Retires the oldest particles once the budget is exceeded.
    fn make_room(&mut self, incoming: usize) {
        let over = (self.particles.len() + incoming).saturating_sub(MAX_PARTICLES);
        if over > 0 {
            self.particles.drain(0..over.min(self.particles.len()));
        }
    }

    /// Adds camera trauma, clamped so a chain of explosions cannot make the
    /// screen unreadable.
    pub fn shake(&mut self, amount: f32) {
        self.trauma = (self.trauma + amount).min(1.0);
    }

    pub fn trauma(&self) -> f32 {
        self.trauma
    }

    /// The current camera offset in virtual units. Uses summed sinusoids rather
    /// than random noise so the shake is smooth instead of a jitter, and so it is
    /// reproducible frame to frame.
    pub fn shake_offset(&self) -> Vec2 {
        if self.trauma <= 0.0 {
            return Vec2::ZERO;
        }
        let m = self.trauma * self.trauma * SHAKE_MAX_OFFSET;
        let t = self.clock;
        Vec2::new(
            m * ((t * 47.0).sin() * 0.6 + (t * 23.3).sin() * 0.4),
            m * ((t * 41.0).cos() * 0.6 + (t * 19.7).cos() * 0.4),
        )
    }

    // -----------------------------------------------------------------------
    // Emitters
    // -----------------------------------------------------------------------

    /// A generic explosion. `scale` is roughly the radius in virtual units and
    /// drives particle count, speed, shake and ring size together, so callers
    /// only ever tune one number.
    pub fn explosion(&mut self, rng: &mut Rng, pos: Vec2, scale: f32, tint: Color) {
        let sparks = ((scale * 1.4) as usize).clamp(8, 90);
        let smoke = ((scale * 0.5) as usize).clamp(3, 26);
        let debris = ((scale * 0.35) as usize).clamp(2, 18);
        self.make_room(sparks + smoke + debris);

        for _ in 0..sparks {
            let a = rng.range(0.0, std::f32::consts::TAU);
            let speed = rng.range(scale * 2.5, scale * 9.0);
            let life = rng.range(0.18, 0.5);
            self.particles.push(Particle {
                pos,
                vel: Vec2::new(a.cos(), a.sin()) * speed,
                life,
                max_life: life,
                size: rng.range(0.8, 2.1),
                color: tint,
                kind: ParticleKind::Spark,
                spin: 0.0,
                angle: 0.0,
            });
        }
        for _ in 0..smoke {
            let a = rng.range(0.0, std::f32::consts::TAU);
            let speed = rng.range(scale * 0.4, scale * 1.8);
            let life = rng.range(0.5, 1.3);
            self.particles.push(Particle {
                pos,
                vel: Vec2::new(a.cos(), a.sin()) * speed,
                life,
                max_life: life,
                size: rng.range(scale * 0.22, scale * 0.5),
                color: Color::new(0.35, 0.32, 0.40, 0.5),
                kind: ParticleKind::Smoke,
                spin: 0.0,
                angle: 0.0,
            });
        }
        for _ in 0..debris {
            let a = rng.range(-std::f32::consts::PI, 0.0); // thrown upward
            let speed = rng.range(scale * 1.5, scale * 5.0);
            let life = rng.range(0.6, 1.4);
            self.particles.push(Particle {
                pos,
                vel: Vec2::new(a.cos(), a.sin()) * speed,
                life,
                max_life: life,
                size: rng.range(1.2, 2.6),
                color: Color::new(0.55, 0.5, 0.48, 1.0),
                kind: ParticleKind::Debris,
                spin: rng.range(-14.0, 14.0),
                angle: 0.0,
            });
        }

        self.waves.push(Shockwave {
            pos,
            radius: 0.0,
            max_radius: scale * 2.6,
            life: 0.36,
            max_life: 0.36,
            color: tint,
        });

        self.shake((scale / 120.0).clamp(0.05, 0.7));
    }

    /// A small directional burst — a round striking armour.
    pub fn impact(&mut self, rng: &mut Rng, pos: Vec2, dir: Vec2, tint: Color) {
        self.make_room(9);
        let base = dir.normalize_or_zero();
        for _ in 0..9 {
            let spread = rng.range(-0.9, 0.9);
            let (s, c) = spread.sin_cos();
            let v = Vec2::new(base.x * c - base.y * s, base.x * s + base.y * c);
            let life = rng.range(0.1, 0.28);
            self.particles.push(Particle {
                pos,
                vel: v * rng.range(45.0, 165.0),
                life,
                max_life: life,
                size: rng.range(0.7, 1.5),
                color: tint,
                kind: ParticleKind::Spark,
                spin: 0.0,
                angle: 0.0,
            });
        }
    }

    /// Engine exhaust. Called every frame while under power, so it stays cheap.
    pub fn exhaust(&mut self, rng: &mut Rng, pos: Vec2, dir: Vec2, intensity: f32) {
        if intensity <= 0.0 {
            return;
        }
        self.make_room(1);
        let life = rng.range(0.12, 0.3) * (0.6 + intensity * 0.6);
        self.particles.push(Particle {
            pos: pos + Vec2::new(rng.signed(), rng.signed()) * 0.8,
            vel: dir * rng.range(30.0, 70.0) + Vec2::new(rng.signed() * 14.0, rng.signed() * 14.0),
            life,
            max_life: life,
            size: rng.range(1.4, 2.8) * (0.7 + intensity * 0.6),
            color: crate::theme::EXHAUST,
            kind: ParticleKind::Exhaust,
            spin: 0.0,
            angle: 0.0,
        });
    }

    /// Dust knocked off a wall the ship scraped past. Purely decorative feedback
    /// that you are flying too close.
    pub fn wall_dust(&mut self, rng: &mut Rng, pos: Vec2, away: Vec2) {
        self.make_room(2);
        for _ in 0..2 {
            let life = rng.range(0.2, 0.45);
            self.particles.push(Particle {
                pos,
                vel: away * rng.range(10.0, 40.0) + Vec2::new(rng.signed() * 20.0, 0.0),
                life,
                max_life: life,
                size: rng.range(0.8, 1.6),
                color: Color::new(0.7, 0.65, 0.55, 0.7),
                kind: ParticleKind::Spark,
                spin: 0.0,
                angle: 0.0,
            });
        }
    }

    pub fn float_text(&mut self, pos: Vec2, text: impl Into<String>, color: Color, size: f32) {
        self.texts.push(FloatText {
            pos,
            text: text.into(),
            life: 1.4,
            max_life: 1.4,
            color,
            size,
        });
    }

    /// Colour of a particle right now, with its age fade applied.
    pub fn particle_color(p: &Particle) -> Color {
        let t = p.t();
        match p.kind {
            // Sparks are white-hot when born and cool through their tint.
            ParticleKind::Spark => {
                let hot = Color::new(1.0, 1.0, 0.92, 1.0);
                fade(crate::util::mix(p.color, hot, t * t), t)
            }
            ParticleKind::Smoke => fade(p.color, t * 0.7),
            ParticleKind::Debris => fade(p.color, t),
            ParticleKind::Exhaust => fade(p.color, t * t),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn particles_expire_and_the_list_drains() {
        let mut fx = Fx::new();
        let mut rng = Rng::new(1);
        fx.explosion(&mut rng, Vec2::ZERO, 40.0, RED);
        assert!(!fx.particles.is_empty());
        for _ in 0..400 {
            fx.update(1.0 / 60.0);
        }
        assert!(fx.particles.is_empty(), "particles leaked");
        assert!(fx.waves.is_empty());
    }

    #[test]
    fn the_particle_budget_is_never_exceeded() {
        let mut fx = Fx::new();
        let mut rng = Rng::new(2);
        for _ in 0..500 {
            fx.explosion(&mut rng, Vec2::new(10.0, 10.0), 120.0, RED);
        }
        assert!(
            fx.particles.len() <= MAX_PARTICLES,
            "budget blown: {}",
            fx.particles.len()
        );
    }

    #[test]
    fn trauma_saturates_and_then_decays_to_zero() {
        let mut fx = Fx::new();
        for _ in 0..50 {
            fx.shake(0.5);
        }
        assert!((fx.trauma() - 1.0).abs() < 1e-6, "trauma should clamp at 1");
        for _ in 0..200 {
            fx.update(1.0 / 60.0);
        }
        assert_eq!(fx.trauma(), 0.0);
        assert_eq!(fx.shake_offset(), Vec2::ZERO);
    }

    #[test]
    fn shake_offset_stays_within_the_configured_bound() {
        let mut fx = Fx::new();
        fx.shake(1.0);
        for _ in 0..600 {
            fx.update(1.0 / 120.0);
            fx.shake(1.0);
            let o = fx.shake_offset();
            assert!(o.x.abs() <= SHAKE_MAX_OFFSET + 0.01);
            assert!(o.y.abs() <= SHAKE_MAX_OFFSET + 0.01);
        }
    }

    #[test]
    fn explosion_scale_drives_particle_count() {
        let mut rng = Rng::new(3);
        let mut small = Fx::new();
        small.explosion(&mut rng, Vec2::ZERO, 12.0, RED);
        let mut big = Fx::new();
        big.explosion(&mut rng, Vec2::ZERO, 90.0, RED);
        assert!(big.particles.len() > small.particles.len());
        assert!(big.trauma() > small.trauma());
    }

    #[test]
    fn float_text_rises_and_expires() {
        let mut fx = Fx::new();
        fx.float_text(Vec2::new(0.0, 100.0), "150", WHITE, 8.0);
        fx.update(0.5);
        assert!(fx.texts[0].pos.y < 100.0);
        for _ in 0..120 {
            fx.update(1.0 / 60.0);
        }
        assert!(fx.texts.is_empty());
    }

    #[test]
    fn clear_resets_everything() {
        let mut fx = Fx::new();
        let mut rng = Rng::new(4);
        fx.explosion(&mut rng, Vec2::ZERO, 50.0, RED);
        fx.float_text(Vec2::ZERO, "x", WHITE, 8.0);
        fx.clear();
        assert!(fx.particles.is_empty() && fx.waves.is_empty() && fx.texts.is_empty());
        assert_eq!(fx.trauma(), 0.0);
    }
}
