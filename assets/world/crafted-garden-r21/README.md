# Crafted workstation desk — r21

Original walnut/brass reference generated with the built-in image_gen tool. The exact prompt is in `references/desk-v1-prompt.txt`, image in `references/desk-v1.png`, and FAL input settings in `references/provenance.json`. The reference retains its generated alpha channel.

Hunyuan 3.1 Pro generated one PBR model at the custom 40,000-face setting. Completed request `01a11329-2853-78c2-ba9d-09e3fac30248` is saved in the user's FAL library. Original `raw/desk.glb` is 41,765,452 bytes, SHA256 `09dd7b0bf26729da07ef7c2158658ad21a372b26a99d897eacd662209717c3fd`. It is locally retained and Git-ignored. Verified HTTPS, exact response length, GLB header and SHA256 checks passed; a second run revalidated and reused the local original without generating anything.

Runtime `processed/desk-v1.glb` is 1,313,304 bytes, 12,000 triangles, one primitive and three embedded 2048² WebP PBR maps. SHA256 `6c23b7e65f9833756274ee1db5ffbafc3e14a4367bf3e71eedc37272c0bde6fc`. Normalized footprint is 1.9 × 0.89 × 0.72 m, floor contact at zero. Actual exported front/reverse renders in `previews/` show four joined legs, open knee space and a coherent tabletop. The generated rear repeats the shallow drawer; accepted as a double-sided writing desk. Packed editable Blender source is retained beside the GLB.

Measured usable tabletop is 0.88968–0.88983 m at the sampled tool/book contact points. Source uses one instanced batch for 11 desks, keeps role-specific tools, corrects floating book stacks/keyboard contact, and preserves the original procedural desks as independent loading/error fallback. Collision now covers the complete desk height and depth. Thirteen targeted tests passed in 32.84 s, including actual embedded PBR decoding, transformed collider containment, contact raycasts, all 26 routes and terrace geometry.

Native acceptance is pending. The next native review also refines the sun halo and planet shading. Source tests and isolated Blender renders do not establish app appearance or frame rate.

Estimated FAL cost is $0.675 for this completed job; total completed-trial estimate is $6.45 against the user's reported $25 credit purchase. This is an estimate, not an account billing balance. No additional credit purchase or pasted API key was needed.

Reproduce acquisition and runtime preparation from the repository root:

```sh
python3 assets/world/crafted-garden-r21/download_exports.py desk
node assets/world/crafted-garden-r21/prepare_runtime.mjs desk
```

Downloads never submit paid generations. Preparation uses existing locked local dependencies. The 10 GiB free-disk floor remains in force for fresh downloads and bounded native/render work.
