import { useState } from 'react';
import type { ProbedMemoryRef, RecalledMemoryRef } from '../../lib/store';
import { font, ease } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

interface Props {
  probed: ProbedMemoryRef[];
  recalled: RecalledMemoryRef[];
}

export function CitationMarker({ probed, recalled }: Props) {
  const { colors } = useTheme();
  const [expanded, setExpanded] = useState(false);
  const total = probed.length + recalled.length;
  if (total === 0) return null;

  return (
    <div style={{ position: 'relative', display: 'inline-block' }}>
      <button
        onClick={() => setExpanded(!expanded)}
        style={{
          display: 'inline-flex', alignItems: 'center', gap: 4,
          padding: '2px 8px', borderRadius: 10,
          background: expanded ? colors.cyanSoft : colors.border,
          border: `1px solid ${expanded ? colors.cyanGlow : colors.border}`,
          color: expanded ? colors.cyan : colors.textDim,
          fontSize: 10, fontFamily: font.body, cursor: 'pointer',
          transition: `all 150ms ${ease.out}`,
        }}
      >
        <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
          <circle cx="12" cy="12" r="10" />
          <path d="M12 16v-4M12 8h.01" />
        </svg>
        based on {total} {total === 1 ? 'memory' : 'memories'}
      </button>

      {expanded && (
        <div style={{
          position: 'absolute', bottom: '100%', right: 0,
          marginBottom: 6, width: 320,
          background: colors.surface, backdropFilter: 'blur(16px)',
          border: `1px solid ${colors.borderHi}`,
          borderRadius: 8, padding: '8px 0',
          boxShadow: '0 8px 32px rgba(0,0,0,0.5)',
          zIndex: 100,
          maxHeight: 280, overflow: 'auto',
        }}>
          {probed.length > 0 && (
            <div style={{ padding: '4px 12px 2px', fontSize: 9, fontWeight: 600, color: colors.textMuted, textTransform: 'uppercase', letterSpacing: 0.5 }}>
              Probed memories
            </div>
          )}
          {probed.map((m, i) => (
            <div key={m.id || i} style={{ padding: '4px 12px', borderBottom: `1px solid ${colors.border}` }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 2 }}>
                <span style={{ fontSize: 10, fontFamily: font.mono, color: colors.cyan }}>
                  {m.relevance.toFixed(2)}
                </span>
                {m.wing && (
                  <span style={{ padding: '0 4px', borderRadius: 3, fontSize: 9, background: colors.cyanSoft, color: colors.cyan }}>
                    {m.wing}
                  </span>
                )}
              </div>
              <div style={{ fontSize: 11, color: colors.textMuted, lineHeight: 1.3 }}>
                {m.content_summary}
              </div>
            </div>
          ))}
          {recalled.length > 0 && (
            <div style={{ padding: '4px 12px 2px', fontSize: 9, fontWeight: 600, color: colors.textMuted, textTransform: 'uppercase', letterSpacing: 0.5 }}>
              Recalled memories
            </div>
          )}
          {recalled.map((m, i) => (
            <div key={m.id || i} style={{ padding: '4px 12px', borderBottom: `1px solid ${colors.border}` }}>
              <div style={{ fontSize: 10, fontFamily: font.mono, color: colors.purple, marginBottom: 2 }}>
                score: {m.signal_score.toFixed(2)}
              </div>
              <div style={{ fontSize: 11, color: colors.textMuted, lineHeight: 1.3 }}>
                {m.content_summary}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
