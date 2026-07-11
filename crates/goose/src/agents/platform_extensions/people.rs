//! People (CRM) platform extension — lets the agent create people and associate
//! them with projects (people-in-graph v1, #578 / #583).
//!
//! Both tools drive the *one* runtime person path
//! ([`crate::people_create::create_person`]): a person becomes a real graph
//! entity (durable via provenance), mints into the `people` directory, and is
//! associable. `create_person` here passes `Provenance::Runtime` — the same path
//! the UI and the v1.5 Librarian extraction consume, differing only in source.
//!
//! Resolution is explicit, never silent: an ambiguous person/project name returns
//! a disambiguation error listing the candidates rather than guessing wrong.

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::agents::platform_extensions::get_global_brain;
use crate::agents::tool_execution::ToolCallContext;
use crate::people::{self, PeopleFilter, Person};
use crate::people_provenance::Provenance;
use crate::{project_association, projects};
use async_trait::async_trait;
use indoc::indoc;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ServerCapabilities, Tool, ToolAnnotations,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "people";

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct CreatePersonParams {
    /// The person's display name, e.g. "Sabaa Quao" (required).
    display_name: String,
    /// Optional project name, slug, or id to associate the new person with right
    /// after creating them — handles "add X and associate with Y" in one call.
    associate_with_project: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct AssociatePersonParams {
    /// The person's name (resolved against the directory; ambiguous names are
    /// rejected with the list of candidates — the agent should then ask).
    person: String,
    /// The project's name, slug, or id.
    project: String,
    /// Role of the person *within this project* (optional, distinct from their
    /// CRM role).
    project_role: Option<String>,
    /// If the person is not found, create them first, then associate (default
    /// false). Set this for "add X and associate with Y" when X is new.
    create_if_missing: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct EnrichPersonParams {
    /// The person to enrich: canonical name or directory entity_uuid.
    person: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ProposedFieldParam {
    /// One of: linkedin, job_title, company, x_handle, personal_site.
    field_name: String,
    /// The found value.
    value: String,
    /// The URL of the page where the value was verified.
    source_url: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ProposeEnrichmentParams {
    /// The person the findings are for: canonical name or directory entity_uuid.
    person: String,
    /// The proposed field values, each with the source URL it was verified on.
    fields: Vec<ProposedFieldParam>,
}

/// Result of resolving a name to a directory person.
enum PersonMatch {
    One(Box<Person>),
    None,
    Many(Vec<Person>),
}

pub struct PeopleClient {
    info: InitializeResult,
    context: PlatformExtensionContext,
}

impl PeopleClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self, anyhow::Error> {
        let info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(EXTENSION_NAME.to_string(), "1.0.0".to_string())
                    .with_title("People"),
            )
            .with_instructions(
                indoc! {r#"
                Create people and associate them with projects. People are
                graph-authoritative: creating one mints a durable graph entity and
                a CRM directory row in a single, deterministic path.

                Use `create_person` when the user says "add <name>", "create a
                person", or "remember <name> as a contact". Use
                `associate_person_with_project` to link an existing person to a
                project. For "add <name> and associate with <project>", either call
                `create_person` with `associate_with_project`, or call
                `associate_person_with_project` with `create_if_missing: true`.

                Names are resolved against the directory. If a name is ambiguous the
                tool returns the candidates instead of guessing — ask the user which
                one they mean.
            "#}
                .to_string(),
            );
        Ok(Self { info, context })
    }

    async fn pool(&self) -> Result<Pool<Sqlite>, String> {
        self.context
            .session_manager
            .pool_clone()
            .await
            .map_err(|e| e.to_string())
    }

    /// Resolve a name to a directory person: exact (case-insensitive) display-name
    /// match wins; otherwise fall back to the single fuzzy match; anything
    /// ambiguous returns all candidates so the caller can ask.
    async fn resolve_person(pool: &Pool<Sqlite>, name: &str) -> Result<PersonMatch, String> {
        let filter = PeopleFilter {
            company: None,
            role: None,
            query: Some(name.to_string()),
        };
        let candidates = people::list_people(pool, &filter).await?;
        let exact: Vec<Person> = candidates
            .iter()
            .filter(|p| p.display_name.eq_ignore_ascii_case(name))
            .cloned()
            .collect();
        Ok(match exact.len() {
            1 => PersonMatch::One(Box::new(exact.into_iter().next().unwrap())),
            n if n > 1 => PersonMatch::Many(exact),
            _ => match candidates.len() {
                0 => PersonMatch::None,
                1 => PersonMatch::One(Box::new(candidates.into_iter().next().unwrap())),
                _ => PersonMatch::Many(candidates),
            },
        })
    }

    /// Resolve a project by id, slug, or (case-insensitive) name. Ambiguous names
    /// error with the candidate list rather than guessing.
    async fn resolve_project(pool: &Pool<Sqlite>, q: &str) -> Result<projects::Project, String> {
        if let Some(p) = projects::get_project_by_id_or_slug(pool, q).await? {
            return Ok(p);
        }
        let all = projects::list_projects(pool, None).await?;
        let ql = q.to_lowercase();
        let exact: Vec<&projects::Project> = all
            .iter()
            .filter(|p| p.name.to_lowercase() == ql || p.slug.to_lowercase() == ql)
            .collect();
        if exact.len() == 1 {
            return Ok(exact[0].clone());
        }
        if exact.len() > 1 {
            return Err(format!(
                "Project \"{q}\" is ambiguous: {}",
                project_list(&all)
            ));
        }
        let sub: Vec<&projects::Project> = all
            .iter()
            .filter(|p| p.name.to_lowercase().contains(&ql) || p.slug.to_lowercase().contains(&ql))
            .collect();
        match sub.len() {
            1 => Ok(sub[0].clone()),
            0 => Err(format!(
                "No project matches \"{q}\". Available: {}",
                project_list(&all)
            )),
            _ => Err(format!(
                "Project \"{q}\" is ambiguous: {}",
                sub.iter()
                    .map(|p| format!("{} (slug: {})", p.name, p.slug))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    async fn create_person_and_row(
        &self,
        pool: &Pool<Sqlite>,
        name: &str,
    ) -> Result<Person, String> {
        let brain = get_global_brain().ok_or_else(|| {
            "Brain is not available — person creation writes a graph entity and needs the Brain \
             mounted. Cannot create people right now."
                .to_string()
        })?;
        crate::people_create::create_person(pool, &brain, name, Provenance::Runtime).await
    }

    async fn handle_create_person(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: CreatePersonParams = serde_json::from_value(serde_json::Value::Object(args))
            .map_err(|e| format!("Invalid arguments: {e}"))?;
        let pool = self.pool().await?;

        let person = self
            .create_person_and_row(&pool, &params.display_name)
            .await?;

        let mut out = format!(
            "Created person \"{}\" (entity_uuid: {}).",
            person.display_name, person.entity_uuid
        );

        if let Some(project_q) = params.associate_with_project.as_deref() {
            let project = Self::resolve_project(&pool, project_q).await?;
            project_association::associate_person(&pool, &project.id, &person.entity_uuid, None)
                .await?;
            out.push_str(&format!(
                " Associated with project \"{}\" (slug: {}).",
                project.name, project.slug
            ));
        }
        Ok(vec![Content::text(out)])
    }

    async fn handle_associate(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: AssociatePersonParams = serde_json::from_value(serde_json::Value::Object(args))
            .map_err(|e| format!("Invalid arguments: {e}"))?;
        let pool = self.pool().await?;

        // Resolve the project first — a bad project name should not mint a person.
        let project = Self::resolve_project(&pool, &params.project).await?;

        let person = match Self::resolve_person(&pool, &params.person).await? {
            PersonMatch::One(p) => *p,
            PersonMatch::Many(candidates) => {
                return Err(format!(
                    "\"{}\" matches {} people — ask which one: {}",
                    params.person,
                    candidates.len(),
                    candidates
                        .iter()
                        .map(|p| format!(
                            "{} (company: {})",
                            p.display_name,
                            p.company.as_deref().unwrap_or("—")
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            PersonMatch::None => {
                if params.create_if_missing.unwrap_or(false) {
                    self.create_person_and_row(&pool, &params.person).await?
                } else {
                    return Err(format!(
                        "No person named \"{}\" in the directory. Pass create_if_missing: true to \
                         create and associate them, or call create_person first.",
                        params.person
                    ));
                }
            }
        };

        project_association::associate_person(
            &pool,
            &project.id,
            &person.entity_uuid,
            params.project_role.as_deref(),
        )
        .await?;

        Ok(vec![Content::text(format!(
            "Associated \"{}\" with project \"{}\" (slug: {}){}.",
            person.display_name,
            project.name,
            project.slug,
            params
                .project_role
                .as_deref()
                .map(|r| format!(" as {r}"))
                .unwrap_or_default()
        ))])
    }

    /// Resolve `person` as a directory `entity_uuid` first, then by name.
    /// Enrichment must never guess an identity: an ambiguous name errors with
    /// the candidates instead of picking one.
    async fn resolve_person_strict(pool: &Pool<Sqlite>, q: &str) -> Result<Person, String> {
        if let Some(p) = people::get_by_uuid(pool, q).await? {
            return Ok(p);
        }
        match Self::resolve_person(pool, q).await? {
            PersonMatch::One(p) => Ok(*p),
            PersonMatch::None => Err(format!(
                "No person named \"{q}\" in the directory. Create them first with create_person."
            )),
            PersonMatch::Many(candidates) => Err(format!(
                "\"{q}\" matches {} people — ask which one: {}",
                candidates.len(),
                candidates
                    .iter()
                    .map(|p| format!(
                        "{} (company: {})",
                        p.display_name,
                        p.company.as_deref().unwrap_or("—")
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// The Enricher's kickoff (#495 slice 4): resolve the person, show what the
    /// graph already holds (with provenance), and return the research briefing.
    /// This tool does NOT browse — the agent researches with its own web tools
    /// and then calls `propose_enrichment`; nothing is written until the user
    /// approves the resulting Decision Inbox item.
    async fn handle_enrich_person(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: EnrichPersonParams = serde_json::from_value(serde_json::Value::Object(args))
            .map_err(|e| format!("Invalid arguments: {e}"))?;
        let pool = self.pool().await?;
        let person = Self::resolve_person_strict(&pool, &params.person).await?;

        // Current graph-side fields, with provenance, so the agent skips what
        // is already manually set (an Enriched write could never clobber it
        // anyway — Spectral's store enforces that — but proposing it wastes
        // the user's attention).
        let mut current = String::new();
        if let (Some(hex), Some(brain)) = (person.graph_entity_id.as_deref(), get_global_brain()) {
            if let Ok(eid) = hex.parse::<spectral::core::entity_id::EntityId>() {
                if let Ok(fields) = brain.get_entity_fields(eid).await {
                    for f in &fields {
                        current.push_str(&format!(
                            "\n  - {}: {} [{}]",
                            f.field_name,
                            f.value,
                            f.source.as_str()
                        ));
                    }
                }
            }
        }
        if current.is_empty() {
            current = "\n  (none on the graph yet)".to_string();
        }

        Ok(vec![Content::text(format!(
            "Enrichment briefing for \"{}\" (entity_uuid: {}).\n\
             \n\
             Current graph fields (with provenance):{}\n\
             \n\
             Research ONLY these structured fields: {}.\n\
             Manual-only fields are OFF LIMITS — do not research or propose email, phone, \
             birthday, or notes.\n\
             \n\
             How to work:\n\
             1. Use your own web tools (read_webpage, web search) to find this person's \
             public professional presence.\n\
             2. Verify every value on the page you cite — each proposed field needs the \
             URL where you actually saw it.\n\
             3. Skip fields already present with [manual] provenance above.\n\
             4. If you cannot confidently tell WHICH \"{}\" this is (common name, several \
             plausible matches), STOP and report the ambiguity to the user instead of \
             proposing — never guess an identity.\n\
             \n\
             When done, call propose_enrichment with person \"{}\" and your findings as \
             fields: [{{field_name, value, source_url}}]. Nothing is written to the profile \
             until the user approves the proposal in the Decision Inbox.",
            person.display_name,
            person.entity_uuid,
            current,
            crate::people::ENRICHABLE_FIELD_NAMES.join(", "),
            person.display_name,
            person.display_name,
        ))])
    }

    /// File the Enricher's findings as a review-gated Decision Inbox proposal
    /// (kind `enrichment_proposal`). Field names are validated against the
    /// enrichable allowlist here (friendly error) and again at decision
    /// creation (malformed, fail-closed). On approve, the decision's effect
    /// writes the fields with Enriched provenance.
    async fn handle_propose_enrichment(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;
        let params: ProposeEnrichmentParams =
            serde_json::from_value(serde_json::Value::Object(args))
                .map_err(|e| format!("Invalid arguments: {e}"))?;
        let pool = self.pool().await?;
        let person = Self::resolve_person_strict(&pool, &params.person).await?;

        let graph_entity_id = person.graph_entity_id.clone().ok_or_else(|| {
            format!(
                "\"{}\" has no graph entity yet (the people→graph bridge hasn't reached this \
                 row) — enrichment has no write target. Try again after the startup sync.",
                person.display_name
            )
        })?;

        if params.fields.is_empty() {
            return Err("No fields proposed — nothing to review.".to_string());
        }
        for f in &params.fields {
            if !crate::people::ENRICHABLE_FIELD_NAMES.contains(&f.field_name.as_str()) {
                return Err(format!(
                    "Field \"{}\" is not enrichable. Allowed fields: {}. Manual-only fields \
                     (email, phone, birthday, notes) can never be proposed.",
                    f.field_name,
                    crate::people::ENRICHABLE_FIELD_NAMES.join(", ")
                ));
            }
            if f.value.trim().is_empty() {
                return Err(format!("Field \"{}\" has an empty value.", f.field_name));
            }
            if f.source_url.trim().is_empty() {
                return Err(format!(
                    "Field \"{}\" is missing its source_url — every finding must cite the \
                     page it was verified on.",
                    f.field_name
                ));
            }
        }

        let payload = crate::decisions::EnrichmentProposalPayload {
            person_name: person.display_name.clone(),
            graph_entity_id,
            entity_uuid: Some(person.entity_uuid.clone()),
            fields: params
                .fields
                .iter()
                .map(|f| crate::decisions::ProposedEnrichmentField {
                    field_name: f.field_name.clone(),
                    value: f.value.trim().to_string(),
                    source_url: f.source_url.trim().to_string(),
                })
                .collect(),
        };

        let mut headline = format!("Approve enriched details for {}", person.display_name);
        if headline.chars().count() > 80 {
            headline = headline.chars().take(79).collect::<String>() + "…";
        }
        let detail = params
            .fields
            .iter()
            .map(|f| {
                format!(
                    "{}: {} (source: {})",
                    f.field_name,
                    f.value.trim(),
                    f.source_url.trim()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let decision = crate::decisions::create_decision(
            &pool,
            crate::decisions::NewDecision {
                kind: "enrichment_proposal".to_string(),
                headline: Some(headline),
                detail: Some(detail),
                payload: serde_json::to_value(&payload).map_err(|e| e.to_string())?,
                ..Default::default()
            },
        )
        .await?;

        if decision.kind == "malformed" {
            return Err(format!(
                "The proposal was rejected as malformed: {}",
                decision.detail
            ));
        }

        Ok(vec![Content::text(format!(
            "Proposed {} field(s) for \"{}\" — decision {} is waiting in the Decision Inbox. \
             Nothing is written to the profile until the user approves it there; approved \
             fields are stored with Enriched provenance and can never overwrite a manually \
             entered value.",
            params.fields.len(),
            person.display_name,
            decision.id
        ))])
    }

    fn get_tools() -> Vec<Tool> {
        let create_schema = serde_json::to_value(schema_for!(CreatePersonParams)).unwrap();
        let associate_schema = serde_json::to_value(schema_for!(AssociatePersonParams)).unwrap();
        vec![
            Tool::new(
                "create_person".to_string(),
                indoc! {r#"
                Create a person in the CRM. Mints a durable graph entity plus a
                directory row in one deterministic step. Optionally associate them
                with a project in the same call via associate_with_project. Use when
                the user asks to add or remember a person/contact.
            "#}
                .to_string(),
                create_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Create Person".to_string()),
                Some(false),
                Some(true),
                Some(false),
                Some(false),
            )),
            Tool::new(
                "associate_person_with_project".to_string(),
                indoc! {r#"
                Associate a person with a project. Resolves both by name; an
                ambiguous person name returns the candidates so you can ask. Set
                create_if_missing: true to create the person first when they are new
                ("add X and associate with Y").
            "#}
                .to_string(),
                associate_schema.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Associate Person With Project".to_string()),
                Some(false),
                Some(true),
                Some(false),
                Some(false),
            )),
            Tool::new(
                "enrich_person".to_string(),
                indoc! {r#"
                Start an enrichment pass on a CRM person (the Enricher, #495).
                Returns a research briefing: the person's current graph fields
                with provenance and the bounded set of enrichable fields
                (linkedin, job_title, company, x_handle, personal_site). You do
                the research yourself with your web tools, then file findings
                via propose_enrichment. Manual-only fields (email, phone,
                birthday, notes) are off limits. Use when the user asks to
                enrich, refresh, or look up a contact's professional details.
            "#}
                .to_string(),
                serde_json::to_value(schema_for!(EnrichPersonParams))
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Enrich Person".to_string()),
                Some(true),
                Some(false),
                Some(true),
                Some(false),
            )),
            Tool::new(
                "propose_enrichment".to_string(),
                indoc! {r#"
                File researched person details as a review-gated proposal in the
                Decision Inbox. Each field needs {field_name, value, source_url}
                where source_url is the page the value was verified on. Only the
                enrichable fields (linkedin, job_title, company, x_handle,
                personal_site) are accepted. NOTHING is written to the person's
                profile until the user approves the proposal; approved fields
                are stored with Enriched provenance and never overwrite
                manually entered values.
            "#}
                .to_string(),
                serde_json::to_value(schema_for!(ProposeEnrichmentParams))
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .annotate(ToolAnnotations::from_raw(
                Some("Propose Enrichment".to_string()),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            )),
        ]
    }
}

/// Compact project list for disambiguation errors.
fn project_list(all: &[projects::Project]) -> String {
    all.iter()
        .map(|p| format!("{} (slug: {})", p.name, p.slug))
        .collect::<Vec<_>>()
        .join(", ")
}

#[async_trait]
impl McpClientTrait for PeopleClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: Self::get_tools(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        _ctx: &ToolCallContext,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let content = match name {
            "create_person" => self.handle_create_person(arguments).await,
            "associate_person_with_project" => self.handle_associate(arguments).await,
            "enrich_person" => self.handle_enrich_person(arguments).await,
            "propose_enrichment" => self.handle_propose_enrichment(arguments).await,
            _ => Err(format!("Unknown tool: {name}")),
        };
        match content {
            Ok(content) => Ok(CallToolResult::success(content)),
            Err(error) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Error: {error}"
            ))])),
        }
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}
