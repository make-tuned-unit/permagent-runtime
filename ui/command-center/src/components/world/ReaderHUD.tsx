import { COLORS } from './constants';
import { AGENT_TRIM } from './shared/palette';
import { HudShell, Section } from './HudShell';
import { Chip } from '../common/Chip';
import { space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

// The Reader — the local OCR / document-ingest pipeline (#336/#342). Unlike Henry and
// the Librarian, the Reader has no live status endpoint yet (its state is sim-ambient
// in-world); a real reader-event wire is a tracked follow-up. So this HUD is an honest
// capability panel rather than a metrics dashboard — it explains what the Reader is and
// how to feed it, without fabricating numbers.

const READER_TRIM = AGENT_TRIM.reader; // cool teal

interface ReaderHUDProps {
  visible: boolean;
  onClose: () => void;
}

export function ReaderHUD({ visible, onClose }: ReaderHUDProps) {
  const { colors } = useTheme();
  if (!visible) return null;

  // Static, and now drawn as such. "LOCAL" is a fact about where the Reader
  // runs — true whether or not anything is being read — but it used to render
  // pixel-identically to the Steward's "SWEEPING", which is a claim that work
  // is in flight right now. The engineering was always honest here; only the
  // presentation lied.
  const statusPill = (
    <Chip
      kind="static"
      color={READER_TRIM}
      title="Where the Reader runs — a capability, not a live status"
    >
      LOCAL
    </Chip>
  );

  return (
    <HudShell visible={visible} onClose={onClose} title="THE READER" statusPill={statusPill}>
      <div style={{ padding: `${space.xs}px 14px ${space.md}px` }}>
        <span style={{ fontSize: textSize.micro, color: colors.textMuted, lineHeight: 1.5 }}>
          On-device OCR &amp; document ingest — turns dropped files into Brain memories,
          entirely on this machine.
        </span>
      </div>

      <Section title="READS" trimColor={READER_TRIM}>
        <Bullet>Images → on-device Vision OCR</Bullet>
        <Bullet>PDFs → text extraction (lopdf)</Bullet>
        <Bullet>Text &amp; code → direct ingest</Bullet>
      </Section>

      <Section title="HOW TO FEED IT" trimColor={COLORS.neonAmber}>
        <span style={{ fontSize: textSize.micro, color: colors.text, lineHeight: 1.5 }}>
          Drag a file onto the window. The Reader extracts its text locally and hands
          the digest to the Brain — nothing leaves the machine.
        </span>
      </Section>
    </HudShell>
  );
}

function Bullet({ children }: { children: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <div style={{ fontSize: textSize.micro, color: colors.text, lineHeight: 1.7, display: 'flex', gap: space.md }}>
      <span style={{ color: READER_TRIM }}>·</span>
      <span>{children}</span>
    </div>
  );
}
