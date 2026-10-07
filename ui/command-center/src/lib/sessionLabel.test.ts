import { describe, expect, it } from 'vitest';
import {
  classifySession,
  repairName,
  sessionDisplayLabel,
  sessionProjectName,
  sessionSearchText,
  titleFromMessage,
  UNTITLED_SESSION,
} from './sessionLabel';

/** The Grow → coding agent directive, shaped as codingAgentDirective.ts writes it. */
const EVNTALLY_DIRECTIVE = [
  'You are a coding agent working in the repository for "Acme".',
  'Working directory: /Users/j/Documents/dev/evntally',
  'Implement the growth action below in this repo. Do not stop at drafting copy in chat — write the change into the right files.',
  '',
  '## Task',
  'Fix event_cta_click funnel leak: 3,836 impressions → 940 clicks (24.5% CTR) suggests either CTA placement or copy issues',
  'Category: conversion',
  '',
  '## Steps',
  "- Track `{name: 'scroll_depth', depth: 25|50|75|100, event_id: '{event-id}'}` on the event page",
].join('\n');

describe('sessionDisplayLabel — every junk title from the 2026-09-24 QA walk', () => {
  it('{ maxChars: Infinity } (History) shows the whole first-message title, not the 80-char cut', () => {
    const long = 'Fix event_cta_click funnel leak: 3,836 impressions to 940 clicks, which is a 24.5 percent CTR that suggests either CTA placement or copy issues are to blame';
    const compact = sessionDisplayLabel({ name: 'x', firstUserText: long });
    const full = sessionDisplayLabel({ name: 'x', firstUserText: long }, { maxChars: Infinity });
    expect(compact.endsWith('…')).toBe(true);
    expect(compact.length).toBeLessThan(full.length);
    expect(full).toBe(long);
    // A name the user actually typed is never capped by this option either way.
    expect(sessionDisplayLabel({ name: long, userSetName: true }, { maxChars: Infinity })).toBe(long);
  });

  it('never shows a raw object literal', () => {
    const raw = "{name: 'scroll_depth', depth: 25|50|75|100, event_id: '{event-id}'}";
    expect(sessionDisplayLabel({ name: raw })).toBe('Scroll depth');
    // With the first message the daemon now carries, the real task wins.
    const label = sessionDisplayLabel({ name: raw, firstUserText: EVNTALLY_DIRECTIVE });
    expect(label.startsWith('Fix event_cta_click funnel leak')).toBe(true);
    expect(label).not.toContain('{');
  });

  it('strips wrapping quotes and treats "Merged text content" as no title', () => {
    expect(repairName('"Merged text content"')).toBeNull();
    expect(sessionDisplayLabel({ name: '"Merged text content"' })).toBe(UNTITLED_SESSION);
    expect(sessionDisplayLabel({
      name: '"Merged text content"',
      firstUserText: "Review Permagent's own logs for the exact last 24 hours. This is a\ndiagnostic job.",
    })).toBe("Review Permagent's own logs for the exact last 24 hours");
    expect(sessionDisplayLabel({ name: '"Plan the Atlas launch"' })).toBe('Plan the Atlas launch');
    expect(sessionDisplayLabel({ name: '“Plan the Atlas launch”' })).toBe('Plan the Atlas launch');
  });

  it('names a scheduled run after its job, not a line of its template', () => {
    expect(sessionDisplayLabel({ name: 'Desktop — no battery', sessionType: 'scheduled', recipeTitle: 'Workspace Snapshot' }))
      .toBe('Workspace Snapshot');
    expect(sessionDisplayLabel({ name: 'Desktop — no battery', sessionType: 'scheduled', scheduleId: 'workspace_snapshot' }))
      .toBe('Workspace snapshot');
    // An older daemon sends neither: the stored name is all there is.
    expect(sessionDisplayLabel({ name: 'Desktop — no battery', sessionType: 'scheduled' })).toBe('Desktop — no battery');
  });

  it('treats "CLI Session", "Human-Readable Summary", "New Chat" and "Session <id>" as missing', () => {
    for (const name of ['CLI Session', 'Human-Readable Summary', 'New Chat', 'New session', 'New Session', 'Session 20260924_12', '']) {
      expect(sessionDisplayLabel({ name })).toBe(UNTITLED_SESSION);
    }
    expect(sessionDisplayLabel({ name: 'CLI Session', firstUserText: 'how do I rotate the staging keys?' }))
      .toBe('how do I rotate the staging keys?');
    expect(sessionDisplayLabel({ name: 'Human-Readable Summary', sessionType: 'scheduled', recipeTitle: 'Storage Insights' }))
      .toBe('Storage Insights');
    expect(sessionDisplayLabel({ name: 'CLI Session', goalTitle: 'Ship the watch app' })).toBe('Ship the watch app');
    expect(sessionDisplayLabel({ name: 'New Chat', projectName: 'Atlas' })).toBe('Atlas');
  });

  it('drops a shouted prefix and the stray space before a colon', () => {
    expect(sessionDisplayLabel({ name: 'ATTENTION : CI failing on' })).toBe('CI failing on');
    expect(sessionDisplayLabel({ name: '**Battery:** 80%' })).toBe('Battery: 80%');
  });

  it('keeps a name the user typed, repaired only for quotes', () => {
    expect(sessionDisplayLabel({ name: 'my notes', userSetName: true, firstUserText: 'something else entirely' }))
      .toBe('my notes');
    expect(sessionDisplayLabel({ name: '"quoted by me"', userSetName: true })).toBe('quoted by me');
  });

  it('keeps the recovered-literal behaviour Home Recent shipped with', () => {
    expect(sessionDisplayLabel({ name: "{name: 'scroll_depth', depth: 0.75, sel..." })).toBe('Scroll depth');
    expect(sessionDisplayLabel({ name: '{"tool":"browser_navigate","args":{"url":"htt' })).toBe('Browser navigate');
    expect(sessionDisplayLabel({ name: '{ "goal": "Ship the release notes", "prio' })).toBe('Ship the release notes');
    expect(sessionDisplayLabel({ name: "{depth: 3, selector: 'main-article', x: 1}" })).toBe('Main article');
    expect(sessionDisplayLabel({ name: '{"depth": 3, "x": 4}' })).toBe(UNTITLED_SESSION);
    expect(sessionDisplayLabel({ name: '[1, 2, 3]' })).toBe(UNTITLED_SESSION);
    expect(sessionDisplayLabel({ name: '   ' })).toBe(UNTITLED_SESSION);
  });

  it('leaves ordinary titles alone, including ones that mention braces', () => {
    expect(sessionDisplayLabel({ name: 'Plan the Atlas launch' })).toBe('Plan the Atlas launch');
    expect(sessionDisplayLabel({ name: 'Why does {x} render twice?' })).toBe('Why does {x} render twice?');
    expect(sessionDisplayLabel({ name: '{curly} braces in a sentence' })).toBe('{curly} braces in a sentence');
  });
});

describe('titleFromMessage', () => {
  it('prefers a Goal: line and skips code literals', () => {
    expect(titleFromMessage('Some context first\nGoal: ship the watch app')).toBe('ship the watch app');
    expect(titleFromMessage("{name: 'x', depth: 25|50}\nAdd scroll depth tracking")).toBe('Add scroll depth tracking');
    expect(titleFromMessage('# Fix login')).toBeNull();
    expect(titleFromMessage('')).toBeNull();
  });

  it('caps a long line on a word boundary', () => {
    const t = titleFromMessage('word '.repeat(40))!;
    expect(t.endsWith('…')).toBe(true);
    expect(Array.from(t).length).toBeLessThanOrEqual(81);
  });

  it('maxChars: Infinity (History) keeps the whole line, still cut at the first sentence', () => {
    const long = 'word '.repeat(40).trim();
    const t = titleFromMessage(long, Infinity)!;
    expect(t).toBe(long); // no cap, no ellipsis — the row wraps it in CSS instead
    const twoSentences = 'Fix the staging key rotation. Also look at the backup job while you are in there.';
    expect(titleFromMessage(twoSentences, Infinity)).toBe('Fix the staging key rotation');
  });
});

describe('classifySession', () => {
  it('files the probe sessions under System', () => {
    for (const w of ['SHIPPED', 'TWO', 'ONE', 'READY']) {
      const text = `Reply with exactly the word ${w} and nothing else. Do not call any tools.`;
      expect(classifySession({ name: text, firstUserText: text, sessionType: 'user' })).toBe('system');
      // Named only (older daemon, no first message): still a probe.
      expect(classifySession({ name: `Reply with exactly the word ${w} and nothing else` })).toBe('system');
    }
  });

  it('files scheduled and hidden sessions under System', () => {
    expect(classifySession({ name: 'Desktop — no battery', sessionType: 'scheduled' })).toBe('system');
    expect(classifySession({ name: 'Human-Readable Summary', scheduleId: 'storage_insights' })).toBe('system');
    expect(classifySession({ name: 'CLI Session', sessionType: 'hidden' })).toBe('system');
  });

  it('files coding-harness and goal workers under Agent runs', () => {
    expect(classifySession({ name: 'You are a coding agent working in the repository for "Acme". Working directory: /Users/j/Docs…' }))
      .toBe('agent_run');
    expect(classifySession({ name: "{name: 'scroll_depth', depth: 25|50|75|100, event_id: '{event-id}'}", firstUserText: EVNTALLY_DIRECTIVE }))
      .toBe('agent_run');
    expect(classifySession({ name: 'Fix the login bug', recipeTitle: 'Permagent Coding Harness' })).toBe('agent_run');
    expect(classifySession({ name: 'Draft the landing copy', goalId: 'card-1' })).toBe('agent_run');
    expect(classifySession({ name: 'x', sessionType: 'sub_agent' })).toBe('agent_run');
  });

  it('keeps people’s chats as Conversations', () => {
    expect(classifySession({ name: 'Plan the Atlas launch', sessionType: 'user' })).toBe('conversation');
    expect(classifySession({ name: 'New Chat', firstUserText: 'Can you reply to Sam about Thursday?' })).toBe('conversation');
    // A person asking for "only" something is not a probe unless it opens that way.
    expect(classifySession({ name: 'x', firstUserText: 'What should I say only to the board?' })).toBe('conversation');
    expect(classifySession({ name: 'x', sessionType: 'terminal' })).toBe('conversation');
  });

  it('does not file real conversations under System just for opening with a reply verb (REVIEW_CONFIRMED #5)', () => {
    // Opens with one of the probe verbs and has "exactly"/"only" nearby, but
    // is not the fixed liveness-check template — must stay a conversation.
    expect(classifySession({ name: 'x', firstUserText: 'Answer only in JSON, no other commentary.', sessionType: 'user' }))
      .toBe('conversation');
    expect(classifySession({ name: 'x', firstUserText: 'Say only what changed in this file.', sessionType: 'user' }))
      .toBe('conversation');
    // Contains the old NO_TOOLS_PROBE phrase, but is ordinary chat, not the probe template.
    expect(classifySession({ name: 'x', firstUserText: "Let's just talk, do not use any tools for this one", sessionType: 'user' }))
      .toBe('conversation');
    // A "reply with exactly the word ... and nothing else" mention that isn't
    // actually the harness's own liveness sentence — no fixed single-token
    // word right after "the word", so it must not be mistaken for a probe.
    expect(classifySession({
      name: 'x',
      firstUserText: 'What is exactly the word for the feeling of missing a place you have never been?',
      sessionType: 'user',
    })).toBe('conversation');
    expect(classifySession({ name: 'x', firstUserText: 'reply with exactly the word count for this chapter', sessionType: 'user' }))
      .toBe('conversation');
  });
});

describe('sessionProjectName / sessionSearchText', () => {
  it('reads the project from the daemon, else from the coding directive', () => {
    expect(sessionProjectName({ projectName: 'Atlas' })).toBe('Atlas');
    expect(sessionProjectName({ firstUserText: EVNTALLY_DIRECTIVE })).toBe('Acme');
    expect(sessionProjectName({ name: 'You are a coding agent working in the repository for "Acme". Work…' })).toBe('Acme');
    expect(sessionProjectName({ name: 'Plan the launch' })).toBeNull();
  });

  it('matches on the label and the first message', () => {
    const text = sessionSearchText({ name: 'CLI Session', firstUserText: 'Rotate the staging keys\nthen redeploy' });
    expect(text).toContain('rotate the staging keys');
    expect(text).toContain('redeploy');
  });
});
