import { useState, useEffect, useRef, useId } from 'react';
import { FiX, FiEye, FiEyeOff } from 'react-icons/fi';
import { api } from '../../lib/api';
import type { SecretSourcesResponse } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import type { ProviderInfo } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import {
  REFERENCE_HINTS,
  SOURCE_KINDS,
  backendBlockedReason,
  buildSpec,
  findKeySource,
  keyStatusMessage,
  kindForKey,
  type SourceKind,
} from './secretSource';

interface Props {
  provider: ProviderInfo;
  onClose: () => void;
  /** Where each key is read from. Fetched once by the parent so the card badge
   *  and this modal cannot disagree. Undefined while it is still loading, or
   *  when the daemon predates the secret-source routes. */
  secretSources?: SecretSourcesResponse;
  /** Ask the parent to re-fetch after a source change. */
  onSourcesChanged?: () => void;
}

export function ConfigureProviderModal({
  provider,
  onClose,
  secretSources,
  onSourcesChanged,
}: Props) {
  const { colors } = useTheme();
  const setDefaultProvider = useCommandCenter(s => s.setDefaultProvider);
  const titleId = useId();

  const secretKey = provider.configKeys.find(k => k.secret);
  const sourceEntry = findKeySource(secretSources?.keys, secretKey?.name ?? '');
  const storedKind = kindForKey(sourceEntry, secretSources?.defaultSource ?? 'keychain');
  // 'file' and 'invalid' are real states the daemon can report but not choices
  // this picker offers; both land on the keychain row so the control always has
  // a selected option, and the status line below says what is actually going on.
  const initialKind: SourceKind =
    storedKind === 'onepassword' || storedKind === 'bitwarden' ? storedKind : 'keychain';

  const [sourceKind, setSourceKind] = useState<SourceKind>(initialKind);
  const [reference, setReference] = useState(sourceEntry?.reference ?? '');
  const [sourceTest, setSourceTest] = useState<{ ok: boolean; message: string } | null>(null);
  const [testingSource, setTestingSource] = useState(false);
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
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);

  const usesManager = sourceKind !== 'keychain';
  // What the daemon currently has, in the same shape buildSpec produces, so
  // "did the user change anything?" is one comparison. `null` means no explicit
  // source, i.e. the default. An `invalid` stored spec keeps its raw text here
  // so switching away from it registers as a change.
  const storedSpec =
    storedKind === 'keychain' || storedKind === 'file' ? null : (sourceEntry?.reference ?? null);
  const storedStatus = keyStatusMessage(sourceEntry);
  const blockedReason = usesManager
    ? backendBlockedReason(secretSources?.backends, sourceKind)
    : null;

  // Keep the latest onClose without re-running the focus-trap effect (the parent
  // passes a fresh closure each render); the trap must set up once on mount.
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  // The parent may still be fetching sources when this mounts. Adopt them when
  // they land — but never over the top of an edit in progress, or the picker
  // would silently snap back while the user was typing a reference.
  const sourceTouched = useRef(false);
  useEffect(() => {
    if (sourceTouched.current || !secretSources) return;
    setSourceKind(initialKind);
    setReference(sourceEntry?.reference ?? '');
  }, [secretSources, initialKind, sourceEntry?.reference]);

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

  // Persist a freshly-entered key: store the secret, then reload so the daemon
  // picks it up without a restart. Reload failure is non-fatal (the key is in the
  // keychain; the next restart applies it). Throws only if the upsert fails.
  const persistKey = async () => {
    if (!(secretKey && apiKey.trim())) return;
    await api.upsertConfig(secretKey.name, apiKey.trim(), true);
    try {
      await api.reloadConfig();
    } catch {
      // Key saved to keychain — reload failed, but not fatal.
    }
  };

  // Persist a changed SOURCE. Separate from persistKey because the two are
  // mutually exclusive by design: a key read from 1Password has no value to
  // type, and the daemon refuses to store one for it (writing a secret nobody
  // reads is the write-side twin of a silent fallback).
  const persistSource = async () => {
    if (!secretKey) return;
    const built = buildSpec(sourceKind, reference);
    if ('error' in built) throw new Error(built.error);
    if (built.spec === storedSpec) return;
    await api.setSecretSource(secretKey.name, built.spec);
    onSourcesChanged?.();
    try {
      // The live provider is still holding whatever it read from the OLD
      // source. Without this the modal would say "1Password" while chat kept
      // using the keychain value — present-looking and wrong.
      await api.reloadConfig();
    } catch {
      // Source saved; the next restart applies it.
    }
  };

  const handleSave = async () => {
    const keyChanged = !!(secretKey && apiKey.trim());
    // An external source IS the credential, so a typed key is neither needed
    // nor accepted here.
    if (!usesManager && !keyChanged && secretKey && !provider.isConfigured) {
      setError('API key is required');
      return;
    }

    setSaving(true);
    setError(null);
    try {
      await persistSource();
      if (!usesManager) await persistKey();
      if (setAsDefault) await setDefaultProvider(provider.name, selectedModel);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to save');
      setSaving(false);
      return;
    }

    setSaving(false);
    onClose();
  };

  // Validate without saving a typed key. Keychain wins over env, so a typed
  // replacement for an already-stored key must persist first (Save & test).
  const typedKey = apiKey.trim();
  const mustPersistToTest = !!(typedKey && provider.isConfigured && !usesManager);
  const handleTest = async () => {
    setTesting(true);
    setError(null);
    setTestResult(null);
    try {
      if (mustPersistToTest) {
        await persistSource();
        await persistKey();
        await api.checkProvider(provider.name);
      } else {
        await api.checkProvider(provider.name, typedKey || undefined);
      }
      setTestResult({ ok: true, message: 'Provider is configured and ready.' });
    } catch (e) {
      setTestResult({ ok: false, message: e instanceof Error ? e.message : 'Connection test failed.' });
    } finally {
      setTesting(false);
    }
  };

  // Read the candidate reference WITHOUT saving it. "Reference saved" only
  // proves a string reached config.yaml; a typo'd vault path looks identical to
  // a working one until the next time chat needs the key.
  const handleTestSource = async () => {
    if (!secretKey) return;
    const built = buildSpec(sourceKind, reference);
    if ('error' in built) {
      setSourceTest({ ok: false, message: built.error });
      return;
    }
    if (built.spec === null) return;
    setTestingSource(true);
    setSourceTest(null);
    try {
      const result = await api.testSecretSource(secretKey.name, built.spec);
      setSourceTest(
        result.ok
          ? { ok: true, message: `Resolved${result.maskedValue ? ` — ${result.maskedValue}` : ''}` }
          : { ok: false, message: result.error || 'Could not read that reference.' },
      );
    } catch (e) {
      setSourceTest({ ok: false, message: e instanceof Error ? e.message : 'Test failed.' });
    } finally {
      setTestingSource(false);
    }
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
              <label htmlFor={`${titleId}-source`} className="block text-xs mb-1.5" style={{ fontFamily: font.body, color: colors.textMuted }}>
                Where this key is read from
              </label>
              <select
                id={`${titleId}-source`}
                value={sourceKind}
                onChange={e => {
                  sourceTouched.current = true;
                  setSourceKind(e.target.value as SourceKind);
                  setSourceTest(null);
                }}
                className="w-full px-3 py-2 rounded text-sm focus:outline-none"
                style={{ backgroundColor: colors.inputBg, border: `1px solid ${colors.border}`, color: colors.text }}
                onFocus={e => { e.currentTarget.style.borderColor = `${colors.cyan}80`; }}
                onBlur={e => { e.currentTarget.style.borderColor = colors.border; }}
              >
                {SOURCE_KINDS.map(s => (
                  <option key={s.kind} value={s.kind}>{s.label}</option>
                ))}
              </select>

              {/* A stored source the daemon cannot read. Shown whether or not
                  the user is mid-edit: "configured but broken" is precisely the
                  state that otherwise surfaces as an unexplained provider
                  error three layers away. */}
              {storedStatus && (
                <div
                  className="text-[11px] mt-1.5 rounded px-2 py-1.5"
                  style={{ color: colors.danger, backgroundColor: `${colors.danger}1A`, border: `1px solid ${colors.danger}33` }}
                >
                  {storedStatus}
                </div>
              )}

              {usesManager && (
                <div className="mt-2 space-y-1.5">
                  {/* Never blocks: a user may be pasting a reference before
                      signing in. It just refuses to imply the manager is ready
                      when it is not. */}
                  {blockedReason && (
                    <div className="text-[11px]" style={{ fontFamily: font.body, color: colors.danger }}>
                      {blockedReason}
                    </div>
                  )}
                  <input
                    id={`${titleId}-reference`}
                    aria-label="Secret reference"
                    type="text"
                    value={reference}
                    onChange={e => { sourceTouched.current = true; setReference(e.target.value); setSourceTest(null); }}
                    placeholder={REFERENCE_HINTS[sourceKind].placeholder}
                    className="provider-modal-input w-full px-3 py-2 rounded text-sm focus:outline-none"
                    style={{ backgroundColor: colors.inputBg, border: `1px solid ${colors.border}`, color: colors.text }}
                    onFocus={e => { e.currentTarget.style.borderColor = `${colors.cyan}80`; }}
                    onBlur={e => { e.currentTarget.style.borderColor = colors.border; }}
                  />
                  <div className="flex items-center gap-2">
                    <button
                      type="button"
                      onClick={handleTestSource}
                      disabled={testingSource}
                      className="text-[11px] px-2 py-1 rounded transition disabled:opacity-50"
                      style={{ border: `1px solid ${colors.cyan}4D`, color: colors.cyan }}
                    >
                      {testingSource ? 'Reading…' : 'Test reference'}
                    </button>
                    <span className="text-[11px]" style={{ fontFamily: font.body, color: colors.textMuted }}>
                      {REFERENCE_HINTS[sourceKind].help}
                    </span>
                  </div>
                  {sourceTest && (
                    <div
                      className="text-[11px] rounded px-2 py-1.5"
                      style={
                        sourceTest.ok
                          ? { color: colors.success, backgroundColor: `${colors.success}1A` }
                          : { color: colors.danger, backgroundColor: `${colors.danger}1A` }
                      }
                    >
                      {sourceTest.message}
                    </div>
                  )}
                </div>
              )}
            </div>
          )}

          {secretKey && !usesManager && (
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
            {/* The model choice only takes effect through setDefaultProvider,
                which runs when "Set as default provider" is checked — with it
                unchecked this select silently did nothing, so it disables and
                says why instead. */}
            <select
              id={`${titleId}-model`}
              ref={secretKey ? undefined : (firstFieldRef as React.RefObject<HTMLSelectElement>)}
              value={selectedModel}
              onChange={e => setSelectedModel(e.target.value)}
              disabled={!setAsDefault || (modelsLoading && models.length === 0)}
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
            {!setAsDefault && (
              <div className="text-[11px] mt-1.5" style={{ fontFamily: font.body, color: colors.textMuted }}>
                Model choice applies only when "Set as default provider" is checked below.
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

          {testResult && (
            <div
              className="text-xs rounded px-3 py-2"
              style={
                testResult.ok
                  ? { color: colors.success, backgroundColor: `${colors.success}1A`, border: `1px solid ${colors.success}33` }
                  : { color: colors.danger, backgroundColor: `${colors.danger}1A`, border: `1px solid ${colors.danger}33` }
              }
            >
              {testResult.message}
            </div>
          )}
        </div>

        <div className="flex items-center justify-between mt-5 pt-4" style={{ borderTop: `1px solid ${colors.border}` }}>
          <div className="flex items-center gap-3">
            <button
              onClick={handleTest}
              disabled={testing || saving}
              className="text-[11px] px-3 py-1.5 rounded transition disabled:opacity-50"
              style={{ border: `1px solid ${colors.cyan}4D`, color: colors.cyan }}
              onMouseEnter={e => { e.currentTarget.style.backgroundColor = colors.cyanSoft; }}
              onMouseLeave={e => { e.currentTarget.style.backgroundColor = ''; }}
            >
              {testing ? 'Testing…' : mustPersistToTest ? 'Save & test' : 'Test connection'}
            </button>
            {/* Hidden while a manager owns the key: there is no stored value to
                remove, and offering the button would imply the reference lives
                in the keychain. */}
            {provider.isConfigured && secretKey && !usesManager && (
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
              disabled={saving || testing}
              className="px-4 py-1.5 text-sm rounded transition disabled:opacity-50"
              style={{ fontFamily: font.display, fontWeight: 600, backgroundColor: colors.cyan, color: colors.textOnCyan }}
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
