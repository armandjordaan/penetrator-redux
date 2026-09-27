//! The cave.
//!
//! Terrain is two parallel height arrays sampled every [`COLUMN_W`] world units:
//! `ceiling[i]` is the y coordinate of the underside of the rock above, and
//! `floor[i]` is the y coordinate of the top of the rock below. Everything
//! between them is flyable air.
//!
//! Storing the cave as heights rather than as polygons keeps three things cheap
//! that would otherwise be fiddly: collision is a single interpolated lookup, the
//! editor can sculpt with a brush that just writes into the arrays, and the whole
//! level serialises to two lists of numbers.
//!
//! Terrain is generated from fractal value noise and then passed through a
//! *playability pass* that guarantees a minimum vertical gap everywhere. The
//! generator is allowed to be as vicious as it likes; the playability pass is what
//! promises the result can actually be flown.

use crate::config::*;
use crate::rng::Rng;
use crate::util::{lerp, smoothstep};

/// A cave section: matched ceiling and floor height arrays.
#[derive(Clone, Debug, Default)]
pub struct Terrain {
    /// Absolute y of the ceiling surface, per column. Larger = ceiling hangs lower.
    pub ceiling: Vec<f32>,
    /// Absolute y of the floor surface, per column. Smaller = floor stands taller.
    pub floor: Vec<f32>,
}

impl Terrain {
    /// An empty cave of `columns` columns with the widest legal gap.
    pub fn flat(columns: usize) -> Self {
        Terrain {
            ceiling: vec![CEILING_MIN; columns],
            floor: vec![FLOOR_MAX; columns],
        }
    }

    pub fn columns(&self) -> usize {
        self.ceiling.len().min(self.floor.len())
    }

    pub fn is_empty(&self) -> bool {
        self.columns() == 0
    }

    /// Total world width. The last column sits exactly on this boundary.
    pub fn world_len(&self) -> f32 {
        (self.columns().saturating_sub(1)) as f32 * COLUMN_W
    }

    /// Converts a world x into a column index, clamped to the array.
    #[inline]
    pub fn column_at(&self, x: f32) -> usize {
        if self.is_empty() {
            return 0;
        }
        ((x / COLUMN_W).floor().max(0.0) as usize).min(self.columns() - 1)
    }

    /// Samples a height array at an arbitrary world x, interpolating between the
    /// two bracketing columns. Out-of-range x clamps to the nearest end rather
    /// than wrapping, so the ship meets a solid edge instead of teleporting.
    #[inline]
    fn sample(data: &[f32], x: f32) -> f32 {
        if data.is_empty() {
            return 0.0;
        }
        let t = x / COLUMN_W;
        if t <= 0.0 {
            return data[0];
        }
        let last = data.len() - 1;
        if t >= last as f32 {
            return data[last];
        }
        let i = t.floor() as usize;
        lerp(data[i], data[i + 1], t - i as f32)
    }

    /// Y of the ceiling surface at world x.
    #[inline]
    pub fn ceiling_at(&self, x: f32) -> f32 {
        Self::sample(&self.ceiling, x)
    }

    /// Y of the floor surface at world x.
    #[inline]
    pub fn floor_at(&self, x: f32) -> f32 {
        Self::sample(&self.floor, x)
    }

    /// The open vertical span at world x as `(top, bottom)`.
    #[inline]
    pub fn gap_at(&self, x: f32) -> (f32, f32) {
        (self.ceiling_at(x), self.floor_at(x))
    }

    /// Height of flyable air at world x. Negative where the cave is pinched shut.
    #[inline]
    pub fn gap_height(&self, x: f32) -> f32 {
        self.floor_at(x) - self.ceiling_at(x)
    }

    /// True if the point is inside rock.
    #[inline]
    pub fn solid_at(&self, x: f32, y: f32) -> bool {
        y <= self.ceiling_at(x) || y >= self.floor_at(x)
    }

    /// The vertical centre line of the cave at world x — where the AI aims and
    /// where the ship is placed on respawn.
    #[inline]
    pub fn centre_at(&self, x: f32) -> f32 {
        let (top, bottom) = self.gap_at(x);
        (top + bottom) * 0.5
    }

    /// Appends another section onto the end of this one. Used to weld the four
    /// zones and the bunker into a single continuous track.
    pub fn append(&mut self, other: &Terrain) {
        self.ceiling.extend_from_slice(&other.ceiling);
        self.floor.extend_from_slice(&other.floor);
    }

    /// Sculpts one surface with a soft circular brush, for the level editor.
    ///
    /// `delta` is positive to grow rock into the cave and negative to cut it away.
    /// Falloff is smoothstepped from the brush centre so hand-drawn terrain still
    /// looks like the generated stuff.
    pub fn sculpt(&mut self, surface: Surface, world_x: f32, radius: f32, delta: f32) {
        if self.is_empty() || radius <= 0.0 {
            return;
        }
        let columns = self.columns();
        let centre = world_x / COLUMN_W;
        let span = (radius / COLUMN_W).ceil() as i32;
        let lo = ((centre as i32) - span).max(0);
        let hi = ((centre as i32) + span).min(columns as i32 - 1);

        for i in lo..=hi {
            let dist = ((i as f32) - centre).abs() * COLUMN_W;
            if dist > radius {
                continue;
            }
            let falloff = smoothstep(1.0 - dist / radius);
            let idx = i as usize;
            match surface {
                // Growing the ceiling means pushing it *down* the screen.
                Surface::Ceiling => {
                    self.ceiling[idx] =
                        (self.ceiling[idx] + delta * falloff).clamp(CEILING_MIN, CEILING_MAX);
                }
                // Growing the floor means pushing it *up* the screen, hence the minus.
                Surface::Floor => {
                    self.floor[idx] =
                        (self.floor[idx] - delta * falloff).clamp(FLOOR_MIN, FLOOR_MAX);
                }
            }
        }
    }

    /// Widens the cave wherever it is narrower than `min_gap`, by pushing both
    /// surfaces apart around their shared midpoint.
    ///
    /// If the requested gap cannot fit between [`CEILING_MIN`] and [`FLOOR_MAX`]
    /// the cave is opened as far as the world allows and left at that. Callers
    /// therefore get "as playable as physically possible", never a panic.
    pub fn enforce_min_gap(&mut self, min_gap: f32) {
        let columns = self.columns();
        let world_gap = FLOOR_MAX - CEILING_MIN;
        let target = min_gap.min(world_gap);

        for i in 0..columns {
            let (mut top, mut bottom) = (self.ceiling[i], self.floor[i]);
            let gap = bottom - top;
            if gap >= target {
                continue;
            }
            let need = (target - gap) * 0.5;
            top -= need;
            bottom += need;

            // Pushing symmetrically can run one surface off the end of the world;
            // when it does, give the whole deficit to the other surface.
            if top < CEILING_MIN {
                bottom += CEILING_MIN - top;
                top = CEILING_MIN;
            }
            if bottom > FLOOR_MAX {
                top -= bottom - FLOOR_MAX;
                bottom = FLOOR_MAX;
            }

            self.ceiling[i] = top.clamp(CEILING_MIN, CEILING_MAX);
            self.floor[i] = bottom.clamp(FLOOR_MIN, FLOOR_MAX);
        }
    }

    /// Caps how far either surface may move between adjacent columns.
    ///
    /// This is the second half of the playability guarantee, and the less
    /// obvious half. [`Terrain::enforce_min_gap`] promises the cave is wide
    /// enough to fit through; this promises the ship can *get* to where the gap
    /// is. A stalagmite that rises 44 units over two columns leaves plenty of
    /// room above it and is still an unavoidable death, because at scroll speed
    /// there is no input that climbs that fast.
    ///
    /// The implementation is a forward-then-backward min-convolution with a
    /// linear cone — the standard one-dimensional distance transform. The result
    /// is the largest `max_step`-Lipschitz function lying under the original
    /// ceiling (and the smallest lying over the original floor), which gives two
    /// properties for free:
    ///
    /// * the slope bound holds everywhere, not just where the pass touched, and
    /// * the ceiling only ever moves up and the floor only ever moves down, so
    ///   this can widen the cave but never narrow it — which is why it is safe
    ///   to run *after* the minimum-gap pass without undoing it.
    pub fn limit_slope(&mut self, max_step: f32) {
        let n = self.columns();
        if n < 2 || max_step <= 0.0 {
            return;
        }
        for i in 1..n {
            self.ceiling[i] = self.ceiling[i].min(self.ceiling[i - 1] + max_step);
            self.floor[i] = self.floor[i].max(self.floor[i - 1] - max_step);
        }
        for i in (0..n - 1).rev() {
            self.ceiling[i] = self.ceiling[i].min(self.ceiling[i + 1] + max_step);
            self.floor[i] = self.floor[i].max(self.floor[i + 1] - max_step);
        }
    }

    /// The steepest per-column change anywhere on either surface.
    pub fn steepest_step(&self) -> f32 {
        let n = self.columns();
        (1..n).fold(0.0f32, |worst, i| {
            worst
                .max((self.ceiling[i] - self.ceiling[i - 1]).abs())
                .max((self.floor[i] - self.floor[i - 1]).abs())
        })
    }

    /// Forces the first `n` columns wide open. Respawns happen at zone starts, and
    /// materialising inside a stalactite is not a fair death.
    pub fn open_lead(&mut self, n: usize) {
        let columns = self.columns();
        let n = n.min(columns);
        for i in 0..n {
            // Ramp back to the generated shape rather than cutting a hard step.
            let t = smoothstep(i as f32 / n.max(1) as f32);
            self.ceiling[i] = lerp(CEILING_MIN, self.ceiling[i], t);
            self.floor[i] = lerp(FLOOR_MAX, self.floor[i], t);
        }
    }

    /// The narrowest gap anywhere, used by the editor's playability warning.
    pub fn tightest_gap(&self) -> f32 {
        (0..self.columns())
            .map(|i| self.floor[i] - self.ceiling[i])
            .fold(f32::INFINITY, f32::min)
    }

    /// Columns whose gap is below `min_gap` — the editor highlights these in red.
    pub fn impassable_columns(&self, min_gap: f32) -> Vec<usize> {
        (0..self.columns())
            .filter(|&i| self.floor[i] - self.ceiling[i] < min_gap)
            .collect()
    }
}

/// Which of the two cave surfaces an operation applies to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Surface {
    Ceiling,
    Floor,
}

// ---------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------

/// The dials that make one zone feel different from the next.
#[derive(Clone, Debug)]
pub struct ZoneShape {
    pub columns: usize,
    /// Mean thickness of the ceiling rock, in pixels from the top of the screen.
    pub ceiling_base: f32,
    pub ceiling_amp: f32,
    /// Mean thickness of the floor rock, in pixels up from the bottom.
    pub floor_base: f32,
    pub floor_amp: f32,
    /// Width of the largest noise feature, in columns. Small = jagged, large = rolling.
    pub wavelength: f32,
    /// Extra octaves of detail on top of the base wave.
    pub octaves: usize,
    /// Probability per column of starting a spike (stalactite or stalagmite).
    pub spike_rate: f32,
    pub spike_height: f32,
    /// The narrowest the cave is ever allowed to get in this zone.
    pub min_gap: f32,
    /// The steepest per-column surface change allowed. See
    /// [`Terrain::limit_slope`] and [`slope_limit_for`].
    pub max_slope: f32,
}

impl Default for ZoneShape {
    fn default() -> Self {
        ZoneShape {
            columns: 420,
            ceiling_base: 52.0,
            ceiling_amp: 22.0,
            floor_base: 56.0,
            floor_amp: 24.0,
            wavelength: 26.0,
            octaves: 3,
            spike_rate: 0.012,
            spike_height: 30.0,
            min_gap: 76.0,
            max_slope: slope_limit_for(140.0),
        }
    }
}

/// The steepest a cave surface may change per column and still be flyable at
/// `scroll` world units per second.
///
/// One column takes `COLUMN_W / scroll` seconds to cross, in which the ship can
/// move at most `SHIP_MAX_VY` times that. [`SLOPE_SAFETY`] keeps the demand
/// comfortably under what is barely possible.
///
/// Pass the *fastest* speed the section will ever be flown at — which for the
/// campaign means the egress speed, not the outbound one.
pub fn slope_limit_for(scroll: f32) -> f32 {
    (COLUMN_W / scroll.max(1.0)) * SHIP_MAX_VY * SLOPE_SAFETY
}

/// Fractal value noise over a 1-D column range, normalised to roughly `[-1, 1]`.
///
/// Each octave lays down random control points at a halving interval and
/// smoothsteps between them; the octaves are summed with halving amplitude. This
/// is cheaper than gradient noise and, for a cave silhouette, indistinguishable.
fn fbm(rng: &mut Rng, columns: usize, wavelength: f32, octaves: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; columns];
    let mut amplitude = 1.0f32;
    let mut total_amplitude = 0.0f32;
    let mut wl = wavelength.max(2.0);

    for _ in 0..octaves.max(1) {
        // One control point every `wl` columns, plus a spare so the final column
        // always has something to interpolate toward.
        let points = (columns as f32 / wl).ceil() as usize + 2;
        let control: Vec<f32> = (0..points).map(|_| rng.signed()).collect();

        for (i, slot) in out.iter_mut().enumerate() {
            let t = i as f32 / wl;
            let idx = t.floor() as usize;
            let frac = smoothstep(t - idx as f32);
            let a = control[idx.min(points - 1)];
            let b = control[(idx + 1).min(points - 1)];
            *slot += lerp(a, b, frac) * amplitude;
        }

        total_amplitude += amplitude;
        amplitude *= 0.5;
        wl *= 0.5;
    }

    if total_amplitude > 0.0 {
        for v in out.iter_mut() {
            *v /= total_amplitude;
        }
    }
    out
}

/// Adds triangular spikes to a height array. `direction` is +1 to grow the spike
/// into the cave and -1 to bite a notch out of the rock.
fn add_spikes(
    rng: &mut Rng,
    heights: &mut [f32],
    rate: f32,
    max_height: f32,
    direction: f32,
) {
    let columns = heights.len();
    if columns == 0 {
        return;
    }
    let mut i = 0usize;
    while i < columns {
        if rng.chance(rate) {
            let half_width = rng.int(2, 6) as usize;
            let height = rng.range(max_height * 0.45, max_height) * direction;
            let lo = i.saturating_sub(half_width);
            let hi = (i + half_width).min(columns - 1);
            for (offset, h) in heights[lo..=hi].iter_mut().enumerate() {
                let d = ((lo + offset) as i32 - i as i32).abs() as f32 / half_width as f32;
                *h += height * (1.0 - d).max(0.0);
            }
            i += half_width + 1;
        } else {
            i += 1;
        }
    }
}

/// Builds one zone's cave from a shape description and a seed.
///
/// Order matters: noise, then spikes, then the playability pass. Enforcing the
/// minimum gap *last* is what stops a spike from sealing the cave shut.
pub fn generate_zone(rng: &mut Rng, shape: &ZoneShape) -> Terrain {
    let columns = shape.columns.max(2);

    let ceil_noise = fbm(rng, columns, shape.wavelength, shape.octaves);
    let floor_noise = fbm(rng, columns, shape.wavelength * 1.17, shape.octaves);

    let mut ceiling: Vec<f32> = ceil_noise
        .iter()
        .map(|n| shape.ceiling_base + n * shape.ceiling_amp)
        .collect();
    // Stored as absolute y, so a taller floor is a *smaller* number.
    let mut floor: Vec<f32> = floor_noise
        .iter()
        .map(|n| VIRTUAL_H - (shape.floor_base + n * shape.floor_amp))
        .collect();

    add_spikes(rng, &mut ceiling, shape.spike_rate, shape.spike_height, 1.0);
    add_spikes(rng, &mut floor, shape.spike_rate, shape.spike_height, -1.0);

    for v in ceiling.iter_mut() {
        *v = v.clamp(CEILING_MIN, CEILING_MAX);
    }
    for v in floor.iter_mut() {
        *v = v.clamp(FLOOR_MIN, FLOOR_MAX);
    }

    let mut terrain = Terrain { ceiling, floor };
    // Order matters. The gap pass guarantees the cave is wide enough; the slope
    // pass guarantees the ship can reach the wide part. The slope pass only ever
    // widens, so running it second leaves both guarantees standing.
    terrain.enforce_min_gap(shape.min_gap);
    terrain.limit_slope(shape.max_slope);
    terrain
}

/// Builds the bunker: a tightening approach corridor, a chamber where the warhead
/// sits, and a sealed back wall. This section is hand-shaped rather than
/// generated, because it is the one place in the game where the exact geometry is
/// part of the puzzle.
///
/// Returns the terrain and the column index of the chamber centre.
pub fn generate_bunker(columns: usize) -> (Terrain, usize) {
    let columns = columns.max(60);
    let mut ceiling = vec![CEILING_MIN; columns];
    let mut floor = vec![FLOOR_MAX; columns];

    let approach_end = columns * 55 / 100;
    let chamber_end = columns * 88 / 100;
    let chamber_centre = (approach_end + chamber_end) / 2;

    for i in 0..columns {
        let (top, bottom) = if i < approach_end {
            // Corridor squeezing from a normal cave down to a slot. 56px is the
            // tightest the game ever gets: about seven ship-heights, which is
            // demanding at bunker scroll speed without being a memory test.
            let t = smoothstep(i as f32 / approach_end as f32);
            let gap = lerp(120.0, 56.0, t);
            let mid = lerp(VIRTUAL_H * 0.52, VIRTUAL_H * 0.58, t);
            (mid - gap * 0.5, mid + gap * 0.5)
        } else if i < chamber_end {
            // The chamber: tall enough to manoeuvre and line up a bomb.
            let mid = VIRTUAL_H * 0.55;
            let gap = 128.0;
            (mid - gap * 0.5, mid + gap * 0.5)
        } else {
            // Back wall. The gap closes to nothing; there is no way through.
            // The denominator counts to the *last* column rather than past it,
            // so the wall actually reaches zero instead of stopping just shy.
            let span = (columns - 1 - chamber_end).max(1) as f32;
            let t = smoothstep((i - chamber_end) as f32 / span);
            let mid = VIRTUAL_H * 0.55;
            let gap = lerp(128.0, 0.0, t);
            (mid - gap * 0.5, mid + gap * 0.5)
        };
        ceiling[i] = top.clamp(CEILING_MIN, CEILING_MAX);
        floor[i] = bottom.clamp(FLOOR_MIN, FLOOR_MAX);
    }

    (Terrain { ceiling, floor }, chamber_centre)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape() -> ZoneShape {
        ZoneShape {
            columns: 300,
            min_gap: 70.0,
            ..Default::default()
        }
    }

    #[test]
    fn generation_is_deterministic_for_a_seed() {
        let a = generate_zone(&mut Rng::new(4242), &shape());
        let b = generate_zone(&mut Rng::new(4242), &shape());
        assert_eq!(a.ceiling, b.ceiling);
        assert_eq!(a.floor, b.floor);
    }

    #[test]
    fn every_generated_zone_is_flyable() {
        // The playability pass is the promise the generator makes. Check it across
        // many seeds and an aggressively spiky shape, which is where it would fail.
        let nasty = ZoneShape {
            spike_rate: 0.25,
            spike_height: 90.0,
            ceiling_amp: 60.0,
            floor_amp: 60.0,
            min_gap: 64.0,
            ..shape()
        };
        for seed in 0..200u64 {
            let t = generate_zone(&mut Rng::new(seed), &nasty);
            assert!(
                t.tightest_gap() >= nasty.min_gap - 0.01,
                "seed {seed} produced a {:.1}px gap, below the {:.1}px minimum",
                t.tightest_gap(),
                nasty.min_gap
            );
        }
    }

    #[test]
    fn terrain_stays_inside_the_screen() {
        for seed in 0..50u64 {
            let t = generate_zone(&mut Rng::new(seed), &shape());
            for i in 0..t.columns() {
                assert!(t.ceiling[i] >= CEILING_MIN - 0.01 && t.ceiling[i] <= CEILING_MAX + 0.01);
                assert!(t.floor[i] >= FLOOR_MIN - 0.01 && t.floor[i] <= FLOOR_MAX + 0.01);
                assert!(t.ceiling[i] < t.floor[i], "cave inverted at column {i}");
            }
        }
    }

    #[test]
    fn min_gap_larger_than_the_world_degrades_gracefully() {
        let mut t = generate_zone(&mut Rng::new(1), &shape());
        t.enforce_min_gap(10_000.0);
        // Cannot deliver the impossible gap, but must still be a valid open cave.
        assert!(t.tightest_gap() > 0.0);
        for i in 0..t.columns() {
            assert!(t.ceiling[i] < t.floor[i]);
        }
    }

    #[test]
    fn sampling_interpolates_between_columns() {
        let t = Terrain {
            ceiling: vec![10.0, 20.0],
            floor: vec![200.0, 200.0],
        };
        assert!((t.ceiling_at(0.0) - 10.0).abs() < 1e-4);
        assert!((t.ceiling_at(COLUMN_W * 0.5) - 15.0).abs() < 1e-4);
        assert!((t.ceiling_at(COLUMN_W) - 20.0).abs() < 1e-4);
    }

    #[test]
    fn sampling_clamps_outside_the_track() {
        let t = Terrain {
            ceiling: vec![10.0, 20.0],
            floor: vec![200.0, 210.0],
        };
        assert_eq!(t.ceiling_at(-500.0), 10.0);
        assert_eq!(t.ceiling_at(50_000.0), 20.0);
        assert_eq!(t.floor_at(-1.0), 200.0);
    }

    #[test]
    fn solid_test_agrees_with_the_surfaces() {
        let t = generate_zone(&mut Rng::new(11), &shape());
        let x = 400.0;
        let (top, bottom) = t.gap_at(x);
        assert!(t.solid_at(x, top - 1.0));
        assert!(!t.solid_at(x, (top + bottom) * 0.5));
        assert!(t.solid_at(x, bottom + 1.0));
    }

    #[test]
    fn the_lead_in_is_opened_up() {
        let mut t = generate_zone(&mut Rng::new(3), &shape());
        t.open_lead(SAFE_LEAD_COLUMNS);
        assert!((t.ceiling[0] - CEILING_MIN).abs() < 0.01);
        assert!((t.floor[0] - FLOOR_MAX).abs() < 0.01);
    }

    #[test]
    fn the_bunker_has_a_chamber_and_a_sealed_back_wall() {
        let (t, chamber) = generate_bunker(90);
        assert!(t.gap_height(chamber as f32 * COLUMN_W) > 100.0);
        let last = (t.columns() - 1) as f32 * COLUMN_W;
        assert!(t.gap_height(last) < 1.0, "the bunker must be a dead end");
    }

    #[test]
    fn appending_welds_two_sections() {
        let mut a = Terrain::flat(10);
        let b = Terrain::flat(5);
        a.append(&b);
        assert_eq!(a.columns(), 15);
    }

    #[test]
    fn sculpting_moves_only_the_targeted_surface() {
        let mut t = Terrain::flat(40);
        let before_floor = t.floor.clone();
        t.sculpt(Surface::Ceiling, 20.0 * COLUMN_W, 24.0, 30.0);
        assert!(t.ceiling[20] > CEILING_MIN);
        assert_eq!(t.floor, before_floor);
        // Falloff means the brush edge is untouched.
        assert!((t.ceiling[0] - CEILING_MIN).abs() < 0.01);
    }

    #[test]
    fn sculpting_cannot_push_surfaces_out_of_the_world() {
        let mut t = Terrain::flat(20);
        for _ in 0..200 {
            t.sculpt(Surface::Ceiling, 10.0 * COLUMN_W, 40.0, 50.0);
            t.sculpt(Surface::Floor, 10.0 * COLUMN_W, 40.0, 50.0);
        }
        assert!(t.ceiling[10] <= CEILING_MAX + 0.01);
        assert!(t.floor[10] >= FLOOR_MIN - 0.01);
    }
}
