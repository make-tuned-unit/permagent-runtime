/**
 * MemoriesPanel — the Brain-loop READ side on a project's Overview.
 *
 * Closes the loop the flow audit flagged: Notes, Documents and the Codebase
 * index all WRITE into the Brain and auto-associate with the project, but nothing
 * linked back — the Brain was a write-only sink from the product's own surfaces.
 * This panel lists the Brain memories associated with the project
 * (`GET /api/projects/:id/memories`, resolved from the LIVE Brain, newest
 * association first) and makes each one clickable to open/focus it in the Brain
 * view via the shared focus seam (focusBrainMemory → BrainView). It fills the
 * slot ProjectOverview reserved for the association layer.
 *
 * Read-only by design (the audit's priority): associations are already written
 * automatically by the note / document / code-index flows, so there is no manual
 * associate/disassociate UI here — just the loop back. Observability per the
 * #568 empty-body lesson: load failures surface inline with a retry, never a
 * silent catch. Styled strictly with the shared Panel shell + theme tokens.
 */

import { useCallback, useEffect, useRef, useState, type CSSProperties } from 'react';
import { FiExternalLink, FiRefreshCw } from 'react-icons/fi';
import { api } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { projectMemoryPreview } from '../brain/brainMemoryFocus';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { Panel } from './Panel';
import type { Project, ProjectMemory } from './types';

import { Tooltip } from '../common/Tooltip';
export function MemoriesPanel({ project }: { project: Project }) {
  const { colors } = useTheme();
  // White veils vanish on silver — flip to a faint graphite tint there.
  const rowVeil = colors.fillSubtle;
  // `fillSubtle` IS this idiom, tokenised (#1162). The hand-written pair it
  // replaces predates the token and was a shade off on both themes; the token
  // carries the THEME's own ink, so one name reads as a lift on the void and
  // as a shade on the pearl without a conditional here.
  const [memories, setMemories] = useState<ProjectMemory[]>([]);
  const [status, setStatus] = useState<'loading' | 'error' | 'ready'>('loading');
  const loadGeneration = useRef(0);
  const focusBrainMemory = useCommandCenter(s => s.focusBrainMemory);
  // #629 multi-client liveness: `project_changed` (change=memories) from
  // another device refetches this association list.
  const projectsRev = useCommandCenter(s => s.projectsRev);

  // Resolves `false` when the load failed (or was superseded) so the refresh /
  // retry buttons can only tick over a load that actually landed.
  const load = useCallback(async () => {
    const generation = ++loadGeneration.current;
    setStatus('loading');
    try {
      const nextMemories = await api.listProjectMemories(project.id);
      if (generation !== loadGeneration.current) return false;
      if (!Array.isArray(nextMemories)) throw new Error('Invalid memories response');
      setMemories(nextMemories);
      setStatus('ready');
      return true;
    } catch {
      if (generation !== loadGeneration.current) return false;
      setStatus('error');
      return false;
    }
  }, [project.id]);

  useEffect(() => { load(); }, [load, projectsRev]);

  // Deep-link into the Brain. The full memory is already in hand, so pass it as
  // the preview: it renders immediately even when the memory isn't in the graph
  // yet (fresh + description-less writes are excluded until the Librarian
  // enriches them), and the live graph copy still wins when present.
  const openInBrain = (m: ProjectMemory) =>
    focusBrainMemory({ id: m.id, key: m.key, preview: projectMemoryPreview(m) });

  return (
    <Panel
      title="Memories"
      action={
        status === 'ready' ? (
          <span style={{ display: 'inline-flex', alignItems: 'center', gap: 8 }}>
            <span style={{ fontSize: 10, color: colors.textDim }}>
              {memories.length} linked
            </span>
            <Tooltip content="Refresh">
              <Button
                colors={colors}
                variant="bare"
                onClick={load}
                aria-label="Refresh memories"
                style={{
                  '--pa-btn-fg': colors.textDim,
                  '--pa-btn-fg-hover': colors.textMuted,
                  '--pa-btn-bg-hover': 'transparent',
                  '--pa-btn-pad': '0',
                } as CSSProperties}
              >
                <FiRefreshCw size={12} />
              </Button>
            </Tooltip>
          </span>
        ) : undefined
      }
    >
      {status === 'loading' && (
        <div style={{ fontSize: textSize.micro, color: colors.textDim }}>Recalling memories…</div>
      )}

      {status === 'error' && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <span style={{ fontSize: textSize.micro, color: colors.danger }}>Couldn't load memories.</span>
          <Button
            colors={colors}
            variant="bare"
            className="hover:underline"
            onClick={load}
            style={{
              '--pa-btn-fg': colors.cyan,
              '--pa-btn-bg-hover': 'transparent',
              '--pa-btn-pad': '0',
              '--pa-btn-weight': 600,
              fontFamily: font.body,
              fontSize: textSize.micro,
            } as CSSProperties}
          >
            Retry
          </Button>
        </div>
      )}

      {status === 'ready' && memories.length === 0 && (
        <div style={{ fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.55 }}>
          Nothing linked yet. Notes you write, documents you drop, and this
          project's indexed code land in your Brain and surface here — each one
          clickable straight to where it lives.
        </div>
      )}

      {status === 'ready' && memories.length > 0 && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {memories.map(m => (
            <Tooltip content="Open in Brain">
              <button
                key={m.id}
                onClick={() => openInBrain(m)}
                style={{
                  textAlign: 'left', width: '100%', cursor: 'pointer',
                  display: 'flex', alignItems: 'flex-start', gap: 8, padding: '8px 10px',
                  borderRadius: radius.md, background: rowVeil, border: `1px solid ${colors.border}`,
                  color: colors.text, fontFamily: font.body, transition: 'border-color 150ms',
                }}
                onMouseEnter={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.borderHi; }}
                onMouseLeave={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.border; }}
              >
                <div style={{ flex: 1, minWidth: 0 }}>
                  {/* Librarian description reads best; before enrichment it's null,
                      so fall back to the raw content the Brain stored. */}
                  <div style={{
                    fontSize: textSize.caption, color: colors.text, lineHeight: 1.5,
                    display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical', overflow: 'hidden',
                  }}>
                    {m.description || m.content || '(empty memory)'}
                  </div>
                  <div style={{ fontSize: 10, color: colors.textDim, marginTop: 4, display: 'flex', gap: 8, alignItems: 'center', minWidth: 0 }}>
                    <span style={{ flexShrink: 0 }}>{relativeTime(m.associated_at)}</span>
                    {m.description && m.content && (
                      <span style={{ minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontFamily: font.mono, opacity: 0.85 }}>
                        {m.content}
                      </span>
                    )}
                  </div>
                </div>
                <FiExternalLink size={12} style={{ color: colors.cyan, flexShrink: 0, marginTop: 2 }} />
              </button>
            </Tooltip>
          ))}
        </div>
      )}
    </Panel>
  );
}

/** Compact relative time ("just now", "3h ago", "2d ago"), falling back to a
 *  date for older items. The backend stores an ISO-8601 / UTC timestamp. */
function relativeTime(iso: string): string {
  const then = new Date(iso.endsWith('Z') || iso.includes('+') ? iso : `${iso}Z`).getTime();
  if (Number.isNaN(then)) return iso;
  const secs = Math.max(0, Math.floor((Date.now() - then) / 1000));
  if (secs < 45) return 'just now';
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  if (days < 7) return `${days}d ago`;
  try {
    return new Date(then).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
  } catch {
    return iso;
  }
}
