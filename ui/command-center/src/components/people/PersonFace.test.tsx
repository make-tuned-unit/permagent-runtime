/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { PersonFace } from './PersonFace';
import { personInitials, safePhotoUrl } from './peopleFace';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

describe('personInitials', () => {
  it('uses first and last initials', () => {
    expect(personInitials('Ada Lovelace')).toBe('AL');
  });
  it('uses two letters of a single name', () => {
    expect(personInitials('Bea')).toBe('BE');
  });
});

describe('safePhotoUrl', () => {
  it('keeps http(s) and drops javascript', () => {
    expect(safePhotoUrl('https://cdn.example.com/ada.jpg')).toBe('https://cdn.example.com/ada.jpg');
    expect(safePhotoUrl('javascript:alert(1)')).toBeNull();
  });

  it('keeps a Wikimedia commons headshot (Claudia Chender)', () => {
    const url = 'https://upload.wikimedia.org/wikipedia/commons/4/41/CLAUDIA-LEADERSHOOT-WEB-10-HeadshotCropped.png';
    expect(safePhotoUrl(`${url}?utm_source=en.wikipedia.org`)).toBe(`${url}?utm_source=en.wikipedia.org`);
  });
});

describe('PersonFace', () => {
  let container: HTMLDivElement;
  let root: Root;
  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });
  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  it('renders the photo and clicking the face fires onClick', async () => {
    let opened = false;
    await act(async () => root.render(
      <PersonFace
        name="Ada Lovelace"
        photoUrl="https://cdn.example.com/ada.jpg"
        size={40}
        accent="#0ff"
        onClick={() => { opened = true; }}
      />,
    ));
    const img = container.querySelector('img');
    expect(img?.getAttribute('src')).toBe('https://cdn.example.com/ada.jpg');
    const btn = container.querySelector('button')!;
    expect(btn.getAttribute('aria-label')).toBe('Open Ada Lovelace');
    await act(async () => btn.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(opened).toBe(true);
  });

  it('falls back to initials when there is no photo', async () => {
    await act(async () => root.render(
      <PersonFace name="Ada Lovelace" photoUrl={null} size={40} accent="#0ff" />,
    ));
    expect(container.querySelector('img')).toBeNull();
    expect(container.textContent).toBe('AL');
  });
});
