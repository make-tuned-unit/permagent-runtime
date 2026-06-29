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

/**
 * A CRM person associated with a project (#530 `GET /api/projects/{id}/people`).
 *
 * NOTE: the People/CRM endpoints serialize **snake_case** (the backend `Person`
 * struct carries no `rename_all`, and `ProjectPerson` flattens it), unlike the
 * camelCase Project/Card shapes above. These field names match the wire exactly
 * — do not camelCase them.
 */
export interface ProjectPerson {
  entity_uuid: string;
  canonical_id: string;
  display_name: string;
  role: string | null;
  company: string | null;
  email: string | null;
  phone: string | null;
  notes: string | null;
  last_contact_at: string | null;
  created_at: string;
  updated_at: string;
  /** Role within *this* project (project_people.role), distinct from CRM role. */
  project_role: string | null;
  /** When the association was created (project_people.added_at). */
  associated_at: string;
}

/** A bare CRM person (#530 `GET /api/people`), used by the associate picker. */
export type Person = Omit<ProjectPerson, 'project_role' | 'associated_at'>;

/** The implicit Personal project — undeletable, can't change status. */
export const PERSONAL_ID = '00000000-0000-0000-0000-000000000001';

/** Goal lifecycle states a goal can still be cancelled from (#490). */
export const CANCELLABLE_STATES = ['triage', 'ready', 'in_progress', 'review'];

/** Which Projects-tab lens is showing for the selected project. */
export type ProjectLens = 'overview' | 'kanban';
