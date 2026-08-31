import { useState, useEffect, useCallback, type CSSProperties, type ReactNode } from 'react';
import { FiLink, FiTerminal, FiTrendingUp } from 'react-icons/fi';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { apiFetch, api } from '../../lib/api';
import { useGoalEvents } from '../../lib/useGoalEvents';
import { useBrowserNavigate } from '../../hooks/useBrowserNavigate';
import { useCommandCenter, navigateToTool } from '../../lib/store';
import { Panel } from './Panel';
import { StateBlock } from '../common/StateBlock';
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
  const [boardLoading, setBoardLoading] = useState(true);
  const [boardError, setBoardError] = useState(false);
  const openGoalDetail = useCommandCenter(s => s.openGoalDetail);
  const growProject = useCommandCenter(s => s.growProject);

  // Swallowing this used to render "No tasks yet." for a board that simply
  // never arrived — the same words as a project that genuinely has none.
  // `ProjectKanban.loadBoard` fetches these two endpoints one lens away and
  // already tracks loading/error/ready; this is that, for the same data.
  const loadBoard = useCallback(async () => {
    try {
      const [cols, cds] = await Promise.all([
        apiFetch<BoardColumn[]>(`/api/projects/${project.id}/columns`),
        apiFetch<Card[]>(`/api/projects/${project.id}/cards`),
      ]);
      setColumns(cols);
      setCards(cds);
      setBoardError(false);
    } catch {
      setBoardError(true);
    } finally {
      setBoardLoading(false);
    }
  }, [project.id]);

  const retryBoard = useCallback(() => { setBoardLoading(true); void loadBoard(); }, [loadBoard]);

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
          <StrixFindingsPanel project={project} />
          <WatcherInsightsPanel project={project} />
          <KeyFactsPanel project={project} />
          <ActivityPanel project={project} />
        </div>

        {/* RIGHT — where it stands, and what's next */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 16, minWidth: 0 }}>
          {/* Build → Grow bridge: take the finished work to market. */}
          <Button
            colors={colors}
            onClick={() => growProject(project.id)}
            style={{
              '--pa-btn-bg': `linear-gradient(90deg, ${colors.cyan}22, ${colors.purple}22)`,
              '--pa-btn-bg-hover': `linear-gradient(90deg, ${colors.cyan}33, ${colors.purple}33)`,
              '--pa-btn-bg-active': `linear-gradient(90deg, ${colors.cyan}3D, ${colors.purple}3D)`,
              '--pa-btn-border': colors.borderHi,
              '--pa-btn-border-hover': colors.cyan,
              '--pa-btn-pad': '11px 14px',
              '--pa-btn-radius': `${radius.lg}px`,
              '--pa-btn-weight': 600,
              fontFamily: font.body, fontSize: textSize.small,
              gap: 8,
            } as CSSProperties}
          >
            <FiTrendingUp size={15} color={colors.cyan} />
            Grow this project
          </Button>
          <TasksPanel
            columns={columns}
            cards={cards}
            loading={boardLoading}
            error={boardError}
            onRetry={retryBoard}
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
        <Button colors={colors} variant="bare" className="hover:underline" onClick={startEditing} style={panelActionVars(colors)}>Edit</Button>
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
            fontSize: textSize.caption, color: project.description ? colors.textMuted : colors.textDim,
            marginTop: 6, lineHeight: 1.55,
          }}>
            {project.description || 'No description yet.'}
          </div>
          {brief ? (
            <div style={{
              fontSize: textSize.caption, color: colors.text, marginTop: 10, lineHeight: 1.6,
              whiteSpace: 'pre-wrap', borderTop: `1px solid ${colors.border}`, paddingTop: 10,
            }}>
              {brief}
            </div>
          ) : (
            <div style={{ fontSize: textSize.micro, color: colors.textDim, marginTop: 10 }}>
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

interface WatcherInsightCard { id: string; title: string }
interface WatcherInsight {
  text: string;
  created_at: string;
  /** The cards this insight is about. Absent on rows written before the
   *  Watcher started naming them — absent and empty render identically. */
  cards?: WatcherInsightCard[];
}

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

export function WatcherInsightsPanel({ project }: { project: Project }) {
  const { colors } = useTheme();
  const openCardOnBoard = useCommandCenter(s => s.openCardOnBoard);
  const insights = readWatcherInsights(project.metadataJson);
  if (insights.length === 0) return null;
  return (
    <Panel title="From the Watcher">
      <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        {insights.map(i => (
          <div key={i.created_at} style={{ display: 'flex', gap: 10, alignItems: 'baseline' }}>
            <span style={{ color: colors.purpleBright, fontSize: textSize.micro, flexShrink: 0, lineHeight: '18px' }}>◆</span>
            <div style={{ minWidth: 0 }}>
              <div style={{ fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.55 }}>{i.text}</div>
              {/* The cards the observation is about. Without these the reader
                  is told something stalled and given no way to reach it. */}
              {(i.cards ?? []).length > 0 && (
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6, marginTop: 5 }}>
                  {(i.cards ?? []).map(c => (
                    <Button
                      key={c.id}
                      colors={colors}
                      variant="bare"
                      onClick={() => openCardOnBoard(project.id, c.id)}
                      title={`Open "${c.title}" on the board`}
                      style={{
                        '--pa-btn-bg': colors.cyanSoft,
                        '--pa-btn-fg': colors.cyan,
                        '--pa-btn-bg-hover': colors.cyanSoft,
                        '--pa-btn-bg-active': colors.cyanSoft,
                        '--pa-btn-border-hover': colors.cyan,
                        '--pa-btn-pad': '2px 7px',
                        '--pa-btn-radius': `${radius.xs}px`,
                        fontSize: 10, fontFamily: font.body, maxWidth: 260,
                        overflow: 'hidden', whiteSpace: 'nowrap',
                      } as CSSProperties}
                    >
                      {c.title}
                    </Button>
                  ))}
                </div>
              )}
              <div style={{ fontSize: 10, color: colors.textDim, marginTop: 4 }}>{formatDate(i.created_at)}</div>
            </div>
          </div>
        ))}
      </div>
    </Panel>
  );
}

// ── The Guard's findings ─────────────────────────────────────────────────────
// The security checklist the Guard's sweep loop keeps on the project
// (daemon strix loop → metadata_json.strix_findings). Each item carries its
// severity, CWE, location, and how to fix it. With no findings the panel says
// WHY there are none — off / never scanned / scanned clean — because silence
// used to read as "secure" whether or not the Guard had ever run (audit
// 2026-08-11).

interface StrixFinding {
  id: string;
  title: string;
  severity: string;
  cwe?: string | null;
  location?: string | null;
  remediation?: string | null;
  found_at: string;
}

const STRIX_SHOWN = 8;

function readStrixFindings(metadata: Record<string, unknown>): StrixFinding[] {
  const raw = metadata?.strix_findings;
  if (!Array.isArray(raw)) return [];
  return raw.filter((f): f is StrixFinding =>
    typeof f === 'object' && f !== null &&
    typeof (f as StrixFinding).id === 'string' &&
    typeof (f as StrixFinding).title === 'string' &&
    typeof (f as StrixFinding).severity === 'string');
}

function StrixFindingsPanel({ project }: { project: Project }) {
  const { colors } = useTheme();
  const findings = readStrixFindings(project.metadataJson);

  // The honesty gate the HUD uses: `null` = unknown (loading / failed read) —
  // never claim OFF on a failed read.
  const [enabled, setEnabled] = useState<boolean | null>(null);
  useEffect(() => {
    let active = true;
    api.readConfig('strix_enabled')
      .then(r => { if (active) setEnabled(r === true); })
      .catch(() => { /* unknown stays unknown */ });
    return () => { active = false; };
  }, []);

  const rawLastScan = (project.metadataJson as Record<string, unknown>)?.strix_last_scan;
  const lastScan = typeof rawLastScan === 'string' && rawLastScan ? rawLastScan : null;
  const rawLastAttempt = (project.metadataJson as Record<string, unknown>)?.strix_last_attempt;
  const lastAttempt = typeof rawLastAttempt === 'string' && rawLastAttempt ? rawLastAttempt : null;

  if (findings.length === 0) {
    // Honest empty state: say why there is nothing, instead of nothing.
    // A newer last_attempt than last_scan means the latest sweep did not
    // finish — do not call that "scanned, no findings".
    const attemptFailed = Boolean(
      lastAttempt && (!lastScan || lastAttempt > lastScan),
    );
    const text = attemptFailed
      ? `Last sweep did not finish (${formatDate(lastAttempt!)})${lastScan ? ` — previous clean scan ${formatDate(lastScan)}` : ' — not a clean scan.'}`
      : lastScan
      ? `Scanned ${formatDate(lastScan)} — no open findings.${enabled === false ? ' The Guard is currently off.' : ''}`
      : enabled === false
        ? 'The Guard is off — enable security sweeps in Settings → Models to scan this project.'
        : 'Never scanned — waiting on the Guard’s first sweep of this project.';
    return (
      <Panel title="Security — from the Guard">
        <div style={{ fontSize: textSize.micro, color: colors.textDim, lineHeight: 1.5 }}>{text}</div>
      </Panel>
    );
  }
  const severityColor = (s: string) =>
    s === 'high' ? colors.danger : s === 'medium' ? colors.warning : colors.textDim;
  const shown = findings.slice(0, STRIX_SHOWN);
  return (
    <Panel title="Security — from the Guard">
      <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
        {shown.map(f => (
          <div key={f.id} style={{ display: 'flex', gap: 10, alignItems: 'baseline' }}>
            <span style={{
              fontSize: 9, fontWeight: 700, letterSpacing: 0.6, flexShrink: 0,
              textTransform: 'uppercase', color: severityColor(f.severity),
              lineHeight: '18px', width: 52,
            }}>
              {f.severity}
            </span>
            <div style={{ minWidth: 0 }}>
              <div style={{ fontSize: textSize.caption, color: colors.text, lineHeight: 1.5 }}>{f.title}</div>
              <div style={{ fontSize: 10, color: colors.textDim, marginTop: 2, display: 'flex', gap: 8, flexWrap: 'wrap' }}>
                {f.cwe && <span>{f.cwe}</span>}
                {f.location && <span style={{ fontFamily: font.mono }}>{f.location}</span>}
                <span>{formatDate(f.found_at)}</span>
              </div>
              {f.remediation && (
                <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: 3, lineHeight: 1.5 }}>
                  {f.remediation}
                </div>
              )}
            </div>
          </div>
        ))}
        {findings.length > STRIX_SHOWN && (
          <div style={{ fontSize: 10, color: colors.textDim }}>
            +{findings.length - STRIX_SHOWN} more on the checklist
          </div>
        )}
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
            <span style={{ fontSize: textSize.micro, color: colors.textDim, width: 88, flexShrink: 0 }}>{f.label}</span>
            <span style={{ fontSize: textSize.caption, color: colors.text, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis' }}>
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
            <span style={{ fontSize: textSize.micro, color: colors.textDim, width: 88, flexShrink: 0 }}>Tags</span>
            <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
              {project.tags.map((tag, ti) => (
                <span key={`${tag}-${ti}`} style={{
                  fontSize: 10, padding: '1px 6px', borderRadius: radius.xs,
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
      <span style={{ fontSize: textSize.micro, color: colors.textDim, width: 88, flexShrink: 0 }}>Root path</span>
      <div style={{ minWidth: 0, flex: 1, display: 'flex', flexDirection: 'column', gap: 6 }}>
        <Mono>
          <span style={{ display: 'block', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {rootPath}
          </span>
        </Mono>
        <div style={{ display: 'flex', gap: 6 }}>
          <Button
            colors={colors}
            variant="ghostOn"
            onClick={openInBuild}
            style={{
              '--pa-btn-bg': colors.cyanSoft,
              '--pa-btn-bg-hover': colors.cyanSoft,
              '--pa-btn-border': colors.borderHi,
              '--pa-btn-border-hover': colors.cyan,
              '--pa-btn-pad': '3px 9px',
              '--pa-btn-radius': `${radius.sm}px`,
              '--pa-btn-weight': 600,
              fontSize: textSize.micro, fontFamily: font.body,
              transition: reduceMotion ? 'none' : undefined,
            } as CSSProperties}
          >
            <FiTerminal size={11} style={{ flexShrink: 0 }} />
            Open in Build
          </Button>
          {/* This one confirms itself — the label flips to "Copied" for 1.5s —
              so the primitive's tick would say the same thing twice. */}
          <Button
            colors={colors}
            onClick={copy}
            aria-label="Copy root path"
            flashSuccess={false}
            style={{
              '--pa-btn-bg': 'rgba(255,255,255,0.03)',
              '--pa-btn-bg-hover': 'rgba(255,255,255,0.03)',
              '--pa-btn-fg': copied ? colors.success : colors.textMuted,
              '--pa-btn-border': colors.border,
              '--pa-btn-border-hover': colors.borderHi,
              '--pa-btn-pad': '3px 9px',
              '--pa-btn-radius': `${radius.sm}px`,
              fontSize: textSize.micro, fontFamily: font.body,
              transition: reduceMotion ? 'none' : undefined,
            } as CSSProperties}
          >
            {copied ? 'Copied' : 'Copy'}
          </Button>
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
                <Button colors={colors} variant="bare"
                  onClick={() => setDraftLinks(ls => ls.filter((_, xi) => xi !== i))}
                  disabled={saving} aria-label={`Remove link ${i + 1}`}
                  style={{
                    ...panelActionVars(colors),
                    '--pa-btn-fg': colors.textMuted,
                    '--pa-btn-fg-hover': colors.text,
                  } as CSSProperties}>
                  ✕
                </Button>
              </div>
            ))}
            <Button
              colors={colors}
              variant="bare"
              className="hover:underline"
              onClick={() => setDraftLinks(ls => [...ls, { label: '', url: '' }])}
              disabled={saving}
              style={{ ...panelActionVars(colors), alignSelf: 'flex-start' }}
            >
              + Add link
            </Button>
          </div>
          <EditControls saving={saving} error={saveError} onSave={save} onCancel={() => setEditing(false)} />
        </div>
      </Panel>
    );
  }

  return (
    <Panel
      title={title}
      action={<Button colors={colors} variant="bare" className="hover:underline" onClick={startEditing} style={panelActionVars(colors)}>Edit</Button>}
    >
      {links.length === 0 ? (
        <div style={{ fontSize: textSize.micro, color: colors.textDim }}>
          No links yet — Edit to add website, repo, or social links.
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {links.map((link, li) => (
            <Button
              key={`${link.label}-${li}`}
              colors={colors}
              onClick={() => navigate(link.url)}
              style={{
                '--pa-btn-bg': 'rgba(255,255,255,0.03)',
                '--pa-btn-bg-hover': 'rgba(255,255,255,0.03)',
                '--pa-btn-border': colors.border,
                '--pa-btn-border-hover': colors.borderHi,
                '--pa-btn-fg': colors.text,
                '--pa-btn-pad': '7px 9px',
                '--pa-btn-radius': `${radius.sm}px`,
                gap: 8, textAlign: 'left', justifyContent: 'flex-start',
                fontFamily: font.body, fontSize: textSize.caption, width: '100%',
              } as CSSProperties}
            >
              <FiLink size={12} color={colors.cyan} style={{ flexShrink: 0 }} />
              <span style={{ flexShrink: 0, color: colors.textMuted }}>{link.label}</span>
              <span style={{ minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: colors.textDim, fontSize: textSize.micro }}>
                {link.url}
              </span>
            </Button>
          ))}
        </div>
      )}
    </Panel>
  );
}

/** Lifecycle order for grouping the to-do summary. Manual columns (no state
 *  binding) sort last, in board position order. */
const STATE_ORDER = ['triage', 'ready', 'in_progress', 'review', 'complete', 'cancelled', 'failed'];

export function TasksPanel({ columns, cards, loading, error, onRetry, onOpenGoal }: {
  columns: BoardColumn[];
  cards: Card[];
  loading: boolean;
  error: boolean;
  onRetry: () => void;
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
      action={
        error || loading
          ? undefined
          : <span style={{ fontSize: 10, color: colors.textDim }}>{total} card{total !== 1 ? 's' : ''}</span>
      }
    >
      {error ? (
        <StateBlock
          compact
          tone="error"
          title="Couldn't load this project's tasks"
          detail="The board's columns and cards didn't load. Check the daemon connection and try again."
          onRetry={onRetry}
        />
      ) : loading ? (
        <div style={{ fontSize: textSize.micro, color: colors.textDim }}>Loading tasks…</div>
      ) : total === 0 ? (
        <div style={{ fontSize: textSize.micro, color: colors.textDim }}>No tasks yet.</div>
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
                          fontSize: textSize.caption, padding: '4px 8px', borderRadius: radius.sm,
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

/** Small cyan text-button, matching PeoplePanel's "+ Associate" affordance.
 *  Feeds `Button`'s custom properties instead of styling the element directly:
 *  an inline `color` would beat `.pa-btn:hover` and kill the state it is being
 *  migrated in for. */
function panelActionVars(colors: ThemeColors): CSSProperties {
  return {
    '--pa-btn-fg': colors.cyan,
    '--pa-btn-bg-hover': 'transparent',
    '--pa-btn-bg-active': 'transparent',
    '--pa-btn-pad': '0',
    fontSize: textSize.micro, fontFamily: font.body,
  } as CSSProperties;
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
    fontSize: textSize.caption, fontFamily: font.body, color: colors.text,
    background: 'rgba(255,255,255,0.04)', border: `1px solid ${colors.border}`,
    borderRadius: radius.sm, padding: '6px 8px', outline: 'none',
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
        <div role="alert" style={{ fontSize: textSize.micro, color: colors.danger }}>
          Couldn't save: {error}
        </div>
      )}
      <div style={{ display: 'flex', gap: 6 }}>
        {/* The work runs in the caller's `onSave`, so the in-flight state is
            handed in rather than awaited off the click. */}
        <Button
          colors={colors}
          variant="ghostOn"
          onClick={onSave}
          pending={saving}
          disabled={saving}
          style={{
            '--pa-btn-bg': colors.cyanSoft,
            '--pa-btn-bg-hover': colors.cyanSoft,
            '--pa-btn-border': colors.borderHi,
            '--pa-btn-border-hover': colors.cyan,
            '--pa-btn-pad': '4px 12px',
            '--pa-btn-radius': `${radius.sm}px`,
            '--pa-btn-weight': 600,
            fontSize: textSize.micro, fontFamily: font.body,
          } as CSSProperties}
        >
          {saving ? 'Saving…' : 'Save'}
        </Button>
        <Button
          colors={colors}
          onClick={onCancel}
          disabled={saving}
          style={{
            '--pa-btn-fg': colors.textMuted,
            '--pa-btn-fg-hover': colors.text,
            '--pa-btn-border': colors.border,
            '--pa-btn-border-hover': colors.borderHi,
            '--pa-btn-bg-hover': 'transparent',
            '--pa-btn-pad': '4px 12px',
            '--pa-btn-radius': `${radius.sm}px`,
            fontSize: textSize.micro, fontFamily: font.body,
          } as CSSProperties}
        >
          Cancel
        </Button>
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
  return <span style={{ fontFamily: font.mono, fontSize: textSize.micro }}>{children}</span>;
}

function formatDate(iso: string): string {
  try {
    return new Date(iso).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
  } catch {
    return iso;
  }
}
