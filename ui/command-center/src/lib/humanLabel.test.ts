import { describe, expect, it } from 'vitest';
import { clampPreview, humanLabel, humanizeId } from './humanLabel';

describe('humanizeId', () => {
  it('splits snake, kebab and colon ids into a sentence', () => {
    expect(humanizeId('config_changed')).toBe('Config changed');
    expect(humanizeId('model_review')).toBe('Model review');
    expect(humanizeId('workspace-snapshot')).toBe('Workspace snapshot');
    expect(humanizeId('activity:terminal_session_started')).toBe('Activity: terminal session started');
  });
  it('splits camelCase and strips an ext__ gateway prefix', () => {
    expect(humanizeId('readWebpage')).toBe('Read webpage');
    expect(humanizeId('extensionmanager__searchMemory')).toBe('Search memory');
  });
  it('keeps acronyms and never returns empty for non-empty input', () => {
    expect(humanizeId('sync_API_keys')).toBe('Sync API keys');
    expect(humanizeId('___')).toBe('___');
  });
});

describe('clampPreview', () => {
  it('collapses whitespace and clamps with an ellipsis', () => {
    expect(clampPreview('a\n  b   c')).toBe('a b c');
    const out = clampPreview('x'.repeat(100), 20);
    expect(out).toHaveLength(20);
    expect(out.endsWith('…')).toBe(true);
  });
});

describe('humanLabel', () => {
  it('humanizes bare ids and keeps the raw one', () => {
    expect(humanLabel('config_changed')).toEqual({ label: 'Config changed', raw: 'config_changed', changed: true });
  });
  it('turns a tool call with an argument dump into a title plus a short preview', () => {
    const { label } = humanLabel('search_memory: query=' + 'family '.repeat(30));
    expect(label.startsWith('Search memory: query=family')).toBe(true);
    expect(label.length).toBeLessThan(80);
    expect(label.endsWith('…')).toBe(true);
  });
  it('leaves prose alone', () => {
    expect(humanLabel('Weekly Storage Report')).toMatchObject({ label: 'Weekly Storage Report', changed: false });
    expect(humanLabel('')).toMatchObject({ label: '', changed: false });
  });
  it('does not touch a plain single word', () => {
    expect(humanLabel('inbox').changed).toBe(false);
  });
});
