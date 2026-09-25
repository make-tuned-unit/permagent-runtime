/** @vitest-environment jsdom */
import { describe, expect, it } from 'vitest';
import { commandForKey } from './DecisionReview.keymap';

const div = () => document.createElement('div');

describe('review keymap (U14)', () => {
  it('maps the verdict, navigation and help keys', () => {
    const t = div();
    expect(commandForKey({ key: '1', target: t })).toEqual({ type: 'verdict', key: '1' });
    expect(commandForKey({ key: '0', target: t })).toEqual({ type: 'verdict', key: '0' });
    expect(commandForKey({ key: 'Enter', target: t })).toEqual({ type: 'default' });
    expect(commandForKey({ key: 'h', target: t })).toEqual({ type: 'later' });
    expect(commandForKey({ key: 'j', target: t })).toEqual({ type: 'next' });
    expect(commandForKey({ key: 'k', target: t })).toEqual({ type: 'prev' });
    expect(commandForKey({ key: 'z', target: t })).toEqual({ type: 'undo' });
    expect(commandForKey({ key: 'z', metaKey: true, target: t })).toEqual({ type: 'undo' });
    expect(commandForKey({ key: '/', target: t })).toEqual({ type: 'ask' });
    expect(commandForKey({ key: '?', shiftKey: true, target: t })).toEqual({ type: 'shortcuts' });
  });

  it('is inert in text fields, while composing, and with other modifiers', () => {
    for (const tag of ['textarea', 'input', 'select']) {
      expect(commandForKey({ key: '1', target: document.createElement(tag) })).toBeNull();
    }
    expect(commandForKey({ key: 'j', isComposing: true, target: div() })).toBeNull();
    expect(commandForKey({ key: 'j', ctrlKey: true, target: div() })).toBeNull();
    expect(commandForKey({ key: 'J', shiftKey: true, target: div() })).toBeNull();
    expect(commandForKey({ key: '1', altKey: true, target: div() })).toBeNull();
  });

  it('works from a focused button, but leaves Return to the button itself', () => {
    const button = document.createElement('button');
    expect(commandForKey({ key: '1', target: button })).toEqual({ type: 'verdict', key: '1' });
    expect(commandForKey({ key: 'Enter', target: button })).toBeNull();
  });
});
