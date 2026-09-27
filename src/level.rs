//! Levels: the campaign definition, the generator that turns it into a playable
//! track, and the `.pen` text format the editor reads and writes.
//!
//! A [`Track`] is one continuous cave — all four zones plus the bunker welded end
//! to end into a single terrain array. Zones survive only as *spans* into that
//! array. Doing it this way means the simulation never has to think about zone
//! boundaries: the ship just flies along one long cave, and "which zone am I in"
//! is a lookup, not a state transition.

use crate::config::*;
use crate::rng::Rng;
use crate::terrain::{generate_bunker, generate_zone, slope_limit_for, Terrain, ZoneShape};
use std::fmt;

// ---------------------------------------------------------------------------
// Spawns
// ---------------------------------------------------------------------------

/// What can be placed in a level. The simulation turns these into live
/// `Enemy` values as the camera approaches them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpawnKind {
    /// Floor-mounted SAM launcher. The backbone of the opposition.
    Silo,
    /// Floor-mounted radar dish. While any dish stands, every SAM homes.
    Radar,
    /// Ceiling-mounted gun that leads its shots.
    Turret,
    /// Free-floating mine. Drifts, does not shoot, ends your run if you touch it.
    Mine,
    /// Interceptor that only scrambles on the way home.
    Drone,
    /// The mission objective, at the back of the bunker.
    Warhead,
}

impl SpawnKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SpawnKind::Silo => "silo",
            SpawnKind::Radar => "radar",
            SpawnKind::Turret => "turret",
            SpawnKind::Mine => "mine",
            SpawnKind::Drone => "drone",
            SpawnKind::Warhead => "warhead",
        }
    }

    pub fn parse(s: &str) -> Option<SpawnKind> {
        Some(match s {
            "silo" => SpawnKind::Silo,
            "radar" => SpawnKind::Radar,
            "turret" => SpawnKind::Turret,
            "mine" => SpawnKind::Mine,
            "drone" => SpawnKind::Drone,
            "warhead" => SpawnKind::Warhead,
            _ => return None,
        })
    }

    /// Human-readable name for the editor palette.
    pub fn label(self) -> &'static str {
        match self {
            SpawnKind::Silo => "SAM SILO",
            SpawnKind::Radar => "RADAR",
            SpawnKind::Turret => "TURRET",
            SpawnKind::Mine => "MINE",
            SpawnKind::Drone => "DRONE",
            SpawnKind::Warhead => "WARHEAD",
        }
    }

    /// Where this thing lives, which decides how its y coordinate is resolved.
    pub fn anchor(self) -> Anchor {
        match self {
            SpawnKind::Silo | SpawnKind::Radar | SpawnKind::Warhead => Anchor::Floor,
            SpawnKind::Turret => Anchor::Ceiling,
            SpawnKind::Mine | SpawnKind::Drone => Anchor::Air,
        }
    }

    /// Everything the editor can place, in palette order.
    pub const PALETTE: [SpawnKind; 6] = [
        SpawnKind::Silo,
        SpawnKind::Radar,
        SpawnKind::Turret,
        SpawnKind::Mine,
        SpawnKind::Drone,
        SpawnKind::Warhead,
    ];
}

/// How a spawn's vertical position is determined.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Anchor {
    /// Sits on the floor; y comes from the terrain, so sculpting moves it.
    Floor,
    /// Hangs from the ceiling; y comes from the terrain.
    Ceiling,
    /// Free-floating; y is stored in the level file.
    Air,
}

/// One placed object. `y` is only meaningful for [`Anchor::Air`] kinds.
#[derive(Clone, Copy, Debug)]
pub struct Spawn {
    pub kind: SpawnKind,
    pub x: f32,
    pub y: f32,
}

// ---------------------------------------------------------------------------
// Zones
// ---------------------------------------------------------------------------

/// A named stretch of the track.
#[derive(Clone, Debug)]
pub struct ZoneSpan {
    pub name: String,
    /// First column of this zone within the track's terrain array.
    pub start_col: usize,
    /// One past the last column.
    pub end_col: usize,
    /// Base camera speed while flying this zone, in world units per second.
    pub scroll: f32,
    /// Index into the renderer's palette table.
    pub palette: usize,
}

impl ZoneSpan {
    pub fn start_x(&self) -> f32 {
        self.start_col as f32 * COLUMN_W
    }
    pub fn end_x(&self) -> f32 {
        self.end_col as f32 * COLUMN_W
    }
    pub fn contains_x(&self, x: f32) -> bool {
        x >= self.start_x() && x < self.end_x()
    }
}

// ---------------------------------------------------------------------------
// Track
// ---------------------------------------------------------------------------

/// A complete, playable mission.
#[derive(Clone, Debug, Default)]
pub struct Track {
    pub terrain: Terrain,
    pub zones: Vec<ZoneSpan>,
    pub spawns: Vec<Spawn>,
    /// The seed this track was generated from. Zero for hand-edited levels.
    pub seed: u64,
    /// World x the camera parks at once the ship reaches the bunker.
    pub bunker_hold_x: f32,
}

impl Track {
    pub fn world_len(&self) -> f32 {
        self.terrain.world_len()
    }

    /// Index of the zone containing `x`, clamped to the ends of the track.
    pub fn zone_index_at(&self, x: f32) -> usize {
        if self.zones.is_empty() {
            return 0;
        }
        for (i, z) in self.zones.iter().enumerate() {
            if z.contains_x(x) {
                return i;
            }
        }
        if x < self.zones[0].start_x() {
            0
        } else {
            self.zones.len() - 1
        }
    }

    pub fn zone_at(&self, x: f32) -> &ZoneSpan {
        &self.zones[self.zone_index_at(x)]
    }

    /// World x the ship respawns at after a crash.
    ///
    /// Outbound that is the front of the zone it died in; on the way home it is
    /// the far end of the same zone, which is the same idea approached from the
    /// other side. Either way the ship restarts the stretch that killed it.
    pub fn checkpoint_x(&self, x: f32, dir: f32) -> f32 {
        let z = self.zone_at(x);
        let inset = SAFE_LEAD_COLUMNS as f32 * COLUMN_W * 0.5;
        if dir >= 0.0 {
            z.start_x() + inset
        } else {
            (z.end_x() - inset).max(z.start_x())
        }
    }

    /// The last column that forms part of the route the player must fly.
    ///
    /// Terrain beyond the objective is scenery: the bunker's back wall is sealed
    /// on purpose, and measuring it would mean every level the game ships with
    /// reports itself as broken.
    fn route_end_column(&self) -> usize {
        match self.warhead_x() {
            Some(x) => self.terrain.column_at(x),
            None => self.terrain.columns().saturating_sub(1),
        }
    }

    /// Columns too tight for the ship, on the route the player actually has to
    /// fly — that is, everything up to the warhead.
    pub fn blocked_columns(&self, min_gap: f32) -> Vec<usize> {
        let limit = self.route_end_column();
        self.terrain
            .impassable_columns(min_gap)
            .into_iter()
            .filter(|&c| c <= limit)
            .collect()
    }

    /// The narrowest gap on the route to the objective.
    ///
    /// [`Terrain::tightest_gap`] answers the same question about the whole
    /// track, which for any campaign includes the sealed dead end and is
    /// therefore always zero.
    pub fn tightest_route_gap(&self) -> f32 {
        if self.terrain.is_empty() {
            return 0.0;
        }
        let limit = self.route_end_column().min(self.terrain.columns() - 1);
        (0..=limit)
            .map(|i| self.terrain.floor[i] - self.terrain.ceiling[i])
            .fold(f32::INFINITY, f32::min)
    }

    /// Where the warhead is, if this track has one.
    pub fn warhead_x(&self) -> Option<f32> {
        self.spawns
            .iter()
            .find(|s| s.kind == SpawnKind::Warhead)
            .map(|s| s.x)
    }

    /// Resolves a spawn's world position against the terrain. Floor and ceiling
    /// kinds are re-anchored every time they are asked for, so editing the cave
    /// never leaves a silo hanging in mid-air.
    pub fn spawn_position(&self, spawn: &Spawn) -> (f32, f32) {
        let y = match spawn.kind.anchor() {
            Anchor::Floor => self.terrain.floor_at(spawn.x),
            Anchor::Ceiling => self.terrain.ceiling_at(spawn.x),
            Anchor::Air => spawn.y,
        };
        (spawn.x, y)
    }

    /// Total radar dishes in the track. The HUD shows how many are left.
    pub fn radar_count(&self) -> usize {
        self.spawns
            .iter()
            .filter(|s| s.kind == SpawnKind::Radar)
            .count()
    }
}

// ---------------------------------------------------------------------------
// Campaign definition
// ---------------------------------------------------------------------------

/// How thickly each kind of defence is sown through a zone. Values are
/// per-column probabilities.
#[derive(Clone, Copy, Debug)]
pub struct Density {
    pub silo: f32,
    pub radar: f32,
    pub turret: f32,
    pub mine: f32,
}

/// The authored description of one zone, before generation turns it into terrain.
pub struct ZoneDef {
    pub name: &'static str,
    pub scroll: f32,
    pub palette: usize,
    pub density: Density,
    pub shape: ZoneShape,
}

/// The four-zone approach flown in the original, retuned for this build.
///
/// The escalation is deliberate and reads bottom-to-top on this list: the cave
/// gets tighter and rougher, the scroll gets faster, and the defences get denser.
/// Zone 1 is a tutorial you are not told is a tutorial.
pub fn campaign_zones() -> Vec<ZoneDef> {
    vec![
        ZoneDef {
            name: "APPROACH",
            scroll: 112.0,
            palette: 0,
            density: Density {
                silo: 0.020,
                radar: 0.005,
                turret: 0.0,
                mine: 0.004,
            },
            shape: ZoneShape {
                columns: 380,
                ceiling_base: 46.0,
                ceiling_amp: 16.0,
                floor_base: 48.0,
                floor_amp: 18.0,
                wavelength: 32.0,
                octaves: 2,
                spike_rate: 0.006,
                spike_height: 22.0,
                min_gap: 96.0,
                max_slope: slope_limit_for(112.0 * EGRESS_SPEED_BONUS),
            },
        },
        ZoneDef {
            name: "RAVINE",
            scroll: 126.0,
            palette: 1,
            density: Density {
                silo: 0.028,
                radar: 0.006,
                turret: 0.008,
                mine: 0.008,
            },
            shape: ZoneShape {
                columns: 420,
                ceiling_base: 54.0,
                ceiling_amp: 24.0,
                floor_base: 58.0,
                floor_amp: 26.0,
                wavelength: 26.0,
                octaves: 3,
                spike_rate: 0.014,
                spike_height: 30.0,
                min_gap: 82.0,
                max_slope: slope_limit_for(126.0 * EGRESS_SPEED_BONUS),
            },
        },
        ZoneDef {
            name: "THE TEETH",
            scroll: 138.0,
            palette: 2,
            density: Density {
                silo: 0.030,
                radar: 0.006,
                turret: 0.014,
                mine: 0.012,
            },
            shape: ZoneShape {
                columns: 440,
                ceiling_base: 64.0,
                ceiling_amp: 30.0,
                floor_base: 66.0,
                floor_amp: 30.0,
                wavelength: 18.0,
                octaves: 3,
                spike_rate: 0.030,
                spike_height: 44.0,
                min_gap: 70.0,
                max_slope: slope_limit_for(138.0 * EGRESS_SPEED_BONUS),
            },
        },
        ZoneDef {
            name: "DEEP CUT",
            scroll: 150.0,
            palette: 3,
            density: Density {
                silo: 0.034,
                radar: 0.007,
                turret: 0.018,
                mine: 0.016,
            },
            shape: ZoneShape {
                columns: 460,
                ceiling_base: 72.0,
                ceiling_amp: 34.0,
                floor_base: 74.0,
                floor_amp: 34.0,
                wavelength: 14.0,
                octaves: 4,
                spike_rate: 0.038,
                spike_height: 52.0,
                min_gap: 62.0,
                max_slope: slope_limit_for(150.0 * EGRESS_SPEED_BONUS),
            },
        },
    ]
}

/// Generates the whole campaign track from a single seed.
pub fn generate_campaign(seed: u64) -> Track {
    let defs = campaign_zones();
    let mut terrain_rng = Rng::new(seed);
    let mut spawn_rng = Rng::new(seed).fork(0xDEFEA7);

    let mut terrain = Terrain::default();
    let mut zones: Vec<ZoneSpan> = Vec::new();
    let mut spawns: Vec<Spawn> = Vec::new();

    for def in &defs {
        let mut zone_terrain = generate_zone(&mut terrain_rng, &def.shape);
        // Every zone is a checkpoint, so every zone gets a survivable entrance.
        zone_terrain.open_lead(SAFE_LEAD_COLUMNS);

        let start_col = terrain.columns();
        terrain.append(&zone_terrain);
        let end_col = terrain.columns();

        zones.push(ZoneSpan {
            name: def.name.to_string(),
            start_col,
            end_col,
            scroll: def.scroll,
            palette: def.palette,
        });

        place_defences(
            &mut spawn_rng,
            &zone_terrain,
            start_col,
            &def.density,
            def.shape.min_gap,
            &mut spawns,
        );
    }

    // The bunker is welded onto the end of the last zone.
    let (bunker, chamber_col) = generate_bunker(96);
    let bunker_start = terrain.columns();
    terrain.append(&bunker);
    let bunker_end = terrain.columns();

    zones.push(ZoneSpan {
        name: "BUNKER".to_string(),
        start_col: bunker_start,
        end_col: bunker_end,
        scroll: 96.0,
        palette: 4,
        });

    let chamber_x = (bunker_start + chamber_col) as f32 * COLUMN_W;
    spawns.push(Spawn {
        kind: SpawnKind::Warhead,
        // Seated a little past the chamber centre, so you have to commit to the
        // dead end to reach it.
        x: chamber_x + 40.0,
        y: 0.0,
    });

    Track {
        terrain,
        zones,
        spawns,
        seed,
        // Park the camera so the whole chamber is on screen.
        bunker_hold_x: chamber_x - VIRTUAL_W * 0.5 + 60.0,
    }
}

/// Sows one zone with defences.
///
/// Two rules keep the result fair rather than merely random: ground emplacements
/// need reasonably level ground to stand on, and nothing is placed within the
/// opened-up lead-in where the player respawns.
fn place_defences(
    rng: &mut Rng,
    zone: &Terrain,
    start_col: usize,
    density: &Density,
    min_gap: f32,
    out: &mut Vec<Spawn>,
) {
    let columns = zone.columns();
    // Leave a gap after the lead-in, and stop short of the zone's tail so nothing
    // straddles a zone seam.
    let first = SAFE_LEAD_COLUMNS + 6;
    let last = columns.saturating_sub(6);
    // Minimum columns between two emplacements, so they never overlap visually.
    let mut cooldown = 0usize;

    for i in first..last {
        if cooldown > 0 {
            cooldown -= 1;
            continue;
        }
        let world_x = (start_col + i) as f32 * COLUMN_W;
        let local_x = i as f32 * COLUMN_W;

        // Slope of the surface across the object's footprint, in pixels.
        let ground_slope = (zone.floor_at(local_x + 12.0) - zone.floor_at(local_x - 12.0)).abs();
        let roof_slope = (zone.ceiling_at(local_x + 12.0) - zone.ceiling_at(local_x - 12.0)).abs();
        let gap = zone.gap_height(local_x);

        if ground_slope < 9.0 && rng.chance(density.radar) {
            out.push(Spawn {
                kind: SpawnKind::Radar,
                x: world_x,
                y: 0.0,
            });
            cooldown = 14;
        } else if ground_slope < 12.0 && rng.chance(density.silo) {
            out.push(Spawn {
                kind: SpawnKind::Silo,
                x: world_x,
                y: 0.0,
            });
            cooldown = 10;
        } else if roof_slope < 12.0 && rng.chance(density.turret) {
            out.push(Spawn {
                kind: SpawnKind::Turret,
                x: world_x,
                y: 0.0,
            });
            cooldown = 12;
        } else if gap > min_gap + 24.0 && rng.chance(density.mine) {
            // Keep a lane open above and below every mine; a mine you cannot fly
            // past is a wall, and walls should be made of rock.
            let (top, bottom) = zone.gap_at(local_x);
            let margin = MINE_RADIUS + 14.0;
            out.push(Spawn {
                kind: SpawnKind::Mine,
                x: world_x,
                y: rng.range(top + margin, bottom - margin),
            });
            cooldown = 8;
        }
    }
}

// ---------------------------------------------------------------------------
// The .pen file format
// ---------------------------------------------------------------------------

/// Why a level file could not be read. Carries the line number so the editor can
/// tell the player exactly where the problem is.
#[derive(Debug)]
pub struct LevelError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for LevelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for LevelError {}

fn err<T>(line: usize, message: impl Into<String>) -> Result<T, LevelError> {
    Err(LevelError {
        line,
        message: message.into(),
    })
}

/// Format version written into the header. Bumped only on a breaking change.
pub const PEN_VERSION: u32 = 1;

/// Serialises a track to the `.pen` text format.
///
/// The format is line-oriented and deliberately boring: a header, then one block
/// per zone holding that zone's two height arrays and its spawns. Heights are
/// written with one decimal place, which is finer than the sub-pixel the renderer
/// can show, and spawn coordinates are *zone-relative* so blocks can be reordered
/// by hand without recomputing anything.
pub fn write_pen(track: &Track) -> String {
    let mut out = String::new();
    out.push_str(&format!("penetrator-level {PEN_VERSION}\n"));
    out.push_str(&format!("seed {}\n", track.seed));
    out.push_str("# heights are absolute screen y; spawn x/y are zone-relative\n");

    for zone in &track.zones {
        out.push('\n');
        out.push_str(&format!(
            "zone \"{}\" scroll {:.1} palette {}\n",
            zone.name.replace('"', "'"),
            zone.scroll,
            zone.palette
        ));

        write_heights(&mut out, "ceil", &track.terrain.ceiling[zone.start_col..zone.end_col]);
        write_heights(&mut out, "floor", &track.terrain.floor[zone.start_col..zone.end_col]);

        let (lo, hi) = (zone.start_x(), zone.end_x());
        for s in track.spawns.iter().filter(|s| s.x >= lo && s.x < hi) {
            match s.kind.anchor() {
                Anchor::Air => out.push_str(&format!(
                    "spawn {} {:.1} {:.1}\n",
                    s.kind.as_str(),
                    s.x - lo,
                    s.y
                )),
                _ => out.push_str(&format!("spawn {} {:.1}\n", s.kind.as_str(), s.x - lo)),
            }
        }
        out.push_str("end\n");
    }
    out
}

/// Writes a height array wrapped at 20 values per line, so the file stays
/// readable in an editor.
fn write_heights(out: &mut String, tag: &str, values: &[f32]) {
    for chunk in values.chunks(20) {
        out.push_str(tag);
        for v in chunk {
            out.push_str(&format!(" {v:.1}"));
        }
        out.push('\n');
    }
}

/// Parses the `.pen` text format.
///
/// Unknown directives are a hard error rather than a silent skip: a typo in a
/// hand-edited level should be reported, not quietly played.
pub fn parse_pen(text: &str) -> Result<Track, LevelError> {
    let mut lines = text.lines().enumerate();

    // --- header ---
    let mut seed = 0u64;
    let mut saw_magic = false;
    for (n, raw) in lines.by_ref() {
        let line = strip_comment(raw);
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("penetrator-level") => {
                let v: u32 = parts
                    .next()
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| LevelError {
                        line: n + 1,
                        message: "expected a version number after 'penetrator-level'".into(),
                    })?;
                if v > PEN_VERSION {
                    return err(
                        n + 1,
                        format!("level format v{v} is newer than this build understands (v{PEN_VERSION})"),
                    );
                }
                saw_magic = true;
            }
            Some("seed") => {
                seed = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            }
            Some("zone") => {
                // Header is over; re-parse this line inside the body loop below.
                if !saw_magic {
                    return err(n + 1, "missing 'penetrator-level' header line");
                }
                return parse_body(text, n, seed);
            }
            Some(other) => return err(n + 1, format!("unexpected directive '{other}' in header")),
            None => {}
        }
    }
    err(0, "level file contains no zones")
}

/// Parses everything from the first `zone` line onward.
fn parse_body(text: &str, first_zone_line: usize, seed: u64) -> Result<Track, LevelError> {
    let mut terrain = Terrain::default();
    let mut zones: Vec<ZoneSpan> = Vec::new();
    let mut spawns: Vec<Spawn> = Vec::new();

    let mut open: Option<OpenZone> = None;

    for (n, raw) in text.lines().enumerate().skip(first_zone_line) {
        let line_no = n + 1;
        let line = strip_comment(raw);
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let directive = parts.next().unwrap_or("");

        match directive {
            "zone" => {
                if open.is_some() {
                    return err(line_no, "'zone' inside a zone — the previous block is missing its 'end'");
                }
                open = Some(parse_zone_header(line, line_no)?);
            }
            "ceil" | "floor" => {
                let zone = open
                    .as_mut()
                    .ok_or_else(|| LevelError { line: line_no, message: format!("'{directive}' outside a zone block") })?;
                let target = if directive == "ceil" {
                    &mut zone.ceiling
                } else {
                    &mut zone.floor
                };
                for tok in parts {
                    let v: f32 = tok.parse().map_err(|_| LevelError {
                        line: line_no,
                        message: format!("'{tok}' is not a number"),
                    })?;
                    if !v.is_finite() {
                        return err(line_no, format!("'{tok}' is not a finite height"));
                    }
                    target.push(v);
                }
            }
            "spawn" => {
                let zone = open.as_mut().ok_or_else(|| LevelError {
                    line: line_no,
                    message: "'spawn' outside a zone block".into(),
                })?;
                let kind_str = parts.next().ok_or_else(|| LevelError {
                    line: line_no,
                    message: "'spawn' needs a kind".into(),
                })?;
                let kind = SpawnKind::parse(kind_str).ok_or_else(|| LevelError {
                    line: line_no,
                    message: format!("unknown spawn kind '{kind_str}'"),
                })?;
                let x: f32 = parts
                    .next()
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| LevelError {
                        line: line_no,
                        message: "'spawn' needs an x coordinate".into(),
                    })?;
                let y: f32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                if !x.is_finite() || !y.is_finite() {
                    return err(line_no, "spawn coordinates must be finite");
                }
                zone.spawns.push(Spawn { kind, x, y });
            }
            "end" => {
                let zone = open.take().ok_or_else(|| LevelError {
                    line: line_no,
                    message: "'end' without a matching 'zone'".into(),
                })?;
                if zone.ceiling.len() != zone.floor.len() {
                    return err(
                        line_no,
                        format!(
                            "zone \"{}\" has {} ceiling values but {} floor values",
                            zone.name,
                            zone.ceiling.len(),
                            zone.floor.len()
                        ),
                    );
                }
                if zone.ceiling.len() < 2 {
                    return err(line_no, format!("zone \"{}\" needs at least 2 columns", zone.name));
                }

                let start_col = terrain.columns();
                let offset = start_col as f32 * COLUMN_W;
                terrain.ceiling.extend_from_slice(&zone.ceiling);
                terrain.floor.extend_from_slice(&zone.floor);
                let end_col = terrain.columns();

                for mut s in zone.spawns {
                    s.x += offset;
                    spawns.push(s);
                }
                zones.push(ZoneSpan {
                    name: zone.name,
                    start_col,
                    end_col,
                    scroll: zone.scroll,
                    palette: zone.palette,
                });
            }
            other => return err(line_no, format!("unknown directive '{other}'")),
        }
    }

    if let Some(z) = open {
        return err(text.lines().count(), format!("zone \"{}\" is missing its 'end'", z.name));
    }
    if zones.is_empty() {
        return err(0, "level file contains no zones");
    }

    let mut track = Track {
        terrain,
        zones,
        spawns,
        seed,
        bunker_hold_x: 0.0,
    };
    // Hand-edited levels do not record the hold point; derive it from the warhead.
    track.bunker_hold_x = match track.warhead_x() {
        Some(x) => x - VIRTUAL_W * 0.5 - 40.0,
        None => (track.world_len() - VIRTUAL_W).max(0.0),
    };
    Ok(track)
}

/// A zone block being accumulated during parsing.
struct OpenZone {
    name: String,
    scroll: f32,
    palette: usize,
    ceiling: Vec<f32>,
    floor: Vec<f32>,
    spawns: Vec<Spawn>,
}

fn parse_zone_header(line: &str, line_no: usize) -> Result<OpenZone, LevelError> {
    // zone "NAME" scroll 120.0 palette 2
    let (name, rest) = if let Some(open_quote) = line.find('"') {
        let after = &line[open_quote + 1..];
        let close = after.find('"').ok_or_else(|| LevelError {
            line: line_no,
            message: "zone name is missing its closing quote".into(),
        })?;
        (after[..close].to_string(), &after[close + 1..])
    } else {
        return err(line_no, "zone name must be quoted, e.g. zone \"APPROACH\"");
    };

    let mut scroll = 120.0f32;
    let mut palette = 0usize;
    let mut toks = rest.split_whitespace();
    while let Some(key) = toks.next() {
        let value = toks.next().ok_or_else(|| LevelError {
            line: line_no,
            message: format!("'{key}' needs a value"),
        })?;
        match key {
            "scroll" => {
                scroll = value.parse().map_err(|_| LevelError {
                    line: line_no,
                    message: format!("scroll '{value}' is not a number"),
                })?;
                if !(1.0..=1000.0).contains(&scroll) {
                    return err(line_no, "scroll must be between 1 and 1000");
                }
            }
            "palette" => {
                palette = value.parse().map_err(|_| LevelError {
                    line: line_no,
                    message: format!("palette '{value}' is not a number"),
                })?;
            }
            other => return err(line_no, format!("unknown zone attribute '{other}'")),
        }
    }

    Ok(OpenZone {
        name,
        scroll,
        palette,
        ceiling: Vec::new(),
        floor: Vec::new(),
        spawns: Vec::new(),
    })
}

/// Strips a `#` comment and surrounding whitespace.
fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => line[..i].trim(),
        None => line.trim(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn campaign_generation_is_deterministic() {
        let a = generate_campaign(12345);
        let b = generate_campaign(12345);
        assert_eq!(a.terrain.ceiling, b.terrain.ceiling);
        assert_eq!(a.spawns.len(), b.spawns.len());
    }

    #[test]
    fn campaign_has_five_zones_and_one_warhead() {
        let t = generate_campaign(7);
        assert_eq!(t.zones.len(), 5, "four approach zones plus the bunker");
        assert_eq!(t.zones.last().unwrap().name, "BUNKER");
        assert_eq!(
            t.spawns.iter().filter(|s| s.kind == SpawnKind::Warhead).count(),
            1
        );
    }

    #[test]
    fn every_campaign_seed_produces_radar_to_shoot() {
        // The radar mechanic is the game's core decision; a track without dishes
        // would silently remove it.
        for seed in 0..40u64 {
            assert!(
                generate_campaign(seed).radar_count() > 0,
                "seed {seed} generated no radar dishes"
            );
        }
    }

    #[test]
    fn zones_tile_the_track_without_gaps_or_overlaps() {
        let t = generate_campaign(99);
        assert_eq!(t.zones[0].start_col, 0);
        for w in t.zones.windows(2) {
            assert_eq!(w[0].end_col, w[1].start_col);
        }
        assert_eq!(t.zones.last().unwrap().end_col, t.terrain.columns());
    }

    #[test]
    fn zone_lookup_covers_the_whole_track_and_beyond() {
        let t = generate_campaign(5);
        assert_eq!(t.zone_index_at(-1000.0), 0);
        assert_eq!(t.zone_index_at(t.world_len() + 1000.0), t.zones.len() - 1);
        let mid = t.zones[2].start_x() + 10.0;
        assert_eq!(t.zone_index_at(mid), 2);
    }

    #[test]
    fn ground_spawns_sit_on_the_terrain() {
        let t = generate_campaign(21);
        for s in t.spawns.iter().filter(|s| s.kind == SpawnKind::Silo) {
            let (_, y) = t.spawn_position(s);
            assert!((y - t.terrain.floor_at(s.x)).abs() < 0.01);
        }
    }

    #[test]
    fn mines_are_placed_with_a_lane_around_them() {
        let t = generate_campaign(33);
        for s in t.spawns.iter().filter(|s| s.kind == SpawnKind::Mine) {
            let (top, bottom) = t.terrain.gap_at(s.x);
            assert!(s.y > top + MINE_RADIUS, "mine buried in the ceiling");
            assert!(s.y < bottom - MINE_RADIUS, "mine buried in the floor");
        }
    }

    #[test]
    fn write_then_parse_round_trips() {
        let original = generate_campaign(2468);
        let text = write_pen(&original);
        let reparsed = parse_pen(&text).expect("round trip should parse");

        assert_eq!(reparsed.zones.len(), original.zones.len());
        assert_eq!(reparsed.terrain.columns(), original.terrain.columns());
        assert_eq!(reparsed.spawns.len(), original.spawns.len());
        assert_eq!(reparsed.seed, original.seed);

        for i in 0..original.terrain.columns() {
            // One decimal place of precision is what the format promises.
            assert!((reparsed.terrain.ceiling[i] - original.terrain.ceiling[i]).abs() < 0.06);
            assert!((reparsed.terrain.floor[i] - original.terrain.floor[i]).abs() < 0.06);
        }
        for (a, b) in reparsed.spawns.iter().zip(original.spawns.iter()) {
            assert_eq!(a.kind, b.kind);
            assert!((a.x - b.x).abs() < 0.06, "{} vs {}", a.x, b.x);
        }
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let text = "\
penetrator-level 1
# a comment
seed 5

zone \"TEST\" scroll 100 palette 0   # trailing comment
ceil 20 20 20
floor 200 200 200
spawn silo 8
end
";
        let t = parse_pen(text).unwrap();
        assert_eq!(t.zones.len(), 1);
        assert_eq!(t.terrain.columns(), 3);
        assert_eq!(t.spawns.len(), 1);
        assert_eq!(t.seed, 5);
    }

    #[test]
    fn spawn_x_is_zone_relative_on_load() {
        let text = "\
penetrator-level 1
zone \"A\" scroll 100 palette 0
ceil 20 20 20 20
floor 200 200 200 200
end
zone \"B\" scroll 100 palette 0
ceil 20 20 20 20
floor 200 200 200 200
spawn silo 0
end
";
        let t = parse_pen(text).unwrap();
        // Zone B starts at column 4, so its local x=0 is world x = 4 * COLUMN_W.
        assert!((t.spawns[0].x - 4.0 * COLUMN_W).abs() < 0.01);
    }

    #[test]
    fn mismatched_height_arrays_are_rejected() {
        let text = "penetrator-level 1\nzone \"A\" scroll 100 palette 0\nceil 20 20 20\nfloor 200 200\nend\n";
        let e = parse_pen(text).unwrap_err();
        assert!(e.message.contains("ceiling"), "{}", e.message);
    }

    #[test]
    fn unterminated_zone_is_rejected() {
        let text = "penetrator-level 1\nzone \"A\" scroll 100 palette 0\nceil 20 20\nfloor 200 200\n";
        assert!(parse_pen(text).unwrap_err().message.contains("'end'"));
    }

    #[test]
    fn unknown_directive_is_reported_with_a_line_number() {
        let text = "penetrator-level 1\nzone \"A\" scroll 100 palette 0\nwobble 3\nend\n";
        let e = parse_pen(text).unwrap_err();
        assert_eq!(e.line, 3);
        assert!(e.message.contains("wobble"));
    }

    #[test]
    fn missing_header_is_rejected() {
        let text = "zone \"A\" scroll 100 palette 0\nceil 20 20\nfloor 200 200\nend\n";
        assert!(parse_pen(text).is_err());
    }

    #[test]
    fn a_future_format_version_is_refused_rather_than_guessed_at() {
        let text = "penetrator-level 99\nzone \"A\" scroll 100 palette 0\nceil 20 20\nfloor 200 200\nend\n";
        let e = parse_pen(text).unwrap_err();
        assert!(e.message.contains("newer"), "{}", e.message);
    }

    #[test]
    fn nan_heights_are_rejected() {
        let text = "penetrator-level 1\nzone \"A\" scroll 100 palette 0\nceil 20 NaN\nfloor 200 200\nend\n";
        assert!(parse_pen(text).is_err());
    }

    #[test]
    fn checkpoints_restart_the_zone_from_whichever_end_you_entered() {
        let t = generate_campaign(1);
        let deep_in_zone_2 = t.zones[2].start_x() + 500.0;

        let outbound = t.checkpoint_x(deep_in_zone_2, 1.0);
        assert!(outbound >= t.zones[2].start_x());
        assert!(outbound < t.zones[2].start_x() + SAFE_LEAD_COLUMNS as f32 * COLUMN_W);

        let egress = t.checkpoint_x(deep_in_zone_2, -1.0);
        assert!(egress <= t.zones[2].end_x());
        assert!(egress > outbound, "the way home restarts from the far end");
    }

    #[test]
    fn the_route_to_the_warhead_is_never_blocked_but_the_back_wall_is() {
        for seed in 0..30u64 {
            let t = generate_campaign(seed);
            assert!(
                t.blocked_columns(56.0).is_empty(),
                "seed {seed} generated a cave the ship cannot get through"
            );
            // And the sealed dead end behind the warhead is still there.
            assert!(
                !t.terrain.impassable_columns(56.0).is_empty(),
                "the bunker should end in a wall"
            );
        }
    }

    #[test]
    fn the_shipped_example_level_parses_and_is_flyable() {
        // The example is documentation, and documentation that has drifted from
        // the code is worse than none. This keeps it honest.
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/levels/example.pen");
        let text = std::fs::read_to_string(path).expect("levels/example.pen should ship");
        let track = parse_pen(&text).expect("the shipped example must parse");

        assert!(track.warhead_x().is_some(), "it needs an objective");
        assert!(track.zones.len() == 1);
        assert!(
            track.blocked_columns(70.0).is_empty(),
            "the route to the objective must be flyable"
        );
        // And the same slope rule the generator obeys, at this zone's speed.
        let limit = crate::terrain::slope_limit_for(track.zones[0].scroll);
        let warhead_col = track.terrain.column_at(track.warhead_x().unwrap());
        for i in 1..=warhead_col {
            let step = (track.terrain.ceiling[i] - track.terrain.ceiling[i - 1])
                .abs()
                .max((track.terrain.floor[i] - track.terrain.floor[i - 1]).abs());
            assert!(
                step <= limit + 0.05,
                "column {i} steps {step:.1}, over the {limit:.1} the ship can climb"
            );
        }
    }

    #[test]
    fn the_route_gap_ignores_terrain_behind_the_objective() {
        for seed in 0..20u64 {
            let t = generate_campaign(seed);
            assert!(
                t.terrain.tightest_gap() < 1.0,
                "the bunker should end in a sealed wall"
            );
            // 56 is the bunker approach corridor, which is the tightest the
            // game ever deliberately gets. Anything below that is a generator
            // fault, not level design.
            assert!(
                t.tightest_route_gap() >= 55.0,
                "seed {seed} reported a route gap of {}",
                t.tightest_route_gap()
            );
        }
    }

    #[test]
    fn the_route_gap_survives_an_empty_track() {
        assert_eq!(Track::default().tightest_route_gap(), 0.0);
    }

    #[test]
    fn a_track_with_no_warhead_is_checked_end_to_end() {
        let mut t = generate_campaign(3);
        t.spawns.retain(|s| s.kind != SpawnKind::Warhead);
        assert!(
            !t.blocked_columns(56.0).is_empty(),
            "with no objective, the sealed bunker counts as blocking the route"
        );
    }
}
