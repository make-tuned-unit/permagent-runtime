import { color, font } from '../../styles/tokens';

export function SectionTitle({ title, right }: { title: string; right?: string }) {
  return (
    <div style={{ display: 'flex', alignItems: 'baseline', marginBottom: 14 }}>
      <h3 style={{ fontFamily: font.display, fontSize: 16, fontWeight: 600, color: color.text, margin: 0 }}>{title}</h3>
      {right && <span style={{ fontFamily: font.body, fontSize: 11, color: color.textDim, marginLeft: 'auto' }}>{right}</span>}
    </div>
  );
}

export function Stat({ label, value, suffix, delta }: {
  label: string; value: string | number; suffix?: string; delta?: string;
}) {
  return (
    <div>
      <div style={{ fontFamily: font.mono, fontSize: 11, fontWeight: 600, color: color.textDim, textTransform: 'uppercase', letterSpacing: '0.10em', marginBottom: 6 }}>
        {label}
      </div>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 4 }}>
        <span style={{ fontFamily: font.display, fontSize: 32, fontWeight: 600, color: color.text }}>{value}</span>
        {suffix && <span style={{ fontFamily: font.display, fontSize: 18, color: color.textMuted }}>{suffix}</span>}
      </div>
      {delta && (
        <div style={{ fontFamily: font.mono, fontSize: 11, color: '#5BD17F', marginTop: 4 }}>
          {delta}
        </div>
      )}
    </div>
  );
}

export function StatusIcon({ state }: { state: string }) {
  const config: Record<string, { bg: string; color: string; icon: string }> = {
    completed: { bg: 'rgba(91,209,127,0.15)', color: '#5BD17F', icon: '✓' },
    paused: { bg: 'rgba(255,180,162,0.12)', color: color.danger, icon: '⏸' },
    awaiting_input: { bg: 'rgba(0,213,255,0.12)', color: color.cyan, icon: '?' },
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
