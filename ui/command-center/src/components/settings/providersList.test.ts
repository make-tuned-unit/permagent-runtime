import { describe, expect, it } from 'vitest';
import { initialProviderTab, partitionProviders } from './providersList';

function p(over: {
  name: string;
  displayName: string;
  isConfigured?: boolean;
  isDefault?: boolean;
}) {
  return {
    name: over.name,
    displayName: over.displayName,
    isConfigured: over.isConfigured ?? false,
    isDefault: over.isDefault ?? false,
  };
}

describe('partitionProviders', () => {
  it('puts configured providers on connected and the rest on providers', () => {
    const { connected, available } = partitionProviders([
      p({ name: 'openai', displayName: 'OpenAI', isConfigured: true }),
      p({ name: 'anthropic', displayName: 'Anthropic' }),
      p({ name: 'zai', displayName: 'Z.AI', isConfigured: true }),
    ]);
    expect(connected.map(x => x.name)).toEqual(['openai', 'zai']);
    expect(available.map(x => x.name)).toEqual(['anthropic']);
  });

  it('pins the default at the top of connected, then sorts by name', () => {
    const { connected } = partitionProviders([
      p({ name: 'zai', displayName: 'Z.AI', isConfigured: true }),
      p({ name: 'anthropic', displayName: 'Anthropic', isConfigured: true, isDefault: true }),
      p({ name: 'openai', displayName: 'OpenAI', isConfigured: true }),
    ]);
    expect(connected.map(x => x.name)).toEqual(['anthropic', 'openai', 'zai']);
  });

  it('sorts the catalogue alphabetically so connecting one does not reshuffle the rest', () => {
    const { available } = partitionProviders([
      p({ name: 'zai', displayName: 'Z.AI' }),
      p({ name: 'anthropic', displayName: 'Anthropic' }),
      p({ name: 'openai', displayName: 'OpenAI' }),
    ]);
    expect(available.map(x => x.displayName)).toEqual(['Anthropic', 'OpenAI', 'Z.AI']);
  });
});

describe('initialProviderTab', () => {
  it('opens on connected once a key exists', () => {
    expect(initialProviderTab(1)).toBe('connected');
    expect(initialProviderTab(3)).toBe('connected');
  });

  it('opens on the catalogue when nothing is connected', () => {
    expect(initialProviderTab(0)).toBe('providers');
  });
});

describe('keyless providers are not "Connected"', () => {
  const keyed = { configKeys: [{ name: 'K', required: true, secret: true }] };
  const keyless = { configKeys: [{ name: 'HOST', required: false, secret: false }] };
  it('puts a configured provider with no secret in ready, not connected', () => {
    const { connected, ready, available } = partitionProviders([
      { ...p({ name: 'openai', displayName: 'OpenAI', isConfigured: true }), ...keyed },
      { ...p({ name: 'lmstudio', displayName: 'LM Studio', isConfigured: true }), ...keyless },
      { ...p({ name: 'ollama', displayName: 'Ollama' }), ...keyless },
    ]);
    expect(connected.map(x => x.name)).toEqual(['openai']);
    expect(ready.map(x => x.name)).toEqual(['lmstudio']);
    expect(available.map(x => x.name)).toEqual(['ollama']);
  });
  it('opens on ready when only key-free providers are set up', () => {
    expect(initialProviderTab(0, 2)).toBe('ready');
    expect(initialProviderTab(1, 2)).toBe('connected');
    expect(initialProviderTab(0, 0)).toBe('providers');
  });
});
