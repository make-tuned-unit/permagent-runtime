/**
 * @vitest-environment jsdom
 *
 * The generated Reel has to be watchable before it is approved.
 *
 * The daemon has produced Reels for a while: `format: "reel"` animates the
 * still through Higgsfield and pushes `{"kind": "video", "file": "…mp4"}` onto
 * the card's `metadata_json.media` (crates/goose/src/grow_media/mod.rs), and
 * the media route serves it as `video/mp4`
 * (crates/goose-server/src/routes/cards.rs). The app never looked: the media
 * read matched `kind === 'still'` and nothing else, and the calendar rendered
 * an `<img>`. There was no `<video>` element anywhere in the UI, so the person
 * pressing Approve on a Reel had never seen it move.
 *
 * These pin the whole path back — the read, the element, and the MIME — because
 * every step of it fails silently: a missing `videoFile` renders the poster and
 * looks correct, and a `<video>` with the wrong `type` renders a black box.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({
  api: { fetchGrowMediaBlob: vi.fn() },
  apiFetch: vi.fn(),
}));

import { PostVideo, mimeForFile } from './PostMedia';
import { readMediaMeta, type SocialCard } from './calendarPosts';
import { api } from '../../lib/api';
import { getThemedColors } from '../../styles/tokens';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const fetchBlob = vi.mocked(api.fetchGrowMediaBlob);
const colors = getThemedColors();

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  fetchBlob.mockReset();
  // jsdom has no blob URL factory.
  (URL as unknown as { createObjectURL: unknown }).createObjectURL = vi.fn(
    (b: Blob) => `blob:${b.type || 'unknown'}`,
  );
  (URL as unknown as { revokeObjectURL: unknown }).revokeObjectURL = vi.fn();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render(node: React.ReactElement) {
  await act(async () => { root.render(node); });
  await act(async () => { await Promise.resolve(); });
}

describe('readMediaMeta finds the Reel', () => {
  const card = (media: unknown[]): SocialCard => ({
    id: 'c1', title: 't', description: '', metadataJson: { media, mediaStatus: 'ready' },
  });

  it('reads the video item the animator writes beside the still', () => {
    const meta = readMediaMeta(card([
      { kind: 'still', file: 'still.png', source: 'compose' },
      { kind: 'video', file: 'out.mp4', source: 'higgsfield' },
    ]));
    expect(meta.stillFile).toBe('still.png');
    expect(meta.videoFile).toBe('out.mp4');
  });

  // A text or carousel post has no video, and a reel whose animation failed has
  // none either (the reason lands in mediaError). Neither may invent one.
  it('is null when the card carries only a still', () => {
    const meta = readMediaMeta(card([{ kind: 'still', file: 'still.png' }]));
    expect(meta.videoFile).toBeNull();
  });

  it('ignores a video item with no filename', () => {
    expect(readMediaMeta(card([{ kind: 'video', source: 'higgsfield' }])).videoFile).toBeNull();
  });
});

describe('PostVideo', () => {
  it('renders a real <video> with controls, so the Reel can be watched', async () => {
    fetchBlob.mockResolvedValue(new Blob(['x'], { type: 'video/mp4' }));
    await render(
      <PostVideo projectId="p1" cardId="c1" filename="out.mp4" colors={colors} />,
    );
    const video = container.querySelector('video');
    expect(video, 'no <video> element rendered').toBeTruthy();
    expect(video!.hasAttribute('controls')).toBe(true);
    // Never autoplay: a calendar of Reels all playing at once is why preload is
    // metadata-only too.
    expect(video!.hasAttribute('autoplay')).toBe(false);
    expect(video!.getAttribute('preload')).toBe('metadata');
  });

  it('gives the source the daemon’s own MIME, not a guess', async () => {
    fetchBlob.mockResolvedValue(new Blob(['x'], { type: 'video/mp4' }));
    await render(
      <PostVideo projectId="p1" cardId="c1" filename="out.mp4" colors={colors} />,
    );
    const source = container.querySelector('video source');
    expect(source!.getAttribute('type')).toBe('video/mp4');
    expect(source!.getAttribute('src')).toBe('blob:video/mp4');
  });

  // The route answers `application/octet-stream` for an extension it does not
  // know. Handing that to <video> renders a black box, so the filename is the
  // fallback rather than the blob's word.
  it('falls back to the filename when the blob has no video MIME', async () => {
    fetchBlob.mockResolvedValue(new Blob(['x'], { type: 'application/octet-stream' }));
    await render(
      <PostVideo projectId="p1" cardId="c1" filename="out.mp4" colors={colors} />,
    );
    expect(container.querySelector('video source')!.getAttribute('type')).toBe('video/mp4');
  });

  it('uses the post’s still as the poster frame', async () => {
    fetchBlob.mockImplementation((_p: string, _c: string, file: string) =>
      Promise.resolve(new Blob(['x'], { type: file.endsWith('.mp4') ? 'video/mp4' : 'image/png' })));
    await render(
      <PostVideo
        projectId="p1"
        cardId="c1"
        filename="out.mp4"
        posterFilename="still.png"
        colors={colors}
      />,
    );
    expect(container.querySelector('video')!.getAttribute('poster')).toBe('blob:image/png');
  });

  // Still generating, or gone. Saying so beats an empty box that reads as a
  // player that failed to start.
  it('says the Reel is loading rather than rendering a dead player', async () => {
    fetchBlob.mockRejectedValue(new Error('not ready'));
    await render(
      <PostVideo projectId="p1" cardId="c1" filename="out.mp4" colors={colors} />,
    );
    expect(container.querySelector('video')).toBeNull();
    expect(container.textContent).toContain('Reel loading');
  });

  it('names a MIME for every container the animator could write', () => {
    expect(mimeForFile('a.mp4')).toBe('video/mp4');
    expect(mimeForFile('a.webm')).toBe('video/webm');
    expect(mimeForFile('a.MOV')).toBe('video/quicktime');
  });
});
