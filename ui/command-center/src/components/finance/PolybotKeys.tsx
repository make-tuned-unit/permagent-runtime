import { useCallback, useEffect, useState, type CSSProperties } from 'react';
import { api } from '../../lib/api';
import { font, radius, type } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { ThemeColors } from '../../styles/tokens';

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

type Row = { masked: string; input: string; busy: boolean; error: string };

const blank = (): Row => ({ masked: '', input: '', busy: false, error: '' });

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

  const patch = (key: string, next: Partial<Row>) =>
    setRows((prev) => ({ ...prev, [key]: { ...prev[key], ...next } }));

  const refresh = useCallback(async () => {
    await Promise.all(POLYBOT_SECRET_FIELDS.map(async (f) => {
      try {
        const r = await api.readSecretConfig(f.key);
        patch(f.key, { masked: r?.maskedValue ?? '' });
      } catch {
        patch(f.key, { masked: '' });
      }
    }));
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const save = async (key: string) => {
    const value = rows[key].input.trim();
    if (!value) return;
    patch(key, { busy: true, error: '' });
    try {
      await api.upsertConfig(key, value, true);
      patch(key, { input: '', masked: '' });
      await refresh();
      onChanged?.();
    } catch (e) {
      patch(key, { error: e instanceof Error ? e.message : String(e) });
    } finally {
      patch(key, { busy: false });
    }
  };

  const remove = async (key: string) => {
    patch(key, { busy: true, error: '' });
    try {
      await api.removeConfig(key, true);
      await refresh();
      onChanged?.();
    } catch (e) {
      patch(key, { error: e instanceof Error ? e.message : String(e) });
    } finally {
      patch(key, { busy: false });
    }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: compact ? 8 : 10 }}>
      <p style={{ ...type.caption, color: colors.textMuted, margin: 0, lineHeight: 1.45 }}>
        Stored in the macOS keychain under Permagent. Never written to chat, the
        LaunchAgent plist, or git.
      </p>
      {POLYBOT_SECRET_FIELDS.map((f) => {
        const r = rows[f.key];
        const saved = Boolean(r.masked);
        return (
          <div key={f.key} style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', gap: 8, alignItems: 'baseline' }}>
              <span style={{ ...type.caption, color: colors.text, fontWeight: 600 }}>
                {f.label}
                {f.required ? '' : ' · optional'}
              </span>
              <span style={{ ...type.caption, color: saved ? colors.success : colors.textMuted, fontFamily: font.mono }}>
                {saved ? r.masked : 'not set'}
              </span>
            </div>
            <div style={{ display: 'flex', gap: 6 }}>
              <input
                type="password"
                autoComplete="off"
                spellCheck={false}
                value={r.input}
                onChange={(e) => patch(f.key, { input: e.target.value })}
                placeholder={saved ? 'replace…' : f.hint}
                style={{
                  flex: 1, fontFamily: font.mono, fontSize: 11, color: colors.text,
                  background: colors.inputBg, border: `1px solid ${colors.border}`,
                  borderRadius: radius.sm, padding: '6px 8px', outline: 'none',
                }}
              />
              <button
                type="button"
                disabled={r.busy || !r.input.trim()}
                onClick={() => void save(f.key)}
                style={keyBtn(colors, true)}
              >
                {r.busy ? '…' : 'Save'}
              </button>
              {saved && (
                <button
                  type="button"
                  disabled={r.busy}
                  onClick={() => void remove(f.key)}
                  style={keyBtn(colors, false)}
                >
                  Remove
                </button>
              )}
            </div>
            {r.error && (
              <div style={{ ...type.caption, color: colors.danger }}>{r.error}</div>
            )}
          </div>
        );
      })}
    </div>
  );
}

function keyBtn(colors: ThemeColors, primary: boolean): CSSProperties {
  return {
    fontFamily: font.body,
    fontSize: 11,
    padding: '6px 10px',
    borderRadius: radius.sm,
    border: `1px solid ${primary ? colors.cyan : colors.border}`,
    color: primary ? colors.cyan : colors.textMuted,
    background: 'transparent',
    cursor: 'pointer',
  };
}
