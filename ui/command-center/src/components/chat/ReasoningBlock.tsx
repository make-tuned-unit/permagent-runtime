import { useEffect, useState } from 'react';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

/**
 * Reasoning disclosure — the model's "thinking" content, shown the way premium
 * agent products do (Claude, Vercel AI Elements): a collapsible block that
 * auto-opens while the model is still thinking (no answer yet), then
 * auto-collapses to a one-line summary the moment the answer starts. The user
 * can re-open it; once they toggle, we stop auto-collapsing.
 *
 * `hasAnswer` is derived from whether the message has text yet — no streaming
 * state needs to be plumbed down.
 */
export function ReasoningBlock({ thinking, hasAnswer }: { thinking: string; hasAnswer: boolean }) {
  const { colors } = useTheme();
  const [open, setOpen] = useState(!hasAnswer);
  const [userToggled, setUserToggled] = useState(false);

  useEffect(() => {
    if (hasAnswer && !userToggled) setOpen(false);
  }, [hasAnswer, userToggled]);

  const label = hasAnswer ? 'Reasoning' : 'Thinking…';

  return (
    <div style={{ marginBottom: 8 }}>
      <button
        onClick={() => { setUserToggled(true); setOpen(o => !o); }}
        aria-expanded={open}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 6,
          background: 'transparent',
          border: 'none',
          cursor: 'pointer',
          padding: '2px 0',
          fontFamily: font.mono,
          fontSize: 11,
          letterSpacing: '0.04em',
          color: colors.textDim,
        }}
      >
        <span style={{ color: colors.cyan, opacity: hasAnswer ? 0.65 : 1 }}>✦</span>
        <span>{label}</span>
        <svg
          width="9"
          height="9"
          viewBox="0 0 10 10"
          style={{ transform: open ? 'rotate(90deg)' : 'none', transition: 'transform 160ms ease', opacity: 0.55 }}
        >
          <path d="M3 2l4 3-4 3" stroke="currentColor" strokeWidth="1.4" fill="none" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </button>

      {open && (
        <div
          style={{
            marginTop: 5,
            paddingLeft: 10,
            borderLeft: `2px solid ${colors.borderHi}`,
            fontFamily: font.body,
            fontSize: 12.5,
            lineHeight: 1.6,
            color: colors.textMuted,
            whiteSpace: 'pre-wrap',
            overflowWrap: 'break-word',
            wordBreak: 'break-word',
          }}
        >
          {thinking}
        </div>
      )}
    </div>
  );
}
