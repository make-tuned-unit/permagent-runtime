// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { isPaneInputFocused, nextPaneTabId, shouldCyclePaneTabs } from './paneTabCycling';

function tabEvent(overrides: Partial<KeyboardEvent> = {}) {
  return { key: 'Tab', metaKey: false, ctrlKey: false, altKey: false, ...overrides } as KeyboardEvent;
}

describe('Build pane tab cycling', () => {
  it('leaves Tab alone when terminal/browser input is focused', () => {
    const pane = document.createElement('div');
    const input = document.createElement('input');
    const xtermInput = document.createElement('textarea');
    pane.append(input, xtermInput);

    expect(isPaneInputFocused(pane, input)).toBe(true);
    expect(isPaneInputFocused(pane, xtermInput)).toBe(true);
    expect(shouldCyclePaneTabs(tabEvent(), { paneSelected: true, pane, activeElement: input })).toBe(false);
    expect(shouldCyclePaneTabs(tabEvent(), { paneSelected: true, pane, activeElement: xtermInput })).toBe(false);
  });

  it('cycles only when the pane is selected and input is not focused', () => {
    const pane = document.createElement('div');
    const tabButton = document.createElement('button');
    pane.append(tabButton);

    expect(shouldCyclePaneTabs(tabEvent(), { paneSelected: true, pane, activeElement: tabButton })).toBe(true);
    expect(shouldCyclePaneTabs(tabEvent(), { paneSelected: false, pane, activeElement: tabButton })).toBe(false);
    expect(shouldCyclePaneTabs(tabEvent({ key: 'Enter' }), { paneSelected: true, pane, activeElement: tabButton })).toBe(false);
  });

  it('wraps forward and backward through tabs', () => {
    expect(nextPaneTabId(['a', 'b', 'c'], 'c')).toBe('a');
    expect(nextPaneTabId(['a', 'b', 'c'], 'a', true)).toBe('c');
    expect(nextPaneTabId(['a'], 'a')).toBeNull();
  });
});
