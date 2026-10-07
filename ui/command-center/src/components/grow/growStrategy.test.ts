/**
 * Strategy text with over-escaped quotes (QA 2026-09-24).
 *
 * Acme's value proposition was stored as
 * `Just \"here's what's worth your time.\"` — a literal backslash before each
 * quote, because the model escaped its tool-call string twice — and the card
 * showed the backslashes. The daemon now strips them on save; the read below
 * keeps rows written before that fix readable.
 */

import { describe, expect, it } from 'vitest';
import type { Project } from '../projects/types';
import { readStrategy, unescapeQuotes } from './growStrategy';

const withStrategy = (value: unknown) => ({ metadataJson: { strategy: { value } } }) as unknown as Project;

describe('unescapeQuotes', () => {
  it('collapses a literal backslash-quote to a quote', () => {
    expect(unescapeQuotes('Just \\"here\'s what\'s worth your time.\\"'))
      .toBe('Just "here\'s what\'s worth your time."');
  });

  it('leaves ordinary text alone', () => {
    expect(unescapeQuotes('A "quoted" word and a back\\slash')).toBe('A "quoted" word and a back\\slash');
  });
});

describe('readStrategy', () => {
  it('unescapes content, points and metrics from stored rows', () => {
    const saved = readStrategy(withStrategy({
      content: 'No infinite scrolling. Just \\"here\'s what\'s worth your time.\\"',
      points: [{ label: '\\"Editorial\\" picks', detail: 'Say \\"no\\"' }],
      metrics: [{ label: 'Open rate', value: '\\"40%\\"' }],
    }), 'value');
    expect(saved?.content).toBe('No infinite scrolling. Just "here\'s what\'s worth your time."');
    expect(saved?.content).not.toContain('\\');
    expect(saved?.points?.[0]).toEqual({ label: '"Editorial" picks', detail: 'Say "no"' });
    expect(saved?.metrics?.[0]).toEqual({ label: 'Open rate', value: '"40%"' });
  });
});
