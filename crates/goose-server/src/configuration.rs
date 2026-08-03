use crate::error::ConfigError;
use serde::Deserialize;
use std::net::SocketAddr;

#[derive(Debug, Default, Deserialize)]
pub struct Settings {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub tls_cert_path: Option<String>,
    pub tls_key_path: Option<String>,
}

/// The current Tailscale IPv4 for this machine, or None if Tailscale is absent,
/// logged out, or has no v4 address.
///
/// NOT used to choose a bind address — the daemon deliberately stays on
/// localhost and is fronted by `tailscale serve` (Tailscale's own guidance: a
/// backend reachable directly on the tailnet can have its identity headers
/// spoofed by any tailnet peer). This exists so the Devices panel can report
/// the machine's tailnet identity.
pub fn detect_tailscale_ipv4() -> Option<String> {
    let candidates = [
        "tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    ];
    for bin in candidates {
        let Ok(out) = std::process::Command::new(bin)
            .args(["status", "--json"])
            .output()
        else {
            continue;
        };
        if !out.status.success() {
            continue;
        }
        let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
        if parsed.pointer("/BackendState").and_then(|v| v.as_str()) != Some("Running") {
            return None;
        }
        let ip = parsed
            .pointer("/Self/TailscaleIPs")
            .and_then(|v| v.as_array())
            .and_then(|ips| {
                ips.iter()
                    .filter_map(|v| v.as_str())
                    .find(|s| s.parse::<std::net::Ipv4Addr>().is_ok())
            })
            .map(str::to_string);
        return ip;
    }
    None
}

impl Settings {
    pub fn socket_addr(&self) -> SocketAddr {
        format!("{}:{}", self.host, self.port)
            .parse()
            .expect("Failed to parse socket address")
    }

    pub fn new() -> Result<Self, ConfigError> {
        Self::load_and_validate()
    }

    /// Apply CLI flag overrides (highest priority).
    pub fn with_overrides(mut self, host: Option<String>, port: Option<u16>) -> Self {
        if let Some(h) = host {
            self.host = h;
        }
        if let Some(p) = port {
            self.port = p;
        }
        self
    }

    fn load_and_validate() -> Result<Self, ConfigError> {
        // ── Defaults ──
        let mut settings = Settings {
            host: default_host(),
            port: default_port(),
            tls: default_tls(),
            tls_cert_path: None,
            tls_key_path: None,
        };

        // ── Layer 1: config.yaml daemon section (PERMAGENT_CONFIG) ──
        if let Ok(config_path) = std::env::var("PERMAGENT_CONFIG") {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                if let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
                    if let Some(daemon) = yaml.get("daemon") {
                        if let Some(h) = daemon.get("host").and_then(|v| v.as_str()) {
                            settings.host = h.to_string();
                        }
                        if let Some(p) = daemon.get("port").and_then(|v| v.as_u64()) {
                            settings.port = p as u16;
                        }
                        if let Some(t) = daemon.get("tls").and_then(|v| v.as_bool()) {
                            settings.tls = t;
                        }
                    }
                }
            }
        }

        // ── Layer 2: env vars (HOST/PORT override config.yaml) ──
        if let Ok(h) = std::env::var("HOST") {
            settings.host = h;
        }
        if let Ok(p) = std::env::var("PORT") {
            if let Ok(p) = p.parse() {
                settings.port = p;
            }
        }

        // GOOSE_* prefixed vars for backwards compatibility
        if let Ok(h) = std::env::var("GOOSE_HOST") {
            settings.host = h;
        }
        if let Ok(p) = std::env::var("GOOSE_PORT") {
            if let Ok(p) = p.parse() {
                settings.port = p;
            }
        }
        if let Ok(t) = std::env::var("GOOSE_TLS") {
            if let Ok(t) = t.parse() {
                settings.tls = t;
            }
        }

        settings.tls_cert_path = std::env::var("GOOSE_TLS_CERT_PATH").ok();
        settings.tls_key_path = std::env::var("GOOSE_TLS_KEY_PATH").ok();

        Ok(settings)
    }
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    3001
}

fn default_tls() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_socket_addr_conversion() {
        let server_settings = Settings {
            host: "127.0.0.1".to_string(),
            port: 3001,
            tls: false,
            tls_cert_path: None,
            tls_key_path: None,
        };
        let addr = server_settings.socket_addr();
        assert_eq!(addr.to_string(), "127.0.0.1:3001");
    }
}
