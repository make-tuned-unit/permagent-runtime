/** @vitest-environment jsdom */
import { createRoot } from 'react-dom/client';
import { act } from 'react';
import { ToneMappingMode } from 'postprocessing';
import { describe, expect, it, vi } from 'vitest';

// jsdom has no WebGL, so the real composer cannot run here. Each effect is
// replaced by a plain element that records its name and props, which pins the
// ORDER of the chain and the tone-mapping mode — the thing that was wrong when
// the white block appeared over the night sky on 2026-09-18: the chain had no
// view transform at all, because the Canvas's AgX is skipped while an
// EffectComposer is mounted. This does not prove any pixel value; the probe
// (`scripts/probe-black-rect.mjs`) and the A/B table in the source do that.
vi.mock('@react-three/postprocessing', () => {
  const effect = (name: string) => (props: Record<string, unknown>) => (
    <i data-effect={name} data-props={JSON.stringify(props)} />
  );
  return {
    EffectComposer: ({ children, multisampling }: { children: React.ReactNode; multisampling?: number }) => (
      <div data-composer data-multisampling={String(multisampling)}>{children}</div>
    ),
    Bloom: effect('Bloom'),
    ToneMapping: effect('ToneMapping'),
    HueSaturation: effect('HueSaturation'),
    BrightnessContrast: effect('BrightnessContrast'),
    FXAA: effect('FXAA'),
  };
});

import { ForumPostProcessing } from './ForumPostProcessing';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
function render(node: React.ReactElement) {
  const container = document.createElement('div');
  const root = createRoot(container);
  act(() => root.render(node));
  return { container };
}

function chain() {
  const { container } = render(<ForumPostProcessing />);
  return Array.from(container.querySelectorAll<HTMLElement>('[data-effect]')).map((el) => ({
    name: el.dataset.effect,
    props: JSON.parse(el.dataset.props || '{}') as Record<string, unknown>,
  }));
}

describe('ForumPostProcessing', () => {
  it('applies AgX inside the composer, after Bloom and before the grade', () => {
    const effects = chain();
    const names = effects.map((e) => e.name);
    expect(names).toEqual(['Bloom', 'ToneMapping', 'HueSaturation', 'BrightnessContrast', 'FXAA']);
    expect(effects[1].props.mode).toBe(ToneMappingMode.AGX);
  });

  it('keeps the composer single-sampled (the ANGLE/Metal black-rectangle fix)', () => {
    const { container } = render(<ForumPostProcessing />);
    expect(container.querySelector('[data-composer]')?.getAttribute('data-multisampling')).toBe('0');
  });
});
