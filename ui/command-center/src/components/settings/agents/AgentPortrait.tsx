/**
 * The agent portrait for Settings → Agents — inline SVG, no assets, no network.
 *
 * There are no per-character image files anywhere in this repo: the world body
 * is procedural (world/agents/rig.ts). So a 2D portrait cannot load a picture of
 * the character; it has to be DRAWN from the same identity the world draws from,
 * or the same agent would wear two unrelated faces on two screens.
 *
 * Two laws from WORLD_VIEW_BIBLE hold here:
 *   §2 — one palette. Every colour below comes from `ENV` or from the ROSTER
 *        trim `portraitSpec` read; there is not a single hex literal in this
 *        file, because a second palette drifts from the first the moment either
 *        is tuned.
 *   §4 — state never repaints identity. The visor deliberately uses
 *        `ENV.gunmetal` and never a `STATE` colour: a still portrait cannot
 *        update, so a state hue here would keep claiming a liveness it stopped
 *        having the instant the worker changed.
 *
 * Pure function of `agentId` — no state, no effects, no clock.
 */

import { ENV } from '../../world/shared/palette';
import { portraitSpec } from './portrait';

export function AgentPortrait({ agentId, size = 56 }: { agentId: string; size?: number }) {
  const spec = portraitSpec(agentId);
  // An agent with no in-world character has no identity colour to borrow, so it
  // falls back to a neutral token rather than wearing someone else's trim.
  const trim = spec.trimColor ?? ENV.marbleVein;

  return (
    <svg
      viewBox="0 0 64 64"
      width={size}
      height={size}
      role="img"
      aria-label={`${spec.label} — portrait`}
      data-testid={`agent-portrait-${agentId}`}
      data-variant={spec.variant}
      data-trim-color={spec.trimColor ?? ''}
      style={{ flexShrink: 0, display: 'block' }}
    >
      <title>{spec.label}</title>

      {/* Ground, shoulders, head, visor — the silhouette every agent shares. */}
      <rect width="64" height="64" rx="8" fill={ENV.deepVoid} />
      <path d="M10 62 Q10 44 32 44 Q54 44 54 62 Z" fill={ENV.gunmetal} />
      <rect x="20" y="14" width="24" height="26" rx="9" fill={ENV.darkStone} />
      <rect x="23" y="24" width="18" height="5" rx="2.5" fill={ENV.gunmetal} />

      {/* ROSTER.weathering, the same darkening the 3D body applies as a vertex
          tint. Washed with the palette's deepest ground rather than pure black,
          so this file stays free of colour it did not get from the palette. */}
      {spec.weathering > 0 && (
        <rect width="64" height="64" rx="8" fill={ENV.deepVoid} opacity={spec.weathering * 0.28} />
      )}

      {/* Identity trim — the ONLY place trimColor is used. */}
      <path d="M18 47 H46" stroke={trim} strokeWidth="2.5" strokeLinecap="round" />
      <rect x="29" y="50" width="6" height="2" rx="1" fill={trim} />

      <SignatureGear variant={spec.variant} trim={trim} />
    </svg>
  );
}

/**
 * One signature mark per character, echoing the gear the 3D rig builds
 * (world/agents/rig.ts `gearChunks`) so the portrait and the body read as the
 * same inhabitant.
 *
 * `unknown` draws nothing at all. An agent the world has no character for gets
 * the bare silhouette: inventing insignia for it would assert an identity
 * nobody assigned.
 *
 * There is one arm per variant `portraitSpec` can actually produce, and no more.
 * The world roster has characters this switch does not: they are unreachable
 * from every call site (all three pass an agents-API id), and gear no screen can
 * draw is not decoration, it is a claim the code cannot keep.
 */
function SignatureGear({ variant, trim }: { variant: ReturnType<typeof portraitSpec>['variant']; trim: string }) {
  switch (variant) {
    case 'librarian':
      return (
        <>
          <rect x="45" y="40" width="4" height="12" fill={ENV.bronze} />
          <rect x="45" y="42" width="4" height="10" fill={ENV.gunmetal} opacity="0.9" />
          <rect x="45" y="44" width="4" height="8" fill={ENV.darkStone} opacity="0.9" />
          <line x1="42" y1="16" x2="47" y2="9" stroke={ENV.darkStone} strokeWidth="1.4" />
          <circle cx="47" cy="9" r="1.6" fill={trim} />
        </>
      );
    case 'watcher':
      return (
        <>
          <line x1="44" y1="15" x2="52" y2="3" stroke={ENV.darkStone} strokeWidth="1.4" />
          <circle cx="52" cy="3" r="2" fill={trim} />
          <rect x="26" y="19" width="3" height="2" rx="1" fill={ENV.gunmetal} />
          <rect x="35" y="19" width="3" height="2" rx="1" fill={ENV.gunmetal} />
        </>
      );
    case 'steward':
      return (
        <>
          <rect x="11" y="43" width="42" height="4" rx="2" fill={ENV.darkStone} />
          <circle cx="46" cy="55" r="3" fill="none" stroke={ENV.bronze} strokeWidth="1.2" />
          <line x1="46" y1="58" x2="46" y2="62" stroke={ENV.bronze} strokeWidth="1.2" />
          <line x1="46" y1="61" x2="49" y2="61" stroke={ENV.bronze} strokeWidth="1.2" />
        </>
      );
    case 'strix':
      return (
        <>
          <path d="M18 23 Q32 12 46 23 L46 26 Q32 17 18 26 Z" fill={ENV.darkStone} />
          <path d="M24 44 A9 9 0 0 0 40 44" fill="none" stroke={trim} strokeWidth="2" />
        </>
      );
    case 'financier':
      return (
        <>
          <rect x="8" y="47" width="10" height="14" rx="1" fill={ENV.darkStone} />
          <path d="M10 51 H16 M10 55 H16" stroke={ENV.gunmetal} strokeWidth="1.2" />
          <path d="M10 59 H16" stroke={trim} strokeWidth="1.2" strokeLinecap="round" />
          <path d="M41 53 H53 M47 53 V58" stroke={ENV.bronze} strokeWidth="1.2" />
          <path
            d="M38 53 A3 3 0 0 0 44 53 M50 53 A3 3 0 0 0 56 53"
            fill="none"
            stroke={ENV.bronze}
            strokeWidth="1.2"
          />
        </>
      );
    case 'unknown':
      return null;
  }
}
