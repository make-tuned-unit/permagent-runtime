import { describe, expect, it } from 'vitest';
import { resolveActBinding } from './useBrowserActBridge';

const SNAPSHOT_WEBVIEW = 'snapshot-webview';
const PAGE = 'https://example.com/form';

describe('resolveActBinding', () => {
  it('acts on the SNAPSHOT webview even when a different tab is active', () => {
    // The act deliberately targets the webview the refs were stamped in, so
    // switching tabs after the snapshot must not retarget or drop the act.
    expect(
      resolveActBinding({ webview_id: SNAPSHOT_WEBVIEW, page_url: PAGE }, [
        'currently-active-webview',
        SNAPSHOT_WEBVIEW,
      ]),
    ).toEqual({
      kind: 'act',
      webviewId: SNAPSHOT_WEBVIEW,
      pageUrl: PAGE,
      generation: null,
    });
  });

  it('carries the snapshot generation through when present', () => {
    expect(
      resolveActBinding(
        { webview_id: SNAPSHOT_WEBVIEW, page_url: PAGE, generation: 'gen-7' },
        [SNAPSHOT_WEBVIEW],
      ),
    ).toEqual({
      kind: 'act',
      webviewId: SNAPSHOT_WEBVIEW,
      pageUrl: PAGE,
      generation: 'gen-7',
    });
  });

  it('ignores an act for a webview this client does not own', () => {
    // #939 fan-out: the event reaches EVERY connected client. A non-owner must
    // stay silent — performing it double-fires the action in the webview, and
    // even answering with an error would race the owner's real result.
    expect(
      resolveActBinding({ webview_id: SNAPSHOT_WEBVIEW, page_url: PAGE }, ['someone-elses-webview']),
    ).toEqual({ kind: 'ignore' });

    // A client with no browser tabs at all owns nothing.
    expect(resolveActBinding({ webview_id: SNAPSHOT_WEBVIEW, page_url: PAGE }, [])).toEqual({
      kind: 'ignore',
    });

    // Null tab slots (a tab whose webview has not been created yet) never match.
    expect(resolveActBinding({ webview_id: SNAPSHOT_WEBVIEW, page_url: PAGE }, [null])).toEqual({
      kind: 'ignore',
    });
  });

  it('reports an act without a complete snapshot identity as unbound', () => {
    const owned = [SNAPSHOT_WEBVIEW];
    expect(resolveActBinding({ webview_id: SNAPSHOT_WEBVIEW }, owned)).toEqual({ kind: 'unbound' });
    expect(resolveActBinding({ page_url: PAGE }, owned)).toEqual({ kind: 'unbound' });
    expect(resolveActBinding({ webview_id: SNAPSHOT_WEBVIEW, page_url: '' }, owned)).toEqual({
      kind: 'unbound',
    });
  });
});
