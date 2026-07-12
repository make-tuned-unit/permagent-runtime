import { useState, useEffect, useCallback, type ReactNode } from 'react';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { apiFetch } from '../../lib/api';
import { useGoalEvents } from '../../lib/useGoalEvents';
import { useBrowserNavigate } from '../../hooks/useBrowserNavigate';
import { useCommandCenter, navigateToTool } from '../../lib/store';
import { Panel } from './Panel';
import { PeoplePanel } from './PeoplePanel';
import { DocumentsPanel } from './DocumentsPanel';
import type { Project, BoardColumn, Card } from './types';

// ── Project Overview ────────────────────────────────────────────────────────
//
// The "command-center dash" lens of the Projects tab (#471, Layer 1). A
// two-column read of a single project. LEFT = substance, RIGHT = people +
// action. Ships Summary, Key Facts, Links, Tasks, People, and Documents (the
// #471 Layer 2 document hub + in-app viewer). The Memories panel is still
// deferred to the association layer; its slot is reserved in-layout below so it
// drops in without restructuring.

export function ProjectOverview({ project }: { project: Project }) {
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
        {/* LEFT — substance */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 16, minWidth: 0 }}>
          <SummaryPanel project={project} />
          <KeyFactsPanel project={project} />
          {/* Memories panel slots here (association layer). */}
          <DocumentsPanel project={project} />
        </div>

        {/* RIGHT — people + action */}
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
          <PeoplePanel project={project} />
          <LinksPanel project={project} />
          <TasksPanel
            columns={columns}
            cards={cards}
            onOpenGoal={(cardId) => openGoalDetail(project.id, cardId)}
          />
        </div>
      </div>
    </div>
  );
}

// ── Left-column panels ──────────────────────────────────────────────────────

function SummaryPanel({ project }: { project: Project }) {
  const { colors } = useTheme();
  return (
    <Panel title="Summary">
      <div style={{ fontFamily: font.display, fontSize: 18, fontWeight: 600, letterSpacing: '-0.01em' }}>
        {project.name}
      </div>
      <div style={{
        fontSize: 12, color: project.description ? colors.textMuted : colors.textDim,
        marginTop: 6, lineHeight: 1.55,
      }}>
        {project.description || 'No description yet.'}
      </div>
    </Panel>
  );
}

function KeyFactsPanel({ project }: { project: Project }) {
  const { colors } = useTheme();
  const facts: { label: string; value: ReactNode }[] = [
    { label: 'Status', value: <StatusPill status={project.status} /> },
    { label: 'Slug', value: project.slug },
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
                  fontSize: 9, padding: '1px 6px', borderRadius: 4,
                  background: 'rgba(255,255,255,0.06)', color: colors.textDim,
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

function LinksPanel({ project }: { project: Project }) {
  const { colors } = useTheme();
  const navigate = useBrowserNavigate();
  const links: { label: string; url: string }[] = [];
  if (project.siteUrl) links.push({ label: 'Website', url: project.siteUrl });
  if (project.repoUrl) links.push({ label: 'Repository', url: project.repoUrl });

  return (
    <Panel title="Links">
      {links.length === 0 ? (
        <div style={{ fontSize: 11, color: colors.textDim }}>No links yet.</div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {links.map(link => (
            <button
              key={link.label}
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
const STATE_ORDER = ['triage', 'ready', 'in_progress', 'review', 'complete', 'cancelled'];

function TasksPanel({ columns, cards, onOpenGoal }: {
  columns: BoardColumn[];
  cards: Card[];
  onOpenGoal: (cardId: string) => void;
}) {
  const { colors } = useTheme();
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
                  <span style={{ fontSize: 9, color: colors.textDim, background: 'rgba(255,255,255,0.06)', padding: '0 5px', borderRadius: 7 }}>
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
                          background: 'rgba(255,255,255,0.02)',
                          color: colors.text, cursor: isGoal ? 'pointer' : 'default',
                          display: 'flex', alignItems: 'center', gap: 6,
                          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                        }}
                        onMouseEnter={e => { if (isGoal) (e.currentTarget as HTMLElement).style.background = colors.cyanSoft; }}
                        onMouseLeave={e => { (e.currentTarget as HTMLElement).style.background = 'rgba(255,255,255,0.02)'; }}
                        onFocus={e => { if (isGoal) (e.currentTarget as HTMLElement).style.background = colors.cyanSoft; }}
                        onBlur={e => { (e.currentTarget as HTMLElement).style.background = 'rgba(255,255,255,0.02)'; }}
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
