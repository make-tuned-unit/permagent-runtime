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
    pub description: String,
    pub affords: Vec<String>,
    pub suggest_when: Vec<String>,
    pub customizable_layout: bool,
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
