import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

export function SectionTitle({ title, right }: { title: string; right?: string }) {
  const { colors } = useTheme();
  return (
    <div style={{ display: 'flex', alignItems: 'baseline', marginBottom: 14 }}>
      <h3 style={{ fontFamily: font.display, fontSize: 16, fontWeight: 600, color: colors.text, margin: 0 }}>{title}</h3>
      {right && <span style={{ fontFamily: font.body, fontSize: 11, color: colors.textDim, marginLeft: 'auto' }}>{right}</span>}
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
          letterSpacing: '-0.02em',
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
