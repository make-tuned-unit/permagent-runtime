import { useCallback, useEffect, useRef, useState } from 'react';
import { api } from '../../lib/api';
import { font, radius, type, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button, MIN_PENDING_MS, SUCCESS_FLASH_MS } from '../common/Button';
import { requiredKeysSet } from './financeLabs';

import { Tooltip } from '../common/Tooltip';
export const POLYBOT_SECRET_FIELDS: Array<{
  key: string;
  label: string;
  required: boolean;
  hint: string;
}> = [
  { key: 'POLYMARKET_API_KEY', label: 'API key', required: true, hint: 'L2 key from Polymarket' },
  { key: 'POLYMARKET_API_SECRET', label: 'API secret', required: true, hint: 'L2 secret' },
  { key: 'POLYMARKET_API_PASSPHRASE', label: 'API passphrase', required: true, hint: 'L2 passphrase' },
  { key: 'POLYMARKET_WALLET_PRIVATE_KEY', label: 'Wallet private key', required: true, hint: 'Signs orders. Keychain only.' },
  { key: 'POLYMARKET_FUNDER_ADDRESS', label: 'Funder address', required: true, hint: 'Proxy / funder wallet' },
  { key: 'ANTHROPIC_API_KEY', label: 'Anthropic', required: false, hint: 'Optional — research strategies' },
];

type Row = { masked: string; input: string; error: string; checked: boolean };

const blank = (): Row => ({ masked: '', input: '', error: '', checked: false });

export function PolybotKeys({
  compact = false,
  onChanged,
}: {
  compact?: boolean;
  onChanged?: () => void;
}) {
  const { colors } = useTheme();
  const [rows, setRows] = useState<Record<string, Row>>(() =>
    Object.fromEntries(POLYBOT_SECRET_FIELDS.map((f) => [f.key, blank()])),
  );
  const [editing, setEditing] = useState<string | null>(null);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => () => { if (closeTimer.current) clearTimeout(closeTimer.current); }, []);

  const patch = (key: string, next: Partial<Row>) =>
    setRows((prev) => ({ ...prev, [key]: { ...prev[key], ...next } }));

  const refresh = useCallback(async () => {
    await Promise.all(POLYBOT_SECRET_FIELDS.map(async (f) => {
      try {
        const r = await api.readSecretConfig(f.key);
        patch(f.key, { masked: r?.maskedValue ?? '', checked: true });
      } catch {
        patch(f.key, { masked: '', checked: true });
      }
    }));
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const save = async (key: string) => {
    const value = rows[key].input.trim();
    if (!value) return false;
    patch(key, { error: '' });
    try {
      await api.upsertConfig(key, value, true);
      patch(key, { input: '' });
      await refresh();
      onChanged?.();
      // Hold the editor open long enough for the button's own success tick to
      // land under the pointer; closing on resolve would swallow it.
      if (closeTimer.current) clearTimeout(closeTimer.current);
      closeTimer.current = setTimeout(() => {
        setEditing((cur) => (cur === key ? null : cur));
      }, MIN_PENDING_MS + SUCCESS_FLASH_MS);
      return true;
    } catch (e) {
      patch(key, { error: `Couldn't save this key — ${detail(e)}` });
      return false;
    }
  };

  const remove = async (key: string) => {
    patch(key, { error: '' });
    try {
      await api.removeConfig(key, true);
      setEditing(null);
      await refresh();
      onChanged?.();
      // The row flipping to "not set" — and Remove itself going away — is this
      // action's acknowledgment; there is no control left to tick.
      return true;
    } catch (e) {
      patch(key, { error: `Couldn't remove this key — ${detail(e)}` });
      return false;
    }
  };

  const maskedMap = Object.fromEntries(
    POLYBOT_SECRET_FIELDS.map((f) => [f.key, rows[f.key]?.masked ?? '']),
  );
  const { have, need } = requiredKeysSet(POLYBOT_SECRET_FIELDS, maskedMap);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: compact ? 6 : 8 }} data-testid="polybot-keys">
      <p style={{ ...type.caption, color: colors.textMuted, margin: 0, lineHeight: 1.45 }}>
        {have} of {need} required keys in the keychain. Never written to chat, the
        LaunchAgent plist, or git.
      </p>
      {POLYBOT_SECRET_FIELDS.map((f) => {
        const r = rows[f.key];
        const saved = Boolean(r.masked);
        const open = editing === f.key;
        return (
          <div key={f.key} data-testid="polybot-key-row" style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <div style={{ display: 'flex', gap: 8, alignItems: 'center', minHeight: 22 }}>
              <span style={{ ...type.caption, color: colors.text, fontWeight: 600, flex: '0 1 140px' }}>
                {f.label}
                {f.required ? '' : ' · optional'}
              </span>
              <span
                style={{
                  ...type.caption,
                  color: saved ? colors.success : colors.textMuted,
                  fontFamily: font.mono,
                  flex: 1,
                  minWidth: 0,
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                }}
              >
                {saved ? r.masked : r.checked ? 'not set' : 'Checking…'}
              </span>
              <Button
                colors={colors}
                type="button"
                onClick={() => setEditing(open ? null : f.key)}
                style={{ flexShrink: 0 }}
              >
                {open ? 'Cancel' : saved ? 'Replace' : 'Add'}
              </Button>
              {saved && !open && (
                <Button
                  colors={colors}
                  type="button"
                  onClick={() => remove(f.key)}
                  style={{ flexShrink: 0 }}
                >
                  Remove
                </Button>
              )}
            </div>
            {open && (
              <div data-testid="polybot-key-editor" style={{ display: 'flex', gap: 6 }}>
                <input
                  type="password"
                  autoComplete="off"
                  spellCheck={false}
                  value={r.input}
                  onChange={(e) => patch(f.key, { input: e.target.value })}
                  placeholder={f.hint}
                  style={{
                    flex: 1, fontFamily: font.mono, fontSize: textSize.micro, color: colors.text,
                    background: colors.inputBg, border: `1px solid ${colors.border}`,
                    borderRadius: radius.sm, padding: '6px 8px', outline: 'none',
                  }}
                />
                <Tooltip content={!r.input.trim() ? 'Enter a value first' : undefined}>
                  <span tabIndex={0} style={{ display: 'inline-flex', outline: 'none' }}>
                    <Button
                      colors={colors}
                      variant="ghostOn"
                      type="button"
                      disabled={!r.input.trim()}
                      onClick={() => save(f.key)}
                      style={{ flexShrink: 0 }}
                    >
                      Save
                    </Button>
                  </span>
                </Tooltip>
              </div>
            )}
            {r.error && (
              <div role="alert" style={{ ...type.caption, color: colors.danger }}>{r.error}</div>
            )}
          </div>
        );
      })}
    </div>
  );
}

/** The daemon's own words, kept after the sentence that says what failed. */
function detail(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
