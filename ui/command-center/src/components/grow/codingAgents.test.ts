import { execFileSync } from 'node:child_process';
import { chmodSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  CODING_AGENTS,
  SUBSCRIPTION_FIRST_HINT,
  codingAgentByCommand,
  codingAgentLaunchCommand,
  codingAgentSelectLabel,
  launchTooltip,
} from './codingAgents';

describe('CODING_AGENTS', () => {
  it('lists subscription CLIs before the metered harness', () => {
    expect(CODING_AGENTS.map(a => a.id)).toEqual(['claude', 'codex', 'cursor', 'permagent']);
    expect(CODING_AGENTS.filter(a => a.costTier === 'subscription').map(a => a.id)).toEqual([
      'claude',
      'codex',
      'cursor',
    ]);
    expect(CODING_AGENTS[CODING_AGENTS.length - 1]?.costTier).toBe('cheap_api');
  });

  it('names subscription cost in Claude and Codex tooltips so users do not have to dig', () => {
    const claude = codingAgentByCommand('claude');
    const codex = codingAgentByCommand('codex');
    expect(claude?.tooltip.toLowerCase()).toContain('subscription');
    expect(claude?.tooltip).not.toContain('$0');
    expect(codex?.tooltip.toLowerCase()).toContain('subscription');
    expect(codex?.tooltip).not.toContain('$0');
    expect(codingAgentByCommand('permagent run --recipe permagent-coding --interactive')?.tooltip)
      .toContain('API usage is billed');
  });

  it('launchTooltip prefers the root-path gate, then the cost policy', () => {
    expect(launchTooltip('claude', false)).toContain('root path');
    expect(launchTooltip('claude', true)).toContain('subscription');
    expect(SUBSCRIPTION_FIRST_HINT).toContain('with your subscription');
  });

  it('labels subscription and API routes without a price promise', () => {
    expect(codingAgentSelectLabel(CODING_AGENTS[0])).toBe('Claude · w/ Subscription');
    expect(codingAgentSelectLabel(CODING_AGENTS[CODING_AGENTS.length - 1]!)).toBe('Permagent · API / Local');
  });

  it('starts Cursor with the prompt as one safely quoted CLI argument', () => {
    const cursor = CODING_AGENTS.find(a => a.id === 'cursor')!;
    const prompt = `Fix it\nKeep user's notes intact`;
    expect(codingAgentLaunchCommand(cursor, prompt)).toBe("cursor-agent -- 'Fix it\nKeep user'\\''s notes intact'");
    expect(codingAgentLaunchCommand(cursor, '-leading directive')).toBe("cursor-agent -- '-leading directive'");
  });

  it('keeps other harnesses on their existing follow-up prompt path', () => {
    const claude = CODING_AGENTS.find(a => a.id === 'claude')!;
    expect(codingAgentLaunchCommand(claude, 'Fix it')).toBe('claude');
  });

  it('round-trips a long directive through zsh as one literal argv without running shell syntax', () => {
    const cursor = CODING_AGENTS.find(a => a.id === 'cursor')!;
    const temp = mkdtempSync(join(tmpdir(), 'grow-cursor-argv-'));
    const capturePath = join(temp, 'argv.json');
    const markerPath = join(temp, 'shell-ran');
    const prompt = `-${"é'".repeat(1800)}\nKeep 'quotes', $(touch ${markerPath}), and \`touch ${markerPath}\` as prompt text.`;
    const command = codingAgentLaunchCommand(cursor, prompt);

    try {
      const fakeCursor = join(temp, 'cursor-agent');
      writeFileSync(fakeCursor, [
        '#!/usr/bin/env node',
        'const fs = require("node:fs");',
        'fs.writeFileSync(process.env.CAPTURE_ARGV, JSON.stringify(process.argv.slice(2)));',
      ].join('\n'));
      chmodSync(fakeCursor, 0o755);

      expect(Buffer.byteLength(command)).toBeGreaterThan(4096);
      execFileSync('/bin/sh', ['-c', command], {
        env: {
          ...process.env,
          PATH: `${temp}:${process.env.PATH ?? ''}`,
          CAPTURE_ARGV: capturePath,
        },
        stdio: 'pipe',
      });

      expect(JSON.parse(readFileSync(capturePath, 'utf8'))).toEqual(['--', prompt]);
      expect(existsSync(markerPath)).toBe(false);
    } finally {
      rmSync(temp, { recursive: true, force: true });
    }
  });
});
