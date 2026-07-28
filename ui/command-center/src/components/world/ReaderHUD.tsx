import { COLORS } from './constants';
import { AGENT_TRIM } from './shared/palette';
import { HudShell, Section } from './HudShell';

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
  if (!visible) return null;

  const statusPill = (
    <div style={{
      display: 'inline-block',
      padding: '2px 8px',
      borderRadius: 3,
      fontSize: 10,
      fontWeight: 700,
      letterSpacing: '0.08em',
      background: 'rgba(79, 209, 197, 0.14)',
      color: READER_TRIM,
      border: `1px solid ${READER_TRIM}55`,
    }}>
      LOCAL
    </div>
  );

  return (
    <HudShell visible={visible} onClose={onClose} title="THE READER" statusPill={statusPill}>
      <div style={{ padding: '4px 14px 8px' }}>
        <span style={{ fontSize: 11, color: '#9CA3AF', lineHeight: 1.5 }}>
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
        <span style={{ fontSize: 11, color: '#D1D5DB', lineHeight: 1.5 }}>
          Drag a file onto the window. The Reader extracts its text locally and hands
          the digest to the Brain — nothing leaves the machine.
        </span>
      </Section>
    </HudShell>
  );
}

function Bullet({ children }: { children: React.ReactNode }) {
  return (
    <div style={{ fontSize: 11, color: '#D1D5DB', lineHeight: 1.7, display: 'flex', gap: 8 }}>
      <span style={{ color: READER_TRIM }}>·</span>
      <span>{children}</span>
    </div>
  );
}
