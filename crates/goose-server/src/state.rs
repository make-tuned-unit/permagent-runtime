use axum::http::StatusCode;
use permagent::builtin_extension::register_builtin_extensions;
use permagent::execution::manager::AgentManager;
use permagent::scheduler_trait::SchedulerTrait;
use permagent::session::SessionManager;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
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
    /// Spectral Brain handle for long-term memory (recall + remember).
    pub brain: Option<Arc<spectral::Brain>>,
    /// Shared agent persona (hot-reloaded via RwLock).
    pub persona: permagent::config::agent_identity::SharedPersona,
    /// Shared full agent config (primary + workers) for worker resolution.
    pub agent_config: permagent::config::agent_identity::SharedAgentConfig,
    /// Bearer token for authenticating /activity/emit requests.
    /// Loaded from ~/.permagent/secrets/daemon_token.json on startup.
    /// The Tauri shell reads the same file to include the token in requests.
    pub daemon_token: Option<String>,
    /// Activity event ingester — writes Always/Aggregated events to Brain.
    pub activity_ingester: Option<Arc<permagent::activity::ingestion::ActivityIngester>>,
    /// Activity context builder — maintains live state for per-turn digests.
    pub context_builder: Option<Arc<permagent::activity::context_builder::ContextBuilder>>,
    /// Bridge for pending browser content extraction requests.
    pub browser_content_bridge: Arc<crate::routes::browser_content::BrowserContentBridge>,
    /// App catalog — static tab/view descriptions for agent navigation.
    pub app_catalog: Arc<permagent::app_catalog::AppCatalog>,
}

impl AppState {
    pub async fn new(tls: bool) -> anyhow::Result<Arc<AppState>> {
        register_builtin_extensions(permagent_mcp::BUILTIN_EXTENSIONS.clone());

        let agent_manager = AgentManager::instance().await?;
        let tunnel_manager = Arc::new(TunnelManager::new(tls));
        let gateway_manager = Arc::new(GatewayManager::new(agent_manager.clone())?);

        // Initialize TaskLogger with the same Spectral DB pool
        if let Ok(pool) = agent_manager.session_manager().pool_clone().await {
            permagent::tasks::init_global(pool);
            tracing::info!("TaskLogger initialized");
        } else {
            tracing::warn!("Failed to initialize TaskLogger — task logging disabled");
        }

        // Mount Spectral Brain for long-term memory.
        // Brain::builder().build() creates its own tokio runtime internally,
        // so we must run it off the async executor via spawn_blocking.
        let brain: Option<Arc<spectral::Brain>> = tokio::task::spawn_blocking(|| {
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

            let device_id_str =
                std::env::var("HOSTNAME").unwrap_or_else(|_| "permagent-host".into());

            let brain = match spectral::Brain::builder()
                .data_dir(&brain_dir)
                .ontology_path(&ontology_path)
                .device_id(spectral::DeviceId::from_descriptor(&device_id_str))
                .build()
            {
                Ok(b) => {
                    tracing::info!(
                        target: "permagentd::brain",
                        "Brain mounted at {}",
                        brain_dir.display()
                    );
                    Arc::new(b)
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

            // Startup health check: verify brain is queryable
            match brain.recall("permagent", spectral::Visibility::Private) {
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

            Some(brain)
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

        // Self-healing annotation backfill — runs at daemon startup, not gated on Ollama.
        // Parses "Related terms:" from existing Librarian descriptions and populates
        // memory_annotations. First run annotates ~1384 memories; subsequent starts are no-ops.
        tokio::task::spawn(async {
            let result = tokio::task::spawn_blocking(|| {
                permagent::agents::platform_extensions::librarian::backfill_annotations()
            })
            .await;
            match result {
                Ok(Ok(n)) if n > 0 => tracing::info!(annotated = n, "Annotation backfill completed"),
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
                        tracing::info!(un_consolidated = un, deleted = del, "Buggy domain-cluster cleanup completed");
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
                        permagent::activity::cleanup::migrate_consolidated_into_to_spectral(&brain)
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
                        Ok(Err(e)) => tracing::warn!(error = %e, "consolidate_into migration failed"),
                        Err(e) => tracing::warn!(error = %e, "consolidate_into migration task panicked"),
                    }
                }
            });
        }

        // Brain cleanup: prune noise memories, then consolidate redundant clusters.
        // Runs after annotation backfill at daemon startup.
        tokio::task::spawn(async {
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
                permagent::activity::cleanup::consolidate_clusters()
            })
            .await;
            match consolidate_result {
                Ok(Ok(n)) if n > 0 => tracing::info!(consolidated = n, "Cluster consolidation completed"),
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

        // Load or generate daemon token for /activity/emit auth.
        let daemon_token = load_or_create_daemon_token();

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
                                            ingester.handle_event(&event);
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

        // Librarian warm-load scheduler: checks once per minute if it's time
        // to warm the Librarian's Ollama model for the configured window.
        tokio::spawn(async move {
            crate::routes::librarian::librarian_scheduler_loop().await;
        });

        // Load app catalog (static tab/view descriptions for agent navigation).
        let app_catalog = crate::app_catalog::init();

        Ok(Arc::new(Self {
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
            activity_ingester,
            context_builder,
            browser_content_bridge: Arc::new(crate::routes::browser_content::BrowserContentBridge::new()),
            app_catalog,
        }))
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
    /// Populates: now (Utc::now), persona ("henry"), session_id (if provided),
    /// focus_wing (from active project, if any).
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
            .with_persona("henry");

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

    if let Err(e) = std::fs::create_dir_all(&secrets_dir) {
        tracing::error!(
            target: "permagentd::auth",
            "Failed to create secrets dir: {}",
            e
        );
        return None;
    }

    let content = serde_json::json!({ "token": token });
    let json_str = serde_json::to_string_pretty(&content).unwrap();

    match std::fs::write(&token_path, &json_str) {
        Ok(_) => {
            // Set file permissions to 0600 on Unix
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600));
            }
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
