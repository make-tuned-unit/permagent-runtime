import { beforeEach, describe, expect, it, vi } from 'vitest';

const { getConfig, getProviders, getSession } = vi.hoisted(() => ({
  getConfig: vi.fn(), getProviders: vi.fn(), getSession: vi.fn(),
}));
vi.mock('./api', () => ({
  api: { getConfig, getProviders, getSession },
  apiFetch: vi.fn(), extractText: vi.fn(() => ''),
  extractSystemNotice: vi.fn(() => null), extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(), readerIngest: vi.fn(),
}));

import { useCommandCenter } from './store';

beforeEach(() => {
  getConfig.mockReset(); getProviders.mockReset(); getSession.mockReset();
  getProviders.mockResolvedValue([{
    name: 'local', is_configured: true, provider_type: 'Builtin',
    metadata: { display_name: 'Local', description: '', default_model: '', known_models: [], config_keys: [] },
  }]);
  useCommandCenter.setState({
    providers: [], providersError: false, currentModel: null, chatResolvedRoute: null,
    chatSessionId: null, chatSessionRouteEvidence: false, chatMessages: [],
    connectSession: vi.fn(), disconnectSession: vi.fn(),
  });
});

describe('Chat route evidence', () => {
  it('stores the daemon-resolved route independently of the raw saved model', async () => {
    const route = { provider: 'openai', model: 'gpt-5', source: 'session_model' };
    getConfig.mockResolvedValue({ config: { GOOSE_MODEL: 'stale-model' }, resolved_routes: { chat: route } });
    await useCommandCenter.getState().loadProviders();
    expect(useCommandCenter.getState().chatResolvedRoute).toEqual(route);
    expect(useCommandCenter.getState().providers[0]).toMatchObject({ name: 'local', isConfigured: true });
  });

  it('leaves a missing projection unknown and clears stale route evidence on failed refresh', async () => {
    getConfig.mockResolvedValueOnce({ config: {} });
    await useCommandCenter.getState().loadProviders();
    expect(useCommandCenter.getState().chatResolvedRoute).toBeNull();
    useCommandCenter.setState({ chatResolvedRoute: { provider: 'openai', model: 'gpt-5', source: 'default' } });
    getConfig.mockRejectedValueOnce(new Error('offline'));
    await useCommandCenter.getState().loadProviders();
    expect(useCommandCenter.getState()).toMatchObject({ providersError: true, chatResolvedRoute: null });
  });

  it('uses saved session provider and model as uncertain evidence and resets it on switch', async () => {
    useCommandCenter.setState({ chatSessionId: 'saved' });
    getSession.mockResolvedValueOnce({ id: 'saved', provider_name: 'anthropic', model_config: { model_name: 'claude' }, conversation: [] });
    await useCommandCenter.getState().loadSessionMessages('saved');
    expect(useCommandCenter.getState().chatSessionRouteEvidence).toBe(true);

    getSession.mockResolvedValueOnce({ id: 'new', conversation: [] });
    await useCommandCenter.getState().switchToSession('new');
    expect(useCommandCenter.getState()).toMatchObject({ chatSessionId: 'new', chatSessionRouteEvidence: false });
  });
});
