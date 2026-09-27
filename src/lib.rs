//! Penetrator Redux — a modern redesign of the 1983 ZX Spectrum cave-flyer.
//!
//! Fly a strike aircraft through four zones of a defended cave system, bomb the
//! warhead at the back of the bunker, and fly out again. The way home is harder.
//!
//! The crate is split into a library and a thin binary so that the simulation can
//! be driven headlessly. [`world::World`] advances from a [`input::Frame`] and a
//! delta time and needs no window, which is what makes the end-to-end mission
//! test in `tests/` possible.
//!
//! # Layout
//!
//! | Module | What it owns |
//! |---|---|
//! | [`config`] | Every tunable number in the game |
//! | [`rng`] | The deterministic generator level generation is built on |
//! | [`terrain`] | The cave: heightmaps, generation, collision, sculpting |
//! | [`level`] | Campaign definition, spawns, and the `.pen` file format |
//! | [`world`] | The simulation: phases, scoring, lives, collision resolution |
//! | [`player`] | Flight model and weapons |
//! | [`enemy`] | Enemy behaviour |
//! | [`projectile`] | Everything in flight |
//! | [`fx`] | Particles, shockwaves, screen shake |
//! | [`render`] | The virtual canvas, the neon look, the CRT filter |
//! | [`hud`] | Head-up display and overlays |
//! | [`theme`] | Palettes and fixed colours |
//! | [`editor`] | The landscape editor |
//! | [`audio`] | Sound effects, synthesised at startup |
//! | [`save`] | High scores, settings and level files |
//! | [`app`] | Screens and the frame loop |

pub mod app;
pub mod audio;
pub mod config;
pub mod editor;
pub mod enemy;
pub mod fx;
pub mod hud;
pub mod input;
pub mod level;
pub mod player;
pub mod projectile;
pub mod render;
pub mod rng;
pub mod save;
pub mod terrain;
pub mod theme;
pub mod util;
pub mod world;
