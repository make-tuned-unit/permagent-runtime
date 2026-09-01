/**
 * Per-agent settings, at the agent (J8 / C7).
 *
 * The Guard's switch and cadence, the Watcher's teaching keys and the
 * Librarian's schedule all lived under **Models**, because each of them names
 * a model — which is true of nearly everything in this app. The result was
 * that "where do I turn off the Guard" had three right answers and none of
 * them was the Guard's own page.
 *
 * Canonical home is **Agents**: the Guard *is* an agent, and so are the other
 * two. Models keeps only its stated purpose — which brain answers which job —
 * and the places that used to host these now carry an "Open Agents →" link,
 * the redirect convention Settings already uses.
 *
 * Each block owns its own reads and writes, so a block is a drop-in for one
 * agent's detail pane and nothing has to be threaded through the roster.
 */

import { useEffect, useState, type CSSProperties } from 'react';
import { Row, Section, ModelStateBadge, selectStyle } from '../atoms';
import { Button } from '../../common/Button';
import { Toggle } from '../../common/Toggle';
import { font, radius, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { api } from '../../../lib/api';

/**
 * Which agents have a settings block here. Keyed by the agents-API id, which
 * is the id `AgentDetailPane` is routed on.
 */
export const AGENTS_WITH_SETTINGS = new Set(['strix', 'watcher', 'librarian']);

const inputStyle = (colors: ReturnType<typeof useTheme>['colors']): CSSProperties => ({
  width: 260, fontFamily: font.body, fontSize: textSize.caption, color: colors.text,
  background: colors.inputBg, border: `1px solid ${colors.border}`,
  borderRadius: radius.sm, padding: '6px 10px', outline: 'none',
});

// ── The Guard ───────────────────────────────────────────────────────────────

/**
 * The sweep cadence and the scanner host. The on/off switch is NOT here — it
 * is `AgentEnableRow`, one row above, which is every agent's switch: two
 * switches for one `strix_enabled` key on one page would recreate the exact
 * problem this move exists to fix.
 */
export function GuardSweepSettings() {
  const { colors } = useTheme();
  const [hours, setHours] = useState(24);
  const [dockerSsh, setDockerSsh] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    api.readConfig('strix_sweep_hours')
      .then(r => {
        const v = Number(r);
        if (active && Number.isFinite(v) && v > 0) setHours(v);
      })
      .catch(() => { /* unset — daemon default (24h) applies */ });
    api.readConfig('strix_docker_ssh')
      .then(r => { if (active && typeof r === 'string' && r.trim()) setDockerSsh(r.trim()); })
      .catch(() => { /* unset — scans locally */ });
    return () => { active = false; };
  }, []);

  const save = (v: number) => {
    const prev = hours;
    setHours(v);
    setError(null);
    api.upsertConfig('strix_sweep_hours', v).catch(err => {
      setHours(prev);
      setError(`Couldn't save: ${err instanceof Error ? err.message : String(err)}`);
    });
  };

  return (
    <Section
      title="Security sweeps"
      sub="The Guard — born of the Strix pentest engine — probes your own projects for security flaws. Each sweep scans ONE active project (rotating through them, least-recently-scanned first) and files a security report with a fix plan as a note on that project, plus a findings checklist on its Overview. Requires the external `strix` scanner and Docker: locally, or on the host in `strix_docker_ssh` (rsync there, scan against that machine's Docker, pull `.strix` back). A forwarded Docker socket is not enough. The cadence below is a cost dial that applies once the Guard is on: every sweep runs on your API credits. Changes apply within ~15 minutes — no restart needed."
    >
      {error && (
        <div style={{ fontSize: textSize.caption, color: colors.danger, padding: '4px 0 8px' }}>{error}</div>
      )}
      <Row label="Sweep every" hint="How often the Guard scans the next project in the rotation. Daily is the cost-effective default.">
        <select
          value={hours}
          data-testid="guard-sweep-hours"
          onChange={e => save(Number(e.target.value))}
          style={{ ...selectStyle(colors), minWidth: 180, width: 'auto' }}
        >
          <option value={12}>12 hours</option>
          <option value={24}>24 hours (recommended)</option>
          <option value={72}>3 days</option>
          <option value={168}>Weekly</option>
        </select>
      </Row>
      <Row
        label="Scanner host"
        hint={dockerSsh
          ? 'Docker and strix run on this machine. This Mac rsyncs the project there and pulls .strix back — a forwarded Docker socket is not enough.'
          : 'Unset: scans on this Mac, which needs local Docker and strix. Set strix_docker_ssh in ~/.permagent/config.yaml to scan on another host.'}
      >
        <span style={{ fontSize: textSize.caption, color: colors.text, fontFamily: font.mono }}>
          {dockerSsh ?? 'this Mac'}
        </span>
      </Row>
    </Section>
  );
}

// ── The Watcher ─────────────────────────────────────────────────────────────

/** Teachability keys: subjects to follow, subjects never to raise again. */
export function WatcherTopicSettings() {
  const { colors } = useTheme();
  const [topics, setTopics] = useState('');
  const [muted, setMuted] = useState('');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    api.readConfig('watcher_topics')
      .then(r => { if (active && Array.isArray(r)) setTopics((r as string[]).join(', ')); })
      .catch(() => { /* unset — no topics taught yet */ });
    api.readConfig('watcher_muted_subjects')
      .then(r => { if (active && Array.isArray(r)) setMuted((r as string[]).join(', ')); })
      .catch(() => { /* unset — nothing muted */ });
    return () => { active = false; };
  }, []);

  const save = (key: 'watcher_topics' | 'watcher_muted_subjects', raw: string) => {
    setError(null);
    const list = raw.split(',').map(s => s.trim()).filter(Boolean);
    api.upsertConfig(key, list).catch(err => {
      setError(`Couldn't save: ${err instanceof Error ? err.message : String(err)}`);
    });
  };

  return (
    <Section
      title="Proactive nudges"
      sub="The Watcher reaches out at most about once a day with the ONE thing genuinely worth your attention — news grounded in your active projects, or a memory thread that went quiet. Separately, it delivers the Financier's overbought sell signals on stocks you already hold (daily per symbol; does not use that taste budget). Teach it here: topics you want followed, and subjects it should never raise again. Changes apply at its next check — no restart needed."
    >
      {error && (
        <div style={{ fontSize: textSize.caption, color: colors.danger, padding: '4px 0 8px' }}>{error}</div>
      )}
      <Row label="Topics to follow" hint="Comma-separated. Treated as relevant by your say-so, alongside subjects inferred from your active projects.">
        <input
          value={topics}
          data-testid="watcher-topics"
          onChange={e => setTopics(e.target.value)}
          onBlur={() => save('watcher_topics', topics)}
          onKeyDown={e => { if (e.key === 'Enter') save('watcher_topics', topics); }}
          placeholder="e.g. local-first software, prediction markets"
          style={inputStyle(colors)}
        />
      </Row>
      <Row label="Muted subjects" hint="Comma-separated. The Watcher never nudges about these again.">
        <input
          value={muted}
          data-testid="watcher-muted"
          onChange={e => setMuted(e.target.value)}
          onBlur={() => save('watcher_muted_subjects', muted)}
          onKeyDown={e => { if (e.key === 'Enter') save('watcher_muted_subjects', muted); }}
          placeholder="e.g. crypto prices"
          style={inputStyle(colors)}
        />
      </Row>
    </Section>
  );
}

// ── The Librarian ───────────────────────────────────────────────────────────

export type LibSchedule = {
  enabled: boolean;
  start_time: string;
  duration_minutes: number;
  model: string;
  run_if_launched_in_window: boolean;
  pruning_enabled?: boolean;
};

export function nextRunText(sched: LibSchedule): string {
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

/** The nightly pass: when it runs, for how long, on which local model. */
export function LibrarianScheduleSettings() {
  const { colors } = useTheme();
  const [schedule, setSchedule] = useState<LibSchedule | null>(null);
  const [installed, setInstalled] = useState<string[]>([]);
  const [running, setRunning] = useState<string[]>([]);
  const [saving, setSaving] = useState(false);
  const [runningNow, setRunningNow] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    const poll = () => {
      api.getLibrarianSchedule().then(s => { if (active) setSchedule(s); }).catch(() => {});
      api.getOllamaStatus()
        .then(s => {
          if (!active) return;
          setInstalled(s.installed.map(m => m.name));
          setRunning(s.running.map(m => m.name));
        })
        .catch(() => {});
    };
    poll();
    const id = setInterval(poll, 8000);
    return () => { active = false; clearInterval(id); };
  }, []);

  const modelState = (name: string): 'running' | 'installed' | 'missing' => {
    if (running.some(m => m === name || m.startsWith(name + ':'))) return 'running';
    if (installed.some(m => m === name || m.startsWith(name + ':'))) return 'installed';
    return 'missing';
  };

  const change = async (patch: Partial<LibSchedule>) => {
    if (!schedule) return;
    const prev = schedule;
    const next = { ...schedule, ...patch };
    setSchedule(next);
    setSaving(true);
    setError(null);
    try {
      await api.setLibrarianSchedule(next);
    } catch (err) {
      // Revert + surface: a swallowed catch left the panel showing a schedule
      // the daemon never persisted.
      setSchedule(prev);
      setError(`Couldn't save the Librarian schedule: ${err instanceof Error ? err.message : String(err)}`);
    }
    setSaving(false);
  };

  const runNow = async () => {
    setRunningNow(true);
    setError(null);
    let started = true;
    try {
      await api.runLibrarianNow();
    } catch (err) {
      // Swallowed into `error`, so the Button contract needs the explicit
      // `false` — a run that never started must not finish with a tick.
      started = false;
      setError(`Couldn't start the Librarian: ${err instanceof Error ? err.message : String(err)}`);
    }
    setRunningNow(false);
    api.getOllamaStatus().then(s => setRunning(s.running.map(m => m.name))).catch(() => {});
    return started;
  };

  if (!schedule) {
    return (
      <Section title="Schedule">
        <div style={{ fontSize: textSize.caption, color: colors.textDim }}>Reading the schedule…</div>
      </Section>
    );
  }

  return (
    <Section title="Schedule" sub="When the nightly pass runs, how long the model stays warm, and which local model it uses.">
      {error && (
        <div style={{ fontSize: textSize.caption, color: colors.danger, padding: '4px 0 8px' }}>{error}</div>
      )}
      <Row label="Enabled" hint="Run the Librarian on a daily schedule to describe memories.">
        <Toggle on={schedule.enabled} onChange={v => change({ enabled: v })} label="Nightly librarian pass" />
      </Row>
      {schedule.enabled && (
        <>
          <Row label="Start time" hint="Daily start time (24h). The Librarian model will warm-load at this time.">
            <input
              type="time"
              value={schedule.start_time}
              onChange={e => change({ start_time: e.target.value })}
              style={{ ...selectStyle(colors), minWidth: 120, width: 'auto' }}
            />
          </Row>
          <Row label="Duration" hint="How long to keep the model loaded (minutes).">
            <input
              type="number"
              min={15}
              max={720}
              value={schedule.duration_minutes}
              onChange={e => change({ duration_minutes: Math.max(15, Math.min(720, parseInt(e.target.value) || 15)) })}
              style={{ ...selectStyle(colors), minWidth: 100, width: 'auto' }}
            />
            <span style={{ fontSize: textSize.micro, color: colors.textDim, marginLeft: 6 }}>min</span>
          </Row>
          <Row label="Model" hint="Ollama model used by the Librarian. Installed models only.">
            <span style={{ fontSize: textSize.small, color: colors.text, display: 'flex', alignItems: 'center', gap: 8 }}>
              <select
                style={{ ...selectStyle(colors), width: 'auto', minWidth: 160 }}
                value={schedule.model}
                onChange={e => change({ model: e.target.value })}
              >
                {!installed.includes(schedule.model) && (
                  <option value={schedule.model}>{schedule.model} (not installed)</option>
                )}
                {installed.map(name => <option key={name} value={name}>{name}</option>)}
              </select>
              <ModelStateBadge state={modelState(schedule.model)} />
            </span>
          </Row>
          <Row label="Nightly pruning" hint="Let the Librarian retire stale, low-signal memories during its window.">
            <Toggle on={schedule.pruning_enabled ?? false} onChange={v => change({ pruning_enabled: v })} />
          </Row>
          <Row label="Next run" hint={nextRunText(schedule)}>
            <span style={{ fontSize: textSize.caption, color: colors.textMuted }}>{nextRunText(schedule)}</span>
          </Row>
        </>
      )}
      <Row label="Run now" hint="Manually warm-load the model and trigger a Librarian run.">
        <Button
          colors={colors}
          onClick={runNow}
          disabled={runningNow || modelState(schedule.model) === 'missing'}
          style={{
            '--pa-btn-bg': colors.cyanSoft,
            '--pa-btn-fg': runningNow ? colors.textDim : colors.cyan,
            '--pa-btn-border': colors.borderHi,
            '--pa-btn-border-hover': colors.cyan,
            '--pa-btn-bg-hover': colors.cyanSoft,
            '--pa-btn-bg-active': colors.cyanGlow,
            '--pa-btn-pad': '0 14px',
            '--pa-btn-radius': `${radius.sm}px`,
            '--pa-btn-weight': 600,
            height: 30, fontSize: textSize.caption, fontFamily: font.body,
          } as CSSProperties}
        >
          {runningNow ? 'Warming...' : 'Run Librarian now'}
        </Button>
      </Row>
      {saving && <div style={{ fontSize: textSize.micro, color: colors.textDim, textAlign: 'right', padding: '4px 0' }}>Saving...</div>}
    </Section>
  );
}

/** The block for one agent, or nothing when that agent has no extra settings. */
export function AgentSettingsBlock({ agentId }: { agentId: string }) {
  if (agentId === 'strix') return <GuardSweepSettings />;
  if (agentId === 'watcher') return <WatcherTopicSettings />;
  if (agentId === 'librarian') return <LibrarianScheduleSettings />;
  return null;
}
