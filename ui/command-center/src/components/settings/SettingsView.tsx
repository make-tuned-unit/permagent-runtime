import { useState, useEffect, useCallback, useRef, type CSSProperties } from 'react';
import {
  FiActivity, FiClock, FiCommand, FiCpu, FiDatabase, FiDollarSign, FiEdit3,
  FiEyeOff, FiInbox, FiKey, FiList, FiLock, FiSearch, FiServer, FiShield,
  FiSliders, FiSmartphone, FiSun, FiUser, FiUsers,
} from 'react-icons/fi';
import { useCommandCenter, navigateToTool } from '../../lib/store';
import { emitActivity } from '../../lib/emitActivity';
import { api, apiFetch, type SovereigntyStatus, type EgressLogEntry, type DeviceInfo, type CrashExportResponse, type IncidentView } from '../../lib/api';
import { relativeTimeAgo } from '../../lib/time-decay';
import { font, radius, setDensity as setDensityFn, setIdleAnim, setMobiusGlow, setReduceMotion as setReduceMotionFn, setShowHeroMobius, setTheme as setThemeFn, type IdleAnim, type ThemePref, type UIDensity, textSize } from '../../styles/tokens';
import { useTheme as useThemeHook } from '../../styles/useTheme';
import { Mobius } from '../mobius/Mobius';
import {
  getNotificationPrefs, setNotificationPref, getOsNotificationsEnabled,
  setOsNotificationsEnabled, KIND_LABELS, useNotifications, type NotificationKind,
} from '../../lib/notifications';
import { ProvidersSection } from './ProvidersSection';
import { DevRootsSection } from './DevRootsSection';
import { SearchToolsSection } from './SearchToolsSection';
import { DataSourcesSection } from './DataSourcesSection';
import { PolybotKeys } from '../finance/PolybotKeys';
import { FundamentalsKey } from '../finance/FundamentalsKey';
import { usePersona } from './useSettings';
import { resolveSettingsSection } from './sections';
import { trustEnvOverrideNotice } from './autonomy';
import { VoicePicker } from '../voice/VoicePicker';
import { PronunciationSection } from '../voice/PronunciationSection';
import { H1, Section, Row, TextInput, Chip, Slider, Kbd, SaveButton, ModelStateBadge, selectStyle } from './atoms';
import { Button } from '../common/Button';
import { StateBlock } from '../common/StateBlock';
import { Toggle } from '../common/Toggle';
import { makeQrMatrix } from '../../lib/qrMatrix';
import { SessionsList } from '../sessions/SessionsList';
import { InboxPanel } from '../inbox/InboxPanel';
import { ExecutionTrace } from '../trace/ExecutionTrace';
import { SpendPanel } from './SpendPanel';
import { AgentsPanel } from './agents/AgentsPanel';
import { FeaturesPanel } from './features/FeaturesPanel';
import { timeAgo } from './format';
import { useDecisions } from '../dashboard/decisions/useDecisions';
import { DecisionInbox } from '../dashboard/decisions/DecisionInbox';
import { summarizeDecisions } from '../dashboard/decisions/summary';
import { getOpenOnLaunch, setOpenOnLaunch, OPEN_ON_LAUNCH_OPTIONS, type OpenOnLaunch } from '../../lib/openOnLaunch';
import { RoleRoutingPrompt } from '../chat/RoleRoutingPrompt';

// The PreviewBadge/PreviewNotice machinery (2026-07-10 audit) is gone: every
// preview-only control has been either wired to real state or removed
// (2026-08 finish-the-settings ruling). Controls that exist here do things.

// ── Shared button styles (theme-aware via colors param) ─────────────
type C = ReturnType<typeof useThemeHook>['colors'];

/**
 * The house ghost button, now expressed as the primitive's custom properties
 * rather than as a finished inline `style` object.
 *
 * Same resting look — 32px tall, hairline border, 12px body type. What it could
 * not have before is the half of a button that only CSS can say: an inline
 * declaration cannot express `:hover` or `:active`, so every one of these ~15
 * controls looked identical pressed and unpressed. Setting the colours through
 * `--pa-btn-*` is not a style preference: an inline `color`/`background` beats
 * the `.pa-btn:hover` rule in the cascade and would silently kill it again.
 */
const ghost = (colors: C): React.CSSProperties => ({
  '--pa-btn-bg': 'transparent',
  '--pa-btn-fg': colors.text,
  '--pa-btn-border': colors.border,
  '--pa-btn-bg-hover': colors.surfaceHi,
  '--pa-btn-border-hover': colors.borderHi,
  '--pa-btn-bg-active': colors.surface,
  '--pa-btn-pad': '0 14px',
  '--pa-btn-radius': `${radius.md}px`,
  '--pa-btn-weight': 500,
  height: 32,
  fontFamily: font.body, fontSize: textSize.caption, gap: 6,
} as React.CSSProperties);

// ── Voice model route readout ────────────────────────────────────────
// Mirrors crates/goose/src/config/voice_model.rs::resolve_voice_model — this
// is a DISPLAY-ONLY mirror of that precedence (disabled > configured >
// half-configured/default), not the source of truth. The daemon resolves the
// real route from config.yaml; this just tells the operator what to expect.
const VOICE_DISABLE_VALUES = new Set(['session', 'off', 'none']);
const DEFAULT_VOICE_PROVIDER_ID = 'custom_deepseek';
const DEFAULT_VOICE_MODEL_ID = 'deepseek-chat';

function describeVoiceRoute(provider: string | null, model: string | null): string {
  const providerVal = (provider ?? '').trim();
  const modelVal = (model ?? '').trim();
  const isDisabled = (v: string) => VOICE_DISABLE_VALUES.has(v.toLowerCase());
  if ((providerVal && isDisabled(providerVal)) || (modelVal && isDisabled(modelVal))) {
    return 'session model';
  }
  if (providerVal && modelVal) {
    return `${providerVal} / ${modelVal}`;
  }
  return `${DEFAULT_VOICE_PROVIDER_ID} / ${DEFAULT_VOICE_MODEL_ID} (default)`;
}

// ── Nav rail categories ──────────────────────────────────────────────
//
// Feather components, not path data. This table used to hold twenty hand-drawn
// `d` strings — a third icon strategy alongside `react-icons/fi` and the
// sidebar's ratified set, with no reason on record for being hand-drawn. The
// design-system ruling (U2 §3.4) allows one library and one named local set,
// and this was neither. Four glyphs have no honest Feather twin and changed
// what they depict rather than what they mean: Memory was a brain (Feather has
// none) and is now a store; Models was a second pulse line, which would have
// been the SAME glyph as Activity; Appearance was a contrast disc; Data &
// privacy was a shield-with-tick, and the plain shield belongs to Autonomy.
// `Tools & MCPs` keeps its pencil (Feather `edit-3`) — the drawing has always
// disagreed with the label, and correcting that is a design call, not a
// migration's.

const CATEGORIES = [
  { group: 'You', items: [
    { key: 'preferences', label: 'Preferences',      icon: FiSliders },
  ]},
  { group: 'Agent', items: [
    { key: 'agent',       label: 'Persona',          icon: FiUser },
    { key: 'agents',      label: 'Agents',           icon: FiUsers },
    { key: 'memory',      label: 'Memory',           icon: FiDatabase },
    { key: 'autonomy',    label: 'Autonomy & guardrails', icon: FiShield },
  ]},
  // The former Console overlay (Sessions / Inbox / Trace / Governance) folded
  // into Settings — 2026-08 ruling. Governance's panels merged into Spend,
  // Sovereignty, Models, and Autonomy.
  // "Console" named the retired overlay, not what the group holds: Sessions,
  // Downloads, Activity and Spend are all records of what already happened.
  { group: 'History', items: [
    { key: 'sessions',    label: 'Sessions',         icon: FiClock },
    // "Inbox" is the decision queue's word everywhere else in the app; this
    // pane is where in-app browser downloads land. Say which it is.
    { key: 'inbox',       label: 'Downloads',        icon: FiInbox },
    { key: 'activity',    label: 'Activity',         icon: FiActivity },
    { key: 'spend',       label: 'Spend',            icon: FiDollarSign },
  ]},
  { group: 'Connections', items: [
    // MCP is defined once, in the pane's own subtitle — a nav label is not the
    // place to teach an acronym.
    { key: 'tools',       label: 'Tools',            icon: FiEdit3 },
    { key: 'models',      label: 'Models',           icon: FiCpu },
    { key: 'keys',        label: 'API keys',         icon: FiKey },
    { key: 'devices',    label: 'Devices',          icon: FiSmartphone },
    { key: 'search',      label: 'Search & tools',   icon: FiSearch },
    { key: 'sources',     label: 'Data sources',     icon: FiServer },
  ]},
  { group: 'System', items: [
    { key: 'appearance',  label: 'Appearance',       icon: FiSun },
    { key: 'shortcuts',   label: 'Shortcuts',        icon: FiCommand },
    { key: 'data',        label: 'Data & privacy',   icon: FiEyeOff },
    { key: 'sovereignty', label: 'Sovereignty',      icon: FiLock },
    { key: 'features',    label: 'Features',         icon: FiList },
  ]},
];

// ── Panels ───────────────────────────────────────────────────────────

// Panels may navigate between settings sections (e.g. Models → API keys).
type PanelProps = { goto: (key: string) => void };

function PersonaPanel() {
  const { colors } = useThemeHook();
  const { data, loading, saving, error, save, reload } = usePersona();
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
    // Only clear dirty when the daemon actually persisted the edits (#167 —
    // clearing unconditionally made a failed save look successful).
    const ok = await save({ first_name: name, opening_greeting: greeting, tone, traits, voice_id: voiceId });
    if (ok) setDirty(false);
  };

  if (loading) return <div style={{ color: colors.textDim, fontSize: textSize.small }}>Loading persona...</div>;
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
        <PronunciationSection />
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
                width: '100%', fontFamily: font.body, fontSize: textSize.small, color: colors.text,
                background: colors.inputBg, border: `1px solid ${colors.border}`,
                borderRadius: radius.md, padding: '8px 12px', outline: 'none',
              }}
            />
          </div>
        </Row>
      </Section>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: 12 }}>
        {error && <span style={{ fontSize: textSize.caption, color: colors.danger }}>{error}</span>}
        {/* Failed initial load: offer a retry instead of leaving a form that
            reads as broken (#167). Saving still works from the form's values. */}
        {error === 'Failed to load persona' && !data && (
          <Button
            colors={colors}
            // `reload` swallows its own failure into the form's `error` line and
            // resolves either way, so the promise is deliberately not handed
            // back: the pane's own loading state says it is retrying, and a tick
            // here would claim a load that may have failed again.
            onClick={() => { void reload(); }}
            style={{
              '--pa-btn-fg': colors.textMuted,
              '--pa-btn-fg-hover': colors.text,
              '--pa-btn-border': colors.border,
              '--pa-btn-border-hover': colors.borderHi,
              '--pa-btn-pad': '6px 14px',
              '--pa-btn-radius': `${radius.md}px`,
              fontSize: textSize.caption, fontFamily: font.body,
            } as CSSProperties}
          >
            Retry
          </Button>
        )}
        <SaveButton onClick={handleSave} disabled={!dirty || saving} saving={saving} />
      </div>
    </div>
  );
}

// The Profile panel (hardcoded name/email/workspace mockup) was removed in the
// 2026-08 finish-the-settings ruling: nothing read any of it and no account
// subsystem exists. It returns when there is a real account to show.

export function PreferencesPanel() {
  const { colors } = useThemeHook();
  // "Open on launch" is LIVE: persisted locally and consumed once by App.tsx
  // after workspaces load. The old preview list ("agent should…" toggles) was
  // removed — two duplicated GOOSE_MODE semantics, two had no backing key.
  const [launch, setLaunch] = useState<OpenOnLaunch>(() => getOpenOnLaunch());
  return (
    <div>
      <H1 sub="Defaults that follow you across sessions. Changes saved locally.">Preferences</H1>
      <Section title="Defaults">
        <Row label="Open on launch" hint="Where Permagent lands when you open the app. Applied on the next launch.">
          <select
            style={selectStyle(colors)}
            value={launch}
            onChange={e => {
              const v = e.target.value as OpenOnLaunch;
              setOpenOnLaunch(v);
              setLaunch(v);
            }}
          >
            {OPEN_ON_LAUNCH_OPTIONS.map(o => (
              <option key={o.value} value={o.value}>{o.label}</option>
            ))}
          </select>
        </Row>
      </Section>
      <Section title="Your code" sub="Where you keep your repositories on this machine. Asked during setup; changeable here at any time.">
        <DevRootsSection />
      </Section>
      <NotificationSettings />
    </div>
  );
}

/** #618 — LIVE notification preferences: per-kind toggles feed the tray/toast
 *  stream directly (localStorage, consumed in lib/notifications.ts), plus the
 *  OS-level opt-in which requests real Notification permission. This replaced
 *  the dead mockup selects the 2026-07-10 audit flagged. */
function NotificationSettings() {
  const [prefs, setPrefs] = useState(getNotificationPrefs());
  const [osOn, setOsOn] = useState(getOsNotificationsEnabled());
  const [osDenied, setOsDenied] = useState(false);
  const kinds = Object.keys(KIND_LABELS) as NotificationKind[];
  return (
    <Section title="Notifications" sub="Live — the agent reaches out when something needs you. Each toggle silences its kind everywhere (tray, toasts, system).">
      {kinds.map(k => (
        <Row key={k} label={KIND_LABELS[k]}>
          <Toggle on={prefs[k]} onChange={v => {
            setNotificationPref(k, v);
            setPrefs(getNotificationPrefs());
          }} />
        </Row>
      ))}
      <Row label="System notifications" hint="Also notify at the OS level (asks for permission).">
        {/* The OS can refuse, and a refusal used to be indistinguishable from a
            grant: the switch showed whatever `setOsNotificationsEnabled`
            returned with nothing said about the difference. */}
        <Toggle
          on={osOn}
          error={osDenied ? 'The system refused notification permission — allow Permagent under System Settings → Notifications.' : null}
          onChange={async v => {
            const actual = await setOsNotificationsEnabled(v);
            setOsOn(actual);
            setOsDenied(v && !actual);
          }}
        />
      </Row>
    </Section>
  );
}

export function MemoryPanel({ goto: _goto }: { goto?: (key: string) => void }) {
  const { colors } = useThemeHook();
  // Lands ON the Librarian's page rather than near it (R6): the pruning
  // setting is one scroll into one agent, not somewhere on the Agents list.
  const openAgentSettings = useCommandCenter(s => s.openAgentSettings);
  // The preview "memory budget" sliders and "what to remember" toggles were
  // removed (2026-08 finish-the-settings ruling): no backing subsystem reads
  // them. What remains is real: the Brain view, and the Librarian's nightly
  // pruning setting (on the Librarian's own page), the live retention control.
  return (
    <div>
      <H1 sub="What your agent remembers about you, your projects, and the people in your world.">Memory</H1>
      <Section title="Manage">
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
          <Button colors={colors} style={ghost(colors)} onClick={() => navigateToTool('memory')}>Open Brain view</Button>
          <Button colors={colors} style={ghost(colors)} data-testid="memory-open-librarian" onClick={() => openAgentSettings('librarian')}>Nightly pruning (the Librarian's schedule) →</Button>
          {/* Export/Forget removed (2026-07-10 audit): a destructive-styled
              button with no handler is worse than no button. They return
              with real endpoints behind them. */}
        </div>
        <div style={{ fontSize: textSize.caption, color: colors.textMuted, marginTop: 12, lineHeight: 1.5 }}>
          Browse and audit everything remembered in the Brain view. Retirement
          of stale, low-signal memories is handled by the Librarian's nightly
          pruning — configure it under Models.
        </div>
      </Section>
    </div>
  );
}

// Only `auto` and `chat` are selectable here today. Per-tool confirmations
// now route to the Decision Inbox (#760) and parked turns are answerable
// there — but NEW selection of `approve`/`smart_approve` stays blocked until
// the trust-chain re-enable gate (eviction ordering, sub-session mode
// inheritance, effective-mode visibility) fully lands. Still surface them if
// a user is already there (e.g. via env or old YAML), so they can switch back.
const SELECTABLE_TRUST_MODES = new Set(['auto', 'chat']);

/**
 * Compact pending-approvals strip (Settings → Autonomy) — a labelled REFERENCE
 * to the canonical rendering, which is Home's decisions card (J3).
 *
 * It used to build its own sentence ("Pending approvals: 3") and open its own
 * copy of the inbox overlay. Two different sentences about one number is how a
 * user comes to suspect there are two queues, so the words now come from the
 * shared `summarizeDecisions` — the same ones Home says — and the action hands
 * the user to Home rather than rendering the board a second time two clicks
 * deep in Settings. This is the "Open X →" convention Settings already uses
 * four times.
 *
 * The overlay stays as the fallback for the one case where the reference cannot
 * be honoured: no workspace holds Home, so there is nowhere to send anyone. A
 * control that looks like it worked and did nothing is the worse failure.
 */
export function ApprovalsStrip() {
  const { colors } = useThemeHook();
  const inbox = useDecisions();
  const { data } = inbox;
  const { data: persona } = usePersona();
  const agentName = persona?.display_name ?? 'your agent';
  const openInbox = useCommandCenter(s => s.openDecisionInbox);
  const [open, setOpen] = useState(false);
  const s = summarizeDecisions(data, agentName);
  return (
    <>
      <div style={{
        display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap',
        padding: '9px 12px', marginBottom: 12, borderRadius: radius.md,
        background: colors.bgDeeper, border: `1px solid ${colors.border}`,
      }}>
        <span style={{ fontSize: textSize.caption, color: s.count > 0 ? colors.text : colors.textMuted }}>
          {s.loading ? s.headline : s.allClear ? s.allClearLabel : s.headline}
          {s.oldestLabel && (
            <span style={{ color: colors.textDim }}> · {s.oldestLabel}</span>
          )}
        </span>
        <div style={{ flex: 1 }} />
        <Button
          colors={colors}
          style={ghost(colors)}
          onClick={() => { if (!openInbox()) setOpen(true); }}
        >
          Open Decisions on Home →
        </Button>
      </div>
      {open && <DecisionInbox inbox={inbox} onClose={() => setOpen(false)} />}
    </>
  );
}

export function AutonomyPanel({ goto }: { goto?: (key: string) => void }) {
  const { colors } = useThemeHook();
  // Trust level is REAL (2026-07-10 audit): it reads/writes the daemon's
  // GOOSE_MODE, which gates tool-call approval in the agent loop.
  const [trust, setTrust] = useState<string | null>(null);
  // What the daemon ACTUALLY runs (env var overrides YAML). Diverges from
  // `trust` when GOOSE_MODE is set in the daemon's environment — the buttons
  // below write YAML, which the env silently wins over.
  const [effectiveTrust, setEffectiveTrust] = useState<string | null>(null);
  const [trustError, setTrustError] = useState<string | null>(null);
  // `configRev` is the /events subscription: the daemon emits `config_changed`
  // from the shared `Config` writer, so a GOOSE_MODE change made by the agent,
  // the CLI, or another device re-reads here instead of leaving this pane
  // showing a mode that is no longer in force.
  const configRev = useCommandCenter(s => s.configRev);
  useEffect(() => {
    api.getConfig().then(cfg => {
      const mode = (cfg.config as Record<string, unknown>)?.GOOSE_MODE;
      // Daemon default is GooseMode::Auto (crates/goose/src/config/goose_mode.rs) —
      // when GOOSE_MODE is unset the agent runs in `auto`, so reflect that here
      // instead of `smart_approve` (which is a hanging mode and no longer newly
      // selectable). Showing `smart_approve` as "active" used to lure users into
      // clicking it and hanging their turn.
      setTrust(typeof mode === 'string' ? mode : 'auto');
      const eff = cfg.effective_goose_mode;
      setEffectiveTrust(typeof eff === 'string' && eff !== '' ? eff : null);
    }).catch(() => setTrust('auto'));
  }, [configRev]);
  const saveTrust = (mode: string) => {
    // Defense in depth: the hanging modes are also disabled in the UI below, but
    // never write one from a fresh selection even if a click slips through.
    if (!SELECTABLE_TRUST_MODES.has(mode)) return;
    const prev = trust;
    setTrust(mode);
    setTrustError(null);
    // Revert + surface on failure (2026-07 wiring audit): the old swallowed
    // catch left the UI showing a trust level the daemon never accepted.
    api.upsertConfig('GOOSE_MODE', mode).catch(err => {
      setTrust(prev);
      setTrustError(`Couldn't save trust level: ${err instanceof Error ? err.message : String(err)}`);
    });
  };
  const trustLevels = [
    { v: 'auto', l: 'Automatic', d: 'Run tool calls without asking (default)' },
    { v: 'chat', l: 'Chat only', d: 'No tool calls at all' },
    { v: 'approve', l: 'Ask every time', d: 'Confirm before every tool call' },
    { v: 'smart_approve', l: 'Smart approve', d: 'Confirm only sensitive calls' },
  ];
  return (
    <div>
      <H1 sub="How much your agent can do without checking in. Higher autonomy = faster, but more rope.">Autonomy &amp; guardrails</H1>
      <Section title="Default autonomy" sub="Live — this writes the daemon's tool-approval mode and applies to new turns.">
        {trustError && (
          <div style={{ fontSize: textSize.caption, color: colors.danger, padding: '4px 0 8px' }}>{trustError}</div>
        )}
        <ApprovalsStrip />
        {(() => {
          // Env-override honesty (re-enable-gate epic part B): with GOOSE_MODE
          // set in the daemon's environment, these buttons write YAML the env
          // silently wins over. Say so instead of highlighting a mode the
          // daemon isn't running.
          const envNotice = trustEnvOverrideNotice(effectiveTrust, trust);
          return envNotice ? (
            <div style={{ marginBottom: 10, padding: '10px 14px', borderRadius: 10, background: `${colors.warning}1A`, border: `1px solid ${colors.warning}55`, color: colors.text, fontSize: textSize.caption, lineHeight: 1.5 }}>
              {envNotice}
            </div>
          ) : null;
        })()}
        <Row label="Trust level" hint="How tool calls are approved.">
          <div style={{ display: 'flex', gap: 6 }}>
            {trustLevels.map(opt => {
              const current = trust === opt.v;
              const locked = !SELECTABLE_TRUST_MODES.has(opt.v);
              return (
                <button key={opt.v} disabled={locked} onClick={() => saveTrust(opt.v)}
                  title={locked ? 'Locked while the approval pipeline is hardened — approval prompts route to the Decision Inbox' : undefined}
                  style={{
                    padding: 12, borderRadius: 10, cursor: locked ? 'not-allowed' : 'pointer',
                    background: current ? colors.cyanSoft : colors.bgDeeper,
                    border: current ? `1px solid ${colors.borderHi}` : `1px solid ${colors.border}`,
                    color: colors.text, textAlign: 'left', flex: 1, fontFamily: font.body,
                    opacity: trust === null ? 0.5 : locked && !current ? 0.55 : 1,
                  }}>
                  <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 6, marginBottom: 4 }}>
                    <span style={{ fontSize: textSize.small, fontWeight: 600, color: current ? colors.cyan : colors.text }}>{opt.l}</span>
                    {locked && (
                      <span style={{ fontSize: 10, fontWeight: 600, letterSpacing: '0.05em', textTransform: 'uppercase', color: colors.textMuted, border: `1px solid ${colors.border}`, borderRadius: radius.pill, padding: '1px 6px' }}>Soon</span>
                    )}
                  </div>
                  <div style={{ fontSize: textSize.micro, color: colors.textMuted }}>{opt.d}</div>
                </button>
              );
            })}
          </div>
        </Row>
        <div style={{ fontSize: textSize.caption, color: colors.textMuted, marginTop: 10, lineHeight: 1.5 }}>
          Per-tool approval (Ask every time / Smart approve) is temporarily
          locked here while the approval pipeline is hardened. Approval prompts
          already land in the <strong>Decision Inbox</strong> on your Dashboard —
          these modes become selectable once the re-enable gate ships.
        </div>
        {trust !== null && !SELECTABLE_TRUST_MODES.has(trust) && (
          <div style={{ marginTop: 10, padding: '10px 14px', borderRadius: 10, background: `${colors.warning}1A`, border: `1px solid ${colors.warning}55`, color: colors.text, fontSize: textSize.caption, lineHeight: 1.5 }}>
            You're on a per-tool-approval mode: tool calls pause until you
            approve them in the <strong>Decision Inbox</strong> on your
            Dashboard. If a turn seems stuck, answer the pending approval
            there. Prefer not to approve per-tool? Switch to{' '}
            <strong>Automatic</strong> or <strong>Chat only</strong> above.
          </div>
        )}
      </Section>
      {/* Spend caps moved to Settings → Spend (which supersedes the old
          sliders here with the full soft/gate/hard ceilings for both scopes),
          so there is exactly one writer of the budget. */}
      <Section title="Spend caps" sub="The session and per-task ceilings the cost router enforces now live on the Spend page, alongside everything you have spent.">
        <Button colors={colors} style={ghost(colors)} onClick={() => goto?.('spend')}>Set spend caps in Spend →</Button>
      </Section>
    </div>
  );
}

export interface ToolExtension {
  enabled: boolean;
  type: string;
  name: string;
  description?: string;
  display_name?: string | null;
  bundled?: boolean | null;
  available_tools?: string[];
  env_keys?: string[];
}

/** What to show as an extension's title. `display_name` is absent on stdio
 *  servers, so the identifying `name` is the fallback — never a blank tile. */
export function extensionLabel(ext: ToolExtension): string {
  const label = (ext.display_name ?? '').trim();
  return label || ext.name || 'Unnamed extension';
}

function ToolsPanel({ goto }: PanelProps) {
  const { colors } = useThemeHook();
  const [extensions, setExtensions] = useState<ToolExtension[]>([]);
  const [loading, setLoading] = useState(true);
  // Re-read on `config_changed`. The agent's `manage_extensions` writes the
  // same `extensions` config entry this panel renders, and used not to write
  // it at all — this pane showed stale enabled/disabled state for the whole
  // session it was open.
  const configRev = useCommandCenter(s => s.configRev);

  useEffect(() => {
    api.getExtensions()
      // Never hand a non-array to the render path: `.filter` on undefined is
      // the same class of crash this panel just had.
      .then(r => { setExtensions(Array.isArray(r?.extensions) ? r.extensions : []); setLoading(false); })
      .catch(() => setLoading(false));
  }, [configRev]);

  const enabledCount = extensions.filter(e => e.enabled).length;
  // stdio servers that declare required env vars are the ones with API keys —
  // the panel points at where those are actually managed.
  const needKeys = extensions.filter(e => (e.env_keys?.length ?? 0) > 0);

  return (
    <div>
      {/* MCP is defined here, once — the nav label carries the user's word and
          the pane carries the acronym. */}
      <H1 sub="Tools your agent can use. Most arrive over MCP — the Model Context Protocol — so connecting a server is what gives the agent something new it can call.">Tools</H1>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 16 }}>
        <div style={{ flex: 1 }} />
        <span style={{ fontSize: textSize.caption, color: colors.textMuted }}>{enabledCount} of {extensions.length} enabled</span>
      </div>
      {needKeys.length > 0 && (
        // API keys are managed in Search & tools, not here. Without this the
        // only way to find that out is to guess — which is how someone ends up
        // on this tab looking for their Brave key.
        <div style={{
          display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap',
          padding: '9px 12px', marginBottom: 14, borderRadius: radius.md,
          background: colors.bgDeeper, border: `1px solid ${colors.border}`,
        }}>
          <span style={{ fontSize: textSize.caption, color: colors.textMuted }}>
            {needKeys.map(extensionLabel).join(' and ')} need API keys.
          </span>
          <Button colors={colors} style={ghost(colors)} onClick={() => goto('search')}>
            Manage keys in Search &amp; tools
          </Button>
        </div>
      )}
      {loading ? (
        <div style={{ color: colors.textDim, fontSize: textSize.small }}>Loading extensions...</div>
      ) : extensions.length === 0 ? (
        <Section title="No extensions"><div style={{ color: colors.textMuted, fontSize: textSize.small }}>No MCP tools or extensions configured.</div></Section>
      ) : (
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(2, 1fr)', gap: 10 }}>
          {extensions.map((ext, i) => (
            <div key={ext.name || `ext-${i}`} style={{ display: 'flex', alignItems: 'center', gap: 14, padding: 14, borderRadius: 10, background: colors.bgDeeper, border: `1px solid ${colors.border}` }}>
              <div style={{ width: 32, height: 32, borderRadius: radius.md, background: ext.enabled ? colors.cyanSoft : colors.surfaceHi, border: `1px solid ${ext.enabled ? colors.borderHi : colors.border}`, display: 'grid', placeItems: 'center', fontFamily: font.display, fontSize: textSize.small, fontWeight: 700, color: ext.enabled ? colors.cyan : colors.textMuted, flexShrink: 0 }}>{extensionLabel(ext).charAt(0).toUpperCase() || '?'}</div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: textSize.small, fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{extensionLabel(ext)}</div>
                <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {ext.type}{ext.bundled ? ' · bundled' : ''} · {ext.available_tools?.length ?? 0} tools
                  {(ext.env_keys?.length ?? 0) > 0 && ` · needs ${ext.env_keys!.join(', ')}`}
                </div>
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
function formatBytes(b: number): string {
  if (b < 1e9) return `${(b / 1e6).toFixed(0)} MB`;
  return `${(b / 1e9).toFixed(1)} GB`;
}

// ── Chat / Voice / Harness role table ──────────────────────────────────
// Mirrors crates/goose/src/config/model_roles.rs's resolve_role_model
// precedence: role keys (chat_provider/chat_model, harness_provider/
// harness_model) > the session model (GOOSE_PROVIDER/GOOSE_MODEL) > the
// measured default (which this UI does not know the id of and does not try
// to guess — it just says "(default)"). Read that module's doc comment
// before touching this: the layers above config (CLI flag, a resumed
// session's own saved model, a recipe's settings block) are real but this
// panel neither sees nor needs to represent them.
const ROLE_DISABLE_VALUES = ['session', 'off', 'none'];
const isRoleDisableValue = (v: string): boolean => ROLE_DISABLE_VALUES.includes(v.trim().toLowerCase());
const trimmedOrNull = (v: string | null | undefined): string | null => {
  if (typeof v !== 'string') return null;
  const t = v.trim();
  return t === '' ? null : t;
};
/** `api.readConfig` answers the bare JSON value or `null` — narrow to string. */
const asConfigString = (v: unknown): string | null => (typeof v === 'string' ? v : null);

type RoleEffective = { display: string; suffix: string | null };

/** Effective model for one role + where it came from. Pure and read-only —
 *  the whole point (per the bug this replaces) is that a user can look at
 *  this and know which model actually answered. `modelKeyLabel` is the
 *  role's model config key (e.g. `chat_model`), used only to word the
 *  "from …" suffix. */
function computeRoleEffective(
  provider: string | null, model: string | null,
  sessionProvider: string | null, sessionModel: string | null,
  modelKeyLabel: string,
): RoleEffective {
  const p = trimmedOrNull(provider);
  const m = trimmedOrNull(model);
  // A `session`/`off`/`none` value in EITHER key disables the role override,
  // regardless of what the other key holds (even a stale leftover model id) —
  // resolve_role_model checks each key independently for this, on purpose:
  // "back to one model for everything" should not depend on tidying up the
  // partner key too.
  if ((p !== null && isRoleDisableValue(p)) || (m !== null && isRoleDisableValue(m))) {
    return { display: 'session model (explicit)', suffix: null };
  }
  if (p !== null && m !== null) {
    return { display: `${p} / ${m}`, suffix: `from ${modelKeyLabel}` };
  }
  // Zero or exactly one role key set: resolves as if neither were — a half
  // pair is a typo, not an intention (RoleModelSource::HalfConfigured).
  const sp = trimmedOrNull(sessionProvider);
  const sm = trimmedOrNull(sessionModel);
  if (sp !== null && sm !== null) {
    return { display: `${sp} / ${sm}`, suffix: 'from GOOSE_MODEL' };
  }
  return { display: '(default)', suffix: 'built-in default' };
}

/** One editable role row — Chat or Harness. Voice is rendered by hand in
 *  ModelsPanel instead, because it resolves through voice_model.rs, whose
 *  precedence differs (see the comment on that row). */
function RoleModelRow({
  label, testId, hint, providerKey, modelKey, provider, model,
  sessionProvider, sessionModel, onSaved,
}: {
  label: string; testId: string; hint: string; providerKey: string; modelKey: string;
  provider: string | null; model: string | null;
  sessionProvider: string | null; sessionModel: string | null;
  onSaved: (provider: string | null, model: string | null) => void;
}) {
  const { colors } = useThemeHook();
  const [providerInput, setProviderInput] = useState(provider ?? '');
  const [modelInput, setModelInput] = useState(model ?? '');
  const [warn, setWarn] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Reflect a config load (or another surface's write) that lands after this
  // row already mounted with empty/stale inputs.
  useEffect(() => { setProviderInput(provider ?? ''); }, [provider]);
  useEffect(() => { setModelInput(model ?? ''); }, [model]);

  const effective = computeRoleEffective(provider, model, sessionProvider, sessionModel, modelKey);

  const handleSave = async () => {
    const p = providerInput.trim();
    const m = modelInput.trim();
    setWarn(false);
    setError(null);
    const bothEmpty = p === '' && m === '';
    const bothSet = p !== '' && m !== '';
    const pDisable = p !== '' && isRoleDisableValue(p);
    const mDisable = m !== '' && isRoleDisableValue(m);
    if (!bothEmpty && !bothSet && !pDisable && !mDisable) {
      // Exactly one filled, and it is not a session/off/none shorthand — a
      // half pair. Warn and write nothing (mirrors HalfConfigured).
      setWarn(true);
      return;
    }
    setSaving(true);
    try {
      if (bothEmpty) {
        // Both cleared: write both keys empty rather than leaving one behind.
        await api.upsertConfig(providerKey, '');
        await api.upsertConfig(modelKey, '');
        onSaved(null, null);
      } else if (bothSet) {
        await api.upsertConfig(providerKey, p);
        await api.upsertConfig(modelKey, m);
        onSaved(p, m);
      } else {
        // Exactly one filled and it is a disable shorthand (e.g. provider =
        // "session", model left blank) — write just that key.
        if (p !== '') await api.upsertConfig(providerKey, p);
        if (m !== '') await api.upsertConfig(modelKey, m);
        onSaved(p || null, m || null);
      }
    } catch (err) {
      setError(`Couldn't save: ${err instanceof Error ? err.message : String(err)}`);
    }
    setSaving(false);
  };

  return (
    <Row label={label} hint={hint}>
      <div data-testid={testId}>
      <div style={{ fontFamily: font.mono, fontSize: textSize.small, color: colors.text, marginBottom: 10 }}>
        {effective.display}
        {effective.suffix && (
          <span style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textMuted, marginLeft: 8 }}>
            · {effective.suffix}
          </span>
        )}
      </div>
      {warn && (
        <div style={{ fontSize: textSize.micro, color: colors.danger, marginBottom: 8 }}>
          provider and model must be set together, or neither
        </div>
      )}
      {error && <div style={{ fontSize: textSize.micro, color: colors.danger, marginBottom: 8 }}>{error}</div>}
      <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
        <div style={{ width: 160 }}>
          <TextInput
            value={providerInput}
            onChange={v => { setProviderInput(v); setWarn(false); }}
            placeholder="provider"
          />
        </div>
        <div style={{ flex: 1 }}>
          <TextInput
            value={modelInput}
            onChange={v => { setModelInput(v); setWarn(false); }}
            placeholder="model id, or session / off / none"
          />
        </div>
        <SaveButton onClick={() => { void handleSave(); }} disabled={saving} saving={saving} />
      </div>
      </div>
    </Row>
  );
}

export function ModelsPanel({ goto }: PanelProps) {
  const { colors } = useThemeHook();
  const [ollama, setOllama] = useState<OllamaStatus | null>(null);

  // Primary-model readout (merged from the retired Governance → Models panel).
  // Read-only: the model/provider switch itself lives in the provider modal on
  // API keys, and the per-role roster lives on Settings → Agents.
  const [primary, setPrimary] = useState<{ model: string | null; provider: string | null; mode: string | null } | null>(null);
  // Every read effect in this panel takes `configRev` so a key changed
  // anywhere else — agent, CLI, second device — re-reads here. The one
  // deliberate exception is the Watcher list below; see the note there.
  const configRev = useCommandCenter(s => s.configRev);
  useEffect(() => {
    let active = true;
    api.getConfig().then(cfg => {
      if (!active) return;
      const map = ((cfg as Record<string, unknown>)['config'] ?? cfg) as Record<string, unknown>;
      setPrimary({
        model: (map['GOOSE_MODEL'] as string) ?? null,
        provider: (map['GOOSE_PROVIDER'] as string) ?? null,
        mode: ((cfg as Record<string, unknown>)['effective_goose_mode'] as string) ?? null,
      });
    }).catch(() => {});
    return () => { active = false; };
  }, [configRev]);

  // Voice model (crates/goose/src/config/voice_model.rs) — which model
  // answers a SPOKEN turn; chat is unaffected. Both `voice_provider` and
  // `voice_model` set together override the measured default; either one set
  // to session/off/none turns the feature off and voice rides the session
  // model. Read on mount only — writes happen on Save / "Use the session
  // model", never as a side effect of loading the panel.
  const [voiceProvider, setVoiceProvider] = useState<string | null>(null);
  const [voiceModel, setVoiceModel] = useState<string | null>(null);
  const [voiceProviderInput, setVoiceProviderInput] = useState('');
  const [voiceModelInput, setVoiceModelInput] = useState('');
  const [voiceSaving, setVoiceSaving] = useState(false);
  const [voiceError, setVoiceError] = useState<string | null>(null);
  useEffect(() => {
    let active = true;
    Promise.all([api.readConfig('voice_provider'), api.readConfig('voice_model')])
      .then(([p, m]) => {
        if (!active) return;
        const provider = typeof p === 'string' ? p : null;
        const model = typeof m === 'string' ? m : null;
        setVoiceProvider(provider);
        setVoiceModel(model);
        setVoiceProviderInput(provider ?? '');
        setVoiceModelInput(model ?? '');
      })
      .catch(() => {});
    return () => { active = false; };
  }, []);
  const saveVoiceModel = () => {
    const prevProvider = voiceProvider;
    const prevModel = voiceModel;
    const nextProvider = voiceProviderInput;
    const nextModel = voiceModelInput;
    setVoiceProvider(nextProvider);
    setVoiceModel(nextModel);
    setVoiceError(null);
    setVoiceSaving(true);
    Promise.all([
      api.upsertConfig('voice_provider', nextProvider),
      api.upsertConfig('voice_model', nextModel),
    ])
      .catch(err => {
        setVoiceProvider(prevProvider);
        setVoiceModel(prevModel);
        setVoiceProviderInput(prevProvider ?? '');
        setVoiceModelInput(prevModel ?? '');
        setVoiceError(`Couldn't save: ${err instanceof Error ? err.message : String(err)}`);
      })
      .finally(() => setVoiceSaving(false));
  };
  const useSessionVoiceModel = () => {
    const prevModel = voiceModel;
    setVoiceModel('session');
    setVoiceModelInput('session');
    setVoiceError(null);
    api.upsertConfig('voice_model', 'session').catch(err => {
      setVoiceModel(prevModel);
      setVoiceModelInput(prevModel ?? '');
      setVoiceError(`Couldn't save: ${err instanceof Error ? err.message : String(err)}`);
    });
  };

  // Chat / Harness role table (crates/goose/src/config/model_roles.rs). Voice
  // is the third row of the same table but keeps its OWN state above, because
  // its precedence genuinely differs: the voice default outranks GOOSE_MODEL
  // (a spoken turn on a reasoning model is ten seconds of silence), while chat
  // and harness fall through to it. One table, two resolvers, on purpose.
  const [chatCfg, setChatCfg] = useState<{ provider: string | null; model: string | null } | null>(null);
  const [harnessCfg, setHarnessCfg] = useState<{ provider: string | null; model: string | null } | null>(null);
  useEffect(() => {
    let active = true;
    Promise.all([
      api.readConfig('chat_provider'), api.readConfig('chat_model'),
      api.readConfig('harness_provider'), api.readConfig('harness_model'),
    ]).then(([cp, cm, hp, hm]) => {
      if (!active) return;
      setChatCfg({ provider: asConfigString(cp), model: asConfigString(cm) });
      setHarnessCfg({ provider: asConfigString(hp), model: asConfigString(hm) });
    }).catch(() => {});
    return () => { active = false; };
  }, [configRev]);

  // Poll Ollama status while panel is visible
  useEffect(() => {
    let active = true;
    const poll = () => {
      api.getOllamaStatus().then(s => { if (active) setOllama(s); }).catch(() => {});
    };
    poll();
    const id = setInterval(poll, 8000);
    return () => { active = false; clearInterval(id); };
  }, []);

  return (
    <div>
      <H1 sub="Pick the brains behind the agent. Use stronger models when stakes are high; cheaper for routine work.">Models</H1>
      <RoleRoutingPrompt variant="settings" />
      <Section title="Providers" sub="Provider credentials live in the API keys tab — add or update a key there, then route to it below.">
        {/* One-line primary readout (condensed from Governance → Models; the
            full editor is redundant with the provider modal on API keys). */}
        <div style={{ fontSize: textSize.small, color: colors.text, fontFamily: font.mono, marginBottom: 12 }}>
          {primary === null
            ? 'Loading primary model…'
            : `${primary.model ?? '—'} · provider: ${primary.provider ?? 'default'}${primary.mode ? ` · mode: ${primary.mode}` : ''}`}
        </div>
        <Button colors={colors} style={ghost(colors)} onClick={() => goto('keys')}>Manage API keys</Button>
      </Section>
      {/* The old Routing/Behavior selects were decorative — hardcoded options
          wired to nothing (2026-07-10 settings audit). The real model/default
          switch lives in the provider modal on the API keys tab. */}

      {/* ── Chat / Voice / Harness role table ───────────────────────
          One concept, one place: which model answers each job, and where
          that answer came from. See crates/goose/src/config/model_roles.rs
          for the precedence this mirrors. */}
      <Section
        title="Chat, Voice and Harness"
        sub="Which model runs each job. Set both boxes together, or neither — a stray half pair is treated as unset. Type session, off, or none in either box to pin a job back to the session model above."
      >
        <RoleModelRow
          label="Chat"
          testId="role-row-chat"
          hint="typed turns in the Command Center"
          providerKey="chat_provider"
          modelKey="chat_model"
          provider={chatCfg?.provider ?? null}
          model={chatCfg?.model ?? null}
          sessionProvider={primary?.provider ?? null}
          sessionModel={primary?.model ?? null}
          onSaved={(p, m) => setChatCfg({ provider: p, model: m })}
        />
        {/* Voice is the one row whose precedence is NOT model_roles.rs's.
            `resolve_voice_model` puts the measured voice default ABOVE
            GOOSE_MODEL, so an operator who set a session model and never
            thought about voice still gets a model that does not stop to think
            before speaking. `describeVoiceRoute` mirrors that, and mirroring
            the wrong resolver here would tell the operator the wrong thing. */}
        <Row label="Voice" hint="spoken turns">
          <div data-testid="role-row-voice">
            <div style={{ fontFamily: font.mono, fontSize: textSize.small, color: colors.text, marginBottom: 10 }}>
              {describeVoiceRoute(voiceProvider, voiceModel)}
            </div>
            {voiceError && <div style={{ fontSize: textSize.micro, color: colors.danger, marginBottom: 8 }}>{voiceError}</div>}
            <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
              <div style={{ width: 160 }}>
                <TextInput
                  value={voiceProviderInput}
                  onChange={setVoiceProviderInput}
                  placeholder={DEFAULT_VOICE_PROVIDER_ID}
                  mono
                />
              </div>
              <div style={{ flex: 1 }}>
                <TextInput
                  value={voiceModelInput}
                  onChange={setVoiceModelInput}
                  placeholder={DEFAULT_VOICE_MODEL_ID}
                  mono
                />
              </div>
              <SaveButton onClick={saveVoiceModel} disabled={voiceSaving} saving={voiceSaving} />
              <Button colors={colors} style={ghost(colors)} onClick={useSessionVoiceModel}>Use the session model</Button>
            </div>
          </div>
        </Row>
        <RoleModelRow
          label="Harness"
          testId="role-row-harness"
          hint="the coding harness in the Build tab"
          providerKey="harness_provider"
          modelKey="harness_model"
          provider={harnessCfg?.provider ?? null}
          model={harnessCfg?.model ?? null}
          sessionProvider={primary?.provider ?? null}
          sessionModel={primary?.model ?? null}
          onSaved={(p, m) => setHarnessCfg({ provider: p, model: m })}
        />
      </Section>

      {/* ── Roster pointer ───────────────────────────────────────── */}
      {/* The per-role roster used to be duplicated here off GET /api/agent/workers
          with less fidelity than the Agents page (no probe-failed state, no
          grants or secrets). One surface now: Settings → Agents over
          /api/agents/roster. */}
      <Section title="Worker roster" sub="Which model each role dispatches to, with live availability, grants and required secrets.">
        <Button colors={colors} data-testid="models-open-agents" style={ghost(colors)} onClick={() => goto('agents')}>Open Agents</Button>
      </Section>

      {/* ── Ollama Status ────────────────────────────────────────── */}
      <Section title="Local models (Ollama)">
        {!ollama ? (
          <Row label="Status" hint="Checking..."><span style={{ fontSize: textSize.caption, color: colors.textDim }}>Loading...</span></Row>
        ) : !ollama.reachable ? (
          <Row label="Status" hint="Ollama is not running. Install from ollama.com and run 'ollama serve'.">
            <span style={{ fontSize: textSize.caption, color: colors.danger }}>Ollama not running</span>
          </Row>
        ) : (
          <>
            <Row label="Connection" hint="Ollama at localhost:11434">
              <span style={{ fontSize: textSize.caption, color: colors.cyan }}>Connected</span>
            </Row>
            {ollama.installed.length === 0 ? (
              <Row label="Models" hint="No models installed. Run 'ollama pull qwen2.5:3b' to get started.">
                <span style={{ fontSize: textSize.caption, color: colors.textDim }}>None</span>
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

      {/* ── Where the agents' own settings went ──────────────────
          The Guard's switch and cadence, the Watcher's teaching keys and the
          Librarian's schedule used to live here, because each of them names a
          model — which is true of nearly everything in this app. They are the
          agents' settings, so they live on the agents' page now (J8/C7), and
          this pane keeps only its stated purpose: which brain answers which
          job. One entry point per concept; this is the pointer to it. */}
      <Section
        title="Agent settings"
        sub="How the Guard sweeps, what the Watcher follows, and when the Librarian runs are settings of those agents, not of the model table. They live on each agent's own page."
      >
        <Row label="The Guard, the Watcher, the Librarian" hint="Switches, cadences and schedules — one page per agent.">
          <Button
            colors={colors}
            style={ghost(colors)}
            data-testid="models-open-agents"
            onClick={() => goto('agents')}
          >
            Open Agents →
          </Button>
        </Row>
      </Section>
    </div>
  );
}

function KeysPanel() {
  return (
    <div>
      <H1 sub="Bring your own keys for the providers you use. Connected keys sit at the top; the rest of the catalogue stays on Providers. Add, replace, or remove a key here — keys are encrypted in your system keychain and never leave your device.">API keys</H1>
      <Section title="Keys">
        <ProvidersSection />
      </Section>
    </div>
  );
}

function SearchPanel() {
  return (
    <div>
      <H1 sub="Web search, Polybot, and other service tools. Add a key, and it is encrypted in your system keychain — it never leaves your device.">Search &amp; tools</H1>
      <Section title="Search providers">
        <SearchToolsSection />
      </Section>
      <Section title="Polybot" sub="Polymarket credentials. Off until you turn Polybot on from the Finance tab (risk disclaimer). Start reads these from the keychain.">
        <PolybotKeys />
      </Section>
      <Section title="Fundamentals" sub="Optional financialdatasets.ai key. Quotes still work without it. Same field as the Finance tab.">
        <FundamentalsKey />
      </Section>
    </div>
  );
}

function AppearancePanel() {
  const { colors } = useThemeHook();
  const prefs = useThemeHook();
  // Literal hex by design: these swatch gradients intentionally depict each theme's own palette, regardless of the active theme.
  const themes: Array<{ id: ThemePref; l: string; g: string }> = [
    { id: 'system', l: 'System', g: 'linear-gradient(135deg, #F8FAFC 0%, #D8DEE8 48%, #0B1220 52%, #1E2433 100%)' },
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
      <Section title="Theme" sub="System follows your device — silver by day, dark when your other apps go dark.">
        <div role="radiogroup" aria-label="Theme" style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 10 }}>
          {themes.map(th => {
            const on = prefs.themePref === th.id;
            return (
              <div
                key={th.id}
                role="radio"
                aria-checked={on}
                aria-label={th.l}
                tabIndex={0}
                onClick={() => setThemeFn(th.id)}
                onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); setThemeFn(th.id); } }}
                style={{
                  padding: 4, borderRadius: radius.lg, cursor: 'pointer', outline: 'none',
                  border: on ? `2px solid ${colors.cyan}` : '2px solid transparent',
                  boxShadow: on ? `0 0 14px ${colors.cyanGlow}` : 'none',
                }}
                onFocus={e => { if (!on) e.currentTarget.style.borderColor = colors.borderHi; }}
                onBlur={e => { if (!on) e.currentTarget.style.borderColor = 'transparent'; }}
              >
                <div style={{ height: 96, borderRadius: radius.md, background: th.g, border: `1px solid ${colors.border}` }} />
                <div style={{ fontSize: textSize.caption, padding: '8px 4px', textAlign: 'center', color: on ? colors.cyan : colors.text }}>{th.l}</div>
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

/** The REAL keyboard map (2026-07 wiring audit). The old list was fictional —
 *  it showed a command palette on ⌘K, G-key navigation, ⌘P pause and more,
 *  none of which exist. Every binding below is implemented; keep this in sync
 *  with the keydown handlers it cites. */
export const SHORTCUT_GROUPS: Array<{ g: string; items: Array<[string, string[]]> }> = [
  { g: 'Global', items: [
    ['Open or close Settings', ['⌘', ',']],
    ['Close Settings', ['Esc']],
    ['Switch workspace 1–5', ['⌘', '1–5']],
  ]},
  { g: 'Chat', items: [
    ['Send message', ['↵']],
    ['New line', ['⇧', '↵']],
  ]},
  { g: 'Terminal', items: [
    ['New terminal tab', ['⌘', 'T']],
    ['Close terminal tab', ['⌘', 'W']],
    ['Clear terminal', ['⌘', 'K']],
  ]},
  { g: 'Browser', items: [
    ['New browser tab', ['⌘', 'T']],
    ['Close browser tab', ['⌘', 'W']],
    ['Focus address bar', ['⌘', 'L']],
    ['Reload page', ['⌘', 'R']],
    ['Zoom in / out', ['⌘', '+ / −']],
    ['Reset zoom', ['⌘', '0']],
  ]},
  { g: 'Projects', items: [
    ['Save note', ['⌘', '↵']],
  ]},
];

function ShortcutsPanel() {
  const { colors } = useThemeHook();
  return (
    <div>
      <H1 sub="The current keyboard map — every binding listed here works today. Rebinding is coming later.">Shortcuts</H1>
      {SHORTCUT_GROUPS.map(grp => (
        <Section key={grp.g} title={grp.g}>
          {grp.items.map(([l, keys]) => (
            <div key={l} style={{ display: 'flex', alignItems: 'center', padding: '12px 0', borderTop: `1px solid ${colors.border}` }}>
              <span style={{ fontSize: textSize.small, flex: 1 }}>{l}</span>
              <div style={{ display: 'flex', gap: 4 }}>{keys.map((k, i) => <Kbd key={i}>{k}</Kbd>)}</div>
            </div>
          ))}
        </Section>
      ))}
    </div>
  );
}

export function DataPanel({ goto }: { goto?: (key: string) => void } = {}) {
  const { colors } = useThemeHook();

  // Product-analytics consent is a REAL backend gate (#327 split; #845 fix).
  // It must render from the backend (off by default, explicit opt-in), never a
  // hardcoded UI default. `null` = not loaded; the toggle reads false until
  // the true value arrives so it can never flash ON.
  //
  // Removed in the 2026-08 finish-the-settings ruling:
  //  - "Share anonymous diagnostics": it persisted crash_reports_consent,
  //    which nothing reads — there is no ambient upload path, and the export
  //    below is explicitly not gated on it.
  //  - "Share prompts to improve models": it flipped this SAME analytics
  //    consent; no prompt-sharing pipeline exists.
  //  - The disabled "Keep everything on this device" / "End-to-end
  //    encryption" toggles: the real local-only control is Sovereignty.
  const [analytics, setAnalytics] = useState<boolean | null>(null);
  const [consentError, setConsentError] = useState<string | null>(null);
  const analyticsGeneration = useRef(0);

  useEffect(() => {
    const analyticsRequest = ++analyticsGeneration.current;
    api.getCrashConsent()
      .then(s => {
        if (analyticsRequest === analyticsGeneration.current) setAnalytics(s.analyticsConsented);
      })
      .catch(() => {
        if (analyticsRequest === analyticsGeneration.current) {
          setConsentError('Could not load analytics consent.');
        }
      });
    return () => {
      ++analyticsGeneration.current;
    };
  }, []);

  // Returns the round trip so the switch is busy while it is in flight; the
  // rollback and the message stay here, where the generation guard lives.
  const saveAnalytics = useCallback((v: boolean) => {
    const generation = ++analyticsGeneration.current;
    setConsentError(null);
    const prev = analytics;
    setAnalytics(v); // optimistic
    return api.setAnalyticsConsent(v)
      .then(s => { if (generation === analyticsGeneration.current) setAnalytics(s.analyticsConsented); })
      .catch(err => {
        if (generation !== analyticsGeneration.current) return;
        setAnalytics(prev); // roll back on failure — never claim consent we didn't persist
        setConsentError(`Couldn't save: ${err instanceof Error ? err.message : String(err)}`);
      });
  }, [analytics]);

  // User-triggered redacted export (#327 MVP): writes a REDACTED bundle locally
  // and returns its path + content so the user can inspect exactly what would
  // be shared. No network upload.
  const [exporting, setExporting] = useState(false);
  const [exportResult, setExportResult] = useState<CrashExportResponse | null>(null);
  const [exportError, setExportError] = useState<string | null>(null);

  const runExport = useCallback(() => {
    setExporting(true);
    setExportError(null);
    setExportResult(null);
    // Handing the promise back gives the export a visible in-flight phase; the
    // `false` in the catch is what keeps a failed export — already swallowed
    // into `exportError` — from finishing with a success tick.
    return api.exportCrashReport()
      .then(r => { setExportResult(r); return true; })
      .catch(err => { setExportError(`Export failed: ${err instanceof Error ? err.message : String(err)}`); return false; })
      .finally(() => setExporting(false));
  }, []);

  return (
    <div>
      <H1 sub="Your data is yours. Everything is local-first today.">Data &amp; privacy</H1>
      <Section title="Local-first">
        <div style={{ fontSize: textSize.small, color: colors.textMuted, lineHeight: 1.6, display: 'flex', alignItems: 'center', gap: 12, flexWrap: 'wrap' }}>
          <span>
            Memory and traces live on this machine. To make the boundary
            enforced — blocking every cloud inference call — use Sovereignty.
          </span>
          <Button colors={colors} style={ghost(colors)} onClick={() => goto?.('sovereignty')}>Open Sovereignty →</Button>
        </div>
      </Section>
      <Section title="Diagnostics" sub="Live — an off-by-default opt-in written to the daemon's consent gate.">
        <Row label="Share product analytics" hint="Anonymous usage and timing. Never your prompts."><Toggle on={!!analytics} onChange={saveAnalytics} /></Row>
        {consentError && (
          <div style={{ fontSize: textSize.caption, color: colors.danger, padding: '2px 0 8px' }}>{consentError}</div>
        )}
      </Section>
      <Section title="Crash report" sub="Export a redacted crash report to attach to a support message. Written locally — home paths, keys, tokens, emails, and UUIDs are redacted first. Nothing is uploaded.">
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap', alignItems: 'center' }}>
          <Button
            colors={colors}
            onClick={runExport}
            disabled={exporting}
            style={{
              '--pa-btn-bg': colors.surfaceHi,
              '--pa-btn-fg': colors.text,
              '--pa-btn-border': colors.border,
              '--pa-btn-border-hover': colors.borderHi,
              '--pa-btn-bg-hover': colors.surfaceHi,
              '--pa-btn-bg-active': colors.surface,
              '--pa-btn-pad': '6px 12px',
              '--pa-btn-radius': `${radius.sm}px`,
              fontSize: textSize.caption,
            } as CSSProperties}
          >{exporting ? 'Exporting…' : 'Export redacted crash report'}</Button>
        </div>
        {exportError && (
          <div style={{ fontSize: textSize.caption, color: colors.danger, padding: '6px 0' }}>{exportError}</div>
        )}
        {exportResult && (
          <div style={{ padding: '8px 0' }}>
            <div style={{ fontSize: textSize.caption, color: colors.textDim }}>
              {exportResult.reportCount === 0
                ? 'No crash reports captured. Saved an empty redacted bundle to:'
                : `${exportResult.reportCount} crash report(s) redacted and saved to:`}
            </div>
            <div style={{ fontSize: textSize.caption, color: colors.text, fontFamily: font.mono, wordBreak: 'break-all', padding: '2px 0 6px' }}>{exportResult.path}</div>
            <div style={{ fontSize: textSize.micro, color: colors.textDim, paddingBottom: 4 }}>Preview (exactly what would be shared):</div>
            <pre style={{
              fontSize: textSize.micro, fontFamily: font.mono, color: colors.text, background: colors.surface,
              border: `1px solid ${colors.border}`, borderRadius: radius.sm, padding: 8, margin: 0,
              maxHeight: 220, overflow: 'auto', whiteSpace: 'pre-wrap', wordBreak: 'break-word',
            }}>{exportResult.content}</pre>
          </div>
        )}
      </Section>
    </div>
  );
}

// ── Panel router ─────────────────────────────────────────────────────

/** Grid columns for the egress-audit table (ported from the retired
 *  Governance → Sovereignty panel — the table read better than stacked rows). */
const EGRESS_COLS = '150px 1fr 90px 80px';

/** Sovereignty — the data boundary. The toggle writes the daemon's global
 *  sovereign flag (enforced fail-closed at the provider choke point); the
 *  egress log shows every cloud inference call this machine has made or
 *  blocked. Live end-to-end (2026-07 sovereignty-router build). Absorbed the
 *  Governance → Sovereignty panel (same endpoints): its "Pull the cable"
 *  button and its egress TABLE layout now live here. */
function SovereigntyPanel() {
  const { colors } = useThemeHook();
  const [status, setStatus] = useState<SovereigntyStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [log, setLog] = useState<EgressLogEntry[] | null>(null);
  /** The read failed. It used to land in `setLog([])`, and the empty state
   *  below is not an invitation — it is a PRIVACY GUARANTEE ("Nothing has left
   *  this machine yet"). A network error was being rendered as a promise about
   *  the user's data, on the one panel whose whole job is to be believed. */
  const [logError, setLogError] = useState(false);

  const refreshLog = useCallback(() => {
    // Returns the round trip so the Refresh button can show it, and resolves
    // `false` on a failure so it cannot tick over one.
    return api.getEgressLog(100)
      .then(l => { setLog(l); setLogError(false); return true; })
      .catch(() => { setLogError(true); return false; });
  }, []);

  useEffect(() => {
    api.getSovereignty().then(setStatus).catch(() => setError('Could not load sovereignty status.'));
    refreshLog();
  }, [refreshLog]);

  const save = (patch: { enabled?: boolean; capturePrompts?: boolean }) => {
    setError(null);
    // Optimistic; the daemon echoes the authoritative status back.
    setStatus(s => (s ? {
      enabled: patch.enabled ?? s.enabled,
      capturePrompts: patch.capturePrompts ?? s.capturePrompts,
      localProviderAvailable: s.localProviderAvailable,
    } : s));
    return api.setSovereignty(patch)
      .then(status => { setStatus(status); refreshLog(); })
      .catch(err => setError(`Couldn't save: ${err instanceof Error ? err.message : String(err)}`));
  };

  return (
    <div>
      <H1 sub="Make the data boundary real. With sovereign mode on, every model call stays on this machine — cloud providers are refused (fail-closed), not just deprioritized.">Sovereignty</H1>

      {error && (
        <div style={{ fontSize: textSize.caption, color: colors.danger, padding: '4px 0 8px' }}>{error}</div>
      )}

      <Section title="Sovereign mode" sub="Live — writes the daemon's global sovereign flag, enforced at the provider choke point for every session.">
        <Row
          label="Data boundary"
          hint={status?.enabled
            ? 'All cloud inference is blocked before any data leaves this machine. Only local models run.'
            : 'Cloud inference is allowed. Every cloud call is still recorded in the audit log below.'}
        >
          <Button
            colors={colors}
            variant={status?.enabled ? 'ghostOn' : 'primary'}
            onClick={() => save({ enabled: !status?.enabled })}
            disabled={status === null}
            // `save` resolves the same way whether the daemon took the write or
            // refused it (it reports through the `error` line above), so there
            // is nothing here that could honestly earn a tick. The label
            // flipping to the opposite verb is the confirmation.
            flashSuccess={false}
            style={{
              '--pa-btn-bg': status?.enabled ? 'transparent' : colors.cyan,
              '--pa-btn-fg': status?.enabled ? colors.cyan : colors.textOnCyan,
              '--pa-btn-border': status?.enabled ? colors.borderHi : 'transparent',
              '--pa-btn-border-hover': status?.enabled ? colors.cyan : 'transparent',
              '--pa-btn-bg-hover': status?.enabled ? colors.cyanSoft : colors.cyan,
              '--pa-btn-bg-active': status?.enabled ? colors.cyanGlow : colors.cyan,
              '--pa-btn-pad': '0 18px',
              '--pa-btn-radius': `${radius.md}px`,
              '--pa-btn-weight': 600,
              height: 32, fontFamily: font.body, fontSize: textSize.caption,
            } as CSSProperties}
          >
            {status === null ? '…' : status.enabled ? 'Allow cloud again' : 'Pull the cable'}
          </Button>
        </Row>
        {status?.enabled && !status.localProviderAvailable && (
          <div style={{ fontSize: textSize.caption, color: colors.warning, padding: '2px 0 8px' }}>
            No local provider (Ollama or local-inference) is registered — with sovereign mode on, inference will be refused until one is available.
          </div>
        )}
        <Row label="Capture full prompts in the audit log" hint="Off by default — only a SHA-256 hash is stored. On records the full prompt text locally.">
          <Toggle on={!!status?.capturePrompts} onChange={v => save({ capturePrompts: v })} />
        </Row>
      </Section>

      <Section title="Egress audit" sub="Every cloud call, allowed or blocked — newest first. BLOCKED means sovereign mode refused it before anything left this machine.">
        <Row label="Cloud inference calls" hint={`${log?.length ?? 0} recorded`}>
          <Button
            colors={colors}
            onClick={refreshLog}
            style={{
              '--pa-btn-bg': colors.surfaceHi,
              '--pa-btn-fg': colors.text,
              '--pa-btn-border': colors.border,
              '--pa-btn-border-hover': colors.borderHi,
              '--pa-btn-bg-hover': colors.surfaceHi,
              '--pa-btn-bg-active': colors.surface,
              '--pa-btn-pad': '4px 10px',
              '--pa-btn-radius': `${radius.sm}px`,
              fontSize: textSize.caption,
            } as CSSProperties}
          >Refresh</Button>
        </Row>
        {logError ? (
          <StateBlock
            tone="error"
            compact
            title="Couldn't read the egress log."
            detail="This says nothing about whether calls were made — only that the record could not be read. Do not take a silent panel as a guarantee."
            onRetry={() => { void refreshLog(); }}
          />
        ) : log === null ? (
          <div style={{ fontSize: textSize.caption, color: colors.textDim, padding: '6px 0' }}>Loading audit log…</div>
        ) : log.length === 0 ? (
          <div style={{ fontSize: textSize.caption, color: colors.textDim, padding: '6px 0' }}>
            Nothing has left this machine yet. Every cloud call will be recorded here.
          </div>
        ) : (
          <div style={{ marginTop: 12, overflowX: 'auto' }}>
            <div style={{ minWidth: 460 }}>
              <div style={{ display: 'grid', gridTemplateColumns: EGRESS_COLS, gap: 12, padding: '0 10px 8px', fontSize: 10, fontWeight: 600, letterSpacing: '0.08em', textTransform: 'uppercase', color: colors.textDim }}>
                <div>When</div><div>Provider · Model</div><div>Kind</div><div>Result</div>
              </div>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                {log.map(e => (
                  <div key={e.id} style={{ display: 'grid', gridTemplateColumns: EGRESS_COLS, gap: 12, alignItems: 'center', padding: '8px 10px', borderRadius: radius.md, background: colors.bgDeeper, border: `1px solid ${colors.border}` }}>
                    <div style={{ fontSize: textSize.caption, color: colors.textMuted, fontFamily: font.mono }} title={`${new Date(e.ts).toLocaleString()}${e.sessionId ? ' · ' + e.sessionId : ''} · ${e.contentHash.slice(0, 12)}…`}>
                      {timeAgo(e.ts) || e.ts}
                    </div>
                    <div style={{ fontSize: textSize.caption, color: colors.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={`${e.provider} · ${e.model}`}>
                      <span style={{ color: colors.textMuted }}>{e.provider}</span> · {e.model}
                    </div>
                    <div style={{ fontSize: textSize.caption, color: colors.textMuted }}>{e.kind}</div>
                    <div>
                      <span style={{
                        fontSize: 10, fontWeight: 700, letterSpacing: '0.04em', textTransform: 'uppercase',
                        padding: '2px 8px', borderRadius: radius.pill, border: `1px solid ${colors.border}`,
                        color: e.blocked ? colors.warning : colors.success,
                      }}>
                        {e.blocked ? 'blocked' : 'allowed'}
                      </span>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          </div>
        )}
      </Section>
    </div>
  );
}

// ── Console pages folded into Settings (2026-08 ruling) ─────────────
// Each pane embeds the SAME component the Console overlay hosted — only the
// chrome changed. Selecting a session (or any action that navigates to chat)
// closes Settings, because those components call setActivePanel('chat').

function SessionsPane() {
  const { colors } = useThemeHook();
  return (
    <div style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
      <H1 sub="Your past conversations — reopen one to pick up where you left off, or rename and delete old ones. Picking a session opens it in the chat.">Sessions</H1>
      <div style={{ flex: 1, minHeight: 320, border: `1px solid ${colors.border}`, borderRadius: radius.lg, overflow: 'hidden' }}>
        <SessionsList />
      </div>
    </div>
  );
}

function InboxPane() {
  const { colors } = useThemeHook();
  return (
    <div style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
      <H1 sub="Files you download in the in-app browser land here — send them to the Brain, a project, or the post scheduler. You choose; nothing is routed for you.">Downloads</H1>
      <div style={{ flex: 1, minHeight: 320, border: `1px solid ${colors.border}`, borderRadius: radius.lg, overflow: 'hidden' }}>
        <InboxPanel embedded />
      </div>
    </div>
  );
}

/** Open-incident triage (wave-1 item 2): the failure-learning loop files
 *  incidents, workers read them into every plan — this is the missing half
 *  where a human closes them out. Honest quiet state: renders nothing when
 *  there are none. */
function IncidentsStrip() {
  const { colors } = useThemeHook();
  const [incidents, setIncidents] = useState<IncidentView[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    api.getIncidents().then(i => { if (live) setIncidents(i); }).catch(() => { if (live) setIncidents([]); });
    return () => { live = false; };
  }, []);

  const resolve = async (id: string) => {
    setBusy(id);
    let resolved = true;
    try {
      await api.resolveIncident(id);
      setIncidents(prev => (prev ?? []).filter(i => i.id !== id));
    } catch {
      // Leave the row; the next load retells the truth. `false` is what keeps
      // the button from ticking over an incident that is still open.
      resolved = false;
    }
    setBusy(null);
    return resolved;
  };

  if (!incidents || incidents.length === 0) return null;
  return (
    <div style={{ marginBottom: 12, border: `1px solid ${colors.border}`, borderRadius: radius.lg, padding: '10px 14px' }}>
      <div style={{ fontSize: 10, fontWeight: 700, letterSpacing: '0.1em', textTransform: 'uppercase', color: '#e8a33d', marginBottom: 6 }}>
        Open incidents — feeding every worker plan until resolved
      </div>
      {incidents.map(i => (
        <div key={i.id} style={{ display: 'flex', alignItems: 'baseline', gap: 10, padding: '5px 0', borderBottom: `1px solid ${colors.border}` }}>
          <span style={{ fontSize: textSize.caption, color: colors.text, flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={`${i.user_goal} — ${i.observation}`}>
            [{i.surface}] {i.observation}
          </span>
          <span style={{ fontSize: 10, color: colors.textDim, fontFamily: 'monospace' }}>{i.mechanism}</span>
          <Button
            colors={colors}
            variant="ghostOn"
            onClick={() => resolve(i.id)}
            disabled={busy === i.id}
            style={{
              '--pa-btn-border': colors.borderHi,
              '--pa-btn-border-hover': colors.cyan,
              '--pa-btn-pad': '2px 10px',
              '--pa-btn-radius': `${radius.sm}px`,
            } as CSSProperties}
          >
            {busy === i.id ? '…' : 'Resolve'}
          </Button>
        </div>
      ))}
    </div>
  );
}

function ActivityPane() {
  const { colors } = useThemeHook();
  return (
    <div style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
      <H1 sub="The runtime's most recent events, live off the running system's event streams — tool calls, worker activity, navigations, and lifecycle signals as they happen.">Activity</H1>
      <IncidentsStrip />
      <div style={{ flex: 1, minHeight: 320, border: `1px solid ${colors.border}`, borderRadius: radius.lg, overflow: 'hidden' }}>
        <ExecutionTrace />
      </div>
    </div>
  );
}

function SpendPane() {
  return (
    <div>
      <H1 sub="What you run costs money — everything you have spent, per project and per session, plus the caps the cost router enforces. Enforced locally, not by a cloud admin.">Spend</H1>
      <SpendPanel />
    </div>
  );
}

function DataSourcesPanel() {
  return (
    <div>
      <H1 sub="Public APIs you can turn on for agents. Browse a category at a time. Enabling a source makes it callable immediately — suggested agents get it, and the Orchestrator can call every enabled source.">Data sources</H1>
      <DataSourcesSection />
    </div>
  );
}

const PANELS: Record<string, (props: PanelProps) => JSX.Element> = {
  agent: PersonaPanel, preferences: PreferencesPanel,
  memory: MemoryPanel, autonomy: AutonomyPanel, tools: ToolsPanel,
  models: ModelsPanel, keys: KeysPanel, devices: DevicesPanel, search: SearchPanel,
  sources: DataSourcesPanel,
  appearance: AppearancePanel, shortcuts: ShortcutsPanel, data: DataPanel,
  sovereignty: SovereigntyPanel,
  sessions: SessionsPane, inbox: InboxPane, activity: ActivityPane, spend: SpendPane,
  agents: AgentsPanel,
  features: FeaturesPanel,
};


function PairingQrCode({ value, size = 112 }: { value: string; size?: number }) {
  let matrix: boolean[][];
  try {
    matrix = makeQrMatrix(value);
  } catch {
    return <span style={{ fontSize: textSize.micro }}>QR unavailable — shorten the hub address or copy the link.</span>;
  }
  const quiet = 4;
  // Module extent of the symbol itself, in QR modules — distinct from `size`,
  // which is the rendered pixel width.
  const extent = matrix.length + quiet * 2;
  const path = matrix.flatMap((row, y) => row.map((dark, x) => dark ? `M${x + quiet},${y + quiet}h1v1h-1z` : '')).join('');
  return (
    <svg
      role="img"
      aria-label="Pairing QR code"
      viewBox={`0 0 ${extent} ${extent}`}
      width={size}
      height={size}
      style={{ display: 'block', background: '#fff', borderRadius: radius.md }}
      shapeRendering="crispEdges"
    >
      <rect width={extent} height={extent} fill="#fff" />
      <path d={path} fill="#000" />
    </svg>
  );
}

/** Devices — hub-and-spoke pairing (MULTI_DEVICE.md, #628). The hub (this
 *  machine) holds the one Brain; every other device connects to it over the
 *  tailnet by opening a pairing URL once. The URL carries a ONE-TIME claim
 *  code (`#claim=`) that the new device exchanges for its own token on first
 *  load (see api.ts pendingClaimCode/exchangeClaim) — so each companion is a
 *  named, individually revocable entry in the registry below. */
function DevicesPanel() {
  const { colors } = useThemeHook();
  const agentName = useCommandCenter(s => s.agentName);
  const [host, setHost] = useState('your-mac.tailnet-name.ts.net');
  const [copied, setCopied] = useState(false);
  const [tailnet, setTailnet] = useState<{ installed: boolean; running: boolean; magic_dns_name: string | null } | null>(null);
  // Reachability is TWO facts: what the user asked for (`enabled`, persisted)
  // and whether the running daemon is actually on the tailnet (`effective`).
  // They diverge whenever a restart is pending or Tailscale is down, and the
  // panel has to show the difference — a green toggle over an unreachable
  // daemon is exactly the lie that made pairing fail silently.
  const [access, setAccess] = useState<{
    enabled: boolean; serve_url: string | null; available: boolean;
  } | null>(null);
  const [accessBusy, setAccessBusy] = useState(false);

  // ── Device registry (#628) ──
  const [devices, setDevices] = useState<DeviceInfo[] | null>(null);
  const [pairName, setPairName] = useState('');
  const [claim, setClaim] = useState<{ code: string; expiresAt: string } | null>(null);
  const [pairError, setPairError] = useState<string | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editName, setEditName] = useState('');
  const [confirmRevokeId, setConfirmRevokeId] = useState<string | null>(null);
  const loadDevices = useCallback(() => {
    api.listDevices().then(setDevices).catch(() => setDevices(null));
  }, []);
  // Live last-seen: the middleware stamps it on every authenticated device
  // request; a light poll keeps "last seen 2m ago" honest while the panel is
  // open.
  useEffect(() => {
    loadDevices();
    const t = setInterval(loadDevices, 30_000);
    return () => clearInterval(t);
  }, [loadDevices]);
  // Deterministic detection: when the hub is on a tailnet, the address fills
  // itself — the user types nothing (zero-strain ruling, 2026-07-11).
  useEffect(() => {
    apiFetch<{ enabled: boolean; serve_url: string | null; available: boolean }>(
      '/api/tailnet/access',
    ).then(setAccess).catch(() => {});
    apiFetch<{ installed: boolean; running: boolean; magic_dns_name: string | null }>('/api/tailnet/status')
      .then(t => {
        setTailnet(t);
        if (t.magic_dns_name) setHost(t.magic_dns_name);
      })
      .catch(() => setTailnet(null));
  }, []);
  // The pairing URL carries a one-time claim code — never a bearer token
  // (#628). It is minted on demand below and goes inert after one use.
  const remoteBase = access?.serve_url ?? (host ? `http://${host}:3001` : null);
  const pairingUrl = claim && remoteBase ? `${remoteBase}/ui/#claim=${claim.code}` : null;
  const isHub = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
  const [detail, setDetail] = useState(0); // progressive disclosure depth
  const [hubUp, setHubUp] = useState<boolean | null>(null);
  useEffect(() => {
    apiFetch<{ status: string }>('/status').then(() => setHubUp(true)).catch(() => setHubUp(false));
  }, []);
  return (
    <div>
      <H1 sub="One Brain, one truth: your strongest machine is the hub — every other device connects to it. No accounts, no sync conflicts; pairing is a URL, opened once per device.">Devices</H1>

      {/* Role clarity (ruling 2026-07-11): friendly first, deeper on ask. */}
      <Section title={isHub ? 'This device is your hub' : 'This device is a companion'}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 8 }}>
          <span style={{
            width: 8, height: 8, borderRadius: '50%',
            background: hubUp === false ? colors.danger : colors.cyan,
            boxShadow: hubUp === false ? 'none' : `0 0 8px ${colors.cyan}`,
          }} />
          <span style={{ fontSize: textSize.small, color: colors.text }}>
            {isHub
              ? 'Everything lives here — your memories, projects, and models. Keep this machine on so your other devices can reach Permagent.'
              : hubUp === false
                ? 'The hub is not answering — make sure it is awake and on the tailnet.'
                : 'You are connected to your hub. Everything you see lives there, not on this device.'}
          </span>
        </div>
        {detail < 2 && (
          // Its whole job is to reveal the paragraph below it, and the reveal
          // is the confirmation — a tick on top would be noise.
          <Button colors={colors} flashSuccess={false} style={ghost(colors)} onClick={() => setDetail(d => d + 1)}>
            {detail === 0 ? 'Tell me more' : 'How does it work exactly?'}
          </Button>
        )}
        {detail >= 1 && (
          <p style={{ fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.6, margin: '10px 0 0' }}>
            Permagent works like a home base with visitors: the hub is the one machine that runs
            the Permagent daemon and stores every memory, project, and model. Phones, laptops,
            and tablets are companions — they show you everything and let you act from anywhere,
            but they keep nothing except their key to the hub. If the hub is asleep or offline,
            companions can't reach Permagent until it's back.
          </p>
        )}
        {detail >= 2 && (
          <p style={{ fontSize: textSize.caption, color: colors.textDim, lineHeight: 1.6, margin: '10px 0 0' }}>
            Under the hood: the hub's daemon (permagentd) serves the API and this very interface
            over your private Tailscale network; companions authenticate with the pairing token
            (a bearer secret — no accounts). There is exactly one writable Brain, so nothing
            ever needs to sync or merge. The hub should be your most capable, always-on machine
            — most RAM and storage, since it runs the largest local model and holds all data.
            Full design: docs/architecture/MULTI_DEVICE.md in the repo.
          </p>
        )}
      </Section>

      {/* #628: the device registry — named companions, last-seen, revocation. */}
      <Section title="Paired devices" sub="Every companion has its own key. Revoking one locks out that device only — nothing else re-pairs.">
        {devices === null && (
          <div style={{ fontSize: textSize.caption, color: colors.textDim, padding: '6px 0' }}>
            Device list unavailable — is the daemon reachable?
          </div>
        )}
        {devices?.length === 0 && (
          <div style={{ fontSize: textSize.caption, color: colors.textDim, padding: '6px 0' }}>
            No devices paired yet. Create a pairing link below.
          </div>
        )}
        {devices?.map(d => (
          <Row
            key={d.id}
            label={d.name}
            hint={
              (d.revoked ? 'Revoked · ' : '')
              + `Paired ${new Date(d.created).toLocaleDateString()}`
              + ` · ${d.last_seen ? `last seen ${relativeTimeAgo(d.last_seen) || 'just now'}` : 'never seen'}`
            }
          >
            {editingId === d.id ? (
              <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <TextInput value={editName} onChange={setEditName} placeholder="Device name" />
                <Button
                  colors={colors}
                  style={ghost(colors)}
                  // Handing the promise back is what buys the round trip a
                  // visible pending phase; the `false` in the catch is what
                  // stops a rename the daemon refused from ticking anyway.
                  onClick={() => {
                    const name = editName.trim();
                    if (!name) return false;
                    return api.renameDevice(d.id, name)
                      .then(() => { setEditingId(null); loadDevices(); return true; })
                      .catch(() => { setEditingId(null); return false; });
                  }}
                >Save</Button>
                <Button colors={colors} style={ghost(colors)} onClick={() => setEditingId(null)}>Cancel</Button>
              </div>
            ) : (
              <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                {d.revoked ? (
                  <span style={{ fontSize: textSize.micro, fontWeight: 600, color: colors.danger }}>REVOKED</span>
                ) : (
                  <>
                    <Button
                      colors={colors}
                      style={ghost(colors)}
                      onClick={() => { setEditingId(d.id); setEditName(d.name); setConfirmRevokeId(null); }}
                    >Rename</Button>
                    <Button
                      colors={colors}
                      // The danger palette rides in as custom properties, not as
                      // an inline `color`/`borderColor`: those would beat the
                      // hover rule and leave this reading as unpressable.
                      style={{
                        ...ghost(colors),
                        '--pa-btn-fg': colors.danger,
                        '--pa-btn-border': `${colors.danger}66`,
                        '--pa-btn-border-hover': colors.danger,
                        '--pa-btn-bg-hover': `${colors.danger}1A`,
                        '--pa-btn-bg-active': `${colors.danger}26`,
                      } as CSSProperties}
                      title="This device stops authenticating immediately. Pair it again to restore access."
                      // The first press only arms the confirmation — nothing is
                      // in flight, so it returns nothing and nothing ticks.
                      onClick={() => {
                        if (confirmRevokeId !== d.id) { setConfirmRevokeId(d.id); return; }
                        return api.revokeDevice(d.id)
                          .then(() => { setConfirmRevokeId(null); loadDevices(); return true; })
                          .catch(() => { setConfirmRevokeId(null); return false; });
                      }}
                    >{confirmRevokeId === d.id ? 'Confirm revoke' : 'Revoke'}</Button>
                  </>
                )}
              </div>
            )}
          </Row>
        ))}
      </Section>

      <Section title="Pair a device" sub="Turn on tailnet access below, then scan the QR code with the device you are adding. Both devices must be on your tailnet.">
        <Row
          label="Remote access"
          hint={
            access?.available === false
              ? 'Tailscale is not installed. Permagent does not require it — any tunnel that gives this Mac a reachable address works, and the hub address below accepts it.'
              : access?.enabled
                ? `Your devices can reach this hub at ${access.serve_url}. Tailscale publishes it to your private network only — the daemon itself still listens on localhost, so nothing is exposed to the Wi-Fi you are joined to, or to the internet.`
                : 'Off — this hub is only reachable from this machine, so no phone can pair with it.'
          }
        >
          <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
            <Toggle
              on={!!access?.enabled}
              disabled={accessBusy || access?.available === false}
              onChange={(next) => {
                setAccessBusy(true);
                apiFetch<{ enabled: boolean; serve_url: string | null; available: boolean }>(
                  '/api/tailnet/access',
                  {
                    method: 'PUT',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ enabled: next }),
                  },
                )
                  .then(setAccess)
                  .catch(e => setPairError(e instanceof Error ? e.message : 'Could not change remote access'))
                  .finally(() => setAccessBusy(false));
              }}
            />
            <span style={{ fontSize: textSize.caption, color: colors.textMuted }}>
              {accessBusy ? 'Applying…'
                : access?.enabled ? `Live at ${access.serve_url}`
                : 'This machine only'}
            </span>
          </div>
        </Row>
        <Row label="Tailnet" hint={tailnet?.running ? 'Detected — address filled in automatically.' : tailnet?.installed ? 'Tailscale is installed but not connected.' : 'Tailscale not detected on this machine.'}>
          {tailnet?.running ? (
            <span style={{ fontSize: textSize.caption, color: colors.cyan }}>● Connected{tailnet.magic_dns_name ? ` — ${tailnet.magic_dns_name}` : ''}</span>
          ) : (
            <Button
              colors={colors}
              style={ghost(colors)}
              title={`Copies a setup request and opens chat — ${agentName} runs the terminal steps for you.`}
              onClick={() => {
                navigator.clipboard.writeText(
                  'Set up Tailscale on this machine so my other devices can reach Permagent: '
                  + 'check if it is installed, install it if not, bring it up (open the login '
                  + 'page for me in the browser when it appears), then tell me my MagicDNS name '
                  + 'and confirm the Devices pairing page shows it.'
                ).catch(() => {});
                navigateToTool('chat');
              }}
            >Have {agentName} set it up</Button>
          )}
        </Row>
        <Row
          label="Hub address"
          hint={access?.serve_url
            ? 'Using the detected tailnet address — the pairing URL is built from it, so there is nothing to type here.'
            : "Your machine's Tailscale MagicDNS name (auto-filled when the tailnet is detected)."}
        >
          {/* With tailnet access on, remoteBase prefers access.serve_url and
              this input is inert — disable it and say why, instead of letting
              edits silently do nothing. */}
          <TextInput
            value={access?.serve_url ?? host}
            onChange={setHost}
            placeholder="my-mac.tailnet-name.ts.net"
            disabled={!!access?.serve_url}
          />
        </Row>
        <Row label="Device name" hint="Name the device you are pairing — this is how it appears in the registry above.">
          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            <TextInput value={pairName} onChange={setPairName} placeholder="e.g. iPhone" />
            <Button
              colors={colors}
              style={ghost(colors)}
              onClick={() => {
                const name = pairName.trim();
                if (!name) { setPairError('Give the device a name first.'); return false; }
                return api.pairDevice(name)
                  .then(r => { setClaim({ code: r.claim_code, expiresAt: r.expires_at }); setPairError(null); return true; })
                  .catch(e => { setClaim(null); setPairError(e instanceof Error ? e.message : 'Pairing failed'); return false; });
              }}
            >Create pairing link</Button>
          </div>
        </Row>
        {pairError && (
          <div style={{ fontSize: textSize.caption, color: colors.danger, padding: '2px 0 6px' }}>{pairError}</div>
        )}
        <Row label="Pairing URL" hint={claim
          ? `Open this on the new device's browser. One-time: it goes inert after first use, and expires ${new Date(claim.expiresAt).toLocaleTimeString()}.`
          : 'Name the device and create a link — the URL carries a one-time claim code, not a token.'}>
          {pairingUrl ? (
            // Scanning is the point: pairing a phone by retyping a MagicDNS
            // hostname and a 16-character claim code is miserable. The QR was
            // originally squeezed in beside the URL at 112px, which read as a
            // decoration and is small for a phone to acquire — it leads now,
            // at a size that scans from a comfortable distance.
            <div style={{ display: 'flex', alignItems: 'flex-start', gap: 16, minWidth: 0 }}>
              <div style={{ flexShrink: 0, textAlign: 'center' }}>
                <PairingQrCode value={pairingUrl} size={196} />
                <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: 6 }}>
                  Scan with your iPhone
                </div>
              </div>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 8, minWidth: 0, paddingTop: 2 }}>
                <span style={{ fontSize: textSize.micro, color: colors.textDim }}>
                  Open Permagent on the phone and scan this. No typing.
                </span>
                <code style={{
                  fontFamily: font.mono, fontSize: 10, color: colors.cyan,
                  background: colors.bgDeeper, padding: '6px 8px', borderRadius: radius.sm,
                  overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', maxWidth: 320,
                }}>{pairingUrl}</code>
                <Button
                  colors={colors}
                  // The label already flips to "Copied ✓" for 1.6s and owns the
                  // confirmation; the primitive's tick on top would say it twice.
                  flashSuccess={false}
                  style={{ ...ghost(colors), alignSelf: 'flex-start' }}
                  onClick={() => {
                    navigator.clipboard.writeText(pairingUrl).then(() => {
                      setCopied(true);
                      setTimeout(() => setCopied(false), 1600);
                      // Copying the pairing URL is deliberate engagement with the
                      // Devices feature — but it is *intent*, not a completed
                      // pairing, so this stays Ephemeral (never a Brain memory).
                      // The real `devices_paired` signal is emitted by the new
                      // device itself when it claims the code and receives its
                      // own token (see exchangeClaim in lib/api.ts). No secret
                      // in the payload.
                      emitActivity('pairing_link_copied', 'settings');
                    });
                  }}
                >{copied ? 'Copied ✓' : 'Copy link instead'}</Button>
              </div>
            </div>
          ) : (
            <span style={{ fontSize: textSize.caption, color: colors.textDim }}>
              No active pairing link — name the device above and create one; the
              QR code to scan appears here.
            </span>
          )}
        </Row>
        <Row label="Security" hint="The URL carries a one-time claim code — the new device swaps it for its own key on first load, so the link stops being a secret after one use. Each device's key can be revoked above without touching the others.">
          <span style={{ fontSize: textSize.caption, color: colors.textMuted }}>Links are single-use and expire in 10 minutes.</span>
        </Row>
      </Section>
    </div>
  );
}

// ── Main Settings View ───────────────────────────────────────────────

export function SettingsView() {
  const setActivePanel = useCommandCenter(s => s.setActivePanel);
  const pendingSettingsSection = useCommandCenter(s => s.pendingSettingsSection);
  const setPendingSettingsSection = useCommandCenter(s => s.setPendingSettingsSection);
  const [section, setSection] = useState<string>(() => resolveSettingsSection(pendingSettingsSection));

  // #6 download feedback: the Inbox nav entry carries an unread count of
  // 'download' notifications (files landed via `inbox_file_received`), fed by
  // the same notification stream the toast/tray read — not a second poll.
  const { items: notificationItems } = useNotifications();
  const downloadUnread = notificationItems.filter(n => n.kind === 'download' && !n.read).length;

  // Honor an agent/voice deep-link (Settings → <pane>): when the store carries a
  // pending section, jump to that pane and consume it so it only fires once.
  useEffect(() => {
    if (pendingSettingsSection) {
      setSection(resolveSettingsSection(pendingSettingsSection));
      setPendingSettingsSection(null);
    }
  }, [pendingSettingsSection, setPendingSettingsSection]);

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
                // A nav rail with no hover and no press was the last surface in
                // Settings where pointing at a row and pressing it looked the
                // same. `justifyContent` and the icon's own wrapper hold the
                // left-aligned row shape the primitive would otherwise centre.
                <Button
                  key={it.key}
                  colors={colors}
                  flashSuccess={false}
                  onClick={() => setSection(it.key)}
                  style={{
                    '--pa-btn-bg': on ? colors.cyanSoft : 'transparent',
                    '--pa-btn-fg': on ? colors.cyan : colors.textMuted,
                    '--pa-btn-border': on ? colors.borderHi : 'transparent',
                    '--pa-btn-border-hover': on ? colors.borderHi : 'transparent',
                    '--pa-btn-bg-hover': on ? colors.cyanSoft : colors.surfaceHi,
                    '--pa-btn-fg-hover': on ? colors.cyan : colors.text,
                    '--pa-btn-bg-active': on ? colors.cyanGlow : colors.surface,
                    '--pa-btn-pad': '8px 10px',
                    '--pa-btn-radius': `${radius.md}px`,
                    '--pa-btn-weight': on ? 600 : 500,
                    width: '100%', justifyContent: 'flex-start', textAlign: 'left',
                    fontFamily: font.body, fontSize: textSize.small,
                  } as CSSProperties}
                >
                  <span style={{ display: 'inline-flex', alignItems: 'center', gap: 10, width: '100%' }}>
                    <it.icon size={14} />
                    {it.label}
                    {it.key === 'inbox' && downloadUnread > 0 && (
                      <span style={{
                        marginLeft: 'auto', minWidth: 16, height: 16, padding: '0 4px', borderRadius: 999,
                        background: colors.cyan, color: colors.textOnCyan, fontSize: 10, fontWeight: 700,
                        display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
                      }}>{downloadUnread > 9 ? '9+' : downloadUnread}</span>
                    )}
                  </span>
                </Button>

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
