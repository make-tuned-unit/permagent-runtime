import { useState, useEffect, useCallback } from 'react';
import { useCommandCenter } from '../../lib/store';
import { color, font, ease } from '../../styles/tokens';
import { Mobius } from '../mobius/Mobius';
import { ProvidersSection } from './ProvidersSection';
import { usePersona } from './useSettings';
import { H1, Section, Row, TextInput, Chip, SaveButton } from './atoms';

// ── Nav rail categories (per design lines 12-32) ─────────────────────

const CATEGORIES = [
  { group: 'You', items: [
    { key: 'profile',     label: 'Profile',          icon: 'M12 12a4 4 0 100-8 4 4 0 000 8zM4 21a8 8 0 0116 0' },
    { key: 'preferences', label: 'Preferences',      icon: 'M3 6h18M6 12h12M10 18h4' },
  ]},
  { group: 'Agent', items: [
    { key: 'agent',       label: 'Persona',          icon: 'M12 2a4 4 0 014 4v3a4 4 0 11-8 0V6a4 4 0 014-4zM4 21v-2a6 6 0 016-6h4a6 6 0 016 6v2' },
    { key: 'memory',      label: 'Memory',           icon: 'M9 4a4 4 0 00-4 4 3 3 0 00-1 5.5A3 3 0 005 18a4 4 0 004 3M15 4a4 4 0 014 4 3 3 0 011 5.5A3 3 0 0119 18a4 4 0 01-4 3' },
    { key: 'autonomy',    label: 'Autonomy & guardrails', icon: 'M12 2l9 4v6c0 5-4 9-9 10-5-1-9-5-9-10V6l9-4z' },
  ]},
  { group: 'Connections', items: [
    { key: 'tools',       label: 'Tools & MCPs',     icon: 'M14.7 6.3a1 1 0 011.4 0l1.6 1.6a1 1 0 010 1.4l-9 9-3 .6.6-3 9-9.6zM3 21h18' },
    { key: 'models',      label: 'Models',           icon: 'M3 12h4l3-9 4 18 3-9h4' },
    { key: 'keys',        label: 'API keys',         icon: 'M14 8a4 4 0 100 8 4 4 0 000-8zm0 4l-9 9m4-4l3 3' },
  ]},
  { group: 'System', items: [
    { key: 'appearance',  label: 'Appearance',       icon: 'M12 3a9 9 0 100 18 9 9 0 000-18zM12 3v18M3 12h18' },
    { key: 'shortcuts',   label: 'Shortcuts',        icon: 'M4 6h16v12H4zM8 10h.01M12 10h.01M16 10h.01M7 14h10' },
    { key: 'data',        label: 'Data & privacy',   icon: 'M12 2l9 4v6c0 5-4 9-9 10-5-1-9-5-9-10V6l9-4zM9 12l2 2 4-4' },
  ]},
];

// ── Persona Panel ────────────────────────────────────────────────────

function PersonaPanel() {
  const { data, loading, saving, error, save } = usePersona();
  const [name, setName] = useState('');
  const [greeting, setGreeting] = useState('');
  const [tone, setTone] = useState('');
  const [traits, setTraits] = useState<string[]>([]);
  const [dirty, setDirty] = useState(false);

  const TRAIT_OPTIONS = ['curious', 'direct', 'patient', 'playful', 'formal', 'concise', 'thorough', 'opinionated'];

  useEffect(() => {
    if (data) {
      setName(data.first_name);
      setGreeting(data.opening_greeting);
      setTone(data.tone);
      setTraits(data.traits);
      setDirty(false);
    }
  }, [data]);

  const edit = <T,>(setter: (v: T) => void) => (v: T) => { setter(v); setDirty(true); };

  const toggleTrait = (t: string) => {
    setTraits(prev => prev.includes(t) ? prev.filter(x => x !== t) : [...prev, t]);
    setDirty(true);
  };

  const handleSave = () => {
    if (!dirty) return;
    save({ first_name: name, opening_greeting: greeting, tone, traits });
    setDirty(false);
  };

  if (loading) return <div style={{ color: color.textDim, fontSize: 13 }}>Loading persona...</div>;

  return (
    <div>
      <H1 sub="Shape how your agent thinks, talks, and decides. Changes take effect at the start of the next conversation.">Persona</H1>

      <Section title="Identity">
        <div style={{ display: 'flex', alignItems: 'center', gap: 24, marginBottom: 8 }}>
          <Mobius size={140} state="idle" glow={1} />
          <div style={{ flex: 1 }}>
            <Row label="Name" hint="What you'll call them.">
              <TextInput value={name} onChange={edit(setName)} />
            </Row>
            <Row label="Greeting" hint="The first line they'll say each session.">
              <TextInput multi value={greeting} onChange={edit(setGreeting)} />
            </Row>
          </div>
        </div>
      </Section>

      <Section title="Tone">
        <Row label="Voice" hint="How they describe their own voice.">
          <TextInput multi value={tone} onChange={edit(setTone)} />
        </Row>
        <Row label="Traits" hint="Pick 3-5. The agent will lean into these.">
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8 }}>
            {TRAIT_OPTIONS.map(t => (
              <Chip key={t} on={traits.includes(t)} onClick={() => toggleTrait(t)}>{t}</Chip>
            ))}
          </div>
        </Row>
      </Section>

      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: 12 }}>
        {error && <span style={{ fontSize: 12, color: color.danger }}>{error}</span>}
        <SaveButton onClick={handleSave} disabled={!dirty || saving} saving={saving} />
      </div>
    </div>
  );
}

// ── Models Panel (wraps existing ProvidersSection) ───────────────────

function ModelsPanel() {
  return (
    <div>
      <H1 sub="Pick the brains behind the agent. Configure providers, manage API keys, and set defaults.">Models</H1>
      <Section title="Providers">
        <ProvidersSection />
      </Section>
    </div>
  );
}

// ── Stub Panels ──────────────────────────────────────────────────────

function StubPanel({ title, sub }: { title: string; sub: string }) {
  return (
    <div>
      <H1 sub={sub}>{title}</H1>
      <Section title="Coming soon">
        <div style={{ color: color.textMuted, fontSize: 13, lineHeight: 1.6 }}>
          This panel is on the way. Check back soon.
        </div>
      </Section>
    </div>
  );
}

const STUBS: Record<string, { title: string; sub: string }> = {
  profile:     { title: 'Profile',     sub: 'Your account, profile photo, and personal details.' },
  preferences: { title: 'Preferences', sub: 'Defaults, locale, and how the app feels by default.' },
  memory:      { title: 'Memory',      sub: 'How long things stick. What gets forgotten. What never does.' },
  autonomy:    { title: 'Autonomy & guardrails', sub: 'How far the agent can go without asking. Spending caps. Approval boundaries.' },
  tools:       { title: 'Tools & MCPs', sub: 'External services your agent can call. Permissions per session.' },
  keys:        { title: 'API keys',    sub: 'Bring-your-own keys for OpenAI, Anthropic, and others.' },
  appearance:  { title: 'Appearance',  sub: 'Theme, accent color, density.' },
  shortcuts:   { title: 'Shortcuts',   sub: 'Keyboard shortcuts you can customize.' },
  data:        { title: 'Data & privacy', sub: 'Where your data lives. How to export or wipe it.' },
};

// ── Main Settings View ───────────────────────────────────────────────

export function SettingsView() {
  const setActivePanel = useCommandCenter(s => s.setActivePanel);
  const [section, setSection] = useState('agent');

  const dismiss = useCallback(() => setActivePanel('chat'), [setActivePanel]);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); dismiss(); } };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [dismiss]);

  const renderPanel = () => {
    if (section === 'agent') return <PersonaPanel />;
    if (section === 'models') return <ModelsPanel />;
    const stub = STUBS[section];
    if (stub) return <StubPanel title={stub.title} sub={stub.sub} />;
    return null;
  };

  return (
    <div style={{
      width: '100%', height: '100%', display: 'flex',
      background: '#0B1220', color: color.text, fontFamily: font.body,
    }}>
      {/* Nav rail */}
      <div style={{
        width: 240, borderRight: `1px solid ${color.border}`,
        background: 'rgba(7,11,20,0.4)',
        padding: '24px 14px', overflow: 'auto', flexShrink: 0,
      }}>
        <div style={{ fontFamily: font.display, fontSize: 18, fontWeight: 700,
          letterSpacing: '-0.01em', padding: '0 10px 18px' }}>Settings</div>

        {CATEGORIES.map(cat => (
          <div key={cat.group} style={{ marginBottom: 16 }}>
            <div style={{ fontSize: 10, fontWeight: 600, letterSpacing: '0.10em',
              textTransform: 'uppercase', color: color.textDim,
              padding: '0 10px 6px' }}>{cat.group}</div>
            {cat.items.map(it => {
              const on = section === it.key;
              return (
                <button key={it.key} onClick={() => setSection(it.key)} style={{
                  display: 'flex', alignItems: 'center', gap: 10,
                  width: '100%', padding: '8px 10px', borderRadius: 8,
                  background: on ? 'rgba(0,213,255,0.08)' : 'transparent',
                  border: on ? `1px solid ${color.borderHi}` : '1px solid transparent',
                  color: on ? color.cyan : color.textMuted,
                  cursor: 'pointer', textAlign: 'left',
                  fontFamily: font.body, fontSize: 13,
                  fontWeight: on ? 600 : 500,
                  transition: `all 140ms ${ease.out}`,
                }}>
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none"
                    stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" strokeLinejoin="round">
                    <path d={it.icon} />
                  </svg>
                  {it.label}
                </button>
              );
            })}
          </div>
        ))}
      </div>

      {/* Content panel */}
      <div style={{ flex: 1, overflow: 'auto', padding: '32px 40px 60px' }}>
        {renderPanel()}
      </div>
    </div>
  );
}
