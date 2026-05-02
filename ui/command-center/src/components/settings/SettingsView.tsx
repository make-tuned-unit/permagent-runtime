import { useState, useEffect, useCallback } from 'react';
import { useCommandCenter } from '../../lib/store';
import { color, font, ease, radius } from '../../styles/tokens';
import { ProvidersSection } from './ProvidersSection';
import { usePersona } from './useSettings';

// ── Shared atoms ─────────────────────────────────────────────────────

function SectionCard({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={{
      padding: 24, borderRadius: radius.lg,
      background: 'rgba(20,28,48,0.5)',
      border: `1px solid ${color.border}`,
    }}>
      <div style={{
        fontSize: 11, fontWeight: 600, letterSpacing: '0.10em',
        textTransform: 'uppercase', color: color.textDim, marginBottom: 20,
      }}>{title}</div>
      {children}
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div style={{ marginBottom: 16 }}>
      <div style={{
        fontSize: 11, fontWeight: 600, letterSpacing: '0.10em',
        textTransform: 'uppercase', color: color.textMuted, marginBottom: 6,
      }}>{label}</div>
      {children}
    </div>
  );
}

const inputStyle: React.CSSProperties = {
  width: '100%', fontFamily: font.body, fontSize: 14, color: color.text,
  background: 'rgba(20,28,48,0.4)',
  border: `1px solid ${color.border}`, borderRadius: radius.md,
  padding: '10px 12px', outline: 'none',
  transition: `border-color 200ms ${ease.out}`,
};

function SaveButton({ onClick, disabled, saving }: {
  onClick: () => void; disabled: boolean; saving: boolean;
}) {
  return (
    <button onClick={onClick} disabled={disabled} style={{
      fontFamily: font.body, fontSize: 13, fontWeight: 600,
      padding: '8px 20px', borderRadius: radius.md,
      background: disabled ? 'rgba(0,213,255,0.08)' : color.cyan,
      color: disabled ? color.textDim : '#000',
      border: 'none', cursor: disabled ? 'default' : 'pointer',
      transition: `all 200ms ${ease.out}`,
      opacity: disabled ? 0.5 : 1,
    }}>
      {saving ? 'Saving...' : 'Save'}
    </button>
  );
}

// ── Persona Section ──────────────────────────────────────────────────

function SectionPersona() {
  const { data, loading, saving, error, save } = usePersona();
  const [name, setName] = useState('');
  const [tone, setTone] = useState('');
  const [dirty, setDirty] = useState(false);

  useEffect(() => {
    if (data) {
      setName(data.first_name);
      setTone(data.tone);
      setDirty(false);
    }
  }, [data]);

  const handleNameChange = (v: string) => { setName(v); setDirty(true); };
  const handleToneChange = (v: string) => { setTone(v); setDirty(true); };

  const handleSave = () => {
    if (!dirty) return;
    save({ first_name: name, tone });
    setDirty(false);
  };

  if (loading) return <SectionCard title="Persona"><div style={{ color: color.textDim, fontSize: 13 }}>Loading...</div></SectionCard>;

  return (
    <SectionCard title="Persona">
      <Field label="Agent name">
        <input
          value={name}
          onChange={e => handleNameChange(e.target.value)}
          style={inputStyle}
          placeholder="e.g. Henry"
          onFocus={e => e.target.style.borderColor = color.borderHi}
          onBlur={e => e.target.style.borderColor = color.border}
        />
      </Field>

      <Field label="Persona tone">
        <textarea
          value={tone}
          onChange={e => handleToneChange(e.target.value)}
          rows={3}
          style={{ ...inputStyle, resize: 'vertical', minHeight: 80 }}
          placeholder="Describe how you want your agent to operate. This shapes its tone and behavior."
          onFocus={e => (e.target as HTMLTextAreaElement).style.borderColor = color.borderHi}
          onBlur={e => (e.target as HTMLTextAreaElement).style.borderColor = color.border}
        />
        <div style={{ fontSize: 11, color: color.textDim, marginTop: 4 }}>
          Shapes how the agent communicates. Changes apply to new sessions.
        </div>
      </Field>

      {data && data.traits.length > 0 && (
        <Field label="Traits">
          <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
            {data.traits.map(t => (
              <span key={t} style={{
                fontSize: 11, fontFamily: font.mono, padding: '4px 10px',
                borderRadius: radius.pill, color: color.cyan,
                background: 'rgba(0,213,255,0.08)',
                border: `1px solid ${color.borderHi}`,
              }}>{t}</span>
            ))}
          </div>
        </Field>
      )}

      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: 12, marginTop: 8 }}>
        {error && <span style={{ fontSize: 12, color: color.danger }}>{error}</span>}
        <SaveButton onClick={handleSave} disabled={!dirty || saving} saving={saving} />
      </div>
    </SectionCard>
  );
}

// ── Providers Section (wrapped) ──────────────────────────────────────

function SectionProviders() {
  return (
    <SectionCard title="Providers &amp; Models">
      <ProvidersSection />
    </SectionCard>
  );
}

// ── About Section ────────────────────────────────────────────────────

function SectionAbout() {
  return (
    <SectionCard title="About">
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16, marginBottom: 16 }}>
        <div>
          <div style={{ fontSize: 11, color: color.textDim, textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 4 }}>App</div>
          <div style={{ fontSize: 14, fontFamily: font.display, fontWeight: 600, color: color.text }}>Permagent</div>
          <div style={{ fontSize: 12, color: color.textMuted, marginTop: 2 }}>v1.31.0</div>
        </div>
        <div>
          <div style={{ fontSize: 11, color: color.textDim, textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 4 }}>Runtime</div>
          <div style={{ fontSize: 14, fontFamily: font.display, fontWeight: 600, color: color.text }}>permagentd</div>
          <div style={{ fontSize: 12, color: color.textMuted, marginTop: 2 }}>Rust + Tauri</div>
        </div>
      </div>

      <div style={{ fontSize: 12, color: color.textMuted, lineHeight: 1.6, marginBottom: 16 }}>
        Persistent AI agent runtime with spectral memory. Built to grow with you.
      </div>

      <div style={{ display: 'flex', gap: 12 }}>
        <a
          href="https://github.com/make-tuned-unit/permagent-runtime"
          target="_blank"
          rel="noreferrer"
          style={{
            fontSize: 12, fontWeight: 500, color: color.cyan,
            textDecoration: 'none', padding: '6px 14px',
            borderRadius: radius.md, border: `1px solid ${color.borderHi}`,
            transition: `background 200ms ${ease.out}`,
          }}
          onMouseEnter={e => (e.currentTarget.style.background = 'rgba(0,213,255,0.08)')}
          onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
        >
          View source
        </a>
      </div>

      <div style={{ marginTop: 20, padding: '12px 0 0', borderTop: `1px solid ${color.border}` }}>
        <div style={{ fontSize: 11, color: color.textDim, textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 8 }}>System</div>
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8, fontSize: 12, color: color.textMuted }}>
          <div>Memory store: <span style={{ fontFamily: font.mono, fontSize: 11, color: color.textDim }}>~/.permagent/spectral/</span></div>
          <div>Config: <span style={{ fontFamily: font.mono, fontSize: 11, color: color.textDim }}>~/.permagent/config.yaml</span></div>
        </div>
      </div>
    </SectionCard>
  );
}

// ── Main Settings View ───────────────────────────────────────────────

export function SettingsView() {
  const setActivePanel = useCommandCenter(s => s.setActivePanel);

  const dismiss = useCallback(() => setActivePanel('chat'), [setActivePanel]);

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === 'Escape') { e.preventDefault(); dismiss(); }
    }
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [dismiss]);

  return (
    <div style={{
      width: '100%', height: '100%', overflowY: 'auto',
      background: 'radial-gradient(120% 80% at 50% 0%, #142035 0%, #0B1220 50%, #050810 100%)',
      padding: '28px 32px 40px',
      fontFamily: font.body, color: color.text,
    }}>
      <div style={{ maxWidth: 720 }}>
        {/* Header */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 28 }}>
          <button onClick={dismiss} style={{
            width: 32, height: 32, borderRadius: radius.md,
            background: 'transparent', border: `1px solid ${color.border}`,
            color: color.textMuted, cursor: 'pointer',
            display: 'grid', placeItems: 'center',
            transition: `all 200ms ${ease.out}`,
          }}
            onMouseEnter={e => { e.currentTarget.style.borderColor = color.borderHi; e.currentTarget.style.color = color.text; }}
            onMouseLeave={e => { e.currentTarget.style.borderColor = color.border; e.currentTarget.style.color = color.textMuted; }}
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
              <path d="M15 6l-6 6 6 6" />
            </svg>
          </button>
          <div>
            <div style={{ fontFamily: font.display, fontSize: 20, fontWeight: 600 }}>Settings</div>
            <div style={{ fontSize: 12, color: color.textMuted, marginTop: 2 }}>Manage your agent, providers, and preferences</div>
          </div>
        </div>

        {/* Section cards */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 24 }}>
          <SectionPersona />
          <SectionProviders />
          <SectionAbout />
        </div>
      </div>
    </div>
  );
}
