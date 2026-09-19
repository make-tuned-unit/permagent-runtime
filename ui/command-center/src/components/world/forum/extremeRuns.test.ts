import { describe, expect, it } from 'vitest';
import { extremeReport, type DecodedImage } from './extremeRuns';

// Synthetic frames, not renders: this pins the detector's RULE (a solid block
// of identical extreme pixels at least 200 CSS px wide), which is what the
// rectangle probes report on. It proves nothing about whether any renderer
// draws the rectangle — only that the instrument counts one when it is there
// and does not count the night sky as one.
function frame(width: number, height: number, fill: [number, number, number]): DecodedImage {
  const channels = 3;
  const pixels = new Uint8Array(width * height * channels);
  for (let i = 0; i < width * height; i++) pixels.set(fill, i * channels);
  return { width, height, channels, stride: width * channels, pixels };
}
function paint(img: DecodedImage, x0: number, y0: number, w: number, h: number, rgb: [number, number, number]) {
  for (let y = y0; y < y0 + h; y++) for (let x = x0; x < x0 + w; x++) img.pixels.set(rgb, y * img.stride + x * img.channels);
}

describe('extremeReport', () => {
  it('finds a solid white block and reports its exact box', () => {
    const img = frame(600, 400, [20, 30, 60]);
    paint(img, 100, 50, 250, 120, [255, 255, 255]);
    const report = extremeReport(img);
    expect(report.white).toEqual({ count: 250 * 120, box: [100, 50, 349, 169] });
    expect(report.black.count).toBe(0);
  });

  it('ignores white runs narrower than a block (glyphs, button fills)', () => {
    const img = frame(600, 400, [20, 30, 60]);
    paint(img, 100, 50, 199, 120, [255, 255, 255]);
    expect(extremeReport(img).white.count).toBe(0);
  });

  it('scales the run with the device pixel ratio, so a DPR-2 night sky is not a black rectangle', () => {
    // At DPR 2 the pure-black gap between stars measured 28,322 "black
    // rectangle" pixels on a clean night frame, because 200 device pixels is
    // only 100 CSS pixels of sky. 300 device px at DPR 2 is 150 CSS px: sky.
    const img = frame(1200, 400, [20, 30, 60]);
    paint(img, 0, 0, 300, 80, [0, 0, 0]);
    expect(extremeReport(img, 2).black.count).toBe(0);
    // ...while a real 250 CSS px block at the same DPR still counts.
    paint(img, 600, 200, 500, 60, [255, 255, 255]);
    expect(extremeReport(img, 2).white.box).toEqual([600, 200, 1099, 259]);
  });
});

// The clipping proxy the A/B table in ForumPostProcessing.tsx is counted in.
// Synthetic frames again: this pins what "white", "any channel" and "largest
// blob" mean, not what any renderer produced.
import { clipReport } from './extremeRuns';

describe('clipReport', () => {
  it('counts pure white, any-channel clipping, and the largest white blob', () => {
    const img = frame(300, 200, [10, 20, 40]);
    paint(img, 10, 10, 40, 30, [255, 255, 255]); // the blob: 1,200 px
    paint(img, 100, 100, 5, 5, [255, 255, 255]); // a glyph: 25 px
    paint(img, 200, 50, 10, 10, [255, 120, 0]); // clipped hue, not white: 100 px
    const r = clipReport(img);
    expect(r.white).toBe(1200 + 25);
    expect(r.anyChannel).toBe(1200 + 25 + 100);
    expect(r.largestWhite).toEqual({ count: 1200, box: [10, 10, 49, 39] });
  });

  it('only counts inside the region it is given (the canvas, not the Agent Desk)', () => {
    const img = frame(300, 200, [10, 20, 40]);
    paint(img, 250, 0, 50, 200, [255, 255, 255]);
    expect(clipReport(img, [0, 0, 250, 200]).white).toBe(0);
    expect(clipReport(img).largestWhite.count).toBe(50 * 200);
  });
});

// The shipped-bytes check `probe-black-rect.mjs --shipped` runs against the
// chunk Permagent.app embeds. The fixture is copied verbatim from the built
// `dist/assets/ForumAppView-BAoLuAgn.js` of 2026-09-18 23:48, the chunk the
// installed app was packaged with; the "before" fixture is the same text with
// the AgX element taken out, which is what the chain as committed (no view
// transform) builds to. This proves the reader recognises the chain in
// minified output. It proves nothing about pixels.
import { composerChains, embeddedAssetNames, isShippedForumChain } from './extremeRuns';

const BUILT_WITH_AGX = 'const Ho=.5,$o=.075,Xo=-.035;function Yo(){return f.jsxs(ni,{multisampling:0,children:[f.jsx(si,{mipmapBlur:!0,intensity:.55,radius:.6,levels:5,luminanceThreshold:.72,luminanceSmoothing:.3}),f.jsx(jr,{mode:ii.AGX}),f.jsx(kr,{saturation:Ho}),f.jsx(wr,{brightness:Xo,contrast:$o})]})}function Qo({onOpen:e}){const t=[1,2];}';
const BUILT_WITHOUT_AGX = BUILT_WITH_AGX.replace('f.jsx(jr,{mode:ii.AGX}),', '');

describe('composerChains', () => {
  it('reads Bloom > AgX > grade, single-sampled, out of the built chunk', () => {
    const chains = composerChains(BUILT_WITH_AGX);
    expect(chains).toEqual([{ multisampling: '0', effects: ['Bloom', 'ToneMapping', 'HueSaturation', 'BrightnessContrast'], agx: true }]);
    expect(isShippedForumChain(chains[0])).toBe(true);
  });

  it('rejects the chain that clipped the gas giant white (no view transform)', () => {
    const [chain] = composerChains(BUILT_WITHOUT_AGX);
    expect(chain.agx).toBe(false);
    expect(isShippedForumChain(chain)).toBe(false);
  });

  it('rejects AgX after the grade, a non-AgX mode, and a multisampled composer', () => {
    const graded = BUILT_WITHOUT_AGX.replace('contrast:$o})', 'contrast:$o}),f.jsx(jr,{mode:ii.AGX})');
    expect(composerChains(graded)[0].effects).toEqual(['Bloom', 'HueSaturation', 'BrightnessContrast', 'ToneMapping']);
    expect(isShippedForumChain(composerChains(graded)[0])).toBe(false);
    expect(composerChains(BUILT_WITH_AGX.replace('ii.AGX', 'ii.ACES_FILMIC'))[0].agx).toBe(false);
    expect(isShippedForumChain(composerChains(BUILT_WITH_AGX.replace('multisampling:0', 'multisampling:4'))[0])).toBe(false);
  });

  it('accepts AgX inlined as its enum number', () => {
    expect(composerChains(BUILT_WITH_AGX.replace('ii.AGX', '7'))[0].agx).toBe(true);
  });

  it('finds nothing in a chunk with no composer', () => {
    expect(composerChains('function a(){return f.jsx(b,{children:[1]})}')).toEqual([]);
  });
});

describe('embeddedAssetNames', () => {
  it('lists each embedded js asset once, out of binary text', () => {
    const nul = String.fromCharCode(0);
    const binary = `${nul}/assets/ForumAppView-BAoLuAgn.js${nul}junk assets/index-BVCoupjq.js${nul}/assets/ForumAppView-BAoLuAgn.js assets/x.css`;
    expect(embeddedAssetNames(binary)).toEqual(['assets/ForumAppView-BAoLuAgn.js', 'assets/index-BVCoupjq.js']);
  });
});
