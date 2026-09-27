# The `.pen` level format and the landscape editor

The 1983 original shipped with a landscape editor, which was extraordinary for a
48K game and is the main reason people still talk about it. Leaving it out of a
redesign would be missing the point.

---

## The editor

Reach it from the main menu. It opens on a freshly generated campaign, which is
usually a better starting point than a blank canvas.

### Keys

| Key | Action |
|---|---|
| `1` | Sculpt the **ceiling** |
| `2` | Sculpt the **floor** |
| `3` | **Place** objects |
| `4` | **Erase** objects |
| `Tab` | Next object type (and switches to Place) |
| Left mouse | Grow rock into the cave / place an object |
| Right mouse | Carve rock away / remove an object |
| Mouse wheel, `[` `]` | Brush size (6–90) |
| `A` `D`, `←` `→` | Pan (hold `Shift` for 3×) |
| `Home` `End` | Jump to the start / end of the track |
| `Ctrl`+`S` | Save to `levels/custom.pen` |
| `Ctrl`+`O` | Load `levels/custom.pen` |
| `Ctrl`+`N` | Generate a new cave |
| `Ctrl`+`B` | Blank canvas — one wide-open zone to build in |
| `F5` | **Playtest** what is on screen |
| `H` | Show / hide the key list |
| `Esc` | Back to the menu |

### The toolbar

Along the top, left to right:

* **Current tool**, and either the brush size or the object about to be placed.
* **ROUTE** — `CLEAR`, or `n BLOCKED` if the cave is pinched shut anywhere
  between the start and the warhead. Terrain *behind* the objective is scenery,
  so a sealed dead end at the back of a chamber does not count.
* **TIGHTEST** — the narrowest gap anywhere, in units. Red below 52.
* **LENGTH** — track length in columns.
* **SAVED / UNSAVED**.

Blocked columns are also shaded red in the playfield, so you can see where the
problem is rather than just being told there is one.

### Two rules worth respecting

The generator guarantees both of these and the editor lets you break both of
them, because a deliberate wall is a legitimate thing to build. If you are
building something you intend to fly, though:

1. **Keep the gap above about 70 units** on the route to the warhead. The ship
   is 8 units tall; below 70 there is no room to react.
2. **Keep the per-column step under about 9 units.** This one is much easier to
   get wrong. A wall that rises faster than the ship can climb at scroll speed is
   an unavoidable death no matter how much room there is above it. The figure
   is `8 / scroll × 178 × 0.72`: about **9.3** units per column at scroll 110,
   about **6.8** at scroll 150. (The campaign is stricter still, because it
   derives the limit from each zone's *egress* speed — 1.18× the outbound one —
   giving 5.8 in the fastest zone.) Sculpting with a wide brush stays inside the
   limit naturally; a narrow brush dragged hard does not.

`docs/ARCHITECTURE.md` explains why the second rule exists.

### Anchoring

Silos, radar dishes and the warhead sit on the floor. Turrets hang from the
ceiling. Their vertical position is **resolved from the terrain every time it is
asked for**, not stored — so sculpting underneath a silo moves the silo. The
editor draws a short tether to the surface each object belongs to, to make that
obvious. Mines and interceptors float, and their `y` is stored.

### Playtesting

`F5` hands the track straight to the simulation. There is no export step and no
separate format: the editor edits the same `Track` the campaign generator
produces and the simulation consumes, so anything you can build is automatically
playable. The one requirement is a warhead — without an objective there is
nothing to fly to, and the editor will say so.

---

## The file format

Line-oriented and deliberately boring. A header, then one block per zone holding
that zone's two height arrays and its spawns.

```
penetrator-level 1
seed 20250726
# comments run to end of line

zone "APPROACH" scroll 112.0 palette 0
ceil 24.0 25.5 26.9 28.4 29.9 31.3 32.8 34.3 35.7 37.2
ceil 38.7 40.1 41.6 43.1 44.5 46.0 46.0 50.9 55.6 60.0
floor 264.0 262.7 261.3 260.0 258.7 257.3 256.0 254.7
floor 253.3 252.0 250.7 249.3 248.0 246.7 245.3 244.0
spawn radar 280.0
spawn silo 420.0
spawn mine 660.0 150.0
end
```

### Header

| Directive | Meaning |
|---|---|
| `penetrator-level <version>` | Required, must be first. Current version is `1`. A file claiming a *newer* version is refused rather than guessed at. |
| `seed <u64>` | Optional. What the track was generated from; `0` for hand-written levels. |

### Zone blocks

```
zone "<NAME>" [scroll <f32>] [palette <usize>]
```

The name must be quoted. `scroll` is the camera speed in world units per second
(1–1000, default 120). `palette` indexes the colour schemes in `theme.rs` and
wraps, so `palette 99` gets a colour scheme rather than a crash.

Inside a block:

| Directive | Meaning |
|---|---|
| `ceil <v> <v> ...` | Ceiling heights, appended in order. May repeat across as many lines as you like. |
| `floor <v> <v> ...` | Floor heights, same. |
| `spawn <kind> <x> [y]` | An object. `y` is only read for `mine` and `drone`. |
| `end` | Closes the block. |

Heights are **absolute screen y** in the 480×270 virtual canvas: `ceil` is the
underside of the rock above, `floor` is the top of the rock below. One value per
8-unit column. The playfield runs from y = 24 (just under the HUD) to y = 264.

Spawn coordinates are **relative to the start of their zone**, so blocks can be
reordered by hand without recomputing anything. The loader adds the offset.

### Spawn kinds

`silo` · `radar` · `turret` · `mine` · `drone` · `warhead`

`drone` is accepted in a file but the campaign only scrambles interceptors on the
egress leg; placing them by hand puts them there from the start.

Exactly one `warhead` makes sense. The editor replaces any existing one when you
place another.

### What the parser will refuse

Errors carry a line number, and the editor shows them.

* An unknown directive or an unknown zone attribute. A typo in a hand-edited
  level should be reported, not quietly played.
* A `ceil` / `floor` mismatch — the two arrays in a zone must be the same length.
* A zone with fewer than two columns.
* `zone` inside a zone, or `end` without a `zone`.
* A zone that is never closed.
* Non-finite heights or coordinates.
* A format version newer than the build understands.
* A file with no zones at all.

### Formatting notes

* `#` starts a comment; blank lines are ignored.
* The writer emits one decimal place, which is finer than the renderer can show,
  and wraps height arrays at 20 values per line to keep files readable.
* A write-then-read round trip is tested, and so is the shipped example.

---

## The shipped example

`levels/example.pen` is a short hand-written mission kept as a worked example: an
open lead-in, a rolling stretch with a radar dish and three silos, a corridor
narrowing into a chamber, the warhead, and a sealed back wall.

It is checked by a test that parses it, confirms it has an objective, confirms
its route is not blocked, and confirms every column obeys the slope limit for its
scroll speed — so the documentation cannot drift away from the code.

To fly it:

```sh
cp levels/example.pen levels/custom.pen
```

then pick **FLY A CUSTOM LEVEL** from the menu. The menu falls back to the
example if you have not saved anything of your own, so on a fresh install the
entry is never a dead end.
