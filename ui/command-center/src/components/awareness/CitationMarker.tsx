import { useId, useState, type CSSProperties } from 'react';
import type { ProbedMemoryRef, RecalledMemoryRef } from '../../lib/store';
import { useCommandCenter } from '../../lib/store';
import type { BrainMemoryTarget } from '../brain/brainMemoryFocus';
import { probedFocusTarget, recalledFocusTarget } from './citationFocus';
import { ease, font, radius, type ThemeColors, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

interface Props {
  probed: ProbedMemoryRef[];
  recalled: RecalledMemoryRef[];
}

export function CitationMarker({ probed, recalled }: Props) {
  const { colors } = useTheme();
  const focusBrainMemory = useCommandCenter(s => s.focusBrainMemory);
  const listId = useId();
  const [expanded, setExpanded] = useState(false);
  const total = probed.length + recalled.length;
  if (total === 0) return null;

  // Close the "see-but-can't-touch" loop: a cited memory deep-links into the
  // Brain via the shared focus seam (#753), graph-preferred with a preview
  // fallback so a fresh, not-yet-in-graph memory still opens. Collapse the
  // popover on the way out (focusBrainMemory switches to the Brain workspace).
  const open = (target: BrainMemoryTarget) => {
    focusBrainMemory(target);
    setExpanded(false);
  };

  return (
    <div style={{ position: 'relative', display: 'inline-block' }}>
      {/* A disclosure toggle, not an action: it opens the citation list below
          and there is nothing to await, so the pending floor and the success
          tick would both be the wrong signal. It takes the shared `.pa-btn`
          interaction rules directly and keeps the aria pairing that describes
          what pressing it does. */}
      <button
        type="button"
        className="pa-btn"
        aria-expanded={expanded}
        aria-controls={listId}
        onClick={() => setExpanded(!expanded)}
        style={{
          '--pa-btn-bg': expanded ? colors.purpleSoft : `${colors.purple}18`,
          '--pa-btn-fg': expanded ? colors.purple : colors.textDim,
          '--pa-btn-border': expanded ? colors.purpleGlow : `${colors.purple}30`,
          '--pa-btn-bg-hover': colors.purpleSoft,
          '--pa-btn-fg-hover': colors.purple,
          '--pa-btn-border-hover': colors.purpleGlow,
          '--pa-btn-bg-active': colors.purpleSoft,
          '--pa-btn-pad': '2px 8px',
          '--pa-btn-radius': '10px',
          gap: 4,
          fontSize: 10, fontFamily: font.body,
        } as CSSProperties}
      >
        <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
          <circle cx="12" cy="12" r="10" />
          <path d="M12 16v-4M12 8h.01" />
        </svg>
        based on {total} {total === 1 ? 'memory' : 'memories'}
      </button>

      {expanded && (
        <div id={listId} style={{
          position: 'absolute', bottom: '100%', right: 0,
          marginBottom: 6, width: 320,
          background: colors.surface, backdropFilter: 'blur(16px)',
          border: `1px solid ${colors.borderHi}`,
          borderRadius: radius.md, padding: '8px 0',
          boxShadow: colors.cardShadow,
          ...(colors.cardHighlight ? { boxShadow: `${colors.cardShadow}, ${colors.cardHighlight}` } : {}),
          zIndex: 100,
          maxHeight: 280, overflow: 'auto',
        }}>
          {probed.length > 0 && (
            <div style={{ padding: '4px 12px 2px', fontSize: 10, fontWeight: 600, fontFamily: font.display, color: colors.textMuted, textTransform: 'uppercase', letterSpacing: 0.5 }}>
              Probed memories
            </div>
          )}
          {probed.map((m, i) => (
            <button
              key={m.id || i}
              type="button"
              onClick={() => open(probedFocusTarget(m))}
              title="Open in Brain"
              onMouseEnter={e => { (e.currentTarget as HTMLElement).style.background = colors.purpleSoft; }}
              onMouseLeave={e => { (e.currentTarget as HTMLElement).style.background = 'transparent'; }}
              style={memoryRowStyle(colors)}
            >
              <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 2 }}>
                <span style={{ fontSize: 10, fontFamily: font.mono, color: colors.cyan }}>
                  {m.relevance.toFixed(2)}
                </span>
                {m.wing && (
                  <span style={{ padding: '0 4px', borderRadius: 3, fontSize: 10, background: colors.cyanSoft, color: colors.cyan }}>
                    {m.wing}
                  </span>
                )}
                <span style={{ flex: 1 }} />
                <OpenArrow color={colors.purple} />
              </div>
              <div style={{ fontSize: textSize.micro, fontFamily: font.body, color: colors.textMuted, lineHeight: 1.3 }}>
                {m.content_summary}
              </div>
            </button>
          ))}
          {recalled.length > 0 && (
            <div style={{ padding: '4px 12px 2px', fontSize: 10, fontWeight: 600, fontFamily: font.display, color: colors.textMuted, textTransform: 'uppercase', letterSpacing: 0.5 }}>
              Recalled memories
            </div>
          )}
          {recalled.map((m, i) => (
            <button
              key={m.id || i}
              type="button"
              onClick={() => open(recalledFocusTarget(m))}
              title="Open in Brain"
              onMouseEnter={e => { (e.currentTarget as HTMLElement).style.background = colors.purpleSoft; }}
              onMouseLeave={e => { (e.currentTarget as HTMLElement).style.background = 'transparent'; }}
              style={memoryRowStyle(colors)}
            >
              <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 2 }}>
                <span style={{ fontSize: 10, fontFamily: font.mono, color: colors.purple }}>
                  score: {m.signal_score.toFixed(2)}
                </span>
                <span style={{ flex: 1 }} />
                <OpenArrow color={colors.purple} />
              </div>
              <div style={{ fontSize: textSize.micro, fontFamily: font.body, color: colors.textMuted, lineHeight: 1.3 }}>
                {m.content_summary}
              </div>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** Shared style for a clickable citation row (each opens its memory in Brain).
 *  These rows stay raw `<button>`s: the row IS the button, a stacked block of
 *  a meta line and a summary line laid out by the button itself, and
 *  `.pa-btn`'s inline-flex + centring would collapse that into one centred
 *  row. */
function memoryRowStyle(colors: ThemeColors): React.CSSProperties {
  return {
    display: 'block', width: '100%', textAlign: 'left',
    background: 'transparent', border: 'none', borderBottom: `1px solid ${colors.border}`,
    padding: '4px 12px', cursor: 'pointer', color: 'inherit', fontFamily: font.body,
    transition: `background 120ms ${ease.out}`,
  };
}

/** "Open ↗" affordance — signals the row deep-links into the Brain. */
function OpenArrow({ color }: { color: string }) {
  return (
    <svg
      width="11" height="11" viewBox="0 0 24 24" fill="none"
      stroke={color} strokeWidth={2} strokeLinecap="round" strokeLinejoin="round"
      aria-hidden style={{ flexShrink: 0, opacity: 0.75 }}
    >
      <line x1="7" y1="17" x2="17" y2="7" />
      <polyline points="7 7 17 7 17 17" />
    </svg>
  );
}
