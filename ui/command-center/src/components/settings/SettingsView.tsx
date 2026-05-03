import { useState, useEffect, useCallback } from 'react';
import { useCommandCenter } from '../../lib/store';
import { api } from '../../lib/api';
import { color, font, ease } from '../../styles/tokens';
import { Mobius } from '../mobius/Mobius';
import { ProvidersSection } from './ProvidersSection';
import { usePersona } from './useSettings';
import { H1, Section, Row, TextInput, Chip, Toggle, Slider, Kbd, SaveButton } from './atoms';

function PreviewBadge() {
  return (
    <span style={{
      fontSize: 10, fontWeight: 600, letterSpacing: '0.06em', textTransform: 'uppercase',
      padding: '3px 8px', borderRadius: 999, marginLeft: 10,
      background: 'rgba(141,68,174,0.12)', color: '#C893E0',
      border: '1px solid rgba(141,68,174,0.25)',
    }}>preview</span>
  );
}

// ── Shared button styles ─────────────────────────────────────────────

const ghost: React.CSSProperties = {
  height: 32, padding: '0 14px', borderRadius: 8,
  background: 'transparent', border: `1px solid ${color.border}`,
  color: color.text, cursor: 'pointer',
  fontFamily: font.body, fontSize: 12, fontWeight: 500,
  display: 'inline-flex', alignItems: 'center', gap: 6,
};
const selectStyle: React.CSSProperties = {
  height: 34, padding: '0 12px', borderRadius: 8,
  background: 'rgba(7,11,20,0.5)', border: `1px solid ${color.border}`,
  color: color.text, fontFamily: font.body, fontSize: 13,
  minWidth: 240, cursor: 'pointer',
};

// ── Nav rail categories ──────────────────────────────────────────────

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

// ── Panels ───────────────────────────────────────────────────────────

function PersonaPanel() {
  const { data, loading, saving, error, save } = usePersona();
  const [name, setName] = useState('');
  const [greeting, setGreeting] = useState('');
  const [tone, setTone] = useState('');
  const [traits, setTraits] = useState<string[]>([]);
  const [dirty, setDirty] = useState(false);
  const TRAIT_OPTIONS = ['curious', 'direct', 'patient', 'playful', 'formal', 'concise', 'thorough', 'opinionated'];

  useEffect(() => {
    if (data) { setName(data.first_name); setGreeting(data.opening_greeting); setTone(data.tone); setTraits(data.traits); setDirty(false); }
  }, [data]);

  const ed = <T,>(s: (v: T) => void) => (v: T) => { s(v); setDirty(true); };
  const toggleTrait = (t: string) => { setTraits(p => p.includes(t) ? p.filter(x => x !== t) : [...p, t]); setDirty(true); };
  const handleSave = () => { if (dirty) { save({ first_name: name, opening_greeting: greeting, tone, traits }); setDirty(false); } };

  if (loading) return <div style={{ color: color.textDim, fontSize: 13 }}>Loading persona...</div>;
  return (
    <div>
      <H1 sub="Shape how your agent thinks, talks, and decides. Changes take effect at the start of the next conversation.">Persona</H1>
      <Section title="Identity">
        <div style={{ display: 'flex', alignItems: 'center', gap: 24, marginBottom: 8 }}>
          <Mobius size={140} state="idle" glow={1} />
          <div style={{ flex: 1 }}>
            <Row label="Name" hint="What you'll call them."><TextInput value={name} onChange={ed(setName)} /></Row>
            <Row label="Greeting" hint="The first line they'll say each session."><TextInput multi value={greeting} onChange={ed(setGreeting)} /></Row>
          </div>
        </div>
      </Section>
      <Section title="Tone">
        <Row label="Voice" hint="How they describe their own voice."><TextInput multi value={tone} onChange={ed(setTone)} /></Row>
        <Row label="Traits" hint="Pick 3-5. The agent will lean into these.">
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8 }}>
            {TRAIT_OPTIONS.map(t => <Chip key={t} on={traits.includes(t)} onClick={() => toggleTrait(t)}>{t}</Chip>)}
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

function ProfilePanel() {
  return (
    <div>
      <H1 sub="Your account — visible to your agent and to anyone you share a workspace with."><>Profile<PreviewBadge /></></H1>
      <Section title="Account">
        <Row label="Display name"><TextInput value="Jesse Sharratt" /></Row>
        <Row label="Email" hint="Used for sign-in and notifications."><TextInput value="" placeholder="email@example.com" /></Row>
        <Row label="Workspace">
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <div style={{ width: 28, height: 28, borderRadius: 6, background: 'linear-gradient(135deg, #8D44AE, #00D5FF)' }} />
            <TextInput value="Personal" />
          </div>
        </Row>
      </Section>
    </div>
  );
}

function PreferencesPanel() {
  const [prefs, setPrefs] = useState([false, true, true, true]);
  return (
    <div>
      <H1 sub="Defaults that follow you across sessions. Changes saved locally."><>Preferences<PreviewBadge /></></H1>
      <Section title="Defaults">
        <Row label="Open on launch" hint="Where Permagent lands when you open the app.">
          <select style={selectStyle}><option>Mission control</option><option>Last open task</option><option>Build</option><option>Brain</option></select>
        </Row>
        <Row label="When you ask, agent should…">
          <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
            {['Always confirm before running tools that change state', 'Show me the plan before starting', "Stream replies as they're generated", 'Read recent memory at start of every session'].map((l, i) => (
              <div key={l} style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                <Toggle on={prefs[i]} onChange={v => setPrefs(p => { const n = [...p]; n[i] = v; return n; })} />
                <span style={{ fontSize: 13 }}>{l}</span>
              </div>
            ))}
          </div>
        </Row>
      </Section>
      <Section title="Notifications">
        <Row label="When agent finishes a task"><select style={selectStyle}><option>Desktop notification</option><option>In-app only</option><option>Silent</option></select></Row>
        <Row label="When agent needs your input"><select style={selectStyle}><option>Desktop + sound</option><option>Desktop only</option><option>Silent</option></select></Row>
      </Section>
    </div>
  );
}

function MemoryPanel() {
  const [maxMem, setMaxMem] = useState(1200);
  const [forget, setForget] = useState(45);
  const [rememberFlags, setRememberFlags] = useState([true, true, true, true, false]);
  const rememberItems = ['Names of people I\'ve introduced', 'Project goals and deadlines', 'Code style preferences from feedback', 'Things I dismissed or pushed back on', 'Sensitive context (medical, financial)'];
  return (
    <div>
      <H1 sub="What your agent remembers about you, your projects, and the people in your world."><>Memory<PreviewBadge /></></H1>
      <Section title="Memory budget" sub="When the brain gets too full, the agent will compress or forget the lowest-signal items first.">
        <Row label="Max memory" hint="Soft cap. The agent prefers to keep things small."><Slider value={maxMem} onChange={setMaxMem} min={200} max={5000} suffix=" nodes" /></Row>
        <Row label="Forget threshold" hint="Items not touched in this many days become candidates for compression."><Slider value={forget} onChange={setForget} min={7} max={365} suffix="d" /></Row>
      </Section>
      <Section title="What to remember">
        {rememberItems.map((l, i) => (
          <div key={l} style={{ display: 'flex', alignItems: 'center', gap: 14, padding: '10px 0', borderTop: `1px solid ${color.border}` }}>
            <Toggle on={rememberFlags[i]} onChange={v => setRememberFlags(p => { const n = [...p]; n[i] = v; return n; })} />
            <span style={{ fontSize: 13, flex: 1 }}>{l}</span>
          </div>
        ))}
      </Section>
      <Section title="Manage">
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
          <button style={ghost}>Open Brain view</button>
          <button style={ghost}>Export memory</button>
          <button style={{ ...ghost, color: color.danger, borderColor: 'rgba(255,180,162,0.30)' }}>Forget everything</button>
        </div>
      </Section>
    </div>
  );
}

function AutonomyPanel() {
  const [trust, setTrust] = useState(1);
  const [confirms, setConfirms] = useState([true, true, true, true, false]);
  const [perSession, setPerSession] = useState(5);
  const [perDay, setPerDay] = useState(20);
  const trustLevels = [
    { l: 'Tight', d: 'Always ask first' },
    { l: 'Default', d: 'Ask for state changes' },
    { l: 'Loose', d: 'Run, summarize after' },
    { l: 'YOLO', d: 'Trust me' },
  ];
  const confirmItems = ['Sending email or messages', 'Spending money', 'Deleting files or records', 'Pushing to main / production', 'Reading sensitive memory'];
  return (
    <div>
      <H1 sub="How much your agent can do without checking in. Higher autonomy = faster, but more rope."><>Autonomy &amp; guardrails<PreviewBadge /></></H1>
      <Section title="Default autonomy">
        <Row label="Trust level" hint="Applies when no project-specific level is set.">
          <div style={{ display: 'flex', gap: 6 }}>
            {trustLevels.map((opt, i) => (
              <button key={opt.l} onClick={() => setTrust(i)} style={{
                padding: 12, borderRadius: 10, cursor: 'pointer',
                background: trust === i ? 'rgba(0,213,255,0.10)' : 'rgba(20,28,48,0.4)',
                border: trust === i ? `1px solid ${color.borderHi}` : `1px solid ${color.border}`,
                color: color.text, textAlign: 'left', flex: 1, fontFamily: font.body,
              }}>
                <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 4, color: trust === i ? color.cyan : color.text }}>{opt.l}</div>
                <div style={{ fontSize: 11, color: color.textMuted }}>{opt.d}</div>
              </button>
            ))}
          </div>
        </Row>
      </Section>
      <Section title="Always confirm before…">
        {confirmItems.map((l, i) => (
          <div key={l} style={{ display: 'flex', alignItems: 'center', gap: 14, padding: '10px 0', borderTop: `1px solid ${color.border}` }}>
            <Toggle on={confirms[i]} onChange={v => setConfirms(p => { const n = [...p]; n[i] = v; return n; })} />
            <span style={{ fontSize: 13, flex: 1 }}>{l}</span>
          </div>
        ))}
      </Section>
      <Section title="Spend cap" sub="Hard ceiling. Agent will stop and ask for more rope before exceeding.">
        <Row label="Per session"><Slider value={perSession} onChange={setPerSession} min={0} max={50} suffix=" USD" /></Row>
        <Row label="Per day"><Slider value={perDay} onChange={setPerDay} min={0} max={200} suffix=" USD" /></Row>
      </Section>
    </div>
  );
}

function ToolsPanel() {
  const [extensions, setExtensions] = useState<Array<{ enabled: boolean; type: string; name: string; description: string; display_name: string; bundled: boolean; available_tools: string[] }>>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    api.getExtensions().then(r => { setExtensions(r.extensions); setLoading(false); }).catch(() => setLoading(false));
  }, []);

  const enabledCount = extensions.filter(e => e.enabled).length;

  return (
    <div>
      <H1 sub="Tools your agent can use. These follow the Model Context Protocol — connect a server and the agent can call into it.">Tools &amp; MCPs</H1>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 16 }}>
        <div style={{ flex: 1 }} />
        <span style={{ fontSize: 12, color: color.textMuted }}>{enabledCount} of {extensions.length} enabled</span>
      </div>
      {loading ? (
        <div style={{ color: color.textDim, fontSize: 13 }}>Loading extensions...</div>
      ) : extensions.length === 0 ? (
        <Section title="No extensions"><div style={{ color: color.textMuted, fontSize: 13 }}>No MCP tools or extensions configured.</div></Section>
      ) : (
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(2, 1fr)', gap: 10 }}>
          {extensions.map(ext => (
            <div key={ext.name} style={{ display: 'flex', alignItems: 'center', gap: 14, padding: 14, borderRadius: 10, background: 'rgba(20,28,48,0.4)', border: `1px solid ${color.border}` }}>
              <div style={{ width: 32, height: 32, borderRadius: 8, background: ext.enabled ? 'rgba(0,213,255,0.08)' : 'rgba(255,255,255,0.04)', border: `1px solid ${ext.enabled ? color.borderHi : color.border}`, display: 'grid', placeItems: 'center', fontFamily: font.display, fontSize: 13, fontWeight: 700, color: ext.enabled ? color.cyan : color.textMuted }}>{ext.display_name[0]?.toUpperCase()}</div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13, fontWeight: 600 }}>{ext.display_name}</div>
                <div style={{ fontSize: 11, color: color.textMuted, marginTop: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{ext.type}{ext.bundled ? ' · bundled' : ''} · {ext.available_tools.length} tools</div>
              </div>
              <div style={{ width: 8, height: 8, borderRadius: '50%', background: ext.enabled ? '#5BD17F' : color.textDim, flexShrink: 0 }} />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function ModelsPanel() {
  return (
    <div>
      <H1 sub="Pick the brains behind the agent. Use stronger models when stakes are high; cheaper for routine work.">Models</H1>
      <Section title="Providers">
        <ProvidersSection />
      </Section>
      <Section title="Routing">
        <Row label="Primary" hint="Used for thinking, planning, and replies."><select style={selectStyle}><option>Claude Sonnet 4.5</option><option>Claude Opus 4.5</option><option>GPT-5</option><option>Gemini 2.5 Pro</option></select></Row>
        <Row label="Quick" hint="Used for short tools, classification, and summaries."><select style={selectStyle}><option>Claude Haiku 4.5</option><option>GPT-5 mini</option><option>Llama 4 70B</option></select></Row>
        <Row label="Reasoning" hint="Used for hard plans, math, and code review."><select style={selectStyle}><option>o3</option><option>Claude Opus 4.5</option></select></Row>
      </Section>
      <Section title="Behavior">
        <Row label="Temperature" hint="Higher = more wandering. Lower = more grounded."><Slider value={50} suffix="%" /></Row>
        <Row label="Max thinking budget" hint="How many tokens to spend on a single thought."><Slider value={4096} min={512} max={32768} suffix=" tok" /></Row>
      </Section>
    </div>
  );
}

function KeysPanel() {
  const providers = useCommandCenter(s => s.providers);
  const loadProviders = useCommandCenter(s => s.loadProviders);
  const [maskedKeys, setMaskedKeys] = useState<Record<string, string>>({});

  useEffect(() => { loadProviders(); }, [loadProviders]);

  // Load masked values for configured providers' secret keys
  useEffect(() => {
    const loadMasked = async () => {
      const masked: Record<string, string> = {};
      for (const p of providers) {
        if (!p.isConfigured) continue;
        for (const ck of p.configKeys || []) {
          if (ck.secret) {
            try {
              const res = await api.readConfig(ck.name, true);
              if (res.masked_value) masked[ck.name] = res.masked_value;
            } catch { /* ignore */ }
          }
        }
      }
      setMaskedKeys(masked);
    };
    if (providers.length > 0) loadMasked();
  }, [providers]);

  // Show providers that have secret config keys
  const providersWithKeys = providers.filter(p => p.configKeys?.some(k => k.secret));

  return (
    <div>
      <H1 sub="Bring your own keys for the providers you use. Keys are encrypted and never leave your device.">API keys</H1>
      <Section title="Providers">
        {providersWithKeys.length === 0 && providers.length === 0 && (
          <div style={{ color: color.textDim, fontSize: 13 }}>Loading providers...</div>
        )}
        {providersWithKeys.length === 0 && providers.length > 0 && (
          <div style={{ color: color.textMuted, fontSize: 13 }}>Configure providers in the Models panel to manage API keys.</div>
        )}
        {providersWithKeys.map(p => (
          <Row key={p.name} label={p.displayName}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <TextInput mono value={maskedKeys[`${p.name.toUpperCase()}_API_KEY`] || (p.isConfigured ? '••••••••' : '')} placeholder={p.isConfigured ? '' : 'paste key…'} />
              <span style={{
                fontSize: 10, fontWeight: 600, letterSpacing: '0.06em', textTransform: 'uppercase',
                color: p.isConfigured ? '#5BD17F' : color.textDim,
              }}>{p.isConfigured ? 'set' : 'missing'}</span>
            </div>
          </Row>
        ))}
      </Section>
    </div>
  );
}

function AppearancePanel() {
  const [theme, setTheme] = useState(0);
  const [glow, setGlow] = useState(70);
  const [anim, setAnim] = useState(1);
  const [density, setDensity] = useState(1);
  const [showHero, setShowHero] = useState(true);
  const [reduceMotion, setReduceMotion] = useState(false);
  const themes = [
    { l: 'Permagent dark', g: 'linear-gradient(135deg, #0B1220, #1E2433)' },
    { l: 'Aurora', g: 'linear-gradient(135deg, #0B1220 30%, #8D44AE)' },
    { l: 'Slate', g: 'linear-gradient(135deg, #161B26, #2A3040)' },
  ];
  return (
    <div>
      <H1 sub="How Permagent looks while it runs alongside you."><>Appearance<PreviewBadge /></></H1>
      <Section title="Theme">
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 10 }}>
          {themes.map((th, i) => (
            <div key={th.l} onClick={() => setTheme(i)} style={{
              padding: 4, borderRadius: 12, cursor: 'pointer',
              border: theme === i ? `2px solid ${color.cyan}` : '2px solid transparent',
              boxShadow: theme === i ? `0 0 14px ${color.cyanGlow}` : 'none',
            }}>
              <div style={{ height: 96, borderRadius: 8, background: th.g, border: `1px solid ${color.border}` }} />
              <div style={{ fontSize: 12, padding: '8px 4px', textAlign: 'center', color: theme === i ? color.cyan : color.text }}>{th.l}</div>
            </div>
          ))}
        </div>
      </Section>
      <Section title="Möbius">
        <Row label="Glow intensity"><Slider value={glow} onChange={setGlow} suffix="%" /></Row>
        <Row label="Animation when idle">
          <div style={{ display: 'flex', gap: 8 }}>{['Still', 'Breathing', 'Drifting'].map((s, i) => <Chip key={s} on={anim === i} onClick={() => setAnim(i)}>{s}</Chip>)}</div>
        </Row>
        <Row label="Show in dashboard hero"><Toggle on={showHero} onChange={setShowHero} /></Row>
      </Section>
      <Section title="Density">
        <Row label="UI density"><div style={{ display: 'flex', gap: 8 }}>{['Comfortable', 'Default', 'Compact'].map((s, i) => <Chip key={s} on={density === i} onClick={() => setDensity(i)}>{s}</Chip>)}</div></Row>
        <Row label="Reduce motion" hint="Honors system preference by default."><Toggle on={reduceMotion} onChange={setReduceMotion} /></Row>
      </Section>
    </div>
  );
}

function ShortcutsPanel() {
  const groups = [
    { g: 'Global', items: [['Open command palette', ['⌘', 'K']], ['Quick task', ['⌘', 'N']], ['Toggle sidebar', ['⌘', 'B']], ['Search everything', ['⌘', '/']]] },
    { g: 'Navigation', items: [['Go to Home', ['G', 'H']], ['Go to Automate', ['G', 'W']], ['Go to Build', ['G', 'B']], ['Go to Brain', ['G', 'M']]] },
    { g: 'Build', items: [['Send message', ['⌘', '↵']], ['Insert tool', ['⌘', 'T']], ['Pause agent', ['⌘', 'P']], ['Take over', ['⌘', '.']]] },
  ];
  return (
    <div>
      <H1 sub="Click any shortcut to rebind. Reset to defaults at the bottom."><>Shortcuts<PreviewBadge /></></H1>
      {groups.map(grp => (
        <Section key={grp.g} title={grp.g}>
          {grp.items.map(([l, keys]) => (
            <div key={l as string} style={{ display: 'flex', alignItems: 'center', padding: '12px 0', borderTop: `1px solid ${color.border}` }}>
              <span style={{ fontSize: 13, flex: 1 }}>{l as string}</span>
              <div style={{ display: 'flex', gap: 4 }}>{(keys as string[]).map((k, i) => <Kbd key={i}>{k}</Kbd>)}</div>
            </div>
          ))}
        </Section>
      ))}
    </div>
  );
}

function DataPanel() {
  const [localFirst, setLocalFirst] = useState(true);
  const [e2e, setE2e] = useState(true);
  const [diagnostics, setDiagnostics] = useState(true);
  const [sharePrompts, setSharePrompts] = useState(false);
  return (
    <div>
      <H1 sub="Your data is yours. These controls govern what we keep and what we use."><>Data &amp; privacy<PreviewBadge /></></H1>
      <Section title="Local-first">
        <Row label="Keep everything on this device" hint="Memory and traces never leave your machine. Cloud sync turns off."><Toggle on={localFirst} onChange={setLocalFirst} /></Row>
        <Row label="End-to-end encryption" hint="Required when you have external collaborators."><Toggle on={e2e} onChange={setE2e} /></Row>
      </Section>
      <Section title="Telemetry">
        <Row label="Share anonymous diagnostics" hint="Crash reports and timing. Never your prompts."><Toggle on={diagnostics} onChange={setDiagnostics} /></Row>
        <Row label="Share prompts to improve models" hint="Off by default. Opt in at your own risk."><Toggle on={sharePrompts} onChange={setSharePrompts} /></Row>
      </Section>
      <Section title="Manage">
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
          <button style={ghost}>Export workspace</button>
          <button style={ghost}>Download memory</button>
          <button style={{ ...ghost, color: color.danger, borderColor: 'rgba(255,180,162,0.30)' }}>Delete workspace</button>
        </div>
      </Section>
    </div>
  );
}

// ── Panel router ─────────────────────────────────────────────────────

const PANELS: Record<string, () => JSX.Element> = {
  agent: PersonaPanel, profile: ProfilePanel, preferences: PreferencesPanel,
  memory: MemoryPanel, autonomy: AutonomyPanel, tools: ToolsPanel,
  models: ModelsPanel, keys: KeysPanel, appearance: AppearancePanel,
  shortcuts: ShortcutsPanel, data: DataPanel,
};

// ── Main Settings View ───────────────────────────────────────────────

export function SettingsView() {
  const setActivePanel = useCommandCenter(s => s.setActivePanel);
  const [section, setSection] = useState('agent');

  const dismiss = useCallback(() => setActivePanel('chat'), [setActivePanel]);
  useEffect(() => {
    const h = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); dismiss(); } };
    window.addEventListener('keydown', h);
    return () => window.removeEventListener('keydown', h);
  }, [dismiss]);

  const Panel = PANELS[section];

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', background: '#0B1220', color: color.text, fontFamily: font.body }}>
      <div style={{ width: 240, borderRight: `1px solid ${color.border}`, background: 'rgba(7,11,20,0.4)', padding: '24px 14px', overflow: 'auto', flexShrink: 0 }}>
        <div style={{ fontFamily: font.display, fontSize: 18, fontWeight: 700, letterSpacing: '-0.01em', padding: '0 10px 18px' }}>Settings</div>
        {CATEGORIES.map(cat => (
          <div key={cat.group} style={{ marginBottom: 16 }}>
            <div style={{ fontSize: 10, fontWeight: 600, letterSpacing: '0.10em', textTransform: 'uppercase', color: color.textDim, padding: '0 10px 6px' }}>{cat.group}</div>
            {cat.items.map(it => {
              const on = section === it.key;
              return (
                <button key={it.key} onClick={() => setSection(it.key)} style={{
                  display: 'flex', alignItems: 'center', gap: 10, width: '100%', padding: '8px 10px', borderRadius: 8,
                  background: on ? 'rgba(0,213,255,0.08)' : 'transparent',
                  border: on ? `1px solid ${color.borderHi}` : '1px solid transparent',
                  color: on ? color.cyan : color.textMuted, cursor: 'pointer', textAlign: 'left',
                  fontFamily: font.body, fontSize: 13, fontWeight: on ? 600 : 500,
                  transition: `all 140ms ${ease.out}`,
                }}>
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" strokeLinejoin="round"><path d={it.icon} /></svg>
                  {it.label}
                </button>
              );
            })}
          </div>
        ))}
      </div>
      <div style={{ flex: 1, overflow: 'auto', padding: '32px 40px 60px' }}>
        {Panel && <Panel />}
      </div>
    </div>
  );
}
