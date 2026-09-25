/**
 * The Learned section used to title a saved skill with its raw call
 * signature ("copy_to_clipboard:-te…") and rank it MASTERED/TRUSTED/LEARNED
 * off a count that measures repetition, not competence. These pin the
 * parse/humanize/group rules that replaced that (see skillHumanize.ts for
 * the field provenance).
 */
import { describe, expect, it } from 'vitest';
import {
  groupSkills,
  humanizeSkill,
  matchesLabel,
  parseCallSignature,
  toolVerb,
} from './skillHumanize';

describe('parseCallSignature', () => {
  it('parses the description form "tool: key=value"', () => {
    expect(parseCallSignature('write: path=/a/b')).toMatchObject({
      tool: 'write', op: 'write', argKey: 'path', argValue: '/a/b', redacted: false,
    });
  });

  it('parses the slugged name form "tool:-key=value"', () => {
    expect(parseCallSignature('copy_to_clipboard:-text=hello')).toMatchObject({
      tool: 'copy_to_clipboard', argKey: 'text', argValue: 'hello',
    });
  });

  it('splits an extension-namespaced tool into extension + op', () => {
    expect(parseCallSignature('orchestrator__goal_status:-card_id=[redacted]')).toMatchObject({
      extension: 'orchestrator', op: 'goal_status', redacted: true,
    });
  });

  it('treats a bare command (no key=value) as the "command" key', () => {
    expect(parseCallSignature('shell: ls -la')).toMatchObject({ argKey: 'command', argValue: 'ls -la' });
  });

  it('returns null for prose that only happens to contain a colon', () => {
    expect(parseCallSignature('Note: buy milk')).toBeNull();
  });

  it('returns null for a user-typed skill name', () => {
    expect(parseCallSignature('Summarize my downloads')).toBeNull();
  });

  it('accepts a bare word with no args or underscore when it matches the known toolUsed', () => {
    // "search" alone has no underscore and no argument, so it would not
    // otherwise pass the "looks like a tool" check — matching `toolUsed`
    // exactly is what makes it one.
    expect(parseCallSignature('search', 'search')).toMatchObject({ tool: 'search', op: 'search' });
  });
});

describe('toolVerb', () => {
  it('reads a known operation in plain words', () => {
    expect(toolVerb({ op: 'copy_to_clipboard', extension: null })).toBe('Copy text to clipboard');
  });

  it('falls back to a sentence-cased operation name for an unknown tool', () => {
    expect(toolVerb({ op: 'frobnicate_widget', extension: null })).toBe('Frobnicate widget');
  });

  it('appends the extension for an unknown namespaced tool', () => {
    expect(toolVerb({ op: 'do_thing', extension: 'gadget' })).toBe('Do thing (gadget)');
  });
});

describe('humanizeSkill', () => {
  it('turns a path argument into "in <last two folders>"', () => {
    const h = humanizeSkill({ id: '1', name: 'write:-path=[redacted]', description: 'write: path=/Users/j/dev/permagent-runtime/docs/x.md', toolUsed: 'write' });
    expect(h.title).toBe('Write a file in permagent-runtime/docs');
    expect(h.groupKey).toBe('tool:Write a file');
  });

  it('quotes a shell command argument', () => {
    const h = humanizeSkill({ id: '2', name: 'shell', description: 'shell: git status', toolUsed: 'shell' });
    expect(h.title).toBe('Run a shell command “git status”');
  });

  it('never surfaces a redacted argument as the variant', () => {
    const h = humanizeSkill({ id: '3', name: 'x', description: 'orchestrator__goal_status: card_id=[REDACTED]', toolUsed: 'orchestrator__goal_status' });
    expect(h.title).toBe("Check a goal's status");
    expect(h.variant).toBeNull();
  });

  it('keeps a user-named skill as-is, with no raw signature to show', () => {
    const h = humanizeSkill({ id: '4', name: 'Summarize my downloads weekly', description: 'Summarize my downloads weekly' });
    expect(h.title).toBe('Summarize my downloads weekly');
    expect(h.raw).toBeNull();
  });

  it('keeps the raw signature for the detail disclosure when one was parsed', () => {
    const h = humanizeSkill({ id: '5', name: 'x', description: 'write: path=/a/b.md', toolUsed: 'write' });
    expect(h.raw).toBe('write: path=/a/b.md');
  });
});

describe('groupSkills', () => {
  it('groups same-tool skills together, busiest group first', () => {
    const groups = groupSkills([
      { id: '1', name: 'x', description: 'read: path=/a', toolUsed: 'read', usageCount: 1 },
      { id: '2', name: 'x', description: 'write: path=/b', toolUsed: 'write', usageCount: 10 },
      { id: '3', name: 'x', description: 'write: path=/c', toolUsed: 'write', usageCount: 5 },
    ]);
    expect(groups[0].label).toBe('Write a file');
    expect(groups[0].items.map(i => i.skill.id)).toEqual(['2', '3']); // busiest pattern first within the group
    expect(groups[0].matches).toBe(15);
    expect(groups[1].label).toBe('Read a file');
  });

  it('gives a user-named skill its own single-item group', () => {
    const groups = groupSkills([{ id: '1', name: 'My weekly digest', usageCount: 2 }]);
    expect(groups).toHaveLength(1);
    expect(groups[0].items).toHaveLength(1);
  });
});

describe('matchesLabel', () => {
  it('reads "matched N×", not a mastery tier', () => {
    expect(matchesLabel(0)).toBe('matched 0×');
    expect(matchesLabel(37)).toBe('matched 37×');
  });
});
