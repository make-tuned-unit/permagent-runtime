/**
 * Decision Inbox — layered evidence digest (Lane L4, amendment A3).
 *
 * Opens with a plain-language summary block (checks, reviewer, cost in
 * dollars). Raw CHECKS / DIFF / VERIFIER output — and token counts — sit
 * collapsed beneath a "Show details" toggle.
 *
 * S2: raw layers render inside a <pre> as React text nodes only. Literal
 * **markdown** stays literal; URLs stay plain text; nothing is auto-linked.
 */

import { useState } from 'react';
import { font, radius, ease } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import type { DecisionEvidence } from './types';
import { formatUsd } from './format';

export function EvidenceDigest({ evidence }: { evidence: DecisionEvidence }) {
  const { colors, reduceMotion } = useTheme();
  const [showDetails, setShowDetails] = useState(false);
  const s = evidence.summary;

  const checksLine =
    s.checks_total === 0
      ? null
      : s.checks_passed === s.checks_total
        ? `All ${s.checks_total} automated check${s.checks_total === 1 ? '' : 's'} passed` +
          (s.tests_passed > 0 ? ` (${s.tests_passed} tests)` : '')
        : `${s.checks_passed} of ${s.checks_total} automated checks passed`;
  const checksOk = s.checks_total > 0 && s.checks_passed === s.checks_total;

  return (
    <div style={{ marginTop: 12 }}>
      {/* Plain-language summary layer */}
      <div style={{
        borderRadius: radius.sm, border: `1px solid ${colors.border}`,
        padding: '10px 14px', fontFamily: font.body, fontSize: 12,
        color: colors.textMuted, display: 'flex', flexDirection: 'column', gap: 6,
      }}>
        {checksLine && (
          <SummaryRow ok={checksOk} text={checksLine} />
        )}
        <SummaryRow ok={s.verifier_confirmed} text={s.verifier_summary} />
        <div style={{ color: colors.text, fontWeight: 500 }}>
          Cost so far: {formatUsd(s.cost_usd)}
        </div>
        <button
          onClick={() => setShowDetails(o => !o)}
          style={{
            alignSelf: 'flex-start', background: 'none', border: 'none',
            color: showDetails ? colors.cyan : colors.textDim,
            fontSize: 11, fontFamily: font.body, cursor: 'pointer', padding: 0,
            transition: reduceMotion ? 'none' : `color 150ms ${ease.out}`,
          }}
        >
          {showDetails ? 'Hide details ▾' : 'Show details ▸'}
        </button>
      </div>

      {/* Raw layer — plain text only */}
      {showDetails && (
        <pre style={{
          margin: '8px 0 0', borderRadius: radius.sm,
          background: colors.codeBg, padding: '12px 14px',
          fontFamily: font.mono, fontSize: 11, lineHeight: 1.6,
          color: colors.textMuted,
          whiteSpace: 'pre-wrap', wordBreak: 'break-word', userSelect: 'text',
        }}>
          <Section label="CHECKS" color={colors.codeText} />
          {evidence.raw.checks}
          {'\n\n'}
          <Section label="DIFF" color={colors.codeText} />
          {evidence.raw.diff}
          {'\n\n'}
          <Section label="VERIFIER" color={colors.codeText} />
          {evidence.raw.verifier}
          {'\n\n'}
          <Section label="COST" color={colors.codeText} />
          {evidence.raw.cost_detail}
        </pre>
      )}
    </div>
  );
}

function SummaryRow({ ok, text }: { ok: boolean; text: string }) {
  const { colors } = useTheme();
  return (
    <div style={{ display: 'flex', gap: 8, alignItems: 'baseline' }}>
      <span style={{ color: ok ? colors.success : colors.warning, flexShrink: 0 }}>
        {ok ? '✓' : '•'}
      </span>
      <span>{text}</span>
    </div>
  );
}

function Section({ label, color }: { label: string; color: string }) {
  return <span style={{ color, fontWeight: 600 }}>{label}{'\n'}</span>;
}
