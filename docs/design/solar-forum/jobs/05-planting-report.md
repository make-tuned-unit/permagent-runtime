# Planting density and silhouette report

Updated only the `leaf_spray` function and the coastal tree generation loop in
`scripts/blender/forum_landscape.py`. The latest `garden-district.webp` showed
the previous foliage reading as isolated pointed triangles, while
`campus-expanded.webp` showed the larger campus as a sparse field of thin
trunks. The north-star image calls for rooted, layered vegetation with visible
leaf mass and branch structure, so this pass builds that relationship directly
in mesh data.

`leaf_spray` now emits folded lanceolate leaves with broad shoulders, a center
fold, varied length and width, and varied outward/upward orientation. Bases
are distributed through four vertical layers around the supplied center. The
function remains a single mesh-data construction path and keeps the existing
material and call sites, so terrace herb beds, conservatory specimens and
orchard crowns gain the same fuller silhouette without introducing green
spheres or particle clouds.

Coastal trees keep the existing 180 placement count and the exact route and
building exclusion predicate. Each surviving tree now has a deterministic
trunk offset, four angled primary branches, a 22-leaf lower canopy clump per
branch, and an 11-leaf raised clump that closes the crown while leaving the
branching visible. Heights are approximately 3.8–7.5 m and crown radii are
approximately 1.45–2.25 m before leaf spread, giving the trees a credible
campus scale.

The coastal leaf budget is bounded at `180 × 4 × (22 + 11) × 6 = 142,560`
triangles before route/building exclusions; the folded leaf mesh uses six
triangles per leaf. Branch cylinders add a small structural overhead while
keeping the full coastal foliage pass below the 250k target. The existing
builder seed (`random.seed(42)`) keeps placement and variation deterministic.

## Checks

Ran:

```text
python3 -m py_compile scripts/blender/forum_landscape.py
git diff --check -- scripts/blender/forum_landscape.py
```

Both checks passed. Blender, native tools, and exports were not launched. Root
should run the normal Blender build and inspect close, garden-district and
campus views for branch anchoring, route clearance, silhouette fullness and
the measured triangle count.
