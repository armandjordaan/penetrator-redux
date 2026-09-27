//! End-to-end mission tests.
//!
//! The unit tests in each module check pieces. These check the one thing no unit
//! test can: that a whole mission is actually completable — that a pilot flying
//! the cave from the mouth to the warhead and back out again reaches the end,
//! from many different seeds, without the simulation deadlocking, running out of
//! world, or leaving the player somewhere they cannot escape from.
//!
//! The pilot is an autopilot: a small controller that reads the same [`World`]
//! the renderer does and produces the same [`Frame`] the keyboard does. It is
//! deliberately not very good. It steers for the middle of the cave, shoots
//! forward, and drops a bomb when the warhead is under it. If a mission can only
//! be finished by a human playing well, these tests fail — which is exactly the
//! signal wanted.

use macroquad::prelude::Vec2;
use penetrator::audio::Audio;
use penetrator::config::*;
use penetrator::input::Frame;
use penetrator::level::{generate_campaign, SpawnKind};
use penetrator::world::{Phase, RunState, World};

/// Simulation step. Small enough that the autopilot's bang-bang steering does not
/// oscillate into the walls.
const DT: f32 = 1.0 / 120.0;
/// Give up after this much simulated flight time.
///
/// A clean round trip is about three and a half minutes. The autopilot is not
/// clean — it dies twenty to sixty times a run and each death costs a zone
/// restart — so the budget has to cover a bad pilot having a bad day. It is a
/// deadlock detector, not a par time.
const TIME_LIMIT: f32 = 1800.0;

/// Reads the world and produces one frame of input.
///
/// The controller has three parts, in priority order: find a lane, break from
/// missiles, and stay off the walls. Nothing here is clever — it is roughly what
/// a competent player does without thinking about it — but it does have to
/// *anticipate*, because a bang-bang controller reacting to the cave one pixel
/// at a time oscillates into the rock and would measure the controller rather
/// than the game.
fn autopilot(w: &World) -> Frame {
    let dir = w.phase.facing();
    let ship = w.player.pos;

    // --- where the lane is ---
    // Look at the tightest the cave gets over the next stretch, not at where it
    // is right now, and aim for the middle of that. The horizon scales with
    // speed: at 150 units per second, half a second of warning is 75 units.
    let horizon = (w.scroll_speed() * 0.55).clamp(40.0, 110.0);
    let mut lowest_ceiling = f32::MIN;
    let mut highest_floor = f32::MAX;
    let mut d = 0.0;
    while d <= horizon {
        let (top, bottom) = w.track.terrain.gap_at(ship.x + d * dir);
        lowest_ceiling = lowest_ceiling.max(top);
        highest_floor = highest_floor.min(bottom);
        d += COLUMN_W * 0.5;
    }
    let mut target_y = if highest_floor - lowest_ceiling > SHIP_HALF_HEIGHT * 2.0 + 6.0 {
        (lowest_ceiling + highest_floor) * 0.5
    } else {
        // The horizon contains a pinch this look-ahead cannot resolve; fall back
        // to the local centre and deal with it when it arrives.
        w.track.terrain.centre_at(ship.x + 16.0 * dir)
    };

    // --- in the bunker, sit above the warhead ---
    if w.phase == Phase::BunkerHold {
        if let Some(wx) = w.track.warhead_x() {
            let (top, bottom) = w.track.terrain.gap_at(wx);
            target_y = top + (bottom - top) * 0.32;
        }
    }

    // --- break from missiles ---
    // Only worth doing for something still guiding: a burnt-out missile is going
    // where it is going, and jinking into a wall to avoid one is a worse trade.
    let mut threat: Option<(f32, Vec2)> = None;
    for p in &w.projectiles {
        if p.friendly || !p.homing {
            continue;
        }
        let to_ship = ship - p.pos;
        let range = to_ship.length();
        if range > 120.0 || to_ship.dot(p.vel) <= 0.0 {
            continue;
        }
        if threat.map(|(r, _)| range < r).unwrap_or(true) {
            threat = Some((range, p.pos));
        }
    }
    if let Some((_, at)) = threat {
        // Break across the missile's approach, but only as far as the lane
        // allows — the cave kills far more often than the missiles do.
        let lane_top = lowest_ceiling + SHIP_HALF_HEIGHT + 6.0;
        let lane_bottom = highest_floor - SHIP_HALF_HEIGHT - 6.0;
        if lane_bottom > lane_top {
            target_y = if at.y > ship.y { lane_top } else { lane_bottom };
        }
    }

    // --- avoid things that are solid but are not rock ---
    for e in &w.enemies {
        if e.kind != SpawnKind::Mine && e.kind != SpawnKind::Drone {
            continue;
        }
        let ahead = (e.pos.x - ship.x) * dir;
        if !(0.0..100.0).contains(&ahead) || (e.pos.y - target_y).abs() > 20.0 {
            continue;
        }
        let above = (e.pos.y - SHIP_HALF_HEIGHT - 10.0).max(lowest_ceiling + 10.0);
        let below = (e.pos.y + SHIP_HALF_HEIGHT + 10.0).min(highest_floor - 10.0);
        target_y = if (above - ship.y).abs() < (below - ship.y).abs() {
            above
        } else {
            below
        };
        break;
    }

    // --- fly to it ---
    // Command a vertical *velocity* proportional to the error rather than
    // slamming the stick to a stop. This is what stops the overshoot.
    let error = target_y - ship.y;
    let desired_vy = (error * 5.5).clamp(-SHIP_MAX_VY, SHIP_MAX_VY);
    let dv = desired_vy - w.player.vel.y;
    let pitch = if dv > 6.0 {
        1.0
    } else if dv < -6.0 {
        -1.0
    } else {
        0.0
    };

    // --- throttle ---
    let throttle = if w.phase == Phase::BunkerHold {
        match w.track.warhead_x() {
            Some(wx) => {
                let dx = wx - ship.x;
                if dx > 4.0 {
                    1.0
                } else if dx < -4.0 {
                    -1.0
                } else {
                    0.0
                }
            }
            None => 0.0,
        }
    } else if w.player.travel_screen > SHIP_SCREEN_REST + 10.0 {
        // Sit back from the leading edge; the extra warning is worth more than
        // the extra speed.
        -1.0
    } else {
        0.0
    };

    // --- weapons ---
    let fire_held = !w.player.overheated && w.player.heat < 0.9;
    let bomb_pressed = w.phase == Phase::BunkerHold
        && w.track
            .warhead_x()
            .map(|wx| (wx - ship.x).abs() < 16.0)
            .unwrap_or(false);

    Frame {
        pitch,
        throttle,
        fire_held,
        bomb_pressed,
        ..Default::default()
    }
}

/// Result of flying one mission to a conclusion.
struct Flight {
    state: RunState,
    score: u32,
    seconds: f32,
    reached_bunker: bool,
    reached_egress: bool,
    deaths: u32,
}

/// Flies a seed with the autopilot until the run ends or the clock runs out.
///
/// `lives` is inflated well past the three a human gets: the autopilot is not
/// meant to prove the game is easy, only that the route exists and the mission
/// terminates.
fn fly(seed: u64, lives: i32) -> Flight {
    let mut world = World::new(generate_campaign(seed));
    world.lives = lives;
    let mut audio = Audio::silent();

    let mut seconds = 0.0f32;
    let mut reached_bunker = false;
    let mut reached_egress = false;

    while seconds < TIME_LIMIT {
        let input = autopilot(&world);
        world.update(&input, DT, &mut audio);
        seconds += DT;

        reached_bunker |= world.phase == Phase::BunkerHold;
        reached_egress |= world.phase == Phase::Egress;

        assert!(
            world.player.pos.x.is_finite() && world.player.pos.y.is_finite(),
            "seed {seed}: the ship left the number line at t={seconds:.1}"
        );

        // Let the end-of-run timer tick past the point the shell would take over.
        if let RunState::Complete { timer } | RunState::GameOver { timer } = world.state {
            if timer > 0.5 {
                break;
            }
        }
    }

    Flight {
        state: world.state,
        score: world.score,
        seconds,
        reached_bunker,
        reached_egress,
        deaths: world.stats.deaths,
    }
}

#[test]
fn the_campaign_can_be_flown_end_to_end() {
    let flight = fly(20250726, 99);
    assert!(
        flight.reached_bunker,
        "never reached the bunker (t={:.0}s)",
        flight.seconds
    );
    assert!(flight.reached_egress, "never turned for home");
    assert!(
        matches!(flight.state, RunState::Complete { .. }),
        "mission did not complete: {:?} after {:.0}s",
        flight.state,
        flight.seconds
    );
    assert!(
        flight.score > SCORE_WARHEAD + SCORE_ESCAPE,
        "completing the mission should be worth more than {} points, got {}",
        SCORE_WARHEAD + SCORE_ESCAPE,
        flight.score
    );
}

#[test]
fn many_different_seeds_are_all_completable() {
    // The generator is allowed to make hard caves. It is not allowed to make
    // impossible ones, and this is the check that holds it to that.
    let seeds: [u64; 8] = [0, 1, 7, 42, 1337, 99991, 20250726, u64::MAX / 3];
    for seed in seeds {
        let flight = fly(seed, 99);
        assert!(
            matches!(flight.state, RunState::Complete { .. }),
            "seed {seed} could not be completed: {:?} after {:.0}s (bunker={}, egress={}, deaths={})",
            flight.state,
            flight.seconds,
            flight.reached_bunker,
            flight.reached_egress,
            flight.deaths
        );
    }
}

#[test]
fn a_mission_always_terminates_rather_than_stalling() {
    // With a single life the run ends one way or the other, but it must end.
    let flight = fly(555, 1);
    assert!(
        flight.state.is_over(),
        "run neither finished nor failed inside {TIME_LIMIT}s"
    );
    assert!(flight.seconds < TIME_LIMIT);
}

#[test]
fn the_return_leg_is_flown_faster_than_the_outbound_one() {
    // The egress speed bonus is meant to be felt, not just configured. Measure it
    // where it actually applies: the time the camera takes to cross one zone.
    let track = generate_campaign(4711);
    let zone = track.zones[1].clone();
    let mut audio = Audio::silent();

    let mut cross = |egress: bool| -> f32 {
        let mut w = World::new(generate_campaign(4711));
        w.lives = 999;
        let (from, to) = if egress {
            w.phase = Phase::Egress;
            (zone.end_x(), zone.start_x())
        } else {
            (zone.start_x(), zone.end_x())
        };
        w.cam_x = from;
        let mut t = 0.0f32;
        while t < TIME_LIMIT {
            // Fly it perfectly. The only thing being measured is scroll speed.
            w.player.pos.y = w.track.terrain.centre_at(w.player.pos.x);
            w.player.vel.y = 0.0;
            w.player.invuln = 10.0;
            w.update(&Frame::default(), DT, &mut audio);
            t += DT;
            let done = if egress { w.cam_x <= to } else { w.cam_x >= to };
            if done {
                break;
            }
        }
        t
    };

    let outbound = cross(false);
    let egress = cross(true);
    assert!(
        egress < outbound,
        "egress took {egress:.1}s, outbound took {outbound:.1}s — the way home should be faster"
    );
}

#[test]
fn destroying_the_radar_network_changes_what_gets_launched() {
    // Fly the opening stretch twice: once with the radar left standing, once with
    // it wiped out up front. The second run must produce unguided missiles.
    let track = generate_campaign(8080);
    let mut audio = Audio::silent();

    let mut guided = World::new(track.clone());
    guided.lives = 99;
    let mut unguided = World::new(track);
    unguided.lives = 99;
    unguided.radars_alive = 0;

    let mut saw_homing = false;
    let mut saw_ballistic = false;
    for _ in 0..(90.0 / DT) as usize {
        guided.update(&autopilot(&guided), DT, &mut audio);
        unguided.update(&autopilot(&unguided), DT, &mut audio);

        saw_homing |= guided.projectiles.iter().any(|p| p.homing);
        saw_ballistic |= unguided
            .projectiles
            .iter()
            .any(|p| p.kind == penetrator::projectile::ProjectileKind::Sam && !p.homing);
    }

    assert!(saw_homing, "a live radar network should produce guided SAMs");
    assert!(
        saw_ballistic,
        "with the radar down every SAM should fly ballistic"
    );
}

#[test]
fn the_ship_never_ends_up_outside_the_playfield() {
    let mut world = World::new(generate_campaign(24680));
    world.lives = 99;
    let mut audio = Audio::silent();

    for _ in 0..(180.0 / DT) as usize {
        // Deliberately terrible input: hold full climb, then full dive.
        let pitch = if (world.stats.elapsed * 2.0) as i32 % 2 == 0 {
            -1.0
        } else {
            1.0
        };
        world.update(
            &Frame {
                pitch,
                throttle: 1.0,
                fire_held: true,
                bomb_pressed: true,
                ..Default::default()
            },
            DT,
            &mut audio,
        );
        let p = world.player.pos;
        assert!(
            p.y >= HUD_H - 1.0 && p.y <= VIRTUAL_H + 1.0,
            "ship escaped vertically to {p:?}"
        );
        let screen_x = p.x - world.cam_x;
        assert!(
            (0.0..=VIRTUAL_W).contains(&screen_x),
            "ship escaped horizontally to screen x {screen_x}"
        );
        assert!(world.cam_x >= -1.0, "camera reversed off the start of the track");
    }
    let _ = Vec2::ZERO;
}
