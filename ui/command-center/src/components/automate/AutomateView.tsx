import { useEffect, useState, useCallback, useRef } from 'react';
import { color, font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { cronToEnglish } from '../../lib/schedule-format';

const API = 'http://127.0.0.1:3001';

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

type Tab = 'automations' | 'runs';

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

export function AutomateView() {
  const [jobs, setJobs] = useState<ScheduledJob[]>([]);
  const [sessions, setSessions] = useState<Map<string, SessionInfo[]>>(new Map());
  const [tab, setTab] = useState<Tab>('automations');
  const [showModal, setShowModal] = useState(false);
  const [expandedRunId, setExpandedRunId] = useState<string | null>(null);
  const [completionToast, setCompletionToast] = useState<string | null>(null);
  const prevRunningRef = useRef<Set<string>>(new Set());
  const { gradient } = useTheme();

  const jobNameMap = new Map(jobs.map(j => [j.id, j.display_name || j.id]));

  const fetchJobs = useCallback(async () => {
    try {
      const res = await fetch(`${API}/schedule/list`);
      if (res.ok) {
        const data = await res.json();
        const newJobs: ScheduledJob[] = data.jobs || [];
        setJobs(newJobs);

        // Detect job completion: was running, now not
        const nowRunning = new Set(newJobs.filter(j => j.currently_running).map(j => j.id));
        const prev = prevRunningRef.current;
        for (const id of prev) {
          if (!nowRunning.has(id)) {
            const name = newJobs.find(j => j.id === id)?.display_name || id;
            setCompletionToast(name);
            setTab('runs');
            setTimeout(() => setCompletionToast(null), 8000);
          }
        }
        prevRunningRef.current = nowRunning;
      }
    } catch {}
  }, []);

  const fetchAllSessions = useCallback(async (jobIds: string[]) => {
    for (const jobId of jobIds) {
      try {
        const res = await fetch(`${API}/schedule/${encodeURIComponent(jobId)}/sessions?limit=10`);
        if (res.ok) {
          const data: SessionInfo[] = await res.json();
          setSessions(prev => new Map(prev).set(jobId, data));
        }
      } catch {}
    }
  }, []);

  // Poll every 5 seconds
  useEffect(() => {
    fetchJobs();
    const interval = setInterval(fetchJobs, 5000);
    return () => clearInterval(interval);
  }, [fetchJobs]);

  // Re-fetch sessions every poll cycle (catches new runs)
  useEffect(() => {
    if (jobs.length > 0) {
      fetchAllSessions(jobs.map(j => j.id));
    }
    const interval = setInterval(() => {
      if (jobs.length > 0) fetchAllSessions(jobs.map(j => j.id));
    }, 5000);
    return () => clearInterval(interval);
  }, [jobs.length]); // eslint-disable-line react-hooks/exhaustive-deps

  const allRuns = Array.from(sessions.entries())
    .flatMap(([jobId, runs]) => runs.map(r => ({ ...r, jobId })))
    .sort((a, b) => new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime());

  const handleRunNow = async (id: string) => {
    try { await fetch(`${API}/schedule/${encodeURIComponent(id)}/run_now`, { method: 'POST' }); fetchJobs(); } catch {}
  };
  const handlePause = async (id: string) => {
    try { await fetch(`${API}/schedule/${encodeURIComponent(id)}/pause`, { method: 'POST' }); fetchJobs(); } catch {}
  };
  const handleUnpause = async (id: string) => {
    try { await fetch(`${API}/schedule/${encodeURIComponent(id)}/unpause`, { method: 'POST' }); fetchJobs(); } catch {}
  };
  const handleDelete = async (id: string) => {
    const name = jobNameMap.get(id) || id;
    if (!confirm(`Delete "${name}"? This can't be undone.`)) return;
    try { await fetch(`${API}/schedule/delete/${encodeURIComponent(id)}`, { method: 'DELETE' }); fetchJobs(); } catch {}
  };
  const handleKill = async (id: string) => {
    try { await fetch(`${API}/schedule/${encodeURIComponent(id)}/kill`, { method: 'POST' }); fetchJobs(); } catch {}
  };

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: gradient.workspace, color: color.text, fontFamily: font.body }}>
      {/* Header */}
      <div style={{ padding: '20px 32px 0', flexShrink: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <div>
            <div style={{ fontFamily: font.display, fontSize: 20, fontWeight: 600 }}>Automate</div>
            <div style={{ fontSize: 12, color: color.textMuted, marginTop: 4 }}>Scheduled tasks that run on your behalf.</div>
          </div>
          <button onClick={() => setShowModal(true)} style={{
            padding: '8px 16px', borderRadius: radius.md, background: color.cyan, color: '#000',
            fontWeight: 600, fontSize: 12, border: 'none', cursor: 'pointer', fontFamily: font.body,
          }}>+ New Automation</button>
        </div>
        <div style={{ display: 'flex', gap: 0, marginTop: 16, borderBottom: `1px solid ${color.border}` }}>
          {([['automations', 'Active Automations'], ['runs', 'Recent Runs']] as const).map(([id, label]) => (
            <button key={id} onClick={() => setTab(id)} style={{
              padding: '8px 16px', fontFamily: font.body, fontSize: 12, fontWeight: 600,
              color: tab === id ? color.cyan : color.textMuted, background: 'transparent',
              border: 'none', cursor: 'pointer',
              borderBottom: tab === id ? `2px solid ${color.cyan}` : '2px solid transparent',
            }}>{label}</button>
          ))}
        </div>
      </div>

      {/* Completion toast */}
      {completionToast && (
        <div style={{
          margin: '12px 32px 0', padding: '10px 16px', borderRadius: radius.md,
          background: 'rgba(91,209,127,0.1)', border: '1px solid rgba(91,209,127,0.25)',
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        }}>
          <span style={{ fontSize: 13, color: '#5BD17F' }}>
            "{completionToast}" completed. Results are below.
          </span>
          <button onClick={() => setCompletionToast(null)} style={{
            background: 'none', border: 'none', color: color.textDim, cursor: 'pointer', fontSize: 16,
          }}>x</button>
        </div>
      )}

      {/* Content */}
      <div style={{ flex: 1, minHeight: 0, overflowY: 'auto', padding: '20px 32px' }}>
        {tab === 'automations' && (
          jobs.length === 0 ? <EmptyAutomations onNew={() => setShowModal(true)} /> : (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
              {jobs.map(job => <AutomationCard key={job.id} job={job} onRunNow={handleRunNow} onPause={handlePause} onUnpause={handleUnpause} onDelete={handleDelete} onKill={handleKill} />)}
            </div>
          )
        )}
        {tab === 'runs' && (
          allRuns.length === 0 ? <EmptyRuns /> : (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              {allRuns.map(run => (
                <RunRow key={run.id} run={run} displayName={jobNameMap.get(run.jobId) || run.jobId}
                  expanded={expandedRunId === run.id} onToggle={() => setExpandedRunId(prev => prev === run.id ? null : run.id)} />
              ))}
            </div>
          )
        )}
      </div>
      {showModal && <NewAutomationModal onClose={() => setShowModal(false)} onCreated={() => { setShowModal(false); fetchJobs(); }} />}
    </div>
  );
}

// ── Automation Card ─────────────────────────────────────────────────

function AutomationCard({ job, onRunNow, onPause, onUnpause, onDelete, onKill }: {
  job: ScheduledJob;
  onRunNow: (id: string) => void;
  onPause: (id: string) => void;
  onUnpause: (id: string) => void;
  onDelete: (id: string) => void;
  onKill: (id: string) => void;
}) {
  const name = job.display_name || job.id;
  const desc = job.description || '';
  const schedule = cronToEnglish(job.cron);
  const statusColor = job.currently_running ? '#5BD17F' : job.paused ? color.textDim : color.cyan;

  let statusLine = 'Never run yet';
  if (job.currently_running) statusLine = 'Running now...';
  else if (job.last_run) statusLine = `Ran ${timeAgo(job.last_run)}`;

  return (
    <div style={{
      padding: '20px 24px', borderRadius: radius.lg,
      background: 'rgba(20,28,48,0.5)', border: `1px solid ${color.border}`,
    }}>
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 14 }}>
        <div style={{
          width: 10, height: 10, borderRadius: '50%', background: statusColor, flexShrink: 0, marginTop: 5,
          boxShadow: job.currently_running ? `0 0 8px ${statusColor}` : 'none',
        }} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 15, fontWeight: 600, fontFamily: font.display }}>{name}</div>
          {desc && <div style={{ fontSize: 12, color: color.textMuted, marginTop: 4, lineHeight: 1.5 }}>{desc}</div>}
          <div style={{ fontSize: 11, color: color.textDim, marginTop: 8, fontFamily: font.mono }}>
            {schedule} &middot; {statusLine}
          </div>
        </div>
      </div>
      <div style={{ display: 'flex', gap: 8, marginTop: 16 }}>
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

// ── Run Row (fetches actual session content when expanded) ──────────

function RunRow({ run, displayName, expanded, onToggle }: {
  run: SessionInfo & { jobId: string };
  displayName: string;
  expanded: boolean;
  onToggle: () => void;
}) {
  const [reportText, setReportText] = useState<string | null>(null);
  const [findings, setFindings] = useState<Finding[]>([]);
  const [actionInFlight, setActionInFlight] = useState<string | null>(null);
  const [loadingContent, setLoadingContent] = useState(false);

  // Fetch session content + findings when expanded
  useEffect(() => {
    if (!expanded) return;
    let cancelled = false;

    const loadContent = async () => {
      setLoadingContent(true);

      // Fetch session messages for the report text
      try {
        const res = await fetch(`${API}/api/sessions/${encodeURIComponent(run.id)}`);
        if (res.ok && !cancelled) {
          const session = await res.json();
          const msgs = session.conversation || [];
          // Extract assistant text messages, skip tool calls
          const text = msgs
            .filter((m: any) => m.role === 'assistant')
            .flatMap((m: any) => (m.content || [])
              .filter((c: any) => c.type === 'text')
              .map((c: any) => c.text))
            .join('\n\n');
          // Strip <findings> block from display text
          const cleaned = text.replace(/<findings>[\s\S]*?<\/findings>/g, '').trim();
          setReportText(cleaned || 'No output captured.');
        }
      } catch {}

      // Fetch structured findings
      try {
        const token = localStorage.getItem('daemon_token') || '';
        const headers: Record<string, string> = {};
        if (token) headers['Authorization'] = `Bearer ${token}`;
        const res = await fetch(`${API}/automation/run/${encodeURIComponent(run.id)}/findings`, { headers });
        if (res.ok && !cancelled) {
          const data = await res.json();
          setFindings(data.findings || []);
        }
      } catch {}

      if (!cancelled) setLoadingContent(false);
    };

    loadContent();
    return () => { cancelled = true; };
  }, [expanded, run.id]);

  const handleAction = async (findingId: string, action: string) => {
    setActionInFlight(findingId);
    try {
      const token = localStorage.getItem('daemon_token') || '';
      const headers: Record<string, string> = { 'Content-Type': 'application/json' };
      if (token) headers['Authorization'] = `Bearer ${token}`;
      const res = await fetch(`${API}/automation/finding/${encodeURIComponent(findingId)}/action`, {
        method: 'POST', headers, body: JSON.stringify({ action, run_id: run.id }),
      });
      if (res.ok) {
        const result = await res.json();
        setFindings(prev => prev.map(f => f.id === findingId
          ? { ...f, action_taken: result.action_taken, actioned_at: result.timestamp, size_recovered_bytes: result.size_recovered_bytes }
          : f));
      } else {
        const err = await res.json().catch(() => ({ error: 'Unknown error' }));
        alert(err.error || `Action failed: ${res.status}`);
      }
    } catch (e) { alert(`Action failed: ${e}`); }
    setActionInFlight(null);
  };

  const totalRecovered = findings.filter(f => f.action_taken === 'trashed').reduce((s, f) => s + (f.size_recovered_bytes || 0), 0);
  const allActioned = findings.length > 0 && findings.every(f => f.action_taken !== null);

  return (
    <div style={{
      borderRadius: radius.md,
      background: expanded ? 'rgba(20,28,48,0.7)' : 'rgba(20,28,48,0.3)',
      border: `1px solid ${expanded ? color.borderHi : color.border}`,
      overflow: 'hidden',
    }}>
      <button onClick={onToggle} style={{
        width: '100%', padding: '12px 16px', display: 'flex', alignItems: 'center', gap: 12,
        background: 'transparent', border: 'none', cursor: 'pointer', textAlign: 'left',
        color: color.text, fontFamily: font.body,
      }}>
        <div style={{ width: 6, height: 6, borderRadius: '50%', background: color.cyan, flexShrink: 0 }} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 13, fontWeight: 600 }}>{displayName}</div>
        </div>
        <div style={{ fontSize: 11, color: color.textMuted, fontFamily: font.mono, flexShrink: 0 }}>
          {run.messageCount} msgs
        </div>
        <div style={{ fontSize: 11, color: color.textDim, flexShrink: 0 }}>{timeAgo(run.createdAt)}</div>
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke={color.textDim} strokeWidth={2}
          style={{ transform: expanded ? 'rotate(180deg)' : 'rotate(0deg)', transition: 'transform 150ms' }}>
          <path d="M6 9l6 6 6-6" />
        </svg>
      </button>
      {expanded && (
        <div style={{ padding: '0 16px 16px', borderTop: `1px solid ${color.border}` }}>
          {loadingContent ? (
            <div style={{ fontSize: 12, color: color.textDim, marginTop: 12 }}>Loading results...</div>
          ) : (
            <>
              {/* Report text */}
              {reportText && (
                <div style={{
                  marginTop: 12, padding: '12px 14px', borderRadius: radius.sm,
                  background: 'rgba(10,14,23,0.6)', border: `1px solid ${color.border}`,
                  fontSize: 12, color: color.textMuted, lineHeight: 1.7, fontFamily: font.body,
                  maxHeight: 400, overflowY: 'auto', whiteSpace: 'pre-wrap', wordBreak: 'break-word',
                }}>
                  {reportText}
                </div>
              )}

              {/* Metadata */}
              <div style={{ fontSize: 11, color: color.textDim, marginTop: 8 }}>
                {new Date(run.createdAt).toLocaleString()} &middot; {run.totalTokens ?? 0} tokens
              </div>

              {/* Findings */}
              {findings.length > 0 && (
                <div style={{ marginTop: 16 }}>
                  <div style={{ fontSize: 11, fontWeight: 600, color: color.textMuted, textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 8 }}>
                    Actionable Findings ({findings.length})
                  </div>
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                    {findings.map(f => <FindingRow key={f.id} finding={f} loading={actionInFlight === f.id} onAction={(a) => handleAction(f.id, a)} />)}
                  </div>
                  {allActioned && totalRecovered > 0 && (
                    <div style={{ marginTop: 12, padding: '8px 12px', borderRadius: radius.sm, background: 'rgba(91,209,127,0.08)', border: '1px solid rgba(91,209,127,0.2)', fontSize: 12, color: '#5BD17F' }}>
                      Recovered {formatBytes(totalRecovered)} across {findings.filter(f => f.action_taken === 'trashed').length} items.
                    </div>
                  )}
                </div>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}

// ── Finding Row ─────────────────────────────────────────────────────

function FindingRow({ finding, loading, onAction }: { finding: Finding; loading: boolean; onAction: (a: string) => void }) {
  const fileName = finding.path.split('/').pop() || finding.path;
  const dirPath = finding.path.replace(/\/[^/]+$/, '').replace(/^\/Users\/[^/]+/, '~');

  if (finding.action_taken === 'trashed') {
    return (
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '6px 10px', borderRadius: radius.sm, background: 'rgba(91,209,127,0.05)', border: '1px solid rgba(91,209,127,0.12)' }}>
        <span style={{ fontSize: 12, color: '#5BD17F' }}>Moved to Trash</span>
        <span style={{ fontSize: 11, color: color.textDim, flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{fileName}</span>
        {finding.size_recovered_bytes != null && <span style={{ fontSize: 11, color: '#5BD17F', fontFamily: font.mono }}>+{formatBytes(finding.size_recovered_bytes)}</span>}
      </div>
    );
  }
  if (finding.action_taken === 'kept' || finding.action_taken === 'skipped') {
    return (
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '6px 10px', borderRadius: radius.sm, background: 'rgba(255,255,255,0.02)', border: `1px solid ${color.border}`, opacity: 0.6 }}>
        <span style={{ fontSize: 12, color: color.textMuted }}>Kept</span>
        <span style={{ fontSize: 11, color: color.textDim, flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{fileName}</span>
      </div>
    );
  }
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '8px 10px', borderRadius: radius.sm, background: 'rgba(20,28,48,0.5)', border: `1px solid ${color.border}` }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 12, fontWeight: 500, color: color.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{fileName}</div>
        <div style={{ fontSize: 10, color: color.textDim, fontFamily: font.mono, marginTop: 2 }}>
          {dirPath} &middot; {formatBytes(finding.size_bytes)}{finding.age_days != null && <> &middot; {finding.age_days}d old</>}
        </div>
      </div>
      <div style={{ display: 'flex', gap: 4, flexShrink: 0 }}>
        <button onClick={() => onAction('trash')} disabled={loading} style={{
          padding: '3px 8px', borderRadius: radius.sm, background: 'rgba(255,100,100,0.1)',
          border: '1px solid rgba(255,100,100,0.2)', color: '#ff6b6b', fontSize: 10,
          fontWeight: 600, cursor: loading ? 'wait' : 'pointer', fontFamily: font.body, opacity: loading ? 0.5 : 1,
        }}>{loading ? '...' : 'Move to Trash'}</button>
        <button onClick={() => onAction('keep')} disabled={loading} style={{
          padding: '3px 8px', borderRadius: radius.sm, background: 'rgba(255,255,255,0.05)',
          border: `1px solid ${color.border}`, color: color.textMuted, fontSize: 10,
          fontWeight: 500, cursor: loading ? 'wait' : 'pointer', fontFamily: font.body, opacity: loading ? 0.5 : 1,
        }}>Keep</button>
      </div>
    </div>
  );
}

// ── Empty States ────────────────────────────────────────────────────

function EmptyAutomations({ onNew }: { onNew: () => void }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', height: '100%', minHeight: 300, gap: 16, textAlign: 'center' }}>
      <div style={{ width: 56, height: 56, borderRadius: 14, background: 'rgba(141,68,174,0.08)', border: '1px solid rgba(141,68,174,0.20)', display: 'grid', placeItems: 'center' }}>
        <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke={color.purple} strokeWidth={1.5} strokeLinecap="round" strokeOpacity={0.6}>
          <path d="M12 2v4M12 18v4M4.93 4.93l2.83 2.83M16.24 16.24l2.83 2.83M2 12h4M18 12h4M4.93 19.07l2.83-2.83M16.24 7.76l2.83-2.83" />
        </svg>
      </div>
      <div style={{ fontSize: 16, fontWeight: 600, fontFamily: font.display }}>No automations yet</div>
      <div style={{ fontSize: 13, color: color.textMuted, maxWidth: 360, lineHeight: 1.5 }}>
        Create scheduled tasks that run on your behalf — daily briefings, weekly cleanups, and more.
      </div>
      <button onClick={onNew} style={{ padding: '8px 20px', borderRadius: radius.md, background: color.cyan, color: '#000', fontWeight: 600, fontSize: 13, border: 'none', cursor: 'pointer', marginTop: 8, fontFamily: font.body }}>
        Create your first automation
      </button>
    </div>
  );
}

function EmptyRuns() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', height: '100%', minHeight: 200, gap: 12, textAlign: 'center' }}>
      <div style={{ fontSize: 14, fontWeight: 500, color: color.textMuted }}>No runs yet</div>
      <div style={{ fontSize: 12, color: color.textDim, maxWidth: 320 }}>
        Your automations haven't run yet. Click <strong>Run Now</strong> on any automation to see it in action.
      </div>
    </div>
  );
}

// ── New Automation Modal ────────────────────────────────────────────

function NewAutomationModal({ onClose, onCreated }: { onClose: () => void; onCreated: () => void }) {
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
      const res = await fetch(`${API}/schedule/create`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ id: name.trim().replace(/\s+/g, '-').toLowerCase(), recipe, cron }),
      });
      if (!res.ok) { const data = await res.json().catch(() => ({ message: 'Unknown error' })); setError(data.message || `Error ${res.status}`); setSaving(false); return; }
      onCreated();
    } catch (e) { setError(String(e)); setSaving(false); }
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
          placeholder="Scan my Downloads folder for files older than 30 days and report the largest ones..." rows={4} style={{
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
            {customCron.trim() && (
              <div style={{ fontSize: 11, color: color.textDim, marginTop: 4, fontFamily: font.mono }}>
                Preview: {cronToEnglish(customCron)}
              </div>
            )}
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
