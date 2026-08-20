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
  /**
   * #495 manual-only CRM fields (graph-`entity_fields`-only, no DB column).
   * Populated by the graph overlay; written via `PATCH /api/people/{id}/fields`.
   */
  birthday: string | null;
  relationship_strength: string | null;
  how_met: string | null;
  /**
   * Profile links (#495 slice 4 Enricher vocabulary, surfaced 2026-08-19).
   * Also graph-only. The Enricher had been writing these since slice 4 while
   * the backend overlay had no mapping for them, so they never reached the
   * wire; they are readable now and `linkedin` is manually editable too.
   * `facebook` / `instagram` joined the same URL-shaped set on 2026-08-20.
   */
  linkedin: string | null;
  x_handle: string | null;
  facebook: string | null;
  instagram: string | null;
  personal_site: string | null;
  /**
   * Public headshot URL for the People graph face. Direct http(s) image,
   * graph-only, enrichable from a public page.
   */
  photo_url: string | null;
  /**
   * Manual-only hint written when an enrichment proposal is rejected.
   * Helps the next enrich pass find this person online (company, LinkedIn,
   * city). Graph-`entity_fields`-only, same overlay as the other CRM extras.
   */
  find_online_hints: string | null;
  created_at: string;
  updated_at: string;
  /** Role within *this* project (project_people.role), distinct from CRM role. */
  project_role: string | null;
  /** When the association was created (project_people.added_at). */
  associated_at: string;
}

/** A bare CRM person (#530 `GET /api/people`), used by the associate picker. */
export type Person = Omit<ProjectPerson, 'project_role' | 'associated_at'>;

/** The project-association fields, carried separately now that a person can be
 *  viewed outside any project (the directory). Null there, set by PeoplePanel. */
export interface PersonAssociation {
  project_role: string | null;
  associated_at: string;
}

/** A project a person belongs to (`GET /api/people/{id}/projects`). */
export interface PersonProject {
  project_id: string;
  project_name: string;
  project_status: string;
  role: string | null;
  added_at: string;
}

/** The minimal project reference a directory row renders as a chip. */
export interface ProjectRef {
  project_id: string;
  project_name: string;
}

/**
 * A row of `GET /api/people/directory` — every person, with the projects they
 * belong to. `projects` is empty for the cohort the directory exists to reach:
 * people with no association at all, invisible to every project-scoped surface.
 */
export type DirectoryPerson = Person & {
  projects: ProjectRef[];
  next_follow_up_at?: string | null;
};

export interface PersonRelationship {
  from_entity_uuid: string;
  to_entity_uuid: string;
  predicate: string;
  other_person: Person;
}

export interface PersonActivity {
  id: string;
  kind: 'memory' | 'note' | 'task' | 'meeting';
  title: string;
  detail: string;
  timestamp: string;
}

export interface PersonMeeting {
  id: string;
  entity_uuid: string;
  title: string;
  starts_at: string;
  ends_at: string | null;
  notes: string;
  calendar_synced: boolean;
  project_id: string | null;
  follow_up_at: string | null;
  follow_up_note: string;
  follow_up_done: boolean;
  calendar_uid: string | null;
  created_at: string;
  updated_at: string;
}

export interface NamedPersonMeeting {
  id: string;
  entity_uuid: string;
  display_name: string;
  title: string;
  starts_at: string;
  ends_at: string | null;
  notes: string;
  calendar_synced: boolean;
  project_id: string | null;
  follow_up_at: string | null;
  follow_up_note: string;
  follow_up_done: boolean;
}

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
  export_path?: string | null;
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
export type ProjectLens = 'overview' | 'details' | 'kanban';
