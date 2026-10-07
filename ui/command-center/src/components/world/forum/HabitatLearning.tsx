import { useState } from 'react';
import { Button } from '../../common/Button';
import { useTheme } from '../../../styles/useTheme';

type LessonId = 'arrival' | 'archive' | 'water';

/** Three small, fictional demonstrations. They never call the daemon, create a
 * task, change a goal, or imply that an environmental result is scientific. */
export function HabitatLearning() {
  const { colors } = useTheme();
  const [lesson, setLesson] = useState<LessonId>('arrival');
  const [arrivalChoice, setArrivalChoice] = useState<number | null>(null);
  const [archiveChoice, setArchiveChoice] = useState<number | null>(null);
  const [waterSetting, setWaterSetting] = useState<'shade' | 'flow' | null>(null);
  const [waterPrediction, setWaterPrediction] = useState<number | null>(null);
  const [waterInspected, setWaterInspected] = useState(false);
  const reset = () => {
    setArrivalChoice(null); setArchiveChoice(null); setWaterSetting(null);
    setWaterPrediction(null); setWaterInspected(false);
  };
  const chooseLesson = (next: LessonId) => { setLesson(next); reset(); };
  return <section className="garden-lesson" aria-label="World learning demonstrations">
    <div className="eyebrow">FIELD GUIDE · DEMONSTRATIONS</div>
    <h3>Learn by inspecting a small example</h3>
    <p>These fictional demonstrations explain evidence and feedback. Nothing here submits a task, approval, trade, or job.</p>
    <nav className="lesson-steps" aria-label="World demonstrations">
      <Button colors={colors} flashSuccess={false} aria-pressed={lesson === 'arrival'} onClick={() => chooseLesson('arrival')}>Arrival table</Button>
      <Button colors={colors} flashSuccess={false} aria-pressed={lesson === 'archive'} onClick={() => chooseLesson('archive')}>Garden archive</Button>
      <Button colors={colors} flashSuccess={false} aria-pressed={lesson === 'water'} onClick={() => chooseLesson('water')}>Water workshop</Button>
    </nav>
    {lesson === 'arrival' && <>
      <h4>Plan against constraints</h4>
      <p>A demonstration brief asks for a walkable garden guide by Friday, with cited sources and no changes to live work. Which plan is better?</p>
      <Button colors={colors} flashSuccess={false} aria-pressed={arrivalChoice === 0} onClick={() => setArrivalChoice(0)}>Confirm outcome, deadline, evidence, and limits first</Button>
      <Button colors={colors} flashSuccess={false} aria-pressed={arrivalChoice === 1} onClick={() => setArrivalChoice(1)}>Start immediately and clarify constraints later</Button>
      {arrivalChoice !== null && <div className="note" role="status"><b>{arrivalChoice === 0 ? 'Supported plan.' : 'A missing constraint changes the plan.'}</b><p>Outcome, deadline, evidence, and limits make the next step reviewable. This demonstration is based on the brief shown above; it does not inspect or send a real request.</p><p>Source: the fictional brief and its stated constraints.</p></div>}
    </>}
    {lesson === 'archive' && <>
      <h4>Choose the supported claim</h4>
      <p>Inspect two fictional sources before answering which venue room has a confirmed step-free route.</p>
      <details open><summary>Open the records</summary><p><b>Record A · 2019 brochure:</b> “Every meeting room is accessible.” It does not describe individual routes.</p><p><b>Record B · dated 2026 venue note:</b> the Garden room has a step-free entrance; the Observatory lift is closed and an alternative route is unconfirmed.</p></details>
      <Button colors={colors} flashSuccess={false} aria-pressed={archiveChoice === 0} onClick={() => setArchiveChoice(0)}>Every meeting room is confirmed step-free</Button>
      <Button colors={colors} flashSuccess={false} aria-pressed={archiveChoice === 1} onClick={() => setArchiveChoice(1)}>Garden room confirmed; Observatory uncertain</Button>
      {archiveChoice !== null && <div className="note" role="status"><b>{archiveChoice === 1 ? 'Supported by the newer record.' : 'The older source is too broad.'}</b><p>Prefer the dated venue note, state the uncertainty, and ask for confirmation before recommending the Observatory.</p><p>Sources: fictional 2019 brochure and fictional dated 2026 venue note.</p></div>}
    </>}
    {lesson === 'water' && <>
      <h4>Change, predict, inspect</h4>
      <p>Choose one bounded illustrative setting, predict what its model will show, then inspect the result. This is not a scientific forecast.</p>
      <div role="group" aria-label="Illustrative water setting"><Button colors={colors} flashSuccess={false} aria-pressed={waterSetting === 'shade'} onClick={() => { setWaterSetting('shade'); setWaterPrediction(null); setWaterInspected(false); }}>Add shade</Button><Button colors={colors} flashSuccess={false} aria-pressed={waterSetting === 'flow'} onClick={() => { setWaterSetting('flow'); setWaterPrediction(null); setWaterInspected(false); }}>Increase flow</Button></div>
      {waterSetting && <><p>Setting selected: <b>{waterSetting === 'shade' ? 'add shade' : 'increase flow'}</b>. What do you predict?</p><Button colors={colors} flashSuccess={false} aria-pressed={waterPrediction === 0} onClick={() => setWaterPrediction(0)}>{waterSetting === 'shade' ? 'The illustrative surface warms more slowly' : 'The illustrative cascade refreshes more quickly'}</Button><Button colors={colors} flashSuccess={false} aria-pressed={waterPrediction === 1} onClick={() => setWaterPrediction(1)}>{waterSetting === 'shade' ? 'The illustrative flow becomes faster' : 'The illustrative surface becomes still'}</Button></>}
      {waterPrediction !== null && <><Button colors={colors} variant="primary" flashSuccess={false} onClick={() => setWaterInspected(true)}>Inspect the illustrative result</Button>{waterInspected && <div className="note" role="status"><b>{waterPrediction === (waterSetting === 'shade' ? 0 : 0) ? 'Prediction matches the stated model.' : 'Compare the prediction with the stated model.'}</b><p>{waterSetting === 'shade' ? 'In this bounded example, shade changes the modeled warming rate; it does not claim a measured environmental result.' : 'In this bounded example, flow changes the modeled refresh rate; it does not claim a measured environmental result.'}</p><p>Source: the water workshop’s explicit illustrative assumptions and the visible model settings.</p></div>}</>}
    </>}
    <Button colors={colors} variant="ghost" flashSuccess={false} onClick={reset}>Revisit this demonstration</Button>
  </section>;
}
