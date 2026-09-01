import { useCallback, useEffect, useState } from 'react';
import { api } from '../../lib/api';
import { font, radius, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { FUNDAMENTALS_KEY } from './financeLabs';

export function FundamentalsKey({
  compact = false,
  onChanged,
}: {
  compact?: boolean;
  onChanged?: () => void;
}) {
  const { colors } = useTheme();
  const [masked, setMasked] = useState('');
  const [input, setInput] = useState('');
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  const refresh = useCallback(async () => {
    try {
      const r = await api.readSecretConfig(FUNDAMENTALS_KEY);
      setMasked(r?.maskedValue ?? '');
    } catch {
      setMasked('');
    }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const save = async () => {
    const value = input.trim();
    if (!value) return;
    setBusy(true);
    setError('');
    try {
      await api.upsertConfig(FUNDAMENTALS_KEY, value, true);
      setInput('');
      setEditing(false);
      await refresh();
      onChanged?.();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    setBusy(true);
    setError('');
    try {
      await api.removeConfig(FUNDAMENTALS_KEY, true);
      setEditing(false);
      await refresh();
      onChanged?.();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const saved = Boolean(masked);

  return (
    <div data-testid="fundamentals-key" style={{ display: 'flex', flexDirection: 'column', gap: compact ? 6 : 8 }}>
      <div style={{ display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
        <span style={{ ...type.caption, color: colors.text, fontWeight: 600 }}>
          financialdatasets.ai
        </span>
        <span style={{ ...type.caption, color: saved ? colors.success : colors.textMuted, fontFamily: font.mono }}>
          {saved ? masked : 'No key — quotes still work'}
        </span>
        <button
          type="button"
          onClick={() => setEditing((v) => !v)}
          style={btn(colors, false)}
        >
          {editing ? 'Cancel' : saved ? 'Replace' : 'Add key'}
        </button>
        {saved && !editing && (
          <button type="button" disabled={busy} onClick={() => void remove()} style={btn(colors, false)}>
            Remove
          </button>
        )}
      </div>
      {editing && (
        <div style={{ display: 'flex', gap: 6 }}>
          <input
            type="password"
            autoComplete="off"
            spellCheck={false}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder="FINANCIAL_DATASETS_API_KEY"
            aria-label="financialdatasets.ai API key"
            style={{
              flex: 1, fontFamily: font.mono, fontSize: 11, color: colors.text,
              background: colors.inputBg, border: `1px solid ${colors.border}`,
              borderRadius: radius.sm, padding: '6px 8px', outline: 'none', minWidth: 0,
            }}
          />
          <button type="button" disabled={busy || !input.trim()} onClick={() => void save()} style={btn(colors, true)}>
            {busy ? '…' : 'Save'}
          </button>
        </div>
      )}
      {error && <div style={{ ...type.caption, color: colors.danger }}>{error}</div>}
    </div>
  );
}

function btn(colors: { cyan: string; border: string; textMuted: string }, primary: boolean) {
  return {
    fontFamily: font.body,
    fontSize: 11,
    padding: '4px 8px',
    borderRadius: radius.sm,
    border: `1px solid ${primary ? colors.cyan : colors.border}`,
    color: primary ? colors.cyan : colors.textMuted,
    background: 'transparent',
    cursor: 'pointer',
    flexShrink: 0,
  };
}
