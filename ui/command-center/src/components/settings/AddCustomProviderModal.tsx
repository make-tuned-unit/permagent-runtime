import { useState, useEffect, useRef, useId } from 'react';
import { FiX, FiEye, FiEyeOff } from 'react-icons/fi';
import { api } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import {
  buildCustomProviderPayload,
  emptyCustomProviderForm,
  ENGINE_OPTIONS,
  type CustomProviderForm,
} from './customProvider';

interface Props {
  onClose: () => void;
}

/** Add a user-defined provider (an OpenAI/Anthropic/Ollama-compatible endpoint
 *  not in the built-in catalog — e.g. a local Ollama/vLLM server or OpenRouter).
 *  Wires POST /config/custom-providers, then runs POST /config/check_provider so
 *  the user learns immediately whether the endpoint is reachable. */
export function AddCustomProviderModal({ onClose }: Props) {
  const { colors } = useTheme();
  const loadProviders = useCommandCenter(s => s.loadProviders);
  const titleId = useId();

  const [f, setF] = useState<CustomProviderForm>(emptyCustomProviderForm);
  const [showKey, setShowKey] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<{ ok: boolean; message: string } | null>(null);
  const [created, setCreated] = useState(false);

  const set = <K extends keyof CustomProviderForm>(key: K, value: CustomProviderForm[K]) =>
    setF(prev => ({ ...prev, [key]: value }));

  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  const dialogRef = useRef<HTMLDivElement>(null);
  const firstFieldRef = useRef<HTMLInputElement>(null);

  // Focus management: mirror ConfigureProviderModal — autofocus the first field,
  // trap Tab inside the dialog, close on Escape, restore focus on unmount.
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

  const handleCreate = async () => {
    const built = buildCustomProviderPayload(f);
    if (!built.ok) {
      setError(built.error);
      setResult(null);
      return;
    }
    setSubmitting(true);
    setError(null);
    setResult(null);
    let providerName: string;
    try {
      const res = await api.createCustomProvider(built.payload);
      providerName = res.provider_name;
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to create provider');
      setSubmitting(false);
      return;
    }

    // The provider now exists — surface it in the list regardless of the check.
    setCreated(true);
    await loadProviders();

    // Immediately validate the new endpoint so a bad URL/key is caught here.
    try {
      await api.checkProvider(providerName);
      setResult({ ok: true, message: `"${built.payload.display_name}" was created and is ready to use.` });
    } catch (e) {
      const reason = e instanceof Error ? e.message : 'connection check failed';
      setResult({
        ok: false,
        message: `Created, but the connection check failed: ${reason}. Fix it via Configure, or remove it.`,
      });
    }
    setSubmitting(false);
  };

  const inputStyle = { backgroundColor: colors.inputBg, border: `1px solid ${colors.border}`, color: colors.text };
  const labelStyle = { fontFamily: font.body, color: colors.textMuted } as const;

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
          <h2 id={titleId} style={{ fontFamily: font.display, fontWeight: 600, color: colors.text }}>Add custom provider</h2>
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

        <p className="text-[11px] mb-4" style={labelStyle}>
          Connect an OpenAI-, Anthropic-, or Ollama-compatible endpoint that isn't in the built-in list — for example a local Ollama/vLLM server or an OpenRouter key.
        </p>

        <fieldset disabled={submitting || created} className="space-y-4 disabled:opacity-60">
          <div>
            <label htmlFor={`${titleId}-name`} className="block text-xs mb-1.5" style={labelStyle}>Display name</label>
            <input
              id={`${titleId}-name`}
              ref={firstFieldRef}
              type="text"
              value={f.displayName}
              onChange={e => set('displayName', e.target.value)}
              placeholder="e.g. Local Llama"
              className="provider-modal-input w-full px-3 py-2 rounded text-sm focus:outline-none"
              style={inputStyle}
              onFocus={e => { e.currentTarget.style.borderColor = `${colors.cyan}80`; }}
              onBlur={e => { e.currentTarget.style.borderColor = colors.border; }}
            />
          </div>

          <div>
            <label htmlFor={`${titleId}-engine`} className="block text-xs mb-1.5" style={labelStyle}>API format</label>
            <select
              id={`${titleId}-engine`}
              value={f.engine}
              onChange={e => set('engine', e.target.value as CustomProviderForm['engine'])}
              className="w-full px-3 py-2 rounded text-sm focus:outline-none"
              style={inputStyle}
              onFocus={e => { e.currentTarget.style.borderColor = `${colors.cyan}80`; }}
              onBlur={e => { e.currentTarget.style.borderColor = colors.border; }}
            >
              {ENGINE_OPTIONS.map(o => (
                <option key={o.value} value={o.value}>{o.label}</option>
              ))}
            </select>
          </div>

          <div>
            <label htmlFor={`${titleId}-url`} className="block text-xs mb-1.5" style={labelStyle}>API URL</label>
            <input
              id={`${titleId}-url`}
              type="text"
              value={f.apiUrl}
              onChange={e => set('apiUrl', e.target.value)}
              placeholder="https://api.example.com/v1"
              className="provider-modal-input w-full px-3 py-2 rounded text-sm focus:outline-none"
              style={inputStyle}
              onFocus={e => { e.currentTarget.style.borderColor = `${colors.cyan}80`; }}
              onBlur={e => { e.currentTarget.style.borderColor = colors.border; }}
            />
          </div>

          <label className="flex items-center gap-2 text-sm" style={labelStyle}>
            <input
              type="checkbox"
              checked={f.requiresAuth}
              onChange={e => set('requiresAuth', e.target.checked)}
              className="accent-accent"
            />
            Requires API key
          </label>

          {f.requiresAuth && (
            <div>
              <label htmlFor={`${titleId}-key`} className="block text-xs mb-1.5" style={labelStyle}>API key</label>
              <div className="relative">
                <input
                  id={`${titleId}-key`}
                  type={showKey ? 'text' : 'password'}
                  value={f.apiKey}
                  onChange={e => set('apiKey', e.target.value)}
                  placeholder="Enter API key"
                  className="provider-modal-input w-full px-3 py-2 pr-10 rounded text-sm focus:outline-none"
                  style={inputStyle}
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
                >
                  {showKey ? <FiEyeOff size={14} /> : <FiEye size={14} />}
                </button>
              </div>
            </div>
          )}

          <div>
            <label htmlFor={`${titleId}-models`} className="block text-xs mb-1.5" style={labelStyle}>Models</label>
            <textarea
              id={`${titleId}-models`}
              value={f.models}
              onChange={e => set('models', e.target.value)}
              placeholder="One id per line, or comma-separated"
              rows={3}
              className="provider-modal-input w-full px-3 py-2 rounded text-sm focus:outline-none resize-y"
              style={inputStyle}
              onFocus={e => { e.currentTarget.style.borderColor = `${colors.cyan}80`; }}
              onBlur={e => { e.currentTarget.style.borderColor = colors.border; }}
            />
          </div>
        </fieldset>

        {error && (
          <div
            className="text-xs rounded px-3 py-2 mt-4"
            style={{ color: colors.danger, backgroundColor: `${colors.danger}1A`, border: `1px solid ${colors.danger}33` }}
          >
            {error}
          </div>
        )}

        {result && (
          <div
            className="text-xs rounded px-3 py-2 mt-4"
            style={
              result.ok
                ? { color: colors.success, backgroundColor: `${colors.success}1A`, border: `1px solid ${colors.success}33` }
                : { color: colors.danger, backgroundColor: `${colors.danger}1A`, border: `1px solid ${colors.danger}33` }
            }
          >
            {result.message}
          </div>
        )}

        <div className="flex items-center justify-end gap-2 mt-5 pt-4" style={{ borderTop: `1px solid ${colors.border}` }}>
          <button
            onClick={onClose}
            className="px-4 py-1.5 text-sm rounded hover:bg-white/5 transition"
            style={{ border: `1px solid ${colors.border}`, color: colors.textMuted }}
          >
            {created ? 'Done' : 'Cancel'}
          </button>
          {!created && (
            <button
              onClick={handleCreate}
              disabled={submitting}
              className="px-4 py-1.5 text-sm rounded transition disabled:opacity-50"
              style={{ fontFamily: font.display, fontWeight: 600, backgroundColor: colors.cyan, color: colors.textOnCyan }}
              onMouseEnter={e => { e.currentTarget.style.backgroundColor = `${colors.cyan}CC`; }}
              onMouseLeave={e => { e.currentTarget.style.backgroundColor = colors.cyan; }}
            >
              {submitting ? 'Creating…' : 'Create & test'}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
