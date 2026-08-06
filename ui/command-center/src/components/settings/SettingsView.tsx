import { useState, useEffect, useCallback, useRef } from 'react';
import { useCommandCenter, navigateToTool } from '../../lib/store';
import { emitActivity } from '../../lib/emitActivity';
import { api, apiFetch, type SovereigntyStatus, type EgressLogEntry, type DeviceInfo, type CrashExportResponse } from '../../lib/api';
import { relativeTimeAgo } from '../../lib/time-decay';
import { font, ease, setTheme as setThemeFn, setMobiusGlow, setIdleAnim, setShowHeroMobius, setDensity as setDensityFn, setReduceMotion as setReduceMotionFn, type ThemePref, type IdleAnim, type UIDensity } from '../../styles/tokens';
import { useTheme as useThemeHook } from '../../styles/useTheme';
import { Mobius } from '../mobius/Mobius';
import {
  getNotificationPrefs, setNotificationPref, getOsNotificationsEnabled,
  setOsNotificationsEnabled, KIND_LABELS, type NotificationKind,
} from '../../lib/notifications';
import { ProvidersSection } from './ProvidersSection';
import { SearchToolsSection } from './SearchToolsSection';
import { usePersona } from './useSettings';
import { resolveSettingsSection } from './sections';
import { trustEnvOverrideNotice } from './autonomy';
import { VoicePicker } from '../voice/VoicePicker';
import { H1, Section, Row, TextInput, Chip, Toggle, Slider, Kbd, SaveButton, StatRow } from './atoms';
import { makeQrMatrix } from '../../lib/qrMatrix';
import { SessionsList } from '../sessions/SessionsList';
import { InboxPanel } from '../inbox/InboxPanel';
import { ExecutionTrace } from '../trace/ExecutionTrace';
import { SpendPanel } from './SpendPanel';
import { timeAgo } from './format';
import { useDecisions } from '../dashboard/decisions/useDecisions';
import { DecisionInbox } from '../dashboard/decisions/DecisionInbox';
import { formatAge } from '../dashboard/decisions/format';
import { getOpenOnLaunch, setOpenOnLaunch, OPEN_ON_LAUNCH_OPTIONS, type OpenOnLaunch } from '../../lib/openOnLaunch';
import type { WorkerInfo } from '../../lib/api';

// The PreviewBadge/PreviewNotice machinery (2026-07-10 audit) is gone: every
// preview-only control has been either wired to real state or removed
// (2026-08 finish-the-settings ruling). Controls that exist here do things.

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
    { key: 'preferences', label: 'Preferences',      icon: 'M3 6h18M6 12h12M10 18h4' },
  ]},
  { group: 'Agent', items: [
    { key: 'agent',       label: 'Persona',          icon: 'M12 2a4 4 0 014 4v3a4 4 0 11-8 0V6a4 4 0 014-4zM4 21v-2a6 6 0 016-6h4a6 6 0 016 6v2' },
    { key: 'memory',      label: 'Memory',           icon: 'M9 4a4 4 0 00-4 4 3 3 0 00-1 5.5A3 3 0 005 18a4 4 0 004 3M15 4a4 4 0 014 4 3 3 0 011 5.5A3 3 0 0119 18a4 4 0 01-4 3' },
    { key: 'autonomy',    label: 'Autonomy & guardrails', icon: 'M12 2l9 4v6c0 5-4 9-9 10-5-1-9-5-9-10V6l9-4z' },
  ]},
  // The former Console overlay (Sessions / Inbox / Trace / Governance) folded
  // into Settings — 2026-08 ruling. Governance's panels merged into Spend,
  // Sovereignty, Models, and Autonomy.
  { group: 'Console', items: [
    { key: 'sessions',    label: 'Sessions',         icon: 'M3 12a9 9 0 109-9 9.75 9.75 0 00-6.74 2.74L3 8M3 3v5h5M12 7v5l4 2' },
    { key: 'inbox',       label: 'Inbox',            icon: 'M22 12h-6l-2 3h-4l-2-3H2M5.45 5.11L2 12v6a2 2 0 002 2h16a2 2 0 002-2v-6l-3.45-6.89A2 2 0 0016.76 4H7.24a2 2 0 00-1.79 1.11z' },
    { key: 'activity',    label: 'Activity',         icon: 'M22 12h-4l-3 9L9 3l-3 9H2' },
    { key: 'spend',       label: 'Spend',            icon: 'M12 1v22M17 5H9.5a3.5 3.5 0 000 7h5a3.5 3.5 0 010 7H6' },
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
    { key: 'sovereignty', label: 'Sovereignty',      icon: 'M7 11V7a5 5 0 0110 0v4M5 11h14v9H5zM12 15v2' },
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
        {/* Failed initial load: offer a retry instead of leaving a form that
            reads as broken (#167). Saving still works from the form's values. */}
        {error === 'Failed to load persona' && !data && (
          <button
            onClick={() => { void reload(); }}
            style={{
              padding: '6px 14px', borderRadius: 8, cursor: 'pointer',
              background: 'transparent', border: `1px solid ${colors.border}`,
              color: colors.textMuted, fontSize: 12, fontFamily: font.body,
            }}
          >
            Retry
          </button>
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
        <Toggle on={osOn} onChange={async v => setOsOn(await setOsNotificationsEnabled(v))} />
      </Row>
    </Section>
  );
}

export function MemoryPanel({ goto }: { goto?: (key: string) => void }) {
  const { colors } = useThemeHook();
  // The preview "memory budget" sliders and "what to remember" toggles were
  // removed (2026-08 finish-the-settings ruling): no backing subsystem reads
  // them. What remains is real: the Brain view, and the Librarian's nightly
  // pruning setting (in Models), which is the live retention control.
  return (
    <div>
      <H1 sub="What your agent remembers about you, your projects, and the people in your world.">Memory</H1>
      <Section title="Manage">
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
          <button style={ghost(colors)} onClick={() => navigateToTool('memory')}>Open Brain view</button>
          <button style={ghost(colors)} onClick={() => goto?.('models')}>Nightly pruning (Librarian schedule) →</button>
          {/* Export/Forget removed (2026-07-10 audit): a destructive-styled
              button with no handler is worse than no button. They return
              with real endpoints behind them. */}
        </div>
        <div style={{ fontSize: 12, color: colors.textMuted, marginTop: 12, lineHeight: 1.5 }}>
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

/** Compact pending-approvals strip (Settings → Autonomy). Reuses the shared
 *  Decision-Inbox data hook + overlay — never forks that surface. Replaces the
 *  old Governance → Approvals panel; its "posture" card is gone because
 *  Autonomy IS the writer of that mode. */
export function ApprovalsStrip() {
  const { colors } = useThemeHook();
  const inbox = useDecisions();
  const { data } = inbox;
  const [open, setOpen] = useState(false);
  const pending = data?.total_pending ?? 0;
  const oldest = data?.oldest_pending_at ?? null;
  return (
    <>
      <div style={{
        display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap',
        padding: '9px 12px', marginBottom: 12, borderRadius: 8,
        background: colors.bgDeeper, border: `1px solid ${colors.border}`,
      }}>
        <span style={{ fontSize: 12, color: pending > 0 ? colors.text : colors.textMuted }}>
          {data === null
            ? 'Checking the Decision Inbox…'
            : `Pending approvals: ${pending}`}
          {data !== null && pending > 0 && oldest && (
            <span style={{ color: colors.textDim }}> · oldest {formatAge(oldest)}</span>
          )}
        </span>
        <div style={{ flex: 1 }} />
        <button style={ghost(colors)} onClick={() => setOpen(true)}>Open Decision Inbox →</button>
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
  }, []);
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
      <Section title="Default autonomy" sub="Live — this writes the daemon's tool-approval mode (GOOSE_MODE) and applies to new turns.">
        {trustError && (
          <div style={{ fontSize: 12, color: colors.danger, padding: '4px 0 8px' }}>{trustError}</div>
        )}
        <ApprovalsStrip />
        {(() => {
          // Env-override honesty (re-enable-gate epic part B): with GOOSE_MODE
          // set in the daemon's environment, these buttons write YAML the env
          // silently wins over. Say so instead of highlighting a mode the
          // daemon isn't running.
          const envNotice = trustEnvOverrideNotice(effectiveTrust, trust);
          return envNotice ? (
            <div style={{ marginBottom: 10, padding: '10px 14px', borderRadius: 10, background: `${colors.warning}1A`, border: `1px solid ${colors.warning}55`, color: colors.text, fontSize: 12, lineHeight: 1.5 }}>
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
                    <span style={{ fontSize: 13, fontWeight: 600, color: current ? colors.cyan : colors.text }}>{opt.l}</span>
                    {locked && (
                      <span style={{ fontSize: 10, fontWeight: 600, letterSpacing: '0.05em', textTransform: 'uppercase', color: colors.textMuted, border: `1px solid ${colors.border}`, borderRadius: 999, padding: '1px 6px' }}>Soon</span>
                    )}
                  </div>
                  <div style={{ fontSize: 11, color: colors.textMuted }}>{opt.d}</div>
                </button>
              );
            })}
          </div>
        </Row>
        <div style={{ fontSize: 12, color: colors.textMuted, marginTop: 10, lineHeight: 1.5 }}>
          Per-tool approval (Ask every time / Smart approve) is temporarily
          locked here while the approval pipeline is hardened. Approval prompts
          already land in the <strong>Decision Inbox</strong> on your Dashboard —
          these modes become selectable once the re-enable gate ships.
        </div>
        {trust !== null && !SELECTABLE_TRUST_MODES.has(trust) && (
          <div style={{ marginTop: 10, padding: '10px 14px', borderRadius: 10, background: `${colors.warning}1A`, border: `1px solid ${colors.warning}55`, color: colors.text, fontSize: 12, lineHeight: 1.5 }}>
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
        <button style={ghost(colors)} onClick={() => goto?.('spend')}>Set spend caps in Spend →</button>
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

  useEffect(() => {
    api.getExtensions()
      // Never hand a non-array to the render path: `.filter` on undefined is
      // the same class of crash this panel just had.
      .then(r => { setExtensions(Array.isArray(r?.extensions) ? r.extensions : []); setLoading(false); })
      .catch(() => setLoading(false));
  }, []);

  const enabledCount = extensions.filter(e => e.enabled).length;
  // stdio servers that declare required env vars are the ones with API keys —
  // the panel points at where those are actually managed.
  const needKeys = extensions.filter(e => (e.env_keys?.length ?? 0) > 0);

  return (
    <div>
      <H1 sub="Tools your agent can use. These follow the Model Context Protocol — connect a server and the agent can call into it.">Tools &amp; MCPs</H1>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 16 }}>
        <div style={{ flex: 1 }} />
        <span style={{ fontSize: 12, color: colors.textMuted }}>{enabledCount} of {extensions.length} enabled</span>
      </div>
      {needKeys.length > 0 && (
        // API keys are managed in Search & tools, not here. Without this the
        // only way to find that out is to guess — which is how someone ends up
        // on this tab looking for their Brave key.
        <div style={{
          display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap',
          padding: '9px 12px', marginBottom: 14, borderRadius: 8,
          background: colors.bgDeeper, border: `1px solid ${colors.border}`,
        }}>
          <span style={{ fontSize: 12, color: colors.textMuted }}>
            {needKeys.map(extensionLabel).join(' and ')} need API keys.
          </span>
          <button style={ghost(colors)} onClick={() => goto('search')}>
            Manage keys in Search &amp; tools
          </button>
        </div>
      )}
      {loading ? (
        <div style={{ color: colors.textDim, fontSize: 13 }}>Loading extensions...</div>
      ) : extensions.length === 0 ? (
        <Section title="No extensions"><div style={{ color: colors.textMuted, fontSize: 13 }}>No MCP tools or extensions configured.</div></Section>
      ) : (
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(2, 1fr)', gap: 10 }}>
          {extensions.map((ext, i) => (
            <div key={ext.name || `ext-${i}`} style={{ display: 'flex', alignItems: 'center', gap: 14, padding: 14, borderRadius: 10, background: colors.bgDeeper, border: `1px solid ${colors.border}` }}>
              <div style={{ width: 32, height: 32, borderRadius: 8, background: ext.enabled ? colors.cyanSoft : colors.surfaceHi, border: `1px solid ${ext.enabled ? colors.borderHi : colors.border}`, display: 'grid', placeItems: 'center', fontFamily: font.display, fontSize: 13, fontWeight: 700, color: ext.enabled ? colors.cyan : colors.textMuted, flexShrink: 0 }}>{extensionLabel(ext).charAt(0).toUpperCase() || '?'}</div>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 13, fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{extensionLabel(ext)}</div>
                <div style={{ fontSize: 11, color: colors.textMuted, marginTop: 2, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
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
    running: { bg: colors.cyanSoft, text: colors.cyan, label: 'Loaded' },
    installed: { bg: colors.surfaceHi, text: colors.textMuted, label: 'Installed' },
    missing: { bg: `${colors.danger}1A`, text: colors.danger, label: 'Not installed' },
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
  const [libError, setLibError] = useState<string | null>(null);

  // Primary-model readout + worker roster (merged from the retired Governance
  // → Models panel; GET /api/agent/workers is its unique surface). Read-only:
  // the model/provider switch itself lives in the provider modal on API keys.
  const [primary, setPrimary] = useState<{ model: string | null; provider: string | null; mode: string | null } | null>(null);
  const [workers, setWorkers] = useState<WorkerInfo[] | null>(null);
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
    api.getWorkers()
      .then(ws => { if (active) setWorkers(Object.values(ws)); })
      .catch(() => { if (active) setWorkers([]); });
    return () => { active = false; };
  }, []);

  // Strix — the security sweep loop (crate::strix). The daemon re-reads
  // `strix_enabled` every tick, so a flip here takes effect at the next tick
  // without a restart.
  const [strix, setStrix] = useState<boolean | null>(null);
  const [strixError, setStrixError] = useState<string | null>(null);
  useEffect(() => {
    let active = true;
    api.readConfig('strix_enabled')
      .then(r => { if (active) setStrix(!!(r && (r as { value?: unknown }).value === true)); })
      .catch(() => { if (active) setStrix(false); });
    return () => { active = false; };
  }, []);
  const saveStrix = (v: boolean) => {
    const prev = strix;
    setStrix(v);
    setStrixError(null);
    api.upsertConfig('strix_enabled', v).catch(err => {
      setStrix(prev);
      setStrixError(`Couldn't save: ${err instanceof Error ? err.message : String(err)}`);
    });
  };

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
    const prev = schedule;
    const next = { ...schedule, ...patch };
    setSchedule(next);
    setSaving(true);
    setLibError(null);
    try {
      await api.setLibrarianSchedule(next);
    } catch (err) {
      // Revert + surface (2026-07 wiring audit): the swallowed catch left the
      // panel showing a schedule the daemon never persisted.
      setSchedule(prev);
      setLibError(`Couldn't save the Librarian schedule: ${err instanceof Error ? err.message : String(err)}`);
    }
    setSaving(false);
  };

  const handleRunNow = async () => {
    setRunningNow(true);
    setLibError(null);
    try {
      await api.runLibrarianNow();
    } catch (err) {
      setLibError(`Couldn't start the Librarian: ${err instanceof Error ? err.message : String(err)}`);
    }
    setRunningNow(false);
    // Refresh status to show model as loaded
    api.getOllamaStatus().then(setOllama).catch(() => {});
  };

  return (
    <div>
      <H1 sub="Pick the brains behind the agent. Use stronger models when stakes are high; cheaper for routine work.">Models</H1>
      <Section title="Providers" sub="Provider credentials live in the API keys tab — add or update a key there, then route to it below.">
        {/* One-line primary readout (condensed from Governance → Models; the
            full editor is redundant with the provider modal on API keys). */}
        <div style={{ fontSize: 13, color: colors.text, fontFamily: font.mono, marginBottom: 12 }}>
          {primary === null
            ? 'Loading primary model…'
            : `${primary.model ?? '—'} · provider: ${primary.provider ?? 'default'}${primary.mode ? ` · mode: ${primary.mode}` : ''}`}
        </div>
        <button style={ghost(colors)} onClick={() => goto('keys')}>Manage API keys</button>
      </Section>
      {/* The old Routing/Behavior selects were decorative — hardcoded options
          wired to nothing (2026-07-10 settings audit). The real model/default
          switch lives in the provider modal on the API keys tab. */}

      {/* ── Worker roster (GET /api/agent/workers) ───────────────── */}
      <Section title="Worker roster" sub="The models each role can dispatch to, with live availability.">
        {workers === null ? (
          <div style={{ color: colors.textDim, fontSize: 13 }}>Loading roster…</div>
        ) : workers.length === 0 ? (
          <div style={{ color: colors.textMuted, fontSize: 13 }}>
            No workers configured. The primary model handles every role.
          </div>
        ) : (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            {workers.map(w => (
              <StatRow
                key={w.key}
                left={w.display_name}
                sub={`${w.role} · ${w.engine}`}
                right={
                  <span style={{
                    fontSize: 11, fontWeight: 600, padding: '2px 8px', borderRadius: 999,
                    border: `1px solid ${colors.border}`,
                    color: w.available ? colors.success : colors.textDim,
                  }} title={w.reason ?? undefined}>
                    {w.available ? 'available' : 'unavailable'}
                  </span>
                }
              />
            ))}
          </div>
        )}
      </Section>

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
          {libError && (
            <div style={{ fontSize: 12, color: colors.danger, padding: '4px 0 8px' }}>{libError}</div>
          )}
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
                background: colors.cyanSoft,
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

      {/* ── Strix security sweeps ────────────────────────────────── */}
      <Section
        title="Security sweeps (Strix)"
        sub="Strix probes your own projects for security flaws and files findings on each project's Overview. Requires the external `strix` scanner and Docker installed. Sweeps every 6 hours; a change here takes effect at the next tick — no restart needed."
      >
        {strixError && (
          <div style={{ fontSize: 12, color: colors.danger, padding: '4px 0 8px' }}>{strixError}</div>
        )}
        <Row label="Enable Strix" hint="Off by default — a scanner that runs live exploit tooling is switched on deliberately, never by upgrade.">
          {strix === null ? (
            <span style={{ fontSize: 12, color: colors.textDim }}>Loading…</span>
          ) : (
            <Toggle on={strix} onChange={saveStrix} />
          )}
        </Row>
      </Section>
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
                  padding: 4, borderRadius: 12, cursor: 'pointer', outline: 'none',
                  border: on ? `2px solid ${colors.cyan}` : '2px solid transparent',
                  boxShadow: on ? `0 0 14px ${colors.cyanGlow}` : 'none',
                }}
                onFocus={e => { if (!on) e.currentTarget.style.borderColor = colors.borderHi; }}
                onBlur={e => { if (!on) e.currentTarget.style.borderColor = 'transparent'; }}
              >
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
              <span style={{ fontSize: 13, flex: 1 }}>{l}</span>
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

  const saveAnalytics = useCallback((v: boolean) => {
    const generation = ++analyticsGeneration.current;
    setConsentError(null);
    const prev = analytics;
    setAnalytics(v); // optimistic
    api.setAnalyticsConsent(v)
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
    api.exportCrashReport()
      .then(setExportResult)
      .catch(err => setExportError(`Export failed: ${err instanceof Error ? err.message : String(err)}`))
      .finally(() => setExporting(false));
  }, []);

  return (
    <div>
      <H1 sub="Your data is yours. Everything is local-first today.">Data &amp; privacy</H1>
      <Section title="Local-first">
        <div style={{ fontSize: 13, color: colors.textMuted, lineHeight: 1.6, display: 'flex', alignItems: 'center', gap: 12, flexWrap: 'wrap' }}>
          <span>
            Memory and traces live on this machine. To make the boundary
            enforced — blocking every cloud inference call — use Sovereignty.
          </span>
          <button style={ghost(colors)} onClick={() => goto?.('sovereignty')}>Open Sovereignty →</button>
        </div>
      </Section>
      <Section title="Diagnostics" sub="Live — an off-by-default opt-in written to the daemon's consent gate.">
        <Row label="Share product analytics" hint="Anonymous usage and timing. Never your prompts."><Toggle on={!!analytics} onChange={saveAnalytics} /></Row>
        {consentError && (
          <div style={{ fontSize: 12, color: colors.danger, padding: '2px 0 8px' }}>{consentError}</div>
        )}
      </Section>
      <Section title="Crash report" sub="Export a redacted crash report to attach to a support message. Written locally — home paths, keys, tokens, emails, and UUIDs are redacted first. Nothing is uploaded.">
        <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap', alignItems: 'center' }}>
          <button
            onClick={runExport}
            disabled={exporting}
            style={{
              fontSize: 12, padding: '6px 12px', borderRadius: 6,
              cursor: exporting ? 'default' : 'pointer', opacity: exporting ? 0.6 : 1,
              background: colors.surfaceHi, color: colors.text, border: `1px solid ${colors.border}`,
            }}
          >{exporting ? 'Exporting…' : 'Export redacted crash report'}</button>
        </div>
        {exportError && (
          <div style={{ fontSize: 12, color: colors.danger, padding: '6px 0' }}>{exportError}</div>
        )}
        {exportResult && (
          <div style={{ padding: '8px 0' }}>
            <div style={{ fontSize: 12, color: colors.textDim }}>
              {exportResult.reportCount === 0
                ? 'No crash reports captured. Saved an empty redacted bundle to:'
                : `${exportResult.reportCount} crash report(s) redacted and saved to:`}
            </div>
            <div style={{ fontSize: 12, color: colors.text, fontFamily: font.mono, wordBreak: 'break-all', padding: '2px 0 6px' }}>{exportResult.path}</div>
            <div style={{ fontSize: 11, color: colors.textDim, paddingBottom: 4 }}>Preview (exactly what would be shared):</div>
            <pre style={{
              fontSize: 11, fontFamily: font.mono, color: colors.text, background: colors.surface,
              border: `1px solid ${colors.border}`, borderRadius: 6, padding: 8, margin: 0,
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

  const refreshLog = useCallback(() => {
    api.getEgressLog(100).then(setLog).catch(() => setLog([]));
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
    api.setSovereignty(patch)
      .then(status => { setStatus(status); refreshLog(); })
      .catch(err => setError(`Couldn't save: ${err instanceof Error ? err.message : String(err)}`));
  };

  return (
    <div>
      <H1 sub="Make the data boundary real. With sovereign mode on, every model call stays on this machine — cloud providers are refused (fail-closed), not just deprioritized.">Sovereignty</H1>

      {error && (
        <div style={{ fontSize: 12, color: colors.danger, padding: '4px 0 8px' }}>{error}</div>
      )}

      <Section title="Sovereign mode" sub="Live — writes the daemon's global sovereign flag, enforced at the provider choke point for every session.">
        <Row
          label="Data boundary"
          hint={status?.enabled
            ? 'All cloud inference is blocked before any data leaves this machine. Only local models run.'
            : 'Cloud inference is allowed. Every cloud call is still recorded in the audit log below.'}
        >
          <button
            onClick={() => save({ enabled: !status?.enabled })}
            disabled={status === null}
            style={{
              height: 32, padding: '0 18px', borderRadius: 8,
              background: status?.enabled ? 'transparent' : colors.cyan,
              border: status?.enabled ? `1px solid ${colors.borderHi}` : 'none',
              color: status?.enabled ? colors.cyan : colors.textOnCyan,
              cursor: status === null ? 'default' : 'pointer',
              fontFamily: font.body, fontSize: 12, fontWeight: 600,
            }}
          >
            {status === null ? '…' : status.enabled ? 'Allow cloud again' : 'Pull the cable'}
          </button>
        </Row>
        {status?.enabled && !status.localProviderAvailable && (
          <div style={{ fontSize: 12, color: colors.warning, padding: '2px 0 8px' }}>
            No local provider (Ollama or local-inference) is registered — with sovereign mode on, inference will be refused until one is available.
          </div>
        )}
        <Row label="Capture full prompts in the audit log" hint="Off by default — only a SHA-256 hash is stored. On records the full prompt text locally.">
          <Toggle on={!!status?.capturePrompts} onChange={v => save({ capturePrompts: v })} />
        </Row>
      </Section>

      <Section title="Egress audit" sub="Every cloud call, allowed or blocked — newest first. BLOCKED means sovereign mode refused it before anything left this machine.">
        <Row label="Cloud inference calls" hint={`${log?.length ?? 0} recorded`}>
          <button
            onClick={refreshLog}
            style={{
              fontSize: 12, padding: '4px 10px', borderRadius: 6, cursor: 'pointer',
              background: colors.surfaceHi, color: colors.text, border: `1px solid ${colors.border}`,
            }}
          >Refresh</button>
        </Row>
        {log === null ? (
          <div style={{ fontSize: 12, color: colors.textDim, padding: '6px 0' }}>Loading audit log…</div>
        ) : log.length === 0 ? (
          <div style={{ fontSize: 12, color: colors.textDim, padding: '6px 0' }}>
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
                  <div key={e.id} style={{ display: 'grid', gridTemplateColumns: EGRESS_COLS, gap: 12, alignItems: 'center', padding: '8px 10px', borderRadius: 8, background: colors.bgDeeper, border: `1px solid ${colors.border}` }}>
                    <div style={{ fontSize: 12, color: colors.textMuted, fontFamily: font.mono }} title={`${new Date(e.ts).toLocaleString()}${e.sessionId ? ' · ' + e.sessionId : ''} · ${e.contentHash.slice(0, 12)}…`}>
                      {timeAgo(e.ts) || e.ts}
                    </div>
                    <div style={{ fontSize: 12, color: colors.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={`${e.provider} · ${e.model}`}>
                      <span style={{ color: colors.textMuted }}>{e.provider}</span> · {e.model}
                    </div>
                    <div style={{ fontSize: 12, color: colors.textMuted }}>{e.kind}</div>
                    <div>
                      <span style={{
                        fontSize: 10, fontWeight: 700, letterSpacing: '0.04em', textTransform: 'uppercase',
                        padding: '2px 8px', borderRadius: 999, border: `1px solid ${colors.border}`,
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
      <div style={{ flex: 1, minHeight: 320, border: `1px solid ${colors.border}`, borderRadius: 12, overflow: 'hidden' }}>
        <SessionsList />
      </div>
    </div>
  );
}

function InboxPane() {
  const { colors } = useThemeHook();
  return (
    <div style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
      <H1 sub="Files you download in the in-app browser land here — send them to the Brain, a project, or the post scheduler. You choose; nothing is routed for you.">Inbox</H1>
      <div style={{ flex: 1, minHeight: 320, border: `1px solid ${colors.border}`, borderRadius: 12, overflow: 'hidden' }}>
        <InboxPanel embedded />
      </div>
    </div>
  );
}

function ActivityPane() {
  const { colors } = useThemeHook();
  return (
    <div style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
      <H1 sub="The runtime's most recent events, live off the running system's event streams — tool calls, worker activity, navigations, and lifecycle signals as they happen.">Activity</H1>
      <div style={{ flex: 1, minHeight: 320, border: `1px solid ${colors.border}`, borderRadius: 12, overflow: 'hidden' }}>
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

const PANELS: Record<string, (props: PanelProps) => JSX.Element> = {
  agent: PersonaPanel, preferences: PreferencesPanel,
  memory: MemoryPanel, autonomy: AutonomyPanel, tools: ToolsPanel,
  models: ModelsPanel, keys: KeysPanel, devices: DevicesPanel, search: SearchPanel,
  appearance: AppearancePanel, shortcuts: ShortcutsPanel, data: DataPanel,
  sovereignty: SovereigntyPanel,
  sessions: SessionsPane, inbox: InboxPane, activity: ActivityPane, spend: SpendPane,
};


function PairingQrCode({ value, size = 112 }: { value: string; size?: number }) {
  let matrix: boolean[][];
  try {
    matrix = makeQrMatrix(value);
  } catch {
    return <span style={{ fontSize: 11 }}>QR unavailable — shorten the hub address or copy the link.</span>;
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
      style={{ display: 'block', background: '#fff', borderRadius: 8 }}
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
  // itself — the user types nothing (Jesse's zero-strain rule, 2026-07-11).
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

      {/* Role clarity (Jesse's rule 2026-07-11): friendly first, deeper on ask. */}
      <Section title={isHub ? 'This device is your hub' : 'This device is a companion'}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 8 }}>
          <span style={{
            width: 8, height: 8, borderRadius: '50%',
            background: hubUp === false ? colors.danger : colors.cyan,
            boxShadow: hubUp === false ? 'none' : `0 0 8px ${colors.cyan}`,
          }} />
          <span style={{ fontSize: 13, color: colors.text }}>
            {isHub
              ? 'Everything lives here — your memories, projects, and models. Keep this machine on so your other devices can reach Permagent.'
              : hubUp === false
                ? 'The hub is not answering — make sure it is awake and on the tailnet.'
                : 'You are connected to your hub. Everything you see lives there, not on this device.'}
          </span>
        </div>
        {detail < 2 && (
          <button style={ghost(colors)} onClick={() => setDetail(d => d + 1)}>
            {detail === 0 ? 'Tell me more' : 'How does it work exactly?'}
          </button>
        )}
        {detail >= 1 && (
          <p style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.6, margin: '10px 0 0' }}>
            Permagent works like a home base with visitors: the hub is the one machine that runs
            the Permagent daemon and stores every memory, project, and model. Phones, laptops,
            and tablets are companions — they show you everything and let you act from anywhere,
            but they keep nothing except their key to the hub. If the hub is asleep or offline,
            companions can't reach Permagent until it's back.
          </p>
        )}
        {detail >= 2 && (
          <p style={{ fontSize: 12, color: colors.textDim, lineHeight: 1.6, margin: '10px 0 0' }}>
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
          <div style={{ fontSize: 12, color: colors.textDim, padding: '6px 0' }}>
            Device list unavailable — is the daemon reachable?
          </div>
        )}
        {devices?.length === 0 && (
          <div style={{ fontSize: 12, color: colors.textDim, padding: '6px 0' }}>
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
                <button
                  style={ghost(colors)}
                  onClick={() => {
                    const name = editName.trim();
                    if (!name) return;
                    api.renameDevice(d.id, name)
                      .then(() => { setEditingId(null); loadDevices(); })
                      .catch(() => setEditingId(null));
                  }}
                >Save</button>
                <button style={ghost(colors)} onClick={() => setEditingId(null)}>Cancel</button>
              </div>
            ) : (
              <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                {d.revoked ? (
                  <span style={{ fontSize: 11, fontWeight: 600, color: colors.danger }}>REVOKED</span>
                ) : (
                  <>
                    <button
                      style={ghost(colors)}
                      onClick={() => { setEditingId(d.id); setEditName(d.name); setConfirmRevokeId(null); }}
                    >Rename</button>
                    <button
                      style={{ ...ghost(colors), color: colors.danger, borderColor: `${colors.danger}66` }}
                      title="This device stops authenticating immediately. Pair it again to restore access."
                      onClick={() => {
                        if (confirmRevokeId !== d.id) { setConfirmRevokeId(d.id); return; }
                        api.revokeDevice(d.id)
                          .then(() => { setConfirmRevokeId(null); loadDevices(); })
                          .catch(() => setConfirmRevokeId(null));
                      }}
                    >{confirmRevokeId === d.id ? 'Confirm revoke' : 'Revoke'}</button>
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
            <span style={{ fontSize: 12, color: colors.textMuted }}>
              {accessBusy ? 'Applying…'
                : access?.enabled ? `Live at ${access.serve_url}`
                : 'This machine only'}
            </span>
          </div>
        </Row>
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
            <button
              style={ghost(colors)}
              onClick={() => {
                const name = pairName.trim();
                if (!name) { setPairError('Give the device a name first.'); return; }
                api.pairDevice(name)
                  .then(r => { setClaim({ code: r.claim_code, expiresAt: r.expires_at }); setPairError(null); })
                  .catch(e => { setClaim(null); setPairError(e instanceof Error ? e.message : 'Pairing failed'); });
              }}
            >Create pairing link</button>
          </div>
        </Row>
        {pairError && (
          <div style={{ fontSize: 12, color: colors.danger, padding: '2px 0 6px' }}>{pairError}</div>
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
                <div style={{ fontSize: 11, color: colors.textMuted, marginTop: 6 }}>
                  Scan with your iPhone
                </div>
              </div>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 8, minWidth: 0, paddingTop: 2 }}>
                <span style={{ fontSize: 11, color: colors.textDim }}>
                  Open Permagent on the phone and scan this. No typing.
                </span>
                <code style={{
                  fontFamily: font.mono, fontSize: 10, color: colors.cyan,
                  background: colors.bgDeeper, padding: '6px 8px', borderRadius: 6,
                  overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', maxWidth: 320,
                }}>{pairingUrl}</code>
                <button
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
                >{copied ? 'Copied ✓' : 'Copy link instead'}</button>
              </div>
            </div>
          ) : (
            <span style={{ fontSize: 12, color: colors.textDim }}>
              No active pairing link — name the device above and create one; the
              QR code to scan appears here.
            </span>
          )}
        </Row>
        <Row label="Security" hint="The URL carries a one-time claim code — the new device swaps it for its own key on first load, so the link stops being a secret after one use. Each device's key can be revoked above without touching the others.">
          <span style={{ fontSize: 12, color: colors.textMuted }}>Links are single-use and expire in 10 minutes.</span>
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
                <button key={it.key} onClick={() => setSection(it.key)} style={{
                  display: 'flex', alignItems: 'center', gap: 10, width: '100%', padding: '8px 10px', borderRadius: 8,
                  background: on ? colors.cyanSoft : 'transparent',
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
