# Environment and inhabitants polish — 2026-09-10

The browser Solar Forum now has collision-checked ambient walking routes, pauses,
smoother turns, and strides driven by distance traveled. Agents pause while their
real feed reports work or while hovered. Movement is ambient presence, not task
progress. Five full patrol routes were checked against the exported architecture.

Night uses individually twinkling stars and sparse shooting stars with varied
headings and timing. Day uses drifting procedural clouds, sunlight and six seabirds
with gliding and flapping motion. Reduced motion freezes ambient movement and
suppresses shooting stars.

The sovereign is the existing orchestrator (stable internal key `henry`), not a
new specialist. The configured identity supplies its display name. Its nameplate
stays visible in the forum overview, its role is identified in the agent desk,
and Find brings the camera to its current position. The old Agora dissolve state
cannot hide the body in this separate forum scene. A regression test covers
selecting a specialist, finding the sovereign, and changing its configured name.

Validation: TypeScript and the separate forum production build passed. All 39
forum tests and 29 pose/idle-life tests passed (69 unique tests, including daemon recovery of the configured name). Vite reports a
large scene bundle; test output also reports duplicate Three imports and the
existing React act deprecation. Live browser visual verification remains pending.

Unreal day/night atmosphere generation passed, including sky materials, lanterns,
clouds and birds. The native maps now include twelve skeletal inhabitants with authored Walk and
Idle clips, looping walking routes, pauses and a sovereign nameplate. All twelve
tracks were evaluated at an interior time in both maps and moved their bound
actors. This checks sequence evaluation, not an interactive gameplay playtest.
The name is a snapshot from the configured local daemon; native job telemetry
and live name-change events are not yet bound. Runtime navigation avoidance,
foot IK and more natural clip blending remain further realism work.

Native source: `scripts/blender/animate_forum_characters.py` and
`scripts/unreal/inhabit-forum.sh day|night`. The script imports FBXs, validates
character scale, creates per-agent sequences and verifies actor motion. It reads
the local daemon credential only for its intended authenticated identity request;
credentials are never written to map assets, reports or logs. Generic role text
is used when identity cannot be loaded. Browser identity retries after daemon
startup and refreshes on identity revision changes.

The existing bundled local daemon was restored through `permagent start`; its
launch service now points to `/Applications/Permagent.app/Contents/MacOS/permagentd`.
The configured persona is preserved. Browser preview access still uses the
existing paired-device authentication.

Sequencer API reference: [Epic Python Scripting in Sequencer](https://dev.epicgames.com/documentation/unreal-engine/python-scripting-in-sequencer-in-unreal-engine).

Preview: http://127.0.0.1:5284/ui/forum.html
