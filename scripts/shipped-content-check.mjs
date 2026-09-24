/**
 * Shipped-content guard — single Node implementation (policy in shipped-content.json).
 * Scope roots use `globSync` from `node:fs`; rule `include` globs are repo-relative and
 * matched with `matchesGlob` from `node:path`.
 */

import { readFileSync, readdirSync, statSync } from 'node:fs';
import { globSync } from 'node:fs';
import { dirname, join, matchesGlob, relative, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const IDENTITY_DIAGNOSTIC =
  'Rulings are cited by date ("ruling 2026-07-03"); fixtures use neutral names; prompts say "the user".';

const POLICY_TOP_KEYS = new Set(['version', 'scope', 'rules', 'exceptions']);
const SCOPE_KEYS = new Set(['roots', 'extensions', 'skipDirectories']);
const RULE_KEYS = new Set(['id', 'kind', 'include', 'match', 'message']);
const EXCEPTION_KEYS = new Set([
  'id',
  'ruleId',
  'path',
  'line',
  'exactLine',
  'contextBefore',
  'contextAfter',
  'matchedText',
  'expectedMatches',
  'reasonCode',
  'reason',
  'reviewBy',
]);
const MATCHER_KEYS = new Set(['type', 'value', 'pattern', 'flags', 'anchor', 'window', 'requireAll', 'rejectAny']);
const PRIMITIVE_MATCHER_KEYS = new Set(['type', 'value', 'pattern', 'flags']);
const CONTEXT_MATCHER_KEYS = new Set(['type', 'anchor', 'window', 'requireAll', 'rejectAny']);
const ANCHOR_KEYS = new Set(['type', 'value', 'pattern', 'flags']);
const WINDOW_KEYS = new Set(['linesBefore', 'linesAfter']);

const RULE_KINDS = new Set(['operationalLiteral', 'unsupportedClaim']);
const REASON_CODES = new Set([
  'redactionPattern',
  'testFixture',
  'illustrativeExample',
  'historicalComment',
  'runtimeEvidence',
]);
const ALLOWED_REGEX_FLAGS = new Set(['i', 'm', 's', 'u']);

function assertNoUnknownKeys(obj, allowed, label) {
  if (obj === null || typeof obj !== 'object' || Array.isArray(obj)) {
    throw new PolicyError(`${label}: expected object`);
  }
  for (const key of Object.keys(obj)) {
    if (!allowed.has(key)) {
      throw new PolicyError(`${label}: unknown field "${key}"`);
    }
  }
}

export class PolicyError extends Error {
  constructor(message) {
    super(message);
    this.name = 'PolicyError';
  }
}

export class StalenessError extends Error {
  constructor(message) {
    super(message);
    this.name = 'StalenessError';
  }
}

function parseRegex(pattern, flags, label) {
  if (typeof pattern !== 'string' || pattern.length === 0) {
    throw new PolicyError(`${label}: pattern must be a non-empty string`);
  }
  if (typeof flags !== 'string') {
    throw new PolicyError(`${label}: flags must be a string`);
  }
  for (const ch of flags) {
    if (!ALLOWED_REGEX_FLAGS.has(ch)) {
      throw new PolicyError(`${label}: invalid regex flag "${ch}"`);
    }
  }
  let re;
  try {
    re = new RegExp(pattern, flags);
  } catch (e) {
    throw new PolicyError(`${label}: invalid regex (${e.message})`);
  }
  if (re.test('')) {
    throw new PolicyError(`${label}: regex must not match empty input`);
  }
  return re;
}

function validatePrimitiveMatcher(matcher, label, kinds = ['literal', 'regex']) {
  assertNoUnknownKeys(matcher, PRIMITIVE_MATCHER_KEYS, label);
  if (!kinds.includes(matcher.type)) {
    throw new PolicyError(`${label}: unsupported matcher type "${matcher.type}"`);
  }
  if (matcher.type === 'literal') {
    if (typeof matcher.value !== 'string' || matcher.value.length === 0) {
      throw new PolicyError(`${label}: literal value must be a non-empty string`);
    }
    return;
  }
  parseRegex(matcher.pattern, matcher.flags ?? '', label);
}

function validateContextMatcher(matcher, label) {
  assertNoUnknownKeys(matcher, CONTEXT_MATCHER_KEYS, label);
  if (matcher.type !== 'context') {
    throw new PolicyError(`${label}: expected context matcher`);
  }
  validatePrimitiveMatcher(matcher.anchor, `${label}.anchor`, ['literal', 'regex']);
  assertNoUnknownKeys(matcher.window, WINDOW_KEYS, `${label}.window`);
  if (
    !Number.isInteger(matcher.window.linesBefore) ||
    matcher.window.linesBefore < 0 ||
    !Number.isInteger(matcher.window.linesAfter) ||
    matcher.window.linesAfter < 0
  ) {
    throw new PolicyError(`${label}.window: linesBefore/linesAfter must be non-negative integers`);
  }
  if (!Array.isArray(matcher.requireAll)) {
    throw new PolicyError(`${label}: requireAll must be an array`);
  }
  if (!Array.isArray(matcher.rejectAny)) {
    throw new PolicyError(`${label}: rejectAny must be an array`);
  }
  matcher.requireAll.forEach((m, i) => validatePrimitiveMatcher(m, `${label}.requireAll[${i}]`));
  matcher.rejectAny.forEach((m, i) => validatePrimitiveMatcher(m, `${label}.rejectAny[${i}]`));
}

function validateRuleMatch(rule) {
  const label = `rule "${rule.id}".match`;
  const m = rule.match;
  if (rule.kind === 'operationalLiteral') {
    validatePrimitiveMatcher(m, label);
    return;
  }
  if (rule.kind === 'unsupportedClaim') {
    validateContextMatcher(m, label);
    return;
  }
  throw new PolicyError(`rule "${rule.id}": unknown kind`);
}

function validateIsoDate(s, label) {
  if (typeof s !== 'string' || !/^\d{4}-\d{2}-\d{2}$/.test(s)) {
    throw new PolicyError(`${label}: reviewBy must be ISO date YYYY-MM-DD`);
  }
  const t = Date.parse(`${s}T00:00:00.000Z`);
  if (!Number.isFinite(t)) {
    throw new PolicyError(`${label}: reviewBy is not a valid date`);
  }
}

function validateStringArray(arr, label) {
  if (!Array.isArray(arr) || arr.length === 0 || arr.some((x) => typeof x !== 'string' || x.length === 0)) {
    throw new PolicyError(`${label}: must be a non-empty array of non-empty strings`);
  }
}

/**
 * @param {unknown} policy
 * @param {{ repoRoot?: string, today?: string }} [opts]
 */
export function validatePolicy(policy, opts = {}) {
  assertNoUnknownKeys(policy, POLICY_TOP_KEYS, 'policy');
  if (policy.version !== 1) {
    throw new PolicyError('policy.version must be integer 1');
  }

  assertNoUnknownKeys(policy.scope, SCOPE_KEYS, 'policy.scope');
  validateStringArray(policy.scope.roots, 'policy.scope.roots');
  validateStringArray(policy.scope.extensions, 'policy.scope.extensions');
  validateStringArray(policy.scope.skipDirectories, 'policy.scope.skipDirectories');

  if (!Array.isArray(policy.rules) || policy.rules.length === 0) {
    throw new PolicyError('policy.rules must be a non-empty array');
  }
  const ruleIds = new Set();
  for (const rule of policy.rules) {
    assertNoUnknownKeys(rule, RULE_KEYS, 'rule');
    if (typeof rule.id !== 'string' || rule.id.length === 0) {
      throw new PolicyError('rule.id must be a non-empty string');
    }
    if (ruleIds.has(rule.id)) {
      throw new PolicyError(`duplicate rule id "${rule.id}"`);
    }
    ruleIds.add(rule.id);
    if (!RULE_KINDS.has(rule.kind)) {
      throw new PolicyError(`rule "${rule.id}": invalid kind`);
    }
    validateStringArray(rule.include, `rule "${rule.id}".include`);
    if (typeof rule.message !== 'string' || rule.message.length === 0) {
      throw new PolicyError(`rule "${rule.id}": message must be non-empty`);
    }
    assertNoUnknownKeys(rule.match, MATCHER_KEYS, `rule "${rule.id}".match`);
    validateRuleMatch(rule);
  }

  if (!Array.isArray(policy.exceptions)) {
    throw new PolicyError('policy.exceptions must be an array');
  }
  const exceptionIds = new Set();
  for (const ex of policy.exceptions) {
    assertNoUnknownKeys(ex, EXCEPTION_KEYS, 'exception');
    if (typeof ex.id !== 'string' || ex.id.length === 0) {
      throw new PolicyError('exception.id must be a non-empty string');
    }
    if (exceptionIds.has(ex.id)) {
      throw new PolicyError(`duplicate exception id "${ex.id}"`);
    }
    exceptionIds.add(ex.id);
    if (!ruleIds.has(ex.ruleId)) {
      throw new PolicyError(`exception "${ex.id}": unknown ruleId "${ex.ruleId}"`);
    }
    if (typeof ex.path !== 'string' || ex.path.length === 0) {
      throw new PolicyError(`exception "${ex.id}": path must be non-empty`);
    }
    if (!Number.isInteger(ex.line) || ex.line < 1) {
      throw new PolicyError(`exception "${ex.id}": line must be a positive integer`);
    }
    if (typeof ex.exactLine !== 'string') {
      throw new PolicyError(`exception "${ex.id}": exactLine must be a string`);
    }
    if (!Array.isArray(ex.contextBefore) || ex.contextBefore.some((l) => typeof l !== 'string')) {
      throw new PolicyError(`exception "${ex.id}": contextBefore must be an array of strings`);
    }
    if (!Array.isArray(ex.contextAfter) || ex.contextAfter.some((l) => typeof l !== 'string')) {
      throw new PolicyError(`exception "${ex.id}": contextAfter must be an array of strings`);
    }
    if (typeof ex.matchedText !== 'string' || ex.matchedText.length === 0) {
      throw new PolicyError(`exception "${ex.id}": matchedText must be non-empty`);
    }
    if (!Number.isInteger(ex.expectedMatches) || ex.expectedMatches < 1) {
      throw new PolicyError(`exception "${ex.id}": expectedMatches must be a positive integer`);
    }
    if (!REASON_CODES.has(ex.reasonCode)) {
      throw new PolicyError(`exception "${ex.id}": invalid reasonCode`);
    }
    if (typeof ex.reason !== 'string' || ex.reason.length === 0) {
      throw new PolicyError(`exception "${ex.id}": reason must be non-empty`);
    }
    validateIsoDate(ex.reviewBy, `exception "${ex.id}"`);
    const today = opts.today ?? new Date().toISOString().slice(0, 10);
    if (ex.reviewBy < today) {
      throw new StalenessError(`exception "${ex.id}": reviewBy ${ex.reviewBy} has expired`);
    }
  }

  if (opts.repoRoot) {
    resolveScopeRoots(policy.scope.roots, opts.repoRoot);
    verifyExceptionAnchors(policy.exceptions, opts.repoRoot);
  }

  return policy;
}

function resolveScopeRoots(roots, repoRoot) {
  const resolved = [];
  for (const pattern of roots) {
    let matches;
    try {
      matches = globSync(pattern, { cwd: repoRoot });
    } catch (e) {
      throw new PolicyError(`scope.roots: glob failed for "${pattern}": ${e.message}`);
    }
    if (matches.length === 0) {
      throw new PolicyError(`scope.roots: pattern "${pattern}" matched no directories`);
    }
    for (const rel of matches) {
      const abs = join(repoRoot, rel);
      let st;
      try {
        st = statSync(abs);
      } catch (e) {
        throw new PolicyError(`scope.roots: cannot stat "${rel}": ${e.message}`);
      }
      if (!st.isDirectory()) {
        throw new PolicyError(`scope.roots: "${rel}" is not a directory`);
      }
      resolved.push(abs);
    }
  }
  return resolved;
}

function verifyExceptionAnchors(exceptions, repoRoot) {
  for (const ex of exceptions) {
    const abs = join(repoRoot, ex.path);
    let text;
    try {
      text = readFileSync(abs, 'utf8');
    } catch (e) {
      throw new StalenessError(`exception "${ex.id}": cannot read ${ex.path}: ${e.message}`);
    }
    const lines = text.split('\n');
    if (ex.line > lines.length) {
      throw new StalenessError(`exception "${ex.id}": line ${ex.line} past end of ${ex.path}`);
    }
    const lineText = lines[ex.line - 1];
    if (lineText !== ex.exactLine) {
      throw new StalenessError(
        `exception "${ex.id}": exactLine mismatch at ${ex.path}:${ex.line}`,
      );
    }
    const beforeStart = ex.line - 1 - ex.contextBefore.length;
    if (beforeStart < 0) {
      throw new StalenessError(`exception "${ex.id}": contextBefore extends before file start`);
    }
    for (let i = 0; i < ex.contextBefore.length; i += 1) {
      if (lines[beforeStart + i] !== ex.contextBefore[i]) {
        throw new StalenessError(`exception "${ex.id}": contextBefore mismatch at ${ex.path}`);
      }
    }
    for (let i = 0; i < ex.contextAfter.length; i += 1) {
      const idx = ex.line + i;
      if (idx >= lines.length || lines[idx] !== ex.contextAfter[i]) {
        throw new StalenessError(`exception "${ex.id}": contextAfter mismatch at ${ex.path}`);
      }
    }
    let count = 0;
    let from = 0;
    while (true) {
      const pos = lineText.indexOf(ex.matchedText, from);
      if (pos === -1) break;
      count += 1;
      from = pos + ex.matchedText.length;
    }
    if (count < ex.expectedMatches) {
      throw new StalenessError(
        `exception "${ex.id}": matchedText "${ex.matchedText}" count ${count} < expectedMatches ${ex.expectedMatches}`,
      );
    }
  }
}

function pathMatchesInclude(relPath, includeGlobs) {
  // Globs are repo-relative; relPath is repo-relative with forward slashes.
  return includeGlobs.some((glob) => matchesGlob(relPath, glob));
}

function compilePrimitiveMatcher(matcher) {
  if (matcher.type === 'literal') {
    const value = matcher.value;
    return {
      findAll(text) {
        const out = [];
        let from = 0;
        while (from < text.length) {
          const pos = text.indexOf(value, from);
          if (pos === -1) break;
          out.push({ index: pos, text: value });
          from = pos + value.length;
        }
        return out;
      },
    };
  }
  const re = parseRegex(matcher.pattern, matcher.flags ?? '', 'matcher');
  return {
    findAll(text) {
      const out = [];
      const r = new RegExp(re.source, re.flags.includes('g') ? re.flags : `${re.flags}g`);
      let m;
      while ((m = r.exec(text)) !== null) {
        out.push({ index: m.index, text: m[0] });
        if (m[0].length === 0) break;
      }
      return out;
    },
    test(text) {
      return re.test(text);
    },
  };
}

function scanOperationalLiteral(rule, relPath, fileText) {
  const matcher = compilePrimitiveMatcher(rule.match);
  const findings = [];
  const lines = fileText.split('\n');
  for (let lineNo = 0; lineNo < lines.length; lineNo += 1) {
    const line = lines[lineNo];
    for (const hit of matcher.findAll(line)) {
      findings.push({
        ruleId: rule.id,
        path: relPath,
        line: lineNo + 1,
        matchedText: hit.text,
        message: rule.message,
        occurrenceKey: `${lineNo + 1}:${hit.index}:${hit.text}`,
      });
    }
  }
  return findings;
}

function scanUnsupportedClaim(rule, relPath, fileText) {
  const { anchor, window, requireAll, rejectAny } = rule.match;
  const anchorMatcher = compilePrimitiveMatcher(anchor);
  const requireMatchers = requireAll.map(compilePrimitiveMatcher);
  const rejectMatchers = rejectAny.map(compilePrimitiveMatcher);
  const lines = fileText.split('\n');
  const findings = [];
  for (let lineNo = 0; lineNo < lines.length; lineNo += 1) {
    const line = lines[lineNo];
    for (const hit of anchorMatcher.findAll(line)) {
      const start = Math.max(0, lineNo - window.linesBefore);
      const end = Math.min(lines.length - 1, lineNo + window.linesAfter);
      const windowText = lines.slice(start, end + 1).join('\n');
      if (requireMatchers.some((m) => !m.test(windowText))) continue;
      if (rejectMatchers.some((m) => m.test(windowText))) continue;
      findings.push({
        ruleId: rule.id,
        path: relPath,
        line: lineNo + 1,
        matchedText: hit.text,
        message: rule.message,
        occurrenceKey: `${lineNo + 1}:${hit.index}:${hit.text}`,
      });
    }
  }
  return findings;
}

function collectScopedFiles(policy, repoRoot) {
  const roots = resolveScopeRoots(policy.scope.roots, repoRoot);
  const extOk = (name) => policy.scope.extensions.some((ext) => name.endsWith(ext));
  const skipDir = new Set(policy.scope.skipDirectories);
  const files = [];

  function walk(dir) {
    let entries;
    try {
      entries = readdirSync(dir, { withFileTypes: true });
    } catch (e) {
      throw new PolicyError(`scan: cannot read directory ${dir}: ${e.message}`);
    }
    for (const ent of entries) {
      const abs = join(dir, ent.name);
      if (ent.isDirectory()) {
        if (skipDir.has(ent.name)) continue;
        walk(abs);
      } else if (ent.isFile() && extOk(ent.name)) {
        files.push(abs);
      }
    }
  }

  for (const root of roots) {
    walk(root);
  }
  return files;
}

function findingLine(f) {
  return `${f.ruleId}: ${f.path}:${f.line}: ${f.matchedText}: ${f.message}`;
}

function exceptionSlotKey(ex) {
  return `${ex.ruleId}|${ex.path}|${ex.line}|${ex.matchedText}`;
}

function applyExceptions(findings, exceptions) {
  const bySlot = new Map();
  for (const ex of exceptions) {
    const key = exceptionSlotKey(ex);
    if (!bySlot.has(key)) bySlot.set(key, []);
    bySlot.get(key).push(ex);
  }
  for (const [, list] of bySlot) {
    if (list.length > 1) {
      throw new StalenessError(`overlapping exceptions: ${list.map((e) => e.id).join(', ')}`);
    }
  }

  const suppressed = new Map();
  for (const ex of exceptions) {
    suppressed.set(ex.id, 0);
  }

  const remaining = [];
  for (const f of findings) {
    const candidates = exceptions.filter(
      (ex) =>
        ex.ruleId === f.ruleId &&
        ex.path === f.path &&
        ex.line === f.line &&
        f.matchedText === ex.matchedText,
    );
    if (candidates.length === 0) {
      remaining.push(f);
      continue;
    }
    const ex = candidates[0];
    const used = suppressed.get(ex.id) ?? 0;
    if (used < ex.expectedMatches) {
      suppressed.set(ex.id, used + 1);
      continue;
    }
    remaining.push(f);
  }

  for (const ex of exceptions) {
    const used = suppressed.get(ex.id) ?? 0;
    if (used === 0) {
      throw new StalenessError(`exception "${ex.id}" suppressed nothing`);
    }
    if (used !== ex.expectedMatches) {
      throw new StalenessError(
        `exception "${ex.id}" suppressed ${used} findings, expected ${ex.expectedMatches}`,
      );
    }
  }

  return remaining;
}

/**
 * @param {{ repoRoot: string, policy?: object, policyPath?: string, today?: string, emitIdentityDiagnostic?: boolean }} options
 */
export function runShippedContentCheck(options) {
  const repoRoot = resolve(options.repoRoot);
  let policy = options.policy;
  if (!policy) {
    const policyPath = options.policyPath ?? join(repoRoot, 'scripts/policies/shipped-content.json');
    policy = JSON.parse(readFileSync(policyPath, 'utf8'));
  }
  validatePolicy(policy, { repoRoot, today: options.today });

  const files = collectScopedFiles(policy, repoRoot);
  let findings = [];

  for (const abs of files) {
    const rel = relative(repoRoot, abs).split('\\').join('/');
    let text;
    try {
      text = readFileSync(abs, 'utf8');
    } catch (e) {
      throw new PolicyError(`scan: cannot read ${rel}: ${e.message}`);
    }
    for (const rule of policy.rules) {
      if (!pathMatchesInclude(rel, rule.include)) continue;
      if (rule.kind === 'operationalLiteral') {
        findings.push(...scanOperationalLiteral(rule, rel, text));
      } else if (rule.kind === 'unsupportedClaim') {
        findings.push(...scanUnsupportedClaim(rule, rel, text));
      }
    }
  }

  findings.sort((a, b) =>
    a.path.localeCompare(b.path) || a.line - b.line || a.ruleId.localeCompare(b.ruleId),
  );

  findings = applyExceptions(findings, policy.exceptions);

  const lines = findings.map(findingLine);
  if (options.emitIdentityDiagnostic) {
    lines.unshift(IDENTITY_DIAGNOSTIC);
  }

  return { findings, lines, ok: findings.length === 0 };
}

export function loadDefaultPolicy(repoRoot) {
  const policyPath = join(repoRoot, 'scripts/policies/shipped-content.json');
  const policy = JSON.parse(readFileSync(policyPath, 'utf8'));
  validatePolicy(policy, { repoRoot });
  return policy;
}

function runCli() {
  const here = dirname(fileURLToPath(import.meta.url));
  const repoRoot = resolve(here, '..');
  try {
    const { lines, ok } = runShippedContentCheck({ repoRoot });
    for (const line of lines) {
      console.log(line);
    }
    process.exit(ok ? 0 : 1);
  } catch (e) {
    console.error(e.message);
    process.exit(2);
  }
}

const invoked = process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url;
if (invoked) {
  runCli();
}
