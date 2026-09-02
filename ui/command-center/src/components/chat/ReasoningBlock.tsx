import { useEffect, useState, type CSSProperties } from 'react';
import { FiChevronRight } from 'react-icons/fi';
import { font, textSize } from '../../styles/tokens';
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
      {/* Disclosure toggle, not an action: there is nothing to await, so the
          pending floor and success tick of the Button primitive are wrong for
          it and `Button` would displace the aria-expanded/aria-controls pairing
          that actually describes what this does. It still opts into the shared
          `.pa-btn` interaction rules — hover, press give — which an inline
          `style` object cannot express at all. (Precedent: the pick-row
          toggles in FinanceView.) */}
      <button
        type="button"
        className="pa-btn"
        onClick={() => { setUserToggled(true); setOpen(o => !o); }}
        aria-expanded={open}
        style={{
          '--pa-btn-fg': colors.textDim,
          '--pa-btn-fg-hover': colors.textMuted,
          '--pa-btn-bg-hover': 'transparent',
          '--pa-btn-pad': '2px 0',
          gap: 6,
          fontFamily: font.mono,
          fontSize: textSize.micro,
          letterSpacing: '0.04em',
        } as CSSProperties}
      >
        <span style={{ color: colors.cyan, opacity: hasAnswer ? 0.65 : 1 }}>✦</span>
        <span>{label}</span>
        <FiChevronRight
          size={9}
          style={{ transform: open ? 'rotate(90deg)' : 'none', transition: 'transform 160ms ease', opacity: 0.55 }}
        />
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
