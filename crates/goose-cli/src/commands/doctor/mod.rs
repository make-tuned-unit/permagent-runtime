mod checks;
mod output;
mod types;

use anyhow::Result;

pub use types::{CheckResult, CheckStatus};

pub async fn handle_doctor(json: bool, interactive: bool) -> Result<()> {
    if interactive {
        // Preserve legacy LLM-assisted diagnosis mode.
        use crate::session::build_session;
        use crate::session::SessionBuilderConfig;
        let mut session = build_session(SessionBuilderConfig {
            no_session: true,
            interactive: true,
            ..Default::default()
        })
        .await;
        return session.interactive(Some("/doctor".to_string())).await;
    }

    let results = checks::run_all().await;
    let has_fail = results.iter().any(|r| r.status == CheckStatus::Fail);

    if json {
        let json_out = serde_json::to_string_pretty(&results)?;
        println!("{json_out}");
    } else {
        output::print_table(&results);
    }

    if has_fail {
        std::process::exit(1);
    }

    Ok(())
}
