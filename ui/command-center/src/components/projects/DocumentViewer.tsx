/**
 * DocumentViewer (#471 Layer 2) — the in-app document viewer as a POLYMORPHIC
 * content renderer (the VS Code content-provider pattern).
 *
 * There is exactly ONE dispatch point — `pickRenderer(mimeType, filename)` maps
 * a document to a renderer kind. PDF / image / markdown / text / CSV each have a
 * content-type renderer; anything unrecognised falls through to a graceful
 * download-only fallback (never a blank or broken preview). Adding a type later
 * = add a case to `pickRenderer` + a branch in `renderContent` — no viewer
 * rework. That is the leverage: capabilities surface through one renderer.
 *
 * The bytes are fetched once (authed) into an object URL; `<img>`/`<iframe>`
 * render that URL directly, text renderers read `blob.text()`. The overlay
 * mirrors the DetailModal scrim/Escape convention but is sized wide for
 * documents (a deck needs room a 560px modal can't give).
 */

import { useEffect, useMemo, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import { FiDownload, FiX } from 'react-icons/fi';
import { api } from '../../lib/api';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { ProjectDocument } from './types';

// ── The single dispatch point ────────────────────────────────────────────────

type RendererKind = 'image' | 'pdf' | 'markdown' | 'csv' | 'text' | 'fallback';

/** Map a document to its renderer. The one place mime types become behaviour. */
function pickRenderer(mimeType: string, filename: string): RendererKind {
  const mime = mimeType.toLowerCase();
  const ext = filename.toLowerCase().split('.').pop() ?? '';

  if (mime.startsWith('image/')) return 'image';
  if (mime === 'application/pdf' || ext === 'pdf') return 'pdf';
  if (mime === 'text/markdown' || ext === 'md' || ext === 'markdown') return 'markdown';
  if (mime === 'text/csv' || ext === 'csv') return 'csv';
  if (mime.startsWith('text/') || mime === 'application/json' || mime === 'application/xml') {
    return 'text';
  }
  // XLSX / DOCX and every unknown type land here — download, no broken preview.
  return 'fallback';
}

// ── Viewer ───────────────────────────────────────────────────────────────────

export function DocumentViewer({ projectId, doc, onClose }: {
  projectId: string;
  doc: ProjectDocument;
  onClose: () => void;
}) {
  const { colors } = useTheme();
  const kind = useMemo(() => pickRenderer(doc.mime_type, doc.filename), [doc]);

  const [objectUrl, setObjectUrl] = useState<string | null>(null);
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Escape closes.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose]);

  // Fetch the bytes once. 'fallback' does not preview, so it skips the fetch.
  useEffect(() => {
    if (kind === 'fallback') return;
    let url: string | null = null;
    let live = true;
    api.fetchProjectDocumentBlob(projectId, doc.id)
      .then(async blob => {
        if (!live) return;
        if (kind === 'markdown' || kind === 'csv' || kind === 'text') {
          setText(await blob.text());
        } else {
          url = URL.createObjectURL(blob);
          setObjectUrl(url);
        }
      })
      .catch(e => { if (live) setError((e as Error).message || 'Failed to load document'); });
    return () => {
      live = false;
      if (url) URL.revokeObjectURL(url);
    };
  }, [projectId, doc.id, kind]);

  const download = async () => {
    try {
      const blob = await api.fetchProjectDocumentBlob(projectId, doc.id);
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = doc.filename;
      a.click();
      URL.revokeObjectURL(url);
    } catch (e) {
      setError((e as Error).message || 'Download failed');
    }
  };

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed', inset: 0, zIndex: 130, background: 'rgba(0,0,0,0.6)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
    >
      <div
        onClick={e => e.stopPropagation()}
        style={{
          width: 'min(1000px, 94vw)', height: '88vh',
          borderRadius: radius.lg, background: colors.surface,
          border: `1px solid ${colors.border}`,
          display: 'flex', flexDirection: 'column', overflow: 'hidden',
        }}
      >
        {/* Header */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 10, padding: '12px 16px',
          borderBottom: `1px solid ${colors.border}`,
        }}>
          <span style={{
            fontFamily: font.display, fontSize: 14, fontWeight: 600, color: colors.text,
            flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}>
            {doc.filename}
          </span>
          <span style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, flexShrink: 0 }}>
            {doc.mime_type} · {formatSize(doc.size_bytes)}
          </span>
          <IconButton title="Download" onClick={download} colors={colors}><FiDownload size={15} /></IconButton>
          <IconButton title="Close" onClick={onClose} colors={colors}><FiX size={16} /></IconButton>
        </div>

        {/* Body — the renderer dispatch */}
        <div style={{
          flex: 1, overflow: 'auto', minHeight: 0,
          background: kind === 'image' ? '#0b0b0f' : colors.surface,
        }}>
          {error
            ? <Notice colors={colors}>Couldn’t load this document: {error}</Notice>
            : renderContent(kind, { objectUrl, text, doc, colors, onDownload: download })}
        </div>
      </div>
    </div>
  );
}

// ── Renderers (one branch per kind; the fallback is never blank) ─────────────

function renderContent(
  kind: RendererKind,
  ctx: {
    objectUrl: string | null;
    text: string | null;
    doc: ProjectDocument;
    colors: ReturnType<typeof useTheme>['colors'];
    onDownload: () => void;
  },
) {
  const { objectUrl, text, doc, colors, onDownload } = ctx;

  switch (kind) {
    case 'image':
      return objectUrl
        ? <div style={{ display: 'flex', justifyContent: 'center', padding: 16 }}>
            <img src={objectUrl} alt={doc.filename} style={{ maxWidth: '100%', maxHeight: '100%', objectFit: 'contain' }} />
          </div>
        : <Loading colors={colors} />;

    case 'pdf':
      return objectUrl
        ? <iframe title={doc.filename} src={objectUrl} sandbox="" style={{ width: '100%', height: '100%', border: 'none' }} />
        : <Loading colors={colors} />;

    case 'markdown':
      return text !== null
        ? <div className="markdown-body" style={{ padding: '18px 22px', fontSize: 13, lineHeight: 1.6, color: colors.text }}>
            <ReactMarkdown>{text}</ReactMarkdown>
          </div>
        : <Loading colors={colors} />;

    case 'csv':
      return text !== null ? <CsvTable text={text} colors={colors} /> : <Loading colors={colors} />;

    case 'text':
      return text !== null
        ? <pre style={{
            margin: 0, padding: '18px 22px', fontFamily: font.mono, fontSize: 12,
            lineHeight: 1.55, color: colors.text, whiteSpace: 'pre-wrap', wordBreak: 'break-word',
          }}>{text}</pre>
        : <Loading colors={colors} />;

    case 'fallback':
    default:
      return (
        <div style={{
          height: '100%', display: 'flex', flexDirection: 'column', alignItems: 'center',
          justifyContent: 'center', gap: 14, padding: 24, textAlign: 'center',
        }}>
          <div style={{ fontSize: 13, color: colors.textMuted }}>
            No inline preview for <span style={{ fontFamily: font.mono }}>{doc.mime_type || 'this type'}</span>.
          </div>
          <button
            onClick={onDownload}
            style={{
              display: 'flex', alignItems: 'center', gap: 8, fontSize: 12, cursor: 'pointer',
              padding: '8px 14px', borderRadius: radius.md, color: colors.text,
              background: 'rgba(255,255,255,0.04)', border: `1px solid ${colors.border}`,
            }}
          >
            <FiDownload size={14} /> Download {doc.filename}
          </button>
        </div>
      );
  }
}

// ── CSV → table (minimal, quote-aware single-line fields) ────────────────────

function CsvTable({ text, colors }: { text: string; colors: ReturnType<typeof useTheme>['colors'] }) {
  const rows = useMemo(() => parseCsv(text), [text]);
  if (rows.length === 0) return <Notice colors={colors}>Empty file.</Notice>;
  const [head, ...body] = rows;
  return (
    <div style={{ padding: 16, overflow: 'auto' }}>
      <table style={{ borderCollapse: 'collapse', fontSize: 12, fontFamily: font.mono, color: colors.text }}>
        <thead>
          <tr>
            {head.map((cell, i) => (
              <th key={i} style={{
                textAlign: 'left', padding: '5px 10px', borderBottom: `1px solid ${colors.border}`,
                color: colors.textMuted, position: 'sticky', top: 0, background: colors.surface,
              }}>{cell}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {body.map((row, r) => (
            <tr key={r}>
              {row.map((cell, c) => (
                <td key={c} style={{ padding: '4px 10px', borderBottom: `1px solid ${colors.border}22` }}>{cell}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Parse CSV handling double-quoted fields (incl. escaped "" and commas). */
function parseCsv(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = '';
  let inQuotes = false;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (inQuotes) {
      if (ch === '"') {
        if (text[i + 1] === '"') { field += '"'; i++; } else { inQuotes = false; }
      } else { field += ch; }
    } else if (ch === '"') {
      inQuotes = true;
    } else if (ch === ',') {
      row.push(field); field = '';
    } else if (ch === '\n' || ch === '\r') {
      if (ch === '\r' && text[i + 1] === '\n') i++;
      row.push(field); field = '';
      if (row.some(c => c.length > 0) || rows.length > 0) rows.push(row);
      row = [];
    } else { field += ch; }
  }
  if (field.length > 0 || row.length > 0) { row.push(field); rows.push(row); }
  return rows.filter(r => r.length > 0);
}

// ── Small shared bits ────────────────────────────────────────────────────────

function IconButton({ title, onClick, colors, children }: {
  title: string; onClick: () => void; colors: ReturnType<typeof useTheme>['colors']; children: React.ReactNode;
}) {
  return (
    <button
      title={title}
      onClick={onClick}
      style={{ background: 'none', border: 'none', color: colors.textMuted, cursor: 'pointer', padding: 4, display: 'flex', flexShrink: 0 }}
    >
      {children}
    </button>
  );
}

function Loading({ colors }: { colors: ReturnType<typeof useTheme>['colors'] }) {
  return <div style={{ padding: 24, fontSize: 12, color: colors.textDim }}>Loading…</div>;
}

function Notice({ colors, children }: { colors: ReturnType<typeof useTheme>['colors']; children: React.ReactNode }) {
  return <div style={{ padding: 24, fontSize: 12, color: colors.textDim }}>{children}</div>;
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
