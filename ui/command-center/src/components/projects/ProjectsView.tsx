import { useState, useEffect, useCallback, useRef } from 'react';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { apiFetch } from '../../lib/api';
import { useGoalEvents } from '../../lib/useGoalEvents';
import { useCommandCenter } from '../../lib/store';
import { ProjectWorkspace } from './ProjectWorkspace';
import { PERSONAL_ID, CANCELLABLE_STATES, type Project, type BoardColumn, type Card } from './types';

const LS_KEY = 'permagent-projects-last-opened';

// ── Main component ─────────────────────────────────────────────────────────

/** Emit a ProjectSelected activity event via Tauri IPC (fire-and-forget). */
function emitProjectSelected(project: Project) {
  import('@tauri-apps/api/core')
    .then(({ invoke }) => {
      invoke('emit_activity', {
        event_type: 'project_selected',
        source_surface: 'project_picker',
        payload: { project_id: project.id, project_name: project.name },
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
  const pendingProjectNavigation = useCommandCenter(s => s.pendingProjectNavigation);
  const setPendingProjectNavigation = useCommandCenter(s => s.setPendingProjectNavigation);

  const loadProjects = useCallback(async () => {
    try {
      const data = await apiFetch<Project[]>('/api/projects');
      setProjects(data);
    } catch {
      // silently fail
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadProjects();
    // Poll for new projects every 5s + refetch on window focus
    const interval = setInterval(loadProjects, 5000);
    const onFocus = () => loadProjects();
    window.addEventListener('focus', onFocus);
    return () => { clearInterval(interval); window.removeEventListener('focus', onFocus); };
  }, [loadProjects]);

  // On first load, restore last-opened project
  useEffect(() => {
    if (loading || projects.length === 0) return;
    const saved = localStorage.getItem(LS_KEY);
    if (saved && projects.some(p => p.id === saved)) {
      setActiveProjectId(saved);
    }
  }, [loading, projects]);

  // Agent/voice navigation: open a specific project when pendingProjectNavigation is set
  useEffect(() => {
    if (!pendingProjectNavigation || loading || projects.length === 0) return;
    const target = projects.find(p => p.id === pendingProjectNavigation);
    if (target) {
      setActiveProjectId(target.id);
      localStorage.setItem(LS_KEY, target.id);
      emitProjectSelected(target);
    }
    setPendingProjectNavigation(null);
  }, [pendingProjectNavigation, loading, projects, setPendingProjectNavigation]);

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
    } catch {
      // silently fail
    }
  }, [loadProjects]);

  if (loading) {
    return (
      <div style={{ width: '100%', height: '100%', display: 'grid', placeItems: 'center', background: gradient.workspace, color: colors.textMuted, fontFamily: font.body, fontSize: 13 }}>
        Loading projects...
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
      />
    );
  }

  return <AllProjectsView projects={projects} onOpenProject={openProject} onStatusChange={handleStatusChange} />;
}

// ── All Projects View — Active-dominant landing (Jesse's ruling 2026-07-10) ──
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
  const { colors } = useTheme();
  const { gradient } = useTheme();
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

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: gradient.workspace, color: colors.text, fontFamily: font.body }}>
      {/* Header */}
      <div style={{ padding: '16px 24px', borderBottom: `1px solid ${colors.border}`, flexShrink: 0 }}>
        <div style={{ fontFamily: font.display, fontSize: 16, fontWeight: 600, letterSpacing: '-0.01em' }}>Projects</div>
        <div style={{ fontSize: 11, color: colors.textMuted, marginTop: 2 }}>
          {active.length} active — drag a card onto Paused or Archived below to shelve it
        </div>
      </div>

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
            borderRadius: 12,
            padding: 4,
            border: dragOverCol === 'active' ? '1px solid rgba(0,213,255,0.25)' : '1px solid transparent',
            background: dragOverCol === 'active' ? 'rgba(0,213,255,0.04)' : 'transparent',
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
            <div style={{ gridColumn: '1 / -1', padding: 40, textAlign: 'center', fontSize: 12, color: colors.textDim }}>
              No active projects — create one, or drag one out of the shelves below.
            </div>
          )}
        </div>

        {/* PAUSED / ARCHIVED — slim disclosure shelves, also drop targets. */}
        {SECONDARY_STATUSES.map(col => {
          const colProjects = projects.filter(p => p.status === col.key).sort(byRecency);
          const isOver = dragOverCol === col.key;
          const isOpen = expanded[col.key] ?? false;
          return (
            <div
              key={col.key}
              onDragOver={(e) => handleDragOver(e, col.key)}
              onDragLeave={handleDragLeave}
              onDrop={(e) => handleDrop(e, col.key)}
              style={{
                marginTop: 14, borderRadius: 10,
                border: isOver ? '1px solid rgba(0,213,255,0.25)' : `1px solid ${colors.border}`,
                background: isOver ? 'rgba(0,213,255,0.05)' : 'rgba(255,255,255,0.02)',
                transition: 'all 150ms',
              }}
            >
              <button
                onClick={() => setExpanded(prev => ({ ...prev, [col.key]: !isOpen }))}
                style={{
                  width: '100%', display: 'flex', alignItems: 'center', gap: 8,
                  padding: '9px 14px', background: 'transparent', border: 'none',
                  color: colors.textMuted, cursor: 'pointer', fontFamily: font.body,
                }}
              >
                <span style={{
                  fontSize: 10, transform: isOpen ? 'rotate(90deg)' : 'none',
                  transition: 'transform 150ms', display: 'inline-block',
                }}>▶</span>
                <span style={{ fontSize: 11, fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.06em' }}>
                  {col.label}
                </span>
                <span style={{ fontSize: 10, color: colors.textDim, background: 'rgba(255,255,255,0.06)', padding: '1px 6px', borderRadius: 8 }}>
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
                <div style={{
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
                <div style={{ padding: '4px 14px 12px', fontSize: 11, color: colors.textDim }}>Empty.</div>
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
  const { colors } = useTheme();
  const isPersonal = project.id === PERSONAL_ID;

  return (
    <div
      draggable={!isPersonal}
      onDragStart={onDragStart}
      onClick={onOpen}
      style={{
        padding: '10px 12px', borderRadius: 8,
        background: colors.surface,
        border: `1px solid ${colors.border}`,
        cursor: 'pointer',
        transition: 'all 150ms',
      }}
      onMouseEnter={e => { (e.currentTarget as HTMLElement).style.borderColor = 'rgba(0,213,255,0.3)'; }}
      onMouseLeave={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.border; }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
        {isPersonal && (
          <span style={{ fontSize: 9, padding: '1px 5px', borderRadius: 4, background: 'rgba(0,213,255,0.1)', color: colors.cyan, fontWeight: 600 }}>
            DEFAULT
          </span>
        )}
        <span style={{ fontSize: 13, fontWeight: 600, flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {project.name}
        </span>
      </div>
      {project.description && (
        <div style={{ fontSize: 11, color: colors.textMuted, marginTop: 4, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {project.description}
        </div>
      )}
      {project.tags.length > 0 && (
        <div style={{ display: 'flex', gap: 4, marginTop: 6, flexWrap: 'wrap' }}>
          {project.tags.slice(0, 3).map((tag, ti) => (
            <span key={`${tag}-${ti}`} style={{ fontSize: 9, padding: '1px 5px', borderRadius: 4, background: 'rgba(255,255,255,0.06)', color: colors.textDim }}>
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
  const { colors } = useTheme();
  const { gradient } = useTheme();
  const [columns, setColumns] = useState<BoardColumn[]>([]);
  const [cards, setCards] = useState<Card[]>([]);
  const [loading, setLoading] = useState(true);
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
    } catch {
      // silently fail
    } finally {
      setLoading(false);
    }
  }, [project.id]);

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
      // No travel = a click → open the goal-detail modal instead of moving.
      if (!moved && cardId) {
        const card = cards.find(c => c.id === cardId);
        if (card?.cardType === 'goal') openGoalDetail(project.id, cardId);
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
          } catch { /* silently fail */ }
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

  const handleAddCard = async (columnId: string) => {
    if (!newCardTitle.trim()) return;
    try {
      await apiFetch(`/api/projects/${project.id}/cards`, {
        method: 'POST',
        body: JSON.stringify({ title: newCardTitle.trim(), columnId }),
      });
      setNewCardTitle('');
      setAddingCardCol(null);
      loadBoard();
    } catch {
      // silently fail
    }
  };

  const handleDeleteCard = async (cardId: string) => {
    try {
      await apiFetch(`/api/projects/${project.id}/cards/${cardId}`, { method: 'DELETE' });
      loadBoard();
    } catch {
      // silently fail
    }
  };

  // Cancel a goal (#490): kills the worker if running and moves it to the
  // terminal Cancelled column. Immediate — no approval gate.
  const handleCancelCard = async (cardId: string) => {
    try {
      await apiFetch(`/api/projects/${project.id}/cards/${cardId}/cancel`, { method: 'POST' });
      loadBoard();
    } catch {
      // silently fail
    }
  };

  // (Card drag handled by pointer events above — no HTML5 DnD)

  if (loading) {
    return (
      <div style={{ width: '100%', height: '100%', display: 'grid', placeItems: 'center', background: gradient.workspace, color: colors.textMuted, fontFamily: font.body, fontSize: 13 }}>
        Loading board...
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
                background: isOver ? 'rgba(0,213,255,0.04)' : 'rgba(255,255,255,0.02)',
                borderRadius: 10, padding: '12px 10px',
                border: isOver ? `1px solid rgba(0,213,255,0.2)` : '1px solid transparent',
                transition: 'all 150ms',
              }}
            >
              {/* Column header */}
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '0 4px 10px', borderBottom: `1px solid ${colors.border}` }}>
                <span style={{ fontSize: 12, fontWeight: 600, color: colors.textMuted, textTransform: 'uppercase', letterSpacing: '0.05em', flex: 1 }}>
                  {col.name}
                </span>
                <span style={{ fontSize: 10, color: colors.textDim, background: 'rgba(255,255,255,0.06)', padding: '1px 6px', borderRadius: 8 }}>
                  {colCards.length}
                </span>
              </div>

              {/* Cards */}
              <div style={{ flex: 1, paddingTop: 8, display: 'flex', flexDirection: 'column', gap: 6, overflow: 'auto' }}>
                {colCards.map(card => (
                  <CardItem
                    key={card.id}
                    card={card}
                    onPointerDown={(e) => handleCardPointerDown(e, card.id, card.title)}
                    isDragging={draggingCard === card.id}
                    onDelete={() => handleDeleteCard(card.id)}
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
                      padding: '6px 8px', borderRadius: 6,
                      background: 'rgba(255,255,255,0.06)',
                      border: `1px solid ${colors.border}`,
                      color: colors.text, fontFamily: font.body, fontSize: 12,
                      outline: 'none',
                    }}
                  />
                  <div style={{ display: 'flex', gap: 4 }}>
                    <button
                      onClick={() => handleAddCard(col.id)}
                      style={{
                        flex: 1, padding: '4px 0', borderRadius: 5,
                        background: 'rgba(0,213,255,0.15)', border: `1px solid rgba(0,213,255,0.3)`,
                        color: colors.cyan, fontSize: 11, fontFamily: font.body, fontWeight: 600, cursor: 'pointer',
                      }}
                    >
                      Add
                    </button>
                    <button
                      onClick={() => { setAddingCardCol(null); setNewCardTitle(''); }}
                      style={{
                        padding: '4px 8px', borderRadius: 5,
                        background: 'transparent', border: `1px solid ${colors.border}`,
                        color: colors.textMuted, fontSize: 11, fontFamily: font.body, cursor: 'pointer',
                      }}
                    >
                      Cancel
                    </button>
                  </div>
                </div>
              ) : (
                <button
                  onClick={() => setAddingCardCol(col.id)}
                  style={{
                    marginTop: 8, padding: '6px 0', borderRadius: 6,
                    background: 'transparent', border: `1px dashed ${colors.border}`,
                    color: colors.textDim, fontSize: 11, fontFamily: font.body,
                    cursor: 'pointer', transition: 'all 150ms',
                  }}
                  onMouseEnter={e => { (e.currentTarget as HTMLElement).style.borderColor = 'rgba(0,213,255,0.3)'; (e.currentTarget as HTMLElement).style.color = colors.textMuted; }}
                  onMouseLeave={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.border; (e.currentTarget as HTMLElement).style.color = colors.textDim; }}
                >
                  + Add card
                </button>
              )}
            </div>
          );
        })}
      </div>

      {/* Drag ghost — follows pointer during card drag */}
      {draggingCard && ghostPos && (
        <div style={{
          position: 'fixed', left: ghostPos.x + 8, top: ghostPos.y - 12,
          padding: '6px 10px', borderRadius: 6,
          background: colors.surface, border: `1px solid ${colors.cyan}`,
          boxShadow: '0 4px 16px rgba(0,0,0,0.3)',
          fontSize: 12, fontWeight: 500, color: colors.text,
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
card, onPointerDown, isDragging, onDelete, onCancel }: {
  card: Card;
  onPointerDown: (e: React.PointerEvent) => void;
  isDragging: boolean;
  onDelete: () => void;
  onCancel?: () => void;
}) {
  const { colors, gradient } = useTheme();
  const [showMenu, setShowMenu] = useState(false);

  return (
    <div
      onPointerDown={onPointerDown}
      onContextMenu={e => { e.preventDefault(); setShowMenu(!showMenu); }}
      style={{
        padding: '8px 10px', borderRadius: 7,
        background: colors.surface,
        border: `1px solid ${colors.border}`,
        cursor: 'grab', position: 'relative',
        opacity: isDragging ? 0.4 : 1,
        transition: 'opacity 150ms',
      }}
      onMouseEnter={e => { (e.currentTarget as HTMLElement).style.borderColor = 'rgba(255,255,255,0.12)'; }}
      onMouseLeave={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.border; setShowMenu(false); }}
    >
      <div style={{ fontSize: 12, fontWeight: 500 }}>{card.title}</div>
      {card.description && (
        <div style={{ fontSize: 10, color: colors.textMuted, marginTop: 3, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {card.description}
        </div>
      )}
      {card.cardType !== 'standard' && (
        <span style={{
          fontSize: 9, padding: '1px 5px', borderRadius: 4, marginTop: 4, display: 'inline-block',
          background: card.cardType === 'goal' ? 'rgba(139,92,246,0.15)' : 'rgba(233,30,99,0.15)',
          color: card.cardType === 'goal' ? '#A78BFA' : '#F48FB1',
        }}>
          {card.cardType}
        </span>
      )}
      {showMenu && (
        <div
          onPointerDown={e => e.stopPropagation()}
          style={{
            position: 'absolute', top: '100%', right: 0, marginTop: 2, zIndex: 10,
            background: gradient.dropdown, border: `1px solid ${colors.border}`, borderRadius: 6,
            boxShadow: '0 4px 16px rgba(0,0,0,0.4)', padding: 2, minWidth: 100,
          }}>
          {onCancel && (
            <button
              onClick={(e) => { e.stopPropagation(); setShowMenu(false); onCancel(); }}
              style={{
                width: '100%', padding: '5px 8px', borderRadius: 4,
                background: 'transparent', border: 'none',
                color: '#F59E0B', fontSize: 11, fontFamily: font.body,
                cursor: 'pointer', textAlign: 'left',
              }}
              onMouseEnter={e => { (e.currentTarget as HTMLElement).style.background = 'rgba(245,158,11,0.1)'; }}
              onMouseLeave={e => { (e.currentTarget as HTMLElement).style.background = 'transparent'; }}
            >
              Cancel goal
            </button>
          )}
          <button
            onClick={(e) => { e.stopPropagation(); onDelete(); }}
            style={{
              width: '100%', padding: '5px 8px', borderRadius: 4,
              background: 'transparent', border: 'none',
              color: '#EF4444', fontSize: 11, fontFamily: font.body,
              cursor: 'pointer', textAlign: 'left',
            }}
            onMouseEnter={e => { (e.currentTarget as HTMLElement).style.background = 'rgba(239,68,68,0.1)'; }}
            onMouseLeave={e => { (e.currentTarget as HTMLElement).style.background = 'transparent'; }}
          >
            Delete card
          </button>
        </div>
      )}
    </div>
  );
}
