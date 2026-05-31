import { useState, useEffect, useCallback, useRef } from 'react';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { apiFetch } from '../../lib/api';

// ── Types ──────────────────────────────────────────────────────────────────

interface Project {
  id: string;
  slug: string;
  name: string;
  description: string;
  status: string;
  rootPath: string | null;
  siteUrl: string | null;
  repoUrl: string | null;
  tags: string[];
  lastOpenedAt: string;
}

interface BoardColumn {
  id: string;
  projectId: string;
  name: string;
  position: number;
  columnKind: string;
  wipLimit: number | null;
}

interface Card {
  id: string;
  projectId: string;
  cardType: string;
  title: string;
  description: string;
  columnId: string;
  position: number;
  createdBy: string;
  assignedTo: string | null;
  metadataJson: Record<string, unknown>;
  createdAt: string;
  updatedAt: string;
  archivedAt: string | null;
}

const PERSONAL_ID = '00000000-0000-0000-0000-000000000001';
const LS_KEY = 'permagent-projects-last-opened';

// ── Main component ─────────────────────────────────────────────────────────

export function ProjectsView() {
  const { gradient, colors } = useTheme();
  const [projects, setProjects] = useState<Project[]>([]);
  const [activeProjectId, setActiveProjectId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

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

  const openProject = useCallback((id: string) => {
    setActiveProjectId(id);
    localStorage.setItem(LS_KEY, id);
  }, []);

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
    return <ProjectDetailView project={project} onBack={backToAll} />;
  }

  return <AllProjectsView projects={projects} onOpenProject={openProject} onStatusChange={handleStatusChange} />;
}

// ── All Projects View (projects-as-cards in status columns) ────────────────

const STATUS_COLUMNS = [
  { key: 'active', label: 'Active' },
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

  const handleDragStart = (e: React.DragEvent, projectId: string) => {
    e.dataTransfer.setData('text/plain', projectId);
    e.dataTransfer.effectAllowed = 'move';
  };

  const handleDragOver = (e: React.DragEvent, status: string) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = 'move';
    setDragOverCol(status);
  };

  const handleDragLeave = () => setDragOverCol(null);

  const handleDrop = (e: React.DragEvent, status: string) => {
    e.preventDefault();
    setDragOverCol(null);
    const projectId = e.dataTransfer.getData('text/plain');
    if (projectId && projectId !== PERSONAL_ID) {
      onStatusChange(projectId, status);
    }
  };

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: gradient.workspace, color: colors.text, fontFamily: font.body }}>
      {/* Header */}
      <div style={{ padding: '16px 24px', borderBottom: `1px solid ${colors.border}`, flexShrink: 0 }}>
        <div style={{ fontFamily: font.display, fontSize: 16, fontWeight: 600, letterSpacing: '-0.01em' }}>Projects</div>
        <div style={{ fontSize: 11, color: colors.textMuted, marginTop: 2 }}>
          {projects.length} project{projects.length !== 1 ? 's' : ''} — drag to change status
        </div>
      </div>

      {/* Kanban columns */}
      <div style={{ flex: 1, display: 'flex', gap: 1, padding: '16px 16px', overflow: 'auto' }}>
        {STATUS_COLUMNS.map(col => {
          const colProjects = projects.filter(p => p.status === col.key);
          const isOver = dragOverCol === col.key;

          return (
            <div
              key={col.key}
              onDragOver={(e) => handleDragOver(e, col.key)}
              onDragLeave={handleDragLeave}
              onDrop={(e) => handleDrop(e, col.key)}
              style={{
                flex: 1, minWidth: 220, display: 'flex', flexDirection: 'column',
                background: isOver ? 'rgba(0,213,255,0.04)' : 'rgba(255,255,255,0.02)',
                borderRadius: 10, padding: '12px 10px',
                border: isOver ? `1px solid rgba(0,213,255,0.2)` : '1px solid transparent',
                transition: 'all 150ms',
              }}
            >
              {/* Column header */}
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '0 4px 10px', borderBottom: `1px solid ${colors.border}` }}>
                <span style={{ fontSize: 12, fontWeight: 600, color: colors.textMuted, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                  {col.label}
                </span>
                <span style={{ fontSize: 10, color: colors.textDim, background: 'rgba(255,255,255,0.06)', padding: '1px 6px', borderRadius: 8 }}>
                  {colProjects.length}
                </span>
              </div>

              {/* Project cards */}
              <div style={{ flex: 1, paddingTop: 8, display: 'flex', flexDirection: 'column', gap: 6, overflow: 'auto' }}>
                {colProjects
                  .sort((a, b) => {
                    if (a.id === PERSONAL_ID) return -1;
                    if (b.id === PERSONAL_ID) return 1;
                    return new Date(b.lastOpenedAt).getTime() - new Date(a.lastOpenedAt).getTime();
                  })
                  .map(project => (
                    <ProjectCard
                      key={project.id}
                      project={project}
                      onOpen={() => onOpenProject(project.id)}
                      onDragStart={(e) => handleDragStart(e, project.id)}
                    />
                  ))}
              </div>
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
        background: 'rgba(255,255,255,0.04)',
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
          {project.tags.slice(0, 3).map(tag => (
            <span key={tag} style={{ fontSize: 9, padding: '1px 5px', borderRadius: 4, background: 'rgba(255,255,255,0.06)', color: colors.textDim }}>
              {tag}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Project Detail View (cards inside a project) ───────────────────────────

function ProjectDetailView({
project, onBack }: {
  project: Project;
  onBack: () => void;
}) {
  const { colors } = useTheme();
  const { gradient } = useTheme();
  const [columns, setColumns] = useState<BoardColumn[]>([]);
  const [cards, setCards] = useState<Card[]>([]);
  const [loading, setLoading] = useState(true);
  const [addingCardCol, setAddingCardCol] = useState<string | null>(null);
  const [newCardTitle, setNewCardTitle] = useState('');
  const [dragOverCol, setDragOverCol] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

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
  useEffect(() => { if (addingCardCol && inputRef.current) inputRef.current.focus(); }, [addingCardCol]);

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

  // Drag and drop for cards between columns
  const handleCardDragStart = (e: React.DragEvent, cardId: string) => {
    e.dataTransfer.setData('text/plain', cardId);
    e.dataTransfer.effectAllowed = 'move';
  };

  const handleColDragOver = (e: React.DragEvent, colId: string) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = 'move';
    setDragOverCol(colId);
  };

  const handleColDragLeave = () => setDragOverCol(null);

  const handleColDrop = async (e: React.DragEvent, colId: string) => {
    e.preventDefault();
    setDragOverCol(null);
    const cardId = e.dataTransfer.getData('text/plain');
    if (!cardId) return;
    try {
      await apiFetch(`/api/projects/${project.id}/cards/${cardId}`, {
        method: 'PATCH',
        body: JSON.stringify({ columnId: colId }),
      });
      loadBoard();
    } catch {
      // silently fail
    }
  };

  if (loading) {
    return (
      <div style={{ width: '100%', height: '100%', display: 'grid', placeItems: 'center', background: gradient.workspace, color: colors.textMuted, fontFamily: font.body, fontSize: 13 }}>
        Loading board...
      </div>
    );
  }

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: gradient.workspace, color: colors.text, fontFamily: font.body }}>
      {/* Header */}
      <div style={{ padding: '12px 24px', borderBottom: `1px solid ${colors.border}`, flexShrink: 0, display: 'flex', alignItems: 'center', gap: 12 }}>
        <button onClick={onBack} style={{
          background: 'none', border: 'none', color: colors.textMuted, cursor: 'pointer',
          padding: '4px 8px', borderRadius: 6, fontSize: 12, fontFamily: font.body,
          display: 'flex', alignItems: 'center', gap: 4,
        }}
          onMouseEnter={e => { (e.currentTarget as HTMLElement).style.color = colors.text; }}
          onMouseLeave={e => { (e.currentTarget as HTMLElement).style.color = colors.textMuted; }}
        >
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
            <path d="M19 12H5M12 19l-7-7 7-7" />
          </svg>
          All Projects
        </button>
        <div style={{ width: 1, height: 16, background: colors.border }} />
        <div>
          <div style={{ fontFamily: font.display, fontSize: 15, fontWeight: 600, letterSpacing: '-0.01em' }}>
            {project.name}
          </div>
          <div style={{ fontSize: 10, color: colors.textMuted, marginTop: 1 }}>
            {project.slug} · {cards.length} card{cards.length !== 1 ? 's' : ''}
          </div>
        </div>
      </div>

      {/* Kanban columns */}
      <div style={{ flex: 1, display: 'flex', gap: 1, padding: '16px 16px', overflow: 'auto' }}>
        {columns.map(col => {
          const colCards = cards.filter(c => c.columnId === col.id).sort((a, b) => a.position - b.position);
          const isOver = dragOverCol === col.id;

          return (
            <div
              key={col.id}
              onDragOver={(e) => handleColDragOver(e, col.id)}
              onDragLeave={handleColDragLeave}
              onDrop={(e) => handleColDrop(e, col.id)}
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
                    onDragStart={(e) => handleCardDragStart(e, card.id)}
                    onDelete={() => handleDeleteCard(card.id)}
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
    </div>
  );
}

function CardItem({
card, onDragStart, onDelete }: {
  card: Card;
  onDragStart: (e: React.DragEvent) => void;
  onDelete: () => void;
}) {
  const { colors } = useTheme();
  const [showMenu, setShowMenu] = useState(false);

  return (
    <div
      draggable
      onDragStart={onDragStart}
      onContextMenu={e => { e.preventDefault(); setShowMenu(!showMenu); }}
      style={{
        padding: '8px 10px', borderRadius: 7,
        background: 'rgba(255,255,255,0.04)',
        border: `1px solid ${colors.border}`,
        cursor: 'grab', position: 'relative',
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
        <div style={{
          position: 'absolute', top: '100%', right: 0, marginTop: 2, zIndex: 10,
          background: '#0F1729', border: `1px solid ${colors.border}`, borderRadius: 6,
          boxShadow: '0 4px 16px rgba(0,0,0,0.4)', padding: 2, minWidth: 100,
        }}>
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
