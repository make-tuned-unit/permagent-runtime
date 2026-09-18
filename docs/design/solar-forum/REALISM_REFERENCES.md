# Garden habitat on a geological disc

Research and implementation direction, 2026-09-11.

The user's target is a rich, inhabited video-game world: a geological disc floating in space, with credible construction, dense places to explore, living agents and day/night atmosphere. Enlarging a lawn does not meet that target.

## User-supplied north star

[north-star.png](north-star.png) is the primary visual target supplied by the user. Judge the scene against its intimate furnished foreground, expressive inhabiting agents, layered greenery, warm directional light, reflective materials, cascading water, immense planted rings, floating landscapes, and celestial depth. The target is the real modeled world, not a replacement poster or a composited screenshot. Current geometry remains an intermediate pass.

## Architectural references

- [Heatherwick Studio — 1000 Trees](https://heatherwick.com/projects/buildings/1000trees/): planted structural columns and stepped building masses. Apply the relationship between construction, terraces and landscape, with visibly occupied intermediate levels.
- [Safdie Architects — Habitat 67](https://www.safdiearchitects.com/projects/habitat-67): stepped modules with daylight and garden terraces. Apply varied heights, recessed fronts and usable outdoor rooms.
- [PARKROYAL on Pickering — President's Design Award](https://pda.designsingapore.org/award-recipients/2013/parkroyal-on-pickering/): substantial planted sky gardens soften the building from street level. Planting must occupy real troughs and terraces instead of floating green blobs.
- [NASA — Retrofuturistic NASA Space Art](https://www.nasa.gov/image-article/retrofuturistic-nasa-space-art/): habitat art makes the enclosing structure and inhabited landscape readable together. Apply an identifiable disc silhouette, exposed underside, structural rings and the contrast with space. This is a fictional art direction, not a claim of physically viable open-air space habitation.

These references inform original procedural Blender geometry; no reference artwork or building model is copied into the runtime.

## Implemented modeling pass

Eight multi-storey garden habitat groups supplement the conservatory, maker hall, theatre, harbor and original civic courts. Each group has terrace slabs, piers, recessed glazing, bronze mullions, timber shading louvres, furnished workspaces, archive cabinets, planter troughs, balcony rails, roof orchards and photovoltaic pergolas. The established cardinal patrol corridors remain clear. Upper floors are visual architecture; unrestricted player access and navigation to every terrace are not yet implemented.

The island now has exposed geological strata tapering to a keel about 48 metres below the walking datum, with compression rings, radial ribs and recessed service lights. The oversized native water plane is constrained to the central lagoon. The source rendering set includes a profile view to inspect the floating-disc silhouette and a human-height view of the new district.

## Quality gates and limits

Inspect silhouette, street-level depth, readable construction thickness, planting distribution, grounded furniture, glazing and paving joins. Passing a geometry budget is not evidence of photorealism. Source-render screenshots are not screenshots of the running app. The current export ceiling is 900,000 triangles / 68 MB for this expanded environment; actual measured values are in the generated manifest. Device FPS and GPU memory still need measured app testing before production integration.

Realism work still needs richer PBR surfacing across the new architecture, more species and natural canopy density, interior lighting, authored destination behavior, and gameplay access to upper levels. The app adapter and release gates are documented separately in APP_INTEGRATION.md; decorative buildings do not imply new runtime capabilities.

## Integrated celestial pass

The original forum now sits beneath a planted orbital ring with two distant garden habitats and waterfall meshes. Civic canopy proxies are replaced with layered foliage. The source uses selective metre-scaled UV texture maps for masonry, strata, rock, timber and glass; source textures remain CC0 with their existing provenance. Opaque textures export as JPEG to limit payload. Latest measured export: 631,562 triangles / 55,425,876 bytes. This is an intermediate environment pass; gameplay performance, animated waterfalls and the full north-star visual target are not established by these numbers.
