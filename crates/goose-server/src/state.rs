use axum::http::StatusCode;
use permagent::builtin_extension::register_builtin_extensions;
use permagent::execution::manager::AgentManager;
use permagent::scheduler_trait::SchedulerTrait;
use permagent::session::SessionManager;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::session_event_bus::SessionEventBus;
use crate::tunnel::TunnelManager;
use permagent::agents::ExtensionLoadResult;
use permagent::gateway::manager::GatewayManager;
#[cfg(feature = "local-inference")]
use permagent::providers::local_inference::InferenceRuntime;

type ExtensionLoadingTasks =
    Arc<Mutex<HashMap<String, Arc<Mutex<Option<JoinHandle<Vec<ExtensionLoadResult>>>>>>>>;

/// In-memory credentials scoped to browser event streams. These tokens are
/// deliberately not consulted by the bearer middleware for protected routes.
#[derive(Clone, Default)]
pub struct StreamTokenStore {
    entries: Arc<StdMutex<HashMap<String, Instant>>>,
}

impl StreamTokenStore {
    pub fn insert(&self, token: String, expires_at: Instant) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        entries.retain(|_, expiry| *expiry > now);
        entries.insert(token, expires_at);
    }

    pub fn contains_unexpired(&self, provided: &str) -> bool {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        entries.retain(|_, expiry| *expiry > now);

        // Do not use HashMap::contains_key for a secret. Scan all live entries
        // and compare every candidate in constant time.
        let mut matched = subtle::Choice::from(0);
        for token in entries.keys() {
            matched |= subtle::ConstantTimeEq::ct_eq(token.as_bytes(), provided.as_bytes());
        }
        bool::from(matched)
    }
}

#[derive(Clone)]
pub struct AppState {
    pub(crate) agent_manager: Arc<AgentManager>,
    pub recipe_file_hash_map: Arc<Mutex<HashMap<String, PathBuf>>>,
    recipe_session_tracker: Arc<Mutex<HashSet<String>>>,
    pub tunnel_manager: Arc<TunnelManager>,
    pub gateway_manager: Arc<GatewayManager>,
    pub extension_loading_tasks: ExtensionLoadingTasks,
    #[cfg(feature = "local-inference")]
    pub inference_runtime: Arc<InferenceRuntime>,
    session_buses: Arc<Mutex<HashMap<String, Arc<SessionEventBus>>>>,
    /// SafeBrain handle for long-term memory (recall + remember).
    /// All Brain operations go through SafeBrain's async methods (spawn_blocking inside).
    pub brain: Option<permagent::brain_handle::SafeBrain>,
    /// Shared agent persona (hot-reloaded via RwLock).
    pub persona: permagent::config::agent_identity::SharedPersona,
    /// Shared full agent config (primary + workers) for worker resolution.
    pub agent_config: permagent::config::agent_identity::SharedAgentConfig,
    /// Bearer token for authenticating /activity/emit requests.
    /// Loaded from ~/.permagent/secrets/daemon_token.json on startup.
    /// The Tauri shell reads the same file to include the token in requests.
    pub daemon_token: Option<String>,
    /// Short-lived credentials accepted only by browser stream auth.
    pub stream_tokens: StreamTokenStore,
    /// Per-device pairing tokens (#628): named companions, last-seen,
    /// revocation. The bearer middleware accepts the master `daemon_token`
    /// (the hub's own app — legacy, zero-breakage) OR any non-revoked device
    /// token from this registry.
    pub device_registry: Arc<crate::device_registry::DeviceRegistry>,
    /// Activity event ingester — writes Always/Aggregated events to Brain.
    pub activity_ingester: Option<Arc<permagent::activity::ingestion::ActivityIngester>>,
    /// Activity context builder — maintains live state for per-turn digests.
    pub context_builder: Option<Arc<permagent::activity::context_builder::ContextBuilder>>,
    /// Bridge for pending browser content extraction requests.
    pub browser_content_bridge: Arc<crate::routes::browser_content::BrowserContentBridge>,
    /// Bridge for pending act-on-page snapshot requests (#649).
    pub browser_snapshot_bridge: Arc<crate::routes::browser_act::SnapshotBridge>,
    /// Bridge for pending act-on-page act requests (#649).
    pub browser_act_bridge: Arc<crate::routes::browser_act::ActBridge>,
    /// App catalog — static tab/view descriptions for agent navigation.
    pub app_catalog: Arc<permagent::app_catalog::AppCatalog>,
    /// Voice STT provider (Moonshine via sherpa-onnx in dev, swappable).
    pub voice_stt: Option<Arc<dyn crate::voice::SpeechToText>>,
    /// Voice TTS provider (Kokoro via ort+misaki). Wrapped in a hot-swappable
    /// slot so the on-demand model downloader can enable TTS without a daemon
    /// restart — `None` until the Kokoro models are present and loaded
    /// (see routes::voice voice-model endpoints).
    pub voice_tts: SharedTts,
}

/// Hot-swappable TTS slot — `None` until Kokoro models are downloaded/loaded,
/// then swapped in place by the on-demand downloader's completion callback.
pub type SharedTts = Arc<tokio::sync::RwLock<Option<Arc<dyn crate::voice::TextToSpeech>>>>;

impl AppState {
    pub async fn new(tls: bool) -> anyhow::Result<Arc<AppState>> {
        register_builtin_extensions(permagent_mcp::BUILTIN_EXTENSIONS.clone());

        // This daemon serves the Decision Inbox answer path (routes/decisions.rs)
        // over the process-wide AgentManager, so agent turns in THIS process may
        // file `tool_approval` decision rows — answering can reach their parked
        // waiters. Out-of-process populations (CLI sessions, examples) never set
        // this and keep their own answer surfaces instead of filing zombie cards;
        // in-process headless agents (scheduled jobs) are excluded agent-side.
        permagent::decisions::mark_process_serves_inbox();

        let agent_manager = AgentManager::instance().await?;
        let tunnel_manager = Arc::new(TunnelManager::new(tls));
        let gateway_manager = Arc::new(GatewayManager::new(agent_manager.clone())?);

        // ── Pre-migration backup: spectral/permagent.db ──
        // pool_clone() triggers lazy schema migration, so snapshot BEFORE it.
        {
            let source = permagent::config::paths::Paths::spectral_db();
            let backup_root = permagent::config::paths::Paths::data_dir().join("backups");
            if let Err(e) = crate::backup::snapshot_if_stale(
                &source,
                &backup_root,
                crate::backup::DbTarget::Spectral,
            ) {
                tracing::error!(
                    target: "permagentd::backup",
                    error = %e,
                    "Startup backup of spectral/permagent.db failed (non-fatal)"
                );
            }
        }

        // Initialize TaskLogger with the same Spectral DB pool
        if let Ok(pool) = agent_manager.session_manager().pool_clone().await {
            permagent::tasks::init_global(pool.clone());
            tracing::info!("TaskLogger initialized");

            // Decision Inbox (L3 Part B): escalations persist as decision
            // rows. Must install before the first escalate tool call, which
            // otherwise falls back to the in-memory sink.
            if permagent::decision_inbox::escalate::install_decision_sink(std::sync::Arc::new(
                permagent::decision_inbox::sink::SqlDecisionSink::new(pool),
            ))
            .is_ok()
            {
                tracing::info!("Decision sink installed (escalations persist as decisions)");
            }
        } else {
            tracing::warn!("Failed to initialize TaskLogger — task logging disabled");
        }

        // Decision Inbox (L2): post-Review verification hook on the
        // orchestrator extension point. After handle_goal_completion moves a
        // goal to Review, verification runs as a spawned, failure-tolerant
        // task. Idempotent (OnceLock) — safe to call on every startup.
        crate::verification::install_review_hook();
        tracing::info!("Goal review hook installed (post-Review verification)");

        // Per-project wing rules (spectral-recognition prep, the "double
        // lever"): wing labels are both the recognition-validation ground
        // truth and the gate on Spectral's TACT fast path. Generated from the
        // project registry so content mentioning a project classifies into
        // that project's wing instead of Spectral's demo defaults (which dump
        // most real content into "general"). Empty (registry unavailable, no
        // projects, or feature off) → builder gets no rules → Spectral
        // defaults, exactly as before.
        #[cfg(feature = "spectral-recognition")]
        let project_wing_rules: Vec<(String, String)> =
            match agent_manager.session_manager().pool_clone().await {
                Ok(pool) => permagent::wing_rules::load_project_wing_rules(&pool).await,
                Err(e) => {
                    tracing::warn!(
                        target: "permagentd::brain",
                        error = %e,
                        "No session pool for wing-rule generation — using default wing rules"
                    );
                    Vec::new()
                }
            };
        #[cfg(not(feature = "spectral-recognition"))]
        let project_wing_rules: Vec<(String, String)> = Vec::new();

        // Mount Spectral Brain for long-term memory.
        // Brain::builder().build() creates its own tokio runtime internally,
        // so we must run it off the async executor via spawn_blocking.
        // Sanctioned raw spectral::Brain construction site.
        // Brain::builder().build() creates its own tokio runtime internally,
        // so we must run it off the async executor via spawn_blocking.
        // What leaves this block is a SafeBrain.
        let brain: Option<permagent::brain_handle::SafeBrain> =
            tokio::task::spawn_blocking(move || {
                let brain_dir = permagent::config::paths::Paths::brain_dir();
                let ontology_path = permagent::config::paths::Paths::brain_ontology();

                if !brain_dir.exists() || !ontology_path.exists() {
                    tracing::info!(
                        target: "permagentd::brain",
                        "No brain directory at {} — running without long-term memory",
                        brain_dir.display()
                    );
                    return None;
                }

                // ── Pre-migration backup: brain/memory.db ──
                // Must run before Brain::builder().build() which triggers Spectral
                // auto-migration.
                {
                    let source = brain_dir.join("memory.db");
                    let backup_root = permagent::config::paths::Paths::data_dir().join("backups");
                    if let Err(e) = crate::backup::snapshot_if_stale(
                        &source,
                        &backup_root,
                        crate::backup::DbTarget::Brain,
                    ) {
                        tracing::error!(
                            target: "permagentd::backup",
                            error = %e,
                            "Startup backup of brain/memory.db failed (non-fatal)"
                        );
                    }
                }

                let device_id_str =
                    std::env::var("HOSTNAME").unwrap_or_else(|_| "permagent-host".into());

                let mut builder = spectral::Brain::builder()
                    .data_dir(&brain_dir)
                    .ontology_path(&ontology_path)
                    .device_id(spectral::DeviceId::from_descriptor(&device_id_str));
                if !project_wing_rules.is_empty() {
                    tracing::info!(
                        target: "permagentd::brain",
                        rules = project_wing_rules.len(),
                        "Opening Brain with per-project wing rules"
                    );
                    builder = builder.wing_rules(project_wing_rules);
                }
                let raw_brain = match builder.build() {
                    Ok(b) => {
                        tracing::info!(
                            target: "permagentd::brain",
                            "Brain mounted at {}",
                            brain_dir.display()
                        );
                        b
                    }
                    Err(e) => {
                        tracing::error!(
                            target: "permagentd::brain",
                            "Brain failed to open at {}: {}",
                            brain_dir.display(),
                            e
                        );
                        return None;
                    }
                };

                // Startup health check: verify brain is queryable (still raw, pre-wrap)
                match raw_brain.recall("permagent", spectral::Visibility::Private) {
                    Ok(result) => {
                        tracing::info!(
                            target: "permagentd::brain",
                            "Brain healthy — recall('permagent') returned {} hits",
                            result.memory_hits.len()
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "permagentd::brain",
                            "Brain recall test failed: {}",
                            e
                        );
                    }
                }

                // #92 — one-time backfill of `event_at` for imported memories,
                // now that Spectral has created/migrated its schema. Additive
                // column, idempotent, non-fatal: it lets the Brain timeline
                // order by original event time (COALESCE(event_at, created_at))
                // instead of import time. Mirrors the people_bridge backfill.
                match crate::event_at_backfill::backfill_event_at(&brain_dir.join("memory.db")) {
                    Ok(stats) => tracing::info!(
                        target: "permagentd::brain",
                        column_added = stats.column_added,
                        rows_examined = stats.rows_examined,
                        rows_backfilled = stats.rows_backfilled,
                        "event_at backfill complete"
                    ),
                    Err(e) => tracing::warn!(
                        target: "permagentd::brain",
                        error = %e,
                        "event_at backfill failed (non-fatal)"
                    ),
                }

                Some(permagent::brain_handle::SafeBrain::new(raw_brain))
            })
            .await
            .unwrap_or_else(|e| {
                tracing::error!(
                    target: "permagentd::brain",
                    "Brain spawn_blocking task panicked: {}",
                    e
                );
                None
            });

        // Wire Brain into scheduler so scheduled jobs get recall/remember.
        agent_manager.scheduler().set_brain(brain.clone()).await;

        // Make Brain available to platform extensions (Librarian etc.)
        if let Some(ref b) = brain {
            permagent::agents::platform_extensions::set_global_brain(b.clone());
        }

        // Make scheduler available to platform extensions (RecipeAuthor).
        permagent::agents::platform_extensions::recipe_author::set_global_scheduler(
            agent_manager.scheduler(),
        );

        // People↔graph bridge (#255/B): mint identity-only `people` rows for graph
        // person entities (e.g. "mel schembri") and backfill graph_entity_id on
        // pre-bridge rows, so the CRM directory shows everyone the Brain knows.
        // Idempotent; the ontology is the authoritative person set. Non-fatal.
        if let Ok(pool) = agent_manager.session_manager().pool_clone().await {
            let ontology_path = permagent::config::paths::Paths::brain_ontology();
            match permagent::people_bridge::sync_people_from_ontology(&pool, &ontology_path).await {
                Ok(n) => tracing::info!(
                    target: "permagentd::people_bridge",
                    "People↔graph bridge synced ({n} rows minted/backfilled)"
                ),
                Err(e) => tracing::warn!(
                    target: "permagentd::people_bridge",
                    error = %e,
                    "People↔graph bridge sync failed (non-fatal)"
                ),
            }

            // Graph-side bridge (people-in-graph v1 #583): mint rows for runtime /
            // extracted graph persons (Henry create / UI / v1.5 extraction). The
            // union counterpart to the ontology bridge above. Non-fatal.
            if let Some(ref b) = brain {
                match permagent::people_bridge::sync_people_from_graph(&pool, b).await {
                    Ok(n) => tracing::info!(
                        target: "permagentd::people_bridge",
                        "People↔graph bridge (graph side) synced ({n} runtime/extracted rows minted)"
                    ),
                    Err(e) => tracing::warn!(
                        target: "permagentd::people_bridge",
                        error = %e,
                        "People↔graph bridge (graph side) sync failed (non-fatal)"
                    ),
                }
            }

            // Skills source-of-truth migration: export any indexed skill that
            // lacks an on-disk SKILL.md folder to the portable agentskills.io
            // format under ~/.permagent/skills. The on-disk folder is the source
            // of truth; the DB row is its index. Idempotent + non-fatal, so it is
            // safe to run on every boot (a steady state exports nothing).
            match permagent::skills::reconcile_skills_to_disk(&pool).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(
                    target: "permagentd::skills",
                    "Skills SKILL.md migration exported {n} skill(s) to disk"
                ),
                Err(e) => tracing::warn!(
                    target: "permagentd::skills",
                    error = %e,
                    "Skills SKILL.md migration failed (non-fatal)"
                ),
            }
        }

        // Self-healing annotation backfill — runs at daemon startup, not gated on Ollama.
        // Parses "Related terms:" from existing Librarian descriptions and populates
        // memory_annotations. First run annotates ~1384 memories; subsequent starts are no-ops.
        tokio::task::spawn(async {
            let result = tokio::task::spawn_blocking(|| {
                permagent::agents::platform_extensions::librarian::backfill_annotations()
            })
            .await;
            match result {
                Ok(Ok(n)) if n > 0 => {
                    tracing::info!(annotated = n, "Annotation backfill completed")
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => tracing::warn!(error = %e, "Annotation backfill failed"),
                Err(e) => tracing::warn!(error = %e, "Annotation backfill task panicked"),
            }
        });

        // One-shot fix: un-consolidate memories grouped under buggy "tps:" / "ttp:" catchall
        // clusters (substring offset bug in find_domain_clusters). Marker-file gated.
        // Then migrate _pm_consolidated_into column data to Spectral's consolidation_edges table.
        // Sequenced: domain cluster cleanup must complete before consolidation migration.
        {
            let brain_for_migration = brain.clone();
            tokio::task::spawn(async move {
                let result = tokio::task::spawn_blocking(|| {
                    permagent::activity::cleanup::cleanup_buggy_domain_clusters()
                })
                .await;
                match result {
                    Ok(Ok((un, del))) if un > 0 || del > 0 => {
                        tracing::info!(
                            un_consolidated = un,
                            deleted = del,
                            "Buggy domain-cluster cleanup completed"
                        );
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => {
                        tracing::warn!(error = %e, "Buggy domain-cluster cleanup failed");
                        return; // Don't proceed to consolidation migration
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Buggy domain-cluster cleanup task panicked");
                        return;
                    }
                }

                // Migrate _pm_consolidated_into → consolidation_edges (Spectral API).
                // Must run after domain cluster cleanup so the data is clean.
                if let Some(ref brain) = brain_for_migration {
                    let brain = brain.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        permagent::activity::cleanup::migrate_consolidated_into_to_spectral_blocking(&brain)
                    })
                    .await;
                    match result {
                        Ok(Ok(stats)) if stats.rows_migrated > 0 => {
                            tracing::info!(
                                rows_migrated = stats.rows_migrated,
                                distinct_targets = stats.distinct_targets,
                                orphans_skipped = stats.orphans_skipped,
                                column_dropped = stats.column_dropped,
                                "consolidate_into migration completed"
                            );
                        }
                        Ok(Ok(_)) => {}
                        Ok(Err(e)) => {
                            tracing::warn!(error = %e, "consolidate_into migration failed")
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "consolidate_into migration task panicked")
                        }
                    }
                }
            });
        }

        // Brain cleanup: first clean orphaned FK children (one-shot migration),
        // then prune noise memories, then consolidate redundant clusters.
        tokio::task::spawn(async {
            // Phase 0: one-shot cleanup of orphaned constellation_fingerprints /
            // memory_spectrogram rows left by Spectral deletions before FK enforcement.
            let fk_result = tokio::task::spawn_blocking(|| {
                permagent::activity::cleanup::cleanup_orphaned_fk_children()
            })
            .await;
            match fk_result {
                Ok(Ok(stats))
                    if stats.fingerprints_deleted > 0 || stats.spectrograms_deleted > 0 =>
                {
                    tracing::info!(
                        fingerprints_deleted = stats.fingerprints_deleted,
                        spectrograms_deleted = stats.spectrograms_deleted,
                        backup = %stats.backup_path,
                        "FK orphan cleanup completed"
                    );
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => tracing::warn!(error = %e, "FK orphan cleanup failed"),
                Err(e) => tracing::warn!(error = %e, "FK orphan cleanup task panicked"),
            }

            // Phase 1: prune pure noise
            let prune_result = tokio::task::spawn_blocking(|| {
                permagent::activity::cleanup::prune_noise_memories()
            })
            .await;
            match prune_result {
                Ok(Ok(n)) if n > 0 => tracing::info!(pruned = n, "Noise memory prune completed"),
                Ok(Ok(_)) => {}
                Ok(Err(e)) => tracing::warn!(error = %e, "Noise memory prune failed"),
                Err(e) => tracing::warn!(error = %e, "Noise memory prune task panicked"),
            }

            // Phase 2: consolidate browser navigation clusters
            let consolidate_result = tokio::task::spawn_blocking(|| {
                permagent::activity::cleanup::consolidate_clusters_blocking()
            })
            .await;
            match consolidate_result {
                Ok(Ok(n)) if n > 0 => {
                    tracing::info!(consolidated = n, "Cluster consolidation completed")
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => tracing::warn!(error = %e, "Cluster consolidation failed"),
                Err(e) => tracing::warn!(error = %e, "Cluster consolidation task panicked"),
            }
        });

        // Load agent config (primary + workers) from ~/.permagent/agent.yaml
        let agent_config = permagent::config::agent_identity::load_shared_agent_config();
        let persona = {
            let ac = agent_config.read().await;
            tracing::info!(
                target: "permagentd::agent",
                "Agent identity loaded: {} ({} workers)",
                ac.primary.display_name(),
                ac.workers.len()
            );
            Arc::new(tokio::sync::RwLock::new(ac.primary.clone()))
        };

        // Wire persona and agent config into scheduler and agent manager.
        agent_manager.scheduler().set_persona(persona.clone()).await;
        agent_manager
            .scheduler()
            .set_agent_config(agent_config.clone())
            .await;
        agent_manager.set_persona(persona.clone()).await;

        // Seed starter recipes (Workspace Snapshot, Storage Insights) on first run.
        crate::automation::starters::seed_starter_recipes(agent_manager.scheduler().as_ref()).await;

        // First-run welcome memories (#298): seed once onboarding is complete.
        // Idempotent (config marker); also triggered immediately on completion via
        // the /config upsert handler. This startup pass covers onboarding that
        // finished before the Brain was ready or before this feature shipped.
        if permagent::config::Config::global()
            .get_param::<bool>("wizard_complete")
            .unwrap_or(false)
        {
            crate::automation::onboarding_seed::seed_onboarding_memories().await;
        }

        // Load or generate daemon token for /activity/emit auth.
        let daemon_token = load_or_create_daemon_token();

        // Per-device pairing tokens (#628), beside the master token in secrets/.
        let device_registry = Arc::new(crate::device_registry::DeviceRegistry::load(
            crate::device_registry::DeviceRegistry::default_path(),
        ));

        // Activity awareness layer: create Ingester + ContextBuilder if Brain is available.
        // Both subscribe to the global event bus via a long-lived tokio task spawned below.
        let (activity_ingester, context_builder) = if let Some(ref brain) = brain {
            let device_id = sanitize_device_id(
                &std::env::var("HOSTNAME").unwrap_or_else(|_| "permagent-host".into()),
            );
            let ingester = Arc::new(permagent::activity::ingestion::ActivityIngester::new(
                brain.clone(),
                device_id.clone(),
            ));
            let cb = Arc::new(permagent::activity::context_builder::ContextBuilder::new(
                brain.clone(),
            ));
            tracing::info!(
                target: "permagentd::activity",
                "ActivityIngester subscribed to event bus, device_id={}",
                device_id
            );
            tracing::info!(
                target: "permagentd::activity",
                "ContextBuilder subscribed to event bus"
            );

            // Spawn a long-lived task that subscribes to the activity event bus
            // and forwards events to both the Ingester and ContextBuilder.
            let ingester_ref = ingester.clone();
            let cb_ref = cb.clone();
            tokio::spawn(async move {
                let mut rx = permagent::events::subscribe();
                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            if event.event_type == permagent::events::PermagentEventType::Activity {
                                // Extract the ActivityEvent from the PermagentEvent payload
                                if let Some(inner) = event.payload.get("event") {
                                    if let Ok(activity_event) =
                                        serde_json::from_value::<
                                            permagent::events::activity::ActivityEvent,
                                        >(inner.clone())
                                    {
                                        // ContextBuilder is non-blocking (in-memory state)
                                        cb_ref.handle_event(&activity_event);
                                        // Ingester calls brain.remember_with() which blocks,
                                        // so run it on the blocking thread pool.
                                        let ingester = ingester_ref.clone();
                                        let event = activity_event;
                                        tokio::task::spawn_blocking(move || {
                                            ingester.handle_event_blocking(&event);
                                        });
                                    }
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(
                                target: "permagentd::activity",
                                "Activity ingestion lagged, missed {} events",
                                n
                            );
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });

            (Some(ingester), Some(cb))
        } else {
            tracing::info!(
                target: "permagentd::activity",
                "No Brain available — activity ingestion disabled"
            );
            (None, None)
        };

        // Initiative driver (#360): native tick loop consuming the same event
        // bus as the ingester above. Explicitly gated (initiative_enabled,
        // default off) — spawn() logs the on/off state either way.
        if let Ok(pool) = agent_manager.session_manager().pool_clone().await {
            permagent::initiative::driver::spawn(pool);
        } else {
            tracing::warn!(
                target: "initiative",
                "no app DB pool available — initiative driver not started"
            );
        }

        // Playbook synthesis worker (learning loop, increment 1): a periodic,
        // project-scoped, local-first pass that distills Jesse's answered
        // decisions + corrections into provenance-linked hints. Flag-gated
        // (PERMAGENT_PLAYBOOK_ENABLED, default OFF) — spawn() logs the on/off
        // state and does nothing when the flag is unset.
        if let Ok(pool) = agent_manager.session_manager().pool_clone().await {
            permagent::playbook::synthesis::spawn(pool);
        } else {
            tracing::warn!(
                target: "playbook",
                "no app DB pool available — playbook synthesis worker not started"
            );
        }

        // Durable activity journal (#619): a long-lived consumer on the same
        // event bus, persisting selected kinds (goal transitions, decisions,
        // librarian describe runs, Watcher nudges, task failures) as
        // append-only rows so "what did my agents do today" survives the
        // 1000-event ring buffer.
        // Starts with a retention pass (rows older than 90 days). Failure-
        // tolerant: a bad event is logged and skipped, never crashes the task.
        if let Ok(pool) = agent_manager.session_manager().pool_clone().await {
            tokio::spawn(async move {
                match permagent::activity_journal::prune_older_than_days(
                    &pool,
                    permagent::activity_journal::RETENTION_DAYS,
                )
                .await
                {
                    Ok(n) if n > 0 => tracing::info!(
                        target: "permagentd::journal",
                        "Activity journal retention: pruned {} rows older than {} days",
                        n,
                        permagent::activity_journal::RETENTION_DAYS
                    ),
                    Ok(_) => {}
                    Err(e) => tracing::warn!(
                        target: "permagentd::journal",
                        error = %e,
                        "Activity journal retention pass failed (non-fatal)"
                    ),
                }

                let mut rx = permagent::events::subscribe();
                tracing::info!(
                    target: "permagentd::journal",
                    "Activity journal subscribed to event bus"
                );
                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            if let Err(e) =
                                permagent::activity_journal::record_event(&pool, &event).await
                            {
                                tracing::warn!(
                                    target: "permagentd::journal",
                                    error = %e,
                                    "Failed to journal event (skipped)"
                                );
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(
                                target: "permagentd::journal",
                                "Activity journal consumer lagged, missed {} events",
                                n
                            );
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        } else {
            tracing::warn!(
                target: "permagentd::journal",
                "no app DB pool available — activity journal disabled"
            );
        }

        // Librarian warm-load scheduler: checks once per minute if it's time
        // to warm the Librarian's Ollama model for the configured window.
        tokio::spawn(async move {
            crate::routes::librarian::librarian_scheduler_loop().await;
        });

        // Backup scheduler: checks once per hour, snapshots if stale (>20h).
        tokio::spawn(async move {
            crate::backup::backup_scheduler_loop().await;
        });

        // WAL checkpoint timer (durability F4): periodically TRUNCATE the Brain
        // and Spectral WALs so a long-lived / pinned reader can't let them grow
        // unbounded and fill a near-full disk.
        match agent_manager.session_manager().pool_clone().await {
            Ok(pool) => {
                tokio::spawn(async move {
                    crate::wal_checkpoint::wal_checkpoint_loop(pool).await;
                });
            }
            Err(e) => tracing::warn!(
                target: "durability",
                "could not clone Spectral pool; WAL checkpoint timer not started: {e}"
            ),
        }

        // Load app catalog (static tab/view descriptions for agent navigation).
        let app_catalog = crate::app_catalog::init();

        // Agent-led onboarding: load durable feature-usage from config, then turn
        // on write-through persistence so engagement observed from the activity
        // bus is remembered across restarts. Order matters — hydrate the durable
        // state into memory before enabling writes so we build on it, not clobber
        // it.
        permagent::agents::self_knowledge::usage::hydrate_from_config();
        permagent::agents::self_knowledge::usage::enable_persistence();

        // Initialize voice providers (STT + TTS) if model files are present.
        let voice_paths = crate::voice::sherpa_backend::VoiceModelPaths::default_paths();
        let (voice_stt, voice_tts) = init_voice_providers(&voice_paths);

        let state = Arc::new(Self {
            agent_manager,
            recipe_file_hash_map: Arc::new(Mutex::new(HashMap::new())),
            recipe_session_tracker: Arc::new(Mutex::new(HashSet::new())),
            tunnel_manager,
            gateway_manager,
            extension_loading_tasks: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(feature = "local-inference")]
            inference_runtime: InferenceRuntime::get_or_init(),
            session_buses: Arc::new(Mutex::new(HashMap::new())),
            brain,
            persona,
            agent_config,
            daemon_token,
            stream_tokens: StreamTokenStore::default(),
            device_registry,
            activity_ingester,
            context_builder,
            browser_content_bridge: Arc::new(
                crate::routes::browser_content::BrowserContentBridge::new(),
            ),
            browser_snapshot_bridge: Arc::new(crate::routes::browser_act::SnapshotBridge::new()),
            browser_act_bridge: Arc::new(crate::routes::browser_act::ActBridge::new()),
            app_catalog,
            voice_stt,
            voice_tts: Arc::new(tokio::sync::RwLock::new(voice_tts)),
        });

        // Agent runtime-state tick (#288 interim A): derive Henry's state from the
        // same signals as /api/henry/status and emit agent_state_changed on
        // transition, so World View reacts live instead of polling.
        crate::agent_state_tick::spawn(state.clone());

        Ok(state)
    }

    pub async fn set_extension_loading_task(
        &self,
        session_id: String,
        task: JoinHandle<Vec<ExtensionLoadResult>>,
    ) {
        let mut tasks = self.extension_loading_tasks.lock().await;
        tasks.insert(session_id, Arc::new(Mutex::new(Some(task))));
    }

    pub async fn take_extension_loading_task(
        &self,
        session_id: &str,
    ) -> Option<Vec<ExtensionLoadResult>> {
        let task_holder = {
            let tasks = self.extension_loading_tasks.lock().await;
            tasks.get(session_id).cloned()
        };

        if let Some(holder) = task_holder {
            let task = holder.lock().await.take();
            if let Some(handle) = task {
                match handle.await {
                    Ok(results) => return Some(results),
                    Err(e) => {
                        tracing::warn!("Background extension loading task failed: {}", e);
                    }
                }
            }
        }
        None
    }

    pub async fn remove_extension_loading_task(&self, session_id: &str) {
        let mut tasks = self.extension_loading_tasks.lock().await;
        tasks.remove(session_id);
    }

    pub fn scheduler(&self) -> Arc<dyn SchedulerTrait> {
        self.agent_manager.scheduler()
    }

    pub fn session_manager(&self) -> &SessionManager {
        self.agent_manager.session_manager()
    }

    /// Build a RecognitionContext for recall_cascade from current runtime state.
    ///
    /// Populates: now (Utc::now), persona (placeholder, see
    /// [`permagent::config::agent_identity::DEFAULT_PERSONA_KEY`] — inert for
    /// recall today), session_id (if provided), focus_wing (from active project,
    /// if any).
    pub fn build_recognition_context(
        &self,
        session_id: Option<&str>,
    ) -> spectral::graph::RecognitionContext {
        let focus_wing = self
            .activity_ingester
            .as_ref()
            .and_then(|ing| ing.active_project())
            .map(|ap| ap.wing);

        let mut ctx = spectral::graph::RecognitionContext::empty()
            .with_persona(permagent::config::agent_identity::DEFAULT_PERSONA_KEY);

        if let Some(sid) = session_id {
            ctx = ctx.with_session(sid);
        }
        if let Some(wing) = focus_wing {
            ctx = ctx.with_focus_wing(wing);
        }
        ctx
    }

    pub async fn set_recipe_file_hash_map(&self, hash_map: HashMap<String, PathBuf>) {
        let mut map = self.recipe_file_hash_map.lock().await;
        *map = hash_map;
    }

    pub async fn mark_recipe_run_if_absent(&self, session_id: &str) -> bool {
        let mut sessions = self.recipe_session_tracker.lock().await;
        if sessions.contains(session_id) {
            false
        } else {
            sessions.insert(session_id.to_string());
            true
        }
    }

    pub async fn get_or_create_event_bus(&self, session_id: &str) -> Arc<SessionEventBus> {
        let mut buses = self.session_buses.lock().await;
        buses
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(SessionEventBus::new()))
            .clone()
    }

    /// Get an existing event bus for a session without creating one.
    pub async fn get_event_bus(&self, session_id: &str) -> Option<Arc<SessionEventBus>> {
        let buses = self.session_buses.lock().await;
        buses.get(session_id).cloned()
    }

    /// Remove the event bus for a session, freeing its replay buffer.
    pub async fn remove_event_bus(&self, session_id: &str) {
        let mut buses = self.session_buses.lock().await;
        buses.remove(session_id);
    }

    pub async fn get_agent(
        &self,
        session_id: String,
    ) -> anyhow::Result<Arc<permagent::agents::Agent>> {
        self.agent_manager.get_or_create_agent(session_id).await
    }

    pub async fn get_agent_for_route(
        &self,
        session_id: String,
    ) -> Result<Arc<permagent::agents::Agent>, StatusCode> {
        self.get_agent(session_id).await.map_err(|e| {
            tracing::error!("Failed to get agent: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
    }
}

type VoiceProviders = (
    Option<Arc<dyn crate::voice::SpeechToText>>,
    Option<Arc<dyn crate::voice::TextToSpeech>>,
);

fn init_voice_providers(
    stt_paths: &crate::voice::sherpa_backend::VoiceModelPaths,
) -> VoiceProviders {
    // STT: sherpa-onnx Moonshine (no espeak dependency for recognition)
    let stt: Option<Arc<dyn crate::voice::SpeechToText>> = if stt_paths.models_exist() {
        match crate::voice::sherpa_backend::SherpaMoonshineStt::new(&stt_paths.stt_model_dir, 4) {
            Ok(s) => {
                tracing::info!(target: "permagentd::voice", "STT loaded: Moonshine via sherpa-onnx");
                Some(Arc::new(s))
            }
            Err(e) => {
                tracing::error!(target: "permagentd::voice", "STT load failed: {}", e);
                None
            }
        }
    } else {
        tracing::info!(target: "permagentd::voice", "STT models not found — voice STT disabled");
        None
    };

    // TTS: standalone Kokoro via ort + misaki-rs (GPL-clean shipping backend)
    (stt, build_kokoro_tts())
}

/// Build the Kokoro TTS provider if its model files are present on disk.
///
/// Returns `None` (voice TTS disabled) when the ~353MB Kokoro assets have not
/// been downloaded yet — the on-demand downloader fetches them and then calls
/// this again to hot-swap a live provider into [`SharedTts`] without a restart.
pub fn build_kokoro_tts() -> Option<Arc<dyn crate::voice::TextToSpeech>> {
    let tts_paths = crate::voice::ort_kokoro_backend::OrtKokoroModelPaths::default_paths();
    if !tts_paths.models_exist() {
        tracing::info!(target: "permagentd::voice", "TTS models not found — voice TTS disabled");
        return None;
    }
    match crate::voice::ort_kokoro_backend::OrtKokoroTts::new(
        &tts_paths.model_path,
        &tts_paths.voices_path,
        "bm_lewis",
    ) {
        Ok(t) => {
            tracing::info!(target: "permagentd::voice", "TTS loaded: Kokoro via ort+misaki-rs (GPL-clean)");
            Some(Arc::new(t))
        }
        Err(e) => {
            tracing::error!(target: "permagentd::voice", "TTS load failed: {}", e);
            None
        }
    }
}

/// Sanitize hostname for use as device_id: lowercase, replace dots/whitespace
/// with hyphens, strip non-alphanumeric except hyphens.
fn sanitize_device_id(hostname: &str) -> String {
    let sanitized: String = hostname
        .to_lowercase()
        .chars()
        .map(|c| {
            if c == '.' || c.is_ascii_whitespace() {
                '-'
            } else {
                c
            }
        })
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    // Collapse repeated hyphens
    let mut result = String::with_capacity(sanitized.len());
    let mut last_was_hyphen = true;
    for c in sanitized.chars() {
        if c == '-' {
            if !last_was_hyphen {
                result.push('-');
                last_was_hyphen = true;
            }
        } else {
            result.push(c);
            last_was_hyphen = false;
        }
    }
    result.trim_end_matches('-').to_string()
}

/// Load daemon token from ~/.permagent/secrets/daemon_token.json.
/// If the file does not exist, generate a new 32-byte random token,
/// persist it with mode 0600, and return it.
fn load_or_create_daemon_token() -> Option<String> {
    let secrets_dir = permagent::config::paths::Paths::data_dir().join("secrets");
    let token_path = secrets_dir.join("daemon_token.json");

    // Try to read existing token
    if token_path.exists() {
        match std::fs::read_to_string(&token_path) {
            Ok(content) => {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(token) = parsed.get("token").and_then(|v| v.as_str()) {
                        tracing::info!(
                            target: "permagentd::auth",
                            "Daemon token loaded from {}",
                            token_path.display()
                        );
                        return Some(token.to_string());
                    }
                }
                tracing::warn!(
                    target: "permagentd::auth",
                    "daemon_token.json exists but is malformed; regenerating"
                );
            }
            Err(e) => {
                tracing::warn!(
                    target: "permagentd::auth",
                    "Failed to read daemon_token.json: {}; regenerating",
                    e
                );
            }
        }
    }

    // Generate new token
    let token_bytes: [u8; 32] = rand::random();
    let token = hex::encode(token_bytes);

    // 0700 from creation, re-enforced if the directory already exists.
    if let Err(e) = permagent::config::secure_fs::ensure_private_dir(&secrets_dir) {
        tracing::error!(
            target: "permagentd::auth",
            "Failed to create secrets dir: {}",
            e
        );
        return None;
    }

    let content = serde_json::json!({ "token": token });
    let json_str = serde_json::to_string_pretty(&content).unwrap();

    // Atomic write, 0600 from the first byte — the control-plane auth token
    // must never be observable world-readable.
    match permagent::config::secure_fs::write_private_file(&token_path, json_str.as_bytes()) {
        Ok(_) => {
            tracing::info!(
                target: "permagentd::auth",
                "Daemon token generated and saved to {}",
                token_path.display()
            );
            Some(token)
        }
        Err(e) => {
            tracing::error!(
                target: "permagentd::auth",
                "Failed to write daemon_token.json: {}",
                e
            );
            None
        }
    }
}
