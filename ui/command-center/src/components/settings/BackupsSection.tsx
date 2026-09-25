/**
 * Privacy → Backups (ST4, dagK settings pass).
 *
 * `GET /api/backups` / `POST /api/backups/run` (crates/goose-server/src/routes/backup.rs)
 * had no UI at all. This lists the most recent snapshot of each database and
 * offers a manual "Back up now" — confirmed, since it forces an immediate
 * compacted snapshot rather than waiting for the nightly one.
 */

import { useCallback, useEffect, useState, type CSSProperties } from 'react';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { api, type BackupRunResponse, type SnapshotInfo } from '../../lib/api';
import { Row, Section } from './atoms';
import { Button } from '../common/Button';
import { ConfirmDialog } from '../common/ConfirmDialog';
import { StateBlock } from '../common/StateBlock';
import { timeAgo } from './format';

const DB_LABEL: Record<string, string> = {
  brain: 'Brain (memory)',
  brain_graph: 'Brain graph',
  brain_recognition: 'Brain recognition',
  spectral: 'Sessions & config',
};

function dbLabel(db: string): string {
  return DB_LABEL[db] ?? db;
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

/** The newest snapshot per database, for the summary rows — the full list can
 *  hold one per retention band (daily/weekly/monthly). */
function latestPerDb(snapshots: SnapshotInfo[]): SnapshotInfo[] {
  const byDb = new Map<string, SnapshotInfo>();
  for (const s of snapshots) {
    const cur = byDb.get(s.db);
    if (!cur || s.timestamp > cur.timestamp) byDb.set(s.db, s);
  }
  return [...byDb.values()].sort((a, b) => a.db.localeCompare(b.db));
}

function runResultLine(db: string, result: BackupRunResponse[keyof BackupRunResponse]): string {
  if (result.status === 'ok') return `${dbLabel(db)}: saved ${formatBytes(result.size_bytes)}${result.integrity_ok ? '' : ' (integrity check failed)'}`;
  if (result.status === 'skipped') return `${dbLabel(db)}: skipped — ${result.reason}`;
  return `${dbLabel(db)}: failed — ${result.message}`;
}

export function BackupsSection() {
  const { colors } = useTheme();
  const [snapshots, setSnapshots] = useState<SnapshotInfo[] | null>(null);
  const [error, setError] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [lastRun, setLastRun] = useState<BackupRunResponse | null>(null);

  const refresh = useCallback(() => {
    return api.getBackups()
      .then(r => { setSnapshots(r.snapshots); setError(false); return true; })
      .catch(() => { setError(true); return false; });
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const runBackup = useCallback(async () => {
    const result = await api.runBackups();
    setLastRun(result);
    await refresh();
    setConfirming(false);
  }, [refresh]);

  const rows = snapshots ? latestPerDb(snapshots) : [];

  return (
    <Section
      title="Backups"
      sub="Snapshots of your memory, graph, and session databases, taken automatically and on demand. Stored locally only — nothing here leaves this machine."
    >
      <Row label="Back up now" hint="Forces an immediate snapshot of every database, outside the automatic schedule.">
        <Button
          colors={colors}
          onClick={() => setConfirming(true)}
          style={{
            '--pa-btn-bg': colors.cyanSoft,
            '--pa-btn-fg': colors.cyan,
            '--pa-btn-border': colors.borderHi,
            '--pa-btn-border-hover': colors.cyan,
            '--pa-btn-bg-hover': colors.cyanSoft,
            '--pa-btn-bg-active': colors.cyanGlow,
            '--pa-btn-pad': '0 14px',
            '--pa-btn-radius': `${radius.sm}px`,
            '--pa-btn-weight': 600,
            height: 30, fontSize: textSize.caption, fontFamily: font.body,
          } as CSSProperties}
        >
          Back up now
        </Button>
      </Row>
      {confirming && (
        <ConfirmDialog
          title="Back up all databases now?"
          consequence="Takes a fresh, compacted snapshot of the memory, graph, recognition, and session databases right away. This can take a few seconds and use disk I/O; it does not touch or replace any existing snapshot."
          confirmLabel="Back up now"
          failureLabel="Backup failed"
          onConfirm={runBackup}
          onCancel={() => setConfirming(false)}
        />
      )}
      {lastRun && (
        <div style={{ fontSize: textSize.caption, color: colors.textMuted, padding: `${space.sm}px 0`, lineHeight: 1.6 }}>
          {(Object.entries(lastRun) as [string, BackupRunResponse[keyof BackupRunResponse]][]).map(([db, result]) => (
            <div key={db} style={{ color: result.status === 'error' ? colors.danger : colors.textMuted }}>
              {runResultLine(db, result)}
            </div>
          ))}
        </div>
      )}
      {error ? (
        <StateBlock tone="error" compact title="Couldn't read backup snapshots." onRetry={() => { void refresh(); }} />
      ) : snapshots === null ? (
        <div style={{ fontSize: textSize.caption, color: colors.textDim, padding: '6px 0' }}>Loading snapshots…</div>
      ) : rows.length === 0 ? (
        <div style={{ fontSize: textSize.caption, color: colors.textDim, padding: '6px 0' }}>
          No snapshots yet. Back up now, or wait for the automatic schedule.
        </div>
      ) : (
        rows.map(s => (
          <Row key={s.db} label={dbLabel(s.db)} hint={`${s.tier} · ${formatBytes(s.size_bytes)}${s.integrity_ok ? '' : ' · integrity check failed'}`}>
            <span data-testid={`backup-row-${s.db}`} style={{ fontSize: textSize.caption, color: colors.text }}>
              {timeAgo(s.timestamp) || s.timestamp}
            </span>
          </Row>
        ))
      )}
    </Section>
  );
}
