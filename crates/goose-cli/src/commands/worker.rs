use anyhow::Result;
use permagent::config::agent_identity::{self, WorkerPersona};

pub async fn handle_worker_command(command: WorkerCommand) -> Result<()> {
    match command {
        WorkerCommand::List => list(),
        WorkerCommand::Show { key } => show(&key),
        WorkerCommand::Create {
            key,
            first_name,
            role,
            tone,
            traits,
        } => create(&key, &first_name, &role, &tone, &traits),
        WorkerCommand::SetNickname { key, name } => set_field(&key, |w| w.nickname = Some(name)),
        WorkerCommand::ClearNickname { key } => set_field(&key, |w| w.nickname = None),
        WorkerCommand::Remove { key } => remove(&key),
    }
}

fn list() -> Result<()> {
    let config = agent_identity::load_agent_config();
    if config.workers.is_empty() {
        println!("No workers defined.");
        return Ok(());
    }
    println!("{:<15} {:<20} ROLE", "KEY", "DISPLAY_NAME");
    let mut keys: Vec<_> = config.workers.keys().collect();
    keys.sort();
    for key in keys {
        let w = &config.workers[key];
        println!("{:<15} {:<20} {}", key, w.display_name(), w.role);
    }
    Ok(())
}

fn show(key: &str) -> Result<()> {
    let config = agent_identity::load_agent_config();
    let w = config
        .workers
        .get(key)
        .ok_or_else(|| anyhow::anyhow!("Worker '{}' not found", key))?;
    println!("Worker: {}", key);
    println!("  first_name:   {}", w.first_name);
    println!(
        "  last_name:    {}",
        w.last_name.as_deref().unwrap_or("(none)")
    );
    println!(
        "  nickname:     {}",
        w.nickname.as_deref().unwrap_or("(none)")
    );
    println!("  display_name: {}", w.display_name());
    println!("  role:         {}", w.role);
    println!(
        "  traits:       {}",
        if w.traits.is_empty() {
            "(none)".into()
        } else {
            w.traits.join(", ")
        }
    );
    println!(
        "  tone:         {}",
        if w.tone.is_empty() { "(none)" } else { &w.tone }
    );
    Ok(())
}

fn create(key: &str, first_name: &str, role: &str, tone: &str, traits: &str) -> Result<()> {
    let mut config = agent_identity::load_agent_config();
    if config.workers.contains_key(key) {
        anyhow::bail!("Worker '{}' already exists. Use set-* to modify.", key);
    }
    let trait_list: Vec<String> = traits
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let worker = WorkerPersona {
        first_name: first_name.to_string(),
        role: role.to_string(),
        traits: trait_list,
        tone: tone.to_string(),
        ..Default::default()
    };
    println!("Created worker '{}': {}", key, worker.display_name());
    config.workers.insert(key.to_string(), worker);
    agent_identity::save_agent_config(&config)?;
    Ok(())
}

fn set_field(key: &str, f: impl FnOnce(&mut WorkerPersona)) -> Result<()> {
    let mut config = agent_identity::load_agent_config();
    let worker = config
        .workers
        .get_mut(key)
        .ok_or_else(|| anyhow::anyhow!("Worker '{}' not found", key))?;
    f(worker);
    println!("Updated. display_name: {}", worker.display_name());
    agent_identity::save_agent_config(&config)?;
    Ok(())
}

fn remove(key: &str) -> Result<()> {
    let mut config = agent_identity::load_agent_config();
    if config.workers.remove(key).is_none() {
        anyhow::bail!("Worker '{}' not found", key);
    }
    agent_identity::save_agent_config(&config)?;
    println!("Removed worker '{}'", key);
    Ok(())
}

#[derive(clap::Subcommand, Debug)]
pub enum WorkerCommand {
    /// List all workers
    List,
    /// Show a worker's details
    Show { key: String },
    /// Create a new worker
    Create {
        key: String,
        #[arg(long)]
        first_name: String,
        #[arg(long)]
        role: String,
        #[arg(long, default_value = "")]
        tone: String,
        #[arg(long, default_value = "")]
        traits: String,
    },
    /// Set a worker's nickname
    #[command(name = "set-nickname")]
    SetNickname { key: String, name: String },
    /// Clear a worker's nickname
    #[command(name = "clear-nickname")]
    ClearNickname { key: String },
    /// Remove a worker
    Remove { key: String },
}
