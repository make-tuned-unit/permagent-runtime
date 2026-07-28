import { useState, useEffect, useCallback, type ReactNode } from 'react';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { apiFetch } from '../../lib/api';
import { useGoalEvents } from '../../lib/useGoalEvents';
import { useBrowserNavigate } from '../../hooks/useBrowserNavigate';
import { useCommandCenter, navigateToTool } from '../../lib/store';
import { Panel } from './Panel';
import { ActivityPanel } from './ActivityPanel';
import { readBrief, readLinks, normalizeUrl, saveProjectSummary, type WorkspaceLink } from './workspaceMeta';
import { PublishSequencePanel } from './PublishSequencePanel';
import type { Project, BoardColumn, Card } from './types';

// ── Project Overview ────────────────────────────────────────────────────────
//
// The "command-center dash" lens of the Projects tab (#471, Layer 1). A
// two-column read of a single project. LEFT = substance, RIGHT = people +
// action. Ships Summary, Key Facts, Links, Tasks, People, Documents (the
// #471 Layer 2 document hub + in-app viewer), Notes (freeform notes the
// user writes, indexed into the Brain), and Stack (#512 — which services the
// project runs on + which login identity per service, reference-only, no
// secrets). The Memories panel closes the Brain
// loop: it reads back the Brain memories this project's own surfaces wrote
// (notes / documents / indexed code), each deep-linkable into the Brain view.

export function ProjectOverview({ project, onProjectUpdated }: {
  project: Project;
  /** Parent refetch after a summary edit persists (ProjectsView.loadProjects). */
  onProjectUpdated?: () => void;
}) {
  const { colors, gradient } = useTheme();
  const [columns, setColumns] = useState<BoardColumn[]>([]);
  const [cards, setCards] = useState<Card[]>([]);
  const openGoalDetail = useCommandCenter(s => s.openGoalDetail);
  const growProject = useCommandCenter(s => s.growProject);

  const loadBoard = useCallback(async () => {
    try {
      const [cols, cds] = await Promise.all([
        apiFetch<BoardColumn[]>(`/api/projects/${project.id}/columns`),
        apiFetch<Card[]>(`/api/projects/${project.id}/cards`),
      ]);
      setColumns(cols);
      setCards(cds);
    } catch {
      // silently fail
    }
  }, [project.id]);

  useEffect(() => { loadBoard(); }, [loadBoard]);
  // Live task status — refetch on any goal create/transition (#473).
  useGoalEvents(loadBoard);

  return (
    <div style={{
      flex: 1, overflow: 'auto', background: gradient.workspace,
      color: colors.text, fontFamily: font.body,
    }}>
      <div style={{
        display: 'grid', gridTemplateColumns: 'minmax(0, 1.3fr) minmax(0, 1fr)',
        gap: 16, padding: '20px 24px', alignItems: 'start',
      }}>
        {/* LEFT — what this is, and what just happened.
            The records themselves (stack, documents, notes, memories, people,
            ecosystem, links, code index) live in the Details lens — see the
            ProjectDetails header for the ruled division. */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 16, minWidth: 0 }}>
          <SummaryPanel project={project} onProjectUpdated={onProjectUpdated} />
          <WatcherInsightsPanel project={project} />
          <KeyFactsPanel project={project} />
          <ActivityPanel project={project} />
        </div>

        {/* RIGHT — where it stands, and what's next */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 16, minWidth: 0 }}>
          {/* Build → Grow bridge: take the finished work to market. */}
          <button
            onClick={() => growProject(project.id)}
            style={{
              display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 8,
              padding: '11px 14px', borderRadius: 12, cursor: 'pointer',
              background: `linear-gradient(90deg, ${colors.cyan}22, ${colors.purple}22)`,
              border: `1px solid ${colors.borderHi}`, color: colors.text,
              fontFamily: font.body, fontSize: 13, fontWeight: 600,
            }}
          >
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke={colors.cyan} strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
              <path d="M3 17l6-6 4 4 8-8M17 7h4v4" />
            </svg>
            Grow this project
          </button>
          <TasksPanel
            columns={columns}
            cards={cards}
            onOpenGoal={(cardId) => openGoalDetail(project.id, cardId)}
          />
          {/* Publish sequence (#457) — post-push steps before "live"; the
              orchestrator reads the same metadata key at dispatch/review. */}
          <PublishSequencePanel project={project} />
        </div>
      </div>
    </div>
  );
}

// ── Left-column panels ──────────────────────────────────────────────────────

/**
 * Summary — name, one-line description, and the long-form brief (#472 residue,
 * `metadata_json.brief`). Editable in place: Edit swaps the panel body for a
 * description input + brief textarea, saved through the existing
 * PATCH /api/projects/:id (no new endpoints). Draft state is initialized on
 * entering edit mode only, so the 5s projects poll can't clobber typing.
 */
function SummaryPanel({ project, onProjectUpdated }: {
  project: Project;
  onProjectUpdated?: () => void;
}) {
  const { colors } = useTheme();
  const brief = readBrief(project.metadataJson);
  const [editing, setEditing] = useState(false);
  const [draftDescription, setDraftDescription] = useState('');
  const [draftBrief, setDraftBrief] = useState('');
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const startEditing = () => {
    setDraftDescription(project.description);
    setDraftBrief(brief);
    setSaveError(null);
    setEditing(true);
  };

  const save = async () => {
    setSaving(true);
    setSaveError(null);
    try {
      await saveProjectSummary(project.id, { description: draftDescription.trim(), brief: draftBrief });
      setEditing(false);
      onProjectUpdated?.();
    } catch (e) {
      // Keep the draft on screen — a failed save must not eat the user's text.
      setSaveError(e instanceof Error ? e.message : 'Save failed');
    } finally {
      setSaving(false);
    }
  };

  return (
    <Panel
      title="Summary"
      action={!editing ? (
        <button onClick={startEditing} style={panelActionBtn(colors)}>Edit</button>
      ) : undefined}
    >
      <div style={{ fontFamily: font.display, fontSize: 18, fontWeight: 600, letterSpacing: '-0.01em' }}>
        {project.name}
      </div>

      {editing ? (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8, marginTop: 8 }}>
          <label style={fieldLabel(colors)}>
            Description
            <input
              value={draftDescription}
              onChange={e => setDraftDescription(e.target.value)}
              placeholder="One-line description"
              disabled={saving}
              style={fieldInput(colors)}
            />
          </label>
          <label style={fieldLabel(colors)}>
            Brief
            <textarea
              value={draftBrief}
              onChange={e => setDraftBrief(e.target.value)}
              placeholder="Longer brief — what is this project, the thesis, current direction…"
              disabled={saving}
              rows={5}
              style={{ ...fieldInput(colors), resize: 'vertical', lineHeight: 1.5 }}
            />
          </label>
          <EditControls saving={saving} error={saveError} onSave={save} onCancel={() => setEditing(false)} />
        </div>
      ) : (
        <>
          <div style={{
            fontSize: 12, color: project.description ? colors.textMuted : colors.textDim,
            marginTop: 6, lineHeight: 1.55,
          }}>
            {project.description || 'No description yet.'}
          </div>
          {brief ? (
            <div style={{
              fontSize: 12, color: colors.text, marginTop: 10, lineHeight: 1.6,
              whiteSpace: 'pre-wrap', borderTop: `1px solid ${colors.border}`, paddingTop: 10,
            }}>
              {brief}
            </div>
          ) : (
            <div style={{ fontSize: 11, color: colors.textDim, marginTop: 10 }}>
              No brief yet — Edit to add one.
            </div>
          )}
        </>
      )}
    </Panel>
  );
}

// ── Watcher insights ────────────────────────────────────────────────────────
// 1-2 grounded daily observations the Watcher silently places on the project
// (daemon watcher_insights loop → metadata_json.watcher_insights). Quiet by
// design: no badge, no notification — read them as you browse. Renders
// nothing until the first insight exists.

interface WatcherInsight { text: string; created_at: string }

function readWatcherInsights(metadata: Record<string, unknown>): WatcherInsight[] {
  const raw = metadata?.watcher_insights;
  if (!Array.isArray(raw)) return [];
  return raw
    .filter((i): i is WatcherInsight =>
      typeof i === 'object' && i !== null &&
      typeof (i as WatcherInsight).text === 'string' &&
      typeof (i as WatcherInsight).created_at === 'string')
    .slice(0, 3);
}

function WatcherInsightsPanel({ project }: { project: Project }) {
  const { colors } = useTheme();
  const insights = readWatcherInsights(project.metadataJson);
  if (insights.length === 0) return null;
  return (
    <Panel title="From the Watcher">
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        {insights.map(i => (
          <div key={i.created_at} style={{ display: 'flex', gap: 10, alignItems: 'baseline' }}>
            <span style={{ color: colors.purpleBright, fontSize: 11, flexShrink: 0, lineHeight: '18px' }}>◆</span>
            <div style={{ minWidth: 0 }}>
              <div style={{ fontSize: 12, color: colors.textMuted, lineHeight: 1.55 }}>{i.text}</div>
              <div style={{ fontSize: 10, color: colors.textDim, marginTop: 2 }}>{formatDate(i.created_at)}</div>
            </div>
          </div>
        ))}
      </div>
    </Panel>
  );
}

function KeyFactsPanel({ project }: { project: Project }) {
  const { colors, theme } = useTheme();
  // White veils vanish on silver — flip to a faint graphite tint there.
  const chipVeil = theme === 'silver' ? 'rgba(30,37,48,0.06)' : 'rgba(255,255,255,0.06)';
  const facts: { label: string; value: ReactNode }[] = [
    { label: 'Status', value: <StatusPill status={project.status} /> },
    { label: 'Slug', value: project.slug },
    // Key dates (#472): created / updated / last opened — all already on the wire.
    { label: 'Created', value: formatDate(project.createdAt) },
    { label: 'Updated', value: formatDate(project.updatedAt) },
    { label: 'Last opened', value: formatDate(project.lastOpenedAt) },
  ];

  return (
    <Panel title="Key facts">
      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
        {facts.map(f => (
          <div key={f.label} style={{ display: 'flex', alignItems: 'baseline', gap: 10 }}>
            <span style={{ fontSize: 11, color: colors.textDim, width: 88, flexShrink: 0 }}>{f.label}</span>
            <span style={{ fontSize: 12, color: colors.text, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis' }}>
              {f.value}
            </span>
          </div>
        ))}
        {/* Root path is actionable, not inert: open a project-aware terminal in
            Build (the same createProjectTab path a launch button uses), or copy
            the path. */}
        {project.rootPath && <RootPathRow project={project} />}
        {project.tags.length > 0 && (
          <div style={{ display: 'flex', alignItems: 'baseline', gap: 10 }}>
            <span style={{ fontSize: 11, color: colors.textDim, width: 88, flexShrink: 0 }}>Tags</span>
            <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
              {project.tags.map((tag, ti) => (
                <span key={`${tag}-${ti}`} style={{
                  fontSize: 10, padding: '1px 6px', borderRadius: 4,
                  background: chipVeil, color: colors.textDim,
                }}>
                  {tag}
                </span>
              ))}
            </div>
          </div>
        )}
      </div>
    </Panel>
  );
}

/**
 * Root-path row — turns the project's filesystem path from inert text into two
 * actions: "Open in Build" (reuses setPendingTerminalLaunch + navigateToTool,
 * the same seam the agent's project_launch event and the human launch button
 * ride) and copy-to-clipboard. No new backend calls.
 */
function RootPathRow({ project }: { project: Project }) {
  const { colors, reduceMotion } = useTheme();
  const setPendingTerminalLaunch = useCommandCenter(s => s.setPendingTerminalLaunch);
  const [copied, setCopied] = useState(false);

  const rootPath = project.rootPath;
  if (!rootPath) return null;

  const openInBuild = () => {
    // Switch to Build first; only queue the launch if a Build workspace exists
    // (mirrors useAppNavigate's project_launch handler).
    if (!navigateToTool('build')) return;
    setPendingTerminalLaunch({ rootPath, label: project.slug });
  };

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(rootPath);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard denied (rare in-app) — leave the label unchanged.
    }
  };

  return (
    <div style={{ display: 'flex', alignItems: 'baseline', gap: 10 }}>
      <span style={{ fontSize: 11, color: colors.textDim, width: 88, flexShrink: 0 }}>Root path</span>
      <div style={{ minWidth: 0, flex: 1, display: 'flex', flexDirection: 'column', gap: 6 }}>
        <Mono>
          <span style={{ display: 'block', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {rootPath}
          </span>
        </Mono>
        <div style={{ display: 'flex', gap: 6 }}>
          <button
            onClick={openInBuild}
            style={{
              display: 'inline-flex', alignItems: 'center', gap: 5,
              padding: '3px 9px', borderRadius: 6, cursor: 'pointer',
              background: colors.cyanSoft, border: `1px solid ${colors.borderHi}`,
              color: colors.cyan, fontSize: 11, fontWeight: 600, fontFamily: font.body,
              transition: reduceMotion ? 'none' : 'all 150ms',
            }}
            onMouseEnter={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.cyan; }}
            onMouseLeave={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.borderHi; }}
          >
            <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" style={{ flexShrink: 0 }}>
              <path d="M4 17l6-6-6-6M12 19h8" />
            </svg>
            Open in Build
          </button>
          <button
            onClick={copy}
            aria-label="Copy root path"
            style={{
              display: 'inline-flex', alignItems: 'center', gap: 5,
              padding: '3px 9px', borderRadius: 6, cursor: 'pointer',
              background: 'rgba(255,255,255,0.03)', border: `1px solid ${colors.border}`,
              color: copied ? colors.success : colors.textMuted, fontSize: 11, fontFamily: font.body,
              transition: reduceMotion ? 'none' : 'all 150ms',
            }}
            onMouseEnter={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.borderHi; }}
            onMouseLeave={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.border; }}
          >
            {copied ? 'Copied' : 'Copy'}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Right-column panels ─────────────────────────────────────────────────────

/**
 * Links — website + repository (existing columns) plus social/other links
 * (#472 residue, `metadata_json.links`). Editable in place through the
 * existing PATCH /api/projects/:id; custom links merge into the shared
 * metadata bag without touching foreign keys (see workspaceMeta.ts).
 */
export function LinksPanel({ project, onProjectUpdated, title = 'Links' }: {
  project: Project;
  onProjectUpdated?: () => void;
  title?: string;
}) {
  const { colors } = useTheme();
  const navigate = useBrowserNavigate();
  const customLinks = readLinks(project.metadataJson);
  const [editing, setEditing] = useState(false);
  const [draftSite, setDraftSite] = useState('');
  const [draftRepo, setDraftRepo] = useState('');
  const [draftLinks, setDraftLinks] = useState<WorkspaceLink[]>([]);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const links: { label: string; url: string }[] = [];
  if (project.siteUrl) links.push({ label: 'Website', url: project.siteUrl });
  if (project.repoUrl) links.push({ label: 'Repository', url: project.repoUrl });
  for (const l of customLinks) links.push({ label: l.label || l.url, url: l.url });

  const startEditing = () => {
    setDraftSite(project.siteUrl ?? '');
    setDraftRepo(project.repoUrl ?? '');
    setDraftLinks(customLinks.map(l => ({ ...l })));
    setSaveError(null);
    setEditing(true);
  };

  const save = async () => {
    setSaving(true);
    setSaveError(null);
    try {
      await saveProjectSummary(project.id, {
        siteUrl: normalizeUrl(draftSite),
        repoUrl: normalizeUrl(draftRepo),
        // Empty-url rows are dropped by the merge; scheme-less urls get https://.
        links: draftLinks.map(l => ({ label: l.label, url: normalizeUrl(l.url) ?? '' })),
      });
      setEditing(false);
      onProjectUpdated?.();
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : 'Save failed');
    } finally {
      setSaving(false);
    }
  };

  if (editing) {
    return (
      <Panel title={title}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          <label style={fieldLabel(colors)}>
            Website
            <input value={draftSite} onChange={e => setDraftSite(e.target.value)}
              placeholder="example.com" disabled={saving} style={fieldInput(colors)} />
          </label>
          <label style={fieldLabel(colors)}>
            Repository
            <input value={draftRepo} onChange={e => setDraftRepo(e.target.value)}
              placeholder="github.com/you/repo" disabled={saving} style={fieldInput(colors)} />
          </label>
          <div style={{ ...fieldLabel(colors), display: 'flex', flexDirection: 'column', gap: 6 }}>
            Other links
            {draftLinks.map((l, i) => (
              <div key={i} style={{ display: 'flex', gap: 6 }}>
                <input value={l.label} placeholder="Label" disabled={saving} aria-label={`Link ${i + 1} label`}
                  onChange={e => setDraftLinks(ls => ls.map((x, xi) => xi === i ? { ...x, label: e.target.value } : x))}
                  style={{ ...fieldInput(colors), width: 90, flexShrink: 0 }} />
                <input value={l.url} placeholder="URL" disabled={saving} aria-label={`Link ${i + 1} URL`}
                  onChange={e => setDraftLinks(ls => ls.map((x, xi) => xi === i ? { ...x, url: e.target.value } : x))}
                  style={{ ...fieldInput(colors), flex: 1, minWidth: 0 }} />
                <button onClick={() => setDraftLinks(ls => ls.filter((_, xi) => xi !== i))}
                  disabled={saving} aria-label={`Remove link ${i + 1}`}
                  style={{ ...panelActionBtn(colors), color: colors.textMuted }}>
                  ✕
                </button>
              </div>
            ))}
            <button
              onClick={() => setDraftLinks(ls => [...ls, { label: '', url: '' }])}
              disabled={saving}
              style={{ ...panelActionBtn(colors), alignSelf: 'flex-start' }}
            >
              + Add link
            </button>
          </div>
          <EditControls saving={saving} error={saveError} onSave={save} onCancel={() => setEditing(false)} />
        </div>
      </Panel>
    );
  }

  return (
    <Panel
      title={title}
      action={<button onClick={startEditing} style={panelActionBtn(colors)}>Edit</button>}
    >
      {links.length === 0 ? (
        <div style={{ fontSize: 11, color: colors.textDim }}>
          No links yet — Edit to add website, repo, or social links.
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {links.map((link, li) => (
            <button
              key={`${link.label}-${li}`}
              onClick={() => navigate(link.url)}
              style={{
                display: 'flex', alignItems: 'center', gap: 8, textAlign: 'left',
                padding: '7px 9px', borderRadius: 7,
                background: 'rgba(255,255,255,0.03)', border: `1px solid ${colors.border}`,
                color: colors.text, fontFamily: font.body, fontSize: 12, cursor: 'pointer',
                transition: 'all 150ms', width: '100%',
              }}
              onMouseEnter={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.borderHi; }}
              onMouseLeave={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.border; }}
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke={colors.cyan} strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" style={{ flexShrink: 0 }}>
                <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" />
                <path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" />
              </svg>
              <span style={{ flexShrink: 0, color: colors.textMuted }}>{link.label}</span>
              <span style={{ minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: colors.textDim, fontSize: 11 }}>
                {link.url}
              </span>
            </button>
          ))}
        </div>
      )}
    </Panel>
  );
}

/** Lifecycle order for grouping the to-do summary. Manual columns (no state
 *  binding) sort last, in board position order. */
const STATE_ORDER = ['triage', 'ready', 'in_progress', 'review', 'complete', 'cancelled', 'failed'];

function TasksPanel({ columns, cards, onOpenGoal }: {
  columns: BoardColumn[];
  cards: Card[];
  onOpenGoal: (cardId: string) => void;
}) {
  const { colors, theme } = useTheme();
  // White veils vanish on silver — flip to a faint graphite tint there.
  const rowVeil = theme === 'silver' ? 'rgba(30,37,48,0.03)' : 'rgba(255,255,255,0.02)';
  const chipVeil = theme === 'silver' ? 'rgba(30,37,48,0.06)' : 'rgba(255,255,255,0.06)';
  const ordered = [...columns].sort((a, b) => {
    const ai = a.stateBinding ? STATE_ORDER.indexOf(a.stateBinding) : 99;
    const bi = b.stateBinding ? STATE_ORDER.indexOf(b.stateBinding) : 99;
    if (ai !== bi) return ai - bi;
    return a.position - b.position;
  });
  const total = cards.filter(c => !c.archivedAt).length;

  return (
    <Panel
      title="Tasks"
      action={<span style={{ fontSize: 10, color: colors.textDim }}>{total} card{total !== 1 ? 's' : ''}</span>}
    >
      {total === 0 ? (
        <div style={{ fontSize: 11, color: colors.textDim }}>No tasks yet.</div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
          {ordered.map(col => {
            const colCards = cards
              .filter(c => c.columnId === col.id && !c.archivedAt)
              .sort((a, b) => a.position - b.position);
            if (colCards.length === 0) return null;
            return (
              <div key={col.id}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 4 }}>
                  <span style={{ fontSize: 10, fontWeight: 600, color: colors.textMuted, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                    {col.name}
                  </span>
                  <span style={{ fontSize: 10, color: colors.textDim, background: chipVeil, padding: '0 5px', borderRadius: 7 }}>
                    {colCards.length}
                  </span>
                </div>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
                  {colCards.map(card => {
                    const isGoal = card.cardType === 'goal';
                    return (
                      <div
                        key={card.id}
                        role={isGoal ? 'button' : undefined}
                        tabIndex={isGoal ? 0 : undefined}
                        aria-label={isGoal ? `Open goal ${card.title}` : undefined}
                        onClick={isGoal ? () => onOpenGoal(card.id) : undefined}
                        onKeyDown={isGoal ? e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onOpenGoal(card.id); } } : undefined}
                        style={{
                          fontSize: 12, padding: '4px 8px', borderRadius: 6,
                          background: rowVeil,
                          color: colors.text, cursor: isGoal ? 'pointer' : 'default',
                          display: 'flex', alignItems: 'center', gap: 6,
                          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                        }}
                        onMouseEnter={e => { if (isGoal) (e.currentTarget as HTMLElement).style.background = colors.cyanSoft; }}
                        onMouseLeave={e => { (e.currentTarget as HTMLElement).style.background = rowVeil; }}
                        onFocus={e => { if (isGoal) (e.currentTarget as HTMLElement).style.background = colors.cyanSoft; }}
                        onBlur={e => { (e.currentTarget as HTMLElement).style.background = rowVeil; }}
                      >
                        {isGoal && (
                          <span style={{ width: 5, height: 5, borderRadius: '50%', background: colors.purpleBright, flexShrink: 0 }} />
                        )}
                        <span style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}>{card.title}</span>
                      </div>
                    );
                  })}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </Panel>
  );
}

// ── Inline-edit helpers (shared by Summary + Links) ─────────────────────────

type ThemeColors = ReturnType<typeof useTheme>['colors'];

/** Small cyan text-button, matching PeoplePanel's "+ Associate" affordance. */
function panelActionBtn(colors: ThemeColors): React.CSSProperties {
  return {
    fontSize: 11, color: colors.cyan, background: 'none', border: 'none',
    cursor: 'pointer', fontFamily: font.body, padding: 0,
  };
}

function fieldLabel(colors: ThemeColors): React.CSSProperties {
  return {
    display: 'flex', flexDirection: 'column', gap: 4,
    fontSize: 10, fontWeight: 600, color: colors.textDim,
    textTransform: 'uppercase', letterSpacing: '0.05em',
  };
}

function fieldInput(colors: ThemeColors): React.CSSProperties {
  return {
    fontSize: 12, fontFamily: font.body, color: colors.text,
    background: 'rgba(255,255,255,0.04)', border: `1px solid ${colors.border}`,
    borderRadius: 6, padding: '6px 8px', outline: 'none',
    textTransform: 'none', letterSpacing: 'normal', fontWeight: 400,
  };
}

/** Save/Cancel row + inline error. Errors keep the editor open — a failed
 *  save must read as a failure, never as a silent success. */
function EditControls({ saving, error, onSave, onCancel }: {
  saving: boolean;
  error: string | null;
  onSave: () => void;
  onCancel: () => void;
}) {
  const { colors } = useTheme();
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
      {error && (
        <div role="alert" style={{ fontSize: 11, color: colors.danger }}>
          Couldn't save: {error}
        </div>
      )}
      <div style={{ display: 'flex', gap: 6 }}>
        <button
          onClick={onSave}
          disabled={saving}
          style={{
            padding: '4px 12px', borderRadius: 6, cursor: saving ? 'default' : 'pointer',
            background: colors.cyanSoft, border: `1px solid ${colors.borderHi}`,
            color: colors.cyan, fontSize: 11, fontWeight: 600, fontFamily: font.body,
            opacity: saving ? 0.6 : 1,
          }}
        >
          {saving ? 'Saving…' : 'Save'}
        </button>
        <button
          onClick={onCancel}
          disabled={saving}
          style={{
            padding: '4px 12px', borderRadius: 6, cursor: saving ? 'default' : 'pointer',
            background: 'none', border: `1px solid ${colors.border}`,
            color: colors.textMuted, fontSize: 11, fontFamily: font.body,
          }}
        >
          Cancel
        </button>
      </div>
    </div>
  );
}

// ── Small helpers ───────────────────────────────────────────────────────────

function StatusPill({ status }: { status: string }) {
  const { colors } = useTheme();
  const map: Record<string, string> = { active: colors.cyan, paused: colors.warning, archived: colors.textDim };
  const color = map[status] ?? colors.textMuted;
  return (
    <span style={{
      fontSize: 10, fontWeight: 600, textTransform: 'capitalize',
      padding: '1px 8px', borderRadius: 5, color,
      background: 'rgba(255,255,255,0.05)', border: `1px solid ${color}33`,
    }}>
      {status}
    </span>
  );
}

function Mono({ children }: { children: ReactNode }) {
  return <span style={{ fontFamily: font.mono, fontSize: 11 }}>{children}</span>;
}

function formatDate(iso: string): string {
  try {
    return new Date(iso).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
  } catch {
    return iso;
  }
}
