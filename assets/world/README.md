# Blender is the World authoring source

Open `blender/observatory-vault.blend` or a file in `blender/characters/` in
Blender. These are editable geometry/material source assets, not flattened images.
The app uses their exported GLBs in `ui/command-center/public/world/`.

Reproducible generation (from the repository root):

```sh
/Applications/Blender.app/Contents/MacOS/Blender --background --factory-startup --python-exit-code 1 --python scripts/blender/build_world_vault.py -- --render
/Applications/Blender.app/Contents/MacOS/Blender --background --factory-startup --python-exit-code 1 --python scripts/blender/build_world_characters.py
```

These generators replace **their own** outputs. If you edit a `.blend` manually,
save that art as a new version before regenerating; do not discard a hand edit.
`--factory-startup` runs in a separate process, never the operator's open scene.

Contract:

- Meters, Y-up GLB; Blender source remains Z-up.
- Bounded static geometry/material batches, no remote assets or textures.
- Character armor retains `schema=permagent.rigid-armor.v1`, `bone` (an existing
  `BONE_NAMES` entry), and `channel` (`metal`, `trim`, `visor`) custom properties.
- Character parts bind to the existing live skeleton. Identity, state lighting,
  task poses, Librarian tablet and real-event choreography stay in the runtime.
- Missing/invalid GLB falls back to the existing hall/rig. No blank World.
- New files require export/binding regression tests, actual render inspection,
  runtime performance and app-bundle checks before acceptance.

The procedural source is retained for fallback and staged migration. Future art
belongs in Blender, but migrating a working interaction is not permission to
replace its live behavior with a baked animation claiming fictitious work.

The current performance evidence is Chrome at 1440×1000, DPR1, static and camera
inspection views. Native WKWebView, sustained motion and local-inference contention
remain separate gates. A capped counter alone is not universal 60fps proof.
