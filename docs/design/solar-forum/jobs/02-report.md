# Runtime surface materials report

Implemented the bounded browser runtime surface pass in
[`ForumSurfaceMaterials.ts`](../../../../ui/command-center/src/components/world/forum/ForumSurfaceMaterials.ts).
The helper operates on the existing `THREE.Object3D` clone, so it does not
alter source geometry, collision meshes, or the GLB's shared source materials.

`applyForumSurfaceMaterials(root)` returns an idempotent cleanup function, and
`disposeForumSurfaceMaterials(root)` is available for callers that keep the
root instead of the returned callback. Each source material is cloned once per
surface profile within that root, preserving sharing between meshes with the
same profile. Cleanup restores original material references and disposes only
owned clones. Reapplying after cleanup creates a fresh owned set, which makes
React StrictMode mount/unmount cycles safe.

The pass classifies named and future materials into glass, water, metal, wood,
stone, earth, authored accent, or conservative generic profiles. Roughness is
bounded by the profile while retaining authored maps and normal scales. Named
standard glass is upgraded to `MeshPhysicalMaterial`, which is the existing
Three.js `MeshStandardMaterial` extension that provides transmission and IOR;
the existing GLB conservatory physical glass keeps its authored transmission.
Emissive and named inlay/display materials are left untouched.

Eligible profiles keep the built-in Standard/Physical shader and inject a small
deterministic value-noise modulation through `onBeforeCompile`. The vertex
shader passes world-space metres to the fragment shader; the profile frequencies
range from broad 5.6–8.3 metre cells to fine approximately 0.45–1.25 metre
cells. The variation is intentionally subtle, affects diffuse colour and
roughness only, and leaves emission and normal maps intact. The shader callback
is marked against duplicate injection and its cache key includes the helper
version and profile.

## Checks

Ran:

```text
cd ui/command-center
npx vitest run src/components/world/forum/ForumSurfaceMaterials.test.ts
```

Result: 4 tests passed. The tests cover shared clone identity, world-space
shader injection and repeat compilation, standard-to-physical glass conversion
with normal-map preservation, authored inlay preservation, material-array
restoration, disposal events, and StrictMode-style reapplication.

```text
cd ui/command-center
npx tsc --noEmit
```

Result: passed with exit code 0.

As a broader regression check, the forum directory suite ran 56 of 57 tests.
The existing harbor route test still fails at segment 7 near `z=135.04` while
walking the exported geometry; this is a collision/asset issue outside the
surface helper and the helper never mutates geometry or collision state.

## Integration

After the GLTF scene clone is created in `ForumAsset` in `ForumView.tsx`, call
the helper from the existing effect and return its cleanup:

```ts
useEffect(() => applyForumSurfaceMaterials(copy), [copy]);
```

Keep the existing collision registration effect separate. The helper must run
on the visual clone only; it does not need to see or mutate the collision
representation. The source material pass is designed for both the current
named GLB materials and expanded geometry with descriptive material or mesh
names.

Browser visual QA remains required after integration. Confirm glass readability,
water response, stone/timber roughness, and the absence of shader compile errors
in both day and night appearances. These unit checks verify material/shader
invariants; they do not establish photorealism or replace running-browser
inspection. No browser, Blender, or Unreal visual run was performed for this
bounded job.
