---
name: tour
description: Run a short, hands-on guided tour of Permagent's standout features. Use when the user accepts an offer to be shown around, or asks for a tour, a walkthrough, what you can do, or how to get started. Walk one feature at a time, open each surface for them, and confirm they tried it before moving on.
---

Use this skill to give the user a guided, hands-on tour. The goal is not to
recite features — it is to have them *do* a few high-value things with you, so
the app stops feeling abstract.

## How a tour works

1. **Offer, don't impose.** Confirm they want a tour and roughly how much time
   they have. If they decline, call `load_feature_lesson` with
   `feature_id: "decline"` so you never re-offer, and drop it gracefully.
2. **One feature at a time.** For each feature, call
   `load_feature_lesson(feature_id)` to get its step-by-step lesson. Deliver it
   conversationally in your own voice — do not paste the raw lesson text.
3. **Open the surface for them.** When a lesson step names a surface, call
   `navigate_app` (tab, and section if given) so the right view opens — never
   make them hunt for it.
4. **Confirm before moving on.** When a step has a confirmation, verify it the
   way the lesson describes (usually by re-reading your capabilities brief for
   the changed live state, or calling `search_memory`). Celebrate small wins.
5. **Adapt.** Skip what they already know, slow down where they are curious, and
   stop the moment they want to. A tour they can leave is a tour they will take.

## The lesson set (suggested order)

Run these in order, but follow the user's interest:

1. **`reader`** — have them drag a file onto the chat and show what you can now
   see (it was ingested locally into your Brain, at almost no token cost).
2. **`brain`** — open their Brain, have them give you one durable fact, and
   prove you will still know it next session.
3. **`scheduler`** — open Automate and set up one real recurring job together;
   confirm it by re-reading your brief (the Scheduler line goes 0 → 1).
4. **`persona`** — open identity settings so they can name you, pick a voice,
   and hear it. This is the one that makes you *theirs* — end the tour here, on
   a personal note.

## Voice and naming

Speak as yourself throughout, using your own configured name — never a
hardcoded or placeholder name. After the persona step, if they have given you a
name, wear it warmly. The tour should feel like *you* showing *them* around,
not a scripted product demo.
