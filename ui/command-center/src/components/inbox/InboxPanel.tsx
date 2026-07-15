import { useEffect, useState, useCallback } from 'react';
import { useCommandCenter } from '../../lib/store';
import { api, type InboxFile } from '../../lib/api';
import { font } from '../../styles/tokens';
import { useTheme as useThemeHook } from '../../styles/useTheme';

function formatBytes(b: number | null): string {
  if (b == null) return '—';
  if (b < 1024) return `${b} B`;
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(0)} KB`;
  if (b < 1024 * 1024 * 1024) return `${(b / (1024 * 1024)).toFixed(1)} MB`;
  return `${(b / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function sourceLabel(url: string | null): string {
  if (!url) return '—';
  try { return new URL(url).hostname; } catch { return url; }
}

function receivedLabel(iso: string): string {
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  return d.toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
}

const COLS = '1fr 160px 90px 150px';

/**
 * Downloads inbox (#392/#393) — a minimal list of the files that landed in
 * ~/.permagent/inbox via the in-app browser download flow. Reads the real
 * `GET /api/inbox` endpoint (newest-first); recording a row (POST) is driven by
 * the desktop download bridge, not the UI. Rendered as an overlay when
 * activePanel === 'inbox' (agent-navigable via navigate_app("Inbox")).
 */
export function InboxPanel() {
  const { gradient, colors } = useThemeHook();
  const setActivePanel = useCommandCenter(s => s.setActivePanel);
  const [files, setFiles] = useState<InboxFile[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const dismiss = useCallback(() => setActivePanel('chat'), [setActivePanel]);

  useEffect(() => {
    let active = true;
    api.getInbox()
      .then(rows => { if (active) { setFiles(rows); setError(null); } })
      .catch(() => { if (active) { setFiles([]); setError('Could not load your inbox.'); } });
    return () => { active = false; };
  }, []);

  useEffect(() => {
    const h = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); dismiss(); } };
    window.addEventListener('keydown', h);
    return () => window.removeEventListener('keydown', h);
  }, [dismiss]);

  return (
    <div style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', background: gradient.shell, color: colors.text, fontFamily: font.body }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '20px 32px', borderBottom: `1px solid ${colors.border}` }}>
        <div style={{ flex: 1 }}>
          <div style={{ fontFamily: font.display, fontSize: 18, fontWeight: 700, letterSpacing: '-0.01em' }}>Downloads inbox</div>
          <div style={{ fontSize: 12, color: colors.textMuted, marginTop: 2 }}>
            Files you download in the in-app browser land here, in ~/.permagent/inbox — not lost in Finder.
          </div>
        </div>
        <button
          onClick={dismiss}
          style={{ height: 30, padding: '0 12px', borderRadius: 8, background: 'transparent', border: `1px solid ${colors.border}`, color: colors.textMuted, cursor: 'pointer', fontFamily: font.body, fontSize: 12 }}
        >Close</button>
      </div>
      <div style={{ flex: 1, overflow: 'auto', padding: '20px 32px' }}>
        {files === null ? (
          <div style={{ color: colors.textDim, fontSize: 13 }}>Loading inbox…</div>
        ) : files.length === 0 ? (
          <div style={{ color: colors.textMuted, fontSize: 13, padding: '24px 0' }}>
            {error ?? 'Your inbox is empty. Download a file in the in-app browser and it will appear here.'}
          </div>
        ) : (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            <div style={{ display: 'grid', gridTemplateColumns: COLS, gap: 12, padding: '0 12px', fontSize: 10, fontWeight: 600, letterSpacing: '0.08em', textTransform: 'uppercase', color: colors.textDim }}>
              <div>Filename</div><div>Source</div><div>Size</div><div>Received</div>
            </div>
            {files.map(f => (
              <div key={f.id} style={{ display: 'grid', gridTemplateColumns: COLS, gap: 12, alignItems: 'center', padding: 12, borderRadius: 10, background: colors.bgDeeper, border: `1px solid ${colors.border}` }}>
                <div style={{ minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: 13, fontWeight: 600 }} title={f.filename}>{f.filename}</div>
                <div style={{ minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: 12, color: colors.textMuted }} title={f.original_url ?? undefined}>{sourceLabel(f.original_url)}</div>
                <div style={{ fontSize: 12, color: colors.textMuted, fontFamily: font.mono }}>{formatBytes(f.size_bytes)}</div>
                <div style={{ fontSize: 12, color: colors.textMuted }}>{receivedLabel(f.created_at)}</div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
