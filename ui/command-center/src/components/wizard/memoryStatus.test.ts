import { describe, expect, it } from 'vitest';
import { memoryStatus } from './memoryStatus';

describe('memory model readiness', () => {
  const enabled = { enabled: true, model: 'qwen2.5:7b' };

  it('does not confuse the product-default enabled preference with an available model', () => {
    expect(memoryStatus({ schedule: enabled, ollama: { reachable: false, installed: [] } })).toBe('needs-ollama');
    expect(memoryStatus({ schedule: enabled, ollama: { reachable: true, installed: [] } })).toBe('needs-model');
    expect(memoryStatus({ schedule: enabled, ollama: null, ollamaUnavailable: true })).toBe('unavailable');
  });

  it('accepts only the selected installed model and keeps an explicit off choice', () => {
    expect(memoryStatus({ schedule: enabled, ollama: { reachable: true, installed: [{ name: 'other:latest' }] } })).toBe('needs-model');
    expect(memoryStatus({ schedule: enabled, ollama: { reachable: true, installed: [{ name: 'qwen2.5:7b' }] } })).toBe('installed');
    expect(memoryStatus({ schedule: { enabled: true, model: 'qwen3' }, ollama: { reachable: true, installed: [{ name: 'qwen3:latest' }] } })).toBe('installed');
    expect(memoryStatus({ schedule: { enabled: false, model: 'qwen2.5:7b' }, ollama: null, ollamaUnavailable: true })).toBe('off');
  });
});
