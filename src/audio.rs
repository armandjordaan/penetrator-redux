//! Sound, synthesised at startup.
//!
//! The game ships no audio files. Every effect is generated into an in-memory
//! WAV buffer when the game boots and handed straight to the mixer. That keeps
//! the whole game a single binary with no asset directory to lose, and it means
//! the *design* of each sound is readable source code rather than an opaque blob.
//!
//! When the crate is built with `--no-default-features` macroquad's audio backend
//! is compiled out entirely and every call here becomes a no-op, so headless and
//! CI builds need no sound device.

use crate::rng::Rng;
use macroquad::audio::{
    load_sound_from_bytes, play_sound, play_sound_once, set_sound_volume, stop_sound,
    PlaySoundParams, Sound,
};

const SAMPLE_RATE: u32 = 22_050;

/// Every sound the game can make.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sfx {
    Cannon,
    BombDrop,
    HitArmour,
    ExplodeSmall,
    ExplodeLarge,
    SamLaunch,
    RadarDown,
    PlayerDeath,
    WarheadKill,
    Overheat,
    UiMove,
    UiConfirm,
}

/// The loaded sound bank.
///
/// Loading is fallible — a machine with no working audio device is a perfectly
/// reasonable machine to play a game on — so every slot is an `Option` and every
/// playback call degrades to silence rather than failing.
pub struct Audio {
    cannon: Option<Sound>,
    bomb_drop: Option<Sound>,
    hit_armour: Option<Sound>,
    explode_small: Option<Sound>,
    explode_large: Option<Sound>,
    sam_launch: Option<Sound>,
    radar_down: Option<Sound>,
    player_death: Option<Sound>,
    warhead_kill: Option<Sound>,
    overheat: Option<Sound>,
    ui_move: Option<Sound>,
    ui_confirm: Option<Sound>,
    engine: Option<Sound>,

    engine_running: bool,
    /// Master mute, toggled with M.
    pub muted: bool,
    pub volume: f32,
}

impl Audio {
    /// Synthesises and uploads the whole bank. Takes a few milliseconds.
    ///
    /// If the mixer rejects everything — no device, no backend compiled in — the
    /// whole bank is replaced with a silent one, so the rest of the game stops
    /// paying for playback calls that can never make a sound.
    pub async fn load() -> Audio {
        let mut rng = Rng::new(0xA0D10);

        let bank = Audio {
            cannon: upload(&synth_cannon()).await,
            bomb_drop: upload(&synth_bomb_drop()).await,
            hit_armour: upload(&synth_hit()).await,
            explode_small: upload(&synth_explosion(&mut rng, 0.42, 900.0, 0.9)).await,
            explode_large: upload(&synth_explosion(&mut rng, 0.95, 420.0, 1.0)).await,
            sam_launch: upload(&synth_sam_launch(&mut rng)).await,
            radar_down: upload(&synth_radar_down()).await,
            player_death: upload(&synth_player_death(&mut rng)).await,
            warhead_kill: upload(&synth_explosion(&mut rng, 1.9, 260.0, 1.0)).await,
            overheat: upload(&synth_overheat()).await,
            ui_move: upload(&synth_blip(660.0, 0.05)).await,
            ui_confirm: upload(&synth_blip(1180.0, 0.09)).await,
            engine: upload(&synth_engine()).await,

            engine_running: false,
            muted: false,
            volume: 0.7,
        };

        if bank.cannon.is_none() && bank.engine.is_none() {
            return Audio::silent();
        }
        bank
    }

    /// A bank with nothing in it. Every call becomes a no-op.
    ///
    /// This is what the simulation tests run against: `load` needs a graphics
    /// context to reach the mixer, and no test of the flight model cares what
    /// noise it makes.
    pub fn silent() -> Audio {
        Audio {
            cannon: None,
            bomb_drop: None,
            hit_armour: None,
            explode_small: None,
            explode_large: None,
            sam_launch: None,
            radar_down: None,
            player_death: None,
            warhead_kill: None,
            overheat: None,
            ui_move: None,
            ui_confirm: None,
            engine: None,
            engine_running: false,
            muted: true,
            volume: 0.0,
        }
    }

    fn slot(&self, sfx: Sfx) -> Option<&Sound> {
        match sfx {
            Sfx::Cannon => self.cannon.as_ref(),
            Sfx::BombDrop => self.bomb_drop.as_ref(),
            Sfx::HitArmour => self.hit_armour.as_ref(),
            Sfx::ExplodeSmall => self.explode_small.as_ref(),
            Sfx::ExplodeLarge => self.explode_large.as_ref(),
            Sfx::SamLaunch => self.sam_launch.as_ref(),
            Sfx::RadarDown => self.radar_down.as_ref(),
            Sfx::PlayerDeath => self.player_death.as_ref(),
            Sfx::WarheadKill => self.warhead_kill.as_ref(),
            Sfx::Overheat => self.overheat.as_ref(),
            Sfx::UiMove => self.ui_move.as_ref(),
            Sfx::UiConfirm => self.ui_confirm.as_ref(),
        }
    }

    /// Relative loudness per effect, so the cannon does not drown the explosions.
    fn gain(sfx: Sfx) -> f32 {
        match sfx {
            Sfx::Cannon => 0.30,
            Sfx::BombDrop => 0.42,
            Sfx::HitArmour => 0.38,
            Sfx::ExplodeSmall => 0.65,
            Sfx::ExplodeLarge => 0.85,
            Sfx::SamLaunch => 0.50,
            Sfx::RadarDown => 0.70,
            Sfx::PlayerDeath => 0.90,
            Sfx::WarheadKill => 1.00,
            Sfx::Overheat => 0.45,
            Sfx::UiMove => 0.35,
            Sfx::UiConfirm => 0.50,
        }
    }

    pub fn play(&self, sfx: Sfx) {
        if self.muted {
            return;
        }
        if let Some(sound) = self.slot(sfx) {
            play_sound(
                sound,
                PlaySoundParams {
                    looped: false,
                    volume: Self::gain(sfx) * self.volume,
                },
            );
        }
    }

    /// Starts the looping engine drone, or updates its volume if already running.
    /// `intensity` is 0..1 and tracks the throttle.
    pub fn engine(&mut self, intensity: f32) {
        let Some(engine) = self.engine.as_ref() else {
            return;
        };
        if self.muted {
            self.stop_engine();
            return;
        }
        let vol = (0.06 + 0.14 * intensity.clamp(0.0, 1.0)) * self.volume;
        if !self.engine_running {
            play_sound(
                engine,
                PlaySoundParams {
                    looped: true,
                    volume: vol,
                },
            );
            self.engine_running = true;
        } else {
            set_sound_volume(engine, vol);
        }
    }

    pub fn stop_engine(&mut self) {
        if self.engine_running {
            if let Some(engine) = self.engine.as_ref() {
                stop_sound(engine);
            }
            self.engine_running = false;
        }
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        if self.muted {
            self.stop_engine();
        }
        if !self.muted {
            if let Some(s) = self.ui_confirm.as_ref() {
                play_sound_once(s);
            }
        }
    }
}

async fn upload(pcm: &[f32]) -> Option<Sound> {
    load_sound_from_bytes(&encode_wav(pcm)).await.ok()
}

// ---------------------------------------------------------------------------
// WAV encoding
// ---------------------------------------------------------------------------

/// Wraps mono float samples in a 16-bit PCM RIFF/WAVE container.
///
/// Samples are hard-clipped to `[-1, 1]` before conversion; a synth voice that
/// overshoots should distort audibly rather than wrap around into a click.
fn encode_wav(samples: &[f32]) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);

    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");

    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // format: PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // channels: mono
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

// ---------------------------------------------------------------------------
// Voices
// ---------------------------------------------------------------------------

fn frames(seconds: f32) -> usize {
    (seconds * SAMPLE_RATE as f32) as usize
}

/// Exponential decay envelope. `sharpness` above 1 makes the tail snap shut.
fn env_decay(t: f32, len: f32, sharpness: f32) -> f32 {
    (1.0 - (t / len).clamp(0.0, 1.0)).powf(sharpness)
}

/// A short attack ramp, so nothing starts with a click.
fn env_attack(t: f32, attack: f32) -> f32 {
    (t / attack).clamp(0.0, 1.0)
}

fn square(phase: f32) -> f32 {
    if phase.fract() < 0.5 {
        1.0
    } else {
        -1.0
    }
}

/// One-pole low-pass. Turns white noise into something with weight behind it.
struct LowPass {
    y: f32,
}
impl LowPass {
    fn new() -> Self {
        LowPass { y: 0.0 }
    }
    fn step(&mut self, x: f32, cutoff_ratio: f32) -> f32 {
        self.y += (x - self.y) * cutoff_ratio.clamp(0.001, 1.0);
        self.y
    }
}

/// Cannon: a square wave dropping fast in pitch. Short enough to fire ten a
/// second without turning into a wall of noise.
fn synth_cannon() -> Vec<f32> {
    let len = 0.075;
    let n = frames(len);
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            let freq = 880.0 * (1.0 - 0.72 * (t / len));
            phase += freq / SAMPLE_RATE as f32;
            square(phase) * env_decay(t, len, 2.2) * env_attack(t, 0.002) * 0.7
        })
        .collect()
}

/// Bomb release: a descending airy whistle.
fn synth_bomb_drop() -> Vec<f32> {
    let len = 0.3;
    let n = frames(len);
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            let freq = 1400.0 * (1.0 - 0.8 * (t / len));
            phase += freq / SAMPLE_RATE as f32;
            (phase * std::f32::consts::TAU).sin() * env_decay(t, len, 1.4) * env_attack(t, 0.01) * 0.6
        })
        .collect()
}

/// A round bouncing off armour: bright, metallic, over immediately.
fn synth_hit() -> Vec<f32> {
    let len = 0.06;
    let n = frames(len);
    (0..n)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            let a = (t * 1500.0 * std::f32::consts::TAU).sin();
            let b = (t * 2270.0 * std::f32::consts::TAU).sin();
            (a * 0.6 + b * 0.4) * env_decay(t, len, 3.0) * 0.8
        })
        .collect()
}

/// Explosion: filtered noise with a sub-bass thump under it. The cutoff sweeps
/// closed over the duration, which is what makes it read as "receding" rather
/// than just "static".
fn synth_explosion(rng: &mut Rng, len: f32, sub_freq: f32, body: f32) -> Vec<f32> {
    let n = frames(len);
    let mut lp = LowPass::new();
    let mut sub_phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            let progress = t / len;

            let cutoff = 0.55 * (1.0 - progress).powf(1.6) + 0.02;
            let noise = lp.step(rng.signed(), cutoff);

            let sub_hz = sub_freq * (1.0 - 0.6 * progress) * 0.25;
            sub_phase += sub_hz / SAMPLE_RATE as f32;
            let sub = (sub_phase * std::f32::consts::TAU).sin();

            let e = env_decay(t, len, 1.8) * env_attack(t, 0.004);
            (noise * 1.6 * body + sub * 0.7) * e
        })
        .collect()
}

/// SAM launch: a rising sweep with noise riding on it.
fn synth_sam_launch(rng: &mut Rng) -> Vec<f32> {
    let len = 0.55;
    let n = frames(len);
    let mut phase = 0.0f32;
    let mut lp = LowPass::new();
    (0..n)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            let progress = t / len;
            let freq = 180.0 + 900.0 * progress * progress;
            phase += freq / SAMPLE_RATE as f32;
            let tone = (phase * std::f32::consts::TAU).sin();
            let hiss = lp.step(rng.signed(), 0.25);
            (tone * 0.5 + hiss * 0.5) * env_decay(t, len, 1.1) * env_attack(t, 0.02) * 0.8
        })
        .collect()
}

/// Radar destroyed: a falling two-tone that is meant to sound like relief.
fn synth_radar_down() -> Vec<f32> {
    let len = 0.7;
    let n = frames(len);
    let mut p1 = 0.0f32;
    let mut p2 = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            let progress = t / len;
            let f1 = 760.0 * (1.0 - 0.55 * progress);
            let f2 = f1 * 0.667;
            p1 += f1 / SAMPLE_RATE as f32;
            p2 += f2 / SAMPLE_RATE as f32;
            let v = (p1 * std::f32::consts::TAU).sin() * 0.5
                + (p2 * std::f32::consts::TAU).sin() * 0.5;
            v * env_decay(t, len, 1.5) * env_attack(t, 0.01) * 0.8
        })
        .collect()
}

/// Losing a ship: a long tumbling tone smothered in noise.
fn synth_player_death(rng: &mut Rng) -> Vec<f32> {
    let len = 1.3;
    let n = frames(len);
    let mut phase = 0.0f32;
    let mut lp = LowPass::new();
    (0..n)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            let progress = t / len;
            // A slow wobble on the way down; the ship is spinning.
            let wobble = 1.0 + 0.18 * (t * 9.0 * std::f32::consts::TAU).sin();
            let freq = 520.0 * (1.0 - 0.8 * progress) * wobble;
            phase += freq / SAMPLE_RATE as f32;
            let tone = square(phase);
            let noise = lp.step(rng.signed(), 0.4 * (1.0 - progress) + 0.03);
            (tone * 0.45 + noise * 0.9) * env_decay(t, len, 1.3) * env_attack(t, 0.005) * 0.85
        })
        .collect()
}

/// Cannon overheat: an unhappy buzz that tells you to stop holding the trigger.
fn synth_overheat() -> Vec<f32> {
    let len = 0.45;
    let n = frames(len);
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            // Amplitude modulation gives it the stutter of an alarm.
            let gate = if (t * 22.0).fract() < 0.5 { 1.0 } else { 0.25 };
            phase += 150.0 / SAMPLE_RATE as f32;
            square(phase) * gate * env_decay(t, len, 1.0) * 0.55
        })
        .collect()
}

/// Menu blip.
fn synth_blip(freq: f32, len: f32) -> Vec<f32> {
    let n = frames(len);
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            phase += freq / SAMPLE_RATE as f32;
            square(phase) * env_decay(t, len, 2.0) * env_attack(t, 0.003) * 0.5
        })
        .collect()
}

/// The engine drone. This one has to loop seamlessly. The buffer is 0.5 s long,
/// so any partial at an even number of hertz completes a whole number of cycles
/// across it and meets itself at the loop point — 120, 90 and 60 Hz all qualify.
/// Pick an odd frequency here and the game clicks twice a second forever.
fn synth_engine() -> Vec<f32> {
    let len = 0.5;
    let n = frames(len);
    (0..n)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            let a = (t * 120.0 * std::f32::consts::TAU).sin();
            let b = (t * 90.0 * std::f32::consts::TAU).sin();
            let c = (t * 60.0 * std::f32::consts::TAU).sin();
            (a * 0.28 + b * 0.32 + c * 0.40) * 0.8
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_well_formed() {
        let pcm = vec![0.0f32; 100];
        let wav = encode_wav(&pcm);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(wav.len(), 44 + 200);

        let riff_size = u32::from_le_bytes(wav[4..8].try_into().unwrap());
        assert_eq!(riff_size as usize, wav.len() - 8);
        let data_size = u32::from_le_bytes(wav[40..44].try_into().unwrap());
        assert_eq!(data_size as usize, 200);
    }

    #[test]
    fn encoding_clips_instead_of_wrapping() {
        // A voice that overshoots must distort, never flip to full negative.
        let wav = encode_wav(&[5.0, -5.0]);
        let a = i16::from_le_bytes(wav[44..46].try_into().unwrap());
        let b = i16::from_le_bytes(wav[46..48].try_into().unwrap());
        assert!(a > 32_000);
        assert!(b < -32_000);
    }

    #[test]
    fn every_voice_produces_audible_bounded_samples() {
        let mut rng = Rng::new(1);
        let voices: Vec<(&str, Vec<f32>)> = vec![
            ("cannon", synth_cannon()),
            ("bomb", synth_bomb_drop()),
            ("hit", synth_hit()),
            ("small", synth_explosion(&mut rng, 0.4, 900.0, 0.9)),
            ("large", synth_explosion(&mut rng, 0.9, 420.0, 1.0)),
            ("sam", synth_sam_launch(&mut rng)),
            ("radar", synth_radar_down()),
            ("death", synth_player_death(&mut rng)),
            ("overheat", synth_overheat()),
            ("blip", synth_blip(660.0, 0.05)),
            ("engine", synth_engine()),
        ];
        for (name, pcm) in voices {
            assert!(!pcm.is_empty(), "{name} generated no samples");
            let peak = pcm.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!(peak > 0.05, "{name} is inaudible (peak {peak})");
            assert!(pcm.iter().all(|s| s.is_finite()), "{name} produced NaN/inf");
        }
    }

    #[test]
    fn one_shot_voices_start_and_end_near_silence() {
        // Anything that does not fade in and out pops in the mixer.
        let pcm = synth_cannon();
        assert!(pcm[0].abs() < 0.05);
        assert!(pcm[pcm.len() - 1].abs() < 0.05);
    }

    #[test]
    fn the_engine_loop_joins_up() {
        // The last sample must be close to the first, or the loop clicks once
        // every half second for the whole game.
        let pcm = synth_engine();
        let seam = (pcm[pcm.len() - 1] - pcm[0]).abs();
        assert!(seam < 0.05, "engine loop seam discontinuity of {seam}");
    }
}
