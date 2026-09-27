# Design notes

What changed from the 1983 original, what stayed, and what the playtesting
found.

---

## What the original was

*Penetrator*, Philip Mitchell / Beam Software, published by Melbourne House for
the ZX Spectrum in 1983. A horizontally scrolling cave-flyer, clearly descended
from *Scramble*, with four zones of terrain, missile silos that launched at you,
radar installations that guided those missiles, and a nuclear warhead at the
bottom that you had to bomb — and then fly all the way back out through the same
cave.

It also shipped with a landscape editor, in 48K, in 1983. That is the part people
still talk about.

---

## What was kept, and why

**The throttle band.** The ship does not control its own x. The camera travels at
the zone's speed and the throttle slides you forward and back *within the
screen*. This looks like a limitation of 1983 hardware and is not — it guarantees
the player can always see what is coming, and it turns "how far ahead do I sit"
into a genuine risk/reward decision. It is still the right answer and it is
unchanged here.

**The radar rule.** Destroy the dishes and every missile in the game goes stupid.
This is the best idea in the original: a single, legible, permanent decision that
transforms the difficulty of everything downstream, and which the player can
choose to make or skip. It is the spine of this build too.

**Bomb the warhead, then fly home.** The return leg is what makes it a mission
rather than a level. You have already learned the cave; now it is faster and the
defences are back.

**The landscape editor.** Non-negotiable.

**The four-zone escalation**, the cannon-and-bombs loadout, the three lives.

---

## What changed

### Presentation

A neon-vector look on a black field, with an optional CRT filter — scanlines, a
rolling brightness band, and a vignette, all toggleable with `F1`. Zones are
distinguished by hue rotation rather than brightness, so no zone is easier to
read than another.

The rendering is deliberately not a pixel-art recreation. The original's
chunky attribute-clash look was a consequence of the hardware, not a design
choice, and reproducing it faithfully would be nostalgia rather than redesign.
What is worth reproducing is the *silhouette*: an angular, faceted cave and a
small, readable ship.

### Procedural caves

The original had fixed, hand-authored zones. This build generates a new
four-zone system from a seed on every run, with per-zone parameters for
roughness, spike density, wavelength and minimum gap. The seed is shown on the
debug overlay and offered as "fly the same cave again" on the menu, so a run you
liked is repeatable.

The editor still exists for hand-authored levels, and the campaign generator's
output is what it opens on — so "generate, then tweak" is the intended workflow
rather than "start from nothing".

### Cannon heat

New. The original's cannon was unlimited and there was no reason not to hold the
trigger down. A heat bar that locks the gun out after about 2.7 seconds of
sustained fire turns shooting into a decision about *what* to shoot, which is
what makes the radar rule matter.

It is deliberately not a magazine. You are never out of ammunition, only
temporarily out of patience.

### Bomb regeneration

Twelve bombs, one back every 2.6 seconds. The original could leave you unable to
complete the mission if you had wasted your ordnance, which is a fail state that
tells the player nothing useful. Here you can be inefficient, but you cannot be
permanently disarmed.

### Turrets, mines and interceptors

Three enemy types the original did not have, added for texture:

* **Ceiling turrets** fire aimed, led shells. They give the ceiling something to
  threaten with, in a game where the floor has all the silos.
* **Mines** drift slowly and do not shoot. Pure spatial pressure.
* **Interceptors** only scramble on the way home, so the egress leg has a threat
  of its own rather than just more of the same.

### The bunker

The original's warhead sat at the end of the fourth zone. Here it gets its own
hand-authored section: a corridor squeezing down to a 56-unit slot, a chamber,
and a sealed back wall. When you reach it the camera **stops** — nothing is
pushing you forward any more.

That pause is the whole point. It is the one moment in the game that is not about
reflexes. You have as long as you like to line up, and the warhead's armour means
the only thing that will do the job is a bomb.

### Quality-of-life

Per-zone checkpoints rather than restarting the mission; 2.4 seconds of
invulnerability after a respawn (against the enemy, never against the cave);
mission callouts for zone arrivals and radar kills; a progress bar that turns
round when you do; a `MISSILE LOCK` warning that only fires for a guided missile
actually closing on you.

---

## What the playtesting found

The end-to-end mission test — an autopilot that flies the whole campaign
headlessly — was written to prove the game was completable. It proved it was not,
several times over. Every change below came out of watching it fail and asking
whether the failure was the pilot's fault or the game's.

That distinction is the useful one. A bot dying because it flies badly tells you
nothing. A bot dying because there was no input that would have worked tells you
a lot.

### 1. Cannon rounds passed straight through missiles

The first run died 92 times in 300 seconds without leaving zone one. Almost all
of them were "SHOT DOWN".

The cannon could not hit ordnance. In the original you could shoot missiles down,
and that is not a bonus feature — it is the answer to a missile in your face with
nowhere to dodge to. Without it the cannon had nothing to do between
emplacements, and a close-range launch was simply unsurvivable.

**Fix:** cannon rounds destroy incoming SAMs and shells. Worth 50 points.

### 2. Emplacements fired at things that had already gone past

Silos engaged anything within range in *either* direction. Since the player's
cannon only fires forward, a missile launched from behind had no answer at all —
you could not shoot it, and dodging only postponed it.

**Fix:** an asymmetric engagement envelope. An emplacement engages a ship that is
*approaching* it and stops 36 units after the ship has gone past. This is how a
real launch envelope works, and more importantly it keeps every shot in the game
something the player could have done something about. The envelope flips with the
direction of travel, so it works on the way home too.

### 3. Homing missiles pursued for their entire lifetime

A guided SAM steered for all seven seconds it existed. Dodging did not defeat it;
it merely postponed the hit while the missile came round again. There was no
input that beat one, only inputs that delayed it.

**Fix:** guidance burnout. A SAM steers for 2.6 seconds and then coasts. Break
hard and *hold the break* and you have beaten it permanently. Lifetime dropped to
5 seconds and the turn rate from 2.35 to 1.8 rad/s, giving a turning circle of
about 73 units — wider than the ship can be pushed sideways in the same time,
which is what makes a hard break work at all.

This single change roughly halved the death rate.

### 4. The generator could build slopes the ship could not climb

This is the subtle one, and it is the finding worth reading twice.

After the missile fixes, the bot got much further and then died repeatedly in
zone three — into the *terrain*, not to enemy fire.

The generator already guaranteed a minimum gap: the cave was never too narrow to
fit through. But a stalagmite rising 44 units over two columns needs a climb rate
of 380 units per second at that zone's scroll speed, and the ship's maximum is
178. There was plenty of room above the spike. Reaching it was physically
impossible.

**Fix:** a second playability pass. `Terrain::limit_slope` caps how far either
surface may move between adjacent columns, at 72% of what the ship could just
barely manage at the fastest speed that section will ever be flown. The
implementation is a forward-then-backward min-convolution, which guarantees the
bound holds everywhere and — crucially — only ever *widens* the cave, so it can
be run after the minimum-gap pass without undoing it.

Wide enough is not the same as reachable. Both need proving.

### 5. Bombs missed a stationary target

With the terrain fixed, the bot reached the bunker on every seed — and then sat
in the chamber until the clock ran out, unable to destroy the warhead.

Ordnance inherited the *zone's nominal scroll speed* rather than the ship's
actual velocity. In the parked chamber the ship is not moving, but bombs still
flew forward at 96 units per second and landed about 48 units past the target.

**Fix:** ordnance inherits the camera's velocity, which is what the ship is
actually doing. In the chamber that is zero, and bombs fall straight down.

A human would have found this in ten seconds and assumed they were aiming badly.

### 6. The mission could not be completed at all

Then every seed reached the egress leg, flew all the way home — and stopped dead
at x = 348, forever.

Completion keyed off the ship's world x reaching 58. On the way home the camera
clamped at 0, and the ship's position is the camera plus its offset in the
throttle band. The furthest left the ship could ever get was 76. The finish line
was 18 units beyond anywhere the ship could physically be.

**Fix:** the camera keeps travelling 340 units past the start of the track, and
completion keys off the camera running out rather than the ship reaching an x.
Terrain sampling clamps below column zero and the first columns are forced wide
open, so there is nothing solid out there — the cave mouth simply recedes behind
you and you are out.

This bug survived every unit test in the project. It could only be found by
flying the whole thing.

### 7. Interceptors were homing mines

Drones steered at the ship until one of them stopped existing. That produced
death loops: 30 to 40 deaths at a single spot, because the drone arrived head-on
at combined speed, there was nowhere to go round it, and respawning at the
checkpoint put you back where it did the same thing again.

**Fix:** attack runs. A drone presses in to 74 units, then breaks off for 1.15
seconds before coming back round. Contact is still fatal — it just has to be
earned, and the break gives the player a window to shoot. Placement rules were
added too: nothing within a screen and a half of the bunker, and nothing in a
passage narrower than 108 units.

### 8. Three silos could put up a wall

The last hotspot was 93 deaths inside a 400-unit stretch. Individually every
missile was beatable. Collectively they were not: three launchers whose timers
lined up produced a wall of ordnance with no gap in it.

**Fix:** at most four SAMs airborne at once. A silo that wants to launch into a
full sky holds. This leaves every individual missile exactly as dangerous as it
was and removes the situation that is not a fight.

Egress aggression came down from 1.55× to 1.35× at the same time.

### Where it landed

Across eight seeds the autopilot now completes the full round trip every time,
dying between eighteen and sixty times depending on the seed. It is a poor pilot
— it does not conserve heat, does not prioritise dishes, and reacts rather than
plans. A human doing those three things will do considerably better.

---

## What only showed up on screen

The mission test proved the game could be *played*. It could say nothing at all
about whether it could be *looked at* — it never touches the renderer. Three
things were waiting the first time anyone took a screenshot, and one of them was
not subtle.

### The entire game rendered upside down

Cave, ship, HUD, menu, everything. The simulation was completely correct; the
projection matrix was not.

`Camera2D::from_display_rect` returns a camera with a **negative** `zoom.y`,
because it is written for render targets, which are stored bottom-up. What is
not obvious is that macroquad then applies its own inversion on top, for any
camera that has no render target:

```rust
let invert_y = if self.render_target.is_some() { 1.0 } else { -1.0 };
let mat_scale = Mat4::from_scale(vec3(self.zoom.x, self.zoom.y * invert_y, 1.0));
```

Drawing straight to the screen, as this game does, the two negatives cancel and
y points the wrong way. Every unit test passed. Every coordinate in the codebase
was right. The composition of two correct-looking pieces was not.

**Fix:** negate `zoom.y` after `from_display_rect`, so the surviving inversion is
macroquad's own.

The interesting part is the test that now guards it. The camera matrix is pure
arithmetic and needs no window, so the check is simply: project virtual `y = 0`
and confirm it lands at the top of normalised device space.

```rust
let top = to_ndc(&cam, Vec2::new(VIRTUAL_W * 0.5, 0.0));
assert!(top.y > 0.9, "virtual y=0 must land at the top of the screen");
```

That is four lines, and it would have caught the bug before it was ever drawn.
The lesson is not "test the renderer" — most of a renderer genuinely does need
eyes on it. It is that the *coordinate transform* at the boundary is arithmetic,
and arithmetic is testable even when the thing it feeds is not.

### The end-of-run callout drew straight through the summary

Losing the last ship set a `MISSION FAILED` banner with a six-second life, and
the summary screen appeared after two. For four seconds the banner and the
statistics table were drawn on top of each other, the phrase "MISSION FAILED"
appearing twice in different sizes.

**Fix:** the in-flight banner is suppressed once the run is over. The summary
screen owns the middle of the display and says the same thing in its own words.

### The editor called every valid level broken

The toolbar's `TIGHTEST` readout measured the narrowest gap on the whole track —
which, for any generated campaign, is the deliberately sealed wall behind the
warhead. It read `0px` in red, always, on levels that were perfectly fine.

This was the same mistake as the `ROUTE` check, which had already been fixed to
measure only up to the objective. Fixing one instance of a bug and leaving its
sibling is an easy thing to do; both now share a `route_end_column` helper so
there is one definition of "the part the player has to fly".

---

## Things deliberately not done

**Difficulty settings.** The zones already escalate, and the radar rule already
lets the player choose how hard the rest of the run is. A menu option would be a
worse version of a decision the game already asks.

**Power-ups.** The loadout is fixed. Every run starts the same and the only thing
that varies is how well you fly it, which is what makes scores comparable.

**A pixel-art recreation.** See above — the original's look was a hardware
consequence, not a design choice.

**Continues.** Three lives, checkpoints per zone, and a score at the end. Running
out is meant to mean something.

**Music.** There is a synthesised engine drone and twelve effects. A soundtrack
would fight the sound design's job here, which is to tell you what is happening —
a launch, a hit, an overheat — in a game where things happen behind you.

---

## Credits

The original *Penetrator* is © 1983 Beam Software / Melbourne House, designed and
programmed by Philip Mitchell. This is a from-scratch homage that shares no code
or assets with it.
