import { useState, useEffect } from 'react';
import { font, textSize } from '../../styles/tokens';
import { useCommandCenter } from '../../lib/store';
import { Mobius } from '../mobius/Mobius';
import { PrimaryButton, GhostLink, Input, Select, Glass, Particles, type SelectOption } from './atoms';
import { api } from '../../lib/api';
import type { SecretBackendStatus } from '../../lib/api';
import { useTheme } from '../../styles/useTheme';
import { REFERENCE_HINTS, buildSpec, suggestedManager, type SourceKind } from '../settings/secretSource';

const PROVIDERS: SelectOption[] = [
  { value: 'anthropic', label: 'Anthropic (Claude)', dot: '#00D5FF' },
  { value: 'openai', label: 'OpenAI', dot: '#10a37f' },
  { value: 'moonshot', label: 'Moonshot (Kimi)', dot: '#A855F7' },
  { value: 'google', label: 'Google (Gemini)', dot: '#4285F4' },
  { value: 'groq', label: 'Groq', dot: '#f55036' },
  { value: 'ollama', label: 'Ollama (Local)', dot: '#8A94A6', note: 'Free' },
];

// Per-provider key-page guidance. The wizard overlays the app, so the page
// opens in the system browser and these numbered steps sit beside the paste
// field — written for someone who has never made an API key before.
const KEY_HELP: Record<string, { url: string; label: string; steps: string[] }> = {
  anthropic: {
    url: 'https://console.anthropic.com/settings/keys',
    label: 'Anthropic Console',
    steps: [
      'Sign in, or create an account (email or Google).',
      'You land on the API Keys page — click "Create Key".',
      'Give it any name (e.g. "Permagent") and click Create.',
      'Click the copy button — the key is only shown once — then come back and paste it.',
    ],
  },
  openai: {
    url: 'https://platform.openai.com/api-keys',
    label: 'OpenAI Platform',
    steps: [
      'Sign in, or create an account.',
      'On the API keys page, click "Create new secret key".',
      'Name it anything (e.g. "Permagent") and click Create.',
      'Copy the key — it is only shown once — then come back and paste it.',
    ],
  },
  moonshot: {
    url: 'https://platform.moonshot.ai/console/api-keys',
    label: 'Moonshot Console',
    steps: [
      'Sign in, or create an account.',
      'Open the API Keys page and click "Create API Key".',
      'Copy the new key, then come back and paste it.',
    ],
  },
  google: {
    url: 'https://aistudio.google.com/apikey',
    label: 'Google AI Studio',
    steps: [
      'Sign in with your Google account.',
      'Click "Create API key" (free — no card needed).',
      'Copy the key, then come back and paste it.',
    ],
  },
  groq: {
    url: 'https://console.groq.com/keys',
    label: 'Groq Console',
    steps: [
      'Sign in, or create a free account.',
      'On the API Keys page, click "Create API Key".',
      'Name it anything, copy the key, then come back and paste it.',
    ],
  },
};

interface Props {
  onAdvance: (provider: string, apiKey: string) => void;
}

export function MomentWelcome({ onAdvance }: Props) {
  const { colors } = useTheme();
  const pushOverlay = useCommandCenter(s => s.pushBrowserOverlay);
  const popOverlay = useCommandCenter(s => s.popBrowserOverlay);
  useEffect(() => { pushOverlay(); return () => { popOverlay(); }; }, [pushOverlay, popOverlay]);

  const [provider, setProvider] = useState('anthropic');
  const [key, setKey] = useState('');
  const [validating, setValidating] = useState(false);
  const [error, setError] = useState('');
  const [showHelp, setShowHelp] = useState(false);
  // Read-back (2026-07 wiring audit): if the wizard re-opens after an
  // interrupted run, a provider key may already be stored — show that instead
  // of a blank field that implies nothing was saved.
  const [configured, setConfigured] = useState<Set<string>>(new Set());
  useEffect(() => {
    api.getProviders()
      .then(ps => {
        const names = ps.filter(p => p.is_configured).map(p => p.name);
        if (names.length === 0) return;
        setConfigured(new Set(names));
        const def = ps.find(p => p.is_configured && p.is_default) ?? ps.find(p => p.is_configured);
        if (def && PROVIDERS.some(o => o.value === def.name)) setProvider(def.name);
      })
      .catch(() => { /* fresh install or daemon still starting — nothing to read back */ });
  }, []);

  // A password manager that is installed AND unlocked right now. Offered, never
  // required: the wizard must not stall because someone has `op` on their PATH.
  // A missing route (older daemon) or any failure simply means no offer.
  const [manager, setManager] = useState<SecretBackendStatus | null>(null);
  const [useReference, setUseReference] = useState(false);
  const [reference, setReference] = useState('');
  useEffect(() => {
    api.getSecretSources()
      .then(r => setManager(suggestedManager(r.backends)))
      .catch(() => { /* no manager offer — the keychain path is unchanged */ });
  }, []);
  const managerKind = manager?.id === 'bitwarden' ? 'bitwarden' : 'onepassword';
  const referenceSpec = useReference
    ? buildSpec(managerKind as SourceKind, reference)
    : null;

  const isLocal = provider === 'ollama';
  const alreadyConfigured = configured.has(provider);
  const canContinue = isLocal
    || alreadyConfigured
    || (useReference ? !!referenceSpec && 'spec' in referenceSpec : key.trim().length > 8);

  const handleSubmit = async () => {
    if (!canContinue) return;
    setValidating(true);
    setError('');
    try {
      // Store the key and set provider
      const providerMeta = await api.getProviders();
      const match = providerMeta.find(p => p.name === provider);
      const secretKey = match?.metadata.config_keys.find(k => k.secret);
      if (secretKey && useReference && referenceSpec && 'spec' in referenceSpec && referenceSpec.spec) {
        // Prove the reference RESOLVES before committing to it. Onboarding is
        // the worst possible place to accept a plausible-looking typo: the user
        // would finish the wizard believing they were set up and hit an
        // unexplained provider error on their first message.
        const probe = await api.testSecretSource(secretKey.name, referenceSpec.spec);
        if (!probe.ok) {
          setError(probe.error || `Couldn't read that reference from ${manager?.displayName}.`);
          setValidating(false);
          return;
        }
        await api.setSecretSource(secretKey.name, referenceSpec.spec);
      } else if (secretKey && key.trim()) {
        await api.upsertConfig(secretKey.name, key.trim(), true);
      }
      const defaultModel = match?.metadata.default_model || '';
      if (defaultModel) {
        await api.setProvider(provider, defaultModel);
      } else {
        await api.upsertConfig('GOOSE_PROVIDER', provider);
      }
      onAdvance(provider, key);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to configure provider');
    } finally {
      setValidating(false);
    }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', height: '100%', position: 'relative' }}>
      <Particles density={24} />
      <Mobius size={160} state="idle" />

      <h1 style={{ fontFamily: font.display, fontSize: 28, fontWeight: 700, color: colors.text, margin: '32px 0 10px', letterSpacing: '-0.02em' }}>
        Welcome to Permagent
      </h1>
      <p style={{ fontFamily: font.body, fontSize: textSize.body, color: colors.textMuted, marginBottom: 32, textAlign: 'center', maxWidth: 380 }}>
        Connect a model provider to power your agent. You can change this later in Settings.
      </p>

      <div style={{ width: 360, display: 'flex', flexDirection: 'column', gap: 14 }}>
        <Select value={provider} onChange={setProvider} options={PROVIDERS} />
        {!isLocal && !useReference && (
          <Input
            value={key}
            onChange={setKey}
            placeholder={alreadyConfigured ? 'Key saved — enter a new one to replace' : 'Paste your API key'}
            type="password"
          />
        )}
        {!isLocal && useReference && (
          <Input
            value={reference}
            onChange={setReference}
            placeholder={REFERENCE_HINTS[managerKind as SourceKind].placeholder}
          />
        )}
        {/* Offered only when a manager is installed AND unlocked. An offer the
            user cannot act on is worse than no offer, and this step must never
            wait on a password manager to proceed. */}
        {!isLocal && manager && (
          <GhostLink onClick={() => { setUseReference(!useReference); setError(''); }} style={{ textAlign: 'center' }}>
            {useReference
              ? 'Paste an API key instead'
              : `${manager.displayName} is unlocked — use a reference instead of pasting a key`}
          </GhostLink>
        )}
        {!isLocal && useReference && (
          <p style={{ fontFamily: font.body, fontSize: textSize.caption, color: colors.textMuted, margin: 0 }}>
            {REFERENCE_HINTS[managerKind as SourceKind].help} The key stays in {manager?.displayName} — nothing is written to the keychain.
          </p>
        )}
        {!isLocal && alreadyConfigured && (
          <p style={{ fontFamily: font.body, fontSize: textSize.caption, color: colors.success, margin: 0 }}>
            ✓ This provider is already connected — continue, or paste a new key to replace it.
          </p>
        )}
        {error && <p style={{ fontFamily: font.body, fontSize: textSize.caption, color: colors.danger, margin: 0 }}>{error}</p>}
        <PrimaryButton onClick={handleSubmit} disabled={!canContinue || validating} full>
          {validating ? 'Saving...' : 'Continue'}
        </PrimaryButton>
        {!isLocal && !showHelp && KEY_HELP[provider] && (
          <GhostLink onClick={() => setShowHelp(true)} style={{ textAlign: 'center' }}>
            I don't have a key — help me get one
          </GhostLink>
        )}
        {!isLocal && showHelp && KEY_HELP[provider] && (
          <Glass r={12} padding={14}>
            <div style={{ fontFamily: font.body, fontSize: textSize.caption, fontWeight: 600, color: colors.text, marginBottom: 8 }}>
              Getting a {KEY_HELP[provider].label} key
            </div>
            <PrimaryButton
              onClick={() => window.open(KEY_HELP[provider].url, '_blank', 'noopener,noreferrer')}
              full
              style={{ height: 36, fontSize: textSize.small, marginBottom: 10 }}
            >
              Open {KEY_HELP[provider].label} ↗
            </PrimaryButton>
            {KEY_HELP[provider].steps.map((s, i) => (
              <div key={i} style={{ display: 'flex', gap: 8, fontFamily: font.body, fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.7 }}>
                <span style={{ color: colors.cyan, fontWeight: 600, flexShrink: 0 }}>{i + 1}.</span>
                <span>{s}</span>
              </div>
            ))}
            <GhostLink onClick={() => setShowHelp(false)} style={{ marginTop: 8, fontSize: textSize.caption }}>Hide steps</GhostLink>
          </Glass>
        )}
      </div>
    </div>
  );
}
