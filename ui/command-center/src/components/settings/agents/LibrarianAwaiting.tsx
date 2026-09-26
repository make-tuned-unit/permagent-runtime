import { useCallback, useEffect, useState } from 'react';
import { apiFetch } from '../../../lib/api';
import { useTheme } from '../../../styles/useTheme';
import { space, radius, textSize } from '../../../styles/tokens';
import { Button } from '../../common/Button';
import { Chip } from '../../common/Chip';
import { StateBlock } from '../../common/StateBlock';

interface Request { entity_id: string; name: string; entity_type: string; why_thin: string; mention_count: number }

export function LibrarianAwaiting({ onCount }: { onCount: (count: number) => void }) {
  const [rows, setRows] = useState<Request[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(async () => {
    try {
      const data = await apiFetch<Request[]>('/api/librarian/awaiting');
      if (!Array.isArray(data)) throw new Error('Invalid response');
      setRows(data); onCount(data.length); setLoaded(true); setError(null);
      return true;
    } catch { setError('Could not load requests for context.'); return false; }
  }, [onCount]);
  useEffect(() => { void load(); }, [load]);
  if (error) return <StateBlock tone="error" title={error} onRetry={load} compact />;
  if (!loaded) return <div aria-busy="true">Loading requests…</div>;
  if (!rows.length) return <StateBlock tone="empty" title="Nothing waiting on you" compact />;
  return <div style={{ display: 'grid', gap: space.xl }}>{rows.map(row =>
    <ContextAnswer key={row.entity_id} row={row} onSaved={() => {
      const next = rows.filter(r => r.entity_id !== row.entity_id); setRows(next); onCount(next.length);
    }} />)}</div>;
}

function ContextAnswer({ row, onSaved }: { row: Request; onSaved: () => void }) {
  const { colors } = useTheme();
  const [text, setText] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  async function save() {
    if (saving || !text.trim()) return false;
    setSaving(true); setError(null);
    try {
      await apiFetch(`/api/librarian/awaiting/${encodeURIComponent(row.entity_id)}/answer`, {
        method: 'POST', body: JSON.stringify({ text: text.trim() }),
      });
      onSaved(); return true;
    } catch { setError('Could not save your answer. Try again.'); return false; }
    finally { setSaving(false); }
  }
  return <div style={{ display: 'grid', gap: space.md, fontSize: textSize.caption }}>
    <div style={{ display: 'flex', gap: space.md, alignItems: 'center' }}><strong>{row.name}</strong><Chip kind="static">{row.entity_type}</Chip></div>
    <span>{row.why_thin}</span>
    <textarea aria-label={`Context for ${row.name}`} value={text} onChange={e => setText(e.target.value)} maxLength={10000} disabled={saving}
      style={{ padding: space.md, borderRadius: radius.sm, color: colors.text, background: colors.surface, border: `1px solid ${colors.border}`, resize: 'vertical' }} />
    {error && <span role="alert" style={{ color: colors.danger }}>{error}</span>}
    <Button colors={colors} disabled={saving || !text.trim()} onClick={save}>Save answer</Button>
  </div>;
}
