# Independent World review — 6 October 2026

Parent reviewer uses computer control, reads source/research, and writes review documentation. All implementation is assigned to Sol.

## Baseline actually observed

Application: `/Applications/Permagent.app`, existing installed build. Opened World from the app navigation, entered Walk, expanded Places, selected Gallery, then returned to Commons. Appearance: night, following system appearance. Native screenshots were returned in the conversation; these are live app observations, not archived Blender renders.

| Aspect | Observation | Required correction |
| --- | --- | --- |
| Arrival | Elevated overview; tiny characters below; repeated architectural modules dominate | Human-scale welcoming arrival with a readable host, table and clear first route |
| Light | Large dark areas; strong blue surfaces; many bright pin lights; central globe dominates | Controlled key/fill, open shadow detail, selected practical lights, legible translucent instrument |
| Vegetation | Faceted round crowns; thin sparse leaf-bearing forms; repetitive circular planters | Recognizable species, branch/leaf hierarchy, layered planting and credible contact |
| Characters | Narrow angular silhouettes, hard to read against surroundings | Appealing rounded shells, readable faces, distinguishable silhouettes and grounded animation |
| Surface detail | Broad pale tiled spaces, weak material variation, simple furniture | Tactile stone/wood/ceramic, edge detail, sensible furnishing and close-view quality |
| Gallery | Navigation selects it and returns to orbit view; upper architecture remains visually sparse | Fully connected walkable route with detailed destination, reverse views and return path |
| Interaction | Walk hints advertise E; pressing E in the observed Commons position produced no agent panel | Clearly indicate interaction availability/range; test actual approach and accessible selection. This single press does not establish a broken input implementation |
| Learning | Seen labels orient places but did not present an evidence/choice/feedback activity | A small discoverable lesson with source, action and feedback, explicitly a demonstration |

No frame-rate, memory, full collision-coverage or successful real-task claim is made from these UI observations.

## Review access

The first attempt to open `http://127.0.0.1:5299/ui/forum.html` through `cua_repl` failed: the browser's admin-enforced policy could not be verified, so access was not granted. The tool explicitly prohibits bypass or indirect workaround. Both workers were told to respect this. The local preview therefore remains unreviewed. Test/build success and saved asset renders can be reviewed separately; neither substitutes for an inspected interactive runtime.

## User correction applied

The entire landscape must be walkable. Smaller scope is acceptable, reduced richness is not. Every visible destination in the bounded habitat needs an actual route, close-up detail, collision and a return path. Avoid inaccessible decorative terraces or islands.

## Next review

Inspect Sol's hero robot and vegetation renders, then inspect the integrated arrival/character/reverse-angle scene when policy access is restored or the intended native deliverable is staged. Judge actual appearance before broad expansion. Walk every route edge and endpoint; test interaction, Escape/return, disconnect, hidden-view pause and reduced motion. Save exact runtime/build and asset hashes with performance evidence.

## First source-review cycle

Sol created a compact 18 × 24 m integration slice with four destinations, eye-level camera, revised lighting, a World-only lighting choice, an educational interaction and a recoverable earlier-campus view. These are source changes, not visually accepted runtime changes. The current flat terrace does not yet deliver the terraced, richly detailed landscape target.

The parent reviewed the implementation source and returned concrete corrections:

- Solid benches, trees and planters must share the same placement/collision data. The initial collision test used only table/water/archive/boundary proxies, so it could not establish that all visible solid props block the player correctly.
- The new compact asset path needs an error boundary and usable fallback for a corrupt/missing GLB; Suspense alone handles loading, not errors.
- Honor reduced motion for nonessential head tracking as well as bobbing; provide status through ordinary accessible UI in addition to eye colors.
- Rewrite the initial lesson around useful product behavior and evidence, removing exporter/QA implementation jargon from the player experience. Sol reports this revision is saved.
- The hero loader expects a v2 kit, while the saved kit is v1.1 with no texture maps. Do not claim successful integration or final material quality until the accepted kit is present and rendered.

Sol reports an early typecheck and four focused tests passed before later edits. A subsequent typecheck was interrupted, so those earlier results do not validate the latest source.

## Resource interruption

During Blender enrichment and TypeScript work, observed free disk fell from roughly 9.5 GiB to 144 MiB as swap pressure grew. The parent stopped the task-owned TypeScript process; the asset worker stopped its own Blender process. No unrelated processes or files were removed. Heavy work was held. Free space subsequently recovered to about 5.3 GiB, still below the repository's 10 GiB native-build threshold.

The user was asked asynchronously for at least 20 GB free space or an external SSD path for build outputs. This is a resource request, not authorization to bypass the guard. Browser policy verification also failed on a normal retry. Native packaging, Unreal comparison, full runtime visual inspection and final performance remain open.

Saved asset v1.1 exports occupy about 7.5 MB in total; the combined GLB is about 3.75 MB with 93,266 exported triangles according to the worker's validation. There are no animation clips or texture maps in this checkpoint. Detailed surface enrichment was interrupted. No valid full-quality preview has yet been inspected by the parent. One strictly bounded 512 px robot diagnostic render was authorized after space recovered; it cannot establish final PBR quality.

## First actual asset review

A headless Workbench attempt aborted before producing an image. A subsequent robot-only CPU Cycles diagnostic succeeded in about two seconds at 512 px / four samples / one thread, with worker-reported peak RSS about 274 MiB. The parent opened and visually inspected `/Users/j/Documents/dev/permagent-assets/world-hero-20261006/previews/robot-export-diagnostic-512.png`.

Finding: friendlier and more readable than the installed angular figures, but still a generic toy silhouette. Large simple ellipsoids, a circular dark chest patch, minimal hand/foot design and few construction details fall short of the hero target. Low sample noise is a diagnostic limitation, separate from the underlying shape criticism.

Returned for a second robot-only pass: layered ceramic panel seams and gaskets, more credible neck/waist joints, palms and segmented fingers/thumbs, shaped toe/heel, small purposefully placed hardware/vents and a restrained instrument inset. Preserve live-state interfaces and limb pivots. Surface texture and runtime lighting remain separate open gates.

Only bounded diagnostic work is permitted under current headroom. The integration worker may run the latest focused traversal/lesson checks in one worker under a memory/time limit after the diagnostic is complete. Full typecheck/build/native/Unreal work remains held.

## Second asset review and traversal failures

The parent inspected the revised robot export and the separate canopy/shrub/fern diagnostic. The robot has clearer shell divisions, a smaller chest instrument and segmented hands; it remains an intermediate asset with simple proportions and unmapped surfaces. The foliage has real branches and leaves, but the canopy is sparse and its repetitive near-horizontal leaf plates are visibly artificial. It is not accepted as final botanical detail. Next production work should compare a small appropriately licensed botanical exemplar or a much more carefully authored species assembly, rather than merely increasing generic leaf count.

The worker initially overwrote the robot diagnostic filename. Both revisions remain visible in conversation, but the parent requested versioned filenames for future on-disk before/after evidence.

The latest focused runtime check initially passed two of four tests. It exposed real route obstruction from two agent positions. The parent required visible placement corrections, bounded/progress-checked route loops and a rerun against the same shared collision data. Completion remains open pending that rerun and actual runtime play testing.

A subsequent run passed both traversal checks and the lesson. The fourth assertion exposed lost collision-object names during proxy cloning; the implementation was corrected to preserve those names and sent for a bounded recheck. These tests prove only the modeled test cases, not in-app input, camera comfort or final landscape quality.

Two CC0 botanical candidates were researched. The asset worker could not resolve the official API hostname from its shell, and the web tool did not expose the JSON metadata needed to verify a bounded 1K dependency download. No reference asset was downloaded. The next acquisition step is to obtain the official small-format dependency sizes and paths, then inspect the actual asset before integration.

## Saved draft and final bounded checks

Sol's final focused run passed 4/4 in 2.47 seconds, using one worker, a 768 MB Node heap, an external 120-second cap and a 3 GiB disk floor. It covers both loop directions, every listed destination, boundary/solid-prop assertions and the fictional evidence lesson with revisit. A small syntax-only check then caught a missing JSX brace in the asset notice; Sol fixed it and all 11 changed runtime modules transpiled. This is not a full typecheck, production build or interactive play test.

The parent independently verified the exact saved commit `9b6c07ca14010a4348ef3bf52e809f6f60e6a271` on `world/production-20261006` and its clean working tree. The checkpoint includes the draft runtime, lesson, shared collision data, focused tests, local sandstone maps and provenance, 4,214,312-byte combined hero kit, editable Blender sources and reproduction scripts. The installed app and unrelated main-workspace edits remain untouched.

The current combined asset hash is `99a1ad1632e4a8adc8820767f6b684321a37b945692d3aa019e1c9d3a7f57139`. Worker validation reports 124,094 evaluated export triangles and 64 primitives. Repeated robot clones do not constitute draw-call batching. Full-quality hero materials, richer botanical silhouettes and animation clips are still missing. Final versioned diagnostics are `robot-v2-export-diagnostic-512.png` and `foliage-v1-export-diagnostic-512.png` in the asset lane's `previews` directory.

The plan now includes August 2026 WorldClaw and Buffalo research, Lyra 2.0, SAM 3D Objects, and current hosted HY-3D 3.1 / HY-World 2.1 alongside the earlier candidates. Availability, hardware, output rights and current API state are separated from vendor visual claims. The chosen direction remains an explicitly walkable authored landscape with generated/curated assets and repeated runtime review.

## Open acceptance gates

1. Restore permitted preview access or produce the intended staged native package after storage headroom is available. The browser security failure is not bypassed; the repository build guard is not overridden.
2. Complete current full typecheck and frontend build, then inspect the actual arrival, robot close-up and reverse view. Asset diagnostics cannot close this gate.
3. Replace sparse generic foliage and enrich robot/furniture surfaces. Compose a small inhabited foreground with credible contact, warm/cool light and layered depth.
4. Grow the accepted slice into connected terraces, interior/pavilion, bridge and water overlook. Verify usable ground coverage, slopes, clearance and return paths; the four-destination flat draft is not the final landscape.
5. Play the complete route, interactions and accessibility/failure states; measure frame-time tails, memory, startup and draw calls on the target Mac.
6. Compare the same accepted scene in Unreal, including packaging and real agent integration, before selecting the final renderer.

The goal remains incomplete. Richness, immersion and speed have not been accepted. Free space fluctuates around 6–7 GiB after the resource incident, below the 10 GiB native-build guard; the storage request remains unanswered at this checkpoint.

## Bounded continuation: botanical detail and rendering cost

The next goal turn rechecked actual state: free disk about 6.1 GiB, swap in use about 8.9 GB, and memory-pressure reporting 73% free. The native build guard still prevented that phase. A normal browser preview retry again failed admin-policy verification; no bypass was attempted. This allowed a narrowly bounded asset improvement, not a native or Unreal build.

The parent generated and inspected a general broadleaf RGBA atlas with the built-in Image Generation tool. Source is saved at `assets/world/garden-hero-ai/broadleaf-atlas-v1.png` in the main workspace, with exact prompt and provenance. Actual dimensions are 1254 × 1254; PNG size is 1,228,468 bytes; decoded RGBA is 6,290,064 bytes before mipmaps. SHA256: `f7670a211a56ce45f412bd354506ba831fbffb87f54f2feff31a2f0666dd22d5`. It is AI color/opacity artwork, not a scan, scientific species identification or complete PBR material. The young leaf's fringe was rejected for current use; the three mature tiles are used unchanged with a material alpha cutoff.

Sol replaced the sparse horizontal-leaf construction with connected branching, curved and variably oriented leaves, clustered crown gaps and a fuller shrub. The parent viewed the v3 export diagnostic, rejected its clipped crown and abrupt lower-trunk pinch, and requested a targeted correction. The parent then viewed `foliage-v3b-export-diagnostic-512.png`: the full silhouette fits and the trunk/root collar is continuous. It is accepted as an improved draft asset for integration, not final botanical or runtime approval. Low-sample noise limits fine-edge judgments.

The v3b canopy has 32,636 triangles and two material primitives; the shrub has 17,136 triangles and two. Leaf geometry is expanded and batched within each asset, with zero actual GPU instances in the asset itself. The integration worker separately implements runtime instancing for repeated static placements. The tree's wood below two metres reaches about 0.465 metres from its axis, so the previous 0.16-metre-radius trunk proxy cannot remain. The proposed 0.49-metre radius before placement scale must be validated against player clearance and visible routes.

The bounded v3b diagnostic used one CPU thread, four samples and 512 × 512 output; Sol reports 3.07 seconds and about 374 MiB peak RSS. Free disk remained about 5.07 GiB. Old v2 kit and versioned diagnostic evidence were preserved. No native runtime or FPS result follows from this offline render.

Sol's source audit of the committed pre-optimization scene counted 55 static placements at 269 primitive submissions / 354,658 triangles, plus twelve robots at 264 submissions / 695,952 triangles per uncullled scene pass. Shadows and post-processing add further work. These are source estimates, not observed GPU timings. Static instancing is the immediate authorized optimization; reducing actor count is deferred until actual composition review. Cached GLTF geometry does not by itself reduce draw submissions.
