/**
 * Listening / thinking cues for the orchestrator avatar (V-36), World style:
 * posture only, nothing glowing or loud.
 *   recording  -> listening: head tilts a little toward the user and lowers
 *                 slightly, eyes stay open and steady.
 *   processing -> thinking: gaze drifts up and aside, eyes narrow a touch.
 * With reduced motion the cue is a held pose with no oscillation.
 */
export interface AgentCue {
  yaw: number;
  pitch: number;
  roll: number;
  /** Multiplier for eye height (blink excluded). */
  eyeScaleY: number;
}

export const NO_CUE: AgentCue = { yaw: 0, pitch: 0, roll: 0, eyeScaleY: 1 };

export function avatarCue(state: string | undefined, t: number, reduced: boolean): AgentCue {
  if (state === 'recording') {
    return {
      yaw: 0,
      pitch: -0.05 + (reduced ? 0 : Math.sin(t * 1.3) * 0.012),
      roll: 0.1,
      eyeScaleY: 1,
    };
  }
  if (state === 'processing') {
    return {
      yaw: 0.2,
      pitch: 0.1,
      roll: 0,
      eyeScaleY: reduced ? 0.82 : 0.82 + Math.sin(t * 1.1) * 0.04,
    };
  }
  return NO_CUE;
}
