# Job 12: North-star art direction (Fable, 2026-09-14)

Reference: `docs/design/solar-forum/north-star.png` (Jesse's inspiration image).
Compared against the current source renders `campus-expanded.webp`,
`celestial-habitat.webp` and `commons-detail.webp` (631,562 triangles).

## What the reference actually shows

1. **Foreground: a warm terrace where agents gather.** Six agents sit and stand
   around a low circular council table with a glowing holographic globe. Warm
   timber and sand-coloured fabric, bronze fittings, amber lantern bowls, glass
   info panels tinted cyan, dense ferns and broadleaf planting between the
   seats, hanging vines and strand lights overhead, a balustrade at the cliff
   edge. The foreground is ~70% of the emotional weight of the image.
2. **Middle ground: a lagoon city.** Water is the floor of the world: a bright
   lagoon with islands, quays, boats and a lit harbour district. Buildings are
   *vertical* eco-towers with tiered garden balconies and glass fins, grouped in
   clusters of three to five, connected by sky bridges. One large glass dome
   holds a forest. Terraced gardens step down to the water.
3. **Far ground: mountains and waterfalls.** A rocky ridge on the far rim with
   waterfalls falling into the lagoon; low warm mist at the water line.
4. **Sky: monumental scale.** Two enormous orbital rings carrying planted
   terraces and towers, two floating islands with their own waterfalls, a gas
   giant, a second moon, star field, and a low golden sun with volumetric haze.
5. **Light: sunrise gold against deep blue.** Warm key from a low sun, cyan and
   teal emissives for holograms and glass, amber for lanterns. Nothing is flat
   white.

## What the current build has instead

- A flat green disc with eight identical stepped pyramids evenly spaced, a
  scattering of single trees and regularly spaced smooth boulders.
- A white marble commons with a small fountain, no seating that reads as a
  gathering, no hologram, no warmth, no planting mass at human height.
- One ring and two small floating pads in a void sky; no lagoon, no mountains,
  no waterfalls on the disc itself, no dome, no tower silhouettes.

## Direction, in priority order

Every item below is geometry authored in Blender modules executed by
`scripts/blender/build_solar_forum.py` (Blender Z-up, metres, routes at z=0,
commons centred at the origin, colonnade at r=17, upper gallery at z=4.32,
promenade r=30–38, courts at r=55, geodisc diameter 358.79 m). Browser-side
lighting/sky changes are listed at the end and belong to a separate worker.

### P0 — The gathering terrace (hero shot, `commons-detail` camera)

Replace the current water court + cascading-bowl fountain at the origin with a
council terrace while keeping the impluvium as a crescent reflecting pool on
the north side (so the "water at the centre" idea survives without blocking
sightlines):

- **Council table**: bronze dais r=3.4 m, h=0.45; oiled-timber table ring
  r=2.6/1.9, top at z=0.78; inset bronze hologram emitter r=0.9 at the centre.
- **Hologram globe**: emissive sphere r=1.15 at z=2.1, material
  `Forum Hologram` (cyan-teal `NEON_ACCENT`, emission strength 6, alpha 0.55);
  three thin lat/long rings around it (bronze-cyan, r=1.25/1.35/1.45, tilted
  0 / 35 / 70 degrees); a faint cone of light (open cone mesh, alpha 0.12)
  from the emitter to the globe.
- **Seating**: eight low curved sofa segments (arc r=5.2, width 1.1, seat
  z=0.42, back z=0.95) in `Forum Sand Fabric` (warm sand, rough 0.9) on timber
  plinths, with gaps at the four cardinal axes so the radial routes stay open.
  Four cushioned stools nearer the table.
- **Lantern bowls**: six bronze bowls r=0.32 with amber emissive cores between
  the sofas at z=0.55; two tall bronze lantern standards (h=2.6) at the north
  gap.
- **Info panels**: two glass slabs 1.4 × 0.9 × 0.03 on bronze stands, one at
  (-4.5, 3.5), one at (4.5, 3.5), material `Forum Hologram Glass` (alpha 0.35,
  faint cyan emission).
- **Planting mass**: between every sofa segment a timber planter with a dense
  fern/broadleaf cluster (use `leaf_spray` from forum_landscape with count ≥ 48
  and two leaf materials); four larger planters with 3 m broadleaf trees at
  the diagonals; trailing vines from the pergola ribs down to z≈4.
- **Overhead**: six bronze catenary cables between pergola columns across the
  commons at z≈6.2 with strand-light beads (small emissive amber spheres,
  r=0.05, every 0.6 m, linked duplicates).
- **Balustrade**: the commons south edge (the arrival stair side) gets a bronze
  rail with glass infill so the terrace reads as an overlook.
- **Keep**: the colonnade, solar canopies, gallery, thresholds, the agent
  spawn anchors and patrol routes. The four teaching tables move outward to
  r≈12.5 (they currently sit where the sofas go).

### P1 — Vertical eco-towers and clustered urban rooms

Remove the eight stepped pyramids (`Archive gardens`, `Research studios`,
`Civic residences`, `Learning ateliers` foundations and their copies). Author
three clusters instead, each a distinct "urban room" with its own plaza:

- **Spire cluster (NE, centred ~(95, 70))**: three slender towers 72 / 96 /
  118 m tall, footprint r=7–9, twelve-sided, with tiered garden balconies every
  6 m (alternating cantilever depth 1.6 / 2.4 m), vertical glass fins, a
  bronze crown lantern (emissive amber) and a planted terrace at the top.
  Connect the two shorter towers with a glass sky bridge at z≈40.
- **Habitat cluster (W, centred ~(-100, -30))**: four mid-rise stepped
  habitats 28–44 m tall with continuous green roofs, deep loggias, and one
  covered market hall between them; two sky bridges.
- **Dome conservatory (SE, centred ~(80, -95))**: geodesic dome r=28 m (icosphere
  subdivisions 3, edges as bronze lattice tubes r=0.18 via linked duplicates,
  faces as `Forum Architectural Glass`), on a stone drum h=4; a forest of eight
  large trees inside; a low glass gallery ring around the drum.
- Every tower balcony carries trailing vegetation (use `leaf_spray`, low
  count, linked duplicates) so the towers read as living at overview scale.
- The existing conservatory, maker hall, theatre and harbour stay.

### P2 — Lagoon, terraces, mountains, waterfalls (the landform)

- **Inner lagoon**: cut the meadow between r=62 and r=118 into a lagoon
  (water plane at z=-2.2) with three islands (r 14–22) carrying the destination
  courts on quays; keep the four radial bridges as timber quays over the water.
  Flatten and preserve every walk route in `patrolRoutes.json` and the
  harbour approach (radius 150 m flattening remains).
- **Terraced gardens**: between the promenade and the lagoon, three stepped
  garden terraces (each 1.2 m rise) with retaining walls of
  `Forum White Sandstone Masonry` and dense planting on each terrace.
- **Mountain ridge (N and NW rim, r 140–178)**: a fractured stratified ridge
  rising to 55–70 m (displaced ring of icospheres merged by boolean union or a
  displaced arc mesh), using the basalt strata materials; two large waterfalls
  (sheet meshes with the existing `Celestial waterfall mist` material) falling
  from ~40 m into the lagoon, plus mist discs at the base.
- **Boulders**: replace the evenly spaced smooth rim boulders with 5–7
  fractured outcrop groups (clusters of 4–9 irregular blocks, rotated, partly
  buried) using the mossy rock material.
- **Shell gaps**: close the black openings along the land/strata join by
  sharing boundary vertices between the meadow and the strata band; verify
  outward face winding (item 4 of `04-visual-review.md`).
- **Canopy density**: trees in groups of 5–12 with understory, not single
  scattered trees; no bare lawn larger than ~25 m across anywhere inside r=120.

### P3 — Celestial layer

- Second orbital ring at a different tilt (≈ 28°) and larger radius, both rings
  carrying planted terraces and tower silhouettes on their inner face at
  intervals; the existing ring gains six tower groups.
- Four floating islands instead of two, two of them large (r 40–60) with
  towers, waterfalls (sheets + mist) and their own lagoons, placed so they
  frame the north-west sky from the commons overlook.
- Keep everything beyond the geodisc non-walkable.

### P4 — The MESH gate (required by Jesse: the World leads to the MESH Agora)

The World's Mesh threshold must become a real portal the Sovereign walks
through. Product context: Mesh is Permagent's peer network (other Permagent
instances pooling compute and skills); the World bible (§3 A5) already
specifies an honest, dormant portal ring and a `MESH: NOT CONNECTED` plaque read
from `meshStatus.ts`. We author the gate now, and the browser transition (below)
reads the real status.

- **Location**: a dedicated gate court at the north-west, polar angle 135°,
  radius 56 m (Blender ≈ (-39.6, 39.6)), reached by a 6 m wide spur bridge from
  the promenade (r=38) to the court edge (r=47), on the same bridge/rail
  pattern as the four existing bridges. The court is a 9 m radius stone plaza
  with the `horizonBlue` civic inlay.
- **The gate**: a monumental ring r=4.6 m, tube r=0.55, standing vertical on a
  three-step dais, its plane perpendicular to the spur so a walker heading
  outward passes through the ring's plane at court centre. Bronze body with
  nine engraved chevron housings; a `horizonBlue` (#5599FF) inlaid channel on the
  inner face (emissive, low strength); a dormant inner disc is NOT authored
  (the browser draws the event horizon when connected).
- **Antechamber beyond**: a semicircular stone exedra (r=7, wall h=5, austere
  dark stone) opening away from the forum, with an engraved plaque reading
  `MESH` on the axis, two Chitin sigil steles flanking it (unlit engraved
  channels), and a single downlight housing. The floor continues 8 m beyond the
  ring so the walk-through has room, then a low parapet at the cliff edge.
- **Wayfinding**: the existing `MESH` threshold lectern at (10, 12) stays as
  the information stele but its wayfinding stele points to the gate court.
- Name every object with a `Mesh gate` prefix so the exporter's material batching
  keeps it identifiable, and add the gate court to `patrolRoutes.json` walk
  routes so `forumWalkRoutes.test.ts` grounds it.

### Budget and export rules

- Raise the builder assertion to `triangles < 1,200,000` and
  `bytes < 95,000,000`. Repeated props (balcony units, lattice tubes, strand
  lights, sofa segments, outcrop blocks, ring terraces) must be linked
  duplicates (`obj.data` shared) so glTF emits shared meshes; the exporter's
  per-material join still batches them.
- Deterministic: `random.seed` per module; no `bpy.ops` per object where a
  data-API path exists; no new bitmaps.
- Every new module is executed from `build_solar_forum.py` after
  `forum_landscape.py` and before `forum_source_materials.py`, and must
  compile on its own (`python3 -m py_compile`).
- Dry runs use `FORUM_SCRATCH=<dir>` so shipped assets are untouched; only the
  final integration build writes real assets, and only when no Unreal job is
  reading the FBX.

### Browser-side (separate worker, not Blender)

- `ForumSky.tsx`: add a gas giant (large banded sphere, low in the north-west
  sky, subtle limb glow) and a smaller moon; keep the galactic sky shader.
- `ForumLighting.tsx`: sunrise key (warm, low, ~15° elevation, from the
  north-west so the commons overlook sees it), cooler fill, exponential fog
  tinted warm at the horizon, bloom on emissives if the post pipeline exists.
- Mesh portal transition: crossing the gate plane while walking triggers the
  Agora state; `meshStatus` offline renders the antechamber honestly
  (`MESH: NOT CONNECTED`), connected renders the event horizon and peer count.

### Acceptance

Re-render `campus-expanded`, `geodisc-profile`, `commons-detail`,
`celestial-habitat` and a new `mesh-gate` human-height view; compare directly
against `north-star.png`. The job is not done because triangles went up.
