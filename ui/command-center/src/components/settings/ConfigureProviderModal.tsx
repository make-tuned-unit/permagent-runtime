import { useState, useEffect } from 'react';
import { FiX, FiEye, FiEyeOff } from 'react-icons/fi';
import { api } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import type { ProviderInfo } from '../../lib/store';
import { useTheme } from '../../styles/useTheme';

interface Props {
  provider: ProviderInfo;
  onClose: () => void;
}

export function ConfigureProviderModal({ provider, onClose }: Props) {
  const { colors } = useTheme();
  const setDefaultProvider = useCommandCenter(s => s.setDefaultProvider);

  const secretKey = provider.configKeys.find(k => k.secret);
  const [apiKey, setApiKey] = useState('');
  const [showKey, setShowKey] = useState(false);
  const [selectedModel, setSelectedModel] = useState(provider.defaultModel);
  const [models, setModels] = useState<string[]>(provider.knownModels);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [setAsDefault, setSetAsDefault] = useState(!provider.isDefault);

  useEffect(() => {
    if (provider.isConfigured) {
      api.getProviderModels(provider.name).then(m => {
        if (m.length > 0) setModels(m);
      });
    }
  }, [provider.name, provider.isConfigured]);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose]);

  const handleSave = async () => {
    if (secretKey && apiKey.trim()) {
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

    setSaving(false);
    onClose();
  };

  const handleRemoveKey = async () => {
    if (!secretKey) return;
    try {
      await api.upsertConfig(secretKey.name, '', true);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to remove key');
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm" onClick={onClose}>
      <div className="rounded-xl border border-dark-border w-full max-w-md p-5 shadow-2xl" style={{ backgroundColor: colors.surface }} onClick={e => e.stopPropagation()}>
        <div className="flex items-center justify-between mb-4">
          <h2 className="font-semibold">Configure {provider.displayName}</h2>
          <button onClick={onClose} className="text-dark-muted hover:text-dark-text transition">
            <FiX size={16} />
          </button>
        </div>

        <div className="space-y-4">
          {secretKey && (
            <div>
              <label className="block text-xs text-dark-muted mb-1.5">
                {secretKey.description || secretKey.name}
              </label>
              <div className="relative">
                <input
                  type={showKey ? 'text' : 'password'}
                  value={apiKey}
                  onChange={e => setApiKey(e.target.value)}
                  placeholder={provider.isConfigured ? '(key stored, enter new to replace)' : 'Enter API key'}
                  className="w-full px-3 py-2 pr-10 rounded border border-dark-border text-sm text-dark-text placeholder:text-dark-muted/40 focus:outline-none focus:border-accent/50"
                  style={{ backgroundColor: colors.inputBg }}
                />
                <button
                  type="button"
                  onClick={() => setShowKey(!showKey)}
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-dark-muted hover:text-dark-text"
                >
                  {showKey ? <FiEyeOff size={14} /> : <FiEye size={14} />}
                </button>
              </div>
            </div>
          )}

          <div>
            <label className="block text-xs text-dark-muted mb-1.5">Model</label>
            <select
              value={selectedModel}
              onChange={e => setSelectedModel(e.target.value)}
              className="w-full px-3 py-2 rounded border border-dark-border text-sm text-dark-text focus:outline-none focus:border-accent/50"
              style={{ backgroundColor: colors.inputBg }}
            >
              {models.map(m => (
                <option key={m} value={m}>{m}</option>
              ))}
            </select>
          </div>

          <label className="flex items-center gap-2 text-sm text-dark-muted">
            <input
              type="checkbox"
              checked={setAsDefault}
              onChange={e => setSetAsDefault(e.target.checked)}
              className="accent-accent"
            />
            Set as default provider
          </label>

          {error && (
            <div className="text-xs text-red-400 bg-red-500/10 border border-red-500/20 rounded px-3 py-2">
              {error}
            </div>
          )}
        </div>

        <div className="flex items-center justify-between mt-5 pt-4 border-t border-dark-border">
          <div>
            {provider.isConfigured && secretKey && (
              <button
                onClick={handleRemoveKey}
                className="text-[11px] text-red-400 hover:text-red-300 transition"
              >
                Remove key
              </button>
            )}
          </div>
          <div className="flex gap-2">
            <button
              onClick={onClose}
              className="px-4 py-1.5 text-sm rounded border border-dark-border text-dark-muted hover:bg-white/5 transition"
            >
              Cancel
            </button>
            <button
              onClick={handleSave}
              disabled={saving}
              className="px-4 py-1.5 text-sm rounded bg-accent font-semibold hover:bg-accent/80 transition disabled:opacity-50"
              style={{ color: colors.textOnAccent }}
            >
              {saving ? 'Saving...' : 'Save'}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
