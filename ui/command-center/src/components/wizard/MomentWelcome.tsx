import { useState, useEffect } from 'react';
import { font } from '../../styles/tokens';
import { useCommandCenter } from '../../lib/store';
import { Mobius } from '../mobius/Mobius';
import { PrimaryButton, GhostLink, Input, Select, Glass, Particles, type SelectOption } from './atoms';
import { api } from '../../lib/api';
import { useTheme } from '../../styles/useTheme';

const PROVIDERS: SelectOption[] = [
  { value: 'anthropic', label: 'Anthropic (Claude)', dot: '#00D5FF' },
  { value: 'openai', label: 'OpenAI', dot: '#10a37f' },
  { value: 'moonshot', label: 'Moonshot (Kimi)', dot: '#A855F7' },
  { value: 'google', label: 'Google (Gemini)', dot: '#4285F4' },
  { value: 'groq', label: 'Groq', dot: '#f55036' },
  { value: 'ollama', label: 'Ollama (Local)', dot: '#8A94A6', note: 'Free' },
];

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

  const isLocal = provider === 'ollama';
  const canContinue = isLocal || key.trim().length > 8;

  const handleSubmit = async () => {
    if (!canContinue) return;
    setValidating(true);
    setError('');
    try {
      // Store the key and set provider
      const providerMeta = await api.getProviders();
      const match = providerMeta.find(p => p.name === provider);
      const secretKey = match?.metadata.config_keys.find(k => k.secret);
      if (secretKey && key.trim()) {
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

      <h1 style={{ fontFamily: font.display, fontSize: 32, fontWeight: 700, color: colors.text, margin: '32px 0 10px', letterSpacing: '-0.02em' }}>
        Welcome to Permagent
      </h1>
      <p style={{ fontFamily: font.body, fontSize: 14, color: colors.textMuted, marginBottom: 32, textAlign: 'center', maxWidth: 380 }}>
        Connect a model provider to power your agent. You can change this later in Settings.
      </p>

      <div style={{ width: 360, display: 'flex', flexDirection: 'column', gap: 14 }}>
        <Select value={provider} onChange={setProvider} options={PROVIDERS} />
        {!isLocal && (
          <Input value={key} onChange={setKey} placeholder="Paste your API key" type="password" />
        )}
        {error && <p style={{ fontFamily: font.body, fontSize: 12, color: colors.danger, margin: 0 }}>{error}</p>}
        <PrimaryButton onClick={handleSubmit} disabled={!canContinue || validating} full>
          {validating ? 'Connecting...' : 'Continue'}
        </PrimaryButton>
        {!isLocal && (
          <GhostLink onClick={() => setShowHelp(true)} style={{ textAlign: 'center' }}>
            Where do I find my API key?
          </GhostLink>
        )}
      </div>

      {showHelp && (
        <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.6)', display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 50 }}
          onClick={() => setShowHelp(false)}>
          <Glass r={16} padding={28} style={{ maxWidth: 420, width: '90%' }}>
            <h3 style={{ fontFamily: font.display, fontSize: 18, color: colors.text, margin: '0 0 12px' }}>Finding your API key</h3>
            <p style={{ fontFamily: font.body, fontSize: 13, color: colors.textMuted, lineHeight: 1.6 }}>
              Visit your provider's dashboard to create an API key:
            </p>
            <ul style={{ fontFamily: font.mono, fontSize: 12, color: colors.cyan, lineHeight: 2, paddingLeft: 18 }}>
              <li>Anthropic: console.anthropic.com</li>
              <li>OpenAI: platform.openai.com</li>
              <li>Moonshot: platform.moonshot.ai</li>
            </ul>
            <GhostLink onClick={() => setShowHelp(false)} style={{ marginTop: 8 }}>Got it</GhostLink>
          </Glass>
        </div>
      )}
    </div>
  );
}
