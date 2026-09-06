# Apple Liquid Glass — implementation-grade reference for Permagent

**Lane A-R1 research deliverable. Untracked; do not commit.**
Compiled 2026-09-01. Sources cited inline. No code was changed and nothing was built to produce this.

**Who this is for:** anyone doing a screen-by-screen visual upgrade of `ui/command-center`. It answers three
questions in order: *what is Apple's current design language actually made of*, *what of it can our stack
physically produce*, and *what rules should the upgrade follow so the result reads as native restraint rather
than as a glassmorphism theme off Dribbble*.

**A warning about the sources.** Apple publishes almost no numbers. The HIG and the WWDC transcripts describe
Liquid Glass qualitatively — "larger radius", "slightly taller", "increases opacity" — and the entire corpus of
primary material contains exactly one hard alpha value (the 35% dimming layer, §1.3). Every px/ms/alpha figure
in this document is therefore labelled by provenance:

- **[APPLE]** — stated in Apple documentation, a WWDC transcript, or Apple release notes.
- **[MEASURED]** — third-party pixel teardown or reverse engineering of shipping Apple software.
- **[OURS]** — a value this document proposes for Permagent. Defensible, but ours, not Apple's.

Do not launder an [OURS] number into a design review as "what Apple does".

---

## 0. Our starting position

| Fact | Value | Source |
| --- | --- | --- |
| Frontend | React 18 + Vite 5 + Tailwind 3.4, `ui/command-center` | `ui/command-center/package.json` |
| Tauri shell | `ui/desktop/src-tauri`, tauri 2.11.0, `features = ["unstable"]` | `ui/desktop/src-tauri/Cargo.toml` |
| Window config | `decorations: true`, no vibrancy, no transparency, no titlebar overlay | `ui/desktop/src-tauri/tauri.conf.json` |
| Distribution | DMG + `.app`, Developer ID signed, self-updating. **Not Mac App Store.** | same |
| Min macOS | `"minimumSystemVersion": "11.0"` | same |
| Webview engine | WKWebView (WebKit). Not Chromium. This constrains §3 heavily. | Tauri on macOS |
| Existing type ramp | 8 roles, 32/20/16/14/13/12/11px, Inter + Manrope, not SF | `src/styles/tokens.ts` |
| Existing radius scale | `4 / 6 / 8 / 12 / 16 / 999`, enforced by a test | `src/styles/tokens.ts`, `radiusScale.test.ts` |
| Existing motion tokens | `duration = {fast:160, base:200, slow:320}` + an `ease` map | `src/styles/tokens.ts` |
| Existing glass usage | ~30 files with ad-hoc `backdropFilter`, values ranging `blur(4px)` → `blur(24px) saturate(140%)` | `grep backdrop src/` |

Two things follow immediately. First, we are **not App Store constrained**, which unlocks the private-API paths
in §3.1 that most Tauri apps have to refuse. Second, we already have ~30 uncoordinated glass surfaces — the
upgrade's first act is consolidation, not addition.

---

## 1. Liquid Glass — the actual substance

### 1.1 What Apple says it is

Introduced at WWDC25 (June 2025), shipping in iOS 26 / iPadOS 26 / macOS Tahoe 26 / watchOS 26 / tvOS 26 —
Apple's first genuinely unified cross-platform design language.

> "The new design features an entirely new material called Liquid Glass. It combines the optical qualities of
> glass with a fluidity only Apple can achieve, as it transforms depending on your content or context."
> — Alan Dye, VP of Human Interface Design

> "This translucent material reflects and refracts its surroundings, while dynamically transforming to help
> bring greater focus to content." … "Liquid Glass uses real-time rendering and dynamically reacts to movement
> with specular highlights." … "Its color is informed by surrounding content and intelligently adapts between
> light and dark environments."

Source: <https://www.apple.com/newsroom/2025/06/apple-introduces-a-delightful-and-elegant-new-software-design/> **[APPLE]**

The operative words are *refracts* and *real-time*. Every previous Apple translucency (Yosemite vibrancy,
iOS blur, `NSVisualEffectView`) **scattered** light — it sampled the backdrop, blurred it, and tinted it.
Liquid Glass **bends** it. That difference is the entire reason a plain `backdrop-filter: blur()` reads as
"2014 frosted glass" and not as Liquid Glass, and it is the crux of §3.

### 1.2 The three optical layers

Apple decomposes the material into three compositional layers:

1. **Highlight** — "light casting and movement". A discrete lighting layer: "light sources inside of this
   environment shine on the material producing highlights that respond to geometry." On events (device unlock,
   window activation) "these lights move in space, causing light to travel around the material, defining its
   silhouette."
2. **Shadow** — "added depth for separation between foreground and background".
3. **Illumination** — "the flexible properties of the material"; on touch, "the material illuminates from
   within as a form of feedback. Starting right under your fingertips, the glow spreads throughout the element
   and onto any Liquid Glass elements nearby."

Sources: WWDC25 session 219 "Meet Liquid Glass" <https://developer.apple.com/videos/play/wwdc2025/219> **[APPLE]**;
CSS-Tricks teardown <https://css-tricks.com/getting-clarity-on-apples-liquid-glass/>

Lensing is described as: the material "dynamically bends, shapes, and concentrates light in real time",
giving controls "definition against the background content while still feeling visually grounded."
Materialization is not a fade — elements appear by "gradually modulating the light bending and lensing,
ensuring a graceful transition that preserves the optical integrity." **[APPLE]**

Note the second half of the illumination quote: glow "spreads … onto any Liquid Glass elements nearby". Glass
elements are not independent; they share a lighting environment. That is what `GlassEffectContainer` exists
to enforce (§1.6).

### 1.3 Variants

Source for this whole subsection: <https://developer.apple.com/design/human-interface-guidelines/materials> **[APPLE]**

**Regular** — the default, and what you should use unless you can justify otherwise.
> "The regular variant blurs and adjusts the luminosity of background content to maintain legibility of text
> and other foreground elements."
> "Use the regular variant when background content might create legibility issues, or when components have a
> significant amount of text, such as alerts, sidebars, or popovers."

It is fully adaptive: darker over dark backdrops, lighter over light. Anything may be placed on top of it.

**Clear** — high translucency, **no adaptive behaviour**.
> "Highly translucent, which is ideal for prioritizing the visibility of the underlying content."
> "Use this variant for components that float above media backgrounds — such as photos and videos."
> "Only use clear Liquid Glass for components that appear over visually rich backgrounds."

Clear ships with a hazard, and Apple's mitigation is the only hard alpha number in the corpus:
> for bright backgrounds, "consider adding a dark dimming layer of **35% opacity**" **[APPLE]**

Community distillation of the three preconditions, all of which must hold before Clear is legitimate: the
element sits over media-rich content; the content is not harmed by a dimming layer; the content *on* the glass
is bold and bright. <https://github.com/conorluddy/LiquidGlassReference>

**Identity** — no effect. Exists so glass can be conditionally switched off without a branch in the view tree.

**For Permagent: use Regular semantics everywhere. We have one place that could justify Clear** — HUD controls
floating over the 3D world view (`components/world/`) — and even there, prove it against a busy scene first.

Separately, the pre-existing non-glass material family (`ultraThin` / `thin` / `regular` / `thick` /
`ultraThick`) still exists and is for **content-layer** surfaces. Do not confuse them with Liquid Glass; they
are the old vibrancy materials and are the correct thing for a panel that is part of the content.

### 1.4 Tint

Source: <https://developer.apple.com/design/human-interface-guidelines/color> **[APPLE]**

> "Liquid Glass has no inherent color, and instead takes on colors from the content directly behind it."
> "Apply color sparingly to the Liquid Glass material, and to symbols or text on the material."
> "Refrain from adding color to the background of multiple controls."

Apple's own wrong/right pair is several tinted buttons (wrong) against exactly one emphasized action (right).
When you do tint, tint the *background* of the primary action rather than its label, "to draw attention and
elevate their visual prominence."

Two size-dependent behaviours worth knowing because they explain the visual system:

- **Small glass** (toolbars, tab bars): symbols and text "follow a monochromatic color scheme, becoming darker
  when the underlying content is light, and lighter when it's dark" — an automatic polarity flip.
- **Large glass** (sidebars): uses *increased opacity* "to preserve legibility over complex backgrounds", and
  deliberately does **not** flip, because "their surface area is too big and transitions like these would be
  distracting." (WWDC25/219) **[APPLE]**

That second rule is directly load-bearing for us: **the bigger the glass surface, the more opaque it gets.**
A full-height sidebar is not "more transparent because it's more glass" — it is *less*.

### 1.5 The layer model — the single most important rule

> "[Liquid Glass] forms a distinct functional layer for controls and navigation elements — like tab bars and
> sidebars — that floats above the content layer, establishing a clear visual hierarchy between functional
> elements and content."

> **"Don't use Liquid Glass in the content layer."** — creates "unnecessary complexity and a confusing visual
> hierarchy."

Sources: HIG materials + layout pages **[APPLE]**

WWDC25/219 gives the concrete counter-example: applying Liquid Glass to a table view would "make it compete
with other elements and muddy the hierarchy". Glass is "best reserved for the navigation layer that floats
above the content of your app." **[APPLE]**

WWDC25/356 adds the mechanism: "Elements using Liquid Glass require clear separation from content to maintain
legibility… controls sit on top of a system material, not directly on content. Without that separation,
contrast can suffer." **[APPLE]**

The only sanctioned exception is transient interactive elements embedded in content — a slider, a toggle.

**Corollary that most web recreations get wrong:** cards are content. Lists are content. A chat message bubble
is content. Modal bodies are content. Glass belongs to the toolbar, the sidebar, the floating command bar, the
popover chrome, the tab strip — and stops there.

**Background extension view.** When content doesn't fill the width behind a floating control, Apple's answer is
not to make the control opaque — it is to *mirror adjacent content underneath it* so the glass has something
plausible to refract. The sidebars page: the extension "mirrors adjacent content to give the impression of
stretching it under the sidebar." **[APPLE]** This is worth stealing conceptually; it's why Apple's sidebars
look right even over a plain background.

### 1.6 Concentric corner radii

Three shape families (WWDC25/356) **[APPLE]**:

1. **Fixed** — constant corner radius.
2. **Capsule** — "a radius that's half the height of the container."
3. **Concentric** — "calculate their radius by subtracting padding from the parent's."

The precise rule, from the SwiftUI API reference for `Edge.Corner.Style.concentric`:

> "When a corner is concentric to its container, the system calculates the corner radius to equal the container
> shape's corner radius minus the distance between corners." If the result would be negative, "the corner is
> square."

<https://developer.apple.com/documentation/swiftui/edge/corner/style/concentric> **[APPLE]**

So: **`r_inner = max(0, r_outer − padding)`**. A 16px-radius panel with 12px padding contains 4px-radius
children. The same panel with 20px padding contains square-cornered children — and that is correct, not a bug.

The design intent (WWDC25/356): "By aligning radii and margins around a shared center, shapes can comfortably
nest within each other." And the failure mode Apple names explicitly: corners "that feel too pinched — or
flared. They can create tension and break the sense of balance." **[APPLE]**

APIs: `RoundedRectangle(cornerRadius: .containerConcentric, style: .continuous)`, the new `ConcentricRectangle`
shape, and `.containerShape` to declare the reference shape. Apple's framing: "Many of our controls have their
corners aligned perfectly within their containers, even if the container is your iPhone!" (WWDC25/323) **[APPLE]**

Device-edge rule (WWDC25/356) **[APPLE]**:
- Phone: "Use a capsule with extra margin to create space near the screen edge."
- **iPad/Mac: "Use a concentric shape that aligns with the window edge for better balance."** ← ours.

**Glass grouping.** `GlassEffectContainer` merges multiple glass shapes into one sampling region. Apple:
"glass elements in different containers will result in inconsistent behavior"; grouped elements "share their
sampling region, providing a consistent visual result." Its `spacing:` parameter is the distance within which
sibling elements visually blend/morph — e.g. `GlassEffectContainer(spacing: 40.0)`. **[APPLE]** / reference impl
<https://github.com/conorluddy/LiquidGlassReference>

And the custom-control rule, verbatim (WWDC25/356): **"make sure to apply the material directly to the control,
not its inner views."** **[APPLE]**

### 1.7 macOS 26 Tahoe specifics

Primary source: WWDC25 session 310 "Build an AppKit app with the new design"
<https://developer.apple.com/videos/play/wwdc2025/310> **[APPLE]**

**Windows.** "Windows with toolbars now use a larger radius, which is designed to wrap concentrically around
the glass toolbar elements. Titlebar-only windows retain a smaller corner radius, wrapping compactly around the
window controls." General principle: "each element is designed with a curvature that sits neatly within the
corner radius of its container." **[APPLE]**

Apple publishes no point value. Third-party measurement puts Tahoe's standard window radius far above
Sequoia's — the widely-circulated figure is **26pt for Tahoe vs 10pt pre-Tahoe**, overridable via
`defaults write -g NSConvolutionOverride1 -float 10` **[MEASURED]**
(<https://forums.macrumors.com/threads/want-the-bigger-window-corner-radius-back-come-inside.2484898/>).
Jeff Johnson independently confirms the radius is *not uniform* — it changes with the presence of a toolbar,
and in Notes it changes dynamically during window setup **[MEASURED]**
(<https://lapcatsoftware.com/articles/2026/3/1.html>, <https://mjtsai.com/blog/2025/10/16/tahoe-window-corners/>).

The radius got large enough to cause a real usability regression: the 19×19px window-resize hit target now sits
~75% *outside* the window at the corner, versus 62% inside with square corners — Gruber's write-up of Heger's
analysis **[MEASURED]** (<https://daringfireball.net/2026/01/resizing_windows_macos_26>). Cited here as a
cautionary note: a big radius is not free.

**Toolbars.** "Toolbar elements are placed on a glass material, and the entire toolbar appears to float above
the content." AppKit auto-groups toolbar buttons onto one glass element, splitting different control types
into separate glass pieces. HIG: "Window titles can display inline with controls, and toolbar items don't
include a bezel." Hard requirement: "Make every toolbar item available as a command in the menu bar." **[APPLE]**

**Sidebars.** A sidebar is "a pane of glass that floats above the window's content." An *inspector* is
different: "edge-to-edge glass that sits alongside the content." AppKit developers are told to **remove legacy
`NSVisualEffectView`s from sidebars** "since they prevent the glass from showing through." Scroll views "now
extend beneath the sidebar by default." HIG adds: don't put "critical information or actions at the bottom of a
sidebar", because people hide lower sections when resizing. **[APPLE]**

**Controls.** Five sizes now: mini, small, medium, large, **extra-large** (new; "ideal for showcasing the most
prominent actions in your application"). Mini/small/medium are "now slightly taller" than pre-Tahoe and keep a
**rounded-rectangle** shape "which preserves horizontal density". Large and extra-large "round out into a
**capsule** shape." **[APPLE]**

That is a genuinely useful rule for a dense desktop app: **capsules are for prominent, large controls only.
Dense toolbar/table controls stay rounded rectangles.** A UI where every button is a pill is not Tahoe; it's
iOS cosplay.

**Menu bar** is transparent by default in Tahoe, with a gradient scrim dimming the wallpaper behind it for
legibility. Toolbar button groupings are now always visible rather than hover-revealed. **[MEASURED]**
(<https://www.macstories.net/stories/macos-26-tahoe-the-macstories-review/2/>)

Apple's own one-line summary in the macOS Tahoe 26 release notes: "Apps with Liquid Glass sidebars and toolbars
reflect and refract what you're viewing, drawing more focus to your content." **[APPLE]**
(<https://support.apple.com/en-us/122868>)

### 1.8 Scroll edge effects, and how bars behave on scroll

This is the mechanism that replaces the old "nav bar gets a background when you scroll" hack.

`ScrollEdgeEffectStyle` <https://developer.apple.com/documentation/swiftui/scrolledgeeffectstyle> **[APPLE]**:

- `.automatic` — "applied automatically when pinned content overlaps scrolling content."
- `.hard` — "a linear, nearly opaque boundary between pinned controls and scrolling content."
- `.soft` — "a subtle, blurred boundary between pinned controls and scrolling content."

WWDC25/356 assigns them by platform: **"Soft is the default and the one you'll use in most cases, especially on
iOS and iPadOS… Hard is mostly used on macOS."** **[APPLE]** ← note that. On a Mac, the correct treatment under
a pinned toolbar is the *hard*, nearly-opaque edge, not a soft gradient.

Further rules **[APPLE]**:
- "Apply one scroll edge effect per view." In split views each pane may have its own, "just keep them
  consistent in height to maintain alignment."
- "Scroll edge effects are not decorative. They don't block or darken like overlays. They simply clarify where
  UI and content meet, and shouldn't be used where there aren't any floating UI elements."
- HIG toolbars page: prefer this over rolling your own — "Any custom backgrounds and appearances you use might
  overlay or interfere with background effects that the system provides."

**Tab bars.** "A tab bar floats above content at the bottom of the screen. Its items rest on a Liquid Glass
background that allows content beneath to peek through." On iOS the bar minimizes on scroll
(`.tabBarMinimizeBehavior(.onScrollDown | .automatic | .never)`) and un-minimizes on tapping a tab or scrolling
to top. **[APPLE]** This is an iOS behaviour; macOS uses sidebars, not tab bars, and we should not import it.

WWDC25/356's hierarchy rule for bars is the transferable one: "Avoid placing screen-specific actions here — a
checkout button, for example, belongs with the content it supports. Mixing elements from different parts of the
UI can blur hierarchy and make it harder to distinguish what's persistent from what's contextual." **[APPLE]**

### 1.9 What Apple explicitly warns against

Verbatim, all **[APPLE]**:

| Warning | Source |
| --- | --- |
| "Always avoid glass on glass. Stacking Liquid Glass elements on top of each other can quickly make the interface feel cluttered and confusing." | WWDC25/219 |
| "Don't use Liquid Glass in the content layer." | HIG materials |
| "Use Liquid Glass effects sparingly… Limit these effects to the most important functional elements in your app." | HIG materials |
| "Only use clear Liquid Glass for components that appear over visually rich backgrounds." | HIG materials |
| "Refrain from adding color to the background of multiple controls." | HIG color |
| "Avoid applying a similar color to toolbar item labels and content layer backgrounds." | HIG toolbars |
| "Any custom backgrounds and appearances you use might overlay or interfere with background effects that the system provides." | HIG toolbars |
| "glass elements in different containers will result in inconsistent behavior" | WWDC25/323 |
| corners "that feel too pinched — or flared. They can create tension and break the sense of balance." | WWDC25/356 |
| "make sure to apply the material directly to the control, not its inner views." | WWDC25/356 |
| "Instead of relying on decoration, hierarchy should be expressed through layout and grouping." | WWDC25/356 |
| "If you've customized your bars, now's the time to clean them up… with the new system appearance, we're all relearning where emphasis comes from, making customizations like these unnecessary." | WWDC25/356 |

The last two are the ones a web team is most likely to violate, because on the web *everything* is a custom
control and decoration is the default tool.

### 1.10 Where Apple LANDED — the legibility retreat

This section matters more than §1.1–1.9 for our purposes. The Liquid Glass that shipped in June 2025 is not
the Liquid Glass a user sees in September 2026. **Design to where it landed.**

**26.0 (Sept 2025).** Ships as unveiled: fully translucent by default, no first-class intensity control, only
the pre-existing accessibility toggles. Immediate and sustained backlash over legibility — transparent toolbars
and notifications over busy content, animation-induced eye strain, inconsistent contrast. Press comparisons to
Windows Vista Aero. Apple's own reviewers-of-record flagged it: MacStories found that in Music and Photos
"controls often get lost in your sea of images", and argued apps should move toward "the Finder and Safari
design pattern" of more opaque toolbars **[MEASURED]**
(<https://www.macstories.net/stories/macos-26-tahoe-the-macstories-review/2/>).

**26.1 (Nov 2025) — the retreat.** Apple added a first-class, non-accessibility appearance setting:

> "Liquid Glass setting gives you the option to choose between the default clear look, and a new tinted look
> that increases the opacity of material in apps and notifications on the Lock Screen." **[APPLE]**
> — iOS 26.1 release notes, <https://support.apple.com/en-us/123075>

> "Liquid Glass setting gives you the option to choose between the default clear look or a new tinted look
> which increases opacity of the material in apps." **[APPLE]**
> — macOS Tahoe 26.1 release notes, <https://support.apple.com/en-us/122868>

Location on Mac: System Settings → Appearance. On iPhone: Settings → Display & Brightness → Liquid Glass.
Practical effect: previously-transparent buttons and chrome get a white/dark tint that stops the background
showing through **[MEASURED]** (<https://www.macrumors.com/2025/11/03/apple-releases-macos-tahoe-26-1/>).

Critically: **if either Reduce Transparency or Increase Contrast is on, the Liquid Glass control is disabled —
accessibility overrides it** **[MEASURED]**
(<https://eclecticlight.co/2025/11/05/appearance-revisited-get-tahoe-26-1-looking-in-better-shape/>).

**26.2.** Finer-grained control: "Additional Lock Screen time customization option lets you further adjust its
appearance, giving the Liquid Glass material more or less opacity." **[APPLE]**

**26.4.** Motion fix: "Reduce Motion setting more reliably reduces the animations of Liquid Glass for users
sensitive to on screen motion." **[APPLE]** — i.e. the original Reduce Motion did *not* suppress the live
specular/lensing animation, and users noticed.

**The HIG now bakes this in.** The materials page states material appearance "can differ in response to certain
system settings, like if people choose a preferred look for Liquid Glass in their device's settings, or turn on
accessibility settings that reduce transparency or increase contrast." **[APPLE]**

**What this means for Permagent, concretely.** Apple spent four point releases making its own material *less*
transparent and *more* opaque, and gave users a switch to turn the effect down. Any design of ours that starts
at the June-2025 transparency level is starting a year behind. Our defaults should sit where Tinted sits, with
the more-transparent look as the deliberate exception — not the reverse.

There is also a hardware cost worth naming: community measurement reported ~13% battery drain vs ~1% on iOS 18
for equivalent workloads on an iPhone 16 Pro Max, and heat on older devices **[MEASURED]**
(<https://github.com/conorluddy/LiquidGlassReference>). Apple's own material is expensive. Ours will be worse
(§3.5).

---

## 2. The broader current Apple feel

Liquid Glass is the loud part. The quiet parts are what actually make an app feel native, and most of them are
cheaper to implement.

### 2.1 Typography

**The font.** SF Pro. In a webview you get it for free from the system font stack — no webfont, no CSP change
(our CSP's `font-src` allowlist is irrelevant because the system font is not fetched):

```css
font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", "SF Pro Display", system-ui, sans-serif;
```

`-apple-system` on macOS WebKit resolves to the correct optical variant automatically. Prefer it over naming
`SF Pro Text`/`SF Pro Display` explicitly — the system picks better than we can.

**Optical sizing — the 20pt rule and its 2020 revision.** SF Text is drawn for below 20pt, SF Display for 20pt
and above. But since SF Pro became a variable font, "the transition is no longer a hard break at 20pt. Instead,
the design **transitions smoothly between 17 and 28 points**." **[APPLE]**
(WWDC20 session 10175, "The details of UI typography", <https://developer.apple.com/videos/play/wwdc2020/10175/>)

What changes across that transition **[APPLE]**: tracking (looser small, tighter large), stroke weight (sturdier
at small sizes), vertical proportions (SF Text is optically larger at the same point size), and detail placement
(the dot on the `i` moves so it doesn't read as an `l`).

Practical consequence for us: **our current ramp tightens tracking as size grows** (`display -0.02em`,
`title -0.01em`, `heading -0.005em`, body 0) — which is exactly the right *direction* and matches SF's own
behaviour. Keep it. If we switch the body face to `-apple-system`, keep the negative tracking on the large
sizes only; SF already handles small-size tracking internally and adding more will look wrong.

**The macOS scale.** Apple's macOS text styles run notably smaller than iOS's: body is **13pt** on macOS
against 17pt on iOS; large title is 26pt on macOS against 34pt on iOS **[MEASURED]**
(<https://superdesign.dev/blog/apple-design-system>; Apple's own HIG typography tables are JS-rendered and
were not directly fetchable). The iOS scale, for reference **[MEASURED]**: Large Title 34 / Title1 28 /
Title2 22 / Title3 20 / Headline 17 semibold / Body 17 / Callout 16 / Subhead 15 / Footnote 13 / Caption1 12 /
Caption2 11.

**Our ramp is already close to macOS-native and should not be replaced.** 32/20/16/14/13/12/11 vs macOS's
26/22/17/15/13/13/12/11 — the shape is right; body at 14px vs macOS 13pt is a defensible density choice for a
web-rendered app (and our px are CSS px, not pt). **Do not "fix" this by importing the iOS scale.** A 17px body
in a dense desktop tool is an iPad app in a Mac window.

**Leading.** Apple's text styles carry a default line height with tight/loose variants at ±2pt on iOS/macOS
(e.g. body: 22pt default, 20 tight, 24 loose) **[APPLE]** (WWDC20/10175). Our ramp's leading (21px on 14px body
= 1.5) is looser than Apple's (22/17 = 1.29 on iOS, similar on macOS). Denser leading is one of the strongest
single levers for making a web UI feel like a Mac app.

**Tahoe's own typography change** (WWDC25/356): "Typography has been refined to strengthen clarity and
structure, now **bolder and left-aligned** to improve readability in key moments like alerts and onboarding."
**[APPLE]** Centered alert text is now the un-Apple choice.

### 2.2 Spacing, layout, chrome

- 8pt grid with 4pt subdivisions. Community-derived convention rather than a formally mandated HIG rule, but
  universally observed **[MEASURED]**.
- Minimum hit target 44×44pt on touch **[APPLE]**; on macOS the constraint is different — pointer input means
  smaller targets are legitimate, and HIG guidance for macOS is instead "use vibrant colors and larger
  clickable areas for pointer input" without a hard minimum.
- Full-height sidebar; content scrolls *beneath* it; inspector is edge-to-edge and beside content (§1.7).
- Inline titlebar: window title displays inline with toolbar controls; toolbar items are bezel-less **[APPLE]**.
- **Shadow discipline.** Apple.com's own design system uses *exactly one* drop shadow, on photographic product
  imagery, and never on cards, buttons, or text — elevation comes from surface-color change and backdrop blur on
  sticky bars **[MEASURED]** (`~/.claude/skills/awesome-design-md/design-md/apple/DESIGN.md`). Our
  `shadow.card = '0 8px 32px rgba(0,0,0,0.5)'` is a web convention, not an Apple one.

### 2.3 Motion

**The spring is the primitive.** Apple's motion is spring physics, not bezier curves. SwiftUI presets, in
increasing bounce **[MEASURED]** (Apple documents behaviour, not all constants):

| Preset | Duration | Bounce | Character |
| --- | --- | --- | --- |
| `.smooth` | 0.5s | 0.0 | critically damped; no overshoot at all |
| `.snappy` | 0.5s (default) | ~0.15 | slightly underdamped; small overshoot |
| `.bouncy` | 0.5s (default) | ~0.3 | visible overshoot + brief oscillation |
| `.spring()` legacy | response 0.55 | dampingFraction 0.825 | the pre-iOS-17 default |

Bounce runs 0 (no overshoot) → 1 (max oscillation); 0.2–0.4 "feels natural for most interactive elements"
**[MEASURED]** (<https://www.createwithswift.com/understanding-spring-animations-in-swiftui/>,
<https://nilcoalescing.com/blog/AnimationTimingInSwiftUI/>).

**HIG rules** **[APPLE]**:
- Keep animation duration under 0.5s to avoid feeling delayed.
- "Prefer quick, precise animations" — brevity + precision reads as lightweight and less intrusive.
- "Avoid adding motion to interactions that occur frequently" — the system already animates standard elements;
  extra motion means the user watches an animation on every single interaction.

**macOS pointer affordances.** Mac is a pointer platform: every interactive element should respond to hover,
click, right-click and drag, and every interactive element should have a visible hover state **[MEASURED]**.
This is the single biggest gap between a typical web UI and a Mac app — web UIs tend to have hover states only
on obvious buttons.

Third-party glass interaction values that read correctly, offered as a starting point **[MEASURED]**
(<https://github.com/tristan-mcinnis/apple-hig-designer-skill-2026>): hover `brightness(1.05)` +
`0 8px 24px rgba(0,0,0,0.12)`; press `scale(0.97)` + `brightness(0.95)` + `0 2px 8px rgba(0,0,0,0.1)`.

### 2.4 SF Symbols

SF Symbols 7 (WWDC25 session 337, <https://developer.apple.com/videos/play/wwdc2025/337>) **[APPLE]**:

- **Draw On / Draw Off** — new calligraphic animation presets, playable Whole Symbol / By Layer / Individually.
- **Variable Draw** — layers act as progress indicators, extending Variable Color with finer resolution.
- **Gradients** — automatically generated linear gradients from a single source color, working across all
  rendering modes, "a subtle lighting effect that adds depth and polish without sacrificing legibility."
- **Weights** — variable stroke widths; animatable transitions between visual weights.
- **Magic Replace** — improved transitions between related symbols sharing an enclosure shape.
- Rendering modes remain monochrome / hierarchical / palette / multicolor.

The transferable rule is the boring one: **icon weight should match adjacent text weight and optical size.**
Symbols are designed to sit on the SF baseline at matching weights. `react-icons` (our current icon source) has
no such relationship to our type, which is why icon-next-to-label pairs in web UIs usually look slightly wrong.

WWDC25/356: "Use the same symbols across devices to preserve meaning and build familiarity through
repetition." **[APPLE]**

### 2.5 Sound and haptics

Not applicable. macOS has no haptic feedback API for standard controls (Force Touch trackpad haptics exist but
are unreachable from a webview), and system UI sound is essentially absent from modern Mac app design. Skip.

### 2.6 Interaction minimalism

The patterns Apple actually ships, which cost nothing but discipline:

- **Progressive disclosure** — advanced options deferred to secondary surfaces, revealed when relevant. The Mac
  version of this is the disclosure triangle and the "Show More" row, not a settings modal with 40 fields.
- **Direct manipulation** — drag the thing itself; don't open a dialog to describe what should happen to it.
- **Inline editing over modals** — Finder renames in place. Notes edits in place. A modal is for a decision that
  must block, not for changing a value.
- **Hierarchy from layout and grouping, not decoration** (WWDC25/356) **[APPLE]** — the rule that kills most
  borders, most background fills, and most card chrome in a typical web UI.
- Toolbar items must all be reachable from the menu bar **[APPLE]**.

---

## 3. Feasibility map for our stack

Two independent layers can carry "glass": the **native window layer** (Tauri/AppKit, below the webview) and the
**webview layer** (CSS in WKWebView). They have completely different capability profiles and completely
different costs. Decide which one carries the effect *before* writing any CSS.

### 3.1 Native window layer — Tauri 2.11 on macOS

**Repo baseline.** `ui/desktop/src-tauri` pins `tauri = { version = "2", features = ["unstable"] }`, resolving
to 2.11.0 (current upstream is 2.11.5, Jul 2026 — a patch gap, not a capability gap). `window-vibrancy 0.6.0`
is present in `Cargo.lock` only as a transitive dependency of `tauri` itself; nothing in `src/` calls it. The
app already makes raw `objc2` / `msg_send!` calls in `main.rs` and `browser.rs` for `NSException` handling, so
the pattern for reaching into AppKit directly is established. `ui/goose2` already ships
`titleBarStyle: "Overlay"`, `hiddenTitle: true`, `trafficLightPosition: {x:12,y:22}` — proof those keys are
live and valid on this codebase.

| Capability | Status | Notes |
| --- | --- | --- |
| `titleBarStyle: "Overlay"` + `hiddenTitle: true` | **SUPPORTED**, config only | Content extends full-height under the traffic lights. Already proven in `ui/goose2`. <https://v2.tauri.app/reference/config/> |
| `trafficLightPosition` | **SUPPORTED**, macOS-only, since Tauri 2.4.0 ([PR #12366](https://github.com/tauri-apps/tauri/pull/12366)) | **⚠ Trap:** [#14072](https://github.com/tauri-apps/tauri/issues/14072) — does not work when the Cargo `unstable` feature is enabled. `ui/desktop` enables `unstable`. Verify against 2.11.0 or drop the feature. |
| Traffic lights survive `set_title` | **NOT SUPPORTED** | [#13044](https://github.com/tauri-apps/tauri/issues/13044): setting the title resets the position. Re-apply after every `set_title`. |
| Traffic lights survive fullscreen exit | **NOT SUPPORTED** natively | Community fix: listen to `onResized` and reposition ([`tauri-plugin-mac-rounded-corners`](https://github.com/cloudworxx/tauri-plugin-mac-rounded-corners)). |
| Change traffic light position at runtime from JS | **NOT SUPPORTED** | [#13790](https://github.com/tauri-apps/tauri/issues/13790) — build-time only. |
| `NSVisualEffectView` vibrancy (Sidebar, WindowBackground, HeaderView, HudWindow, UnderWindowBackground, Popover, Menu, Titlebar, Selection, Sheet, Tooltip, FullScreenUI, ContentBackground, UnderPageBackground) | **SUPPORTED** via `window-vibrancy` | `apply_vibrancy(&window, NSVisualEffectMaterial::Sidebar, Some(NSVisualEffectState::Active), Some(radius))`. macOS 10.10+. <https://docs.rs/window-vibrancy> |
| Native **Liquid Glass** (`NSGlassEffectView`) | **SUPPORTED via PRIVATE API**, macOS 26+ | `window-vibrancy` 0.7.x `apply_liquid_glass(...)` with `LiquidGlassOptions{radius, opaque, tint}`; falls back to `UnderWindowBackground` pre-26. Richer alternative: [`tauri-plugin-liquid-glass`](https://github.com/hkandala/tauri-plugin-liquid-glass) exposing 24 glass variants (`Regular, Clear, Sidebar, AbuttedSidebar, Inspector, Control, Loupe, ControlCenter, …`). Known bug: [#198](https://github.com/tauri-apps/window-vibrancy/issues/198) corner misalignment on focus change (fixed in #199); [#208](https://github.com/tauri-apps/window-vibrancy/issues/208) two glass NSPanels can't both be key. |
| Transparent webview background (prerequisite for all of the above) | **SUPPORTED but expensive** | Requires `transparent: true` **and** `macOSPrivateApi: true`, plus CSS `html,body{background:transparent}`. Forfeits Mac App Store — irrelevant for us, we ship a signed DMG. |
| Real `NSToolbar` | **NOT SUPPORTED** — REQUIRES-CUSTOM-RUST | No Tauri config or builder method. Would need raw `NSWindow*` via `raw_window_handle` + `objc2-app-kit`. No community plugin exists. Everyone fakes it with an HTML row + `data-tauri-drag-region`. |
| macOS 26 window corner radius | **AUTOMATIC** if `decorations: true` | The moment you go `decorations: false` + `transparent: true`, native rounding **and** the native drop shadow are lost — [#3481](https://github.com/tauri-apps/tauri/issues/3481), [#9287](https://github.com/tauri-apps/tauri/issues/9287), [#4243](https://github.com/tauri-apps/tauri/issues/4243). Must be re-masked manually. |
| Tauri core Liquid Glass support | **NOT SUPPORTED** | Open discussion [#13610](https://github.com/tauri-apps/tauri/discussions/13610), feature request [#14207](https://github.com/tauri-apps/tauri/issues/14207). It lives entirely in `window-vibrancy` / third-party plugins. |

**The cost that decides this.** [tauri#15471](https://github.com/tauri-apps/tauri/issues/15471) (open):
`transparent: true` on macOS forces WebKit/WindowServer to alpha-composite the **entire window every frame,
even for a completely static page**, measured at roughly **8× the GPU power draw** of an opaque window; much
worse on Intel Macs than Apple Silicon. Permagent is an always-on desktop agent UI. Paying 8× idle GPU for a
material effect is a real product decision, not a styling tweak.

**Second cost.** `minimumSystemVersion` is `"11.0"`. Native Liquid Glass needs macOS 26. Any native-glass path
must be runtime-gated with an `NSVisualEffectView` fallback for 11–15 and a flat fallback below that — i.e.
three visual paths to maintain, forever.

**Recommendation for the native layer.** Do **not** turn on `transparent: true` as part of a visual upgrade.
Ship the cheap, high-yield native wins first — they are pure config and cost nothing:

```jsonc
// ui/desktop/src-tauri/tauri.conf.json  → app.windows[0]
"titleBarStyle": "Overlay",
"hiddenTitle": true,
"trafficLightPosition": { "x": 20, "y": 22 }
// keep "decorations": true  → keeps the free macOS 26 corner radius + native shadow
```

That alone gives us a full-height, edge-to-edge app surface with a real inline titlebar and correctly inset
traffic lights, which is 80% of "looks like a Tahoe app" for 0% of the GPU cost. Revisit native vibrancy only
if we later decide a genuinely refracting sidebar is worth an 8× idle GPU bill, three code paths, and a private
API dependency — and measure it on an Intel Mac before committing.

### 3.2 Webview layer — what WKWebView can actually render

**The headline constraint.** The technique that makes every impressive web Liquid Glass demo work —
`backdrop-filter: url(#svg-filter)` referencing an `feDisplacementMap` — **does not work in WebKit.**

> **WebKit bug 245510** — "`backdrop-filter: url(#some-svg-filter)` doesn't work with SVG filters like
> feDisplacementMap." Filed Sept 2022. **Still `NEW` (open) as of Sept 2026.** Affects `feDisplacementMap` and
> `feColorMatrix` when referenced via `backdrop-filter`. Cross-browser test cases re-added June 2026; patches
> posted for review July 2026 but **not shipped**.
> <https://bugs.webkit.org/show_bug.cgi?id=245510>

Critical distinction: **`filter: url(#id)` on a normal element works fine in Safari.** It is specifically the
*backdrop* variant that is broken. There is also an open spec issue,
[w3c/svgwg#1142](https://github.com/w3c/svgwg/issues/1142) (June 2026), explicitly motivated by "liquid glass"
UI, proposing a `BackdropGraphic` filter input — open, unassigned, no browser commitments.

This is a hard platform ceiling for a Tauri macOS app. Every library that offers refraction —
`liquid-glass-react`, `nikdelvin/liquid-glass`, `shuding/liquid-glass` — ships a Safari fallback that is
*exactly* plain blur + tint. So: **the Safari fallback is our primary implementation.** Design to it directly
rather than treating it as a degradation.

**The one escape hatch**, for the record: a WebGL layer that snapshots the backdrop into a texture and does
displacement in a fragment shader (as [dashersw's demo](https://codepen.io/dashersw/pen/EajEWyZ) does) is not
blocked by the bug, because it never touches `backdrop-filter`. We already ship `three` and
`@react-three/fiber`. It is still a bad idea for chrome: DOM-to-texture is fragile against live/animated
content, breaks scroll sync, and duplicates the accessibility tree. Not recommended.

**What does work in WebKit** — all standard, all long-supported:

| Primitive | Use |
| --- | --- |
| `backdrop-filter: blur() saturate() brightness() contrast()` | The base material. `saturate()` is load-bearing, not decorative — without it blur reads as washed-out grey rather than glass. |
| layered `inset box-shadow` | The specular rim / bevel. This is the WebKit-safe substitute for Apple's highlight layer. |
| `conic-gradient()` + `mask-image` on a pseudo-element ring | Directional edge light that brightens toward a notional light source — the closest we get to "light travels around the material." |
| `radial-gradient` on `::after`, faded in on `:active` | Apple's "illuminates from within on touch". Feed pointer position in via CSS custom properties set from a `pointermove` handler. |
| `mask-image: radial-gradient(closest-side, transparent 60%, black 100%)` | Edge-only effects without simulating thickness. |
| `linear()` easing | Springs. See §3.4. |
| `@starting-style` (Safari 17.5+), View Transitions (Safari 18+ same-doc, 18.2+ cross-doc) | Entry animation and shape morphing. Both safely usable in a WKWebView-only target. |
| `font-optical-sizing: auto` (Baseline in Safari; was buggy only in 16.1–16.3) | Real SF Pro optical sizing, for free. |

**What does not work:**

| Primitive | Status |
| --- | --- |
| `backdrop-filter: url(#svg-filter)` | **Broken in WebKit** (bug 245510). |
| `corner-shape: squircle` / `superellipse()` | **Chromium 139+ only.** Safari and Firefox ignore it entirely and fall back to circular `border-radius`. <https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/corner-shape> |
| `prefers-reduced-transparency` | **Not implemented in WebKit** (fingerprinting objection). See §3.5. |
| SF Symbols as an icon font/webfont | Licensed for Apple-platform UI mockups by registered developers, not for general web font hosting. |
| Self-hosted SF Pro `@font-face` files | Same licence restriction. Unnecessary anyway — `-apple-system` resolves the real installed variable SF Pro on-device. |

**Squircles.** Apple's corners are continuous-curvature superellipses; CSS `border-radius` draws circular arcs.
The difference is visible at large radii — which is exactly where Tahoe now operates. `corner-shape` would fix
it but is Chromium-only. Options: accept circular arcs (recommended at our radii ≤18px, where the difference is
sub-perceptual), or use an SVG `clip-path` / the [hyperellipse](https://hyperellipse.vercel.app/) polyfill on
the two or three largest surfaces only. Do **not** polyfill squircles app-wide; the cost is real and the payoff
at small radii is nil.

### 3.3 The grades

**FAITHFUL** = achievable and indistinguishable in practice. **APPROXIMATE** = close, with a stated compromise.
**SKIP** = native-only; faking it looks worse than not doing it.

| # | Liquid Glass / Apple property | Grade | How, and what we give up |
| --- | --- | --- | --- |
| 1 | **Layer model** — glass only on the floating control layer, never on content | **FAITHFUL** | Pure discipline, zero cost, highest payoff of anything in this table. |
| 2 | **Concentric corner radii** (`r_inner = max(0, r_outer − padding)`) | **FAITHFUL** | Arithmetic. Encode as a token function; the existing `radiusScale.test.ts` can enforce it. |
| 3 | **Capsule = half container height** | **FAITHFUL** | `border-radius: 999px` (`radius.pill`, already in the scale). |
| 4 | **Large/XL controls capsule, mini–medium rounded-rect** | **FAITHFUL** | Discipline. Stops the everything-is-a-pill failure. |
| 5 | **Regular glass base material** (blur + saturation + tint) | **APPROXIMATE** | `backdrop-filter: blur(20px) saturate(180%)` + tint fill. Compromise: light scatters, it does not bend. No lensing. |
| 6 | **Adaptive luminosity** — glass lightens over light content, darkens over dark | **APPROXIMATE** | Achievable *per theme*, not *per backdrop pixel*. CSS cannot sample what is behind an element. We get one tint per theme, not a live response. |
| 7 | **Clear variant + 35% dimming layer** | **APPROXIMATE** | Reproducible literally (`rgba(0,0,0,0.35)` scrim + light blur). Compromise: no adaptive fallback if the backdrop turns hostile. |
| 8 | **Edge lensing / refraction** | **SKIP** | WebKit bug 245510. The only web technique that produces it is unavailable to us. Faking it with a border gradient reads as a cheap bevel. |
| 9 | **Chromatic aberration at the rim** | **SKIP** | Requires the same broken filter path (3× `feColorMatrix` + `feBlend`). |
| 10 | **Specular highlight tracking geometry** | **APPROXIMATE** | Layered `inset box-shadow` (bright top edge, dark bottom edge) + a masked `conic-gradient` ring. Compromise: static light source; it does not follow window activation or device motion. |
| 11 | **Real-time light movement on window activation / unlock** | **SKIP** | No hook, no budget, and a JS-driven approximation would jank. |
| 12 | **Press illumination from within** | **APPROXIMATE** | `::after` radial-gradient fading in on `:active`, centred on pointer position via CSS custom properties. Compromise: single element only. |
| 13 | **Glow spreading to *nearby* glass elements** | **SKIP** | Requires the shared sampling region only `GlassEffectContainer` provides. |
| 14 | **`GlassEffectContainer` — one shared sampling region** | **APPROXIMATE** | Model it as *one* glass element with non-glass children. This is both the honest translation and a performance necessity (§3.5). Compromise: no automatic blending/morphing between siblings. |
| 15 | **Glass morphing between shapes (`glassEffectID`)** | **APPROXIMATE** | View Transitions API (Safari 18+) or a FLIP transition. Compromise: shapes cross-fade and tween, they do not merge as a fluid. |
| 16 | **Scroll edge effect — `hard` (the macOS default)** | **FAITHFUL** | A near-opaque boundary under the pinned toolbar, toggled on scroll position. Trivial and correct. |
| 17 | **Scroll edge effect — `soft`** | **FAITHFUL** | `mask-image: linear-gradient()` gradient fade on the scroll container. |
| 18 | **Background extension view** (mirroring content under a floating control) | **APPROXIMATE** | Duplicate a blurred, clipped copy of adjacent content behind the sidebar. Compromise: real cost for real benefit; only worth it on the sidebar, if at all. |
| 19 | **Tint = exactly one primary action** | **FAITHFUL** | Discipline. We already have `NEON_ACCENT`; the work is *removing* its other uses. |
| 20 | **Monochromatic polarity flip on small glass** | **SKIP** | Requires backdrop luminance sampling. Pick a polarity per theme and hold it. |
| 21 | **Larger glass ⇒ more opaque** | **FAITHFUL** | A token rule: sidebar opacity > toolbar opacity > popover opacity. Costs nothing. |
| 22 | **Continuous-curvature (squircle) corners** | **SKIP** at our radii | `corner-shape` is Chromium-only. Circular arcs at ≤18px are visually indistinguishable. Revisit only if we adopt a ≥26px surface. |
| 23 | **Native window corner radius (Tahoe)** | **FAITHFUL**, free | Keep `decorations: true` and macOS supplies it. Lost the moment we go transparent/decorationless. |
| 24 | **Inline titlebar + inset traffic lights** | **FAITHFUL** | `titleBarStyle: "Overlay"` + `hiddenTitle` + `trafficLightPosition`. ⚠ Verify against the `unstable` feature ([#14072](https://github.com/tauri-apps/tauri/issues/14072)). |
| 25 | **Native sidebar vibrancy (`NSVisualEffectMaterial::Sidebar`)** | **FAITHFUL** but expensive | `window-vibrancy`. Costs `transparent: true` → private API → ~8× idle GPU ([#15471](https://github.com/tauri-apps/tauri/issues/15471)). |
| 26 | **Native Liquid Glass (`NSGlassEffectView`)** | **FAITHFUL** with three caveats | Private API, macOS 26+ only (we support 11+), plus the same 8× GPU tax. Genuinely the real material — at genuinely real cost. |
| 27 | **Real `NSToolbar`** | **SKIP** | Not exposed by Tauri; custom Rust only; nobody in the ecosystem has done it. Fake it with an HTML row + `data-tauri-drag-region`. |
| 28 | **SF Pro typography + optical sizing** | **FAITHFUL**, free | `-apple-system, system-ui` + `font-optical-sizing: auto`. Real variable SF Pro, correctly licensed, resolved on-device. |
| 29 | **Spring motion** | **FAITHFUL** | `linear()` easing (§3.4). Indistinguishable from a real spring for UI-scale motion. |
| 30 | **Reduce Motion** | **FAITHFUL** | `prefers-reduced-motion` — well-supported in WebKit since Safari 10.1. |
| 31 | **Increase Contrast** | **FAITHFUL** | `prefers-contrast` — WebKit added `more`/`less`/`custom` by Safari 18.0. |
| 32 | **Reduce Transparency** | **APPROXIMATE** | `prefers-reduced-transparency` is **not implemented in WebKit**. Bridge the native `NSWorkspace.accessibilityDisplayShouldReduceTransparency` value from Rust into a `data-` attribute, or ship our own in-app toggle. |
| 33 | **SF Symbols** | **SKIP** as an asset | Licence forbids shipping the files. Approximate by choosing an icon set whose stroke weights are tuned to sit next to our text weights, and by matching icon optical size to adjacent type. |
| 34 | **SF Symbols animations (Draw On/Off, Magic Replace, Variable Draw)** | **SKIP** | Deep asset-level feature; a web approximation reads as generic icon animation. |
| 35 | **Tab bar minimize-on-scroll** | **SKIP** | An iOS behaviour. macOS uses sidebars. Importing it would make us look like an iPad app in a Mac window. |

**Score:** 13 FAITHFUL, 11 APPROXIMATE, 11 SKIP. Notice which ones are FAITHFUL — they are almost entirely
*discipline and arithmetic*, not effects. That is the real finding of this document.

### 3.4 Motion in the webview

`linear()` easing is supported and is the correct primitive for spring motion in CSS. Generate strings with
[Jake Archibald's Linear Easing Generator](https://linear-easing-generator.netlify.app/) (feed it
mass/stiffness/damping) or take the Apple-derived presets from [kvin.me/css-springs](https://www.kvin.me/css-springs/how-to-use),
which ship `smooth` / `snappy` / `bouncy` named after Apple's own. Cost is negligible — three 75-point strings
measured at ~1.3KB gzipped.

One nuance from that tool that people get wrong: the **spring duration** you put in CSS is longer than the
*perceptual* duration, because the settle tail extends past where the motion feels finished. Using perceptual
duration as the CSS duration clips the animation.

Pattern:

```css
:root {
  --ease-smooth: cubic-bezier(0.22, 1, 0.36, 1);          /* fallback */
  --ease-smooth-time: 320ms;
}
@supports (animation-timing-function: linear(0, 1)) {
  :root { --ease-smooth: linear(0, 0.1, 0.25, 0.5, 0.68, 0.8, 0.88, 0.94, 0.98, 0.995, 1); }
}
@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after { animation-duration: 1ms !important; transition-duration: 1ms !important; }
}
```

⚠ **`requestAnimationFrame` is capped at 60fps in WKWebView on macOS 13–15** regardless of the display's actual
refresh rate. Any JS-driven animation will be clamped on a ProMotion Mac; CSS-driven animation will not. One
more reason to keep motion in CSS.

### 3.5 Performance budget

`backdrop-filter` forces a full compositing pass that reads the framebuffer behind the element. Each element
with its own `backdrop-filter` is a **separate** pass — the browser cannot batch them. Every serious
recreation author independently landed on the same empirical ceiling: **more than one or two animated glass
surfaces visibly degrades the frame rate.**

Nesting is worse than tiling. Three sibling filter layers are cheaper than three nested ones, because each
nested filter boundary forces a render-to-texture pass. So Apple's aesthetic rule ("avoid glass on glass") and
the engineering rule are the same rule. That is convenient, and it should make the rule easy to hold.

Rules of thumb:
- **Blur ≤ 20px** on large-area surfaces. Cost scales with blurred area × radius.
- **One glass plane per screen region.** Never two glass surfaces overlapping.
- **Never `backdrop-filter` inside a scrolling list item.** This is the single most common way to destroy scroll
  performance, and it is also the content-layer violation from §1.5.
- **`will-change: backdrop-filter` only around the animation window** — add before, remove after. Persistent
  `will-change` on many elements costs VRAM and layer-management overhead with no benefit.
- Test in the actual WKWebView, not in desktop Safari. Tauri uses the system WebKit build, so behaviour drifts
  with the user's macOS version, and there are unexplained community reports of scroll micro-lag specific to
  Tauri's WKWebView.

**Our current state is already over budget in places.** ~30 files carry ad-hoc `backdropFilter`, including
inside `world/` HUD elements that animate. Auditing and deleting most of those is a performance win *and* a
correctness win before a single new glass surface is added.

### 3.6 What made the good web recreations good

Surveying the canonical implementations — [shuding/liquid-glass](https://github.com/shuding/liquid-glass),
[kube.io](https://kube.io/blog/liquid-glass-css-svg/), [Atlas Pup Labs](https://atlaspuplabs.com/blog/liquid-glass-but-in-css),
[liquid-glass-react](https://www.npmjs.com/package/liquid-glass-react) — three things separate the convincing
ones from the rest, and only the third is available to us:

1. **They computed the displacement map from optics, not from a gradient.** kube.io actually solves Snell's law
   (glass IOR 1.5) along one radius at 127 samples, encoding `r = 128 + x*127`, `g = 128 + y*127` where 128 is
   "no displacement". The refraction profile that matched Apple best was a **convex squircle**
   (`y = ⁴√(1 − (1−x)⁴)`), not a convex circle — a circle reads as a ball lens. `feDisplacementMap scale`
   values in the wild run 20 (conservative) to 40–64 (typical) to ~100 (ball-like). **Unavailable to us.**
2. **They reconstructed Apple's three-layer composition** rather than guessing at one. Atlas Pup Labs
   reverse-engineered from Apple's beta binaries and built highlight / shadow / illumination as distinct
   layers. **Available to us**, minus the displacement stage — and it is the structural insight worth stealing.
3. **They were disciplined about saturation and edge light.** The recurring, engine-agnostic finding: the base
   is `backdrop-filter: blur(12–20px) saturate(150–180%)`, and the *edge* is what sells it — inset shadows and
   a masked directional gradient, not the blur. **Fully available to us.**

The corollary is encouraging: the parts of a good recreation that WebKit denies us are the parts that need a
busy photographic backdrop to even be visible. Over a dark, mostly-flat app background — which is what
Permagent has — refraction has almost nothing to refract. **We lose less than the demos suggest.**

---

## 4. DESIGN DIRECTIVE

The rules a screen-by-screen upgrade follows. Each is enforceable; several are testable.

**D1. Glass lives on the floating control layer and nowhere else; content stays opaque.**
Glass goes on the toolbar, the sidebar, the floating command bar, popover/menu chrome, the HUD overlay. Never on
a card, a list row, a table, a chat bubble, a modal body, or any panel that is part of the content. Apple:
*"Don't use Liquid Glass in the content layer."* Content surfaces use solid theme fills (`color.surface`,
`color.surfaceHi`); if one needs to feel elevated, change its fill or its spacing — not its transparency. This
is the one rule that, alone, decides whether the result reads as native or as a theme.

**D2. One glass plane per region. Never glass on glass.**
Apple: *"Always avoid glass on glass."* Where you need a control on top of glass, use a fill, a vibrancy-style
opacity step, or a border — never a second `backdrop-filter`. This is simultaneously the aesthetic rule and the
performance rule (each nested filter forces a render-to-texture pass).

**D3. Apply the material to the control, not its inner views.**
Verbatim from WWDC25/356. A glass toolbar is *one* element with plain children; it is not five glass buttons.
Groups of controls sit on one shared glass shape, mirroring `GlassEffectContainer`.

**D4. Concentric radii — `r_inner = max(0, r_outer − padding)` — anchored to the window.**
If the result is ≤ 0 the child gets square corners, and that is correct. macOS 26 gives a toolbar-bearing window
a large corner radius (~26px measured), so our outermost floating surface is that minus its inset from the
window edge: with an 8px inset, **`radius.glass = 18`**. Add that one step to the scale, derive everything inside
it concentrically, and invent no second large radius. Add a `concentric(outer, pad)` helper to `tokens.ts` and
extend `radiusScale.test.ts` to fail hand-written nested radii the way it already fails hand-written scale
values.

**D5. Capsules are for large and extra-large prominent controls only.**
Mini, small and medium controls keep rounded rectangles, "which preserves horizontal density" (WWDC25/310).
A dense agent console where every button is a pill is iOS cosplay, not Tahoe.

**D6. Bigger glass surface ⇒ more opaque, not less.**
Sidebar more opaque than toolbar, toolbar more opaque than a popover. This is Apple's own stated behaviour
(large glass raises opacity and refuses the light/dark polarity flip because the flip would be distracting).

**D7. Standardise on one glass token set. Delete every hand-written `backdropFilter`.**
Currently ~30 files with values from `blur(4px)` to `blur(24px) saturate(140%)`. Three tokens replace all of
them, and a lint/test gate keeps them from regrowing — the same shape of gate that `radiusScale.test.ts`
already applies to radii.

```ts
// dark theme (default) — proposed [OURS]
glass: {
  // The default. Sits where Apple's "Tinted" landed, not where the June-2025 beta started.
  regular: {
    background: 'rgba(30, 36, 51, 0.82)',            // color.surface @ 82%
    backdropFilter: 'blur(20px) saturate(180%)',
    boxShadow: [
      'inset 0 1px 0 rgba(255,255,255,0.14)',        // specular top edge
      'inset 0 -1px 0 rgba(0,0,0,0.22)',             // shaded bottom edge
      'inset 0 0 0 1px rgba(255,255,255,0.07)',      // hairline (= color.border)
      '0 8px 32px rgba(0,0,0,0.28)',                 // ambient depth
    ].join(', '),
  },
  // Sidebar — the largest surface, therefore the most opaque (D6).
  sidebar: {
    background: 'rgba(30, 36, 51, 0.90)',
    backdropFilter: 'blur(24px) saturate(170%)',
  },
  // Clear — HUD over the 3D world view ONLY, and only with the scrim.
  clear: {
    background: 'rgba(11, 18, 32, 0.35)',            // Apple's 35% dimming layer, literally
    backdropFilter: 'blur(12px) saturate(150%)',
  },
}
```

Light (silver) theme mirrors this with `rgba(255,255,255,0.82 / 0.90)` fills and the inset highlight/shadow
polarity inverted (bright edge stays on top; the bottom edge softens rather than darkens).

**D8. Tint exactly one action per view, on its background, never on its label.**
Apple: *"Refrain from adding color to the background of multiple controls."* We currently use `NEON_ACCENT`
everywhere. The upgrade is mostly *subtraction*: pick the single primary action per screen; everything else is
monochromatic.

**D9. Motion tokens are springs, and stay under 500ms.**
Adopt three: `smooth` (no overshoot, ~320ms — the default for almost everything), `snappy` (~bounce 0.15, for
control state changes), `bouncy` (~bounce 0.3, reserved for one or two delight moments). Express as `linear()`
with a `cubic-bezier` fallback behind `@supports`. HIG: *"Prefer quick, precise animations"*, and
*"avoid adding motion to interactions that occur frequently."* Our existing `duration.base = 200` /
`slow = 320` are already in the right range — keep them and swap the curves.

**D10. Every interactive element has a visible hover state, and press feedback is physical.**
Mac is a pointer platform. Hover `brightness(1.05)` + a slightly lifted shadow; press `scale(0.97)` +
`brightness(0.95)` + a compressed shadow. Not a color swap. This is the cheapest single change that makes a web
UI feel like a Mac app.

**D11. Scroll boundaries use a hard edge, not a soft one.**
Apple assigns `soft` to iOS and **`hard` "mostly used on macOS"**. Under a pinned toolbar, the boundary should
be a near-opaque line-plus-fill that appears on scroll — one per view, consistent in height across split panes.
And per Apple: it is not decoration; do not use it where nothing is floating.

**D12. Type: system font, macOS-scale, tighter leading, left-aligned.**
Switch the body face to `-apple-system, system-ui, BlinkMacSystemFont, sans-serif` with
`font-optical-sizing: auto`. **Keep our existing size ramp** — 32/20/16/14/13/12/11 is already close to macOS's
26/22/17/15/13/12/11 and far better suited to a dense desktop tool than the iOS scale. Tighten leading toward
Apple's ~1.3 on body copy. Keep negative tracking on the large sizes only; SF handles small-size tracking
itself. Alerts and onboarding are **bolder and left-aligned** (WWDC25/356) — centered alert text is now the
un-Apple choice.

**D13. Hierarchy comes from layout and grouping, not decoration.**
Verbatim from WWDC25/356: *"Instead of relying on decoration, hierarchy should be expressed through layout and
grouping."* And *"If you've customized your bars, now's the time to clean them up… making customizations like
these unnecessary."* Practically: delete borders that only separate, delete background fills that only group,
delete shadows that only elevate. Apple.com's own system uses exactly one drop shadow in the entire design
language, and it is on product photography — never on cards, buttons, or text.

**D14. Respect the three accessibility settings, and bridge the one CSS can't see.**
`prefers-reduced-motion` and `prefers-contrast` work in WebKit. `prefers-reduced-transparency` **does not** —
read `accessibilityDisplayShouldReduceTransparency` on the Rust side, push it to the webview as a `data-`
attribute on `<html>`, and have it collapse all glass tokens to their opaque equivalents. Also ship our own
Clear/Tinted preference, because Apple did, in 26.1, after a year of complaints.

**D15. Ship the free native wins before the expensive ones.**
`titleBarStyle: "Overlay"` + `hiddenTitle` + `trafficLightPosition`, keeping `decorations: true`. That is pure
config, keeps the free Tahoe corner radius and native shadow, and delivers most of the native impression.
Do not enable `transparent: true` as part of a visual pass — it is an 8× idle GPU decision requiring its own
measurement and its own sign-off.

---

## 5. Anti-slop list

What separates Apple's restraint from a glassmorphism knockoff. If a screen does any of these, it is wrong,
regardless of how good it looks in a screenshot.

1. **Glass on cards, list rows, or message bubbles.** The #1 tell, and Apple's #1 explicit prohibition. Content
   is opaque.
2. **Glass on glass.** A frosted panel inside a frosted panel. Reads as mush; costs a render-to-texture pass.
3. **Every button is a pill.** Capsules are for large/XL prominent controls. Dense controls stay rounded rects.
4. **Blur without saturation.** `blur()` alone desaturates the backdrop to grey haze. `saturate(150–180%)` is
   what makes it read as glass rather than as fog.
5. **Uniform corner radius everywhere.** Real concentricity means nested elements have *different*, derived
   radii. A single radius token applied at every level is the flattest possible tell.
6. **A radius so large it fights the content.** Apple's own oversized Tahoe corners caused a measurable
   usability regression. Big is not the point; *concentric* is.
7. **Rainbow tinting.** Three tinted buttons in a toolbar. Apple's wrong/right example is literally this.
   One tinted action per view.
8. **Neon glow as depth.** Our `shadow.glow` / `shadow.glowStrong` / the `glow` keyframe are a game-UI idiom,
   not an Apple one. Apple gets depth from surface luminosity and one soft ambient shadow.
9. **Animated glass.** A `backdrop-filter` surface that moves, pulses, or continuously animates. Pathological
   for performance and explicitly discouraged in Apple's own guidance.
10. **Faked refraction.** A bright bevel border or an SVG squiggle pretending to be lensing. WebKit cannot do
    real refraction; a fake one reads as a 2013 skeuomorphic button, which is strictly worse than clean blur.
11. **Modals for things that should edit inline.** Finder renames in place. A modal is for a blocking decision,
    not for changing a value.
12. **Decoration standing in for hierarchy.** Borders that only separate, fills that only group, shadows that
    only elevate. If layout and spacing can express it, delete the decoration.
13. **iOS metrics on a Mac.** 17px body, 44px touch targets, bottom tab bars, minimize-on-scroll. This is a
    pointer-driven desktop app; it should be dense.
14. **Designing to the June-2025 beta.** Maximum transparency, minimum contrast. Apple spent 26.1, 26.2 and
    26.4 walking that back. Start where they landed.
15. **Centered body text in alerts and onboarding.** Tahoe's typography is bolder and left-aligned.
16. **Glass that ignores accessibility.** A user with Reduce Transparency on should see opaque surfaces. CSS
    can't tell you that in WebKit — bridge it, don't skip it.

---

## 6. Open questions to settle before implementation

1. **Does `trafficLightPosition` actually work on tauri 2.11.0 with `features = ["unstable"]`?**
   [#14072](https://github.com/tauri-apps/tauri/issues/14072) says no as of 2.8.3. Test before designing an
   inline titlebar around it. Fallback: drop the `unstable` feature (audit what depends on it first).
2. **Measure the 8× GPU claim ourselves** ([#15471](https://github.com/tauri-apps/tauri/issues/15471)) on both
   Apple Silicon and Intel before any native-transparency work is scheduled.
3. **`transparent: true` + signed DMG** — [#13415](https://github.com/tauri-apps/tauri/issues/13415) reports
   transparent windows rendering correctly in `tauri dev` but coming out solid white from a signed build. We
   ship signed DMGs. Verify early or not at all.
4. **Confirm `prefers-contrast` support** in the WKWebView builds our users actually run. The Safari 18.0 figure
   is corroborated by a WebKit blog reference rather than a pinned compat table.
5. **Get the real Tahoe window corner radius.** The 26pt figure is a third-party measurement. Screenshot a
   Tahoe window with and without a toolbar, measure, and fix `radius.glass` from that rather than from a forum
   post.
6. **Decide the Clear/Tinted default.** This document recommends shipping at Tinted-equivalent opacity with a
   user preference, mirroring what Apple did in 26.1.

---

## 7. Sources

**Apple primary**
- Newsroom, "Apple introduces a delightful and elegant new software design" — <https://www.apple.com/newsroom/2025/06/apple-introduces-a-delightful-and-elegant-new-software-design/>
- HIG Materials — <https://developer.apple.com/design/human-interface-guidelines/materials>
- HIG Color — <https://developer.apple.com/design/human-interface-guidelines/color>
- HIG Toolbars — <https://developer.apple.com/design/human-interface-guidelines/toolbars>
- HIG Sidebars — <https://developer.apple.com/design/human-interface-guidelines/sidebars>
- HIG Tab bars — <https://developer.apple.com/design/human-interface-guidelines/tab-bars>
- HIG Layout — <https://developer.apple.com/design/human-interface-guidelines/layout>
- WWDC25/219 "Meet Liquid Glass" — <https://developer.apple.com/videos/play/wwdc2025/219>
- WWDC25/310 "Build an AppKit app with the new design" — <https://developer.apple.com/videos/play/wwdc2025/310>
- WWDC25/323 "Build a SwiftUI app with the new design" — <https://developer.apple.com/videos/play/wwdc2025/323>
- WWDC25/356 "Get to know the new design system" — <https://developer.apple.com/videos/play/wwdc2025/356>
- WWDC25/337 "What's new in SF Symbols 7" — <https://developer.apple.com/videos/play/wwdc2025/337>
- WWDC20/10175 "The details of UI typography" — <https://developer.apple.com/videos/play/wwdc2020/10175/>
- `Edge.Corner.Style.concentric` — <https://developer.apple.com/documentation/swiftui/edge/corner/style/concentric>
- `ScrollEdgeEffectStyle` — <https://developer.apple.com/documentation/swiftui/scrolledgeeffectstyle>
- macOS Tahoe 26 release notes (incl. 26.1 Liquid Glass setting) — <https://support.apple.com/en-us/122868>
- iOS 26.1 release notes — <https://support.apple.com/en-us/123075>

> **Note:** there is no `/design/human-interface-guidelines/liquid-glass` page — it 404s. Liquid Glass guidance
> is distributed across `/materials`, `/color`, `/toolbars`, `/tab-bars`, `/sidebars`, `/layout`. No WWDC26
> Liquid Glass follow-up session surfaced in search as of Sept 2026.

**Post-launch reception and tuning**
- MacStories, macOS 26 Tahoe review — <https://www.macstories.net/stories/macos-26-tahoe-the-macstories-review/2/>
- Eclectic Light, "Appearance revisited: Get Tahoe 26.1 looking in better shape" — <https://eclecticlight.co/2025/11/05/appearance-revisited-get-tahoe-26-1-looking-in-better-shape/>
- MacRumors, macOS Tahoe 26.1 release — <https://www.macrumors.com/2025/11/03/apple-releases-macos-tahoe-26-1/>
- Michael Tsai, "Tahoe Window Corners" — <https://mjtsai.com/blog/2025/10/16/tahoe-window-corners/>
- Lapcat Software, "macOS Tahoe windows have different corner radiuses" — <https://lapcatsoftware.com/articles/2026/3/1.html>
- Daring Fireball, "Why It's Difficult to Resize Windows on macOS 26" — <https://daringfireball.net/2026/01/resizing_windows_macos_26>
- CSS-Tricks, "Getting Clarity on Apple's Liquid Glass" — <https://css-tricks.com/getting-clarity-on-apples-liquid-glass/>

**Web recreation technique**
- WebKit bug 245510 (`backdrop-filter: url()` + SVG filters) — <https://bugs.webkit.org/show_bug.cgi?id=245510>
- w3c/svgwg#1142, backdrop input for filters — <https://github.com/w3c/svgwg/issues/1142>
- kube.io, "Liquid Glass in the Browser" (Snell's law displacement maps) — <https://kube.io/blog/liquid-glass-css-svg/>
- Atlas Pup Labs, "Liquid Glass, but in CSS" — <https://atlaspuplabs.com/blog/liquid-glass-but-in-css>
- shuding/liquid-glass — <https://github.com/shuding/liquid-glass>
- liquid-glass-react — <https://www.npmjs.com/package/liquid-glass-react>
- nikdelvin/liquid-glass — <https://github.com/nikdelvin/liquid-glass>
- dashersw, WebGL liquid glass — <https://codepen.io/dashersw/pen/EajEWyZ>
- MDN `corner-shape` — <https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/corner-shape>
- MDN `prefers-reduced-transparency` — <https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/At-rules/@media/prefers-reduced-transparency>
- Linear Easing Generator — <https://linear-easing-generator.netlify.app/>
- kvin.me CSS springs (Apple-derived presets) — <https://www.kvin.me/css-springs/how-to-use>

**Tauri / native layer**
- Tauri config reference — <https://v2.tauri.app/reference/config/>
- Tauri window customization — <https://v2.tauri.app/learn/window-customization/>
- window-vibrancy — <https://github.com/tauri-apps/window-vibrancy> · <https://docs.rs/window-vibrancy>
- tauri-plugin-liquid-glass (24 glass variants) — <https://github.com/hkandala/tauri-plugin-liquid-glass>
- tauri-plugin-mac-rounded-corners (public-API path) — <https://github.com/cloudworxx/tauri-plugin-mac-rounded-corners>
- Issues: [#12366](https://github.com/tauri-apps/tauri/pull/12366) (trafficLightPosition), [#14072](https://github.com/tauri-apps/tauri/issues/14072) (`unstable` breaks it), [#13044](https://github.com/tauri-apps/tauri/issues/13044) (set_title resets it), [#13790](https://github.com/tauri-apps/tauri/issues/13790) (no runtime API), [#13415](https://github.com/tauri-apps/tauri/issues/13415) (transparent + signed build), [#15471](https://github.com/tauri-apps/tauri/issues/15471) (**8× GPU**), [#3481](https://github.com/tauri-apps/tauri/issues/3481) / [#9287](https://github.com/tauri-apps/tauri/issues/9287) (rounded transparent windows), [#13610](https://github.com/tauri-apps/tauri/discussions/13610) / [#14207](https://github.com/tauri-apps/tauri/issues/14207) (Liquid Glass in core)

**Community reference material**
- conorluddy/LiquidGlassReference — <https://github.com/conorluddy/LiquidGlassReference>
- tristan-mcinnis/apple-hig-designer-skill-2026 — <https://github.com/tristan-mcinnis/apple-hig-designer-skill-2026>
- Superdesign, Apple design system breakdown (2026) — <https://superdesign.dev/blog/apple-design-system>
- createwithswift, spring animations — <https://www.createwithswift.com/understanding-spring-animations-in-swiftui/>
- nilcoalescing, animation timing in SwiftUI — <https://nilcoalescing.com/blog/AnimationTimingInSwiftUI/>
- Local: `~/.claude/skills/awesome-design-md/design-md/apple/DESIGN.md` (apple.com production values)
