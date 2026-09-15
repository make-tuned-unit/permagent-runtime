# Render review: remaining realism work

2026-09-11. Directly inspected `assets/world/forum/campus-expanded.webp` and `geodisc-profile.webp` from the completed 525,340-triangle source model. These are Blender renders, not browser evidence.

The enlarged architecture and deep disc are present. The north-star standard is still unmet. The next modeling job must address these observed issues:

1. Buildings repeat the same stepped-pyramid silhouette. Add varied massing and clustered, connected urban rooms; preserve the original civic forum.
2. Planting is too sparse and foliage appears nearly leafless at overview scale. Use fuller, layered canopies, understory and trailing vegetation grounded in real beds.
3. Perimeter boulders remain regularly spaced and smooth, like props on a rim. Replace repetition with coherent fractured outcrops and continuous eroded geology.
4. Black openings along the land/strata join expose inconsistent boundary heights. Close the shell using shared boundary vertices, verify outward face winding and eliminate gaps between layers.
5. The aerial view has large empty lawns and visually flat, pale surfacing. Add destination clusters, constructed ground detail and physically scaled material variation, guided by human-height views.
6. The source scene lacks the monumental rings, distant habitats, waterfalls, celestial depth and warm atmospheric contrast shown in the north-star image. Job 01 covers part of this; rendering and lighting remain an integration task.
7. The foreground agent gathering space must receive its own close-range furniture, foliage, lighting and animation pass; overview density alone is insufficient.

Do not mark the visual goal complete based on this export or the triangle count. Re-render the same overview/profile and a human-height gathering view after changes, and compare directly against the user reference.

## Collision regression

The expanded GLB passed 11/12 route checks; harbor travel failed around z=135 because terrain interpolation rose into the quay approach. The source path-flattening radius is corrected from 138 to 150 metres to protect the entire approach. Rebuild the GLB and rerun `npm test -- src/components/world/forum/forumWalkRoutes.test.ts` before accepting the correction.
