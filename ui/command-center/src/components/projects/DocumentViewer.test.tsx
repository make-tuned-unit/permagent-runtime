/** @vitest-environment jsdom */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { fetchProjectDocumentBlob } = vi.hoisted(() => ({
  fetchProjectDocumentBlob: vi.fn(),
}));

vi.mock('../../lib/api', () => ({
  api: { fetchProjectDocumentBlob },
}));

import { DocumentViewer } from './DocumentViewer';
import type { ProjectDocument } from './types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  fetchProjectDocumentBlob.mockReset().mockResolvedValue(
    new Blob(['%PDF-1.7'], { type: 'application/pdf' }),
  );
  vi.stubGlobal('URL', {
    createObjectURL: vi.fn(() => 'blob:document-preview'),
    revokeObjectURL: vi.fn(),
  });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

describe('DocumentViewer', () => {
  it('sandboxes PDF previews without scripts or same-origin access', async () => {
    const doc: ProjectDocument = {
      id: 'document-id',
      project_id: 'project-id',
      filename: 'invoice.pdf',
      mime_type: 'application/pdf',
      size_bytes: 8,
      path: '/stored/document-id',
      uploaded_at: '2026-07-24T00:00:00Z',
    };

    await act(async () => {
      root.render(<DocumentViewer projectId="project-id" doc={doc} onClose={() => {}} />);
    });

    const iframe = container.querySelector('iframe');
    expect(iframe).not.toBeNull();
    expect(iframe?.hasAttribute('sandbox')).toBe(true);
    expect(iframe?.getAttribute('sandbox')).toBe('');
    expect(iframe?.getAttribute('sandbox')).not.toContain('allow-same-origin');
    expect(iframe?.getAttribute('sandbox')).not.toContain('allow-scripts');
  });
});
