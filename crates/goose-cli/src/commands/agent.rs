use anyhow::Result;
use permagent::config::agent_identity::{self, AgentConfig, PrimaryPersona};

pub async fn handle_agent_command(command: AgentCommand) -> Result<()> {
    match command {
        AgentCommand::Show => show(),
        AgentCommand::SetFirstName { name } => set_field(|p| p.first_name = name),
        AgentCommand::SetLastName { name } => set_field(|p| p.last_name = Some(name)),
        AgentCommand::SetNickname { name } => set_field(|p| p.nickname = Some(name)),
        AgentCommand::ClearNickname => set_field(|p| p.nickname = None),
        AgentCommand::SetTone { tone } => set_field(|p| p.tone = tone),
        AgentCommand::SetGreeting { greeting } => set_field(|p| p.opening_greeting = greeting),
    }
}

fn show() -> Result<()> {
    let config = agent_identity::load_agent_config();
    let p = &config.primary;
    println!("Agent Identity");
    println!("  first_name:       {}", p.first_name);
    println!(
        "  last_name:        {}",
        p.last_name.as_deref().unwrap_or("(none)")
    );
    println!(
        "  nickname:         {}",
        p.nickname.as_deref().unwrap_or("(none)")
    );
    println!("  display_name:     {}", p.display_name());
    println!(
        "  traits:           {}",
        if p.traits.is_empty() {
            "(none)".into()
        } else {
            p.traits.join(", ")
        }
    );
    println!(
        "  tone:             {}",
        if p.tone.is_empty() { "(none)" } else { &p.tone }
    );
    println!(
        "  opening_greeting: {}",
        if p.opening_greeting.is_empty() {
            "(none)"
        } else {
            &p.opening_greeting
        }
    );
    println!(
        "  voice_id:         {}",
        p.voice_id.as_deref().unwrap_or("(none)")
    );
    Ok(())
}

fn set_field(f: impl FnOnce(&mut PrimaryPersona)) -> Result<()> {
    let mut config = agent_identity::load_agent_config();
    f(&mut config.primary);
    agent_identity::save_agent_config(&config)?;
    println!("Updated. display_name: {}", config.primary.display_name());
    Ok(())
}

#[derive(clap::Subcommand, Debug)]
pub enum AgentCommand {
    /// Show current agent identity
    Show,
    /// Set the agent's first name
    #[command(name = "set-first-name")]
    SetFirstName { name: String },
    /// Set the agent's last name
    #[command(name = "set-last-name")]
    SetLastName { name: String },
    /// Set a nickname (overrides display name)
    #[command(name = "set-nickname")]
    SetNickname { name: String },
    /// Clear the nickname (revert to first+last)
    #[command(name = "clear-nickname")]
    ClearNickname,
    /// Set the agent's tone description
    #[command(name = "set-tone")]
    SetTone { tone: String },
    /// Set the agent's opening greeting
    #[command(name = "set-greeting")]
    SetGreeting { greeting: String },
}
