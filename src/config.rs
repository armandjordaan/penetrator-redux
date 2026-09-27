//! Every tunable number in the game lives here.
//!
//! The rule is deliberate: gameplay modules import constants from this module and
//! never define magic numbers of their own. If a value affects how the game *feels*,
//! it belongs here so it can be found and tweaked without reading the simulation.

// ---------------------------------------------------------------------------
// Virtual resolution
// ---------------------------------------------------------------------------

/// The game is simulated and drawn in a fixed 480x270 virtual canvas, then
/// letterboxed onto whatever window the player has. All gameplay coordinates in
/// the codebase are virtual units; the only place real pixels appear is
/// `render::Viewport`.
pub const VIRTUAL_W: f32 = 480.0;
pub const VIRTUAL_H: f32 = 270.0;
pub const ASPECT: f32 = VIRTUAL_W / VIRTUAL_H;

/// Height of the HUD strip drawn over the top of the playfield. Terrain
/// generation keeps the cave ceiling below this so the HUD never hides a wall.
pub const HUD_H: f32 = 22.0;

// ---------------------------------------------------------------------------
// Terrain
// ---------------------------------------------------------------------------

/// World-space width of one terrain column. Terrain is a pair of height arrays
/// sampled at this interval and linearly interpolated in between, which is what
/// gives the cave its angular, faceted silhouette.
pub const COLUMN_W: f32 = 8.0;

/// Absolute limits the generator will never push the cave surfaces past.
pub const CEILING_MIN: f32 = HUD_H + 2.0;
pub const CEILING_MAX: f32 = VIRTUAL_H * 0.55;
pub const FLOOR_MIN: f32 = VIRTUAL_H * 0.45;
pub const FLOOR_MAX: f32 = VIRTUAL_H - 6.0;

/// Columns at the very start of the track that are forced wide open, so a
/// respawn never drops you inside a wall.
pub const SAFE_LEAD_COLUMNS: usize = 14;

/// Fraction of the ship's theoretical climb rate the cave is allowed to demand.
///
/// A cave surface that rises faster than the ship can climb is not difficult, it
/// is a scripted death: at scroll speed there is no input that clears it. The
/// generator therefore caps how much either surface may move between adjacent
/// columns, at this fraction of what the ship could just barely manage. The
/// slack is what the player flies with — reaction time, and room to be a bit
/// out of position when the wall arrives.
pub const SLOPE_SAFETY: f32 = 0.72;

// ---------------------------------------------------------------------------
// Camera / scrolling
// ---------------------------------------------------------------------------

/// How far ahead of the rear edge the ship may sit, in screen units. The player
/// throttles within this band; the camera does the rest of the travelling.
pub const SHIP_SCREEN_MIN: f32 = 54.0;
pub const SHIP_SCREEN_MAX: f32 = 404.0;
/// Where the ship sits when a leg begins (measured along the travel direction).
pub const SHIP_SCREEN_REST: f32 = 132.0;

/// Rate at which the throttle slides the ship through the band.
pub const THROTTLE_SPEED: f32 = 132.0;
/// How hard the ship is pulled back toward the rest position when not throttling.
pub const THROTTLE_RECENTER: f32 = 26.0;

// ---------------------------------------------------------------------------
// Ship
// ---------------------------------------------------------------------------

pub const SHIP_ACCEL_Y: f32 = 900.0;
pub const SHIP_MAX_VY: f32 = 178.0;
/// Per-second velocity retention when no vertical input is held (drag).
pub const SHIP_DRAG_Y: f32 = 0.0016;
pub const SHIP_HALF_LEN: f32 = 9.0;
pub const SHIP_HALF_HEIGHT: f32 = 4.0;
/// Radius used for ship-vs-projectile and ship-vs-enemy tests.
pub const SHIP_HIT_RADIUS: f32 = 6.0;
/// Invulnerable window after respawning.
pub const RESPAWN_INVULN: f32 = 2.4;

// ---------------------------------------------------------------------------
// Weapons
// ---------------------------------------------------------------------------

pub const CANNON_COOLDOWN: f32 = 0.105;
pub const CANNON_SPEED: f32 = 460.0;
pub const CANNON_RANGE: f32 = 340.0;
pub const CANNON_DAMAGE: i32 = 1;
/// Heat added per shot; heat bleeds off at `HEAT_COOL` per second. Hitting 1.0
/// locks the cannon until it falls back under `HEAT_RESUME`.
pub const HEAT_PER_SHOT: f32 = 0.085;
pub const HEAT_COOL: f32 = 0.44;
pub const HEAT_RESUME: f32 = 0.35;

pub const BOMB_COOLDOWN: f32 = 0.34;
pub const BOMB_GRAVITY: f32 = 300.0;
pub const BOMB_BLAST_RADIUS: f32 = 26.0;
pub const BOMB_DAMAGE: i32 = 4;
pub const BOMB_MAX: i32 = 12;
/// Seconds to regenerate one bomb. You can never be permanently disarmed.
pub const BOMB_REGEN: f32 = 2.6;

// ---------------------------------------------------------------------------
// Enemies
// ---------------------------------------------------------------------------

pub const SILO_HP: i32 = 2;
pub const SILO_FIRE_INTERVAL: f32 = 3.4;
/// Horizontal distance at which a silo notices you and starts its launch timer.
pub const SILO_TRIGGER_RANGE: f32 = 264.0;
/// How far past an emplacement the ship may get before it stops shooting.
/// Small on purpose — see `Enemy::engagement_envelope`.
pub const TRAILING_FIRE_MARGIN: f32 = 36.0;

pub const RADAR_HP: i32 = 2;
pub const TURRET_HP: i32 = 2;
pub const TURRET_FIRE_INTERVAL: f32 = 2.15;
pub const TURRET_SHELL_SPEED: f32 = 138.0;
pub const TURRET_TRIGGER_RANGE: f32 = 250.0;

pub const MINE_HP: i32 = 1;
pub const MINE_RADIUS: f32 = 6.5;
pub const MINE_DRIFT: f32 = 16.0;

pub const DRONE_HP: i32 = 2;
pub const DRONE_SPEED: f32 = 84.0;
pub const DRONE_RADIUS: f32 = 6.0;
/// Interceptors are only ever placed where the cave is at least this open. In a
/// narrow passage there is nowhere to go and nowhere to shoot from.
pub const DRONE_MIN_GAP: f32 = 108.0;
/// How close an interceptor presses before breaking off.
pub const DRONE_STANDOFF: f32 = 74.0;
/// How long it spends heading away before turning back in.
pub const DRONE_BREAK_TIME: f32 = 1.15;

pub const WARHEAD_HP: i32 = 8;
pub const WARHEAD_RADIUS: f32 = 15.0;

/// SAM behaviour. With a radar still standing, missiles steer; with every radar
/// destroyed they fly ballistically and are trivial to dodge. This is the single
/// most important tactical decision the game asks of the player.
pub const SAM_SPEED: f32 = 152.0;
pub const SAM_HOMING_SPEED: f32 = 132.0;
/// Radians per second a homing SAM may turn. Gives a turning circle of roughly
/// 73 units — wider than the ship can be pushed sideways in the same time, which
/// is what makes a hard break work.
pub const SAM_TURN_RATE: f32 = 1.8;
/// How long a guided SAM steers before its motor burns out and it coasts.
///
/// This number is the difference between a fair game and an unfair one. Without
/// a burnout a missile pursues for its whole life, so dodging only postpones the
/// hit and the player has no way to actually defeat one. With it, breaking hard
/// and holding the break beats a missile permanently — and that is a skill worth
/// having.
pub const SAM_GUIDANCE_TIME: f32 = 2.6;
/// Speed a SAM leaves the tube at, before its motor lights properly.
pub const SAM_LAUNCH_SPEED: f32 = 46.0;
/// How long the boost phase lasts.
///
/// This is the telegraph. A missile that appears at full speed is a hit you
/// could not have read; a missile that climbs slowly out of its silo for a third
/// of a second is a hit you should have seen coming. It costs the player nothing
/// in difficulty and buys the game its fairness.
pub const SAM_BOOST_TIME: f32 = 0.38;
pub const SAM_LIFETIME: f32 = 5.0;
pub const SAM_RADIUS: f32 = 3.0;

// ---------------------------------------------------------------------------
// Scoring
// ---------------------------------------------------------------------------

pub const SCORE_SILO: u32 = 150;
pub const SCORE_RADAR: u32 = 400;
pub const SCORE_TURRET: u32 = 200;
pub const SCORE_MINE: u32 = 75;
pub const SCORE_DRONE: u32 = 250;
/// Shooting an incoming missile out of the air.
pub const SCORE_INTERCEPT: u32 = 50;
pub const SCORE_WARHEAD: u32 = 5000;
pub const SCORE_ZONE_CLEAR: u32 = 1000;
pub const SCORE_ESCAPE: u32 = 10000;
/// Awarded per remaining life when the mission is completed.
pub const SCORE_LIFE_BONUS: u32 = 2500;

pub const STARTING_LIVES: i32 = 3;

// ---------------------------------------------------------------------------
// Feel
// ---------------------------------------------------------------------------

/// Screen-shake trauma decays linearly at this rate; shake magnitude is
/// trauma-squared so small hits stay subtle and big ones really land.
pub const TRAUMA_DECAY: f32 = 1.5;
pub const SHAKE_MAX_OFFSET: f32 = 9.0;

/// Slow-motion factor applied briefly when the warhead detonates.
pub const SLOWMO_SCALE: f32 = 0.28;
pub const SLOWMO_TIME: f32 = 1.1;

/// The simulation never advances by more than this in one step, so a stalled
/// frame (window drag, breakpoint) cannot teleport the ship through a wall.
pub const MAX_DT: f32 = 1.0 / 30.0;

// ---------------------------------------------------------------------------
// Difficulty
// ---------------------------------------------------------------------------

/// The most guided or unguided SAMs that may be airborne at once.
///
/// Individually every missile in this game is beatable. Collectively they stop
/// being: three silos whose timers happen to line up put up a wall with no gap
/// in it, and no amount of skill opens one. Capping the count leaves each
/// missile exactly as dangerous as it was and removes the situation that is not
/// a fight. A silo that wants to launch into a full sky simply holds.
pub const MAX_LIVE_SAMS: usize = 4;

/// Multiplier applied to enemy fire rates on the egress leg. The way home is
/// supposed to hurt.
pub const EGRESS_AGGRESSION: f32 = 1.35;
/// Extra scroll speed on the way out.
pub const EGRESS_SPEED_BONUS: f32 = 1.18;

/// How far past the start of the track the camera keeps travelling on the way
/// home, so the ship flies out of the cave mouth into open air rather than
/// stopping dead against an invisible limit.
///
/// This has to be at least a ship's-eye view wide. The camera's left edge is the
/// origin of screen space, so with the camera pinned at zero the ship can only
/// ever get as far left as its own throttle band allows — which is most of a
/// screen short of the mouth. Letting the camera run negative is what turns
/// "the scrolling stopped" into "you got out".
pub const EGRESS_EXIT_RUN: f32 = 340.0;
