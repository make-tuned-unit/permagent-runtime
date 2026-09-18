/**
 * Card registration mechanism (issue #182) — merge/registration unit tests.
 *
 * Pins the contract that manifest cards register alongside first-party code
 * cards, that first-party keys win on collision, and that a manifest becomes a
 * ManifestCard-backed entry carrying its provenance.
 */
import { describe, expect, it } from 'vitest';
import { CARD_REGISTRY, mergeRegistry, manifestToEntry, type CardManifest } from './registry';
import { ManifestCard } from './ManifestCard';

const weatherManifest: CardManifest = {
  type: 'weather',
  name: 'Weather',
  description: 'Local conditions',
  defaultSize: { w: 5, h: 4 },
  layout: 'stat-grid',
  dataEndpoint: '/api/dashboard/weather',
  refreshSeconds: 600,
  source: 'built-in',
};

describe('manifestToEntry', () => {
  it('produces a ManifestCard-backed entry that carries the manifest and source', () => {
    const entry = manifestToEntry(weatherManifest);
    expect(entry.component).toBe(ManifestCard);
    expect(entry.name).toBe('Weather');
    expect(entry.description).toBe('Local conditions');
    expect(entry.defaultSize).toEqual({ w: 5, h: 4 });
    expect(entry.manifest).toBe(weatherManifest);
    expect(entry.source).toBe('built-in');
  });
});

describe('mergeRegistry', () => {
  it('adds manifest cards not present in the base registry', () => {
    const merged = mergeRegistry(CARD_REGISTRY, [weatherManifest]);
    expect(merged.weather).toBeDefined();
    expect(merged.weather.manifest).toBe(weatherManifest);
    // Base cards survive.
    expect(merged.stats).toBe(CARD_REGISTRY.stats);
  });

  it('never lets a manifest override a first-party card of the same type', () => {
    const impostor: CardManifest = { ...weatherManifest, type: 'stats', name: 'Not Stats', source: 'evil-pack' };
    const merged = mergeRegistry(CARD_REGISTRY, [impostor]);
    // Built-in stats wins; the impostor manifest is dropped.
    expect(merged.stats).toBe(CARD_REGISTRY.stats);
    expect(merged.stats.manifest).toBeUndefined();
  });

  it('does not mutate the base registry', () => {
    const before = Object.keys(CARD_REGISTRY).length;
    mergeRegistry(CARD_REGISTRY, [weatherManifest]);
    expect(Object.keys(CARD_REGISTRY).length).toBe(before);
    expect(CARD_REGISTRY.weather).toBeUndefined();
  });

  it('is a no-op passthrough of the base when there are no manifests', () => {
    const merged = mergeRegistry(CARD_REGISTRY, []);
    expect(Object.keys(merged).sort()).toEqual(Object.keys(CARD_REGISTRY).sort());
  });

  it('ships the first-party Council card', () => {
    expect(CARD_REGISTRY.council).toBeDefined();
    expect(CARD_REGISTRY.council.name).toBe('Council');
  });
});

describe('hero card retirement (2026-09-01)', () => {
  it('no longer offers the hero card — replaced by the sidebar status indicator', () => {
    expect(CARD_REGISTRY.hero).toBeUndefined();
    expect(Object.keys(mergeRegistry(CARD_REGISTRY, []))).not.toContain('hero');
  });
});
