# Architecture

How the code is put together, and why it is put together that way. About 8 700
lines of Rust across eighteen modules — a shade under 6 900 of production code
and 1 900 of tests sitting next to it, plus another 400 in `tests/mission.rs`.
One external dependency, `macroquad`, pulled in with default features off.

---

## The shape of it

```
                     ┌──────────┐
                     │  main.rs │  window, frame loop
                     └────┬─────┘
                          │
                     ┌────▼─────┐
                     │  app.rs  │  screens: menu, briefing, playing,
                     └────┬─────┘  paused, run-over, editor
              ┌───────────┼───────────┬──────────────┐
              │           │           │              │
        ┌─────▼────┐ ┌────▼─────┐ ┌───▼────┐   ┌─────▼─────┐
        │ world.rs │ │editor.rs │ │ hud.rs │   │  audio.rs │
        │   the    │ │landscape │ │        │   │ synthesis │
        │simulation│ │  editor  │ └───┬────┘   └───────────┘
        └─────┬────┘ └────┬─────┘     │
              │           │           │
       ┌──────┼───────┐   └───────┬───┘
       │      │       │           │
  ┌────▼───┐┌─▼────┐┌─▼──────┐ ┌──▼───────┐
  │player  ││enemy ││project.│ │ render.rs│  virtual canvas,
  └────┬───┘└─┬────┘└─┬──────┘ └──┬───────┘  neon primitives, CRT
       │      │       │           │
       └──────┴───┬───┴───────────┘
                  │
        ┌─────────▼──────────┐
        │ level.rs           │  campaign, spawns, .pen format
        │   └── terrain.rs   │  heightmaps, generation, collision
        └─────────┬──────────┘
                  │
      ┌───────────┼───────────┬──────────┐
  ┌───▼────┐ ┌────▼───┐  ┌────▼───┐ ┌────▼───┐
  │config  │ │ rng.rs │  │ util   │ │ theme  │
  └────────┘ └────────┘  └────────┘ └────────┘
```

Dependencies point downward only. `render` reads the world and never writes to
it; `world` knows nothing about drawing beyond which colour to tint an
explosion.

---

## The five decisions that shaped everything else

### 1. One fixed virtual canvas

Everything — gameplay, HUD layout, the editor, collision — happens in a 480×270
coordinate space. `render::Viewport` is the only place in the codebase that
knows about real pixels, and all it does is letterbox that canvas onto whatever
window exists.

The payoff is that resolution stops being a concern anywhere else. A HUD element
at `x = 396` is at `x = 396` on a 4K display and on a laptop, and the game looks
identical at both. The cost is a black bar on non-16:9 windows, which is the
right trade for a game that is entirely thin diagonal lines.

### 2. The cave is two arrays of numbers

`terrain::Terrain` is a `ceiling: Vec<f32>` and a `floor: Vec<f32>`, sampled
every 8 world units and linearly interpolated in between. That single
representation makes four separate problems easy:

* **Collision** is an interpolated lookup and a comparison.
* **Rendering** is a triangle strip and a polyline over the same data.
* **The editor's brush** writes directly into the arrays.
* **Serialisation** is two lists of numbers.

The linear interpolation between columns is also what gives the cave its
angular, faceted silhouette, which is the look the original had for entirely
different reasons.

### 3. The whole mission is one continuous cave

`level::Track` welds all four zones *and* the bunker into a single terrain array.
Zones survive only as **spans** — `ZoneSpan { start_col, end_col, .. }` — into
that array.

This means the simulation never handles a zone transition. There is no loading,
no stitching, no state machine for "which zone am I in": the ship flies along one
long cave and the zone is a lookup by x. Checkpoints, palette blending across
boundaries, and the progress bar all fall out of the same spans.

### 4. Enemies stream; placements persist

Every emplacement exists permanently as a `level::Spawn` — a kind and a
position, nothing else. `World::stream_enemies` turns one into a live
`enemy::Enemy` when the camera gets within 120 units of the screen edge, and
drops it again 260 units behind.

Per-frame cost is therefore proportional to what is on screen rather than to the
length of the level, which matters for a 14 000-unit track. But the real payoff
was unplanned: the egress leg's "they rebuilt it while you were gone" effect
comes for free. Flipping direction clears the live list, resets the `destroyed`
flags for everything except radar, and lets it all stream back in.

A parallel `Vec<SpawnState>` carries `{ live, destroyed }` per placement.
`live` means "currently instantiated"; `destroyed` means "killed, do not bring
back". Retiring an enemy for distance clears the first and leaves the second
alone — which is exactly the distinction between "off screen" and "dead".

### 5. Direction is a `f32`, not a branch

The mission is flown out and then back. Rather than duplicating logic for the
two legs, everything with a handedness takes a `dir` of `+1.0` or `-1.0`:

```rust
pub fn screen_x_for(travel_screen: f32, dir: f32) -> f32 {
    if dir >= 0.0 { travel_screen } else { VIRTUAL_W - travel_screen }
}
```

The player's throttle band, the cannon's muzzle, the hull's collision points, the
enemy engagement envelope and the checkpoint rule are all written once and work
in both directions. `Phase::facing()` supplies the number.

---

## The simulation

`world::World::update` runs one step. The order is not arbitrary:

```
 1. slow-motion scaling and banner timers
 2. camera        — advance, apply the phase's limits
 3. ship          — physics, then clamp to the playfield
 4. weapons       — turn the ship's output into projectiles
 5. streaming     — bring nearby placements to life, retire distant ones
 6. enemy AI      — each returns an Action; the world turns those into projectiles
 7. projectiles   — advance, expire
 8. interception  — cannon rounds against incoming ordnance
 9. friendly fire — player ordnance against enemies and rock
10. hazards       — everything against the ship
11. transitions   — phase changes, zone announcements
12. effects       — particles, shake
```

Three things are worth pointing out.

**Interception runs before friendly fire.** A cannon round that swats a missile
should not also hit the silo behind it.

**Rock is checked before enemies.** A round already buried in a wall should not
also hit the emplacement standing on that wall.

**Enemies return `Action`, not side effects.** `Enemy::update` says
`Action::LaunchSam { from, homing }` and the world decides what that becomes.
That keeps the borrow checker out of the way — an enemy never needs `&mut World`
— and it makes AI testable with no world to put it in. Every enemy test in
`enemy.rs` constructs one enemy and a `Senses` struct and asserts on what comes
back.

### Phases

```rust
enum Phase { Outbound, BunkerHold, Egress }
```

`BunkerHold` exists because the warhead chamber needs the camera to stop while
the ship stays free. Giving it a name rather than special-casing `scroll = 0`
made the rest of the code honest about it: `scroll_dir()` returns `0.0` and
`facing()` returns `1.0`, and both are used.

### The frame-time clamp

`config::MAX_DT` is 1/30 of a second, applied at the top of `World::update`. A
stalled frame — dragging the window, hitting a breakpoint — cannot teleport the
ship through a wall. There is a test that feeds it a ten-second frame and checks
the ship is still where it should be.

---

## Determinism

`rng::Rng` is a hand-rolled xorshift64\*. The game deliberately does not use
`macroquad::rand`.

The reason is that a seed has to mean something. It is printed on the debug
overlay, written into saved `.pen` files, and offered as "fly the same cave
again" on the menu — all of which require that the same number produces the same
cave on every machine and every run. A generator the project owns and can
unit-test is the only way to promise that.

The only entropy in the whole codebase is one call to the wall clock in
`app::fresh_seed`, which picks the number. Everything downstream is a pure
function of it.

---

## The playability guarantee

This is the part of the design that took the most work, and the part that most
of the tests exist to defend.

A procedural cave generator can very easily produce terrain that looks
reasonable and cannot be flown. There are **two** ways it happens, and the second
is much less obvious than the first.

### The cave must be wide enough

`Terrain::enforce_min_gap` widens any column narrower than the zone's minimum by
pushing both surfaces apart around their midpoint, giving the whole deficit to
one surface if the other runs out of screen. If the requested gap physically
cannot fit, it opens the cave as far as the world allows and stops — callers get
"as playable as possible", never a panic.

### The cave must not be *steeper* than the ship can climb

A stalagmite that rises 44 units over two columns leaves plenty of room above it
and is still an unavoidable death: at scroll speed there is no input that climbs
that fast. The gap is fine. Getting to the gap is impossible.

`Terrain::limit_slope` caps how far either surface may move between adjacent
columns, at `SLOPE_SAFETY` (72%) of what the ship could just barely manage at the
speed that section will be flown — which for the campaign means the *egress*
speed, since it is flown both ways.

The implementation is a forward-then-backward min-convolution with a linear cone
— the standard one-dimensional distance transform. That gets two properties that
a naive local clamp does not:

* the slope bound holds **everywhere**, not just where the pass happened to
  touch; and
* the ceiling only ever moves up and the floor only ever moves down, so the pass
  can **widen** the cave but never narrow it.

The second property is what makes the ordering safe. Run the gap pass first and
the slope pass second, and the slope pass cannot undo the gap guarantee.

Both invariants are asserted over hundreds of seeds in `terrain.rs` and
`level.rs`, and the shipped `levels/example.pen` is checked against the same two
rules so the documentation cannot drift from the code.

---

## Rendering

There is no shader. The neon look is built from ordinary primitives drawn two or
three times at decreasing width and increasing alpha:

```rust
pub fn glow_line(&self, a: Vec2, b: Vec2, width: f32, color: Color) {
    draw_line(a.x, a.y, b.x, b.y, width * 4.5, fade(color, 0.07));
    draw_line(a.x, a.y, b.x, b.y, width * 2.2, fade(color, 0.20));
    draw_line(a.x, a.y, b.x, b.y, width, color);
}
```

Cheap, portable to any backend macroquad supports, and — usefully — it degrades
to a plain vector look rather than to nothing on a machine that cannot keep up.

Everything is drawn straight to the screen through a `Camera2D` with a viewport,
rather than through an offscreen render target. Render targets in macroquad
involve a y-flip on the blit; a viewport does not, and the visual result is the
same.

**Text** is rasterised at the size it will actually occupy on screen and then
scaled back into virtual units, so it stays sharp at any window size instead of
being a blurry upscale of a 6-pixel glyph.

**Palettes** live in `theme.rs`, one per zone, and `render::blend_palette`
cross-fades them over 220 units either side of a zone boundary so the colour
scheme changes over most of a screen rather than in a single frame. Zones are
distinguished by hue rather than by brightness, so no zone is easier to read than
another — there is a test that checks every palette's edge colour is bright
enough to glow and every fill colour dark enough to sit behind it.

---

## Audio

The game ships no audio files. Every effect is synthesised into an in-memory WAV
buffer at startup and handed to the mixer.

That keeps the whole game a single binary, and it makes the *design* of each
sound readable source code rather than an opaque blob — the explosion is
low-passed noise with a sub-bass thump and a cutoff that sweeps closed, and you
can read that in `synth_explosion`.

Two details that matter:

* **The engine drone has to loop seamlessly.** The buffer is 0.5 s long, so any
  partial at an even number of hertz completes a whole number of cycles across
  it and meets itself at the loop point. 120, 90 and 60 Hz all qualify. Pick an
  odd frequency and the game clicks twice a second forever. There is a test.
* **Every slot is `Option<Sound>`.** A machine with no working audio device is a
  perfectly reasonable machine to play a game on. If the mixer rejects
  everything, `Audio::load` returns a silent bank and every call becomes a no-op.

Building with `--no-default-features` compiles macroquad's audio backend out
entirely, which is how the tests run and how a Linux box without ALSA headers
builds.

---

## Testing

159 tests in three layers.

### Unit tests

Next to the code they cover. These check pieces in isolation: that the RNG is
reproducible and does not jam on a zero seed, that `turn_toward` takes the short
way round the circle, that a missile cannot turn faster than its stated rate,
that `damp` is frame-rate independent, that the `.pen` parser rejects a typo and
reports the line number.

### Invariant tests

These check the promises the generator makes, across many seeds at once:

```rust
#[test]
fn every_generated_zone_is_flyable() {
    let nasty = ZoneShape { spike_rate: 0.25, spike_height: 90.0, .. };
    for seed in 0..200u64 {
        let t = generate_zone(&mut Rng::new(seed), &nasty);
        assert!(t.tightest_gap() >= nasty.min_gap - 0.01);
    }
}
```

Note the deliberately vicious shape. The point is not to check the generator's
normal output; it is to check that the playability pass holds when the generator
is doing its worst.

### Mission tests

`tests/mission.rs` contains an autopilot — a controller that reads the same
`World` the renderer does and produces the same `input::Frame` a keyboard would —
and flies complete missions from the cave mouth to the warhead and back out,
across eight seeds.

This is why the crate is split into a library and a thin binary. `World::update`
takes a `Frame` and a delta time and needs no window, so a whole mission can be
simulated headlessly in about a tenth of a second.

The autopilot is deliberately mediocre: it steers for the middle of the cave,
breaks from guided missiles, shoots forward, and bombs the warhead when it is
underneath. It dies twenty to sixty times a run. If a mission can only be
finished by a human playing well, the test fails.

It earned its keep immediately. It found:

* **Bombs drifting past a stationary target.** Ordnance inherited the *zone's
  nominal scroll speed* rather than the ship's actual velocity. In the parked
  bunker chamber the ship is not moving, but bombs still flew forward at 96
  units per second and landed past the warhead. The chamber was unwinnable.
* **An escape condition the ship could not reach.** Completion keyed off the
  ship's world x while the camera stopped at zero — putting the finish line
  beyond the back of the throttle band. The mission could be flown perfectly and
  never end.

Neither is visible in any unit test, and both are fatal.

The same harness drove the balance work described in
[DESIGN-NOTES.md](DESIGN-NOTES.md).

---

## Where things live

| Module | Lines | Owns |
|---|---|---|
| `world.rs` | 1344 | The simulation |
| `level.rs` | 1093 | Campaign, spawns, the `.pen` format |
| `render.rs` | 793 | Virtual canvas, neon primitives, CRT |
| `editor.rs` | 639 | The landscape editor |
| `terrain.rs` | 637 | The cave: heightmaps, generation, collision, sculpting |
| `app.rs` | 571 | Screens and the frame loop |
| `audio.rs` | 547 | Synthesised sound |
| `enemy.rs` | 509 | Enemy behaviour |
| `player.rs` | 439 | Flight model and weapons |
| `fx.rs` | 426 | Particles, shockwaves, screen shake |
| `projectile.rs` | 338 | Everything in flight |
| `hud.rs` | 327 | Head-up display and overlays |
| `save.rs` | 243 | High scores, settings, level files |
| `config.rs` | 234 | Every tunable number, and nothing else |
| `util.rs` | 158 | Numeric helpers |
| `rng.rs` | 139 | Deterministic seeded generator |
| `theme.rs` | 132 | Palettes and fixed colours |
| `input.rs` | 78 | Keyboard to abstract actions |

### The config rule

Gameplay modules import constants from `config.rs` and never define magic numbers
of their own. If a value affects how the game *feels*, it lives there and can be
found and tweaked without reading the simulation.

The value of this became obvious during balance work. Every change in
DESIGN-NOTES.md — missile guidance burnout, the engagement envelope, the launch
telegraph, the simultaneous-missile cap — was a constant plus a few lines, and
the reasoning for each is documented at its definition rather than buried at its
use site.
