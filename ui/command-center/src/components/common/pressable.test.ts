/**
 * pressable() — the a11y/keyboard contract every `.pa-pressable` call site
 * gets for free. See pressable.ts for why this exists rather than converting
 * ~550 call sites directly.
 */
import { describe, expect, it, vi } from 'vitest';
import { pressable, pressableEnterOnly } from './pressable';

function fakeKeyEvent(key: string, opts: { fromChild?: boolean } = {}) {
  const currentTarget = {};
  const target = opts.fromChild ? {} : currentTarget;
  return {
    key, target, currentTarget, preventDefault: vi.fn(),
  } as unknown as Parameters<ReturnType<typeof pressable>['onKeyDown']>[0];
}

describe('pressable', () => {
  it('defaults to role="button", tabIndex 0, and the pa-pressable class', () => {
    const props = pressable(() => {});
    expect(props.role).toBe('button');
    expect(props.tabIndex).toBe(0);
    expect(props.className.split(' ')).toContain('pa-pressable');
  });

  it('accepts a custom role and merges extra class names', () => {
    const props = pressable(() => {}, { role: 'option', className: 'my-row' });
    expect(props.role).toBe('option');
    expect(props.className.split(' ')).toEqual(expect.arrayContaining(['pa-pressable', 'my-row']));
  });

  it('activates on click', () => {
    const onActivate = vi.fn();
    const props = pressable(onActivate);
    const e = { preventDefault: vi.fn() } as unknown as Parameters<NonNullable<typeof props.onClick>>[0];
    props.onClick?.(e);
    expect(onActivate).toHaveBeenCalledTimes(1);
  });

  it('activates on Enter and on Space, each preventing default', () => {
    const onActivate = vi.fn();
    const props = pressable(onActivate);
    const enter = fakeKeyEvent('Enter');
    props.onKeyDown(enter);
    expect(onActivate).toHaveBeenCalledTimes(1);
    expect(enter.preventDefault).toHaveBeenCalled();

    const space = fakeKeyEvent(' ');
    props.onKeyDown(space);
    expect(onActivate).toHaveBeenCalledTimes(2);
    expect(space.preventDefault).toHaveBeenCalled();
  });

  it('ignores other keys', () => {
    const onActivate = vi.fn();
    const props = pressable(onActivate);
    props.onKeyDown(fakeKeyEvent('Tab'));
    expect(onActivate).not.toHaveBeenCalled();
  });

  it('disabled (enabled: false) drops tabIndex to -1, sets aria-disabled, and does not activate', () => {
    const onActivate = vi.fn();
    const props = pressable(onActivate, { enabled: false });
    expect(props.tabIndex).toBe(-1);
    expect(props['aria-disabled']).toBe(true);
    expect(props.onClick).toBeUndefined();
    props.onKeyDown(fakeKeyEvent('Enter'));
    expect(onActivate).not.toHaveBeenCalled();
  });

  it('sets aria-label only when a label is given', () => {
    expect(pressable(() => {})['aria-label']).toBeUndefined();
    expect(pressable(() => {}, { label: 'Open row' })['aria-label']).toBe('Open row');
  });

  it('pressableEnterOnly ignores Space (for rows nesting their own controls)', () => {
    const onActivate = vi.fn();
    const props = pressableEnterOnly(onActivate);
    props.onKeyDown(fakeKeyEvent(' '));
    expect(onActivate).not.toHaveBeenCalled();
    props.onKeyDown(fakeKeyEvent('Enter'));
    expect(onActivate).toHaveBeenCalledTimes(1);
  });

  it('ignores a keydown that bubbled up from a nested interactive child, so it does not steal the child\'s own Enter/Space', () => {
    const onActivate = vi.fn();
    const props = pressable(onActivate);
    const enter = fakeKeyEvent('Enter', { fromChild: true });
    props.onKeyDown(enter);
    expect(onActivate).not.toHaveBeenCalled();
    expect(enter.preventDefault).not.toHaveBeenCalled();

    const space = fakeKeyEvent(' ', { fromChild: true });
    props.onKeyDown(space);
    expect(onActivate).not.toHaveBeenCalled();
    expect(space.preventDefault).not.toHaveBeenCalled();
  });

  it('pressableEnterOnly also ignores a bubbled Enter from a nested child (its own documented use case)', () => {
    const onActivate = vi.fn();
    const props = pressableEnterOnly(onActivate);
    const enter = fakeKeyEvent('Enter', { fromChild: true });
    props.onKeyDown(enter);
    expect(onActivate).not.toHaveBeenCalled();
  });
});
