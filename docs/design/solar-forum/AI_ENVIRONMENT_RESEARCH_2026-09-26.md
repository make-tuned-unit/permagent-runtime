# AI environment production research — 26 September 2026

## Decision for Permagent

Use Blender and Unreal as serious production tools, with Fable 5.1 driving an evidence-based iteration loop. The current pictures show an asset/art-direction problem as well as a possible renderer problem. Replacing Three.js alone will not fix primitive silhouettes, repetitive planting, empty spaces, weak scale cues, or flat materials. Nevertheless, Three.js is not a constraint: Jesse explicitly authorizes replacing it if it prevents the target quality. The deliverable is a beautiful, interactive World in the downloaded app, not an editor screenshot.

The most promising first move is a small, excellent arrival terrace that matches the reference's composition and material quality, then expansion using reusable assemblies. Prove a packaged Unreal runtime and real agent connection early if Unreal is the selected renderer. Do not spend the whole allowance building a beautiful scene that cannot reach the user.

## What current workflows actually demonstrate

1. **AI orchestration followed by real production editing.** World Labs' account of the IMAGE-BLASTER/fal workflow describes Claude Code coordinating image cleanup, world reconstruction, mesh generation, and audio. Static environment splats and extracted interactive meshes are separate assets, refined downstream in Blender and Unreal. This is a relevant working pattern, not independent evidence of a finished AAA game. [AI-Native 3D Pipelines](https://www.worldlabs.ai/labs/showcase/ai-native-3d-pipelines).

2. **Generated worlds still need gameplay systems.** World Labs' Robot Roommates uses Marble/Spark/Three.js for scenery, Rapier for physics, and Socket.IO for synchronization. The renderer does not inherently force a blocky style; interaction and world generation are separate engineering tasks. This vendor-built demonstration is a useful architecture example, not a visual acceptance test for Permagent. [Robot Roommates](https://www.worldlabs.ai/labs/showcase/robot-roommates).

3. **Art-directed procedural assembly beats indiscriminate detail.** Epic's Electric Dreams combines a small set of detailed scanned assets with hand-crafted areas and procedural assemblies, integrating lighting, geometry, and sound. Apply that principle to planted stone terraces, vines, shoreline, and architectural modules. This is a UE 5.2-era sample page: its old experimental-status and free-Megascans statements must not be treated as current licensing or support guidance. Do not download the whole sample on this disk. [Electric Dreams](https://www.unrealengine.com/electric-dreams-environment).

4. **Single-object generation is an asset supply tool.** Meshy's documented process includes generation, remeshing, PBR texturing, rigging, animation, and FBX/GLB export. Its own use-case framing includes prototypes and indie projects. Generated outputs need silhouette selection, topology/UV inspection, coherent material scale, collision and rig validation. Use for selected props or characters only if existing access permits; it is not proof that raw generated objects meet AAA quality. [Meshy game assets](https://docs.meshy.ai/en/webapp/guides/use-cases/game-assets).

5. **Image-to-world is useful but has distinct representations.** Marble offers splats, a visual triangle mesh, and a separate simplified collider. Its enhanced video may add details absent from the underlying 3D scene. Consider it for a vista or exploration experiment, but verify geometry, occlusion, lighting changes, collision, and close views in the actual runtime. Enhanced video is not evidence of a playable environment. [Marble world model](https://www.worldlabs.ai/blog/marble-world-model).

6. **Good source materials are available without paid generation.** Poly Haven's HDRIs, textures, and models are CC0. Reuse existing local materials first; keep source/license records for anything acquired. Asset licensing and website/API terms are separate. [Poly Haven license](https://polyhaven.com/license).

## Local visual diagnosis

The original reference is `docs/design/solar-forum/north-star.png`. Its defining traits are a furnished, intimate garden foreground; rounded expressive robots; warm sun against a cool space vista; enormous inhabited rings; cascading water; planting at several scales; and overlapping architectural depths. Preserve those relationships in traversable geometry. Do not reproduce the poster's branding as world geometry or replace the scene with the image.

Inspected archived renders: `assets/world/forum/forum-preview.png` and `assets/world/forum/unreal/unreal-sovereign-day.webp`. These are not fresh runtime captures. They show repeated circular forms, sparse or blob-like vegetation, simple/blocky figures, repetitive paving, and weak furnishing. The Unreal image additionally has large crushed dark regions. Obtain current captures before making claims about today's build.

Priority order, based on those artifacts:

1. **Composition and scale:** human-height arrival view, a welcoming table and agents, framed garden canopy, readable route toward a vast inhabited ring. Make near/middle/far depth clear before increasing asset count.
2. **Hero assets:** sculpt/model rounded robot forms in Blender, bevel manufactured edges, use smooth normals deliberately, articulated joints, eyes and readable expressions, coherent idle/turn/walk animation. Avoid stacked cube bodies and identical silhouettes.
3. **Vegetation:** recognizable species silhouettes, branches and leaf clusters, groundcover/shrubs/canopy/vines; controlled density and variation, plausible rooting and contact. Instance detailed assemblies, not random green primitives.
4. **Materials:** correct world-unit texture scale; roughness variation, normals and restrained weathering; believable stone, bronze, wood, ceramic and foliage. Inspect UVs and texture seams. Audit existing water's metallic setting: ordinary water is dielectric. Avoid baked specular highlights in base color.
5. **Light and atmosphere:** warm key, cool fill, open shadow detail, grounded contacts, depth haze, reflected light and water. Lock exposure for comparisons. Bloom cannot repair geometry. Existing web shadow coverage is large relative to map resolution; measure foreground shadow quality before choosing cascades or tighter coverage.
6. **Motion and sound:** water, foliage, subtle environmental movement, spatial ambience and responsive agents. Still-frame beauty must survive camera movement without shimmer, popping, ghosting or clipping.

## Renderer and delivery decision

`APP_INTEGRATION.md` identifies the shipping view as React Three Fiber/Three.js `ForumAppView`, with live daemon bindings. Unreal has separate scene assets and identity snapshots, but the documented browser event/approval bridge is not complete there. Revalidate this at startup. A GLB export does not carry Unreal's Lumen or Nanite into the web renderer.

Evaluate the same small hero scene at comparable camera, resolution, lighting intention and frame budget. Use actual performance/visual evidence to choose. If Unreal materially wins, implement the production runtime path, not merely an editor-launch button. A native packaged companion may be a practical route; investigate actual platform/window integration rather than assuming Unreal can simply be embedded into the existing web panel. Preserve agent selection, chat, real state updates, approvals, cancellation and return navigation. Record architecture choice and limitations.

Unreal packaging creates a standalone application; development/editor execution is not a distribution test. The delivered app must locate cooked content with relative paths and run without Unreal Editor, Blender, development servers or this Mac's `/Users/j` files. Verify process lifetime, focus/input, install/update, missing-asset recovery, offline behavior and supported hardware. Local clean staging is an initial check, not proof of every platform. [Epic packaging](https://dev.epicgames.com/documentation/unreal-engine/packaging-your-project).

Current UE 5.8 macOS documentation lists software Lumen on M1+, Nanite/VSM on M2+ as beta, and hardware Lumen/MegaLights on M2+ as experimental. It recommends 32GB memory, with 16GB minimum. Detect this Mac's chip/memory and benchmark; do not assume all users have it or enable every expensive setting blindly. Provide a high-quality default and measured fallback tiers. [Epic macOS requirements](https://dev.epicgames.com/documentation/en-us/unreal-engine/macos-development-requirements-for-unreal-engine).

## Iteration and evidence

Use a fixed camera suite: arrival hero; close agent/table; garden path; ring/skyline vista; water/shore; high-level navigation; night arrival. Add a repeatable human-height walk with interaction. Save camera transforms, resolution, quality tier, exposure, build/asset hashes, frame-time distribution, memory, cold launch time and package size with each pass. Compare baseline/reference/current side by side; critique actual visible shortcomings before editing.

Score composition/depth, materials, foliage, character appeal, lighting, motion stability, interaction and delivery separately. Numeric scores are subjective aids, never a substitute for screenshot evidence or user judgment. Change the three highest-impact deficits, then recapture. Keep the last known good version if a pass regresses. Do not declare “AAA achieved” solely because an agent gives itself a high score.

Start with an explicit provisional 30fps floor at the actual World panel resolution on measured supported hardware, aiming for 60fps where feasible. Record p50/p95 frame time and long hitches, not just average FPS. Quality must scale gracefully without reverting to a primitive blockout. Keep essential task actions accessible and stop rendering when hidden.

## Research limits

These sources document real tools and demonstrations, not a verified one-prompt process for producing a complete AAA game. The production recommendation combines those documented capabilities with inspection of Permagent's source and artifacts. No paid service, new asset purchase, or cloud GPU is required to start. Fable's exact requested model identifier is `claude-fable-5-1`, verified against [Anthropic's model documentation](https://platform.claude.com/docs/en/models/fable-5-1/overview).
