import { useEffect, useState, useCallback, type CSSProperties } from 'react';
import { Section, StatRow } from './HudShell';
import { useIdentityStore } from '../../stores/identityStore';
import { computePortalEligibility } from '../../utils/portalEligibility';
import { SBT_CONTRACT } from '../../config/chain';
import { duration, ease, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

// ── External link helper ─────────────────────────────────────────

async function openExternal(url: string): Promise<void> {
  try {
    const { open } = await import('@tauri-apps/plugin-shell');
    await open(url);
  } catch {
    window.open(url, '_blank', 'noopener');
  }
}

// ── Helpers ──────────────────────────────────────────────────────

function truncAddr(addr: string): string {
  return addr.slice(0, 6) + '...' + addr.slice(-4);
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

// ── URLs ─────────────────────────────────────────────────────────

function sbtUrl(tokenId: number): string {
  return `https://basescan.org/nft/${SBT_CONTRACT}/${tokenId}`;
}

function passportUrl(tokenId: number): string {
  return `https://www.8004scan.io/agents/base/${tokenId}`;
}

function arweaveUrl(txId: string): string {
  return `https://arweave.net/${txId}`;
}

// ── Theme-derived identity ink (was hardcoded greens/reds) ─────────

function sealedPill(success: string) {
  return { bg: `${success}1f`, text: success, border: success };
}
function connectivityColor(c: { success: string; warning: string; dangerStrong: string }, state: string): string {
  if (state === 'ok') return c.success;
  if (state === 'degraded') return c.warning;
  return c.dangerStrong;
}

// ── Component ────────────────────────────────────────────────────

export function HenryIdentityTab() {
  const { colors } = useTheme();
  const { data: id, loading, connectivity, lastSuccessfulFetch, refresh, startPolling, stopPolling } = useIdentityStore();

  useEffect(() => {
    startPolling();
    return () => stopPolling();
  }, [startPolling, stopPolling]);

  const handleRefresh = useCallback(() => {
    if (!loading) refresh();
  }, [loading, refresh]);

  // First launch — no cached data at all
  if (!id) {
    return (
      <div style={{ padding: '20px 14px', textAlign: 'center' }}>
        <div style={{ fontSize: textSize.micro, color: colors.textDim, lineHeight: 1.6 }}>
          Awaiting first verification
        </div>
      </div>
    );
  }

  const eligibility = computePortalEligibility(id);

  return (
    <>
      {/* Header row: avatar + name + status pill + connectivity indicator */}
      <div style={{ padding: '8px 14px 4px', display: 'flex', alignItems: 'center', gap: 10 }}>
        <img
          src={id.avatarUrl}
          alt={id.name}
          style={{ width: 36, height: 36, borderRadius: radius.sm, border: `1px solid ${colors.success}40` }}
        />
        <div style={{ flex: 1 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            <span style={{ fontSize: textSize.body, fontWeight: 600, color: colors.text }}>
              {id.name}
            </span>
            {connectivity !== 'ok' && (
              <span style={{
                width: 6,
                height: 6,
                borderRadius: radius.pill,
                background: connectivityColor(colors, connectivity),
                display: 'inline-block',
                flexShrink: 0,
              }} title={`Chain: ${connectivity}`} />
            )}
          </div>
          <div style={{
            display: 'inline-block',
            padding: '1px 8px',
            borderRadius: radius.xs,
            fontSize: textSize.micro,
            fontWeight: 700,
            letterSpacing: '0.08em',
            background: sealedPill(colors.success).bg,
            color: sealedPill(colors.success).text,
            border: `1px solid ${sealedPill(colors.success).border}`,
            marginTop: 2,
          }}>
            {id.status.toUpperCase()}
          </div>
        </div>
      </div>

      {/* SOUL */}
      <Section title="SOUL" trimColor={colors.success}>
        <StatRow label="DID" value={id.did} />
        <StatRow label="Born" value={formatDate(id.bornAt)} />
        <StatRow label="Owner" value={truncAddr(id.owner)} />
      </Section>

      {/* VERIFICATION */}
      <Section title="VERIFICATION" trimColor={colors.success}>
        <StatRow label="Status" value={id.status.toUpperCase()} />
        <StatRow label="Soul" value={id.soulValid ? 'Valid' : 'Invalid'} />
        <StatRow label="Last verified" value={eligibility.verifiedDaysAgo != null ? `${eligibility.verifiedDaysAgo}d ago` : 'N/A'} />
        <StatRow label="Alignment" value={id.alignmentScore != null ? String(id.alignmentScore) : 'N/A'} />
      </Section>

      {/* ON-CHAIN */}
      <Section title="ON-CHAIN" trimColor={colors.warning}>
        <LinkRow label="SBT" value={`#${id.sbtId}`} href={sbtUrl(id.sbtId)} />
        <LinkRow label="Passport" value={`#${id.passportId}`} href={passportUrl(id.passportId)} />
        <LinkRow label="Chain" value="Base L2" />
        <LinkRow label="Arweave" value={truncAddr(id.arweaveTxId)} href={arweaveUrl(id.arweaveTxId)} />
      </Section>

      {/* PORTAL READINESS */}
      <Section title="PORTAL READINESS" trimColor={colors.cyan}>
        <CheckRow
          label="Sealed"
          pass={eligibility.sealed}
          detail={eligibility.sealed ? 'OK' : 'Required'}
        />
        <CheckRow
          label="Soul valid"
          pass={eligibility.soulValid}
          detail={eligibility.soulValid ? 'OK' : 'Required'}
        />
        <CheckRow
          label="Last verified"
          pass={eligibility.freshlyVerified}
          detail={eligibility.verifiedDaysAgo != null ? `${eligibility.verifiedDaysAgo}d ago (requires <30d)` : 'N/A (requires <30d)'}
          warn={eligibility.verifiedDaysAgo != null && !eligibility.freshlyVerified}
        />
        <CheckRow
          label="Bindings"
          pass={eligibility.hasBindings}
          detail={`${id.bindingsCount} (requires >= 1)`}
        />
        {/* Overall status */}
        <div style={{
          fontSize: textSize.micro,
          fontWeight: 700,
          letterSpacing: '0.06em',
          marginTop: 8,
          color: eligibility.ready ? colors.success : colors.textDim,
        }}>
          {eligibility.ready
            ? 'READY'
            : 'NOT READY — verification actions coming soon'}
        </div>
      </Section>

      {/* ACTIONS */}
      <div style={{ padding: '4px 14px 12px' }}>
        <div style={{
          fontSize: textSize.micro,
          fontWeight: 700,
          letterSpacing: '0.1em',
          color: colors.textDim,
          marginBottom: 6,
        }}>
          ACTIONS
        </div>
        <div style={{ display: 'flex', gap: 8 }}>
          <ActionButton
            label={loading ? '[ Refreshing... ]' : '[ Refresh ]'}
            disabled={loading}
            onClick={handleRefresh}
          />
          <ActionButton
            label="[ BaseScan ]"
            onClick={() => openExternal(sbtUrl(id.sbtId))}
          />
        </div>
      </div>

      {/* Footer: last refreshed */}
      <div style={{
        padding: '4px 14px 8px',
        fontSize: textSize.micro,
        color: colors.textDim,
        textAlign: 'right',
      }}>
        Last refreshed: {relativeMinutes(lastSuccessfulFetch)}
      </div>
    </>
  );
}

// ── Sub-components ───────────────────────────────────────────────

function LinkRow({ label, value, href }: { label: string; value: string; href?: string }) {
  const { colors } = useTheme();
  const [hovered, setHovered] = useState(false);
  const clickable = !!href;

  const handleClick = () => {
    if (href) openExternal(href);
  };

  return (
    <div
      style={{
        display: 'flex',
        justifyContent: 'space-between',
        fontSize: textSize.micro,
        lineHeight: 1.6,
        cursor: clickable ? 'pointer' : 'default',
      }}
      onClick={handleClick}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      <span style={{ color: colors.textMuted }}>{label}</span>
      <span style={{
        color: colors.text,
        fontWeight: 500,
        textDecoration: clickable && hovered ? 'underline' : 'none',
      }}>
        {value}
        {clickable && (
          <span style={{
            color: hovered ? colors.cyan : colors.textDim,
            marginLeft: 4,
            fontSize: textSize.micro,
            transition: `color ${duration.snappy}ms ${ease.snappy}`,
          }}>↗</span>
        )}
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
  const { colors } = useTheme();
  const icon = pass ? '✓' : '✗';
  const color = pass ? colors.success : warn ? colors.warning : colors.dangerStrong;
  return (
    <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: textSize.micro, lineHeight: 1.6 }}>
      <span style={{ color: colors.textMuted }}>
        <span style={{ color, marginRight: 4 }}>{icon}</span>
        {label}
      </span>
      <span style={{ color: colors.textDim, fontSize: textSize.micro, fontWeight: 400 }}>{detail}</span>
    </div>
  );
}

function ActionButton({ label, disabled, onClick }: {
  label: string;
  disabled?: boolean;
  onClick?: () => void;
}) {
  // The muted-to-cyan hover this kept a `hovered` state for is exactly what
  // `--pa-btn-fg-hover` expresses, so the state and its handlers go and the
  // press give and focus ring arrive.
  const { colors } = useTheme();
  return (
    <Button
      colors={colors}
      variant="bare"
      type="button"
      disabled={disabled}
      onClick={onClick}
      style={{
        '--pa-btn-fg': disabled ? colors.textDim : colors.textMuted,
        '--pa-btn-fg-hover': colors.cyan,
        '--pa-btn-bg-hover': colors.fillHover,
        '--pa-btn-bg-active': colors.fillActive,
        '--pa-btn-pad': '5px 0',
        '--pa-btn-radius': '0',
        '--pa-btn-weight': 600,
        flex: 1,
        fontSize: textSize.micro,
        fontFamily: 'monospace',
        letterSpacing: '0.04em',
      } as CSSProperties}
    >
      {label}
    </Button>
  );
}
