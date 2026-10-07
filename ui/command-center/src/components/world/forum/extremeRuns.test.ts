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
    expect(report.white).toEqual({ count: 250 * 120, box: [100, 50, 349, 169], fill: 1, solid: true, blocks: 1, extent: [100, 50, 349, 169] });
    expect(report.solid).toBe(true);
    expect(report.black.count).toBe(0);
  });

  it('does not call a crushed dark texture a rectangle: its runs are long but never stack into a block', () => {
    // The shape the --app-state night run found on 2026-09-19: the far tower
    // facade graded to exact 0 in long runs broken by lit window rows, so
    // the runs span a wide box and fill little of it.
    const img = frame(1200, 400, [20, 30, 60]);
    for (let y = 100; y < 260; y += 8) paint(img, 100 + (y % 3) * 40, y, 420, 1, [0, 0, 0]);
    const report = extremeReport(img);
    expect(report.black.count).toBeGreaterThan(0);
    // Every run is its own 1-row block: full, but nowhere near block height.
    expect(report.black.blocks).toBe(20);
    expect(report.black.solid).toBe(false);
    expect(report.solid).toBe(false);
  });

  it('still calls the job-19 driver rectangle solid', () => {
    const img = frame(1200, 400, [20, 30, 60]);
    paint(img, 437, 0, 340, 251, [0, 0, 0]);
    expect(extremeReport(img).black).toMatchObject({ count: 340 * 251, fill: 1, solid: true });
  });

  it('still finds the job-19 rectangle when the crushed facade is in the same frame (the user layout)', () => {
    // The reviewer's reproduction of 2026-09-19: with the desk and rail closed
    // the facade crushes to 0 on every night frame, 1-px runs at y 664..824.
    // Measured over ONE box around every run, that texture diluted the real
    // rectangle to fill 0.221 and solid:false — the probe would have reported
    // framesWithArtifact 0 and saved no frame with the defect on screen.
    const img = frame(1512, 945, [20, 30, 60]);
    paint(img, 437, 0, 340, 251, [0, 0, 0]);
    for (let y = 664; y <= 824; y += 8) paint(img, 257 + (y % 3) * 40, y, 420, 1, [0, 0, 0]);
    const report = extremeReport(img);
    expect(report.black.solid).toBe(true);
    expect(report.black.box).toEqual([437, 0, 776, 250]);
    expect(report.black.fill).toBe(1);
    expect(report.black.blocks).toBeGreaterThan(1);
    expect(report.black.extent![3]).toBe(824); // the texture is still counted, just not as the shape
    expect(report.solid).toBe(true);
    // ...and at DPR 2, where the real frames are captured.
    const hi = frame(3024, 1890, [20, 30, 60]);
    paint(hi, 874, 0, 680, 502, [0, 0, 0]);
    for (let y = 1328; y <= 1648; y += 16) paint(hi, 514 + (y % 3) * 80, y, 840, 2, [0, 0, 0]);
    expect(extremeReport(hi, 2).black).toMatchObject({ solid: true, box: [874, 0, 1553, 501] });
  });

  it('does not call short dense clusters of crushed rows a block (the real facade: 7 rows at fill 0.86)', () => {
    // Per-block fill alone cannot reject the facade: at DPR 2 it forms
    // clusters of up to 7 touching rows that fill 86% of their own box.
    // What they are not is tall.
    const img = frame(3024, 1890, [20, 30, 60]);
    for (let y = 1328; y < 1648; y += 20) paint(img, 514, y, 1028, 7, [0, 0, 0]);
    const report = extremeReport(img, 2);
    expect(report.black.count).toBe(16 * 1028 * 7);
    expect(report.black.fill).toBe(1);
    expect(report.black.solid).toBe(false);
    // A lone run fills its own 1-row box: it is not a rectangle either.
    const one = frame(1200, 400, [20, 30, 60]);
    paint(one, 100, 100, 600, 1, [255, 255, 255]);
    expect(extremeReport(one).white).toMatchObject({ count: 600, fill: 1, solid: false });
  });

  it('keeps separate blocks apart even when they share rows', () => {
    // Two solid blocks side by side must not merge into one box that includes
    // the gap between them: that box would describe a shape that is not there.
    const img = frame(1200, 400, [20, 30, 60]);
    paint(img, 0, 0, 300, 100, [0, 0, 0]);
    paint(img, 700, 50, 400, 200, [0, 0, 0]);
    const r = extremeReport(img).black;
    expect(r.blocks).toBe(2);
    expect(r.box).toEqual([700, 50, 1099, 249]);
    expect(r.extent).toEqual([0, 0, 1099, 249]);
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

import { forumStorageFromRows, parseElapsed, runningAppProcesses } from './extremeRuns';

describe('runningAppProcesses', () => {
  const BIN = '/Applications/Permagent.app/Contents/MacOS/permagent-app';
  const written = Date.parse('2026-09-19T06:07:10Z');

  it('parses every etime shape ps prints', () => {
    expect(parseElapsed('05:28')).toBe(328);
    expect(parseElapsed('01:55:28')).toBe(6928);
    expect(parseElapsed('2-01:55:28')).toBe(2 * 86400 + 6928);
    expect(parseElapsed('soon')).toBeNaN();
  });

  it('flags an app launched before its binary was replaced: it still shows the old build', () => {
    const now = Date.parse('2026-09-19T07:03:00Z');
    const ps = [
      ` 7001  2-00:00:00 ${BIN}`,
      `76299       55:28 ${BIN}`,
      `76118       55:41 /Applications/Permagent.app/Contents/MacOS/permagentd`,
    ].join('\n');
    expect(runningAppProcesses(ps, BIN, written, now)).toEqual([
      { pid: 7001, startedAt: '2026-09-17T07:03:00.000Z', stale: true },
      { pid: 76299, startedAt: '2026-09-19T06:07:32.000Z', stale: false },
    ]);
  });
});

describe('forumStorageFromRows', () => {
  const utf16 = (s: string) => [...s].map((c) => c.charCodeAt(0).toString(16).padStart(2, '0').toUpperCase() + '00').join('');

  it('decodes the World keys out of a WebKit localStorage dump and drops everything else', () => {
    const rows = [
      `permagent.forum.desk|${utf16('closed')}`,
      `permagent.forum.dismissed|${utf16('["passport"]')}`,
      `permagent-theme|${utf16('system')}`,
      `chat.draft|${utf16('private words')}`,
      'permagent.forum.rail|not-hex',
    ].join('\n');
    expect(forumStorageFromRows(rows)).toEqual({ 'permagent.forum.desk': 'closed', 'permagent.forum.dismissed': '["passport"]' });
  });
});

// The guards `probe-black-rect.mjs --app-window` runs before it will judge a
// capture of the real app window. Parser inputs, not captures: they pin which
// window is taken and when the probe refuses, nothing about pixels.
import { pickAppWindow, screenIsLocked, worldAppearanceFromDefaults } from './extremeRuns';

describe('screenIsLocked', () => {
  it('reads the lock flag out of `ioreg -n Root -d1 -a`', () => {
    expect(screenIsLocked('<dict>\n\t\t\t<key>CGSSessionScreenIsLocked</key>\n\t\t\t<true/>\n</dict>')).toBe(true);
    expect(screenIsLocked('<dict><key>CGSSessionScreenIsLocked</key><false/></dict>')).toBe(false);
    expect(screenIsLocked('<dict><key>kCGSSessionOnConsoleKey</key><true/></dict>')).toBe(false);
  });
});

describe('pickAppWindow', () => {
  it('takes the largest normal window owned by the app pid, never a same-size window of another process', () => {
    const rows = [
      { id: 8613, pid: 397, layer: 2004, x: 0, y: 0, w: 2560, h: 1440 }, // a system overlay
      { id: 9000, pid: 1234, layer: 0, x: 0, y: 0, w: 2000, h: 1200 }, // another app
      { id: 8856, pid: 76299, layer: 0, x: 700, y: 300, w: 180, h: 60 }, // a tooltip-sized panel
      { id: 8855, pid: 76299, layer: 0, x: 640, y: 335, w: 1280, h: 800 }, // the window, as listed 2026-09-19
    ];
    expect(pickAppWindow(rows, [76299])?.id).toBe(8855);
    expect(pickAppWindow(rows, [4242])).toBeNull();
  });
});

describe('worldAppearanceFromDefaults', () => {
  // `--app-window` refuses unless this is 'night': the user's white block was
  // over the night sky, and a light-mode capture is the day scene.
  it('is night only when `defaults read -g AppleInterfaceStyle` succeeds with Dark', () => {
    expect(worldAppearanceFromDefaults('Dark\n', true)).toBe('night');
    // Light mode: the key does not exist and `defaults` exits 1.
    expect(worldAppearanceFromDefaults('', false)).toBe('day');
    // A failed read is never taken as night, whatever it printed.
    expect(worldAppearanceFromDefaults('Dark\n', false)).toBe('day');
    expect(worldAppearanceFromDefaults('Light\n', true)).toBe('day');
  });
});

// `--app-window` used to print "clean" for whatever view was open, and on
// 2026-09-19 the white-block node could not be confirmed because nobody could
// check the capture showed the World. These are real Vision reads (the lines
// `VNRecognizeTextRequest` returned for saved probe frames), not captures: they
// pin which text counts as "the World is on screen" and which appearance it
// says it resolved. They prove nothing about pixels or about Vision's accuracy.
import { worldFromOcr } from './extremeRuns';

describe('worldFromOcr', () => {
  it('confirms the night World from the Vision read of the --app-state night frame', () => {
    const night = ['Show the desk', 'WORLD/ SOLAR FÖRUM', 'The Solar Forum', 'Living forum -Night / follows system appearance', '^PLACES', 'AGEN', 'Fly the skiff', 'Walk the world', 'Show 1 hidden card'];
    const r = worldFromOcr(night);
    expect(r.world).toBe(true);
    expect(r.appearance).toBe('night');
    expect(r.anchors).toEqual(['The Solar Forum', 'follows system appearance', 'Walk the world', 'Fly the skiff', 'Show the desk']);
  });

  it('reads Day when the title card says the World resolved day', () => {
    expect(worldFromOcr(['The Solar Forum', 'Living forum · Day / follows system appearance']).appearance).toBe('day');
  });

  it('still confirms the World with the title card dismissed, but leaves the appearance unknown', () => {
    const r = worldFromOcr(['Hide the desk', 'AGENT DESK', 'Fly the skiff', 'Walk the world']);
    expect(r.world).toBe(true);
    expect(r.appearance).toBeNull();
  });

  it('refuses a capture that is not the World: another view, or one stray label', () => {
    // A Dashboard-shaped read: no World label at all.
    expect(worldFromOcr(['Dashboard', 'In flight', 'Calendar', 'Manifest']).world).toBe(false);
    // One label is not enough: a chat can quote "The Solar Forum".
    expect(worldFromOcr(['Ada', 'Tell me about The Solar Forum', 'Send']).world).toBe(false);
    expect(worldFromOcr([]).world).toBe(false);
  });
});
