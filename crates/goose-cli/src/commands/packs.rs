//! `permagent packs` — the objective model advisor AND the wiring that acts on
//! it. `recommend` prints the best-fit model per workflow role (no vendor bias);
//! `apply`/`set` PERSIST a role→model mapping the dispatch path routes by;
//! `show`/`clear` inspect and revert it.
//!
//! Persisting NOTHING is the default and a first-class state: with no mapping the
//! harness stays on the single session model — it never falls back to a built-in
//! vendor pack (see `permagent::cost_router::role_map`). The recommendation is
//! computed in `permagent::cost_router::recommend`; this is the CLI seam over it.

use anyhow::Result;
use permagent::cost_router::{
    clear_role_model, configured_role_models, mappings_to_persist, recommend_configured,
    recommend_from_available, set_role_model, AvailableModel, Recommendation, WorkflowRole,
};

pub async fn handle_packs_command(command: PacksCommand) -> Result<()> {
    match command {
        PacksCommand::Recommend { json, models } => recommend_cmd(json, models),
        PacksCommand::Apply { models, dry_run } => apply_cmd(models, dry_run),
        PacksCommand::Set {
            role,
            provider,
            model,
        } => set_cmd(&role, &provider, &model),
        PacksCommand::Show { json } => show_cmd(json),
        PacksCommand::Clear { role } => clear_cmd(role.as_deref()),
    }
}

/// Resolve a recommendation from either declared `--models` or auto-detection.
fn resolve_recommendation(models: Option<String>) -> Recommendation {
    match models {
        Some(spec) => {
            let declared: Vec<AvailableModel> = spec
                .split(',')
                .filter_map(|s| {
                    let s = s.trim();
                    s.split_once('/')
                        .map(|(p, m)| AvailableModel::new(p.trim(), m.trim()))
                })
                .filter(|a| !a.provider.is_empty() && !a.model.is_empty())
                .collect();
            recommend_from_available(&declared)
        }
        None => recommend_configured(),
    }
}

/// Parse an operator-supplied role tag, erroring with the accepted set.
fn parse_role(tag: &str) -> Result<WorkflowRole> {
    WorkflowRole::from_tag(tag).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown role '{}' — expected one of: orchestrate (alias: hard), edit, mechanical, \
             review, local",
            tag
        )
    })
}

fn recommend_cmd(json: bool, models: Option<String>) -> Result<()> {
    let result = resolve_recommendation(models);

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    println!("Objective per-role model recommendation");
    println!(
        "(by measured diff-format reliability, orchestration strength, and cost — no vendor bias)\n"
    );

    if result.considered.is_empty() {
        println!("No known models detected among your configured providers.");
        if !result.unknown_models.is_empty() {
            println!(
                "Configured but not in the knowledge base: {}",
                result.unknown_models.join(", ")
            );
        }
        println!(
            "\nSet a provider's API key (auto-detected), or pass \
             `--models provider/model,provider/model` to declare what you have."
        );
        return Ok(());
    }

    println!("Considered: {}\n", result.considered.join(", "));
    for r in &result.recommendations {
        let role = format!("{:<11}", r.role.as_str().to_uppercase());
        if r.model.is_empty() {
            println!("  {role} (none) — {}", r.reason);
        } else {
            println!(
                "  {role} {}/{}  [{}]  ~${:.2}/Mtok",
                r.provider, r.model, r.display_name, r.blended_cost_per_mtok
            );
            println!("              {} — {}", r.role.description(), r.reason);
        }
        for w in &r.warnings {
            println!("              ! {w}");
        }
    }

    if !result.unknown_models.is_empty() {
        println!(
            "\nNot in the knowledge base (add a row to include in the recommendation): {}",
            result.unknown_models.join(", ")
        );
    }
    println!(
        "\nThis is advisory. Run `permagent packs apply` to persist it and route each role to \
         its model, or `permagent packs set --role <r> --provider <p> --model <m>` to pin one."
    );
    Ok(())
}

fn apply_cmd(models: Option<String>, dry_run: bool) -> Result<()> {
    let result = resolve_recommendation(models);
    let to_persist = mappings_to_persist(&result.recommendations);

    if to_persist.is_empty() {
        println!(
            "Nothing to apply — no known models detected. The harness stays single-model (no \
             vendor default). Set a provider key or pass `--models provider/model,...`."
        );
        return Ok(());
    }

    if dry_run {
        println!("Would route each workflow role to (no changes written):");
    } else {
        println!("Routing each workflow role to its recommended model:");
    }
    for (role, provider, model) in &to_persist {
        println!(
            "  {:<11} → {}/{}",
            role.as_str().to_uppercase(),
            provider,
            model
        );
        if !dry_run {
            set_role_model(*role, provider, model)
                .map_err(|e| anyhow::anyhow!("failed to persist {}: {}", role.as_str(), e))?;
        }
    }
    // Roles the recommender could not fill (e.g. LOCAL with no on-device model)
    // are left unset → those stay on the single session model.
    let filled: std::collections::HashSet<_> = to_persist.iter().map(|(r, _, _)| *r).collect();
    let unset: Vec<&str> = WorkflowRole::all()
        .iter()
        .filter(|r| !filled.contains(*r))
        .map(|r| r.as_str())
        .collect();
    if !unset.is_empty() {
        println!(
            "\nLeft single-model (no recommended model): {}",
            unset.join(", ")
        );
    }
    if dry_run {
        println!("\n(dry run — re-run without --dry-run to persist)");
    } else {
        println!("\nDone. `permagent packs show` to review, `permagent packs clear` to revert.");
    }
    Ok(())
}

fn set_cmd(role: &str, provider: &str, model: &str) -> Result<()> {
    let role = parse_role(role)?;
    if provider.trim().is_empty() || model.trim().is_empty() {
        anyhow::bail!("--provider and --model must both be non-empty");
    }
    set_role_model(role, provider.trim(), model.trim())
        .map_err(|e| anyhow::anyhow!("failed to persist {}: {}", role.as_str(), e))?;
    println!(
        "Set {} → {}/{}. This role now routes to that model.",
        role.as_str().to_uppercase(),
        provider.trim(),
        model.trim()
    );
    Ok(())
}

fn show_cmd(json: bool) -> Result<()> {
    let configured = configured_role_models();

    if json {
        let map: serde_json::Map<String, serde_json::Value> = configured
            .iter()
            .map(|(role, rm)| {
                (
                    role.as_str().to_string(),
                    serde_json::json!({ "provider": rm.provider, "model": rm.model }),
                )
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&map)?);
        return Ok(());
    }

    if configured.is_empty() {
        println!(
            "No role→model mapping configured — every workflow role runs on your single session \
             model (no vendor default). Run `permagent packs apply` to route roles objectively."
        );
        return Ok(());
    }
    println!("Configured role → model routing:");
    for (role, rm) in &configured {
        println!(
            "  {:<11} → {}/{}",
            role.as_str().to_uppercase(),
            rm.provider,
            rm.model
        );
    }
    let unset: Vec<&str> = WorkflowRole::all()
        .iter()
        .filter(|r| !configured.iter().any(|(cr, _)| cr == *r))
        .map(|r| r.as_str())
        .collect();
    if !unset.is_empty() {
        println!("  (single-model: {})", unset.join(", "));
    }
    Ok(())
}

fn clear_cmd(role: Option<&str>) -> Result<()> {
    match role {
        Some(tag) => {
            let role = parse_role(tag)?;
            clear_role_model(role)
                .map_err(|e| anyhow::anyhow!("failed to clear {}: {}", role.as_str(), e))?;
            println!(
                "Cleared {} — it reverts to the single session model.",
                role.as_str().to_uppercase()
            );
        }
        None => {
            for role in WorkflowRole::all() {
                clear_role_model(role)
                    .map_err(|e| anyhow::anyhow!("failed to clear {}: {}", role.as_str(), e))?;
            }
            println!("Cleared all role→model mappings — the harness is back to single-model.");
        }
    }
    Ok(())
}

#[derive(clap::Subcommand, Debug)]
pub enum PacksCommand {
    /// Recommend the best-fit model per workflow role among your configured models
    Recommend {
        /// Print the recommendation as JSON (for tooling / the Build tab)
        #[arg(long)]
        json: bool,
        /// Override auto-detection with a comma-separated list of provider/model pairs
        #[arg(long, value_name = "PROVIDER/MODEL,...")]
        models: Option<String>,
    },
    /// Persist the objective recommendation — route each workflow role to its recommended model
    Apply {
        /// Override auto-detection with a comma-separated list of provider/model pairs
        #[arg(long, value_name = "PROVIDER/MODEL,...")]
        models: Option<String>,
        /// Print what would change without writing anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Pin one role's model explicitly (overrides the recommendation for that role)
    Set {
        /// Role: orchestrate (alias: hard), edit, mechanical, review, or local
        #[arg(long)]
        role: String,
        /// Provider id (e.g. anthropic, openai, ollama)
        #[arg(long)]
        provider: String,
        /// Model id (e.g. claude-sonnet-5)
        #[arg(long)]
        model: String,
    },
    /// Show the currently persisted role→model mapping (empty = single-model)
    Show {
        /// Print the mapping as JSON
        #[arg(long)]
        json: bool,
    },
    /// Clear a role's mapping (or all roles), reverting to the single session model
    Clear {
        /// Role to clear; omit to clear every role
        #[arg(long)]
        role: Option<String>,
    },
}
