/**
 * R3 (dagK UX standard, 2026-09-25): tool cards showed raw identifiers
 * (`card_create`, `gmail__search`) as user-facing copy. toolLabel() covers
 * the override map for common tools and the mechanical fallback for
 * everything else.
 */
import { describe, expect, it } from 'vitest';
import { genericToolLabel, toolLabel } from './toolLabel';

describe('toolLabel overrides', () => {
  it('names the common Permagent tools in plain English', () => {
    expect(toolLabel('card_create')).toBe('Create a card');
    expect(toolLabel('column_delete')).toBe('Delete a column');
    expect(toolLabel('project_launch')).toBe('Launch the project');
    expect(toolLabel('council_convene')).toBe('Convene the Council');
    expect(toolLabel('search_memory')).toBe('Search memory');
    expect(toolLabel('read_dashboard')).toBe('Read the dashboard');
  });

  it('names the Gmail and developer tools already special-cased in ToolResult', () => {
    expect(toolLabel('gmail__search')).toBe('Search Gmail');
    expect(toolLabel('gmail_search')).toBe('Search Gmail');
    expect(toolLabel('developer__shell')).toBe('Run a shell command'); // falls through to the generic path below
    expect(toolLabel('shell')).toBe('Run a shell command');
    expect(toolLabel('read')).toBe('Read a file');
  });
});

describe('genericToolLabel fallback', () => {
  it('strips an ext__tool prefix before reading the name', () => {
    expect(genericToolLabel('developer__shell')).toBe('Shell');
    expect(genericToolLabel('someext__do_a_thing')).toBe('Do a thing');
  });

  it('splits snake_case and sentence-cases it', () => {
    expect(genericToolLabel('widget_status')).toBe('Widget status');
  });

  it('moves a known trailing verb to the front (Permagent\'s noun_verb convention)', () => {
    expect(genericToolLabel('foo_bar_delete')).toBe('Delete foo bar');
    expect(genericToolLabel('ticket_resolve')).toBe('Resolve ticket');
  });

  it('never throws and never returns empty for an empty or single-word name', () => {
    expect(genericToolLabel('')).toBe('');
    expect(genericToolLabel('frobnicate')).toBe('Frobnicate');
  });

  it('is what toolLabel falls back to for anything not in the override map', () => {
    expect(toolLabel('some_unmapped_tool_name')).toBe(genericToolLabel('some_unmapped_tool_name'));
  });
});
