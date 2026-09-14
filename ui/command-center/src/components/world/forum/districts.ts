import type { ToolType } from '../../../lib/store';
import { ENV } from '../shared/palette';
export const DISTRICTS = [
  { id: 'commons', name: 'Commons', subtitle: 'Meet your agents', description: 'A shared place to understand each collaborator and prepare a clear request.', position: [0, 0, 0], agent: 'henry', accent: ENV.neonCyan, tool: 'world' },
  { id: 'gallery', name: 'Gallery', subtitle: 'The upper archive', description: 'A raised garden walkway connects the archive, reading places and the observatory above the commons.', position: [0, 4.32, -20], agent: 'librarian', accent: ENV.violet, tool: 'memory' },
  { id: 'build', name: 'Build', subtitle: 'Make and verify', description: 'Turn a project brief into work, then inspect changes, tests and the finished artifact.', position: [-14, 0, 0], agent: 'steward', accent: ENV.neonAmber, tool: 'build' },
  { id: 'brain', name: 'Brain', subtitle: 'Memory with sources', description: 'Explore what your agents remember, where it came from, and what should be corrected.', position: [14, 0, 0], agent: 'librarian', accent: ENV.violet, tool: 'memory' },
  { id: 'automate', name: 'Automate', subtitle: 'Work over time', description: 'Understand a scheduled job: its trigger, scope, last result and next run.', position: [-10, 0, -12], agent: 'watcher', accent: ENV.neonAmber, tool: 'automate' },
  // The gate court on the 135-degree axis (jobs/13-terrace-gate-report.md).
  // `tool: null` is load-bearing: the Mesh has no tab and no panel to open —
  // the only thing behind it is the honest status in `shared/meshStatus.ts`.
  { id: 'mesh', name: 'Mesh gate', subtitle: 'The gate to the MESH', description: 'Walk through the ring to reach the Mesh Agora. Shows real connection state only.', position: [-39.6, 0, -39.6], agent: 'council', accent: ENV.horizonBlue, tool: null },
  { id: 'grove', name: 'Reading grove', subtitle: 'A quiet place to understand', description: 'Walk through the eastern gardens to a shaded reading court. Bring a source and a question.', position: [55, 0, 0], agent: 'reader', accent: ENV.violet, tool: 'memory' },
  { id: 'maker', name: 'Maker court', subtitle: 'Work made tangible', description: 'A western workshop court for inspecting plans, prototypes and the evidence of a finished job.', position: [-55, 0, 0], agent: 'steward', accent: ENV.neonAmber, tool: 'build' },
  { id: 'council_garden', name: 'Council garden', subtitle: 'Room for different perspectives', description: 'A distant northern court where plans can be compared, discussed and refined.', position: [0, 0, -55], agent: 'council', accent: ENV.horizonBlue, tool: null },
  { id: 'arrival', name: 'Arrival gardens', subtitle: 'The long approach', description: 'An open garden approach connected to the commons by a bridge and the planted promenade.', position: [0, 0, 55], agent: 'henry', accent: ENV.neonCyan, tool: 'world' },
  { id: 'conservatory', name: 'Conservatory', subtitle: 'A living seed archive', description: 'A vaulted conservatory opens onto the outer garden circuit, with a long central nave and planted work bays.', position: [116,0,0], agent: 'growth_measurement', accent: ENV.neonCyan, tool: 'grow' },
  { id: 'maker_hall', name: 'Maker hall', subtitle: 'The long workshop', description: 'An open timber-framed workshop with a mezzanine, work benches and a view across the gardens.', position: [-116,0,0], agent: 'steward', accent: ENV.neonAmber, tool: 'build' },
  { id: 'theatre', name: 'Debating theatre', subtitle: 'A landscape for discussion', description: 'Tiered stone seating and radial aisles frame a shared stage and a view back to the forum.', position: [0,0,-115], agent: 'council', accent: ENV.horizonBlue, tool: null },
  { id: 'harbor', name: 'Harbor', subtitle: 'The coastal arrival', description: 'A monumental gate leads toward the timber quay and an open horizon beyond the coastal gardens.', position: [0,0,118], agent: 'henry', accent: ENV.neonCyan, tool: 'world' },
] satisfies Array<{ id: string; name: string; subtitle: string; description: string; position: [number,number,number]; agent: string; accent: string; tool: ToolType | null }>;
export type ForumDistrict = typeof DISTRICTS[number];
