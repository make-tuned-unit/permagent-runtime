/**
 * DocumentsPanel (#471 Layer 2) — the Documents panel on a project's Overview.
 *
 * Lists the files attached to a project (`GET /api/projects/{id}/documents`),
 * uploads via file-picker or drag-drop (`POST …/documents`, multipart — the
 * same storage mechanism as chat attachments, scoped to a project), and opens
 * the in-app polymorphic viewer on click. Delete removes the row + file.
 *
 * Observability (per the #568 empty-body lesson): upload and delete failures
 * surface inline instead of a silent catch, so a non-2xx is visible rather than
 * looking like a dead click. Reuses the Panel shell + associate-button chrome.
 */

import { useCallback, useEffect, useRef, useState, type CSSProperties } from 'react';
import { FiFile, FiTrash2, FiUploadCloud } from 'react-icons/fi';
import { api } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { projectMemoryPreview } from '../brain/brainMemoryFocus';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { Panel } from './Panel';
import { DocumentViewer, formatSize } from './DocumentViewer';
import type { Project, ProjectDocument } from './types';

import { Tooltip } from '../common/Tooltip';
export function DocumentsPanel({ project }: { project: Project }) {
  const { colors } = useTheme();
  // White veils vanish on silver — flip to a faint graphite tint there.
  const rowVeil = colors.fillSubtle;
  // `fillSubtle` IS this idiom, tokenised (#1162). The hand-written pair it
  // replaces predates the token and was a shade off on both themes; the token
  // carries the THEME's own ink, so one name reads as a lift on the void and
  // as a shade on the pearl without a conditional here.
  const [docs, setDocs] = useState<ProjectDocument[]>([]);
  const [viewing, setViewing] = useState<ProjectDocument | null>(null);
  const [dragging, setDragging] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // First-load / load-failure / ready — kept separate from `error` (which is for
  // upload+delete failures) so a failed *list* fetch shows a recoverable error
  // instead of the empty drop-zone, and never a perpetual spinner.
  const [status, setStatus] = useState<'loading' | 'error' | 'ready'>('loading');
  const fileInput = useRef<HTMLInputElement>(null);
  const loadGeneration = useRef(0);
  // #629 multi-client liveness: bumped by `project_changed` on /events, so an
  // upload/delete from another device refetches this list.
  const projectsRev = useCommandCenter(s => s.projectsRev);
  const focusBrainMemory = useCommandCenter(s => s.focusBrainMemory);

  // Resolves `false` when the load failed (or was superseded) so the retry
  // button can only tick over a load that actually landed.
  const load = useCallback(async () => {
    const generation = ++loadGeneration.current;
    try {
      const nextDocs = await api.listProjectDocuments(project.id);
      if (generation !== loadGeneration.current) return false;
      if (!Array.isArray(nextDocs)) throw new Error('Invalid documents response');
      setDocs(nextDocs);
      setStatus('ready');
      return true;
    } catch {
      if (generation !== loadGeneration.current) return false;
      // Surface as a recoverable error rather than masquerading as empty.
      setStatus('error');
      return false;
    }
  }, [project.id]);

  useEffect(() => { load(); }, [load, projectsRev]);

  const upload = useCallback(async (files: File[]) => {
    if (files.length === 0) return;
    setError(null);
    setBusy(true);
    try {
      await api.uploadProjectDocuments(project.id, files);
      await load();
      try {
        const mems = await api.listProjectMemories(project.id);
        const hit = mems[0];
        if (hit) {
          focusBrainMemory({
            id: hit.id,
            key: hit.key,
            preview: projectMemoryPreview(hit),
          });
        }
      } catch {
        /* indexing is best-effort; the file row already landed */
      }
    } catch (e) {
      const err = e as Error;
      setError(`Upload failed: ${err.message || 'request failed'}`);
    } finally {
      setBusy(false);
    }
  }, [project.id, load, focusBrainMemory]);

  const remove = async (doc: ProjectDocument) => {
    setError(null);
    try {
      const res = await api.deleteProjectDocument(project.id, doc.id);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      setViewing(v => (v?.id === doc.id ? null : v));
      await load();
      return true;
    } catch (e) {
      setError(`Couldn’t delete ${doc.filename}: ${(e as Error).message || 'request failed'}`);
      return false;
    }
  };

  return (
    <>
      <Panel
        title="Documents"
        action={
          <Button
            colors={colors}
            variant="bare"
            className="hover:underline"
            onClick={() => fileInput.current?.click()}
            disabled={busy}
            style={{
              '--pa-btn-fg': colors.cyan,
              '--pa-btn-bg-hover': 'transparent',
              '--pa-btn-pad': '0',
              '--pa-btn-weight': 'inherit',
              fontFamily: font.body,
              fontSize: textSize.micro,
            } as CSSProperties}
          >
            {busy ? 'Uploading…' : '+ Upload'}
          </Button>
        }
      >
        <input
          ref={fileInput}
          type="file"
          multiple
          hidden
          onChange={e => {
            upload(Array.from(e.target.files ?? []));
            e.target.value = '';
          }}
        />

        {/* Drop zone — also the empty state. */}
        <div
          role="button"
          tabIndex={0}
          aria-label="Upload documents"
          onDragOver={e => { e.preventDefault(); setDragging(true); }}
          onDragLeave={() => setDragging(false)}
          onDrop={e => {
            e.preventDefault();
            setDragging(false);
            upload(Array.from(e.dataTransfer.files ?? []));
          }}
          onClick={() => fileInput.current?.click()}
          onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); fileInput.current?.click(); } }}
          style={{
            display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 8,
            padding: '10px 12px', marginBottom: docs.length ? 8 : 0, cursor: 'pointer',
            borderRadius: radius.md, fontSize: textSize.micro,
            color: dragging ? colors.cyan : colors.textDim,
            border: `1px dashed ${dragging ? colors.cyan : colors.border}`,
            background: dragging ? colors.cyanSoft : 'transparent',
            transition: 'all 150ms',
          }}
        >
          <FiUploadCloud size={14} />
          {docs.length === 0 ? 'Drop files here — they become searchable in Brain' : 'Add more'}
        </div>

        {error && (
          <div style={{ fontSize: textSize.micro, color: colors.danger, marginBottom: 8 }}>{error}</div>
        )}

        {status === 'loading' && (
          <div style={{ fontSize: textSize.micro, color: colors.textDim }}>Loading documents…</div>
        )}

        {status === 'error' && (
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <span style={{ fontSize: textSize.micro, color: colors.danger }}>Couldn't load documents.</span>
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

        {status === 'ready' && docs.length > 0 && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            {docs.map(doc => (
              <div
                key={doc.id}
                style={{
                  display: 'flex', alignItems: 'center', gap: 8, padding: '6px 9px',
                  borderRadius: radius.md, background: rowVeil, border: `1px solid ${colors.border}`,
                  transition: 'border-color 150ms',
                }}
                onMouseEnter={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.borderHi; }}
                onMouseLeave={e => { (e.currentTarget as HTMLElement).style.borderColor = colors.border; }}
              >
                <Tooltip content="Open">
                  <Button
                    colors={colors}
                    variant="bare"
                    onClick={() => setViewing(doc)}
                    // `contents` dissolves Button's `.pa-btn__label` wrapper so
                    // the icon, the truncating filename and the size stay the
                    // button's own flex row — otherwise the ellipsis is lost.
                                      style={{
                      '--pa-btn-fg': colors.text,
                      '--pa-btn-bg-hover': 'transparent',
                      '--pa-btn-fg-hover': colors.cyan,
                      '--pa-btn-pad': '0',
                      justifyContent: 'flex-start',
                      textAlign: 'left',
                      flex: 1,
                      minWidth: 0,
                      gap: 8,
                      fontFamily: font.body,
                    } as CSSProperties}
                  >
                    <FiFile size={13} color={colors.textMuted} style={{ flexShrink: 0 }} />
                    <span style={{ fontSize: textSize.caption, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                      {doc.filename}
                    </span>
                    <span style={{ fontSize: 10, color: colors.textDim, flexShrink: 0 }}>{formatSize(doc.size_bytes)}</span>
                  </Button>
                </Tooltip>
                <Tooltip content="Delete">
                  <Button
                    colors={colors}
                    variant="bare"
                    onClick={() => remove(doc)}
                    aria-label="Delete"
                    style={{
                      '--pa-btn-fg': colors.textDim,
                      '--pa-btn-fg-hover': colors.danger,
                      '--pa-btn-bg-hover': 'transparent',
                      '--pa-btn-pad': '2px',
                      flexShrink: 0,
                    } as CSSProperties}
                  >
                    <FiTrash2 size={13} />
                  </Button>
                </Tooltip>
              </div>
            ))}
          </div>
        )}
      </Panel>

      {viewing && (
        <DocumentViewer projectId={project.id} doc={viewing} onClose={() => setViewing(null)} />
      )}
    </>
  );
}
