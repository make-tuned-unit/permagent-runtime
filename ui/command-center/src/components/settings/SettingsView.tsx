import { useState, useEffect, useCallback } from 'react';
import { useCommandCenter, navigateToTool } from '../../lib/store';
import { api, apiFetch, loadDaemonToken } from '../../lib/api';
import { font, ease, setTheme as setThemeFn, setMobiusGlow, setIdleAnim, setShowHeroMobius, setDensity as setDensityFn, setReduceMotion as setReduceMotionFn, type ThemeId, type IdleAnim, type UIDensity } from '../../styles/tokens';
import { useTheme as useThemeHook } from '../../styles/useTheme';
import { Mobius } from '../mobius/Mobius';
import { ProvidersSection } from './ProvidersSection';
import { SearchToolsSection } from './SearchToolsSection';
import { usePersona } from './useSettings';
import { VoicePicker } from '../voice/VoicePicker';
import { H1, Section, Row, TextInput, Chip, Toggle, Slider, Kbd, SaveButton } from './atoms';

function PreviewBadge() {
  const { colors } = useThemeHook();
  return (
    <span style={{
      fontSize: 10, fontWeight: 600, letterSpacing: '0.06em', textTransform: 'uppercase',
      padding: '3px 8px', borderRadius: 999, marginLeft: 10,
      background: colors.purpleSoft, color: colors.purpleBright,
      border: `1px solid ${colors.purple}40`,
    }}>preview</span>
  );
}

/** The 2026-07-10 settings audit rule: a control that does nothing must SAY
 *  so. Preview sections render this banner and their controls disabled until
 *  each is wired end to end. */
function PreviewNotice() {
  const { colors } = useThemeHook();
  return (
    <div style={{
      padding: '10px 14px', borderRadius: 10, margin: '4px 0 18px',
      background: colors.purpleSoft, border: `1px solid ${colors.purple}40`,
      color: colors.textMuted, fontSize: 12, lineHeight: 1.5,
    }}>
      This section is a design preview — these controls aren't wired up yet and
      changing them has no effect. They'll activate as each lands.
    </div>
  );
}

// ── Shared button styles (theme-aware via colors param) ─────────────
type C = ReturnType<typeof useThemeHook>['colors'];

const ghost = (colors: C): React.CSSProperties => ({
  height: 32, padding: '0 14px', borderRadius: 8,
  background: 'transparent', border: `1px solid ${colors.border}`,
  color: colors.text, cursor: 'pointer',
  fontFamily: font.body, fontSize: 12, fontWeight: 500,
  display: 'inline-flex', alignItems: 'center', gap: 6,
});
const selectStyle = (colors: C): React.CSSProperties => ({
  height: 34, padding: '0 12px', borderRadius: 8,
  background: colors.inputBg, border: `1px solid ${colors.border}`,
  color: colors.text, fontFamily: font.body, fontSize: 13,
  minWidth: 240, cursor: 'pointer',
});

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
    { key: 'devices',    label: 'Devices',          icon: 'M17 2H7a2 2 0 00-2 2v16a2 2 0 002 2h10a2 2 0 002-2V4a2 2 0 00-2-2zM12 18h.01' },
    { key: 'search',      label: 'Search & tools',   icon: 'M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z' },
  ]},
  { group: 'System', items: [
    { key: 'appearance',  label: 'Appearance',       icon: 'M12 3a9 9 0 100 18 9 9 0 000-18zM12 3v18M3 12h18' },
    { key: 'shortcuts',   label: 'Shortcuts',        icon: 'M4 6h16v12H4zM8 10h.01M12 10h.01M16 10h.01M7 14h10' },
    { key: 'data',        label: 'Data & privacy',   icon: 'M12 2l9 4v6c0 5-4 9-9 10-5-1-9-5-9-10V6l9-4zM9 12l2 2 4-4' },
  ]},
];

// ── Panels ───────────────────────────────────────────────────────────

// Panels may navigate between settings sections (e.g. Models → API keys).
type PanelProps = { goto: (key: string) => void };

function PersonaPanel() {
  const { colors } = useThemeHook();
  const { data, loading, saving, error, save } = usePersona();
  const [name, setName] = useState('');
  const [greeting, setGreeting] = useState('');
  const [tone, setTone] = useState('');
  const [traits, setTraits] = useState<string[]>([]);
  const [voiceId, setVoiceId] = useState<string | null>(null);
  const [newTrait, setNewTrait] = useState('');
  const [dirty, setDirty] = useState(false);
  const TRAIT_OPTIONS = ['curious', 'direct', 'patient', 'playful', 'formal', 'concise', 'thorough', 'opinionated'];

  useEffect(() => {
    if (data) { setName(data.first_name); setGreeting(data.opening_greeting); setTone(data.tone); setTraits(data.traits); setVoiceId(data.voice_id); setDirty(false); }
  }, [data]);

  const changeName = (v: string) => { setName(v); setDirty(true); };
  const changeGreeting = (v: string) => { setGreeting(v); setDirty(true); };
  const changeTone = (v: string) => { setTone(v); setDirty(true); };
  const changeVoice = (v: string | null) => { setVoiceId(v); setDirty(true); };
  const toggleTrait = (t: string) => { setTraits(p => p.includes(t) ? p.filter(x => x !== t) : [...p, t]); setDirty(true); };
  const addTrait = () => {
    const t = newTrait.trim();
    if (t && !traits.some(x => x.toLowerCase() === t.toLowerCase())) { setTraits(p => [...p, t]); setDirty(true); }
    setNewTrait('');
  };
  const handleSave = async () => {
    if (!dirty) return;
    await save({ first_name: name, opening_greeting: greeting, tone, traits, voice_id: voiceId });
    setDirty(false);
  };

  if (loading) return <div style={{ color: colors.textDim, fontSize: 13 }}>Loading persona...</div>;
  return (
    <div>
      <H1 sub="Shape how your agent thinks, talks, and decides. Changes take effect at the start of the next conversation.">Persona</H1>
      <Section title="Identity">
        <div style={{ display: 'flex', alignItems: 'center', gap: 24, marginBottom: 8 }}>
          <Mobius size={140} state="idle" glow={1} />
          <div style={{ flex: 1 }}>
            <Row label="Name" hint="What you'll call them."><TextInput value={name} onChange={changeName} /></Row>
            <Row label="Greeting" hint="The first line they'll say each session."><TextInput multi value={greeting} onChange={changeGreeting} /></Row>
          </div>
        </div>
      </Section>
      <Section title="Voice">
        <Row label="Voice" hint="The spoken voice used for voice replies and the greeting. Tap ▶ to audition.">
          <VoicePicker value={voiceId} onChange={changeVoice} />
        </Row>
      </Section>
      <Section title="Tone">
        <Row label="Tone" hint="How they describe their own speaking style (text, not audio)."><TextInput multi value={tone} onChange={changeTone} /></Row>
        <Row label="Traits" hint="Pick from suggestions or add your own. The agent will lean into these.">
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8 }}>
              {/* Selected traits (incl. custom ones not in the suggestion list) — click to remove */}
              {traits.map(t => <Chip key={t} on onClick={() => toggleTrait(t)}>{t}</Chip>)}
              {/* Unselected suggestions — click to add */}
              {TRAIT_OPTIONS.filter(t => !traits.includes(t)).map(t => <Chip key={t} on={false} onClick={() => toggleTrait(t)}>{t}</Chip>)}
            </div>
            <input
              value={newTrait}
              onChange={e => setNewTrait(e.target.value)}
              onKeyDown={e => { if (e.key === 'Enter' || e.key === ',') { e.preventDefault(); addTrait(); } }}
              onBlur={addTrait}
              placeholder="Add a custom trait — type and press Enter"
              style={{
                width: '100%', fontFamily: font.body, fontSize: 13, color: colors.text,
                background: colors.inputBg, border: `1px solid ${colors.border}`,
                borderRadius: 8, padding: '8px 12px', outline: 'none',
              }}
            />
          </div>
        </Row>
      </Section>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: 12 }}>
        {error && <span style={{ fontSize: 12, color: colors.danger }}>{error}</span>}
        <SaveButton onClick={handleSave} disabled={!dirty || saving} saving={saving} />
      </div>
    </div>
  );
}

function ProfilePanel() {
  const { colors } = useThemeHook();
  return (
    <div>
      <H1 sub="Your account — visible to your agent and to anyone you share a workspace with."><>Profile<PreviewBadge /></></H1>
      <PreviewNotice />
      <Section title="Account">
        <Row label="Display name"><TextInput value="Jesse Sharratt" /></Row>
        <Row label="Email" hint="Used for sign-in and notifications."><TextInput value="" placeholder="email@example.com" /></Row>
        <Row label="Workspace">
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <div style={{ width: 28, height: 28, borderRadius: 6, background: `linear-gradient(135deg, ${colors.purple}, ${colors.cyan})` }} />
            <TextInput value="Personal" />
          </div>
        </Row>
      </Section>
    </div>
  );
}

function PreferencesPanel() {
  const { colors } = useThemeHook();
  const [prefs, setPrefs] = useState([false, true, true, true]);
  return (
    <div>
      <H1 sub="Defaults that follow you across sessions. Changes saved locally."><>Preferences<PreviewBadge /></></H1>
      <PreviewNotice />
      <Section title="Defaults">
        <Row label="Open on launch" hint="Where Permagent lands when you open the app.">
          <select style={selectStyle(colors)}><option>Mission control</option><option>Last open task</option><option>Build</option><option>Brain</option></select>
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
        <Row label="When agent finishes a task"><select style={selectStyle(colors)}><option>Desktop notification</option><option>In-app only</option><option>Silent</option></select></Row>
        <Row label="When agent needs your input"><select style={selectStyle(colors)}><option>Desktop + sound</option><option>Desktop only</option><option>Silent</option></select></Row>
      </Section>
    </div>
  );
}

function MemoryPanel() {
  const { colors } = useThemeHook();
  const [maxMem, setMaxMem] = useState(1200);
  const [forget, setForget] = useState(45);
  const [rememberFlags, setRememberFlags] = useState([true, true, true, true, false]);
  const rememberItems = ['Names of people I\'ve introduced', 'Project goals and deadlines', 'Code style preferences from feedback', 'Things I dismissed or pushed back on', 'Sensitive context (medical, financial)'];
  return (
    <div>
      <H1 sub="What your agent remembers about you, your projects, and the people in your world."><>Memory<PreviewBadge /></></H1>
      <PreviewNotice />
      <Section title="Memory budget" sub="When the brain gets too full, the agent will compress or forget the lowest-signal items first.">
        <Row label="Max memory" hint="Soft cap. The agent prefers to keep things small."><Slider value={maxMem} onChange={setMaxMem} min={200} max={5000} suffix=" nodes" /></Row>
        <Row label="Forget threshold" hint="Items not touched in this many days become candidates for compression."><Slider value={forget} onChange={setForget} min={7} max={365} suffix="d" /></Row>
      </Section>
      <Section title="What to remember">
        {rememberItems.map((l, i) => (
          <div key={l} style={{ display: 'flex', alignItems: 'center', gap: 14, padding: '10px 0', borderTop: `1px solid ${colors.border}` }}>
            <Toggle on={rememberFlags[i]} onChange={v => setRememberFlags(p => { const n = [...p]; n[i] = v; return n; })} />
            <span style={{ fontSize: 13, flex: 1 }}>{l}</span>
          </div>
        ))}
      </Section>
      <Section title="Manage">
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
          <button style={ghost(colors)} onClick={() => navigateToTool('memory')}>Open Brain view</button>
          {/* Export/Forget removed (2026-07-10 audit): a destructive-styled
              button with no handler is worse than no button. They return
              with real endpoints behind them. */}
        </div>
      </Section>
    </div>
  );
}

function AutonomyPanel() {
  const { colors } = useThemeHook();
  // Trust level is REAL (2026-07-10 audit): it reads/writes the daemon's
  // GOOSE_MODE, which gates tool-call approval in the agent loop.
  const [trust, setTrust] = useState<string | null>(null);
  useEffect(() => {
    api.getConfig().then(cfg => {
      const mode = (cfg.config as Record<string, unknown>)?.GOOSE_MODE;
      setTrust(typeof mode === 'string' ? mode : 'smart_approve');
    }).catch(() => setTrust('smart_approve'));
  }, []);
  const saveTrust = (mode: string) => {
    setTrust(mode);
    api.upsertConfig('GOOSE_MODE', mode).catch(() => {});
  };
  const [confirms, setConfirms] = useState([true, true, true, true, false]);
  const [perSession, setPerSession] = useState(5);
  const [perDay, setPerDay] = useState(20);
  const trustLevels = [
    { v: 'approve', l: 'Tight', d: 'Ask before every tool call' },
    { v: 'smart_approve', l: 'Default', d: 'Ask only for sensitive tool calls' },
    { v: 'auto', l: 'Loose', d: 'Approve tool calls automatically' },
    { v: 'chat', l: 'Chat only', d: 'No tool calls at all' },
  ];
  const confirmItems = ['Sending email or messages', 'Spending money', 'Deleting files or records', 'Pushing to main / production', 'Reading sensitive memory'];
  return (
    <div>
      <H1 sub="How much your agent can do without checking in. Higher autonomy = faster, but more rope."><>Autonomy &amp; guardrails<PreviewBadge /></></H1>
      <Section title="Default autonomy" sub="Live — this writes the daemon's tool-approval mode (GOOSE_MODE) and applies to new turns.">
        <Row label="Trust level" hint="How tool calls are approved.">
          <div style={{ display: 'flex', gap: 6 }}>
            {trustLevels.map(opt => (
              <button key={opt.v} onClick={() => saveTrust(opt.v)} style={{
                padding: 12, borderRadius: 10, cursor: 'pointer',
                background: trust === opt.v ? colors.cyanSoft : colors.bgDeeper,
                border: trust === opt.v ? `1px solid ${colors.borderHi}` : `1px solid ${colors.border}`,
                color: colors.text, textAlign: 'left', flex: 1, fontFamily: font.body,
                opacity: trust === null ? 0.5 : 1,
              }}>
                <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 4, color: trust === opt.v ? colors.cyan : colors.text }}>{opt.l}</div>
                <div style={{ fontSize: 11, color: colors.textMuted }}>{opt.d}</div>
              </button>
            ))}
          </div>
        </Row>
      </Section>
      <PreviewNotice />
      <Section title="Always confirm before…">
        {confirmItems.map((l, i) => (
          <div key={l} style={{ display: 'flex', alignItems: 'center', gap: 14, padding: '10px 0', borderTop: `1px solid ${colors.border}` }}>
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
  const { colors } = useThemeHook();
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
        <span style={{ fontSize: 12, color: colors.textMuted }}>{enabledCount} of {extensions.length} enabled</span>
      </div>
      {loading ? (
        <div style={{ color: colors.textDim, fontSize: 13 }}>Loading extensions...</div>
      ) : extensions.length === 0 ? (
        <Section title="No extensions"><div style={{ color: colors.textMuted, fontSize: 13 }}>No MCP tools or extensions configured.</div></Section>
      ) : (
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(2, 1fr)', gap: 10 }}>
          {extensions.map(ext => (
            <div key={ext.name} style={{ display: 'flex', alignItems: 'center', gap: 14, padding: 14, borderRadius: 10, background: colors.bgDeeper, border: `1px solid ${colors.border}` }}>
              <div style={{ width: 32, height: 32, borderRadius: 8, background: ext.enabled ? 'rgba(0,213,255,0.08)' : 'rgba(255,255,255,0.04)', border: `1px solid ${ext.enabled ? colors.borderHi : colors.border}`, display: 'grid', placeItems: 'center', fontFamily: font.display, fontSize: 13, fontWeight: 700, color: ext.enabled ? colors.cyan : colors.textMuted }}>{ext.display_name[0]?.toUpperCase()}</div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13, fontWeight: 600 }}>{ext.display_name}</div>
                <div style={{ fontSize: 11, color: colors.textMuted, marginTop: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{ext.type}{ext.bundled ? ' · bundled' : ''} · {ext.available_tools.length} tools</div>
              </div>
              <div style={{ width: 8, height: 8, borderRadius: '50%', background: ext.enabled ? colors.success : colors.textDim, flexShrink: 0 }} />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

type OllamaModel = { name: string; size: number; digest: string; modified_at: string };
type OllamaRunning = { name: string; size: number; size_vram: number; digest: string; expires_at: string };
type OllamaStatus = { reachable: boolean; installed: OllamaModel[]; running: OllamaRunning[] };
type LibSchedule = { enabled: boolean; start_time: string; duration_minutes: number; model: string; run_if_launched_in_window: boolean; pruning_enabled?: boolean };

function formatBytes(b: number): string {
  if (b < 1e9) return `${(b / 1e6).toFixed(0)} MB`;
  return `${(b / 1e9).toFixed(1)} GB`;
}

function nextRunText(sched: LibSchedule): string {
  if (!sched.enabled) return 'Disabled';
  const [h, m] = sched.start_time.split(':').map(Number);
  const now = new Date();
  const next = new Date(now);
  next.setHours(h, m, 0, 0);
  if (next <= now) next.setDate(next.getDate() + 1);
  const diff = next.getTime() - now.getTime();
  const hrs = Math.floor(diff / 3600000);
  const mins = Math.floor((diff % 3600000) / 60000);
  const ampm = h >= 12 ? 'PM' : 'AM';
  const h12 = h === 0 ? 12 : h > 12 ? h - 12 : h;
  const mStr = String(m).padStart(2, '0');
  if (hrs < 1) return `Next run: in ${mins}m (${h12}:${mStr} ${ampm})`;
  return `Next run: in ${hrs}h ${mins}m (${h12}:${mStr} ${ampm})`;
}

function ModelStateBadge({ state }: { state: 'running' | 'installed' | 'missing' }) {
  const { colors } = useThemeHook();
  const styles: Record<string, { bg: string; text: string; label: string }> = {
    running: { bg: 'rgba(0,213,255,0.12)', text: colors.cyan, label: 'Loaded' },
    installed: { bg: 'rgba(255,255,255,0.06)', text: colors.textMuted, label: 'Installed' },
    missing: { bg: 'rgba(255,180,162,0.1)', text: colors.danger, label: 'Not installed' },
  };
  const s = styles[state];
  return (
    <span style={{ fontSize: 10, fontWeight: 600, padding: '2px 8px', borderRadius: 999, background: s.bg, color: s.text }}>
      {s.label}
    </span>
  );
}

function ModelsPanel({ goto }: PanelProps) {
  const { colors } = useThemeHook();
  const [ollama, setOllama] = useState<OllamaStatus | null>(null);
  const [schedule, setSchedule] = useState<LibSchedule | null>(null);
  const [saving, setSaving] = useState(false);
  const [runningNow, setRunningNow] = useState(false);

  // Poll Ollama status while panel is visible
  useEffect(() => {
    let active = true;
    const poll = () => {
      api.getOllamaStatus().then(s => { if (active) setOllama(s); }).catch(() => {});
      api.getLibrarianSchedule().then(s => { if (active) setSchedule(s); }).catch(() => {});
    };
    poll();
    const id = setInterval(poll, 8000);
    return () => { active = false; clearInterval(id); };
  }, []);

  const modelState = (name: string): 'running' | 'installed' | 'missing' => {
    if (!ollama) return 'missing';
    if (ollama.running.some(m => m.name === name || m.name.startsWith(name + ':'))) return 'running';
    if (ollama.installed.some(m => m.name === name || m.name.startsWith(name + ':'))) return 'installed';
    return 'missing';
  };

  const handleScheduleChange = async (patch: Partial<LibSchedule>) => {
    if (!schedule) return;
    const next = { ...schedule, ...patch };
    setSchedule(next);
    setSaving(true);
    try { await api.setLibrarianSchedule(next); } catch { /* */ }
    setSaving(false);
  };

  const handleRunNow = async () => {
    setRunningNow(true);
    try { await api.runLibrarianNow(); } catch { /* */ }
    setRunningNow(false);
    // Refresh status to show model as loaded
    api.getOllamaStatus().then(setOllama).catch(() => {});
  };

  return (
    <div>
      <H1 sub="Pick the brains behind the agent. Use stronger models when stakes are high; cheaper for routine work.">Models</H1>
      <Section title="Providers" sub="Provider credentials live in the API keys tab — add or update a key there, then route to it below.">
        <button style={ghost(colors)} onClick={() => goto('keys')}>Manage API keys</button>
      </Section>
      {/* The old Routing/Behavior selects were decorative — hardcoded options
          wired to nothing (2026-07-10 settings audit). The real model/default
          switch lives in the provider modal on the API keys tab. */}

      {/* ── Ollama Status ────────────────────────────────────────── */}
      <Section title="Local models (Ollama)">
        {!ollama ? (
          <Row label="Status" hint="Checking..."><span style={{ fontSize: 12, color: colors.textDim }}>Loading...</span></Row>
        ) : !ollama.reachable ? (
          <Row label="Status" hint="Ollama is not running. Install from ollama.com and run 'ollama serve'.">
            <span style={{ fontSize: 12, color: colors.danger }}>Ollama not running</span>
          </Row>
        ) : (
          <>
            <Row label="Connection" hint="Ollama at localhost:11434">
              <span style={{ fontSize: 12, color: colors.cyan }}>Connected</span>
            </Row>
            {ollama.installed.length === 0 ? (
              <Row label="Models" hint="No models installed. Run 'ollama pull qwen2.5:3b' to get started.">
                <span style={{ fontSize: 12, color: colors.textDim }}>None</span>
              </Row>
            ) : (
              ollama.installed.map(m => {
                const running = ollama.running.find(r => r.name === m.name);
                return (
                  <Row key={m.name} label={m.name} hint={formatBytes(m.size)}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                      <ModelStateBadge state={running ? 'running' : 'installed'} />
                      {running?.expires_at && (
                        <span style={{ fontSize: 10, color: colors.textDim }}>
                          unloads {new Date(running.expires_at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
                        </span>
                      )}
                    </div>
                  </Row>
                );
              })
            )}
          </>
        )}
      </Section>

      {/* ── Librarian Schedule ───────────────────────────────────── */}
      {schedule && (
        <Section title="Librarian schedule">
          <Row label="Enabled" hint="Run the Librarian on a daily schedule to describe memories.">
            <Toggle on={schedule.enabled} onChange={v => handleScheduleChange({ enabled: v })} />
          </Row>
          {schedule.enabled && (
            <>
              <Row label="Start time" hint="Daily start time (24h). The Librarian model will warm-load at this time.">
                <input
                  type="time"
                  value={schedule.start_time}
                  onChange={e => handleScheduleChange({ start_time: e.target.value })}
                  style={{ ...selectStyle(colors), minWidth: 120, width: 'auto' }}
                />
              </Row>
              <Row label="Duration" hint="How long to keep the model loaded (minutes).">
                <input
                  type="number"
                  min={15}
                  max={720}
                  value={schedule.duration_minutes}
                  onChange={e => handleScheduleChange({ duration_minutes: Math.max(15, Math.min(720, parseInt(e.target.value) || 15)) })}
                  style={{ ...selectStyle(colors), minWidth: 100, width: 'auto' }}
                />
                <span style={{ fontSize: 11, color: colors.textDim, marginLeft: 6 }}>min</span>
              </Row>
              <Row label="Model" hint="Ollama model used by the Librarian. Installed models only.">
                <span style={{ fontSize: 13, color: colors.text, display: 'flex', alignItems: 'center', gap: 8 }}>
                  <select
                    style={{ ...selectStyle(colors), width: 'auto', minWidth: 160 }}
                    value={schedule.model}
                    onChange={e => handleScheduleChange({ model: e.target.value })}
                  >
                    {!(ollama?.installed ?? []).some(m => m.name === schedule.model) && (
                      <option value={schedule.model}>{schedule.model} (not installed)</option>
                    )}
                    {(ollama?.installed ?? []).map(m => (
                      <option key={m.name} value={m.name}>{m.name}</option>
                    ))}
                  </select>
                  <ModelStateBadge state={modelState(schedule.model)} />
                </span>
              </Row>
              <Row label="Nightly pruning" hint="Let the Librarian retire stale, low-signal memories during its window.">
                <Toggle on={schedule.pruning_enabled ?? false} onChange={v => handleScheduleChange({ pruning_enabled: v })} />
              </Row>
              <Row label="Next run" hint={nextRunText(schedule)}>
                <span style={{ fontSize: 12, color: colors.textMuted }}>{nextRunText(schedule)}</span>
              </Row>
            </>
          )}
          <Row label="Run now" hint="Manually warm-load the model and trigger a Librarian run.">
            <button
              onClick={handleRunNow}
              disabled={runningNow || modelState(schedule.model) === 'missing'}
              style={{
                height: 30, padding: '0 14px', borderRadius: 6,
                background: runningNow ? 'rgba(0,213,255,0.08)' : 'rgba(0,213,255,0.12)',
                border: `1px solid ${colors.borderHi}`,
                color: runningNow ? colors.textDim : colors.cyan,
                fontSize: 12, fontWeight: 600, fontFamily: font.body,
                cursor: runningNow || modelState(schedule.model) === 'missing' ? 'not-allowed' : 'pointer',
                transition: `all 150ms ${ease.out}`,
              }}
            >
              {runningNow ? 'Warming...' : 'Run Librarian now'}
            </button>
          </Row>
          {saving && <div style={{ fontSize: 10, color: colors.textDim, textAlign: 'right', padding: '4px 0' }}>Saving...</div>}
        </Section>
      )}
    </div>
  );
}

function KeysPanel() {
  return (
    <div>
      <H1 sub="Bring your own keys for the providers you use. Add, replace, or remove a key here — keys are encrypted in your system keychain and never leave your device.">API keys</H1>
      <Section title="Providers">
        <ProvidersSection />
      </Section>
    </div>
  );
}

function SearchPanel() {
  return (
    <div>
      <H1 sub="Web search and other service tools. Add a key, and your agent can search the web. Keys are encrypted in your system keychain and never leave your device.">Search &amp; tools</H1>
      <Section title="Search providers">
        <SearchToolsSection />
      </Section>
    </div>
  );
}

function AppearancePanel() {
  const { colors } = useThemeHook();
  const prefs = useThemeHook();
  // Literal hex by design: these swatch gradients intentionally depict each theme's own palette, regardless of the active theme.
  const themes: Array<{ id: ThemeId; l: string; g: string }> = [
    { id: 'dark', l: 'Permagent dark', g: 'linear-gradient(135deg, #0B1220, #1E2433)' },
    { id: 'aurora', l: 'Aurora', g: 'linear-gradient(135deg, #0B1220 30%, #8D44AE)' },
    { id: 'silver', l: 'Silver', g: 'linear-gradient(135deg, #F8FAFC 0%, #D8DEE8 70%, #00BFEF 85%, #8B5CFF 100%)' },
  ];
  const animOptions: IdleAnim[] = ['still', 'breathing', 'drifting'];
  const animLabels = ['Still', 'Breathing', 'Drifting'];
  const densityOptions: UIDensity[] = ['comfortable', 'default', 'compact'];
  const densityLabels = ['Comfortable', 'Default', 'Compact'];
  return (
    <div>
      <H1 sub="How Permagent looks while it runs alongside you.">Appearance</H1>
      <Section title="Theme">
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 10 }}>
          {themes.map(th => {
            const on = prefs.theme === th.id;
            return (
              <div key={th.id} onClick={() => setThemeFn(th.id)} style={{
                padding: 4, borderRadius: 12, cursor: 'pointer',
                border: on ? `2px solid ${colors.cyan}` : '2px solid transparent',
                boxShadow: on ? `0 0 14px ${colors.cyanGlow}` : 'none',
              }}>
                <div style={{ height: 96, borderRadius: 8, background: th.g, border: `1px solid ${colors.border}` }} />
                <div style={{ fontSize: 12, padding: '8px 4px', textAlign: 'center', color: on ? colors.cyan : colors.text }}>{th.l}</div>
              </div>
            );
          })}
        </div>
      </Section>
      <Section title="Möbius">
        <Row label="Glow intensity"><Slider value={prefs.mobiusGlow} onChange={v => setMobiusGlow(v)} suffix="%" /></Row>
        <Row label="Animation when idle">
          <div style={{ display: 'flex', gap: 8 }}>{animOptions.map((a, i) => <Chip key={a} on={prefs.idleAnim === a} onClick={() => setIdleAnim(a)}>{animLabels[i]}</Chip>)}</div>
        </Row>
        <Row label="Show in dashboard hero"><Toggle on={prefs.showHeroMobius} onChange={v => setShowHeroMobius(v)} /></Row>
      </Section>
      <Section title="Density">
        <Row label="UI density"><div style={{ display: 'flex', gap: 8 }}>{densityOptions.map((d, i) => <Chip key={d} on={prefs.density === d} onClick={() => setDensityFn(d)}>{densityLabels[i]}</Chip>)}</div></Row>
        <Row label="Reduce motion" hint="Honors system preference by default."><Toggle on={prefs.reduceMotion} onChange={v => setReduceMotionFn(v)} /></Row>
      </Section>
    </div>
  );
}

function ShortcutsPanel() {
  const { colors } = useThemeHook();
  const groups = [
    { g: 'Global', items: [['Open command palette', ['⌘', 'K']], ['Quick task', ['⌘', 'N']], ['Toggle sidebar', ['⌘', 'B']], ['Search everything', ['⌘', '/']]] },
    { g: 'Navigation', items: [['Go to Home', ['G', 'H']], ['Go to Automate', ['G', 'W']], ['Go to Build', ['G', 'B']], ['Go to Brain', ['G', 'M']]] },
    { g: 'Build', items: [['Send message', ['⌘', '↵']], ['Insert tool', ['⌘', 'T']], ['Pause agent', ['⌘', 'P']], ['Take over', ['⌘', '.']]] },
  ];
  return (
    <div>
      <H1 sub="The current keyboard map. Rebinding is coming — these are reference-only today."><>Shortcuts<PreviewBadge /></></H1>
      {groups.map(grp => (
        <Section key={grp.g} title={grp.g}>
          {grp.items.map(([l, keys]) => (
            <div key={l as string} style={{ display: 'flex', alignItems: 'center', padding: '12px 0', borderTop: `1px solid ${colors.border}` }}>
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
      <H1 sub="Your data is yours. Everything is local-first today; these controls activate as remote features land."><>Data &amp; privacy<PreviewBadge /></></H1>
      <PreviewNotice />
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
          {/* Export/Download/Delete removed (2026-07-10 audit): action-styled
              buttons with no handlers. They return with real endpoints. */}
        </div>
      </Section>
    </div>
  );
}

// ── Panel router ─────────────────────────────────────────────────────

const PANELS: Record<string, (props: PanelProps) => JSX.Element> = {
  agent: PersonaPanel, profile: ProfilePanel, preferences: PreferencesPanel,
  memory: MemoryPanel, autonomy: AutonomyPanel, tools: ToolsPanel,
  models: ModelsPanel, keys: KeysPanel, devices: DevicesPanel, search: SearchPanel,
  appearance: AppearancePanel, shortcuts: ShortcutsPanel, data: DataPanel,
};

/** Devices — hub-and-spoke pairing (MULTI_DEVICE.md). The hub (this machine)
 *  holds the one Brain; every other device connects to it over the tailnet by
 *  opening the pairing URL once (the token rides the #fragment and is
 *  captured into that device's localStorage — see api.ts browserToken). */
function DevicesPanel() {
  const { colors } = useThemeHook();
  const [token, setToken] = useState<string | null>(null);
  const [host, setHost] = useState('your-mac.tailnet-name.ts.net');
  const [copied, setCopied] = useState(false);
  const [tailnet, setTailnet] = useState<{ installed: boolean; running: boolean; magic_dns_name: string | null } | null>(null);
  useEffect(() => { loadDaemonToken().then(setToken); }, []);
  // Deterministic detection: when the hub is on a tailnet, the address fills
  // itself — the user types nothing (Jesse's zero-strain rule, 2026-07-11).
  useEffect(() => {
    apiFetch<{ installed: boolean; running: boolean; magic_dns_name: string | null }>('/api/tailnet/status')
      .then(t => {
        setTailnet(t);
        if (t.magic_dns_name) setHost(t.magic_dns_name);
      })
      .catch(() => setTailnet(null));
  }, []);
  const pairingUrl = token
    ? `http://${host}:3001/ui/#token=${token}`
    : null;
  return (
    <div>
      <H1 sub="One Brain, one truth: this machine is the hub — every other device connects to it. No accounts, no sync conflicts; pairing is this URL, opened once per device.">Devices</H1>
      <Section title="Pair a device" sub="Live — requires the daemon bound to your tailnet (HOST=0.0.0.0 or your Tailscale IP in the daemon environment) and Tailscale on both devices.">
        <Row label="Tailnet" hint={tailnet?.running ? 'Detected — address filled in automatically.' : tailnet?.installed ? 'Tailscale is installed but not connected.' : 'Tailscale not detected on this machine.'}>
          {tailnet?.running ? (
            <span style={{ fontSize: 12, color: colors.cyan }}>● Connected{tailnet.magic_dns_name ? ` — ${tailnet.magic_dns_name}` : ''}</span>
          ) : (
            <button
              style={ghost(colors)}
              title="Copies a setup request and opens chat — Henry runs the terminal steps for you."
              onClick={() => {
                navigator.clipboard.writeText(
                  'Set up Tailscale on this machine so my other devices can reach Permagent: '
                  + 'check if it is installed, install it if not, bring it up (open the login '
                  + 'page for me in the browser when it appears), then tell me my MagicDNS name '
                  + 'and confirm the Devices pairing page shows it.'
                ).catch(() => {});
                navigateToTool('chat');
              }}
            >Have Henry set it up</button>
          )}
        </Row>
        <Row label="Hub address" hint="Your machine's Tailscale MagicDNS name (auto-filled when the tailnet is detected).">
          <TextInput value={host} onChange={setHost} placeholder="my-mac.tailnet-name.ts.net" />
        </Row>
        <Row label="Pairing URL" hint="Open this on the new device's browser. The token is captured on first load and scrubbed from the URL.">
          {pairingUrl ? (
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0 }}>
              <code style={{
                fontFamily: font.mono, fontSize: 10, color: colors.cyan,
                background: colors.bgDeeper, padding: '6px 8px', borderRadius: 6,
                overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', maxWidth: 320,
              }}>{pairingUrl}</code>
              <button
                style={ghost(colors)}
                onClick={() => {
                  navigator.clipboard.writeText(pairingUrl).then(() => {
                    setCopied(true);
                    setTimeout(() => setCopied(false), 1600);
                  });
                }}
              >{copied ? 'Copied ✓' : 'Copy'}</button>
            </div>
          ) : (
            <span style={{ fontSize: 12, color: colors.textDim }}>
              Token unavailable — pairing works from the desktop app on the hub machine.
            </span>
          )}
        </Row>
        <Row label="Security" hint="The URL contains this daemon's bearer token — share it only through your own devices. Tailscale encrypts the transport; the token is the pairing secret.">
          <span style={{ fontSize: 12, color: colors.textMuted }}>Treat the pairing URL like a password.</span>
        </Row>
      </Section>
    </div>
  );
}

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
  const { gradient, colors } = useThemeHook();

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', background: gradient.shell, color: colors.text, fontFamily: font.body }}>
      <div style={{ width: 240, borderRight: `1px solid ${colors.border}`, background: gradient.navRail, padding: '24px 14px', overflow: 'auto', flexShrink: 0 }}>
        <div style={{ fontFamily: font.display, fontSize: 18, fontWeight: 700, letterSpacing: '-0.01em', padding: '0 10px 18px' }}>Settings</div>
        {CATEGORIES.map(cat => (
          <div key={cat.group} style={{ marginBottom: 16 }}>
            <div style={{ fontSize: 10, fontWeight: 600, letterSpacing: '0.10em', textTransform: 'uppercase', color: colors.textDim, padding: '0 10px 6px' }}>{cat.group}</div>
            {cat.items.map(it => {
              const on = section === it.key;
              return (
                <button key={it.key} onClick={() => setSection(it.key)} style={{
                  display: 'flex', alignItems: 'center', gap: 10, width: '100%', padding: '8px 10px', borderRadius: 8,
                  background: on ? 'rgba(0,213,255,0.08)' : 'transparent',
                  border: on ? `1px solid ${colors.borderHi}` : '1px solid transparent',
                  color: on ? colors.cyan : colors.textMuted, cursor: 'pointer', textAlign: 'left',
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
        {Panel && <Panel goto={setSection} />}
      </div>
    </div>
  );
}
