use anyhow::Result;
use permagent::config::Config;
use permagent::recipe::read_recipe_file_content::RecipeFile;

use super::github_recipe::{
    list_github_recipes, retrieve_recipe_from_github, RecipeInfo, RecipeSource,
    GOOSE_RECIPE_GITHUB_REPO_CONFIG_KEY,
};
use permagent::recipe::local_recipes::{discover_local_recipes, load_local_recipe_file};

use super::builtin_recipes::builtin_recipe_file;

pub fn load_recipe_file(recipe_name: &str) -> Result<RecipeFile> {
    // Resolution order: a local recipe of this name wins (operators can
    // override), then Permagent's built-in recipes (e.g. the coding harness),
    // then the configured GitHub recipe repo.
    load_local_recipe_file(recipe_name)
        .or_else(|local_err| builtin_recipe_file(recipe_name).ok_or(local_err))
        .or_else(|err| {
            if let Some(recipe_repo_full_name) = configured_github_recipe_repo() {
                retrieve_recipe_from_github(recipe_name, &recipe_repo_full_name)
            } else {
                Err(err)
            }
        })
}

fn configured_github_recipe_repo() -> Option<String> {
    let config = Config::global();
    match config.get_param(GOOSE_RECIPE_GITHUB_REPO_CONFIG_KEY) {
        Ok(Some(recipe_repo_full_name)) => Some(recipe_repo_full_name),
        _ => None,
    }
}

/// Lists all available recipes from local paths and GitHub repositories
pub fn list_available_recipes() -> Result<Vec<RecipeInfo>> {
    let mut recipes = Vec::new();

    // Search local recipes
    if let Ok(discovery) = discover_local_recipes() {
        recipes.extend(discovery.recipes.into_iter().map(|(path, recipe)| {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            RecipeInfo {
                name,
                source: RecipeSource::Local,
                path: path.display().to_string(),
                title: Some(recipe.title),
                description: Some(recipe.description),
            }
        }));
    }

    // Search GitHub recipes if configured
    if let Some(repo) = configured_github_recipe_repo() {
        if let Ok(github_recipes) = list_github_recipes(&repo) {
            recipes.extend(github_recipes);
        }
    }

    Ok(recipes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipes::builtin_recipes::PERMAGENT_CODING_RECIPE_NAME;
    use permagent::recipe::template_recipe::parse_recipe_content;

    /// The Build tab's third option runs
    /// `permagent run --recipe permagent-coding --interactive`. This proves that
    /// command resolves to a real coding configuration of the internal loop with
    /// no filesystem setup and no GitHub repo — i.e. the option is not a dead
    /// button but a mounted seam onto the internal agent loop.
    #[test]
    fn builtin_permagent_coding_recipe_resolves_and_parses() {
        let rf = load_recipe_file(PERMAGENT_CODING_RECIPE_NAME)
            .expect("built-in coding recipe should resolve by name");

        // Pins the coding toolset explicitly: the root developer extension is
        // read-only (read + search + tree), while dispatched goal workers get
        // their own full developer extension; analyze = the code-structure map;
        // summon = the subagent seam for tiered cost-routing; orchestrator = executable DAGs;
        // extensionmanager = on-demand research/MCP discovery without paying
        // the tool-schema cost in every coding turn.
        assert!(
            rf.content.contains("name: developer"),
            "coding harness must enable the developer extension"
        );
        assert!(
            rf.content.contains("name: analyze"),
            "coding harness must enable the analyze extension"
        );
        assert!(
            rf.content.contains("name: summon"),
            "coding harness must enable the summon extension for tiered subagent routing"
        );
        assert!(
            rf.content.contains("name: orchestrator"),
            "coding harness must create executable goal DAGs rather than static plans"
        );
        assert!(
            rf.content.contains("name: extensionmanager"),
            "coding harness must be able to discover user-configured MCPs"
        );

        // Parses into a valid recipe with a coding system prompt and six
        // pinned extensions.
        let (recipe, _) =
            parse_recipe_content(&rf.content, None).expect("built-in coding recipe should parse");
        assert_eq!(recipe.title, "Permagent Coding Harness");
        assert!(
            recipe.instructions.is_some(),
            "coding harness must set a system prompt"
        );
        let instructions = recipe
            .instructions
            .as_deref()
            .expect("coding harness must set a system prompt");
        let extensions = recipe.extensions.expect("coding harness pins extensions");
        assert_eq!(
            extensions.len(),
            6,
            "coding harness pins developer, analyze, summon, orchestrator, council, and extensionmanager capabilities"
        );
        let developer = extensions
            .iter()
            .find(|extension| extension.name() == "developer")
            .expect("coding harness pins developer");
        assert_eq!(developer.available_tools(), ["read", "tree", "search"]);
        // Lane R58a: a headless coordinator probed `check_worker` and
        // `routing_roster`, then ended its turn asking the absent user to pick
        // the `worker_tier` (`select-checks-worker-before-dispatch`). Routing
        // within the roster is the coordinator's own call.
        assert!(
            instructions.contains("your own decision, never the user's")
                && instructions.contains("never end a turn asking the user to choose routing"),
            "coding recipe must make the worker/worker_tier choice the coordinator's own"
        );
        assert!(
            instructions.contains("SMALLEST executable graph")
                && instructions.contains("Coalesce changes that share a file into one card"),
            "coding recipe must prevent overlapping micro-goals from exhausting headless runs"
        );
        // Lane R57a: the PAUSED half of exception (a). Headless coordinators
        // read "continue this paused roadmap" as licence to dispatch its one
        // Ready goal with `goal_advance`, which ran the goal and left the
        // roadmap paused behind it
        // (`select-existing-roadmap-resumes-not-redecomposes`). The
        // orchestrator refuses that dispatch now; the recipe must still say
        // so, so the model reaches for the right tool rather than learning it
        // from a refusal.
        assert!(
            instructions.contains("When the roadmap is PAUSED")
                && instructions.contains("never dispatch a paused roadmap's goals one by one"),
            "coding recipe must send a paused roadmap to resume_roadmap, not to per-goal dispatch"
        );
        // Lane R59a: a headless coordinator read the shipping-quote-signature
        // task, then called `escalate kind: decision` asking whether to apply
        // the surcharge before or after rounding — a detail the request never
        // raised — and ended the run with no roadmap
        // (`harness-refactor-shipping-quote-signature`). The recipe must say
        // that a detail the request is silent on, with an existing-code
        // convention to follow, is decided and noted, not escalated — while
        // still escalating a choice the request itself names, a missing
        // fact/credential, or a spend cap (so the ambiguous-choice and
        // missing-credential/no-repo-facts fixtures keep escalating).
        assert!(
            instructions.contains("own convention already answers it")
                && instructions.contains("never merely because the request stayed silent"),
            "coding recipe must decide a silent implementation detail from the existing code's \
             convention and proceed, not escalate it"
        );
        // Lane R61a (ruling 2026-09-16): the harness only sees memories about
        // the repo it is in unless the user approves more at the terminal. The
        // coordinator must know its recall is repo-scoped, that the only way
        // wider is `request_memory_access`, and that it is for requests which
        // actually need another project — not a reflex.
        assert!(
            instructions.contains("Your recalled memory is scoped to this repo")
                && instructions.contains("`request_memory_access`")
                && instructions.contains("actually needs another project's patterns")
                && instructions.contains("user approves or denies it in the terminal")
                && instructions.contains("a headless run refuses it"),
            "coding recipe must say memory is repo-scoped and how (and when) to ask for more"
        );
        // Lane R59b: a coordinator asked to review `payment.py` without
        // editing it wrote the review inline, declined to escalate, and was
        // parked with neither a roadmap nor an escalation
        // (`select-review-request-does-not-mutate`). A review request is a
        // one-goal, read-only roadmap — never an inline review ending in prose.
        assert!(
            instructions.contains("A request to review")
                && instructions.contains("ONE read-only goal")
                && instructions.contains("never review it inline and end the turn in"),
            "coding recipe must route a review request to a one-goal read-only roadmap, not an \
             inline prose review"
        );
        // R56a: a write-scope refusal is transient and self-clearing. The
        // coordinator that escalates it burns the run's one escalation and
        // ends parked with the sibling never dispatched.
        assert!(
            instructions.contains("write scope conflicts with active goal")
                && instructions.contains("is a WAIT"),
            "coding recipe must tell the coordinator a write-scope refusal is a wait, not an \
             escalation"
        );
    }
}
