import { describe, expect, it } from 'vitest';
import { resolveActBinding } from './useBrowserActBridge';

describe('resolveActBinding', () => {
  it('targets the snapshot webview instead of a different active webview', () => {
    expect(
      resolveActBinding(
        { webview_id: 'snapshot-webview', page_url: 'https://example.com/form' },
        'currently-active-webview',
      ),
    ).toEqual({
      webviewId: 'snapshot-webview',
      pageUrl: 'https://example.com/form',
    });
  });

  it('rejects an act without a complete snapshot identity', () => {
    expect(resolveActBinding({ webview_id: 'snapshot-webview' }, 'active-webview')).toBeNull();
    expect(resolveActBinding({ page_url: 'https://example.com/' }, 'active-webview')).toBeNull();
    expect(
      resolveActBinding({ webview_id: 'snapshot-webview', page_url: '' }, 'active-webview'),
    ).toBeNull();
  });
});
