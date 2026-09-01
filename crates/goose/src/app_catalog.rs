//! App Catalog — static description of every tab/view in the Permagent UI.
//!
//! The catalog is loaded once at daemon startup and made available globally
//! so platform extensions (e.g., app_conductor) can validate navigation
//! requests and render catalog info in the agent's system prompt.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

// ── Global catalog (set once by daemon startup) ─────────────────────────────

static GLOBAL_CATALOG: OnceLock<Arc<AppCatalog>> = OnceLock::new();

pub fn set_global_catalog(catalog: Arc<AppCatalog>) {
    let _ = GLOBAL_CATALOG.set(catalog);
}

pub fn get_global_catalog() -> Option<Arc<AppCatalog>> {
    GLOBAL_CATALOG.get().cloned()
}

// ── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppCatalog {
    pub tabs: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub name: String,
    pub tool_type: String,
    pub panel_type: String,
    /// Fixed sub-section within the target surface. Set on entries whose page
    /// lives inside another surface (the 2026-08 Console consolidation:
    /// Sessions/Trace/Inbox are Settings sections now) so `navigate_app` still
    /// resolves by the stable name and lands on the right pane. An explicit
    /// `section` argument from the agent overrides this default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    pub description: String,
    pub affords: Vec<String>,
    /// The store this tab renders, the machine-checkable half of the
    /// observability contract; the coverage test asserts the observe_app aspect
    /// claiming this tab reads the same store.
    pub reads: String,
    pub suggest_when: Vec<String>,
    pub customizable_layout: bool,
    /// Sub-panels rendered INSIDE this tab (e.g. the Documents panel on a
    /// project's detail view, inside the Projects tab) that read their own
    /// distinct store — a tab's `reads` names only the tab's OWN top-level
    /// store, so a panel nested inside it needs its own declared `reads` or
    /// its store stays invisible to the coverage guard. This is the gap
    /// `project_documents` fell through: the Projects tab's `reads: projects`
    /// said nothing about the Documents panel living inside it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panels: Vec<PanelEntry>,
}

/// One sub-panel inside a [`CatalogEntry`] tab. Deliberately minimal — a name
/// and the store it reads — because the only thing the coverage guard needs
/// is "this store must map to an observable aspect somewhere", not a second
/// copy of the whole tab schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelEntry {
    pub name: String,
    pub reads: String,
}

impl AppCatalog {
    /// Look up a catalog entry by user-facing name (case-insensitive).
    pub fn find_by_name(&self, name: &str) -> Option<&CatalogEntry> {
        let lower = name.to_lowercase();
        self.tabs.iter().find(|e| e.name.to_lowercase() == lower)
    }

    /// Render the catalog as a system prompt block for the agent.
    pub fn to_prompt_block(&self) -> String {
        let mut out = String::from(
            "You can guide users to specific tabs in the app. When helpful, \
             call navigate_app with the tab name. Available tabs:\n\n",
        );
        for entry in &self.tabs {
            out.push_str(&format!("- **{}**: {}\n", entry.name, entry.description));
            if !entry.suggest_when.is_empty() {
                out.push_str("  Suggest when: ");
                out.push_str(&entry.suggest_when.join("; "));
                out.push('\n');
            }
        }
        out.push_str(
            "\nWhen the user asks to be taken somewhere, asks where something is, or asks to \
             open/visit/go-to a tab, you MUST call the navigate_app tool. Do not just describe \
             the navigation in text — the user will not be navigated unless you call the tool. \
             Explain briefly in chat what you're doing (one sentence), then call navigate_app \
             with the tab name.",
        );
        out
    }

    /// List all tab names (for error messages).
    pub fn tab_names(&self) -> Vec<&str> {
        self.tabs.iter().map(|e| e.name.as_str()).collect()
    }
}
