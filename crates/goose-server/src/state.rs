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

            let device_id_str = std::env::var("HOSTNAME")
                .unwrap_or_else(|_| "permagent-host".into());

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

        // Load agent persona from ~/.permagent/agent.yaml
        let persona = permagent::config::agent_identity::load_shared_persona();
        {
            let p = persona.read().await;
            tracing::info!(
                target: "permagentd::agent",
                "Agent identity loaded: {}",
                p.display_name()
            );
        }

        // Wire persona into scheduler and agent manager for system prompts.
        agent_manager.scheduler().set_persona(persona.clone()).await;
        agent_manager.set_persona(persona.clone()).await;

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
