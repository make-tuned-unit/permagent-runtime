import { useState, useEffect, useCallback, type ReactNode } from 'react';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { apiFetch } from '../../lib/api';
import { useGoalEvents } from '../../lib/useGoalEvents';
import { useBrowserNavigate } from '../../hooks/useBrowserNavigate';
import { useCommandCenter } from '../../lib/store';
import type { Project, BoardColumn, Card } from './types';

// ── Project Overview ────────────────────────────────────────────────────────
//
// The "command-center dash" lens of the Projects tab (#471, Layer 1). A
// two-column read of a single project. LEFT = substance, RIGHT = people +
// action. This slice ships only the panels whose data exists today (Summary,
// Key Facts, Links, Tasks) — all from existing endpoints, no backend route.
// The Memories / Documents / People panels are deferred to the association
// layer (built separately); their slots are reserved in-layout below so they
// drop in without restructuring.

export function ProjectOverview({ project }: { project: Project }) {
  const { colors, gradient } = useTheme();
  const [columns, setColumns] = useState<BoardColumn[]>([]);
  const [cards, setCards] = useState<Card[]>([]);
  const openGoalDetail = useCommandCenter(s => s.openGoalDetail);

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
          {/* Documents panel slots here (document hub). */}
        </div>

        {/* RIGHT — people + action */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 16, minWidth: 0 }}>
          {/* People panel slots here (association layer). */}
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

// ── Panel shell ─────────────────────────────────────────────────────────────

function Panel({ title, action, children }: { title: string; action?: ReactNode; children: ReactNode }) {
  const { colors } = useTheme();
  return (
    <section style={{
      background: 'rgba(255,255,255,0.02)', border: `1px solid ${colors.border}`,
      borderRadius: 10, padding: '14px 16px',
    }}>
      <div style={{ display: 'flex', alignItems: 'center', marginBottom: 10 }}>
        <span style={{
          fontSize: 11, fontWeight: 600, color: colors.textMuted,
          textTransform: 'uppercase', letterSpacing: '0.06em', flex: 1,
        }}>
          {title}
        </span>
        {action}
      </div>
      {children}
    </section>
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
  ];
  if (project.rootPath) facts.push({ label: 'Root path', value: <Mono>{project.rootPath}</Mono> });
  facts.push({ label: 'Last opened', value: formatDate(project.lastOpenedAt) });

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
        {project.tags.length > 0 && (
          <div style={{ display: 'flex', alignItems: 'baseline', gap: 10 }}>
            <span style={{ fontSize: 11, color: colors.textDim, width: 88, flexShrink: 0 }}>Tags</span>
            <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
              {project.tags.map(tag => (
                <span key={tag} style={{
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
              onMouseEnter={e => { (e.currentTarget as HTMLElement).style.borderColor = 'rgba(0,213,255,0.3)'; }}
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
                        onClick={isGoal ? () => onOpenGoal(card.id) : undefined}
                        style={{
                          fontSize: 12, padding: '4px 8px', borderRadius: 6,
                          background: 'rgba(255,255,255,0.02)',
                          color: colors.text, cursor: isGoal ? 'pointer' : 'default',
                          display: 'flex', alignItems: 'center', gap: 6,
                          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                        }}
                        onMouseEnter={e => { if (isGoal) (e.currentTarget as HTMLElement).style.background = 'rgba(0,213,255,0.05)'; }}
                        onMouseLeave={e => { (e.currentTarget as HTMLElement).style.background = 'rgba(255,255,255,0.02)'; }}
                      >
                        {isGoal && (
                          <span style={{ width: 5, height: 5, borderRadius: '50%', background: '#A78BFA', flexShrink: 0 }} />
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
  const map: Record<string, string> = { active: colors.cyan, paused: '#F59E0B', archived: colors.textDim };
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
