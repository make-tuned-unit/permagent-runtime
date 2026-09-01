import { useEffect, useState, useCallback, type CSSProperties } from 'react';
import { useCommandCenter } from '../../lib/store';
import { emitActivity } from '../../lib/emitActivity';
import { api, type InboxFile } from '../../lib/api';
import type { Project } from '../projects/types';
import {
  canRoute,
  describeRouteResult,
  fetchRoutableProjects,
  needsProject,
  projectLabel,
  routeInboxFile,
  statusLabel,
  type RouteDestination,
} from './inboxRouting';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme as useThemeHook } from '../../styles/useTheme';
import { Button } from '../common/Button';

function formatBytes(b: number | null): string {
  if (b == null) return '—';
  if (b < 1024) return `${b} B`;
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(0)} KB`;
  if (b < 1024 * 1024 * 1024) return `${(b / (1024 * 1024)).toFixed(1)} MB`;
  return `${(b / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function sourceLabel(url: string | null): string {
  if (!url) return '—';
  try { return new URL(url).hostname; } catch { return url; }
}

function receivedLabel(iso: string): string {
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  return d.toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
}

const COLS = '1fr 120px 64px 118px 118px 76px';

interface PendingPick {
  fileId: string;
  destination: Extract<RouteDestination, 'project' | 'scheduler'>;
  projectId: string;
}

type RowResult = { ok: boolean; text: string };

/** Per-(file, destination) result key — a Brain failure must not overwrite a
 *  Project success (or vice versa) on the same row when both were sent. */
function resultKey(fileId: string, destination: RouteDestination): string {
  return `${fileId}:${destination}`;
}

const DESTINATION_LABEL: Record<RouteDestination, string> = {
  brain: 'the Brain',
  project: 'the project',
  scheduler: 'the post scheduler',
};

/**
 * Downloads inbox (#392/#393/#395) — the files that landed in
 * ~/.permagent/inbox via the in-app browser download flow, with explicit
 * user-controlled routing to a destination surface: the Brain (Reader), a
 * project (documents), or the scheduler (a social_post draft card). Reads the
 * real `GET /api/inbox` and routes through the real
 * `POST /api/inbox/{id}/route`; Permagent never guesses where a file goes.
 * Hosted embedded inside Settings → Inbox (2026-08 Console consolidation:
 * `embedded` strips the header/Close chrome and the Escape handler, both of
 * which Settings provides). navigate_app("Inbox") deep-links to that pane.
 */
export function InboxPanel({ embedded = false }: { embedded?: boolean } = {}) {
  const { gradient, colors } = useThemeHook();
  const setActivePanel = useCommandCenter(s => s.setActivePanel);
  const focusBrainMemory = useCommandCenter(s => s.focusBrainMemory);
  const [files, setFiles] = useState<InboxFile[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [projects, setProjects] = useState<Project[] | null>(null);
  /** The project list failed to load. Both catches used to `setProjects([])`,
   *  which the picker then read as "No projects yet" — telling a user with a
   *  dozen boards that they have none, in the one control they need to file a
   *  file. The `error` state two lines up already does this correctly for
   *  `files`; this is the second fetch in the same file. */
  const [projectsError, setProjectsError] = useState(false);
  const [pick, setPick] = useState<PendingPick | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [results, setResults] = useState<Record<string, RowResult>>({});

  const dismiss = useCallback(() => setActivePanel('chat'), [setActivePanel]);

  // The panel mounts only while open (App renders it behind `showInbox`), so a
  // mount effect is a faithful "user opened the Inbox" engagement signal.
  useEffect(() => {
    emitActivity('inbox_opened', 'inbox');
  }, []);

  useEffect(() => {
    let active = true;
    api.getInbox()
      .then(rows => { if (active) { setFiles(rows); setError(null); } })
      .catch(() => { if (active) { setFiles([]); setError('Could not load your inbox.'); } });
    return () => { active = false; };
  }, []);

  // Projects load on mount now, not only when a picker opens: the Project
  // column needs names to show, and a file that HAS been filed must look
  // different from one that has not.
  useEffect(() => {
    let active = true;
    fetchRoutableProjects()
      .then(rows => { if (active) { setProjects(rows); setProjectsError(false); } })
      .catch(() => { if (active) setProjectsError(true); });
    return () => { active = false; };
  }, []);

  useEffect(() => {
    if (embedded) return; // the hosting Settings view owns Escape
    const h = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); dismiss(); } };
    window.addEventListener('keydown', h);
    return () => window.removeEventListener('keydown', h);
  }, [dismiss, embedded]);

  // Projects load lazily, the first time a project-scoped picker opens.
  const ensureProjects = useCallback(() => {
    if (projects !== null) return;
    fetchRoutableProjects()
      .then(rows => { setProjects(rows); setProjectsError(false); })
      .catch(() => setProjectsError(true));
  }, [projects]);

  // Resolves whether the route actually landed. The catch below turns a failure
  // into a row message rather than rethrowing, so without this the button that
  // triggered it would tick "done" over the top of "Could not route …".
  const sendTo = useCallback(async (file: InboxFile, destination: RouteDestination, projectId?: string) => {
    if (needsProject(destination) && !projectId) return; // picker enforces this; belt-and-braces
    const key = resultKey(file.id, destination);
    setBusyId(file.id);
    // Clear only THIS destination's prior result — a Brain retry must not
    // wipe a Project success already showing on the same row.
    setResults(r => { const { [key]: _drop, ...rest } = r; return rest; });
    try {
      const resp = await routeInboxFile(file.id, destination, projectId);
      const projectName = projects?.find(p => p.id === projectId)?.name;
      setFiles(fs => (fs ?? []).map(f => (f.id === file.id ? resp.file : f)));
      setResults(r => ({ ...r, [key]: { ok: true, text: describeRouteResult(resp, projectName) } }));
      setPick(null);
      if (destination === 'brain' && resp.memory_key) {
        focusBrainMemory({
          key: resp.memory_key,
          preview: resp.summary ? { text: resp.summary, description: file.filename } : null,
        });
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      const dest = DESTINATION_LABEL[destination];
      setResults(r => ({ ...r, [key]: { ok: false, text: `Could not send ${file.filename} to ${dest}: ${msg}` } }));
    } finally {
      setBusyId(null);
    }
  }, [projects, focusBrainMemory]);

  const openPicker = useCallback((file: InboxFile, destination: 'project' | 'scheduler') => {
    ensureProjects();
    setPick(p =>
      p && p.fileId === file.id && p.destination === destination
        ? null
        : { fileId: file.id, destination, projectId: '' },
    );
  }, [ensureProjects]);

  // `onClick` is `unknown`-returning on purpose: the routing handlers are
  // async, and handing their promise to the primitive is what makes the round
  // trip visible.
  const actionBtn = (label: string, title: string, onClick: () => unknown, disabled: boolean, active = false) => (
    <Button
      colors={colors}
      onClick={onClick}
      disabled={disabled}
      title={title}
      style={{
        '--pa-btn-bg': active ? colors.surface : 'transparent',
        '--pa-btn-fg': disabled ? colors.textDim : colors.text,
        '--pa-btn-border': colors.border,
        '--pa-btn-bg-hover': active ? colors.surface : colors.surfaceHi,
        '--pa-btn-border-hover': colors.borderHi,
        '--pa-btn-bg-active': colors.surface,
        '--pa-btn-pad': '0 10px',
        '--pa-btn-radius': `${radius.sm}px`,
        height: 24,
        fontFamily: font.body,
        fontSize: textSize.micro,
        whiteSpace: 'nowrap',
      } as CSSProperties}
    >{label}</Button>
  );

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: gradient.shell, color: colors.text, fontFamily: font.body }}>
      {!embedded && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '20px 32px', borderBottom: `1px solid ${colors.border}` }}>
          <div style={{ flex: 1 }}>
            <div style={{ fontFamily: font.display, fontSize: 18, fontWeight: 700, letterSpacing: '-0.01em' }}>Downloads inbox</div>
            <div style={{ fontSize: textSize.caption, color: colors.textMuted, marginTop: 2 }}>
              Files you download in the in-app browser land here — send them to the Brain, a project, or the post scheduler. You choose; nothing is routed for you.
            </div>
          </div>
          <Button
            colors={colors}
            onClick={dismiss}
            style={{
              '--pa-btn-fg': colors.textMuted,
              '--pa-btn-fg-hover': colors.text,
              '--pa-btn-pad': '0 12px',
              '--pa-btn-radius': `${radius.md}px`,
              height: 30,
              fontFamily: font.body,
              fontSize: textSize.caption,
            } as CSSProperties}
          >Close</Button>
        </div>
      )}
      <div style={{ flex: 1, overflow: 'auto', padding: embedded ? '16px 20px' : '20px 32px' }}>
        {files === null ? (
          <div style={{ color: colors.textDim, fontSize: textSize.small }}>Loading inbox…</div>
        ) : files.length === 0 ? (
          <div style={{ color: colors.textMuted, fontSize: textSize.small, padding: '24px 0' }}>
            {error ?? 'Your inbox is empty. Download a file in the in-app browser — send it to Brain and it becomes searchable memory.'}
          </div>
        ) : (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            <div style={{ display: 'grid', gridTemplateColumns: COLS, gap: 12, padding: '0 12px', fontSize: 10, fontWeight: 600, letterSpacing: '0.08em', textTransform: 'uppercase', color: colors.textDim }}>
              <div>Filename</div><div>Source</div><div>Size</div><div>Received</div><div>Project</div><div>Status</div>
            </div>
            {files.map(f => {
              const busy = busyId === f.id;
              const routable = canRoute(f.status) && !busy;
              const rowPick = pick?.fileId === f.id ? pick : null;
              // Every destination this row has been sent to keeps its own
              // result — a later Brain failure must not erase an earlier
              // Project success on the same row (#3).
              const rowResults = (['brain', 'project', 'scheduler'] as const)
                .map(dest => ({ dest, result: results[resultKey(f.id, dest)] }))
                .filter((r): r is { dest: RouteDestination; result: RowResult } => Boolean(r.result));
              return (
                <div key={f.id} style={{ borderRadius: 10, background: colors.bgDeeper, border: `1px solid ${colors.border}` }}>
                  <div style={{ display: 'grid', gridTemplateColumns: COLS, gap: 12, alignItems: 'center', padding: '12px 12px 4px' }}>
                    <div style={{ minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: textSize.small, fontWeight: 600 }} title={f.filename}>{f.filename}</div>
                    <div style={{ minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: textSize.caption, color: colors.textMuted }} title={f.original_url ?? undefined}>{sourceLabel(f.original_url)}</div>
                    <div style={{ fontSize: textSize.caption, color: colors.textMuted, fontFamily: font.mono }}>{formatBytes(f.size_bytes)}</div>
                    <div style={{ fontSize: textSize.caption, color: colors.textMuted }}>{receivedLabel(f.created_at)}</div>
                    <div
                      style={{ minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: textSize.caption, color: f.project_id ? colors.text : colors.textDim }}
                      title={projectLabel(f.project_id, projects) ?? 'Not filed to a project yet'}
                    >{projectLabel(f.project_id, projects) ?? '—'}</div>
                    <div>
                      <span style={{ fontSize: 10, fontWeight: 600, letterSpacing: '0.04em', padding: '2px 8px', borderRadius: radius.pill, border: `1px solid ${colors.border}`, color: f.status === 'received' ? colors.text : colors.textMuted }}>
                        {statusLabel(f.status)}
                      </span>
                    </div>
                  </div>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '6px 12px 10px', flexWrap: 'wrap' }}>
                    <span style={{ fontSize: 10, color: colors.textDim, letterSpacing: '0.06em', textTransform: 'uppercase' }}>Send to</span>
                    {actionBtn(busy ? 'Sending…' : 'Brain', 'The Reader reads it and stores the text in your Brain', () => sendTo(f, 'brain'), !routable)}
                    {actionBtn('Project…', 'File it as a document on a project', () => openPicker(f, 'project'), !routable, rowPick?.destination === 'project')}
                    {actionBtn('Post…', 'Draft it as a social post card on a project board', () => openPicker(f, 'scheduler'), !routable, rowPick?.destination === 'scheduler')}
                  </div>
                  {rowResults.length > 0 && (
                    // One line per destination this file was sent to (#3) — a
                    // Brain failure must not hide an earlier Project success,
                    // and every line names the destination it is about.
                    <div style={{ display: 'flex', flexDirection: 'column', gap: 2, padding: '0 12px 10px' }}>
                      {rowResults.map(({ dest, result }) => (
                        <span key={dest} style={{ fontSize: 11, color: result.ok ? colors.textMuted : colors.danger }}>
                          {result.text}
                        </span>
                      ))}
                    </div>
                  )}
                  {rowPick && (
                    <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '0 12px 12px' }}>
                      <select
                        value={rowPick.projectId}
                        onChange={e => setPick({ ...rowPick, projectId: e.target.value })}
                        style={{ height: 26, borderRadius: radius.sm, background: 'transparent', border: `1px solid ${colors.border}`, color: colors.text, fontFamily: font.body, fontSize: textSize.caption, maxWidth: 280 }}
                      >
                        <option value="" disabled>
                          {projectsError
                            ? "Couldn't load your projects"
                            : projects === null ? 'Loading projects…'
                            : projects.length === 0 ? 'No projects yet'
                            : 'Choose a project'}
                        </option>
                        {(projects ?? []).map(p => (
                          <option key={p.id} value={p.id}>{p.name}</option>
                        ))}
                      </select>
                      {actionBtn(
                        busy ? 'Sending…' : rowPick.destination === 'project' ? 'File it' : 'Draft post',
                        rowPick.destination === 'project' ? 'Add it to this project’s documents' : 'Create the social post draft on this project’s board',
                        () => sendTo(f, rowPick.destination, rowPick.projectId),
                        !routable || !rowPick.projectId,
                      )}
                      {actionBtn('Cancel', 'Close the project picker', () => setPick(null), busy)}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
