import { useCallback, useEffect, useRef, useState } from 'react';
import { api } from '../../lib/api';
import { font, radius, type, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button, MIN_PENDING_MS, SUCCESS_FLASH_MS } from '../common/Button';
import { FUNDAMENTALS_KEY } from './financeLabs';

import { Tooltip } from '../common/Tooltip';
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
  const [checked, setChecked] = useState(false);
  const [error, setError] = useState('');
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => () => { if (closeTimer.current) clearTimeout(closeTimer.current); }, []);

  const refresh = useCallback(async () => {
    try {
      const r = await api.readSecretConfig(FUNDAMENTALS_KEY);
      setMasked(r?.maskedValue ?? '');
    } catch {
      setMasked('');
    } finally {
      setChecked(true);
    }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const save = async () => {
    const value = input.trim();
    if (!value) return false;
    setError('');
    try {
      await api.upsertConfig(FUNDAMENTALS_KEY, value, true);
      setInput('');
      await refresh();
      onChanged?.();
      // Let the button's success tick land before the editor closes under it.
      if (closeTimer.current) clearTimeout(closeTimer.current);
      closeTimer.current = setTimeout(() => setEditing(false), MIN_PENDING_MS + SUCCESS_FLASH_MS);
      return true;
    } catch (e) {
      setError(`Couldn't save the key — ${detail(e)}`);
      return false;
    }
  };

  const remove = async () => {
    setError('');
    try {
      await api.removeConfig(FUNDAMENTALS_KEY, true);
      setEditing(false);
      await refresh();
      onChanged?.();
      return true;
    } catch (e) {
      setError(`Couldn't remove the key — ${detail(e)}`);
      return false;
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
          {saved ? masked : checked ? 'No key — quotes still work' : 'Checking…'}
        </span>
        <Button
          colors={colors}
          type="button"
          onClick={() => setEditing((v) => !v)}
          style={{ flexShrink: 0 }}
        >
          {editing ? 'Cancel' : saved ? 'Replace' : 'Add key'}
        </Button>
        {saved && !editing && (
          <Button colors={colors} type="button" onClick={() => remove()} style={{ flexShrink: 0 }}>
            Remove
          </Button>
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
              flex: 1, fontFamily: font.mono, fontSize: textSize.micro, color: colors.text,
              background: colors.inputBg, border: `1px solid ${colors.border}`,
              borderRadius: radius.sm, padding: '6px 8px', outline: 'none', minWidth: 0,
            }}
          />
          <Tooltip content={!input.trim() ? 'Enter a key first' : undefined}>
            <span tabIndex={0} style={{ display: 'inline-flex', outline: 'none' }}>
              <Button
                colors={colors}
                variant="ghostOn"
                type="button"
                disabled={!input.trim()}
                onClick={() => save()}
                style={{ flexShrink: 0 }}
              >
                Save
              </Button>
            </span>
          </Tooltip>
        </div>
      )}
      {error && (
        <div role="alert" style={{ ...type.caption, color: colors.danger }}>{error}</div>
      )}
    </div>
  );
}

/** The daemon's own words, kept after the sentence that says what failed. */
function detail(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
