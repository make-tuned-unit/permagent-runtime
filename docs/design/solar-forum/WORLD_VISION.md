# A living civic garden for intelligence

The World should be a beautiful, inhabited place to build understanding and get
useful work done. Solarpunk futurism gives it a coherent set of values: abundant
light, shared infrastructure, visible stewardship, repairable tools and knowledge
that grows through participation.

Permagent's classical architecture gives those values a civic form. Its existing
stone, bronze and engraved-light palette makes the future feel substantial and
crafted. Gardens soften the institution; precise instruments and responsive
interfaces make its intelligence legible. The World belongs inside the macOS app,
so it needs no separate logo, splash screen or competing navigation system.

## Picture arriving

You open World in the morning. Neutral daylight falls across a pale stone forum.
Water gathers in a shallow basin. Bronze rails lead your eye toward a shaded
workshop and the library balcony above. The space has broad paths and places to
pause; important destinations are visible from the arrival point.

Your coordinating agent is recognizable by an ivory mantle and a measured manner.
Selecting them opens a familiar Permagent panel. You can see what they can help
with, ask a question, inspect current work, or prepare a brief. No conversation is
started just because you walked near someone.

A question about memory leads you upstairs to the archive. The Librarian, wearing
indexed folios and a high collar, explains what is known and shows its sources.
An artifact has a place on a shelf, but the panel also offers ordinary search,
copy, correction and navigation. You can understand the record without learning
a game control scheme.

From the gallery, you see a workshop below and an observatory above. The Forecaster
helps compare possible futures. A council gathering exposes competing arguments.
Neither the height of a room nor the confidence of a voice makes its claims more
certain; evidence stays available in the same interface.

When macOS changes to dark appearance, the forum becomes night. The stone recedes
into Permagent's dark blue background. Warm light pools at desks and beneath the
colonnade. Cyan is a fine line in the architecture, violet marks memory, and agent
identity colors remain consistent. There is no simulated rush hour or fake work
cycle tied to the visual change.

## Spatial design

| Layer | Feeling | Purpose | Architectural expression |
| --- | --- | --- | --- |
| Commons | Open, welcoming, readable | Meet, ask, orient, prepare a brief | Broad paved forum, low water, clear paths, civic desks |
| Working rooms | Focused, tangible | Follow execution and inspect results | Shaded bays, durable tables, visible artifacts and approval trays |
| Garden archive | Quiet, curious | Explore memory, sources and learning | Raised gallery, book/record niches, planted edges and reading benches |
| Observatory | Expansive, reflective | Compare scenarios and competing plans | Small elevated prospect, instruments, long views back over the forum |
| Mesh threshold | Connected, bounded | Understand collaboration and handoffs | A distinct gateway with visible ownership and connection state |

The current prototype spans about 134 meters, with the commons, a 4.32 m gallery,
an 8.26 m observatory prospect, a garden promenade and four outer courts. Browser
walk mode makes the campus explorable at human scale. Further work should make
the spaces less uniformly circular: a workshop
wing, an intimate courtyard, a long shaded passage, and a distant planted skyline
would create a richer sequence of reveal and arrival. Keep landmarks visible so
exploration does not turn into searching for controls.

## Play and discovery

Exploration should reward curiosity with a new view, a useful explanation or an
encounter. A five-minute walk can start at the fountain, follow the shaded gallery,
meet the Librarian, climb to the observatory and return through a garden. Longer
visits can branch into workshops and the outer promenade. The central fountain,
archive roof and observatory should remain recognizable landmarks at walking height.

Optional guided journeys can teach planning, memory, tool use and verification.
Each stop pairs a small interaction with an inspectable example; completing a
lesson can leave a personal field note in the archive. These are proposed learning
mechanics, not implemented rewards or simulated agent jobs. Product access must
never depend on completing a walk, collecting objects or maintaining a streak.

The next environmental pass should add changes in enclosure, sound and light:
water in the open forum, leaf shade along a narrow passage, a quiet reading court,
and an expansive view at the top. This sequence gives the campus a sense of scale
that increasing its radius alone cannot provide.

## What makes it agentic

Each meaningful visual should expose an answerable question:

- A task at a bench: who owns it, what was requested, what happened, and what is next?
- An artifact in the archive: what produced it, when, from which sources, and can I inspect it?
- A pending decision: what action is proposed, what could change, and what are my choices?
- An agent: what can it do, how does it work, which tools and context does it have, and what are its limits?
- A handoff: which agent delegated to whom, what scope traveled with the request, and who remains responsible?

Use actual daemon events for these answers. Decorative water, foliage and lighting
may provide ambience, but they do not indicate progress, health, approval, resource
usage or completion. Avoid turning every background metric into particles. A small
number of understandable signals is more useful and more beautiful.

A job should have a visible lifecycle: brief → accepted run → work → decision if
needed → result → verification → archive. A user can enter that lifecycle from the
spatial interface or from normal Permagent controls, with the same state and records.

## Inhabitants

All twelve real roster identities have distinct character briefs and Blender outfit
variants. Differences include proportion of clothing, shoulder profile, headgear,
carried instruments and working manner. Color supports identity but does not carry
it alone. The shared live skeleton and status palette remain the authority for
movement and activity: gray idle, amber working, cyan available, red error.

The direction is warm, capable and individual. Personality appears in explanation,
attention, gesture and choice of words. It should help people predict how to work
with an agent. It should not imply human feelings, hidden consciousness, capabilities
that do not exist, or certainty unsupported by evidence.

The current character briefs are presentation direction. They do not silently
replace persistent daemon identities or specialist system prompts. Ask can include
a role's visible presentation preference while clearly routing through the orchestrator.

## Aesthetic rules for the next passes

1. Build silhouette and spatial depth before adding surface detail. Every new level
   needs a readable route and a useful destination.
2. Keep vegetation lush at the edges and comfortable around places to sit. Preserve
   sightlines to agents, entrances and the central court.
3. Use warm stone, dark stone and bronze for most of the image. Light is engraved or
   housed in a fixture; each accent has a purpose.
4. Make the structures feel maintained: drainage, irrigation, joints, handrails,
   tool storage, solar infrastructure and shade should have believable placement.
5. Preserve calm negative space. Leave room for work and people rather than filling
   every surface with decorative machinery.
6. Use the shared app typography, controls, accessibility and state colors. All
   essential actions remain available without orbiting, walking or precision clicking.
7. Honor system appearance, reduced motion and inactive-view rendering. Beauty must
   coexist with agents doing work on the same machine.

## Next playable slice

The highest-value next milestone is a complete short journey: arrive, meet an agent,
ask about a capability, prepare a project-scoped job, receive a server-acknowledged
run, inspect an approval if needed, and open the verified result in the archive.

Before adding a larger city, verify that journey in both web and Unreal, including
keyboard access, cancellation, disconnect/reconnect, day/night appearance and frame
time on the target Mac. Unreal currently has the imported architecture and a
compiled first-person template; its rendered play experience and daemon bridge
still need implementation and testing.
