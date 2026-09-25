/**
 * Models → Local models (GGUF) — ST2, dagK settings pass.
 *
 * `local-inference/*` (crates/goose-server/src/routes/local_inference.rs) is
 * the on-device GGUF runtime distinct from Ollama (above, in ModelsPanel) —
 * it had no UI at all. This lists the registry (featured + anything already
 * downloaded), shows live download progress, and lets the user download or
 * delete a model. Delete is confirmed: it is unrecoverable disk space, and
 * deleting the model backing an active route would leave that role broken.
 */

import { useCallback, useEffect, useState, type CSSProperties } from 'react';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { api, type LocalModelInfo } from '../../lib/api';
import { ListRow, ModelStateBadge, Section } from './atoms';
import { Button } from '../common/Button';
import { ConfirmDialog } from '../common/ConfirmDialog';
import { StateBlock } from '../common/StateBlock';

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

/** Local-model status. Reuses `ModelStateBadge`'s installed/missing vocabulary
 *  (R6 — one badge style) for the two steady states; downloading gets its own
 *  percent pill rather than borrowing Ollama's "Loaded" label, which means
 *  something already in memory — a download in flight is neither. */
function statusDetail(m: LocalModelInfo): { state: 'installed' | 'missing' | null; pct: number | null; detail: string } {
  if (m.status.state === 'Downloaded') return { state: 'installed', pct: null, detail: `${m.repo_id} · ${m.quantization} · ${formatBytes(m.size_bytes)}` };
  if (m.status.state === 'Downloading') {
    const pct = Math.round(m.status.progress_percent);
    const speed = m.status.speed_bps ? ` · ${formatBytes(m.status.speed_bps)}/s` : '';
    return { state: null, pct, detail: `Downloading ${pct}% — ${formatBytes(m.status.bytes_downloaded)} of ${formatBytes(m.status.total_bytes)}${speed}` };
  }
  return { state: 'missing', pct: null, detail: `${m.repo_id} · ${m.quantization} · ${formatBytes(m.size_bytes)} · not downloaded` };
}

export function LocalModelsSection() {
  const { colors } = useTheme();
  const [models, setModels] = useState<LocalModelInfo[] | null>(null);
  const [error, setError] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [rowError, setRowError] = useState<Record<string, string>>({});

  const refresh = useCallback(() => {
    return api.listLocalModels()
      .then(list => { setModels(list); setError(false); return true; })
      .catch(() => { setError(true); return false; });
  }, []);

  useEffect(() => {
    void refresh();
    const id = setInterval(refresh, 4000);
    return () => clearInterval(id);
  }, [refresh]);

  const download = async (m: LocalModelInfo) => {
    setBusyId(m.id);
    setRowError(r => ({ ...r, [m.id]: '' }));
    try {
      await api.downloadLocalModel(`${m.repo_id}:${m.quantization}`);
      await refresh();
      return true;
    } catch (err) {
      setRowError(r => ({ ...r, [m.id]: err instanceof Error ? err.message : 'Download failed' }));
      return false;
    } finally {
      setBusyId(null);
    }
  };

  const doDelete = async (id: string) => {
    await api.deleteLocalModel(id);
    await refresh();
    setConfirmDeleteId(null);
  };

  return (
    <Section
      title="Local models (GGUF)"
      sub="Models that run entirely on this machine, no cloud call — separate from the Ollama models above. Download once, then route Chat, Voice, or Harness to a provider that uses it."
    >
      {error ? (
        <StateBlock tone="error" compact title="Couldn't read the local-model registry." onRetry={() => { void refresh(); }} />
      ) : models === null ? (
        <div style={{ fontSize: textSize.caption, color: colors.textDim, padding: '6px 0' }}>Loading local models…</div>
      ) : models.length === 0 ? (
        <div style={{ fontSize: textSize.caption, color: colors.textDim, padding: '6px 0' }}>No local models in the registry.</div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: space.md }}>
          {models.map(m => {
            const { state, pct, detail } = statusDetail(m);
            return (
              <ListRow
                key={m.id}
                title={m.id}
                chip={m.recommended && <span style={{ fontSize: textSize.micro, color: colors.cyan, fontWeight: 600 }}>Recommended</span>}
                status={state ? <ModelStateBadge state={state} /> : (
                  <span style={{ fontSize: textSize.micro, fontWeight: 600, padding: `${space.xxs}px ${space.md}px`, borderRadius: radius.pill, background: colors.cyanSoft, color: colors.cyan }}>
                    {pct}%
                  </span>
                )}
                detail={
                  <>
                    {detail}
                    {rowError[m.id] && <div style={{ color: colors.danger, marginTop: space.xxs }}>{rowError[m.id]}</div>}
                  </>
                }
                trailing={
                  <div style={{ display: 'flex', gap: space.md }}>
                    {m.status.state === 'NotDownloaded' && (
                      <Button
                        colors={colors}
                        disabled={busyId === m.id}
                        onClick={() => download(m)}
                        style={{
                          '--pa-btn-bg': colors.cyanSoft, '--pa-btn-fg': colors.cyan,
                          '--pa-btn-border': colors.borderHi, '--pa-btn-border-hover': colors.cyan,
                          '--pa-btn-bg-hover': colors.cyanSoft, '--pa-btn-bg-active': colors.cyanGlow,
                          '--pa-btn-pad': '3px 10px', '--pa-btn-radius': `${radius.sm}px`,
                          fontSize: textSize.micro, fontFamily: font.body,
                        } as CSSProperties}
                        data-testid={`local-model-download-${m.id}`}
                      >
                        {busyId === m.id ? 'Starting…' : 'Download'}
                      </Button>
                    )}
                    {m.status.state === 'Downloaded' && (
                      <Button
                        colors={colors}
                        onClick={() => setConfirmDeleteId(m.id)}
                        style={{
                          '--pa-btn-fg': colors.danger, '--pa-btn-border': colors.border,
                          '--pa-btn-border-hover': `${colors.danger}4D`, '--pa-btn-bg-hover': `${colors.danger}1A`,
                          '--pa-btn-bg-active': `${colors.danger}26`,
                          '--pa-btn-pad': '3px 10px', '--pa-btn-radius': `${radius.sm}px`,
                          fontSize: textSize.micro, fontFamily: font.body,
                        } as CSSProperties}
                        data-testid={`local-model-delete-${m.id}`}
                      >
                        Delete
                      </Button>
                    )}
                    {m.status.state === 'Downloading' && (
                      <Button
                        colors={colors}
                        onClick={async () => { await api.cancelLocalModelDownload(m.id); await refresh(); return true; }}
                        style={{
                          '--pa-btn-fg': colors.textMuted, '--pa-btn-border': colors.border,
                          '--pa-btn-pad': '3px 10px', '--pa-btn-radius': `${radius.sm}px`,
                          fontSize: textSize.micro, fontFamily: font.body,
                        } as CSSProperties}
                      >
                        Cancel
                      </Button>
                    )}
                  </div>
                }
              />
            );
          })}
        </div>
      )}
      {confirmDeleteId && (
        <ConfirmDialog
          title="Delete this local model?"
          consequence={`Removes the downloaded weights from disk (${models?.find(m => m.id === confirmDeleteId)?.id ?? confirmDeleteId}). Any role routed to it stops working until you download it again.`}
          confirmLabel="Delete"
          failureLabel="Couldn't delete it"
          onConfirm={() => doDelete(confirmDeleteId)}
          onCancel={() => setConfirmDeleteId(null)}
        />
      )}
    </Section>
  );
}
