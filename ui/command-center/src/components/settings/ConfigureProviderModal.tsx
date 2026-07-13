import { useState, useEffect, useRef, useId } from 'react';
import { FiX, FiEye, FiEyeOff } from 'react-icons/fi';
import { api } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import type { ProviderInfo } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

interface Props {
  provider: ProviderInfo;
  onClose: () => void;
}

export function ConfigureProviderModal({ provider, onClose }: Props) {
  const { colors } = useTheme();
  const setDefaultProvider = useCommandCenter(s => s.setDefaultProvider);
  const titleId = useId();

  const secretKey = provider.configKeys.find(k => k.secret);
  const [apiKey, setApiKey] = useState('');
  const [showKey, setShowKey] = useState(false);
  const [selectedModel, setSelectedModel] = useState(provider.defaultModel);
  const [models, setModels] = useState<string[]>(provider.knownModels);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [modelsError, setModelsError] = useState(false);
  const [reloadModels, setReloadModels] = useState(0);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [setAsDefault, setSetAsDefault] = useState(!provider.isDefault);

  // Keep the latest onClose without re-running the focus-trap effect (the parent
  // passes a fresh closure each render); the trap must set up once on mount.
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  const dialogRef = useRef<HTMLDivElement>(null);
  const firstFieldRef = useRef<HTMLInputElement | HTMLSelectElement | null>(null);

  // Fetch the live model list for configured providers. Handles the rejection
  // (previously an unhandled promise) and drives explicit loading/error state so
  // the select is never silently empty or stuck loading. `knownModels` remains
  // the fallback, so an error still leaves a usable list.
  useEffect(() => {
    if (!provider.isConfigured) return;
    let active = true;
    setModelsLoading(true);
    setModelsError(false);
    api.getProviderModels(provider.name)
      .then(m => { if (active && m.length > 0) setModels(m); })
      .catch(() => { if (active) setModelsError(true); })
      .finally(() => { if (active) setModelsLoading(false); });
    return () => { active = false; };
  }, [provider.name, provider.isConfigured, reloadModels]);

  // Focus management: autofocus the first field on open, trap Tab inside the
  // dialog, close on Escape, and return focus to the trigger on close.
  useEffect(() => {
    const trigger = document.activeElement as HTMLElement | null;
    const node = dialogRef.current;

    const focusable = () =>
      node
        ? Array.from(
            node.querySelectorAll<HTMLElement>(
              'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
            ),
          ).filter(el => el.offsetParent !== null)
        : [];

    const raf = requestAnimationFrame(() => {
      (firstFieldRef.current ?? focusable()[0])?.focus();
    });

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        // Stop the ancestor Settings-view handler from also firing (it dismisses
        // the whole surface); Esc here should only close the modal.
        e.stopPropagation();
        onCloseRef.current();
        return;
      }
      if (e.key !== 'Tab') return;
      const els = focusable();
      if (els.length === 0) return;
      const first = els[0];
      const last = els[els.length - 1];
      const active = document.activeElement as HTMLElement | null;
      if (e.shiftKey) {
        if (active === first || !node?.contains(active)) { e.preventDefault(); last.focus(); }
      } else if (active === last || !node?.contains(active)) {
        e.preventDefault();
        first.focus();
      }
    };

    document.addEventListener('keydown', onKeyDown);
    return () => {
      cancelAnimationFrame(raf);
      document.removeEventListener('keydown', onKeyDown);
      trigger?.focus?.();
    };
  }, []);

  const handleSave = async () => {
    const keyChanged = secretKey && apiKey.trim();

    if (keyChanged) {
      setSaving(true);
      setError(null);
      try {
        await api.upsertConfig(secretKey.name, apiKey.trim(), true);
      } catch (e) {
        setError(e instanceof Error ? e.message : 'Failed to save key');
        setSaving(false);
        return;
      }
    } else if (secretKey && !provider.isConfigured && !apiKey.trim()) {
      setError('API key is required');
      return;
    }

    if (setAsDefault) {
      try {
        await setDefaultProvider(provider.name, selectedModel);
      } catch (e) {
        setError(e instanceof Error ? e.message : 'Failed to set default');
        setSaving(false);
        return;
      }
    }

    // Reload the provider so fresh credentials take effect without daemon restart
    if (keyChanged) {
      try {
        await api.reloadConfig();
      } catch {
        // Key saved to keychain — reload failed, but not fatal.
        // Next daemon restart will pick it up.
      }
    }

    setSaving(false);
    onClose();
  };

  const handleRemoveKey = async () => {
    if (!secretKey) return;
    try {
      await api.removeConfig(secretKey.name, true);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to remove key');
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center backdrop-blur-sm"
      style={{ background: 'rgba(0,0,0,0.5)' }}
      onClick={onClose}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="rounded-xl w-full max-w-md p-5"
        style={{ backgroundColor: colors.surface, border: `1px solid ${colors.border}`, boxShadow: colors.cardShadow }}
        onClick={e => e.stopPropagation()}
      >
        <style>{`.provider-modal-input::placeholder { color: ${colors.textMuted}; opacity: 0.6; }`}</style>
        <div className="flex items-center justify-between mb-4">
          <h2 id={titleId} style={{ fontFamily: font.display, fontWeight: 600, color: colors.text }}>Configure {provider.displayName}</h2>
          <button
            onClick={onClose}
            aria-label="Close"
            className="transition"
            style={{ color: colors.textMuted }}
            onMouseEnter={e => { e.currentTarget.style.color = colors.text; }}
            onMouseLeave={e => { e.currentTarget.style.color = colors.textMuted; }}
            onFocus={e => { e.currentTarget.style.color = colors.text; }}
            onBlur={e => { e.currentTarget.style.color = colors.textMuted; }}
          >
            <FiX size={16} />
          </button>
        </div>

        <div className="space-y-4">
          {secretKey && (
            <div>
              <label htmlFor={`${titleId}-key`} className="block text-xs mb-1.5" style={{ fontFamily: font.body, color: colors.textMuted }}>
                {secretKey.description || secretKey.name}
              </label>
              <div className="relative">
                <input
                  id={`${titleId}-key`}
                  ref={firstFieldRef as React.RefObject<HTMLInputElement>}
                  type={showKey ? 'text' : 'password'}
                  value={apiKey}
                  onChange={e => setApiKey(e.target.value)}
                  placeholder={provider.isConfigured ? '(key stored, enter new to replace)' : 'Enter API key'}
                  className="provider-modal-input w-full px-3 py-2 pr-10 rounded text-sm focus:outline-none"
                  style={{ backgroundColor: colors.inputBg, border: `1px solid ${colors.border}`, color: colors.text }}
                  onFocus={e => { e.currentTarget.style.borderColor = `${colors.cyan}80`; }}
                  onBlur={e => { e.currentTarget.style.borderColor = colors.border; }}
                />
                <button
                  type="button"
                  onClick={() => setShowKey(!showKey)}
                  aria-label={showKey ? 'Hide API key' : 'Show API key'}
                  className="absolute right-2 top-1/2 -translate-y-1/2"
                  style={{ color: colors.textMuted }}
                  onMouseEnter={e => { e.currentTarget.style.color = colors.text; }}
                  onMouseLeave={e => { e.currentTarget.style.color = colors.textMuted; }}
                  onFocus={e => { e.currentTarget.style.color = colors.text; }}
                  onBlur={e => { e.currentTarget.style.color = colors.textMuted; }}
                >
                  {showKey ? <FiEyeOff size={14} /> : <FiEye size={14} />}
                </button>
              </div>
            </div>
          )}

          <div>
            <label htmlFor={`${titleId}-model`} className="block text-xs mb-1.5" style={{ fontFamily: font.body, color: colors.textMuted }}>Model</label>
            <select
              id={`${titleId}-model`}
              ref={secretKey ? undefined : (firstFieldRef as React.RefObject<HTMLSelectElement>)}
              value={selectedModel}
              onChange={e => setSelectedModel(e.target.value)}
              disabled={modelsLoading && models.length === 0}
              className="w-full px-3 py-2 rounded text-sm focus:outline-none disabled:opacity-60"
              style={{ backgroundColor: colors.inputBg, border: `1px solid ${colors.border}`, color: colors.text }}
              onFocus={e => { e.currentTarget.style.borderColor = `${colors.cyan}80`; }}
              onBlur={e => { e.currentTarget.style.borderColor = colors.border; }}
            >
              {models.map(m => (
                <option key={m} value={m}>{m}</option>
              ))}
            </select>
            {modelsLoading && (
              <div className="text-[11px] mt-1.5" style={{ fontFamily: font.body, color: colors.textMuted }}>
                Loading models…
              </div>
            )}
            {!modelsLoading && modelsError && (
              <div className="text-[11px] mt-1.5 flex items-center gap-1.5" style={{ fontFamily: font.body, color: colors.textMuted }}>
                <span>Couldn't refresh the model list{models.length > 0 ? ' — showing known models' : ''}.</span>
                <button
                  type="button"
                  onClick={() => setReloadModels(n => n + 1)}
                  className="transition"
                  style={{ color: colors.cyan }}
                  onMouseEnter={e => { e.currentTarget.style.opacity = '0.8'; }}
                  onMouseLeave={e => { e.currentTarget.style.opacity = '1'; }}
                >
                  Retry
                </button>
              </div>
            )}
            {!modelsLoading && !modelsError && models.length === 0 && (
              <div className="text-[11px] mt-1.5" style={{ fontFamily: font.body, color: colors.textMuted }}>
                No models available. Add a key and save to load this provider's models.
              </div>
            )}
          </div>

          <label className="flex items-center gap-2 text-sm" style={{ fontFamily: font.body, color: colors.textMuted }}>
            <input
              type="checkbox"
              checked={setAsDefault}
              onChange={e => setSetAsDefault(e.target.checked)}
              className="accent-accent"
            />
            Set as default provider
          </label>

          {error && (
            <div
              className="text-xs rounded px-3 py-2"
              style={{ color: colors.danger, backgroundColor: `${colors.danger}1A`, border: `1px solid ${colors.danger}33` }}
            >
              {error}
            </div>
          )}
        </div>

        <div className="flex items-center justify-between mt-5 pt-4" style={{ borderTop: `1px solid ${colors.border}` }}>
          <div>
            {provider.isConfigured && secretKey && (
              <button
                onClick={handleRemoveKey}
                className="text-[11px] transition"
                style={{ color: colors.danger }}
                onMouseEnter={e => { e.currentTarget.style.opacity = '0.8'; }}
                onMouseLeave={e => { e.currentTarget.style.opacity = '1'; }}
                onFocus={e => { e.currentTarget.style.opacity = '0.8'; }}
                onBlur={e => { e.currentTarget.style.opacity = '1'; }}
              >
                Remove key
              </button>
            )}
          </div>
          <div className="flex gap-2">
            <button
              onClick={onClose}
              className="px-4 py-1.5 text-sm rounded hover:bg-white/5 transition"
              style={{ border: `1px solid ${colors.border}`, color: colors.textMuted }}
            >
              Cancel
            </button>
            <button
              onClick={handleSave}
              disabled={saving}
              className="px-4 py-1.5 text-sm rounded transition disabled:opacity-50"
              style={{ fontFamily: font.display, fontWeight: 600, backgroundColor: colors.cyan, color: colors.textOnAccent }}
              onMouseEnter={e => { e.currentTarget.style.backgroundColor = `${colors.cyan}CC`; }}
              onMouseLeave={e => { e.currentTarget.style.backgroundColor = colors.cyan; }}
            >
              {saving ? 'Saving...' : 'Save'}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
