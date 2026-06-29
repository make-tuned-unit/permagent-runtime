// Shared types + constants for the Projects surface (Overview + Kanban lenses).

export interface Project {
  id: string;
  slug: string;
  name: string;
  description: string;
  status: string;
  rootPath: string | null;
  siteUrl: string | null;
  repoUrl: string | null;
  tags: string[];
  lastOpenedAt: string;
}

export interface BoardColumn {
  id: string;
  projectId: string;
  name: string;
  position: number;
  columnKind: string;
  stateBinding?: string | null;
  wipLimit: number | null;
}

export interface Card {
  id: string;
  projectId: string;
  cardType: string;
  title: string;
  description: string;
  columnId: string;
  position: number;
  createdBy: string;
  assignedTo: string | null;
  metadataJson: Record<string, unknown>;
  createdAt: string;
  updatedAt: string;
  archivedAt: string | null;
}

/** The implicit Personal project — undeletable, can't change status. */
export const PERSONAL_ID = '00000000-0000-0000-0000-000000000001';

/** Goal lifecycle states a goal can still be cancelled from (#490). */
export const CANCELLABLE_STATES = ['triage', 'ready', 'in_progress', 'review'];

/** Which Projects-tab lens is showing for the selected project. */
export type ProjectLens = 'overview' | 'kanban';
