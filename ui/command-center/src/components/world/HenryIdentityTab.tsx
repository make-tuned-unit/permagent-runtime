import { useEffect } from 'react';
import { COLORS } from './constants';
import { Section, StatRow } from './HudShell';
import { useIdentityStore } from '../../stores/identityStore';

// ── Helpers ──────────────────────────────────────────────────────

function truncAddr(addr: string): string {
  return addr.slice(0, 6) + '...' + addr.slice(-4);
}

function daysAgo(iso: string): number {
  return Math.floor((Date.now() - new Date(iso).getTime()) / (24 * 60 * 60 * 1000));
}

function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
}

function relativeMinutes(date: Date | null): string {
  if (!date) return 'never';
  const mins = Math.floor((Date.now() - date.getTime()) / 60000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  return `${hrs}h ago`;
}

// ── Colors ───────────────────────────────────────────────────────

const IDENTITY_GREEN = '#4ADE80';
const SEALED_PILL = { bg: 'rgba(74, 222, 128, 0.12)', text: IDENTITY_GREEN, border: '#22C55E' };
const CHECK_PASS = '#4ADE80';
const CHECK_FAIL = '#EF4444';
const CHECK_WARN = COLORS.neonAmber;
const MUTED = '#6B7280';

const CONNECTIVITY_COLORS: Record<string, string> = {
  ok: '#4ADE80',
  degraded: COLORS.neonAmber,
  offline: '#EF4444',
};

// ── Component ────────────────────────────────────────────────────

export function HenryIdentityTab() {
  const { data: id, connectivity, lastSuccessfulFetch, startPolling, stopPolling } = useIdentityStore();

  useEffect(() => {
    startPolling();
    return () => stopPolling();
  }, [startPolling, stopPolling]);

  // First launch — no cached data at all
  if (!id) {
    return (
      <div style={{ padding: '20px 14px', textAlign: 'center' }}>
        <div style={{ fontSize: 11, color: MUTED, lineHeight: 1.6 }}>
          Awaiting first verification
        </div>
      </div>
    );
  }

  const verifiedDaysAgo = id.lastVerifiedAt ? daysAgo(id.lastVerifiedAt) : null;

  return (
    <>
      {/* Header row: avatar + name + status pill + connectivity indicator */}
      <div style={{ padding: '8px 14px 4px', display: 'flex', alignItems: 'center', gap: 10 }}>
        <img
          src={id.avatarUrl}
          alt={id.name}
          style={{ width: 36, height: 36, borderRadius: 6, border: `1px solid ${IDENTITY_GREEN}40` }}
        />
        <div style={{ flex: 1 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            <span style={{ fontSize: 14, fontWeight: 600, color: COLORS.primaryMarble }}>
              {id.name}
            </span>
            {connectivity !== 'ok' && (
              <span style={{
                width: 6,
                height: 6,
                borderRadius: '50%',
                background: CONNECTIVITY_COLORS[connectivity],
                display: 'inline-block',
                flexShrink: 0,
              }} title={`Chain: ${connectivity}`} />
            )}
          </div>
          <div style={{
            display: 'inline-block',
            padding: '1px 8px',
            borderRadius: 3,
            fontSize: 9,
            fontWeight: 700,
            letterSpacing: '0.08em',
            background: SEALED_PILL.bg,
            color: SEALED_PILL.text,
            border: `1px solid ${SEALED_PILL.border}`,
            marginTop: 2,
          }}>
            {id.status.toUpperCase()}
          </div>
        </div>
      </div>

      {/* SOUL */}
      <Section title="SOUL" trimColor={IDENTITY_GREEN}>
        <StatRow label="DID" value={id.did} />
        <StatRow label="Born" value={formatDate(id.bornAt)} />
        <StatRow label="Owner" value={truncAddr(id.owner)} />
      </Section>

      {/* VERIFICATION */}
      <Section title="VERIFICATION" trimColor={IDENTITY_GREEN}>
        <StatRow label="Status" value={id.status.toUpperCase()} />
        <StatRow label="Soul" value={id.soulValid ? 'Valid' : 'Invalid'} />
        <StatRow label="Last verified" value={verifiedDaysAgo != null ? `${verifiedDaysAgo}d ago` : 'N/A'} />
        <StatRow label="Alignment" value={id.alignmentScore != null ? String(id.alignmentScore) : 'N/A'} />
      </Section>

      {/* ON-CHAIN */}
      <Section title="ON-CHAIN" trimColor={COLORS.neonAmber}>
        <LinkRow label="SBT" value={`#${id.sbtId}`} />
        <LinkRow label="Passport" value={`#${id.passportId}`} />
        <LinkRow label="Chain" value="Base L2" />
        <LinkRow label="Arweave" value={truncAddr(id.arweaveTxId)} />
      </Section>

      {/* PORTAL READINESS */}
      <Section title="PORTAL READINESS" trimColor={COLORS.neonCyan}>
        <CheckRow
          label="Soul sealed"
          pass={id.soulValid}
          detail={id.soulValid ? 'OK' : 'Required'}
        />
        <CheckRow
          label="Last verified"
          pass={verifiedDaysAgo != null && verifiedDaysAgo < 30}
          detail={verifiedDaysAgo != null ? `${verifiedDaysAgo}d ago (requires <30d)` : 'N/A (requires <30d)'}
          warn={verifiedDaysAgo != null && verifiedDaysAgo >= 30}
        />
        <CheckRow
          label="Chronicle count"
          pass={id.chronicleCount >= 1}
          detail={`${id.chronicleCount} (requires >= 1)`}
        />
        <CheckRow
          label="Bindings"
          pass={id.bindingsCount >= 1}
          detail={`${id.bindingsCount} (requires >= 1)`}
        />
        <div style={{
          fontSize: 9,
          color: MUTED,
          marginTop: 6,
          fontStyle: 'italic',
          lineHeight: 1.4,
        }}>
          Diagnostic only — verification actions coming soon
        </div>
      </Section>

      {/* ACTIONS */}
      <div style={{ padding: '4px 14px 12px' }}>
        <div style={{
          fontSize: 10,
          fontWeight: 700,
          letterSpacing: '0.1em',
          color: MUTED,
          marginBottom: 6,
        }}>
          ACTIONS
        </div>
        <div style={{ display: 'flex', gap: 8 }}>
          <PlaceholderButton label="Verify Soul" />
          <PlaceholderButton label="Sync Chain" />
        </div>
      </div>

      {/* Footer: last refreshed */}
      <div style={{
        padding: '4px 14px 8px',
        fontSize: 9,
        color: '#4B5563',
        textAlign: 'right',
      }}>
        Last refreshed: {relativeMinutes(lastSuccessfulFetch)}
      </div>
    </>
  );
}

// ── Sub-components ───────────────────────────────────────────────

function LinkRow({ label, value }: { label: string; value: string }) {
  return (
    <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 11, lineHeight: 1.6 }}>
      <span style={{ color: '#9CA3AF' }}>{label}</span>
      <span style={{ color: COLORS.primaryMarble, fontWeight: 500 }}>
        {value}
        <span style={{ color: MUTED, marginLeft: 4, fontSize: 10 }}>↗</span>
      </span>
    </div>
  );
}

function CheckRow({ label, pass, detail, warn }: {
  label: string;
  pass: boolean;
  detail: string;
  warn?: boolean;
}) {
  const icon = pass ? '✓' : '✗';
  const color = pass ? CHECK_PASS : warn ? CHECK_WARN : CHECK_FAIL;
  return (
    <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 11, lineHeight: 1.6 }}>
      <span style={{ color: '#9CA3AF' }}>
        <span style={{ color, marginRight: 4 }}>{icon}</span>
        {label}
      </span>
      <span style={{ color: MUTED, fontSize: 10, fontWeight: 400 }}>{detail}</span>
    </div>
  );
}

function PlaceholderButton({ label }: { label: string }) {
  return (
    <button
      disabled
      style={{
        flex: 1,
        padding: '5px 0',
        background: 'rgba(107, 114, 128, 0.1)',
        border: '1px solid #4B556340',
        borderRadius: 4,
        color: '#4B5563',
        fontSize: 10,
        fontWeight: 600,
        fontFamily: 'monospace',
        letterSpacing: '0.04em',
        cursor: 'default',
      }}
    >
      {label}
    </button>
  );
}
