/**
 * What the Brain graph's key says.
 *
 * The graph had no key at all. Its filter row was the closest thing — and that
 * row gives projects, tools and organisations the same ■, because the scene
 * gives all three the same cube (`BrainScene.ts`: one BoxGeometry for
 * project/tool/organization, one octahedron for concept/location, a sphere for
 * a person). Shape groups them; only colour tells them apart. So the key draws
 * the scene's own swatches, from the scene's own palette.
 *
 * Gestures, each read off `BrainScene.ts`'s handlers:
 *   drag → `onMouseDown`/`onMouseMove` (yaw + pitch; there is NO pan here,
 *     unlike the World and People, so the key does not offer one),
 *   scroll → `onWheel` (radius 8–180),
 *   hover → raycast pick, which fires the tooltip,
 *   click → `onMouseUp` when the pointer did not drag, which opens the panel.
 *
 * And the honesty line. Every edge carries a travelling light, always, at a
 * speed set by the link's weight (`rebuildPulses`) — nothing about it is a
 * live event. Under the Chip doctrine a pulse is a claim that something is
 * happening now, so a scene full of constant pulses has to say plainly that
 * they are not that.
 */

import type { LegendRow } from '../common/CanvasLegend';
import { MEMORY_STRENGTH } from '../../lib/vocabulary';
import { hex, MEMORY_FRESH, NODE_COLORS } from './graphPalette';

function Swatch({ color, shape }: { color: number; shape: 'sphere' | 'cube' | 'diamond' }) {
  const size = shape === 'sphere' ? 9 : 8;
  return (
    <span
      style={{
        display: 'inline-block',
        width: size,
        height: size,
        background: hex(color),
        borderRadius: shape === 'sphere' ? '50%' : 2,
        transform: shape === 'diamond' ? 'rotate(45deg)' : undefined,
        verticalAlign: 'middle',
      }}
    />
  );
}

export const BRAIN_GESTURES: LegendRow[] = [
  { term: 'Drag', meaning: 'turns the graph; it drifts on its own when you let go' },
  { term: 'Scroll', meaning: 'moves you closer in or further back' },
  { term: 'Hover a node', meaning: 'its name and what it is' },
  { term: 'Click a node', meaning: 'opens it in the panel beside the graph' },
];

export const BRAIN_VOCABULARY: LegendRow[] = [
  {
    marker: <Swatch color={NODE_COLORS.person} shape="sphere" />,
    term: 'Round',
    meaning: 'a person.',
  },
  {
    marker: (
      <span style={{ display: 'inline-flex', gap: 2 }}>
        <Swatch color={NODE_COLORS.project} shape="cube" />
        <Swatch color={NODE_COLORS.tool} shape="cube" />
        <Swatch color={NODE_COLORS.organization} shape="cube" />
      </span>
    ),
    term: 'Square',
    meaning: 'a project (purple), a tool (cyan) or an organisation (orange) — same shape, colour tells them apart.',
  },
  {
    marker: (
      <span style={{ display: 'inline-flex', gap: 3 }}>
        <Swatch color={NODE_COLORS.concept} shape="diamond" />
        <Swatch color={NODE_COLORS.location} shape="diamond" />
      </span>
    ),
    term: 'Diamond',
    meaning: 'an idea (blue) or a place (green).',
  },
  {
    marker: <Swatch color={MEMORY_FRESH} shape="sphere" />,
    term: 'Small dots',
    meaning: `memories — bright when new, fading to grey as they age. A bigger one is held with more ${MEMORY_STRENGTH.one}.`,
  },
  {
    term: 'Size',
    meaning: 'how many links a thing has. Anything you keep coming back to grows.',
  },
  {
    term: 'Travelling lights',
    meaning: 'always on. They show how strong a link is by how fast they move — not live traffic.',
  },
];
