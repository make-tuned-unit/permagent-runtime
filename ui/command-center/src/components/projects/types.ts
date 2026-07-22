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
  /**
   * General project metadata bag (schema v26, camelCase `metadataJson` on the
   * wire). Shared by multiple features — the Overview's `brief` + `links` keys
   * (#472) live alongside foreign keys like `build_command` (#456), so writes
   * must MERGE, never replace blindly (see workspaceMeta.ts).
   */
  metadataJson: Record<string, unknown>;
  createdAt: string;
  updatedAt: string;
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

/**
 * A document attached to a project (#471 Layer 2,
 * `GET /api/projects/{id}/documents`). Serialized **snake_case** (the backend
 * `ProjectDocument` struct carries no `rename_all`) — match the wire exactly.
 */
export interface ProjectDocument {
  id: string;
  project_id: string;
  filename: string;
  mime_type: string;
  size_bytes: number;
  path: string;
  uploaded_at: string;
}

/**
 * A freeform note on a project (`GET /api/projects/{id}/notes`). Serialized
 * **snake_case** (the backend `ProjectNote` struct carries no `rename_all`) —
 * match the wire exactly. `title` and `memory_key` are nullable.
 */
export interface ProjectNote {
  id: string;
  project_id: string;
  title: string | null;
  body: string;
  memory_key: string | null;
  created_at: string;
  updated_at: string;
}

/**
 * A stack-organizer entry (#512, `GET /api/projects/{id}/stack`): one service
 * the project is built on + WHICH login identity is used for it. Serialized
 * **snake_case** (the backend `StackEntry` struct carries no `rename_all`) —
 * match the wire exactly. REFERENCE-ONLY: `identity` is the account label
 * (email/handle), never a password/secret — the backend has no field for one
 * and rejects unknown fields.
 */
export interface StackEntry {
  id: string;
  project_id: string;
  service_name: string;
  category: StackCategory;
  identity: string | null;
  notes: string;
  dashboard_url: string | null;
  created_at: string;
  updated_at: string;
}

/** Valid stack-entry categories (mirrors the backend CHECK constraint). */
export const STACK_CATEGORIES = [
  'hosting',
  'database',
  'backend',
  'auth',
  'analytics',
  'social',
  'domain',
  'other',
] as const;

export type StackCategory = (typeof STACK_CATEGORIES)[number];

/** Display labels for stack categories, in display order. */
export const STACK_CATEGORY_LABELS: Record<StackCategory, string> = {
  hosting: 'Hosting',
  database: 'Database',
  backend: 'Backend',
  auth: 'Auth',
  analytics: 'Analytics',
  social: 'Social',
  domain: 'Domain',
  other: 'Other',
};

/**
 * A Brain memory associated with a project (`GET /api/projects/{id}/memories`).
 * Resolved from the LIVE Brain (`memory.db`) — content/description reflect the
 * current memory, not a stale copy. Serialized **snake_case** (the backend
 * `ProjectMemory` struct carries no `rename_all`) — match the wire exactly.
 * `id` is the Spectral memory id; `key` is the stable memory key.
 */
export interface ProjectMemory {
  id: string;
  key: string;
  content: string;
  description: string | null;
  signal_score: number;
  created_at: string;
  associated_at: string;
}

/** The implicit Personal project — undeletable, can't change status. */
export const PERSONAL_ID = '00000000-0000-0000-0000-000000000001';

/** Goal lifecycle states a goal can still be cancelled from (#490); Failed
 *  goals (#250) can be abandoned too. */
export const CANCELLABLE_STATES = ['triage', 'ready', 'in_progress', 'review', 'failed'];

/** Which Projects-tab lens is showing for the selected project. */
export type ProjectLens = 'overview' | 'kanban';
