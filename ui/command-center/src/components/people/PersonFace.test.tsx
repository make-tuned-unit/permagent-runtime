/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { PersonFace } from './PersonFace';
import { faceVisuals, personInitials, safePhotoUrl, shouldShowLabel, withAlpha } from './peopleFace';

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

describe('withAlpha', () => {
  it('expands a 6-digit hex to rgba', () => {
    expect(withAlpha('#00D5FF', 0.28)).toBe('rgba(0, 213, 255, 0.28)');
  });

  it('expands a 3-digit hex to rgba', () => {
    expect(withAlpha('#0ff', 0.5)).toBe('rgba(0, 255, 255, 0.5)');
  });

  it('passes through a non-hex color unchanged', () => {
    expect(withAlpha('rgba(1, 2, 3, 0.4)', 0.28)).toBe('rgba(1, 2, 3, 0.4)');
    expect(withAlpha('rgb(1, 2, 3)', 0.28)).toBe('rgb(1, 2, 3)');
  });
});

describe('shouldShowLabel', () => {
  const base = { isYou: false, hovered: false, focused: false, selected: false };

  it('is always true for the ego node, whatever the other flags are', () => {
    expect(shouldShowLabel({ ...base, isYou: true })).toBe(true);
  });

  it('is false for a person who is not hovered, focused, or selected', () => {
    expect(shouldShowLabel(base)).toBe(false);
  });

  it('is true while hovered', () => {
    expect(shouldShowLabel({ ...base, hovered: true })).toBe(true);
  });

  it('is true while keyboard-focused', () => {
    expect(shouldShowLabel({ ...base, focused: true })).toBe(true);
  });

  it('is true while selected (detail modal open)', () => {
    expect(shouldShowLabel({ ...base, selected: true })).toBe(true);
  });
});

describe('faceVisuals', () => {
  const accent = '#00D5FF';

  it('active overrides isQuiet dimming to full opacity and adds a ring glow + scale', () => {
    const v = faceVisuals({ active: true, dimmed: true, accent, reducedMotion: false });
    expect(v.opacity).toBe(1);
    expect(v.boxShadow).toContain(withAlpha(accent, 0.28));
    expect(v.boxShadow).toContain('0 6px 16px rgba(0,0,0,0.35)');
    expect(v.transform).toBe('scale(1.08)');
    expect(v.transition).toContain('transform 140ms ease');
  });

  it('reduced motion keeps opacity and the ring glow but drops the scale and its transition', () => {
    const v = faceVisuals({ active: true, dimmed: true, accent, reducedMotion: true });
    expect(v.opacity).toBe(1);
    expect(v.boxShadow).toContain(withAlpha(accent, 0.28));
    expect(v.transform).not.toContain('scale(');
    expect(v.transition).not.toContain('transform');
    expect(v.transition).toContain('opacity 140ms ease');
  });

  it('inactive keeps today\'s treatment: isQuiet dims, no ring glow', () => {
    const dimmed = faceVisuals({ active: false, dimmed: true, accent, reducedMotion: false });
    expect(dimmed.opacity).toBe(0.42);
    expect(dimmed.boxShadow).toBe('0 6px 16px rgba(0,0,0,0.35)');
    expect(dimmed.transform).not.toContain('scale(');

    const notDimmed = faceVisuals({ active: false, dimmed: false, accent, reducedMotion: false });
    expect(notDimmed.opacity).toBe(1);
    expect(notDimmed.boxShadow).toBe('0 6px 16px rgba(0,0,0,0.35)');
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

  it('active: full opacity even when dimmed, ring glow, and a scale-up', async () => {
    await act(async () => root.render(
      <PersonFace name="Ada Lovelace" photoUrl={null} size={40} accent="#00D5FF" dimmed active />,
    ));
    const el = container.querySelector('div[aria-label="Ada Lovelace"]') as HTMLDivElement;
    expect(el.style.opacity).toBe('1');
    expect(el.style.boxShadow).toContain('rgba(0, 213, 255, 0.28)');
    expect(el.style.transform).toContain('scale(1.08)');
  });

  it('reduced motion: no scale in the transform, but opacity and the ring glow still apply', async () => {
    await act(async () => root.render(
      <PersonFace name="Ada Lovelace" photoUrl={null} size={40} accent="#00D5FF" dimmed active reducedMotion />,
    ));
    const el = container.querySelector('div[aria-label="Ada Lovelace"]') as HTMLDivElement;
    expect(el.style.opacity).toBe('1');
    expect(el.style.transform).not.toContain('scale(');
    expect(el.style.boxShadow).toContain('rgba(0, 213, 255, 0.28)');
  });

  it('keyboard focus fires onFocusChange without swallowing onClick', async () => {
    let opened = false;
    const focusEvents: boolean[] = [];
    await act(async () => root.render(
      <PersonFace
        name="Ada Lovelace"
        photoUrl={null}
        size={40}
        accent="#0ff"
        onClick={() => { opened = true; }}
        onFocusChange={f => focusEvents.push(f)}
      />,
    ));
    const btn = container.querySelector('button')!;
    // React 17+ listens for focusin/focusout at the root (native focus/blur
    // don't bubble) to power onFocus/onBlur — dispatch those, not focus/blur.
    await act(async () => btn.dispatchEvent(new FocusEvent('focusin', { bubbles: true })));
    expect(focusEvents).toEqual([true]);
    await act(async () => btn.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(opened).toBe(true);
    await act(async () => btn.dispatchEvent(new FocusEvent('focusout', { bubbles: true })));
    expect(focusEvents).toEqual([true, false]);
  });
});
