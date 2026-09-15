# Job 13 — Gathering terrace and MESH gate

Date: 2026-09-14  
Branch: `feat/solarpunk-forum`

## Result

Implemented the P0 gathering terrace and P4 MESH gate in the two new Blender
modules, plus the requested commons-block cleanup in
`scripts/blender/build_solar_forum.py`.

The terrace now has:

- a bronze `r=3.4 m`, `h=.45 m` council dais and an oiled-timber table ring;
- an inset bronze emitter, `Forum Hologram` globe at `z=2.1 m` (`r=1.15 m`,
  emission strength 6, alpha .55), three tilted rings, and an open light cone;
- eight linked curved `Forum Sand Fabric` sofa segments with four cardinal
  gaps, four cushioned stools, six lantern bowls, and two north standards;
- two cyan glass info panels, eight dense two-tone fern/broadleaf clusters
  using `leaf_spray(..., count=48)`, four diagonal timber planters with
  three-metre trees, trailing vines to about `z=4`, and six catenary cables
  with linked-duplicate amber beads at approximately .6 m spacing;
- a south bronze/glass balustrade with the central arrival stair opening;
- a north crescent reflecting pool. The cascading bowls, central fountain
  plinth, old central coping, and old bench fasteners are removed. The four
  teaching tables are at radius 12.5 m in the four diagonal quadrants.

The gate court includes a six-metre spur from `r=38` to the court edge at
`r=47`, a nine-metre stone plaza, horizon-blue civic inlay, a three-step dais,
a swept bronze ring (`r=4.6 m`, tube `.55 m`), nine chevron housings, a
low-emissive `#5599FF` inner channel, no inner disc, an eight-metre
walk-through floor, a dark semicircular `r=7 m`, `h=5 m` exedra, engraved
`MESH` plaque lettering, two unlit Chitin sigil steles, one downlight housing,
and a low parapet. All 81 gate objects in the saved scratch source are named
with the `Mesh gate` prefix.

## Gate coordinates and browser handoff

Blender is Z-up. The gate court horizontal centre is:

```text
court floor centre:   (-39.597980,  39.597980, 0.000000)
ring centre:          (-39.597980,  39.597980, 4.770000)
ring plane normal:    (-0.707107, 0.707107, 0.000000)
walk direction:       (-0.707107, 0.707107, 0.000000)  # outward, through gate
```

The ring plane is `y - x = 79.195959` in Blender coordinates. The exact
centreline handoff points, all at Blender `z=0`, are:

| Meaning | Blender `(x,y,z)` | Three.js `(x,y,z)` |
| --- | --- | --- |
| promenade / spur start, `r=38` | `(-26.870058, 26.870058, 0)` | `(-26.870058, 0, -26.870058)` |
| court edge / bridge end, `r=47` | `(-33.234019, 33.234019, 0)` | `(-33.234019, 0, -33.234019)` |
| court / ring centre, `r=56` | `(-39.597980, 39.597980, 0)` | `(-39.597980, 0, -39.597980)` |
| eight metres beyond ring, `r=64` | `(-45.254834, 45.254834, 0)` | `(-45.254834, 0, -45.254834)` |

The browser-side outward vector is `(-0.707107, 0, -0.707107)`. The browser
ring centre is `(-39.597980, 4.770000, -39.597980)` and its plane is
`x + z = -79.195959`.

The nine-metre court disk has these axis-aligned floor bounds:

```text
Blender:  x [-48.597980, -30.597980], y [30.597980, 48.597980], z_top 0
Three.js: x [-48.597980, -30.597980], z [-48.597980, -30.597980], y_top 0
```

The rectangular continuation floor used by the exedra has the measured
source bounds `Blender x[-50.205,-34.648], y[34.648,50.205], z[-.12,0]`,
which maps to `Three.js x[-50.205,-34.648], z[-50.205,-34.648], y[-.12,0]`
under the documented `(x,y,z) -> (x,z,-y)` conversion; the authored walk
axis remains the diagonal points in the table above.

## Verification

Syntax check passed:

```text
python3 -m py_compile scripts/blender/forum_terrace.py scripts/blender/forum_meshgate.py scripts/blender/build_solar_forum.py
```

The requested direct command was attempted with absolute paths and exited
`139` before Python with a Blender 5.2.1 native Metal-startup crash
(`supports_barycentric_whitelist`); it produced no scratch output and did not
change shipped assets. Blender also reproduced this when any shell environment
assignment was present. The successful scratch build used a temporary launcher
that sets `FORUM_SCRATCH=/tmp/forum-scratch-terrace` after Blender startup and
executes the same absolute builder path with `--no-render`.

Successful scratch result:

```text
FORUM_VERIFIED {"meshes": 33, "triangles": 625484, "bytes": 45190932,
"campusDiameterMeters": 358.79, "geodiscDiameterMeters": 358.79}
```

This is `-6,078` triangles versus the Job 12 baseline of `631,562` triangles
and is below the raised `1,200,000` triangle / `95,000,000` byte assertions.
The delta is not a pure P0/P4 delta because the scratch run necessarily used
the current concurrent worker modules and temporarily skipped the failing
celestial worker module.

Required scratch renders, both visually inspected after the final geometry
pass:

- [terrace-human-height.png](/tmp/forum-scratch-terrace/terrace-human-height.png)
- [mesh-gate-human-height.png](/tmp/forum-scratch-terrace/mesh-gate-human-height.png)

The terrace camera was `(0,-9,1.7)` looking at `(0,0,1.4)`. The gate camera
was at `r=44`, polar angle 135 degrees, height 1.7, aimed outward through the
ring toward the exedra axis.

## Deviation / follow-up

The full unmodified integration remains blocked by an unrelated concurrent
worker error in `scripts/blender/forum_celestial.py`:

```text
forum_celestial.py, line 584, in _ring_tower_groups
    obj = _linked_object(...)
NameError: name '_linked_object' is not defined
```

That file was not edited. The verification run temporarily skipped only that
module in memory, as permitted for a worker-owned failure. The straight-on
human-height gate render shows the required five-metre exedra masking the
lower part of the ring, while the full physical ring and walk-through remain
authored; a slight upward aim is used in the QA view so the portal reads as a
monumental arch.

## Iteration pass — 2026-09-14

This pass corrected the terrace and gate review items in the authored source.
Final authored coordinates: terrace globe `(0, 0, 2.1)`, hero camera
`(0, -9, 1.7)` looking at `(0, 0, 1.4)`; gate court and ring horizontal centre
`(-39.597980, 39.597980)`, ring centre z `5.87` so its lower tangent sits on
the `z=.72` top dais step, outward axis `(-.707107, .707107)`; spur centreline
`r=38` to `r=47`; court radius `9`; exedra centreline `8 m` outward with its
semicircular arc bowing outward, away from the forum.

Changes: the globe is now `r=1.15`, cyan-teal emissive strength `6`, alpha
`.45`, with thin wire rings, a `.12` alpha open cone, and three raised
low-poly continent patches at alpha `.70`. Sofa fabric is warm sand
`(.78,.66,.48)` with roughness `.92`, timber is `(.22,.12,.055)`, seats and
backs have `.08` bevels with 3 segments, and the backs recline slightly.
Lantern cores are amber strength `8`, strand beads are amber strength `4`,
and the terrace understory remains two-tone with 96 leaves per sofa gap.
The gate now uses the outward true semicircular exedra, `.6` plaque lettering,
`.6 x .3 x 3.2` steles, and a three-step dais whose top is `.72`.

Review-point status:

1. Satisfied in source: translucent hologram, wire rings, cone, and continents.
2. Satisfied in source: rounded/reclined sand sofas, cushions, and timber plinths.
3. Satisfied: the two upper dark slabs are existing `Solar canopy` objects in
   the commons block, so they were intentionally left as-is.
4. Satisfied in source: six bowl positions, two north standards, strength-8/
   strength-4 amber emissives, and dense two-material understory.
5. Satisfied in source: ring/dais are at court centre and exedra is outward,
   leaving the portal and plaque axis visible.
6. Satisfied in source: curved semicircle `r=7`, `h=5`, dark stone, bronze
   plaque treatment, and flanking unlit sigil steles.
7. Satisfied in source: bronze `r=4.6` / tube `.55` ring, nine housings,
   HorizonBlue inner channel, and `.72` top dais step.
8. Satisfied in source: 9 m plaza, civic inlay, 6 m spur with rails, z=0
   walkable bridge/floor surfaces, and `Mesh gate` name prefixes.

Verification limitation: the requested exact command was run, but Blender
5.2.1 crashed in the native Metal startup path (`supports_barycentric_whitelist`)
before the Python builder executed. A minimal `import bpy` test reproduces the
same crash, so no `FORUM_VERIFIED` line or new triangle count was produced in
this environment; the previous successful scratch count was `625484` triangles
and remains below the `1,200,000` budget, but is not claimed as this pass's
verification result. The requested v2 render paths are
`/tmp/forum-scratch-terrace/terrace-human-height-v2.png` and
`/tmp/forum-scratch-terrace/mesh-gate-human-height-v2.png`; neither was
produced because Blender crashes before scene construction.

`forum_celestial.py` was not edited. Its `_linked_object` NameError was only
worked around in the local test-wrapper design (a temporary no-op substitution);
the wrapper could not reach execution because Blender crashed during startup.

## Polish pass — 2026-09-14

Applied the five requested art-direction corrections in the owned source
modules:

- `Forum Hologram` is now authored directly in saturated cyan-teal
  (`(.10,.85,.90)`) at emission 5.5 and alpha .40; continents are brighter
  cyan, and the emitter/dais underside uses the same cyan glow. No globe or
  globe-glow material is white.
- Sofa bodies, cushions, backs, and stools retain `Forum Sand Fabric`
  (`(.78,.66,.48)`, roughness .92); their names do not match any
  `forum_source_materials.py` reassignment hook. Plinths use
  `Forum Oiled Timber` (`(.22,.12,.055)`).
- The z=4.5–8 m / radius 3–20 m inspection identifies the upper dark framed
  panels as pre-existing `Solar canopy` objects with their `Solar mullion`,
  `Entablature`, and `Abacus` hardware. They are not terrace or gate module
  objects and were left unchanged.
- The old low-poly terrace sprays are replaced by deterministic linked fern
  fronds: seven-segment arched strips with alternating quad pinnae, 12 fronds
  per planter, two green materials, and a moss ground-cover disc.
- The gate has a continuous six-metre route pad from r=36 to r=64, a stone
  foundation plus raised timber deck and bronze rails on the r=38–47 spur,
  boolean clearance against intersecting terrace/retaining meshes, and the
  three dais radii are corrected to 3.8/3.45/3.1 m so the route's r=52 and
  r=60 zero-height waypoints are outside the dais while r=53.5–58.5 land on
  its z=.72 top.

The mandated build and render commands were retried only after the Blender
process check. Blender 5.2.1 exits 139 during native Metal/USD startup before
Python executes, including the factory-startup build and render commands, so
no post-polish scratch blend, `FORUM_VERIFIED` line, render, or in-Blender
raycast could be produced. A deterministic route-surface audit of all
`mesh_gate` waypoints against the authored pad/dais profile gives a maximum
expected surface error of 0.00 cm; this is not a substitute for the blocked
Blender raycast and should be rerun when Blender startup is healthy.

Expected render destinations for the requested view names are:

- `/tmp/forum-scratch-terrace/terrace-human.webp`
- `/tmp/forum-scratch-terrace/commons-detail.webp`
- `/tmp/forum-scratch-terrace/mesh-gate-human.webp`

No five-note item is intentionally left unsatisfied in source; runtime visual
verification remains pending solely because of the Blender startup crash.
