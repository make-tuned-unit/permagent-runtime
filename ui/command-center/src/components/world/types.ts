import type { Vector3Tuple } from 'three';

export interface AgentState {
  id: string;
  name: string;
  role: 'orchestrator' | 'agent';
  position: { x: number; y: number; z: number };
  activity: 'idle' | 'walking' | 'working' | 'thinking';
  currentStation: string | null;
  togaTrimColor: string;
  isHenry: boolean;
}

export interface StationConfig {
  id: string;
  name: string;
  position: Vector3Tuple;
  iconType: 'gear' | 'scroll' | 'planets' | 'portal' | 'rings';
  tooltip: string;
  disabled?: boolean;
}

export type CameraMode = 'orbit' | 'third-person';

export interface WorldContextState {
  cameraMode: CameraMode;
  hoveredAgent: string | null;
  hoveredStation: string | null;
  selectedAgent: string | null;
  showFps: boolean;
}
