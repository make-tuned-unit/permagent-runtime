import { useState } from 'react';
import { Button } from '../../common/Button';
import { useTheme } from '../../../styles/useTheme';

/** Fixed fictional records; this exercise never sends a request or changes a job. */
export function EvidenceLesson() {
  const { colors } = useTheme();
  const [choice, setChoice] = useState<number | null>(null);
  return <section className="garden-lesson" aria-label="Completion evidence demonstration">
    <div className="eyebrow">EVIDENCE ARCHIVE · DEMONSTRATION</div>
    <h3>Which answer is supported?</h3>
    <p>An example request asks: “Which meeting room can my guest reach without using stairs? Use the venue notes and flag uncertainty.” These records are fictional.</p>
    <details open><summary>Inspect the two records</summary>
      <p><b>Record A · older brochure</b><br />“Every meeting room is accessible.” The brochure is from 2019 and does not describe individual routes.</p>
      <p><b>Record B · current venue note</b><br />A dated 2026 venue note confirms a step-free entrance to the Garden room. The Observatory lift is closed for maintenance; an alternative route has not been confirmed.</p>
    </details>
    <p>What should the agent report?</p>
    <Button colors={colors} flashSuccess={false} aria-pressed={choice === 0} onClick={() => setChoice(0)}>Every meeting room is step-free</Button>
    <Button colors={colors} flashSuccess={false} aria-pressed={choice === 1} onClick={() => setChoice(1)}>Garden room confirmed; Observatory uncertain</Button>
    {choice !== null && <div className="note" role="status"><b>{choice === 1 ? 'Supported by the records.' : 'The newer record supports a narrower claim.'}</b><p>Recommend the Garden room and cite the dated venue note. Do not repeat the older brochure’s broad claim when newer evidence identifies an exception. Ask the venue to confirm an alternative route before recommending the Observatory.</p><p>This is an illustrative example, not advice about a real venue or a report about your jobs.</p><a href="https://ies.ed.gov/ncee/WWC/PracticeGuide/1" target="_blank" rel="noreferrer">Learning design reference · IES practice guide</a></div>}
    <Button colors={colors} variant="ghost" flashSuccess={false} onClick={() => setChoice(null)}>Revisit the question</Button>
  </section>;
}
