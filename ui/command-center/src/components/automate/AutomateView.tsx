import { useEffect, useState, useCallback, useRef, useMemo } from 'react';
import { color, font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { cronToEnglish } from '../../lib/schedule-format';
import { useCommandCenter } from '../../lib/store';
import { apiFetch } from '../../lib/api';

// ── Types ────────────────────────────────────────────────────────────

interface ScheduledJob {
  id: string;
  source: string;
  cron: string;
  last_run: string | null;
  currently_running: boolean;
  paused: boolean;
  current_session_id: string | null;
  process_start_time: string | null;
  worker_persona: string | null;
  display_name: string | null;
  description: string | null;
}

interface SessionInfo {
  id: string;
  name: string;
  createdAt: string;
  workingDir: string;
  scheduleId: string | null;
  messageCount: number;
  totalTokens: number | null;
  inputTokens: number | null;
  outputTokens: number | null;
}

interface Finding {
  id: string;
  type: string;
  path: string;
  size_bytes: number;
  age_days: number | null;
  recommendation: string;
  action_taken: string | null;
  actioned_at: string | null;
  size_recovered_bytes: number | null;
}

interface ExtensionInfo {
  enabled: boolean;
  type: string;
  name: string;
  description: string;
  display_name: string;
  bundled: boolean;
  available_tools: string[];
}

const CRON_PRESETS = [
  { label: 'Every weekday morning (8 AM)', cron: '0 8 * * 1-5' },
  { label: 'Every morning (8 AM)', cron: '0 8 * * *' },
  { label: 'Every Sunday evening (7 PM)', cron: '0 19 * * 0' },
  { label: 'Every Monday morning (9 AM)', cron: '0 9 * * 1' },
  { label: 'Every hour', cron: '0 * * * *' },
  { label: 'Every day at midnight', cron: '0 0 * * *' },
  { label: 'First day of every month', cron: '0 0 1 * *' },
  { label: 'Custom (cron expression)', cron: '' },
];

function timeAgo(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  return `${days}d ago`;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

// ── Detail panel types ──────────────────────────────────────────────

type DetailTarget =
  | { kind: 'recipe'; job: ScheduledJob }
  | { kind: 'extension'; ext: ExtensionInfo }
  | { kind: 'run'; run: SessionInfo & { jobId: string }; displayName: string };

// ═══════════════════════════════════════════════════════════════════════
// Main AutomateView — single scrollable page with semantic sections
// ═══════════════════════════════════════════════════════════════════════

export function AutomateView() {
  const [jobs, setJobs] = useState<ScheduledJob[]>([]);
  const [sessions, setSessions] = useState<Map<string, SessionInfo[]>>(new Map());
  const [extensions, setExtensions] = useState<ExtensionInfo[]>([]);
  const [showModal, setShowModal] = useState(false);
  const [completionToast, setCompletionToast] = useState<string | null>(null);
  const [detail, setDetail] = useState<DetailTarget | null>(null);
  const [search, setSearch] = useState('');
  const [showSearch, setShowSearch] = useState(false);
  const [showInstalledExpanded, setShowInstalledExpanded] = useState(false);
  const prevRunningRef = useRef<Set<string>>(new Set());
  const { gradient } = useTheme();

  const skills = useCommandCenter(s => s.skills);
  const skillsLoading = useCommandCenter(s => s.skillsLoading);
  const loadSkills = useCommandCenter(s => s.loadSkills);

  // ── Data fetching ──

  const fetchJobs = useCallback(async () => {
    try {
      const data = await apiFetch<{ jobs: ScheduledJob[] }>('/schedule/list');
      const newJobs: ScheduledJob[] = data.jobs || [];
      setJobs(newJobs);
      const nowRunning = new Set(newJobs.filter(j => j.currently_running).map(j => j.id));
      const prev = prevRunningRef.current;
      for (const id of prev) {
        if (!nowRunning.has(id)) {
          const name = newJobs.find(j => j.id === id)?.display_name || id;
          setCompletionToast(name);
          setTimeout(() => setCompletionToast(null), 8000);
        }
      }
      prevRunningRef.current = nowRunning;
    } catch {}
  }, []);

  const fetchAllSessions = useCallback(async (jobIds: string[]) => {
    for (const jobId of jobIds) {
      try {
        const data = await apiFetch<SessionInfo[]>(`/schedule/${encodeURIComponent(jobId)}/sessions?limit=5`);
        setSessions(prev => new Map(prev).set(jobId, data));
      } catch {}
    }
  }, []);

  const fetchExtensions = useCallback(async () => {
    try {
      const data = await apiFetch<{ extensions: ExtensionInfo[]; warnings: string[] }>('/config/extensions');
      setExtensions((data.extensions || []).filter(e => e.enabled));
    } catch {}
  }, []);

  useEffect(() => { fetchJobs(); fetchExtensions(); loadSkills(); }, [fetchJobs, fetchExtensions, loadSkills]);
  useEffect(() => {
    const interval = setInterval(fetchJobs, 5000);
    return () => clearInterval(interval);
  }, [fetchJobs]);
  useEffect(() => {
    if (jobs.length > 0) fetchAllSessions(jobs.map(j => j.id));
    const interval = setInterval(() => {
      if (jobs.length > 0) fetchAllSessions(jobs.map(j => j.id));
    }, 10000);
    return () => clearInterval(interval);
  }, [jobs.length]); // eslint-disable-line react-hooks/exhaustive-deps

  // ── Derived data ──

  const runningJobs = jobs.filter(j => j.currently_running);
  const allRuns = useMemo(() =>
    Array.from(sessions.entries())
      .flatMap(([jobId, runs]) => runs.map(r => ({ ...r, jobId })))
      .sort((a, b) => new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime())
      .slice(0, 10),
    [sessions],
  );
  const jobNameMap = new Map(jobs.map(j => [j.id, j.display_name || j.id]));

  // ── Search filter ──

  const q = search.toLowerCase().trim();
  const filteredJobs = q ? jobs.filter(j => (j.display_name || j.id).toLowerCase().includes(q) || (j.description || '').toLowerCase().includes(q)) : jobs;
  const filteredSkills = q ? skills.filter(s => s.name.toLowerCase().includes(q) || (s.description || '').toLowerCase().includes(q)) : skills;
  const filteredExtensions = q ? extensions.filter(e => e.display_name.toLowerCase().includes(q) || e.description.toLowerCase().includes(q)) : extensions;

  // ── Actions ──

  const handleRunNow = async (id: string) => { try { await apiFetch<unknown>(`/schedule/${encodeURIComponent(id)}/run_now`, { method: 'POST' }); fetchJobs(); } catch {} };
  const handlePause = async (id: string) => { try { await apiFetch<unknown>(`/schedule/${encodeURIComponent(id)}/pause`, { method: 'POST' }); fetchJobs(); } catch {} };
  const handleUnpause = async (id: string) => { try { await apiFetch<unknown>(`/schedule/${encodeURIComponent(id)}/unpause`, { method: 'POST' }); fetchJobs(); } catch {} };
  const handleDelete = async (id: string) => {
    const name = jobNameMap.get(id) || id;
    if (!confirm(`Delete "${name}"? This can't be undone.`)) return;
    try { await apiFetch<unknown>(`/schedule/delete/${encodeURIComponent(id)}`, { method: 'DELETE' }); fetchJobs(); setDetail(null); } catch {}
  };
  const handleKill = async (id: string) => { try { await apiFetch<unknown>(`/schedule/${encodeURIComponent(id)}/kill`, { method: 'POST' }); fetchJobs(); } catch {} };

  // ── Truly-empty check ──
  const trulyEmpty = jobs.length === 0 && skills.length === 0 && !skillsLoading;

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', background: gradient.workspace, color: color.text, fontFamily: font.body }}>

      {/* ── Main scrollable content ── */}
      <div style={{ flex: 1, minWidth: 0, height: '100%', overflowY: 'auto', padding: '20px 32px 40px' }}>

        {/* Header */}
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 24 }}>
          <div style={{ fontFamily: font.display, fontSize: 20, fontWeight: 600 }}>Automate</div>
          <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
            {showSearch && (
              <input
                autoFocus
                value={search}
                onChange={e => setSearch(e.target.value)}
                placeholder="Filter..."
                style={{
                  width: 180, padding: '5px 10px', borderRadius: radius.sm,
                  background: 'rgba(20,28,48,0.6)', border: `1px solid ${color.border}`,
                  color: color.text, fontSize: 12, fontFamily: font.mono, outline: 'none',
                }}
                onKeyDown={e => { if (e.key === 'Escape') { setSearch(''); setShowSearch(false); } }}
              />
            )}
            <button onClick={() => setShowSearch(!showSearch)} style={{
              background: 'none', border: 'none', cursor: 'pointer', color: color.textMuted, padding: 4,
            }}>
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2}>
                <circle cx="11" cy="11" r="8" /><path d="M21 21l-4.35-4.35" />
              </svg>
            </button>
            <button onClick={() => setShowModal(true)} style={{
              padding: '6px 14px', borderRadius: radius.md, background: color.cyan, color: '#000',
              fontWeight: 600, fontSize: 12, border: 'none', cursor: 'pointer', fontFamily: font.body,
            }}>+ Create</button>
          </div>
        </div>

        {/* Completion toast */}
        {completionToast && (
          <div style={{
            marginBottom: 16, padding: '10px 16px', borderRadius: radius.md,
            background: 'rgba(91,209,127,0.1)', border: '1px solid rgba(91,209,127,0.25)',
            display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          }}>
            <span style={{ fontSize: 13, color: '#5BD17F' }}>"{completionToast}" completed.</span>
            <button onClick={() => setCompletionToast(null)} style={{ background: 'none', border: 'none', color: color.textDim, cursor: 'pointer', fontSize: 16 }}>x</button>
          </div>
        )}

        {/* Truly-empty invitation */}
        {trulyEmpty && jobs.length === 0 && (
          <div style={{
            padding: '32px 28px', borderRadius: radius.lg, marginBottom: 24,
            background: 'rgba(20,28,48,0.5)', border: `1px solid ${color.border}`, textAlign: 'center',
          }}>
            <div style={{ fontSize: 15, fontWeight: 600, fontFamily: font.display, marginBottom: 8 }}>
              Your agent can do a lot already
            </div>
            <div style={{ fontSize: 13, color: color.textMuted, lineHeight: 1.6, maxWidth: 420, margin: '0 auto' }}>
              Try asking Henry to summarize your Downloads folder — if you like the result,
              ask him to remember how. Skills appear here when you save them.
            </div>
            <div style={{ marginTop: 16, display: 'flex', gap: 12, justifyContent: 'center' }}>
              <button onClick={() => setShowModal(true)} style={{
                padding: '8px 20px', borderRadius: radius.md, background: color.cyan, color: '#000',
                fontWeight: 600, fontSize: 13, border: 'none', cursor: 'pointer', fontFamily: font.body,
              }}>Schedule a task</button>
            </div>
            <button onClick={() => setShowInstalledExpanded(true)} style={{
              marginTop: 12, background: 'none', border: 'none', color: color.textDim,
              fontSize: 11, cursor: 'pointer', fontFamily: font.body,
            }}>or browse what your agent can do &rarr;</button>
          </div>
        )}

        {/* ── RUNNING NOW ── (only when something is executing) */}
        {runningJobs.length > 0 && (
          <Section title="Running Now" count={runningJobs.length} accentColor="#5BD17F">
            {runningJobs.map(job => (
              <RecipeCard key={job.id} job={job} onRunNow={handleRunNow} onPause={handlePause}
                onUnpause={handleUnpause} onDelete={handleDelete} onKill={handleKill}
                onSelect={() => setDetail({ kind: 'recipe', job })}
                selected={detail?.kind === 'recipe' && detail.job.id === job.id} />
            ))}
          </Section>
        )}

        {/* ── RECIPES ── */}
        {(filteredJobs.length > 0 || !trulyEmpty) && (
          <Section title="Recipes" count={filteredJobs.length}>
            {filteredJobs.length === 0 ? (
              <div style={{ fontSize: 12, color: color.textDim, padding: '8px 0' }}>
                No recipes yet. Click "+ Create" to schedule your first automation.
              </div>
            ) : (
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))', gap: 12 }}>
                {filteredJobs.filter(j => !j.currently_running).map(job => (
                  <RecipeCard key={job.id} job={job} onRunNow={handleRunNow} onPause={handlePause}
                    onUnpause={handleUnpause} onDelete={handleDelete} onKill={handleKill}
                    onSelect={() => setDetail({ kind: 'recipe', job })}
                    selected={detail?.kind === 'recipe' && detail.job.id === job.id} />
                ))}
              </div>
            )}
          </Section>
        )}

        {/* ── LEARNED ── */}
        <Section title="Learned" count={filteredSkills.length}>
          {skillsLoading ? (
            <div style={{ fontSize: 12, color: color.textDim }}>Loading...</div>
          ) : filteredSkills.length === 0 ? (
            <div style={{
              padding: '20px 24px', borderRadius: radius.lg,
              background: 'rgba(141,68,174,0.04)', border: '1px solid rgba(141,68,174,0.12)',
            }}>
              <div style={{ fontSize: 13, color: color.textMuted, lineHeight: 1.6 }}>
                When you repeat tasks, Henry notices patterns and offers to save them.
                Your first skill will appear here.
              </div>
              <div style={{ fontSize: 12, color: color.textDim, marginTop: 8 }}>
                Try asking Henry to do something twice — like "summarize my Downloads folder"
                — and he'll offer to remember how.
              </div>
            </div>
          ) : (
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))', gap: 12 }}>
              {filteredSkills.map(skill => (
                <div key={skill.id} style={{
                  padding: '16px 20px', borderRadius: radius.lg, cursor: 'pointer',
                  background: 'rgba(20,28,48,0.5)', border: `1px solid ${color.border}`,
                  transition: 'border-color 150ms',
                }} onClick={() => {}}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 6 }}>
                    <div style={{ width: 8, height: 8, borderRadius: '50%', background: skill.status === 'active' ? '#5BD17F' : color.textDim }} />
                    <div style={{ fontSize: 14, fontWeight: 600, fontFamily: font.display, flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{skill.name}</div>
                    <span style={{ fontSize: 9, fontFamily: font.mono, padding: '2px 6px', borderRadius: radius.sm, background: color.purpleSoft, color: color.purpleBright }}>LEARNED</span>
                  </div>
                  {skill.description && <div style={{ fontSize: 12, color: color.textMuted, lineHeight: 1.5, marginBottom: 8 }}>{skill.description}</div>}
                  <div style={{ fontSize: 10, color: color.textDim, fontFamily: font.mono }}>
                    {skill.usage_count || 0} runs &middot; {skill.status}
                  </div>
                </div>
              ))}
            </div>
          )}
        </Section>

        {/* ── INSTALLED ── */}
        <Section title="Installed" count={filteredExtensions.length} collapsed>
          {!showInstalledExpanded ? (
            <button onClick={() => setShowInstalledExpanded(true)} style={{
              background: 'none', border: 'none', color: color.textMuted, cursor: 'pointer',
              fontSize: 12, fontFamily: font.body, padding: '4px 0', textAlign: 'left',
            }}>
              Henry has {extensions.length} capabilities &middot; <span style={{ color: color.cyan }}>Show what your agent can do &rarr;</span>
            </button>
          ) : (
            <>
              <button onClick={() => setShowInstalledExpanded(false)} style={{
                background: 'none', border: 'none', color: color.textDim, cursor: 'pointer',
                fontSize: 11, fontFamily: font.body, padding: '0 0 8px', textAlign: 'left',
              }}>Hide &uarr;</button>
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8 }}>
                {filteredExtensions.map(ext => (
                  <button key={ext.name} onClick={() => setDetail({ kind: 'extension', ext })} style={{
                    padding: '8px 14px', borderRadius: radius.md, cursor: 'pointer',
                    background: detail?.kind === 'extension' && detail.ext.name === ext.name ? color.cyanSoft : 'rgba(20,28,48,0.5)',
                    border: `1px solid ${detail?.kind === 'extension' && detail.ext.name === ext.name ? color.borderHi : color.border}`,
                    color: color.text, fontSize: 12, fontFamily: font.body, textAlign: 'left',
                    transition: 'border-color 150ms, background 150ms',
                  }}>
                    <div style={{ fontWeight: 600, marginBottom: 2 }}>{ext.display_name}</div>
                    <div style={{ fontSize: 10, color: color.textDim, maxWidth: 220, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{ext.description}</div>
                  </button>
                ))}
              </div>
            </>
          )}
        </Section>

        {/* ── RECENT ACTIVITY ── */}
        {allRuns.length > 0 && (
          <Section title="Recent Activity" count={allRuns.length}>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
              {allRuns.map(run => {
                const displayName = jobNameMap.get(run.jobId) || run.jobId;
                return (
                  <button key={run.id} onClick={() => setDetail({ kind: 'run', run, displayName })} style={{
                    display: 'flex', alignItems: 'center', gap: 10, padding: '8px 12px',
                    borderRadius: radius.sm, background: 'transparent', border: 'none',
                    cursor: 'pointer', textAlign: 'left', color: color.text, fontFamily: font.body,
                    width: '100%', transition: 'background 100ms',
                  }} onMouseEnter={e => (e.currentTarget.style.background = 'rgba(255,255,255,0.03)')}
                     onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}>
                    <span style={{ fontSize: 11, color: '#5BD17F' }}>&#10003;</span>
                    <span style={{ fontSize: 12, flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{displayName}</span>
                    <span style={{ fontSize: 10, color: color.textDim, fontFamily: font.mono, flexShrink: 0 }}>{run.messageCount} msgs</span>
                    <span style={{ fontSize: 10, color: color.textDim, flexShrink: 0 }}>{timeAgo(run.createdAt)}</span>
                  </button>
                );
              })}
            </div>
          </Section>
        )}
      </div>

      {/* ── Detail panel (slides in from right) ── */}
      {detail && (
        <DetailPanel detail={detail} onClose={() => setDetail(null)}
          onRunNow={handleRunNow} onPause={handlePause} onUnpause={handleUnpause}
          onDelete={handleDelete} onKill={handleKill} />
      )}

      {showModal && <NewAutomationModal onClose={() => setShowModal(false)} onCreated={() => { setShowModal(false); fetchJobs(); }} />}
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════
// Section header
// ═══════════════════════════════════════════════════════════════════════

function Section({ title, count, accentColor, collapsed, children }: {
  title: string; count: number; accentColor?: string; collapsed?: boolean;
  children: React.ReactNode;
}) {
  return (
    <div style={{ marginBottom: 28 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12 }}>
        <div style={{ fontSize: 10, fontWeight: 700, fontFamily: font.mono, letterSpacing: '0.08em', textTransform: 'uppercase', color: accentColor || color.textDim }}>
          {title}
        </div>
        {!collapsed && count > 0 && (
          <span style={{ fontSize: 10, fontFamily: font.mono, color: color.textDim, padding: '1px 6px', borderRadius: radius.sm, background: 'rgba(255,255,255,0.04)' }}>
            {count}
          </span>
        )}
        <div style={{ flex: 1, height: 1, background: color.border }} />
      </div>
      {children}
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════
// Recipe card (for Recipes + Running Now sections)
// ═══════════════════════════════════════════════════════════════════════

function RecipeCard({ job, onRunNow, onPause, onUnpause, onDelete, onKill, onSelect, selected }: {
  job: ScheduledJob;
  onRunNow: (id: string) => void;
  onPause: (id: string) => void;
  onUnpause: (id: string) => void;
  onDelete: (id: string) => void;
  onKill: (id: string) => void;
  onSelect: () => void;
  selected: boolean;
}) {
  const name = job.display_name || job.id;
  const desc = job.description || '';
  const schedule = cronToEnglish(job.cron);
  const statusColor = job.currently_running ? '#5BD17F' : job.paused ? color.textDim : color.cyan;

  return (
    <div onClick={onSelect} style={{
      padding: '16px 20px', borderRadius: radius.lg, cursor: 'pointer',
      background: selected ? 'rgba(0,213,255,0.04)' : 'rgba(20,28,48,0.5)',
      border: `1px solid ${selected ? color.borderHi : color.border}`,
      transition: 'border-color 150ms, background 150ms',
    }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 6 }}>
        <div style={{
          width: 8, height: 8, borderRadius: '50%', background: statusColor, flexShrink: 0,
          boxShadow: job.currently_running ? `0 0 8px ${statusColor}` : 'none',
        }} />
        <div style={{ fontSize: 14, fontWeight: 600, fontFamily: font.display, flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{name}</div>
        <span style={{ fontSize: 9, fontFamily: font.mono, padding: '2px 6px', borderRadius: radius.sm, background: color.cyanSoft, color: color.cyan }}>
          {job.currently_running ? 'RUNNING' : job.paused ? 'PAUSED' : 'SCHEDULED'}
        </span>
      </div>
      {desc && <div style={{ fontSize: 12, color: color.textMuted, lineHeight: 1.5, marginBottom: 8, overflow: 'hidden', textOverflow: 'ellipsis', display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical' } as React.CSSProperties}>{desc}</div>}
      <div style={{ fontSize: 10, color: color.textDim, fontFamily: font.mono, marginBottom: 10 }}>
        {schedule} &middot; {job.last_run ? `Ran ${timeAgo(job.last_run)}` : 'Never run yet'}
      </div>
      <div style={{ display: 'flex', gap: 6 }} onClick={e => e.stopPropagation()}>
        {job.currently_running ? (
          <Btn label="Stop" onClick={() => onKill(job.id)} danger />
        ) : (
          <>
            <Btn label="Run Now" onClick={() => onRunNow(job.id)} primary />
            {job.paused ? <Btn label="Resume" onClick={() => onUnpause(job.id)} /> : <Btn label="Pause" onClick={() => onPause(job.id)} />}
          </>
        )}
        <Btn label="Delete" onClick={() => onDelete(job.id)} danger muted />
      </div>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════
// Detail panel (right slide-in)
// ═══════════════════════════════════════════════════════════════════════

function DetailPanel({ detail, onClose, onRunNow, onPause, onUnpause, onDelete, onKill }: {
  detail: DetailTarget;
  onClose: () => void;
  onRunNow: (id: string) => void;
  onPause: (id: string) => void;
  onUnpause: (id: string) => void;
  onDelete: (id: string) => void;
  onKill: (id: string) => void;
}) {
  // Close on Escape
  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose]);

  return (
    <div style={{
      width: '50%', minWidth: 480, maxWidth: 640, height: '100%',
      borderLeft: `1px solid ${color.border}`, background: 'rgba(11,18,32,0.95)',
      overflowY: 'auto', padding: '20px 24px', flexShrink: 0,
    }}>
      <div style={{ display: 'flex', justifyContent: 'flex-end', marginBottom: 12 }}>
        <button onClick={onClose} style={{ background: 'none', border: 'none', color: color.textDim, cursor: 'pointer', fontSize: 18 }}>&times;</button>
      </div>

      {detail.kind === 'recipe' && (
        <RecipeDetail job={detail.job} onRunNow={onRunNow} onPause={onPause}
          onUnpause={onUnpause} onDelete={onDelete} onKill={onKill} />
      )}
      {detail.kind === 'extension' && <ExtensionDetail ext={detail.ext} />}
      {detail.kind === 'run' && <RunDetail run={detail.run} displayName={detail.displayName} />}
    </div>
  );
}

function RecipeDetail({ job, onRunNow, onPause, onUnpause, onDelete, onKill }: {
  job: ScheduledJob;
  onRunNow: (id: string) => void; onPause: (id: string) => void;
  onUnpause: (id: string) => void; onDelete: (id: string) => void;
  onKill: (id: string) => void;
}) {
  const name = job.display_name || job.id;
  const statusColor = job.currently_running ? '#5BD17F' : job.paused ? color.textDim : color.cyan;
  return (
    <>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 16 }}>
        <div style={{ width: 10, height: 10, borderRadius: '50%', background: statusColor, boxShadow: job.currently_running ? `0 0 8px ${statusColor}` : 'none' }} />
        <div style={{ fontSize: 18, fontWeight: 600, fontFamily: font.display }}>{name}</div>
      </div>
      {job.description && <div style={{ fontSize: 13, color: color.textMuted, lineHeight: 1.6, marginBottom: 16 }}>{job.description}</div>}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '12px 24px', marginBottom: 20 }}>
        <MetaField label="Schedule" value={cronToEnglish(job.cron)} />
        <MetaField label="Status" value={job.currently_running ? 'Running' : job.paused ? 'Paused' : 'Active'} />
        <MetaField label="Last Run" value={job.last_run ? timeAgo(job.last_run) : 'Never'} />
        <MetaField label="ID" value={job.id} mono />
      </div>
      <div style={{ display: 'flex', gap: 8 }}>
        {job.currently_running ? (
          <Btn label="Stop" onClick={() => onKill(job.id)} danger />
        ) : (
          <>
            <Btn label="Run Now" onClick={() => onRunNow(job.id)} primary />
            {job.paused ? <Btn label="Resume" onClick={() => onUnpause(job.id)} /> : <Btn label="Pause" onClick={() => onPause(job.id)} />}
          </>
        )}
        <Btn label="Delete" onClick={() => onDelete(job.id)} danger muted />
      </div>
    </>
  );
}

function ExtensionDetail({ ext }: { ext: ExtensionInfo }) {
  return (
    <>
      <div style={{ fontSize: 18, fontWeight: 600, fontFamily: font.display, marginBottom: 8 }}>{ext.display_name}</div>
      <div style={{ fontSize: 13, color: color.textMuted, lineHeight: 1.6, marginBottom: 16 }}>{ext.description}</div>
      <MetaField label="Type" value={ext.type} />
      <div style={{ marginTop: 16 }}>
        <div style={{ fontSize: 10, fontFamily: font.mono, textTransform: 'uppercase', color: color.textDim, marginBottom: 6, letterSpacing: '0.05em' }}>
          Tools provided
        </div>
        {ext.available_tools.length > 0 ? (
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
            {ext.available_tools.map(t => (
              <span key={t} style={{ fontSize: 11, fontFamily: font.mono, padding: '3px 8px', borderRadius: radius.sm, background: 'rgba(255,255,255,0.04)', color: color.textMuted }}>{t}</span>
            ))}
          </div>
        ) : (
          <div style={{ fontSize: 12, color: color.textDim }}>Tool list not available — tools are loaded at runtime.</div>
        )}
      </div>
    </>
  );
}

function RunDetail({ run, displayName }: { run: SessionInfo & { jobId: string }; displayName: string }) {
  const [reportText, setReportText] = useState<string | null>(null);
  const [findings, setFindings] = useState<Finding[]>([]);
  const [actionInFlight, setActionInFlight] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const session = await apiFetch<{ conversation?: Array<{ role: string; content?: Array<{ type: string; text?: string }> }> }>(`/api/sessions/${encodeURIComponent(run.id)}`);
        if (!cancelled) {
          const msgs = session.conversation || [];
          const text = msgs.filter((m: any) => m.role === 'assistant')
            .flatMap((m: any) => (m.content || []).filter((c: any) => c.type === 'text').map((c: any) => c.text))
            .join('\n\n');
          setReportText(text.replace(/<findings>[\s\S]*?<\/findings>/g, '').trim() || 'No output captured.');
        }
      } catch {}
      try {
        const data = await apiFetch<{ findings: Finding[] }>(`/automation/run/${encodeURIComponent(run.id)}/findings`);
        if (!cancelled) setFindings(data.findings || []);
      } catch {}
      if (!cancelled) setLoading(false);
    })();
    return () => { cancelled = true; };
  }, [run.id]);

  const handleAction = async (findingId: string, action: string) => {
    setActionInFlight(findingId);
    try {
      const result = await apiFetch<{ action_taken: string; timestamp: string; size_recovered_bytes?: number }>(`/automation/finding/${encodeURIComponent(findingId)}/action`, {
        method: 'POST', body: JSON.stringify({ action, run_id: run.id }),
      });
      setFindings(prev => prev.map(f => f.id === findingId ? { ...f, action_taken: result.action_taken, actioned_at: result.timestamp, size_recovered_bytes: result.size_recovered_bytes ?? null } : f));
    } catch {
      setFindings(prev => prev.map(f => f.id === findingId ? { ...f, action_taken: 'skipped', actioned_at: new Date().toISOString() } : f));
    }
    setActionInFlight(null);
  };

  const totalRecovered = findings.filter(f => f.action_taken === 'trashed').reduce((s, f) => s + (f.size_recovered_bytes || 0), 0);
  const allActioned = findings.length > 0 && findings.every(f => f.action_taken !== null);

  return (
    <>
      <div style={{ fontSize: 18, fontWeight: 600, fontFamily: font.display, marginBottom: 4 }}>{displayName}</div>
      <div style={{ fontSize: 11, color: color.textDim, fontFamily: font.mono, marginBottom: 16 }}>
        {new Date(run.createdAt).toLocaleString()} &middot; {run.messageCount} msgs &middot; {run.totalTokens ?? 0} tokens
      </div>
      {loading ? (
        <div style={{ fontSize: 12, color: color.textDim }}>Loading results...</div>
      ) : findings.length > 0 ? (
        <>
          <FindingsPanel findings={findings} actionInFlight={actionInFlight} onAction={handleAction} totalRecovered={totalRecovered} allActioned={allActioned} />
          {reportText && <ReportToggle text={reportText} createdAt={run.createdAt} tokens={run.totalTokens} />}
        </>
      ) : reportText ? (
        <>
          <div style={{ maxHeight: 600, overflowY: 'auto' }}><RenderedReport text={reportText} /></div>
        </>
      ) : null}
    </>
  );
}

function MetaField({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div>
      <div style={{ fontSize: 10, fontFamily: font.mono, textTransform: 'uppercase', color: color.textDim, marginBottom: 2, letterSpacing: '0.05em' }}>{label}</div>
      <div style={{ fontSize: 12, color: color.textMuted, fontFamily: mono ? font.mono : font.body }}>{value}</div>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════
// Shared button
// ═══════════════════════════════════════════════════════════════════════

function Btn({ label, onClick, primary, danger, muted }: {
  label: string; onClick: () => void; primary?: boolean; danger?: boolean; muted?: boolean;
}) {
  const bg = primary ? color.cyan : danger ? 'rgba(255,100,100,0.1)' : 'rgba(255,255,255,0.05)';
  const fg = primary ? '#000' : danger ? '#ff6b6b' : color.textMuted;
  const bdr = primary ? 'transparent' : danger ? 'rgba(255,100,100,0.2)' : color.border;
  return (
    <button onClick={onClick} style={{
      padding: primary ? '6px 16px' : '4px 10px', borderRadius: radius.sm,
      background: bg, border: `1px solid ${bdr}`, color: fg,
      fontSize: 11, fontWeight: primary ? 600 : 500, cursor: 'pointer', fontFamily: font.body,
      opacity: muted ? 0.6 : 1,
    }}>{label}</button>
  );
}

// ═══════════════════════════════════════════════════════════════════════
// Sub-components preserved from original (Findings, Reports, Modal)
// ═══════════════════════════════════════════════════════════════════════

function ReportToggle({ text, createdAt, tokens }: { text: string; createdAt: string; tokens: number | null }) {
  const [open, setOpen] = useState(false);
  return (
    <div style={{ marginTop: 16 }}>
      <button onClick={() => setOpen(!open)} style={{
        display: 'flex', alignItems: 'center', gap: 8, padding: '8px 0',
        background: 'none', border: 'none', cursor: 'pointer', color: color.textDim,
        fontSize: 11, fontFamily: font.body,
      }}>
        <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2}
          style={{ transform: open ? 'rotate(90deg)' : 'rotate(0deg)', transition: 'transform 150ms' }}>
          <path d="M9 18l6-6-6-6" />
        </svg>
        View full report &middot; {new Date(createdAt).toLocaleString()} &middot; {tokens ?? 0} tokens
      </button>
      {open && <div style={{ maxHeight: 400, overflowY: 'auto', marginTop: 4 }}><RenderedReport text={text} /></div>}
    </div>
  );
}

function RenderedReport({ text }: { text: string }) {
  const lines = text.split('\n');
  const elements: React.ReactNode[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) { i++; continue; }
    if (line.startsWith('# ')) { elements.push(<div key={i} style={{ fontSize: 16, fontWeight: 700, fontFamily: font.display, color: color.text, marginTop: 16, marginBottom: 8 }}>{renderInline(line.slice(2))}</div>); i++; continue; }
    if (line.startsWith('## ')) { elements.push(<div key={i} style={{ fontSize: 14, fontWeight: 600, fontFamily: font.display, color: color.text, marginTop: 14, marginBottom: 6 }}>{renderInline(line.slice(3))}</div>); i++; continue; }
    if (line.startsWith('### ')) { elements.push(<div key={i} style={{ fontSize: 13, fontWeight: 600, color: color.cyan, marginTop: 12, marginBottom: 4 }}>{renderInline(line.slice(4))}</div>); i++; continue; }
    if (line.match(/^---+$/)) { elements.push(<hr key={i} style={{ border: 'none', borderTop: `1px solid ${color.border}`, margin: '12px 0' }} />); i++; continue; }
    if (line.startsWith('```')) { const codeLines: string[] = []; i++; while (i < lines.length && !lines[i].startsWith('```')) { codeLines.push(lines[i]); i++; } i++; elements.push(<pre key={`code-${i}`} style={{ padding: '10px 12px', borderRadius: radius.sm, background: 'rgba(10,14,23,0.8)', border: `1px solid ${color.border}`, fontSize: 11, fontFamily: font.mono, color: color.textMuted, overflow: 'auto', margin: '6px 0', lineHeight: 1.5 }}>{codeLines.join('\n')}</pre>); continue; }
    if (line.match(/^\s*-\s/)) { const bl: string[] = []; while (i < lines.length && lines[i].match(/^\s*-\s/)) { bl.push(lines[i].replace(/^\s*-\s/, '')); i++; } elements.push(<div key={`b-${i}`} style={{ margin: '4px 0 4px 4px' }}>{bl.map((b, j) => <div key={j} style={{ display: 'flex', gap: 8, marginBottom: 3, fontSize: 12, color: color.textMuted, lineHeight: 1.5 }}><span style={{ color: color.cyan, flexShrink: 0, marginTop: 1 }}>&#8226;</span><span>{renderInline(b)}</span></div>)}</div>); continue; }
    if (line.match(/^\d+\.\s/)) { const nl: string[] = []; while (i < lines.length && lines[i].match(/^\d+\.\s/)) { nl.push(lines[i].replace(/^\d+\.\s/, '')); i++; } elements.push(<div key={`n-${i}`} style={{ margin: '4px 0 4px 4px' }}>{nl.map((n, j) => <div key={j} style={{ display: 'flex', gap: 8, marginBottom: 3, fontSize: 12, color: color.textMuted, lineHeight: 1.5 }}><span style={{ color: color.textDim, flexShrink: 0, fontFamily: font.mono, fontSize: 11, minWidth: 16 }}>{j + 1}.</span><span>{renderInline(n)}</span></div>)}</div>); continue; }
    elements.push(<div key={i} style={{ fontSize: 12, color: color.textMuted, lineHeight: 1.6, marginBottom: 4 }}>{renderInline(line)}</div>); i++;
  }
  return <div>{elements}</div>;
}

function renderInline(text: string): React.ReactNode {
  const parts: React.ReactNode[] = []; let remaining = text; let key = 0;
  while (remaining.length > 0) {
    const bold = remaining.match(/^(.*?)\*\*(.+?)\*\*(.*)/s);
    if (bold) { if (bold[1]) parts.push(<span key={key++}>{bold[1]}</span>); parts.push(<strong key={key++} style={{ color: color.text, fontWeight: 600 }}>{bold[2]}</strong>); remaining = bold[3]; continue; }
    const code = remaining.match(/^(.*?)`(.+?)`(.*)/s);
    if (code) { if (code[1]) parts.push(<span key={key++}>{code[1]}</span>); parts.push(<code key={key++} style={{ fontFamily: font.mono, fontSize: 11, padding: '1px 4px', borderRadius: 3, background: 'rgba(255,255,255,0.06)', color: color.cyan }}>{code[2]}</code>); remaining = code[3]; continue; }
    parts.push(<span key={key++}>{remaining}</span>); break;
  }
  return parts.length === 1 ? parts[0] : <>{parts}</>;
}

function groupFindings(findings: Finding[]): Map<string, Finding[]> {
  const groups = new Map<string, Finding[]>();
  for (const f of findings) {
    let group = 'Other';
    const p = f.path.toLowerCase();
    if (p.includes('/desktop/')) group = 'Desktop';
    else if (p.includes('/downloads/')) group = 'Downloads';
    else if (p.includes('/documents/')) group = 'Documents';
    else if (p.includes('/.cache/') || p.includes('/.npm') || p.includes('/.rustup')) group = 'Developer Caches';
    (groups.get(group) || (() => { const l: Finding[] = []; groups.set(group, l); return l; })()).push(f);
  }
  return groups;
}

function FindingsPanel({ findings, actionInFlight, onAction, totalRecovered, allActioned }: {
  findings: Finding[]; actionInFlight: string | null;
  onAction: (findingId: string, action: string) => Promise<void>;
  totalRecovered: number; allActioned: boolean;
}) {
  const groups = groupFindings(findings);
  const [expandedGroup, setExpandedGroup] = useState<string | null>(null);
  const [previewGroup, setPreviewGroup] = useState<string | null>(null);
  const [cleaning, setCleaning] = useState(false);
  const [cleanProgress, setCleanProgress] = useState(0);
  const [cleanTotal, setCleanTotal] = useState(0);

  const runCleanup = async (items: Finding[]) => {
    const pending = items.filter(f => !f.action_taken);
    if (pending.length === 0) return;
    setCleaning(true); setCleanTotal(pending.length); setCleanProgress(0);
    for (let i = 0; i < pending.length; i++) { setCleanProgress(i + 1); await onAction(pending[i].id, 'trash'); await new Promise(r => setTimeout(r, 100)); }
    setCleaning(false); setPreviewGroup(null);
  };

  const totalPending = findings.filter(f => !f.action_taken).length;
  const totalPendingBytes = findings.filter(f => !f.action_taken).reduce((s, f) => s + f.size_bytes, 0);

  return (
    <div style={{ marginTop: 12 }}>
      <div style={{
        padding: '20px 24px', borderRadius: radius.lg, marginBottom: 20,
        background: allActioned ? 'linear-gradient(135deg, rgba(91,209,127,0.12), rgba(91,209,127,0.04))' : 'linear-gradient(135deg, rgba(0,213,255,0.1), rgba(141,68,174,0.06))',
        border: `1px solid ${allActioned ? 'rgba(91,209,127,0.25)' : color.borderHi}`,
      }}>
        <div style={{ fontSize: 22, fontWeight: 700, fontFamily: font.display, color: allActioned ? '#5BD17F' : color.text }}>
          {allActioned ? `${formatBytes(totalRecovered)} recovered` : `${formatBytes(totalPendingBytes)} to clean up`}
        </div>
        <div style={{ fontSize: 13, color: color.textMuted, marginTop: 4 }}>
          {allActioned ? `All ${findings.length} items cleaned.` : `${totalPending} items across ${groups.size} locations.`}
        </div>
        {totalPending > 0 && (
          <button onClick={() => setPreviewGroup('__all__')} style={{ marginTop: 14, padding: '12px 32px', borderRadius: radius.md, background: color.cyan, color: '#000', fontWeight: 700, fontSize: 14, border: 'none', cursor: 'pointer', fontFamily: font.body }}>
            Clean Up All — {formatBytes(totalPendingBytes)}
          </button>
        )}
      </div>
      {previewGroup && !cleaning && (() => {
        const items = previewGroup === '__all__' ? findings.filter(f => !f.action_taken) : (groups.get(previewGroup) || []).filter(f => !f.action_taken);
        const bytes = items.reduce((s, f) => s + f.size_bytes, 0);
        return (
          <div style={{ marginBottom: 16, borderRadius: radius.lg, overflow: 'hidden', border: `1px solid ${color.borderHi}`, background: 'rgba(10,14,23,0.8)' }}>
            <div style={{ padding: '16px 20px', borderBottom: `1px solid ${color.border}` }}>
              <div style={{ fontSize: 14, fontWeight: 600, fontFamily: font.display }}>Review ({items.length} items, {formatBytes(bytes)})</div>
            </div>
            <div style={{ maxHeight: 300, overflowY: 'auto', padding: '8px 20px' }}>
              {items.map(f => <div key={f.id} style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '6px 0', borderBottom: `1px solid ${color.border}` }}>
                <div style={{ flex: 1, minWidth: 0 }}><div style={{ fontSize: 12, color: color.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{f.path.split('/').pop()}</div></div>
                <div style={{ fontSize: 11, color: color.textMuted, fontFamily: font.mono }}>{formatBytes(f.size_bytes)}</div>
              </div>)}
            </div>
            <div style={{ padding: '12px 20px', display: 'flex', gap: 8, justifyContent: 'flex-end', borderTop: `1px solid ${color.border}` }}>
              <Btn label="Cancel" onClick={() => setPreviewGroup(null)} />
              <Btn label={`Move ${items.length} to Trash`} onClick={() => runCleanup(items)} primary />
            </div>
          </div>
        );
      })()}
      {cleaning && (
        <div style={{ marginBottom: 16, padding: '16px 20px', borderRadius: radius.lg, background: 'rgba(0,213,255,0.06)', border: `1px solid ${color.borderHi}` }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: color.text }}>Cleaning... {cleanProgress}/{cleanTotal}</div>
          <div style={{ marginTop: 8, height: 4, borderRadius: 2, background: 'rgba(255,255,255,0.1)', overflow: 'hidden' }}>
            <div style={{ height: '100%', borderRadius: 2, background: color.cyan, width: `${(cleanProgress / cleanTotal) * 100}%`, transition: 'width 200ms' }} />
          </div>
        </div>
      )}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        {Array.from(groups.entries()).map(([groupName, items]) => {
          const pending = items.filter(f => !f.action_taken);
          const pendingBytes = pending.reduce((s, f) => s + f.size_bytes, 0);
          const groupRecovered = items.filter(f => f.action_taken === 'trashed').reduce((s, f) => s + (f.size_recovered_bytes || 0), 0);
          const isExpanded = expandedGroup === groupName;
          const allDone = pending.length === 0;
          return (
            <div key={groupName} style={{ borderRadius: radius.lg, overflow: 'hidden', border: `1px solid ${allDone ? 'rgba(91,209,127,0.2)' : color.border}`, background: allDone ? 'rgba(91,209,127,0.03)' : 'rgba(20,28,48,0.5)' }}>
              <div style={{ padding: '16px 20px', display: 'flex', alignItems: 'center', gap: 14 }}>
                <button onClick={() => setExpandedGroup(isExpanded ? null : groupName)} style={{ background: 'none', border: 'none', cursor: 'pointer', padding: 0, opacity: allDone ? 0.5 : 1 }}>
                  <div style={{ width: 36, height: 36, borderRadius: 8, background: 'rgba(255,255,255,0.04)', display: 'grid', placeItems: 'center' }}>
                    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke={color.textMuted} strokeWidth={1.8}><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" /></svg>
                  </div>
                </button>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 15, fontWeight: 600, fontFamily: font.display, color: allDone ? '#5BD17F' : color.text }}>{groupName}</div>
                  <div style={{ fontSize: 12, color: color.textDim, marginTop: 2 }}>{allDone ? `Cleaned — ${formatBytes(groupRecovered)}` : `${pending.length} items · ${formatBytes(pendingBytes)}`}</div>
                </div>
                {allDone ? <div style={{ padding: '8px 16px', borderRadius: radius.md, background: 'rgba(91,209,127,0.1)', color: '#5BD17F', fontSize: 12, fontWeight: 600 }}>Done</div> : (
                  <button onClick={() => setPreviewGroup(groupName)} style={{ padding: '10px 22px', borderRadius: radius.md, background: color.cyan, color: '#000', fontWeight: 700, fontSize: 13, border: 'none', cursor: 'pointer', fontFamily: font.body }}>Clean — {formatBytes(pendingBytes)}</button>
                )}
              </div>
              {isExpanded && (
                <div style={{ padding: '0 20px 16px', borderTop: `1px solid ${color.border}` }}>
                  {items.map(f => <FindingRow key={f.id} finding={f} loading={actionInFlight === f.id} onAction={(a) => onAction(f.id, a)} />)}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function FindingRow({ finding, loading, onAction }: { finding: Finding; loading: boolean; onAction: (a: string) => void }) {
  const fileName = finding.path.split('/').pop() || finding.path;
  if (finding.action_taken === 'trashed') return <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '6px 10px', borderRadius: radius.sm, background: 'rgba(91,209,127,0.05)', border: '1px solid rgba(91,209,127,0.12)' }}><span style={{ fontSize: 12, color: '#5BD17F' }}>Trashed</span><span style={{ fontSize: 11, color: color.textDim, flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{fileName}</span>{finding.size_recovered_bytes != null && <span style={{ fontSize: 11, color: '#5BD17F', fontFamily: font.mono }}>+{formatBytes(finding.size_recovered_bytes)}</span>}</div>;
  if (finding.action_taken) return <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '6px 10px', borderRadius: radius.sm, opacity: 0.6 }}><span style={{ fontSize: 12, color: color.textMuted }}>Kept</span><span style={{ fontSize: 11, color: color.textDim, flex: 1 }}>{fileName}</span></div>;
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '8px 10px', borderRadius: radius.sm, background: 'rgba(20,28,48,0.5)', border: `1px solid ${color.border}`, marginTop: 4 }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 12, fontWeight: 500, color: color.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{fileName}</div>
        <div style={{ fontSize: 10, color: color.textDim, fontFamily: font.mono, marginTop: 2 }}>{formatBytes(finding.size_bytes)}{finding.age_days != null && <> &middot; {finding.age_days}d old</>}</div>
      </div>
      <div style={{ display: 'flex', gap: 4, flexShrink: 0 }}>
        <button onClick={() => onAction('trash')} disabled={loading} style={{ padding: '3px 8px', borderRadius: radius.sm, background: 'rgba(255,100,100,0.1)', border: '1px solid rgba(255,100,100,0.2)', color: '#ff6b6b', fontSize: 10, fontWeight: 600, cursor: loading ? 'wait' : 'pointer', fontFamily: font.body }}>{loading ? '...' : 'Trash'}</button>
        <button onClick={() => onAction('keep')} disabled={loading} style={{ padding: '3px 8px', borderRadius: radius.sm, background: 'rgba(255,255,255,0.05)', border: `1px solid ${color.border}`, color: color.textMuted, fontSize: 10, cursor: loading ? 'wait' : 'pointer', fontFamily: font.body }}>Keep</button>
      </div>
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════
// New Automation Modal (preserved from original)
// ═══════════════════════════════════════════════════════════════════════

function NewAutomationModal({ onClose, onCreated }: { onClose: () => void; onCreated: () => void }) {
  const pushOverlay = useCommandCenter(s => s.pushBrowserOverlay);
  const popOverlay = useCommandCenter(s => s.popBrowserOverlay);
  useEffect(() => { pushOverlay(); return () => { popOverlay(); }; }, [pushOverlay, popOverlay]);

  const [name, setName] = useState('');
  const [prompt, setPrompt] = useState('');
  const [selectedPreset, setSelectedPreset] = useState(0);
  const [customCron, setCustomCron] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const overlayRef = useRef<HTMLDivElement>(null);

  const cron = selectedPreset < CRON_PRESETS.length - 1 ? CRON_PRESETS[selectedPreset].cron : customCron;

  const handleSave = async () => {
    if (!name.trim() || !prompt.trim() || !cron.trim()) { setError('All fields are required.'); return; }
    setSaving(true); setError('');
    try {
      const recipe = { version: '1.0.0', title: name.trim(), description: prompt.trim().slice(0, 120), prompt: prompt.trim() };
      await apiFetch<unknown>('/schedule/create', {
        method: 'POST', body: JSON.stringify({ id: name.trim().replace(/\s+/g, '-').toLowerCase(), recipe, cron }),
      });
      onCreated();
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); setSaving(false); }
  };

  return (
    <div ref={overlayRef} onClick={e => e.target === overlayRef.current && onClose()} style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.6)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 1000 }}>
      <div style={{ width: 480, maxHeight: '80vh', overflowY: 'auto', background: color.bg, borderRadius: radius.lg, border: `1px solid ${color.border}`, padding: 28 }}>
        <div style={{ fontFamily: font.display, fontSize: 18, fontWeight: 600, marginBottom: 20 }}>New Automation</div>

        <label style={{ fontSize: 12, fontWeight: 600, color: color.textMuted, display: 'block', marginBottom: 6 }}>What should we call this?</label>
        <input value={name} onChange={e => setName(e.target.value)} placeholder="e.g., Weekly Cleanup" style={{
          width: '100%', padding: '8px 12px', borderRadius: radius.sm, background: 'rgba(20,28,48,0.4)',
          border: `1px solid ${color.border}`, color: color.text, fontSize: 13, fontFamily: font.body,
          outline: 'none', marginBottom: 16, boxSizing: 'border-box',
        }} />

        <label style={{ fontSize: 12, fontWeight: 600, color: color.textMuted, display: 'block', marginBottom: 6 }}>What should the agent do?</label>
        <textarea value={prompt} onChange={e => setPrompt(e.target.value)}
          placeholder="Scan my Downloads folder for files older than 30 days..." rows={4} style={{
            width: '100%', padding: '8px 12px', borderRadius: radius.sm, background: 'rgba(20,28,48,0.4)',
            border: `1px solid ${color.border}`, color: color.text, fontSize: 13, fontFamily: font.body,
            outline: 'none', resize: 'vertical', marginBottom: 16, boxSizing: 'border-box',
          }} />

        <label style={{ fontSize: 12, fontWeight: 600, color: color.textMuted, display: 'block', marginBottom: 6 }}>When should it run?</label>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4, marginBottom: 16 }}>
          {CRON_PRESETS.map((preset, i) => (
            <label key={i} style={{ display: 'flex', alignItems: 'center', gap: 8, cursor: 'pointer', padding: '4px 0' }}>
              <input type="radio" name="cron" checked={selectedPreset === i} onChange={() => setSelectedPreset(i)} style={{ accentColor: color.cyan }} />
              <span style={{ fontSize: 13, color: selectedPreset === i ? color.text : color.textMuted }}>{preset.label}</span>
            </label>
          ))}
        </div>

        {selectedPreset === CRON_PRESETS.length - 1 && (
          <div style={{ marginBottom: 16 }}>
            <input value={customCron} onChange={e => setCustomCron(e.target.value)} placeholder="0 9 * * 1-5" style={{
              width: '100%', padding: '8px 12px', borderRadius: radius.sm, background: 'rgba(20,28,48,0.4)',
              border: `1px solid ${color.border}`, color: color.text, fontSize: 13, fontFamily: font.mono,
              outline: 'none', boxSizing: 'border-box',
            }} />
            {customCron.trim() && <div style={{ fontSize: 11, color: color.textDim, marginTop: 4, fontFamily: font.mono }}>Preview: {cronToEnglish(customCron)}</div>}
          </div>
        )}

        {error && <div style={{ fontSize: 12, color: '#ff6b6b', marginBottom: 12 }}>{error}</div>}

        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
          <button onClick={onClose} style={{ padding: '8px 16px', borderRadius: radius.sm, background: 'transparent', border: `1px solid ${color.border}`, color: color.textMuted, fontSize: 12, cursor: 'pointer', fontFamily: font.body }}>Cancel</button>
          <button onClick={handleSave} disabled={saving} style={{ padding: '8px 20px', borderRadius: radius.sm, background: color.cyan, color: '#000', fontWeight: 600, fontSize: 12, border: 'none', cursor: saving ? 'wait' : 'pointer', fontFamily: font.body, opacity: saving ? 0.6 : 1 }}>{saving ? 'Creating...' : 'Create Automation'}</button>
        </div>
      </div>
    </div>
  );
}
