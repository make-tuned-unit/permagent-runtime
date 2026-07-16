//! `permagent packs recommend` — the objective model advisor.
//!
//! Discovers the models the user has configured, then recommends the best fit
//! for each workflow role from measured attributes (diff-format reliability,
//! orchestration strength) and cost — with NO vendor bias. Read-only: it prints
//! a recommendation the user can act on; it does not (yet) persist a mapping or
//! change routing (that is the wiring follow-up). The recommendation is fully
//! computed in `permagent::cost_router::recommend`, unit-tested there; this is a
//! thin presentation layer.

use anyhow::Result;
use permagent::cost_router::{
    recommend_configured, recommend_from_available, AvailableModel, Recommendation,
};

pub async fn handle_packs_command(command: PacksCommand) -> Result<()> {
    match command {
        PacksCommand::Recommend { json, models } => recommend_cmd(json, models),
    }
}

fn recommend_cmd(json: bool, models: Option<String>) -> Result<()> {
    // `--models a/b,c/d` overrides auto-detection (declare/confirm your models);
    // otherwise discover from the configured provider surface + the default.
    let result: Recommendation = match models {
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
    };

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
        "\nThis is advisory. Persisting a role→model mapping and routing each role to its \
         model is the wiring follow-up."
    );
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
}
