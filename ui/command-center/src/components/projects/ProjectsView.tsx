import { useState, useEffect, useCallback, useRef, type CSSProperties } from 'react';
import { font, radius, textSize } from '../../styles/tokens';
import { Button } from '../common/Button';
import { useTheme } from '../../styles/useTheme';
import { apiFetch } from '../../lib/api';
import { insertRoadmapGoal } from '../../lib/roadmapClient';
import { toast } from '../../lib/notifications';
import { useGoalEvents } from '../../lib/useGoalEvents';
import { useCommandCenter } from '../../lib/store';
import { ProjectWorkspace } from './ProjectWorkspace';
import { CardDetailModal } from './CardDetailModal';
import { PERSONAL_ID, CANCELLABLE_STATES, type Project, type BoardColumn, type Card } from './types';
import { ViewHeader } from '../common/ViewHeader';
import { StateBlock } from '../common/StateBlock';

const LS_KEY = 'permagent-projects-last-opened';

function isProject(value: unknown): value is Project {
  if (!value || typeof value !== 'object') return false;
  const project = value as Record<string, unknown>;
  return typeof project.id === 'string'
    && typeof project.slug === 'string'
    && typeof project.name === 'string'
    && typeof project.status === 'string';
}

// ── Main component ─────────────────────────────────────────────────────────

/**
 * Emit a ProjectSelected activity event via Tauri IPC (fire-and-forget).
 *
 * `sessionId` is the id of the chat session this selection led to — pass it
 * whenever one is in scope (e.g. a "start chat in this project" action).
 * Opening a project's board with no chat session attached is a genuine,
 * sessionless selection: `sessionId` stays absent there, and `session_id` is
 * still sent explicitly as `null` (never omitted) to match the daemon's
 * ActivityEvent shape — never fabricate an id.
 */
function emitProjectSelected(project: Project, sessionId: string | null = null) {
  import('@tauri-apps/api/core')
    .then(({ invoke }) => {
      invoke('emit_activity', {
        event_type: 'project_selected',
        source_surface: 'project_picker',
        payload: { project_id: project.id, project_name: project.name },
        session_id: sessionId,
        project_id: `project:${project.slug}`,
      });
    })
    .catch(() => { /* not in Tauri context */ });
}

export function ProjectsView() {
  const { gradient, colors } = useTheme();
  const [projects, setProjects] = useState<Project[]>([]);
  const [activeProjectId, setActiveProjectId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(false);
  const pendingProjectNavigation = useCommandCenter(s => s.pendingProjectNavigation);
  const setPendingProjectNavigation = useCommandCenter(s => s.setPendingProjectNavigation);
  // #629 multi-client liveness: `project_changed` on /events bumps this, so a
  // status drag / create / delete from another device pushes instantly instead
  // of waiting for the 5s poll (which stays as the backstop).
  const projectsRev = useCommandCenter(s => s.projectsRev);
  const requestGeneration = useRef(0);

  const loadProjects = useCallback(async () => {
    const generation = ++requestGeneration.current;
    try {
      const data = await apiFetch<unknown>('/api/projects');
      if (generation !== requestGeneration.current) return;
      if (!Array.isArray(data) || !data.every(isProject)) {
        throw new Error('Malformed projects response');
      }
      setProjects(data);
      setError(false);
    } catch {
      if (generation !== requestGeneration.current) return;
      // Surface the failure instead of leaving a blank surface. Transient poll
      // failures are ignored while we already have projects on screen; only a
      // genuine load-with-nothing-to-show promotes to the error state below.
      setError(true);
    } finally {
      if (generation !== requestGeneration.current) return;
      setLoading(false);
    }
  }, []);

  const retry = useCallback(() => { setLoading(true); loadProjects(); }, [loadProjects]);

  useEffect(() => {
    loadProjects();
    // Poll for new projects every 5s + refetch on window focus
    const interval = setInterval(loadProjects, 5000);
    const onFocus = () => loadProjects();
    window.addEventListener('focus', onFocus);
    return () => {
      clearInterval(interval);
      window.removeEventListener('focus', onFocus);
      ++requestGeneration.current;
    };
  }, [loadProjects]);

  // Event-driven refetch (#629): fires only on a real remote mutation.
  useEffect(() => {
    if (projectsRev > 0) loadProjects();
  }, [projectsRev, loadProjects]);

  // On first load, restore last-opened project.
  useEffect(() => {
    if (loading || projects.length === 0) return;
    const saved = localStorage.getItem(LS_KEY);
    if (saved && projects.some(p => p.id === saved)) {
      setActiveProjectId(saved);
    }
  }, [loading, projects]);

  // Agent/voice navigation: open a specific project when pendingProjectNavigation
  // is set. The id is resolved daemon-side against the LIVE project list
  // (project_resolve queries the DB), which can be AHEAD of this view's ≤5s-old
  // polled snapshot — and in the voice flow the drill-in fires seconds later,
  // only after the narration audio drains. So a target that is missing from the
  // current `projects` is NOT proof it doesn't exist. Dropping it there (the old
  // behavior) IS #266: the drill-in "reports done" but the board never changes,
  // flaky on whether the snapshot happened to be current. Instead, force ONE
  // fresh load and HOLD the pending id; clear it only once applied, or once a
  // freshly-loaded list still lacks it (a genuinely stale/deleted id) — so the
  // navigation self-heals against a lagging cache without ever looping forever.
  const refreshedForPendingRef = useRef<string | null>(null);
  useEffect(() => {
    if (!pendingProjectNavigation || loading || projects.length === 0) return;
    const target = projects.find(p => p.id === pendingProjectNavigation);
    if (target) {
      setActiveProjectId(target.id);
      localStorage.setItem(LS_KEY, target.id);
      emitProjectSelected(target);
      refreshedForPendingRef.current = null;
      setPendingProjectNavigation(null);
      return;
    }
    // Absent from this snapshot. If we haven't already re-checked for THIS id,
    // force a fresh load (the snapshot may simply be stale) and keep the pending
    // id so the reloaded list can still resolve it.
    if (refreshedForPendingRef.current !== pendingProjectNavigation) {
      refreshedForPendingRef.current = pendingProjectNavigation;
      loadProjects();
      return;
    }
    // Already refreshed against the live list for this id and it's still not
    // there — a stale or deleted id. Drop it rather than hold a navigation that
    // can never resolve (and rather than re-loading on every future poll).
    refreshedForPendingRef.current = null;
    setPendingProjectNavigation(null);
  }, [pendingProjectNavigation, loading, projects, setPendingProjectNavigation, loadProjects]);

  const openProject = useCallback((id: string) => {
    setActiveProjectId(id);
    localStorage.setItem(LS_KEY, id);
    // Emit activity so agent knows which project is now active
    const project = projects.find(p => p.id === id);
    if (project) emitProjectSelected(project);
  }, [projects]);

  const backToAll = useCallback(() => {
    setActiveProjectId(null);
    localStorage.removeItem(LS_KEY);
  }, []);

  const handleStatusChange = useCallback(async (projectId: string, newStatus: string) => {
    try {
      await apiFetch(`/api/projects/${projectId}`, {
        method: 'PATCH',
        body: JSON.stringify({ status: newStatus }),
      });
      loadProjects();
    } catch (err) {
      // Surfaced (2026-07 wiring audit): a swallowed failure left the card
      // visually snapped back with no explanation.
      toast('Couldn\'t move project', err instanceof Error ? err.message : String(err));
    }
  }, [loadProjects]);

  if (loading) {
    return (
      <div style={{ width: '100%', height: '100%', display: 'grid', placeItems: 'center', background: gradient.workspace, color: colors.textMuted, fontFamily: font.body, fontSize: textSize.small }}>
        Loading projects...
      </div>
    );
  }

  // Initial load failed with nothing to show — an explicit, recoverable dead-end
  // instead of a blank canvas.
  if (error && projects.length === 0) {
    return (
      <div style={{ width: '100%', height: '100%', display: 'grid', placeItems: 'center', background: gradient.workspace, fontFamily: font.body }}>
        <StateBlock
          tone="error"
          title="Couldn't load projects"
          detail="The projects service didn't respond. Check that the daemon is running, then try again."
          onRetry={retry}
        />
      </div>
    );
  }

  if (activeProjectId) {
    const project = projects.find(p => p.id === activeProjectId);
    if (!project) {
      setActiveProjectId(null);
      return null;
    }
    return (
      <ProjectWorkspace
        project={project}
        projects={projects}
        onSwitchProject={openProject}
        onBack={backToAll}
        onProjectUpdated={loadProjects}
      />
    );
  }

  return (
    <AllProjectsView projects={projects} onOpenProject={openProject} onStatusChange={handleStatusChange} />
  );
}

// ── Shared empty / error state block ────────────────────────────────────────
//
// One shape for every "nothing to show" moment on the Projects surface, so an
// empty board and a failed fetch never read the same (blank) way. `error` tone
// gets a danger accent + a Retry affordance; `empty` tone is calm/dim.

// ── All Projects View — Active-dominant landing (ruling 2026-07-10) ──
//
// Progressive disclosure (the pattern behind Linear/Notion/Vercel landings):
// most projects are active, so Active IS the page — a responsive card grid.
// Paused and Archived collapse into slim disclosure strips below, expanding
// on demand and doubling as drop targets, so drag-to-change-status survives.

const SECONDARY_STATUSES = [
  { key: 'paused', label: 'Paused' },
  { key: 'archived', label: 'Archived' },
];

function AllProjectsView({
projects, onOpenProject, onStatusChange }: {
  projects: Project[];
  onOpenProject: (id: string) => void;
  onStatusChange: (id: string, status: string) => void;
}) {
  const { colors, theme } = useTheme();
  const { gradient } = useTheme();
  // White veils vanish on silver's white surfaces — flip to a faint graphite
  // tint there (same approach as BrainList's theme-conditional rows).
  const stripVeil = theme === 'silver' ? 'rgba(30,37,48,0.03)' : 'rgba(255,255,255,0.02)';
  const chipVeil = theme === 'silver' ? 'rgba(30,37,48,0.06)' : 'rgba(255,255,255,0.06)';
  const [dragOverCol, setDragOverCol] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});

  const handleDragStart = (e: React.DragEvent, projectId: string) => {
    e.dataTransfer.setData('text/plain', projectId);
    e.dataTransfer.effectAllowed = 'move';
  };

  const handleDragOver = (e: React.DragEvent, status: string) => {
    e.preventDefault();
    e.stopPropagation();
    e.dataTransfer.dropEffect = 'move';
    setDragOverCol(status);
  };

  const handleDragLeave = () => setDragOverCol(null);

  const handleDrop = (e: React.DragEvent, status: string) => {
    e.preventDefault();
    e.stopPropagation();
    setDragOverCol(null);
    const projectId = e.dataTransfer.getData('text/plain');
    if (projectId && projectId !== PERSONAL_ID) {
      onStatusChange(projectId, status);
    }
  };

  const byRecency = (a: Project, b: Project) => {
    if (a.id === PERSONAL_ID) return -1;
    if (b.id === PERSONAL_ID) return 1;
    return new Date(b.lastOpenedAt).getTime() - new Date(a.lastOpenedAt).getTime();
  };
  const active = projects.filter(p => p.status === 'active').sort(byRecency);
  const hasShelved = projects.some(p => p.status === 'paused' || p.status === 'archived');

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: gradient.workspace, color: colors.text, fontFamily: font.body }}>
      <ViewHeader
        title="Projects"
        subtitle={`${active.length} active — drag a card onto Paused or Archived below to shelve it`}
      />

      <div style={{ flex: 1, overflowY: 'auto', padding: '18px 24px' }}>
        {/* ACTIVE — the page. Responsive grid, drop target for reactivation. */}
        <div
          onDragOver={(e) => handleDragOver(e, 'active')}
          onDragLeave={handleDragLeave}
          onDrop={(e) => handleDrop(e, 'active')}
          style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fill, minmax(240px, 1fr))',
            gap: 12,
            borderRadius: radius.lg,
            padding: 4,
            border: dragOverCol === 'active' ? `1px solid ${colors.borderHi}` : '1px solid transparent',
            background: dragOverCol === 'active' ? colors.cyanSoft : 'transparent',
            transition: 'all 150ms',
          }}
        >
          {active.map(project => (
            <ProjectCard
              key={project.id}
              project={project}
              onOpen={() => onOpenProject(project.id)}
              onDragStart={(e) => handleDragStart(e, project.id)}
            />
          ))}
          {active.length === 0 && (
            <div style={{ gridColumn: '1 / -1' }}>
              <StateBlock
                tone="empty"
                title="No active projects yet"
                detail={hasShelved
                  ? 'Create one, or drag a project out of the Paused or Archived shelves below to reactivate it.'
                  : 'Create your first project to start organizing goals, people, and documents.'}
              />
            </div>
          )}
        </div>

        {/* PAUSED / ARCHIVED — slim disclosure shelves, also drop targets. */}
        {SECONDARY_STATUSES.map(col => {
          const colProjects = projects.filter(p => p.status === col.key).sort(byRecency);
          const isOver = dragOverCol === col.key;
          const isOpen = expanded[col.key] ?? false;
          const shelfId = `projects-shelf-${col.key}`;
          return (
            <div
              key={col.key}
              onDragOver={(e) => handleDragOver(e, col.key)}
              onDragLeave={handleDragLeave}
              onDrop={(e) => handleDrop(e, col.key)}
              style={{
                marginTop: 14, borderRadius: 10,
                border: isOver ? `1px solid ${colors.borderHi}` : `1px solid ${colors.border}`,
                background: isOver ? colors.cyanSoft : stripVeil,
                transition: 'all 150ms',
              }}
            >
              {/* Disclosure toggle for the shelf below it, so it keeps its own
                  element (a `Button` would wrap this row of chips in one inline
                  label span and collapse the layout) and takes the shared
                  `.pa-btn` interaction rules instead. */}
              <button
                className="pa-btn"
                aria-expanded={isOpen}
                aria-controls={shelfId}
                onClick={() => setExpanded(prev => ({ ...prev, [col.key]: !isOpen }))}
                style={{
                  '--pa-btn-fg': colors.textMuted,
                  '--pa-btn-fg-hover': colors.text,
                  '--pa-btn-bg-hover': 'transparent',
                  '--pa-btn-pad': '9px 14px',
                  width: '100%', gap: 8, justifyContent: 'flex-start',
                  fontFamily: font.body,
                } as CSSProperties}
              >
                <span style={{
                  fontSize: 10, transform: isOpen ? 'rotate(90deg)' : 'none',
                  transition: 'transform 150ms', display: 'inline-block',
                }}>▶</span>
                <span style={{ fontSize: textSize.micro, fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.06em' }}>
                  {col.label}
                </span>
                <span style={{ fontSize: 10, color: colors.textDim, background: chipVeil, padding: '1px 6px', borderRadius: radius.md }}>
                  {colProjects.length}
                </span>
                {!isOpen && colProjects.length > 0 && (
                  <span style={{ fontSize: 10, color: colors.textDim, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                    {colProjects.slice(0, 4).map(p => p.name).join(' · ')}{colProjects.length > 4 ? ' · …' : ''}
                  </span>
                )}
                {isOver && <span style={{ fontSize: 10, color: colors.cyan, marginLeft: 'auto' }}>drop to {col.label.toLowerCase()}</span>}
              </button>
              {isOpen && colProjects.length > 0 && (
                <div id={shelfId} style={{
                  display: 'grid',
                  gridTemplateColumns: 'repeat(auto-fill, minmax(240px, 1fr))',
                  gap: 10, padding: '4px 12px 12px',
                }}>
                  {colProjects.map(project => (
                    <ProjectCard
                      key={project.id}
                      project={project}
                      onOpen={() => onOpenProject(project.id)}
                      onDragStart={(e) => handleDragStart(e, project.id)}
                    />
                  ))}
                </div>
              )}
              {isOpen && colProjects.length === 0 && (
                <div id={shelfId} style={{ padding: '4px 14px 12px', fontSize: textSize.micro, color: colors.textDim }}>Empty.</div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function ProjectCard({
project, onOpen, onDragStart }: {
  project: Project;
  onOpen: () => void;
  onDragStart: (e: React.DragEvent) => void;
}) {
  const { colors, theme, reduceMotion } = useTheme();
  const tagVeil = theme === 'silver' ? 'rgba(30,37,48,0.06)' : 'rgba(255,255,255,0.06)';
  const isPersonal = project.id === PERSONAL_ID;

  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={`Open project ${project.name}`}
      draggable={!isPersonal}
      onDragStart={onDragStart}
      onClick={onOpen}
      onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onOpen(); } }}
      style={{
        padding: '10px 12px', borderRadius: radius.md,
        background: colors.surface,
        border: `1px solid ${colors.border}`,
        cursor: 'pointer',
        transition: reduceMotion ? 'none' : 'all 150ms',
      }}
      onMouseEnter={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.borderHi; }}
      onMouseLeave={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.border; }}
      onFocus={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.borderHi; }}
      onBlur={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.border; }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
        {isPersonal && (
          <span style={{ fontSize: 10, padding: '1px 5px', borderRadius: radius.xs, background: colors.cyanSoft, color: colors.cyan, fontWeight: 600 }}>
            DEFAULT
          </span>
        )}
        <span style={{ fontSize: textSize.small, fontWeight: 600, flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {project.name}
        </span>
      </div>
      {project.description && (
        <div style={{ fontSize: textSize.micro, color: colors.textMuted, marginTop: 4, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {project.description}
        </div>
      )}
      {project.tags.length > 0 && (
        <div style={{ display: 'flex', gap: 4, marginTop: 6, flexWrap: 'wrap' }}>
          {project.tags.slice(0, 3).map((tag, ti) => (
            <span key={`${tag}-${ti}`} style={{ fontSize: 10, padding: '1px 5px', borderRadius: radius.xs, background: tagVeil, color: colors.textDim }}>
              {tag}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Project Kanban (cards inside a project) ────────────────────────────────
//
// The board lens of the Projects tab. The shared workspace chrome
// (back · project switcher · view toggle) lives in ProjectWorkspace, so this
// renders only the columns — no header of its own.

export function ProjectKanban({ project }: { project: Project }) {
  const { colors, theme } = useTheme();
  const { gradient } = useTheme();
  // Theme-safe veils — white washes are invisible on silver's white surfaces.
  const colVeil = theme === 'silver' ? 'rgba(30,37,48,0.03)' : 'rgba(255,255,255,0.02)';
  const chipVeil = theme === 'silver' ? 'rgba(30,37,48,0.06)' : 'rgba(255,255,255,0.06)';
  const [columns, setColumns] = useState<BoardColumn[]>([]);
  const [cards, setCards] = useState<Card[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(false);
  const [addingCardCol, setAddingCardCol] = useState<string | null>(null);
  const [newCardTitle, setNewCardTitle] = useState('');
  const [dragOverCol, setDragOverCol] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Pointer-based card drag (bypasses HTML5 DnD which Tauri's native layer consumes)
  const [draggingCard, setDraggingCard] = useState<string | null>(null);
  const [ghostPos, setGhostPos] = useState<{ x: number; y: number } | null>(null);
  const [ghostLabel, setGhostLabel] = useState('');
  const colRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  // Distinguish a click (open detail) from a drag (move column): a press that
  // never travels past the threshold is a click. #503.
  const pressStart = useRef<{ x: number; y: number } | null>(null);
  const dragMoved = useRef(false);
  const openGoalDetail = useCommandCenter(s => s.openGoalDetail);
  const pendingCard = useCommandCenter(s => s.pendingCardNavigation);
  const clearPendingCardNavigation = useCommandCenter(s => s.clearPendingCardNavigation);
  // The card the dashboard's to-do list asked us to surface. Held in local
  // state so the ring outlives the store entry, which is cleared as soon as we
  // consume it (one-shot deep link — see openCardOnBoard).
  const [highlightedCardId, setHighlightedCardId] = useState<string | null>(null);
  const cardEls = useRef<Map<string, HTMLDivElement>>(new Map());
  // The non-goal card whose detail modal is open. Goals keep their own modal
  // (lifecycle, evidence, cancel); every other card type opens this one — until
  // now they opened nothing at all, which is why a click looked broken.
  const [detailCardId, setDetailCardId] = useState<string | null>(null);

  const getColumnAtPoint = useCallback((x: number, y: number): string | null => {
    for (const [colId, el] of colRefs.current.entries()) {
      const rect = el.getBoundingClientRect();
      if (x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom) {
        return colId;
      }
    }
    return null;
  }, []);

  const loadBoard = useCallback(async () => {
    try {
      const [cols, cds] = await Promise.all([
        apiFetch<BoardColumn[]>(`/api/projects/${project.id}/columns`),
        apiFetch<Card[]>(`/api/projects/${project.id}/cards`),
      ]);
      setColumns(cols);
      setCards(cds);
      setError(false);
    } catch {
      // Don't leave the board blank on failure — flag it so the render path
      // shows a recoverable error state instead of empty columns.
      setError(true);
    } finally {
      setLoading(false);
    }
  }, [project.id]);

  const retryBoard = useCallback(() => { setLoading(true); loadBoard(); }, [loadBoard]);

  // Deep link from the dashboard's to-do list. Waits for the card to actually
  // exist in this board's state — the navigation and the board fetch race, and
  // scrolling to an element that hasn't rendered silently does nothing. The
  // store entry is cleared on consumption so returning to Projects later does
  // not re-scroll to a stale card; the ring then fades on a timer.
  useEffect(() => {
    if (!pendingCard || pendingCard.projectId !== project.id) return;
    if (!cards.some(c => c.id === pendingCard.cardId)) return;
    const cardId = pendingCard.cardId;
    setHighlightedCardId(cardId);
    clearPendingCardNavigation();
    const el = cardEls.current.get(cardId);
    el?.scrollIntoView({ behavior: 'smooth', block: 'center' });
    const timer = setTimeout(() => setHighlightedCardId(null), 2600);
    return () => clearTimeout(timer);
  }, [pendingCard, project.id, cards, clearPendingCardNavigation]);

  // Set or clear a to-do's due date, then refetch so the badge reflects what
  // actually persisted rather than what we hoped would.
  const handleSetDueDate = useCallback(async (cardId: string, dueDate: string | null) => {
    try {
      await apiFetch(`/api/projects/${project.id}/cards/${cardId}/due-date`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ dueDate }),
      });
    } finally {
      await loadBoard();
    }
  }, [project.id, loadBoard]);

  useEffect(() => { loadBoard(); }, [loadBoard]);
  // Live board: refetch when any goal is created or transitions (shared
  // subscription) so Henry's changes appear without a full app reload.
  useGoalEvents(loadBoard);
  useEffect(() => { if (addingCardCol && inputRef.current) inputRef.current.focus(); }, [addingCardCol]);

  // Pointer drag: track move + handle drop on pointerup
  useEffect(() => {
    if (!draggingCard) return;
    const onMove = (e: PointerEvent) => {
      const start = pressStart.current;
      if (start && Math.hypot(e.clientX - start.x, e.clientY - start.y) > 4) {
        dragMoved.current = true;
      }
      setGhostPos({ x: e.clientX, y: e.clientY });
      setDragOverCol(getColumnAtPoint(e.clientX, e.clientY));
    };
    const onUp = async (e: PointerEvent) => {
      const targetCol = getColumnAtPoint(e.clientX, e.clientY);
      const cardId = draggingCard;
      const moved = dragMoved.current;
      setDraggingCard(null);
      setGhostPos(null);
      setDragOverCol(null);
      pressStart.current = null;
      dragMoved.current = false;
      // No travel = a click → open the card's detail instead of moving it. The
      // 4px threshold in onMove is what keeps the two apart: a drag sets
      // dragMoved and never opens anything, a press that never travels opens
      // the detail and never moves the card.
      if (!moved && cardId) {
        const card = cards.find(c => c.id === cardId);
        if (card?.cardType === 'goal') openGoalDetail(project.id, cardId);
        else if (card) setDetailCardId(cardId);
        return;
      }
      if (targetCol && cardId) {
        const card = cards.find(c => c.id === cardId);
        if (card && card.columnId !== targetCol) {
          try {
            await apiFetch(`/api/projects/${project.id}/cards/${cardId}`, {
              method: 'PATCH',
              body: JSON.stringify({ columnId: targetCol }),
            });
            loadBoard();
          } catch (err) {
            toast('Couldn\'t move card', err instanceof Error ? err.message : String(err));
          }
        }
      }
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    return () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
  }, [draggingCard, cards, project.id, loadBoard, getColumnAtPoint, openGoalDetail]);

  const handleCardPointerDown = (e: React.PointerEvent, cardId: string, title: string) => {
    e.preventDefault();
    pressStart.current = { x: e.clientX, y: e.clientY };
    dragMoved.current = false;
    setDraggingCard(cardId);
    setGhostPos({ x: e.clientX, y: e.clientY });
    setGhostLabel(title);
  };

  // Resolves `true`/`false` rather than void: the Add button reads the result to
  // decide whether it may tick, and this helper swallows its own failure into a
  // toast, so a silent `undefined` would confirm an add that never happened.
  const handleAddCard = async (columnId: string): Promise<boolean> => {
    if (!newCardTitle.trim()) return false;
    try {
      const column = columns.find(c => c.id === columnId);
      if (column?.stateBinding === 'triage') {
        // #251: adding into Triage inserts a real roadmap goal (validated
        // dependency wiring, lifecycle metadata) rather than a bare card.
        await insertRoadmapGoal(project.id, { title: newCardTitle.trim() });
      } else {
        await apiFetch(`/api/projects/${project.id}/cards`, {
          method: 'POST',
          body: JSON.stringify({ title: newCardTitle.trim(), columnId }),
        });
      }
      setNewCardTitle('');
      setAddingCardCol(null);
      loadBoard();
      return true;
    } catch (err) {
      toast('Couldn\'t add card', err instanceof Error ? err.message : String(err));
      return false;
    }
  };

  const handleDeleteCard = async (cardId: string) => {
    try {
      await apiFetch(`/api/projects/${project.id}/cards/${cardId}`, { method: 'DELETE' });
      loadBoard();
    } catch (err) {
      toast('Couldn\'t delete card', err instanceof Error ? err.message : String(err));
    }
  };

  // Cancel a goal (#490): kills the worker if running and moves it to the
  // terminal Cancelled column. Immediate — no approval gate.
  const handleCancelCard = async (cardId: string) => {
    try {
      await apiFetch(`/api/projects/${project.id}/cards/${cardId}/cancel`, { method: 'POST' });
      loadBoard();
    } catch (err) {
      toast('Couldn\'t cancel goal', err instanceof Error ? err.message : String(err));
    }
  };

  // (Card drag handled by pointer events above — no HTML5 DnD)

  if (loading) {
    return (
      <div style={{ width: '100%', height: '100%', display: 'grid', placeItems: 'center', background: gradient.workspace, color: colors.textMuted, fontFamily: font.body, fontSize: textSize.small }}>
        Loading board...
      </div>
    );
  }

  // Fetch failed — a recoverable error rather than a blank board.
  if (error && columns.length === 0) {
    return (
      <div style={{ width: '100%', flex: 1, minHeight: 0, display: 'grid', placeItems: 'center', background: gradient.workspace, fontFamily: font.body }}>
        <StateBlock
          tone="error"
          title="Couldn't load this board"
          detail="The board's columns and cards didn't load. Check the daemon connection and try again."
          onRetry={retryBoard}
        />
      </div>
    );
  }

  // Loaded cleanly but there is no board configured yet.
  if (columns.length === 0) {
    return (
      <div style={{ width: '100%', flex: 1, minHeight: 0, display: 'grid', placeItems: 'center', background: gradient.workspace, fontFamily: font.body }}>
        <StateBlock
          tone="empty"
          title="No board yet"
          detail="This project doesn't have any columns. A board is created automatically once the project has its first goal or card."
        />
      </div>
    );
  }

  return (
    <div style={{ width: '100%', flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', background: gradient.workspace, color: colors.text, fontFamily: font.body }}>
      {/* Kanban columns */}
      <div style={{ flex: 1, display: 'flex', gap: 1, padding: '16px 16px', overflow: 'auto' }}>
        {columns.map(col => {
          const colCards = cards.filter(c => c.columnId === col.id).sort((a, b) => a.position - b.position);
          const isOver = dragOverCol === col.id;

          return (
            <div
              key={col.id}
              ref={(el) => { if (el) colRefs.current.set(col.id, el); else colRefs.current.delete(col.id); }}
              style={{
                flex: 1, minWidth: 200, display: 'flex', flexDirection: 'column',
                background: isOver ? colors.cyanSoft : colVeil,
                borderRadius: 10, padding: '12px 10px',
                border: isOver ? `1px solid ${colors.borderHi}` : '1px solid transparent',
                transition: 'all 150ms',
              }}
            >
              {/* Column header */}
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '0 4px 10px', borderBottom: `1px solid ${colors.border}` }}>
                <span style={{ fontSize: textSize.caption, fontWeight: 600, color: colors.textMuted, textTransform: 'uppercase', letterSpacing: '0.05em', flex: 1 }}>
                  {col.name}
                </span>
                <span style={{ fontSize: 10, color: colors.textDim, background: chipVeil, padding: '1px 6px', borderRadius: radius.md }}>
                  {colCards.length}
                </span>
              </div>

              {/* Cards */}
              <div style={{ flex: 1, paddingTop: 8, display: 'flex', flexDirection: 'column', gap: 6, overflow: 'auto' }}>
                {colCards.map(card => (
                  <CardItem
                    key={card.id}
                    card={card}
                    cardRef={el => {
                      if (el) cardEls.current.set(card.id, el);
                      else cardEls.current.delete(card.id);
                    }}
                    highlighted={highlightedCardId === card.id}
                    onPointerDown={(e) => handleCardPointerDown(e, card.id, card.title)}
                    onOpen={card.cardType === 'goal'
                      ? () => openGoalDetail(project.id, card.id)
                      : () => setDetailCardId(card.id)}
                    isDragging={draggingCard === card.id}
                    onDelete={() => handleDeleteCard(card.id)}
                    onSetDueDate={
                      card.cardType === 'standard'
                        ? (dueDate) => { void handleSetDueDate(card.id, dueDate); }
                        : undefined
                    }
                    onCancel={
                      card.cardType === 'goal' &&
                      CANCELLABLE_STATES.includes(col.stateBinding ?? '')
                        ? () => handleCancelCard(card.id)
                        : undefined
                    }
                  />
                ))}
              </div>

              {/* Add card */}
              {addingCardCol === col.id ? (
                <div style={{ marginTop: 8, display: 'flex', flexDirection: 'column', gap: 4 }}>
                  <input
                    ref={inputRef}
                    value={newCardTitle}
                    onChange={e => setNewCardTitle(e.target.value)}
                    onKeyDown={e => {
                      if (e.key === 'Enter') handleAddCard(col.id);
                      if (e.key === 'Escape') { setAddingCardCol(null); setNewCardTitle(''); }
                    }}
                    placeholder="Card title..."
                    style={{
                      padding: '6px 8px', borderRadius: radius.sm,
                      background: colors.inputBg,
                      border: `1px solid ${colors.border}`,
                      color: colors.text, fontFamily: font.body, fontSize: textSize.caption,
                      outline: 'none',
                    }}
                  />
                  <div style={{ display: 'flex', gap: 4 }}>
                    <Button
                      colors={colors}
                      variant="ghostOn"
                      onClick={() => handleAddCard(col.id)}
                      style={{
                        '--pa-btn-bg': colors.cyanSoft,
                        '--pa-btn-bg-hover': colors.cyanSoft,
                        '--pa-btn-border': colors.borderHi,
                        '--pa-btn-border-hover': colors.cyan,
                        '--pa-btn-pad': '4px 0',
                        '--pa-btn-radius': '5px',
                        '--pa-btn-weight': 600,
                        flex: 1, fontSize: textSize.micro, fontFamily: font.body,
                      } as CSSProperties}
                    >
                      Add
                    </Button>
                    <Button
                      colors={colors}
                      onClick={() => { setAddingCardCol(null); setNewCardTitle(''); }}
                      style={{
                        '--pa-btn-fg': colors.textMuted,
                        '--pa-btn-fg-hover': colors.text,
                        '--pa-btn-border': colors.border,
                        '--pa-btn-border-hover': colors.borderHi,
                        '--pa-btn-pad': '4px 8px',
                        '--pa-btn-radius': '5px',
                        fontSize: textSize.micro, fontFamily: font.body,
                      } as CSSProperties}
                    >
                      Cancel
                    </Button>
                  </div>
                </div>
              ) : (
                <Button
                  colors={colors}
                  onClick={() => setAddingCardCol(col.id)}
                  style={{
                    '--pa-btn-fg': colors.textDim,
                    '--pa-btn-fg-hover': colors.textMuted,
                    '--pa-btn-border': colors.border,
                    '--pa-btn-border-hover': colors.borderHi,
                    '--pa-btn-bg-hover': 'transparent',
                    '--pa-btn-pad': '6px 0',
                    '--pa-btn-radius': `${radius.sm}px`,
                    // The dashed rule is what says "not a card yet". `.pa-btn`
                    // owns the 1px width and the (hoverable) color, so only the
                    // style is overridden here.
                    borderStyle: 'dashed',
                    marginTop: 8, fontSize: textSize.micro, fontFamily: font.body,
                  } as CSSProperties}
                >
                  + Add card
                </Button>
              )}
            </div>
          );
        })}
      </div>

      {/* Card detail (#503 sibling) — opened by a click that did not drag. */}
      {detailCardId && (
        <CardDetailModal
          projectId={project.id}
          projectName={project.name}
          cardId={detailCardId}
          onClose={() => setDetailCardId(null)}
          onSaved={loadBoard}
        />
      )}

      {/* Drag ghost — follows pointer during card drag */}
      {draggingCard && ghostPos && (
        <div style={{
          position: 'fixed', left: ghostPos.x + 8, top: ghostPos.y - 12,
          padding: '6px 10px', borderRadius: radius.sm,
          background: colors.surface, border: `1px solid ${colors.cyan}`,
          boxShadow: colors.elevationRaised,
          fontSize: textSize.caption, fontWeight: 500, color: colors.text,
          pointerEvents: 'none', zIndex: 9999, maxWidth: 200,
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>
          {ghostLabel}
        </div>
      )}
    </div>
  );
}

function CardItem({
card, onPointerDown, onOpen, isDragging, onDelete, onCancel, onSetDueDate, highlighted, cardRef }: {
  card: Card;
  onPointerDown: (e: React.PointerEvent) => void;
  /** Activate the card via keyboard — mirrors the click path the parent's
   *  pointer handler runs (goal detail for goals, card detail for the rest).
   *  Every card is openable, so in practice this is always supplied; it stays
   *  optional so the component does not force a caller to invent a handler. */
  onOpen?: () => void;
  isDragging: boolean;
  onDelete: () => void;
  onCancel?: () => void;
  /** Set (or clear, with null) the to-do's due date. Absent for goal cards,
   *  whose scheduling belongs to the goal lifecycle, not a date field. */
  onSetDueDate?: (dueDate: string | null) => void;
  /** Briefly ringed after a deep-link from the dashboard's to-do list, so the
   *  card you clicked is findable on a board that may hold dozens. */
  highlighted?: boolean;
  cardRef?: (el: HTMLDivElement | null) => void;
}) {
  const { colors, gradient, reduceMotion } = useTheme();
  const [showMenu, setShowMenu] = useState(false);
  const [editingDue, setEditingDue] = useState(false);
  const isGoal = card.cardType === 'goal';
  const dueDate = typeof card.metadataJson?.dueDate === 'string' ? card.metadataJson.dueDate : null;

  return (
    <div
      ref={cardRef}
      role="button"
      tabIndex={0}
      aria-label={onOpen ? `Open card ${card.title}` : card.title}
      onPointerDown={onPointerDown}
      onKeyDown={e => {
        if (onOpen && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); onOpen(); }
        if (e.key === 'Escape') setShowMenu(false);
      }}
      onContextMenu={e => { e.preventDefault(); setShowMenu(m => !m); }}
      style={{
        padding: '8px 10px', borderRadius: 7,
        background: colors.surface,
        border: `1px solid ${highlighted ? colors.cyan : colors.border}`,
        boxShadow: highlighted ? `0 0 0 3px ${colors.cyanSoft}` : undefined,
        cursor: 'grab', position: 'relative',
        opacity: isDragging ? 0.4 : 1,
        transition: reduceMotion ? 'none' : 'opacity 150ms, box-shadow 300ms, border-color 300ms',
      }}
      onMouseEnter={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.borderHi; }}
      onMouseLeave={e => { (e.currentTarget as HTMLElement).style.borderColor = highlighted ? colors.cyan : colors.border; setShowMenu(false); }}
      onFocus={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.borderHi; }}
      onBlur={e => { (e.currentTarget as HTMLElement).style.borderColor = highlighted ? colors.cyan : colors.border; }}
    >
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 6 }}>
        <div style={{ fontSize: textSize.caption, fontWeight: 500, flex: 1, minWidth: 0 }}>{card.title}</div>
        {/* Visible, keyboard-reachable menu trigger — right-click still works too.
            stopPropagation on pointer-down so opening the menu never starts a drag.

            It was already labelled and already focusable; what it lacked was
            looking like a control. At 18×18 with no chrome at any state it read
            as a stray character that happened to sit in the corner, and it was
            below any reasonable pointer target — on a card you are also
            expected to drag, so a near-miss starts a drag instead of opening a
            menu. It is now a 28px target with resting chrome and a `title`, so
            it is both hittable and identifiable before it is hovered. */}
        <button
          className="pa-btn"
          data-testid={`card-menu-${card.id}`}
          aria-label={`Card actions for ${card.title}`}
          aria-haspopup="menu"
          aria-expanded={showMenu}
          title="Card actions"
          onPointerDown={e => e.stopPropagation()}
          onClick={e => { e.stopPropagation(); setShowMenu(m => !m); }}
          style={{
            '--pa-btn-bg': showMenu ? colors.surfaceHi : 'transparent',
            '--pa-btn-fg': showMenu ? colors.text : colors.textDim,
            '--pa-btn-border': showMenu ? colors.borderHi : colors.border,
            '--pa-btn-bg-hover': colors.surfaceHi,
            '--pa-btn-fg-hover': colors.text,
            '--pa-btn-border-hover': colors.borderHi,
            '--pa-btn-pad': '0',
            '--pa-btn-radius': `${radius.sm}px`,
            flexShrink: 0,
            width: 28, height: 28, marginTop: -4, marginRight: -6,
            lineHeight: 1, fontSize: textSize.body,
            transition: reduceMotion ? 'none' : undefined,
          } as CSSProperties}
        >
          ⋯
        </button>
      </div>
      {card.description && (
        <div style={{ fontSize: 10, color: colors.textMuted, marginTop: 3, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {card.description}
        </div>
      )}
      {editingDue && onSetDueDate ? (
        <input
          type="date"
          autoFocus
          defaultValue={dueDate ?? ''}
          onPointerDown={e => e.stopPropagation()}
          onClick={e => e.stopPropagation()}
          onBlur={e => { onSetDueDate(e.target.value || null); setEditingDue(false); }}
          onKeyDown={e => {
            e.stopPropagation();
            if (e.key === 'Enter') { onSetDueDate((e.target as HTMLInputElement).value || null); setEditingDue(false); }
            if (e.key === 'Escape') setEditingDue(false);
          }}
          style={{
            marginTop: 5, fontSize: 10, padding: '2px 4px', width: '100%', boxSizing: 'border-box',
            borderRadius: radius.xs, border: `1px solid ${colors.border}`,
            background: colors.surface, color: colors.text,
          }}
        />
      ) : dueDate ? (
        <Button
          colors={colors}
          variant="bare"
          onPointerDown={e => e.stopPropagation()}
          onClick={e => { e.stopPropagation(); if (onSetDueDate) setEditingDue(true); }}
          title={onSetDueDate ? 'Change the due date' : undefined}
          style={{
            '--pa-btn-bg': colors.cyanSoft,
            '--pa-btn-fg': colors.cyan,
            // Without a handler this chip is a label, not an affordance: it must
            // not light up under the mouse as if it did something.
            '--pa-btn-bg-hover': colors.cyanSoft,
            '--pa-btn-bg-active': colors.cyanSoft,
            '--pa-btn-border-hover': onSetDueDate ? colors.cyan : 'transparent',
            '--pa-btn-pad': '1px 5px',
            '--pa-btn-radius': `${radius.xs}px`,
            fontSize: 10, marginTop: 4, fontFamily: font.body,
            cursor: onSetDueDate ? 'pointer' : 'default',
          } as CSSProperties}
        >
          Due {dueDate}
        </Button>
      ) : null}
      {card.cardType !== 'standard' && (
        <span style={{
          fontSize: 10, padding: '1px 5px', borderRadius: radius.xs, marginTop: 4, display: 'inline-block',
          background: isGoal ? colors.purpleSoft : colors.cyanSoft,
          color: isGoal ? colors.purpleBright : colors.cyan,
        }}>
          {card.cardType}
        </span>
      )}
      {showMenu && (
        <div
          role="menu"
          onPointerDown={e => e.stopPropagation()}
          style={{
            position: 'absolute', top: '100%', right: 0, marginTop: 2, zIndex: 10,
            background: gradient.dropdown, border: `1px solid ${colors.border}`, borderRadius: radius.sm,
            boxShadow: colors.elevationRaised, padding: 2, minWidth: 100,
          }}>
          {/* The menu rows keep their own elements: `role="menuitem"` is what
              makes this a menu, and `Button` would flatten it. They take the
              shared `.pa-btn` interaction rules instead. */}
          {onCancel && (
            <button
              className="pa-btn"
              role="menuitem"
              onClick={(e) => { e.stopPropagation(); setShowMenu(false); onCancel(); }}
              style={{
                '--pa-btn-fg': colors.warning,
                '--pa-btn-bg-hover': colors.warning + '1A',
                '--pa-btn-bg-active': colors.warning + '2E',
                '--pa-btn-pad': '5px 8px',
                '--pa-btn-radius': `${radius.xs}px`,
                width: '100%', justifyContent: 'flex-start', textAlign: 'left',
                fontSize: textSize.micro, fontFamily: font.body,
              } as CSSProperties}
            >
              Cancel goal
            </button>
          )}
          {onSetDueDate && (
            <button
              className="pa-btn"
              role="menuitem"
              onClick={(e) => { e.stopPropagation(); setShowMenu(false); setEditingDue(true); }}
              style={{
                '--pa-btn-fg': colors.textMuted,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-bg-hover': colors.borderHi,
                '--pa-btn-bg-active': colors.cyanSoft,
                '--pa-btn-pad': '5px 8px',
                '--pa-btn-radius': `${radius.xs}px`,
                width: '100%', justifyContent: 'flex-start', textAlign: 'left',
                fontSize: textSize.micro, fontFamily: font.body,
              } as CSSProperties}
            >
              {dueDate ? 'Change due date' : 'Set due date'}
            </button>
          )}
          {onSetDueDate && dueDate && (
            <button
              className="pa-btn"
              role="menuitem"
              onClick={(e) => { e.stopPropagation(); setShowMenu(false); onSetDueDate(null); }}
              style={{
                '--pa-btn-fg': colors.textMuted,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-bg-hover': colors.borderHi,
                '--pa-btn-bg-active': colors.cyanSoft,
                '--pa-btn-pad': '5px 8px',
                '--pa-btn-radius': `${radius.xs}px`,
                width: '100%', justifyContent: 'flex-start', textAlign: 'left',
                fontSize: textSize.micro, fontFamily: font.body,
              } as CSSProperties}
            >
              Clear due date
            </button>
          )}
          <button
            className="pa-btn"
            role="menuitem"
            onClick={(e) => { e.stopPropagation(); setShowMenu(false); onDelete(); }}
            style={{
              '--pa-btn-fg': colors.danger,
              '--pa-btn-bg-hover': colors.danger + '1A',
              '--pa-btn-bg-active': colors.danger + '2E',
              '--pa-btn-pad': '5px 8px',
              '--pa-btn-radius': `${radius.xs}px`,
              width: '100%', justifyContent: 'flex-start', textAlign: 'left',
              fontSize: textSize.micro, fontFamily: font.body,
            } as CSSProperties}
          >
            Delete card
          </button>
        </div>
      )}
    </div>
  );
}
