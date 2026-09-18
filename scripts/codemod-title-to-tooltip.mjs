#!/usr/bin/env node
/**
 * Semi-automated title= → <Tooltip> conversion for other UI DAG lanes.
 *
 * Usage (from repo root or ui/command-center):
 *   node scripts/codemod-title-to-tooltip.mjs ui/command-center/src/components/finance
 *
 * Handles simple cases only:
 *   <button … title="Foo" …>kids</button>
 *   <Button … title={'Foo'} …>kids</Button>
 * Leaves multi-line / expression titles for a human, and never touches
 * ViewHeader / DetailModal / FormModal / ConfirmDialog heading props or
 * iframe / svg titles.
 */

import { readFileSync, writeFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

const root = process.argv[2];
if (!root) {
  console.error('Usage: node scripts/codemod-title-to-tooltip.mjs <dir>');
  process.exit(1);
}

const SKIP_COMPONENTS = new Set([
  'ViewHeader', 'DetailModal', 'FormModal', 'ConfirmDialog', 'Chip',
]);

const TITLE_ATTR = /\s+title=\{([^}]*)\}|\s+title=("([^"]*)"|'([^']*)')/;

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (name.endsWith('.tsx') && !name.endsWith('.test.tsx')) out.push(p);
  }
  return out;
}

function ensureImport(src) {
  if (/from ['"].*\/Tooltip['"]/.test(src) || /from ['"]\.\/Tooltip['"]/.test(src)) {
    return src;
  }
  // Prefer a sibling-relative import; lanes adjust if wrong.
  const line = `import { Tooltip } from '../common/Tooltip';\n`;
  const m = src.match(/^import .+$/m);
  if (!m) return line + src;
  return src.replace(m[0], `${m[0]}\n${line.trimEnd()}`);
}

let changed = 0;
const manual = [];

for (const file of walk(root)) {
  let src = readFileSync(file, 'utf8');
  let fileChanged = false;

  // One-line self-closing or paired tags with a string/simple title.
  const tagRe = /<([A-Za-z][\w.]*)([^>]*?)\btitle=(?:\{([^}]*)\}|"([^"]*)"|'([^']*)')([^>]*?)(\/>|>)/g;
  src = src.replace(tagRe, (full, name, before, expr, d1, d2, after, close) => {
    if (SKIP_COMPONENTS.has(name.split('.')[0])) return full;
    if (name === 'iframe') return full;
    const content = expr != null ? expr.trim() : JSON.stringify(d1 ?? d2);
    if (content.includes('\n') || content.length > 120) {
      manual.push(`${relative(process.cwd(), file)}: complex title on <${name}>`);
      return full;
    }
    fileChanged = true;
    const attrs = `${before}${after}`.replace(/\s+/g, ' ').trim();
    const open = attrs ? `<${name} ${attrs}${close === '/>' ? ' />' : '>'}` : `<${name}${close === '/>' ? ' />' : '>'}`;
    if (close === '/>') {
      return `<Tooltip content={${content.startsWith('"') || content.startsWith("'") || content.startsWith('`') ? content : content}}>${open}</Tooltip>`;
    }
    // Paired tag — only safe when the rest of the element is on later lines;
    // mark for manual if we cannot see the closer on this slice.
    manual.push(`${relative(process.cwd(), file)}: paired <${name} title=…> — wrap manually`);
    return full;
  });

  // Simpler pass: button/Button with title="..." on the opening tag line only,
  // when the closing tag is on a following line — leave to manual list above.
  // Dedicated string-literal self-closing rewrite:
  const selfClose = /<(button|Button|span|div|a)\b([^>]*?)\s+title=("([^"]*)"|'([^']*)')([^>]*?)\/>/g;
  src = src.replace(selfClose, (full, name, before, _q, d1, d2, after) => {
    fileChanged = true;
    const label = d1 ?? d2;
    const attrs = `${before}${after}`.replace(/\s+/g, ' ').trim();
    return `<Tooltip content=${JSON.stringify(label)}><${name}${attrs ? ` ${attrs}` : ''} /></Tooltip>`;
  });

  if (fileChanged) {
    src = ensureImport(src);
    writeFileSync(file, src);
    changed += 1;
    console.log('updated', relative(process.cwd(), file));
  }
}

console.log(`\nRewrote ${changed} file(s).`);
if (manual.length) {
  console.log('\nNeeds a human:');
  for (const m of manual) console.log(' -', m);
}
