#!/usr/bin/env node
/**
 * AST-based title= → <Tooltip> conversion (AF3 sweep).
 *
 * Usage (from repo root or ui/command-center):
 *   node scripts/codemod-title-to-tooltip-ast.mjs ui/command-center/src/components/finance
 *
 * Skips heading/label props (ViewHeader, Section, Panel, …), iframe/option/svg
 * titles. Disabled hosts get the #1177 span+tabIndex wrapper so the tip stays
 * reachable. Non-interactive hosts get tabIndex={0}.
 */

import { readFileSync, writeFileSync, readdirSync, statSync, existsSync } from 'node:fs';
import { join, relative, dirname, resolve, sep } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, '..');

// Prefer command-center's typescript (worktree may run from repo root).
const require = createRequire(join(repoRoot, 'ui/command-center/package.json'));
const ts = require('typescript');

const rootArg = process.argv[2];
if (!rootArg) {
  console.error('Usage: node scripts/codemod-title-to-tooltip-ast.mjs <dir>');
  process.exit(1);
}
const root = resolve(process.cwd(), rootArg);
if (!existsSync(root)) {
  console.error('Not found:', root);
  process.exit(1);
}

const SKIP_COMPONENTS = new Set([
  'ViewHeader', 'DetailModal', 'FormModal', 'ConfirmDialog', 'Chip',
  'Section', 'WorkSection', 'StateBlock', 'SectionTitle', 'Panel',
  'HudShell', 'LinksPanel', 'H1', 'H2', 'Group',
  'DisclaimerDialog', 'MeetingsSection', 'ToggleChip', 'IconButton',
  'GrowthProjectRow', 'LinkButton',
]);
const SKIP_TAGS = new Set(['iframe', 'option', 'svg', 'title']);
const INTERACTIVE_TAGS = new Set(['button', 'Button', 'a', 'A', 'input', 'select', 'textarea', 'summary']);

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (name.endsWith('.tsx') && !name.endsWith('.test.tsx')) out.push(p);
  }
  return out;
}

function tooltipImportPath(file) {
  const tip = join(repoRoot, 'ui/command-center/src/components/common/Tooltip.tsx');
  let rel = relative(dirname(file), tip).replace(/\\/g, '/').replace(/\.tsx$/, '');
  if (!rel.startsWith('.')) rel = `./${rel}`;
  return rel;
}

function ensureImport(src, file) {
  if (/from ['"].*\/Tooltip['"]/.test(src) || /from ['"]\.\/Tooltip['"]/.test(src)) {
    return src;
  }
  const spec = tooltipImportPath(file);
  const line = `import { Tooltip } from '${spec}';`;
  // Insert after the last top-level import statement.
  const importRe = /^import\s[\s\S]*?;\s*$/gm;
  let last = null;
  let m;
  while ((m = importRe.exec(src))) last = m;
  if (!last) return `${line}\n${src}`;
  const insertAt = last.index + last[0].length;
  const after = src.slice(insertAt);
  const pad = after.startsWith('\n') ? '\n' : '\n\n';
  return src.slice(0, insertAt) + pad + line + (after.startsWith('\n') ? '' : '\n') + after;
}

function attrName(attr) {
  return attr.name.getText();
}

function tagBase(node) {
  return node.tagName.getText().split('.')[0];
}

/** Returns { kind: 'string', value } | { kind: 'expr', value } */
function contentParts(attr, sf) {
  if (!attr.initializer) return { kind: 'expr', value: 'true' };
  if (attr.initializer.kind === ts.SyntaxKind.StringLiteral) {
    return { kind: 'string', value: attr.initializer.text };
  }
  if (attr.initializer.kind === ts.SyntaxKind.JsxExpression) {
    const expr = attr.initializer.expression;
    if (!expr) return { kind: 'expr', value: 'undefined' };
    // title={'literal'} → treat as string when possible
    if (ts.isStringLiteral(expr) || ts.isNoSubstitutionTemplateLiteral(expr)) {
      return { kind: 'string', value: expr.text };
    }
    return { kind: 'expr', value: expr.getText(sf) };
  }
  return { kind: 'expr', value: attr.initializer.getText(sf) };
}

function contentAttr(parts) {
  if (parts.kind === 'string') return `content=${JSON.stringify(parts.value)}`;
  return `content={${parts.value}}`;
}

function hasAttr(attrs, name) {
  for (const a of attrs.properties) {
    if (ts.isJsxAttribute(a) && attrName(a) === name) return a;
  }
  return null;
}

function isInteractive(tag, attrs) {
  if (INTERACTIVE_TAGS.has(tag)) return true;
  if (hasAttr(attrs, 'onClick') || hasAttr(attrs, 'onPress')) return true;
  if (hasAttr(attrs, 'tabIndex') || hasAttr(attrs, 'tabindex')) return true;
  const role = hasAttr(attrs, 'role');
  if (role?.initializer) {
    const t = role.initializer.getText();
    if (/button|link|menuitem|tab|switch|checkbox|radio/.test(t)) return true;
  }
  return false;
}

/** Remove a title= attribute, preserving surrounding whitespace cleanly. */
function stripTitle(fullText, titleAttr, sf) {
  const start = titleAttr.getFullStart(); // includes leading trivia (whitespace/newline)
  const end = titleAttr.getEnd();
  // Prefer dropping the leading whitespace that belonged to this attr.
  return { start, end, text: '' };
}

function wrapElement(src, elStart, elEnd, content, { disabledWrap, needTabIndex, indent }) {
  const inner = src.slice(elStart, elEnd);
  let body = inner;
  // If we need tabIndex on a non-interactive host and it doesn't have one,
  // inject into the opening tag. Simpler: wrap in span with tabIndex.
  if (disabledWrap) {
    body = `<span tabIndex={0} style={{ display: 'inline-flex', outline: 'none' }}>\n${indent}  ${inner.split('\n').join(`\n${indent}  `)}\n${indent}</span>`;
    return `<Tooltip content={${content}}>\n${indent}${body}\n${indent}</Tooltip>`;
  }
  if (needTabIndex) {
    // Wrap so we don't have to splice into the opening tag attributes.
    return `<Tooltip content={${content}}><span tabIndex={0} style={{ outline: 'none' }}>${inner}</span></Tooltip>`;
  }
  return `<Tooltip content={${content}}>${inner}</Tooltip>`;
}

let changedFiles = 0;
const manual = [];
const converted = [];

for (const file of walk(root)) {
  let src = readFileSync(file, 'utf8');
  const sf = ts.createSourceFile(file, src, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);

  /** @type {{ elStart: number, elEnd: number, titleStart: number, titleEnd: number, content: string, disabledWrap: boolean, needTabIndex: boolean, indent: string, tag: string, line: number }[]} */
  const jobs = [];

  function visit(node) {
    const isOpen = ts.isJsxOpeningElement(node);
    const isSelf = ts.isJsxSelfClosingElement(node);
    if (!isOpen && !isSelf) {
      ts.forEachChild(node, visit);
      return;
    }

    const tag = tagBase(node);
    if (SKIP_COMPONENTS.has(tag) || SKIP_TAGS.has(tag.toLowerCase())) {
      ts.forEachChild(node, visit);
      return;
    }

    const titleAttr = hasAttr(node.attributes, 'title');
    if (!titleAttr) {
      ts.forEachChild(node, visit);
      return;
    }

    const line = sf.getLineAndCharacterOfPosition(titleAttr.getStart()).line + 1;
    const parts = contentParts(titleAttr, sf);
    if (parts.kind === 'expr' && parts.value.includes('\n') && parts.value.length > 400) {
      manual.push(`${relative(process.cwd(), file)}:${line}: oversized title expr on <${tag}>`);
      ts.forEachChild(node, visit);
      return;
    }

    let elStart;
    let elEnd;
    if (isSelf) {
      elStart = node.getStart(sf);
      elEnd = node.getEnd();
    } else {
      // Opening element — parent should be JsxElement
      const parent = node.parent;
      if (!parent || !ts.isJsxElement(parent)) {
        manual.push(`${relative(process.cwd(), file)}:${line}: opening <${tag}> without JsxElement parent`);
        ts.forEachChild(node, visit);
        return;
      }
      elStart = parent.getStart(sf);
      elEnd = parent.getEnd();
    }

    const disabledAttr = hasAttr(node.attributes, 'disabled');
    const interactive = isInteractive(tag, node.attributes);
    const lineStart = src.lastIndexOf('\n', elStart - 1) + 1;
    const indent = src.slice(lineStart, elStart).match(/^[ \t]*/)?.[0] ?? '';

    jobs.push({
      elStart,
      elEnd,
      titleStart: titleAttr.getFullStart(),
      titleEnd: titleAttr.getEnd(),
      contentAttr: contentAttr(parts),
      disabledWrap: !!disabledAttr,
      needTabIndex: !interactive && !disabledAttr,
      indent,
      tag,
      line,
    });

    ts.forEachChild(node, visit);
  }
  visit(sf);

  if (!jobs.length) continue;

  // Nested title hosts: converting an inner range invalidates the outer's
  // end offset. Keep only jobs that are not strict ancestors of another job.
  const nestedSafe = jobs.filter((job, i) => {
    for (let j = 0; j < jobs.length; j += 1) {
      if (i === j) continue;
      const other = jobs[j];
      if (other.elStart > job.elStart && other.elEnd < job.elEnd) {
        // `job` strictly contains `other` — skip the outer; a later pass /
        // hand edit can wrap it once the inner is done.
        return false;
      }
    }
    return true;
  });
  for (const skipped of jobs) {
    if (!nestedSafe.includes(skipped)) {
      manual.push(`${relative(process.cwd(), file)}:${skipped.line}: nested title on <${skipped.tag}> — convert outer after inner`);
    }
  }
  jobs.length = 0;
  jobs.push(...nestedSafe);

  if (!jobs.length) continue;

  jobs.sort((a, b) => b.elStart - a.elStart);

  let next = src;
  const fileHits = [];
  for (const job of jobs) {
    const slice = next.slice(job.elStart, job.elEnd);
    // Strip title attr inside the slice (positions relative to original next,
    // which still matches original src for untouched suffix — since we go from
    // end, [0, elStart) is unchanged and title is inside [elStart, elEnd).
    const relTitleStart = job.titleStart - job.elStart;
    const relTitleEnd = job.titleEnd - job.elStart;
    let stripped = slice.slice(0, relTitleStart) + slice.slice(relTitleEnd);
    // Drop a blank line left behind when title sat on its own line.
    stripped = stripped.replace(/\n[ \t]*\n([ \t]*\S)/g, '\n$1');
    // Clean doubled spaces left on the same line as a removed title.
    stripped = stripped.replace(/(\S) {2,}(\S)/g, '$1 $2');
    // Trim trailing space before `/>` only (do NOT touch `>` — it breaks
    // `=>` and collapses the `>` that closes a multi-line opening tag).
    stripped = stripped.replace(/ +\/>/g, ' />');

    let wrapped;
    const multiline = stripped.includes('\n');
    // `stripped` keeps absolute file indentation on continuation lines; only
    // the first line is flush at column 0 relative to elStart. Re-indent by
    // prefixing the first line and adding two spaces to the rest.
    const indentBlock = (extra = '  ') => stripped.split('\n').map((ln, i) => (
      i === 0 ? `${job.indent}${extra}${ln}` : `${extra}${ln}`
    )).join('\n');
    if (job.disabledWrap) {
      wrapped = `<Tooltip ${job.contentAttr}>\n${job.indent}  <span tabIndex={0} style={{ display: 'inline-flex', outline: 'none' }}>\n${indentBlock('    ')}\n${job.indent}  </span>\n${job.indent}</Tooltip>`;
    } else if (job.needTabIndex) {
      if (multiline) {
        wrapped = `<Tooltip ${job.contentAttr}>\n${job.indent}  <span tabIndex={0} style={{ outline: 'none' }}>\n${indentBlock('    ')}\n${job.indent}  </span>\n${job.indent}</Tooltip>`;
      } else {
        wrapped = `<Tooltip ${job.contentAttr}><span tabIndex={0} style={{ outline: 'none' }}>${stripped}</span></Tooltip>`;
      }
    } else if (multiline) {
      wrapped = `<Tooltip ${job.contentAttr}>\n${indentBlock()}\n${job.indent}</Tooltip>`;
    } else {
      wrapped = `<Tooltip ${job.contentAttr}>${stripped}</Tooltip>`;
    }

    next = next.slice(0, job.elStart) + wrapped + next.slice(job.elEnd);
    fileHits.push(`${job.tag}@${job.line}`);
  }

  next = ensureImport(next, file);
  writeFileSync(file, next);
  changedFiles += 1;
  converted.push(`${relative(process.cwd(), file)} (${fileHits.length}): ${fileHits.join(', ')}`);
  console.log('updated', relative(process.cwd(), file), `(${fileHits.length})`);
}

console.log(`\nRewrote ${changedFiles} file(s), ${converted.reduce((n, c) => n + Number(c.match(/\((\d+)\)/)[1]), 0)} attrs.`);
if (manual.length) {
  console.log('\nNeeds a human:');
  for (const m of manual) console.log(' -', m);
}
