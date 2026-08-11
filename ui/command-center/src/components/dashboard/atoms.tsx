import { font, tabularNums } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

export function SectionTitle({ title, right }: { title: string; right?: string }) {
  const { colors } = useTheme();
  // 16px/600 with 14px of air below is a page heading, not a card heading. On a
  // dashboard the CONTENT is the subject and every card repeating a large title
  // is chrome competing with data. Sized down to a quiet label.
  return (
    <div style={{ display: 'flex', alignItems: 'baseline', marginBottom: 10 }}>
      <h3 style={{ fontFamily: font.display, fontSize: 13, fontWeight: 600, letterSpacing: '-0.005em', color: colors.text, margin: 0 }}>{title}</h3>
      {right && <span style={{ fontFamily: font.body, fontSize: 10.5, color: colors.textDim, marginLeft: 'auto' }}>{right}</span>}
    </div>
  );
}

/**
 * The quiet state a card shows when it has nothing to report.
 *
 * Deliberately NOT vertically centred in the card. Centring is why an empty
 * dashboard reads as broken: six cards each park one sentence in the middle of
 * 360px of nothing, and the eye has no idea where to rest. An empty card
 * should occupy the top of its box, state the fact in one line, and — where
 * there is one — offer the action that would fill it (the guidance is "tell
 * them what to do next", not "show a void").
 */
export function EmptyNote({ children, hint }: { children: React.ReactNode; hint?: string }) {
  const { colors } = useTheme();
  return (
    <div style={{ paddingTop: 2 }}>
      <div style={{ fontFamily: font.body, fontSize: 12, color: colors.textMuted, lineHeight: 1.4 }}>
        {children}
      </div>
      {hint && (
        <div style={{ fontFamily: font.body, fontSize: 10.5, color: colors.textDim, marginTop: 3, lineHeight: 1.4 }}>
          {hint}
        </div>
      )}
    </div>
  );
}

/**
 * Compact statistic. The 32px number with a 6px-gapped uppercase label was
 * built for a card with room to spare; at dashboard density it is the single
 * biggest consumer of vertical space for the least information.
 */
export function StatCompact({ label, value, cyan, delta }: {
  label: string; value: string | number; cyan?: boolean; delta?: string;
}) {
  const { colors } = useTheme();
  return (
    <div style={{ minWidth: 0 }}>
      <div style={{
        fontFamily: font.body, fontSize: 9.5, fontWeight: 600, letterSpacing: '0.08em',
        textTransform: 'uppercase', color: colors.textDim, marginBottom: 1,
        whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
      }}>{label}</div>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 5 }}>
        <span style={{
          fontFamily: font.display, fontSize: 19, fontWeight: 600, lineHeight: 1.15,
          letterSpacing: '-0.015em', ...tabularNums,
          color: cyan ? colors.cyan : colors.text,
        }}>{value}</span>
        {delta && <span style={{ fontSize: 10, fontWeight: 600, color: colors.success }}>{delta}</span>}
      </div>
    </div>
  );
}

export function Stat({ label, value, suffix, delta, cyan }: {
  label: string; value: string | number; suffix?: string; delta?: string; cyan?: boolean;
}) {
  const { colors } = useTheme();
  return (
    <div>
      <div style={{ fontSize: 11, fontWeight: 600, letterSpacing: '0.10em',
        textTransform: 'uppercase', color: colors.textDim, marginBottom: 6 }}>{label}</div>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 8 }}>
        <div style={{ fontFamily: font.display, fontSize: 32, fontWeight: 600,
          letterSpacing: '-0.02em', ...tabularNums,
          color: cyan ? colors.cyan : colors.text }}>
          {value}<span style={{ fontSize: 18, color: colors.textMuted, marginLeft: 2 }}>{suffix || ''}</span>
        </div>
        {delta && (
          <div style={{ fontSize: 11, fontWeight: 600, color: colors.success }}>{delta}</div>
        )}
      </div>
    </div>
  );
}

export function StatusIcon({ state }: { state: string }) {
  const { colors } = useTheme();
  const successBg = colors.success + '26'; // ~15% opacity
  const dangerBg = colors.danger + '1f'; // ~12% opacity
  const config: Record<string, { bg: string; color: string; icon: string }> = {
    completed: { bg: successBg, color: colors.success, icon: '✓' },
    paused: { bg: dangerBg, color: colors.danger, icon: '⏸' },
    awaiting_input: { bg: colors.cyanSoft, color: colors.cyan, icon: '?' },
  };
  const c = config[state] || config.completed;
  return (
    <div style={{
      width: 24, height: 24, borderRadius: '50%', background: c.bg,
      display: 'flex', alignItems: 'center', justifyContent: 'center',
      fontFamily: font.body, fontSize: 12, color: c.color, flexShrink: 0,
    }}>{c.icon}</div>
  );
}
