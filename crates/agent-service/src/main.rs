use fleet_core::{agent::Agent, config::AgentConfig};
use tracing::info;

/// Entry point when running as a systemd service.
/// Config is read from environment variables so ops can inject values
/// without rebuilding the binary.
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_ansi(false) // systemd journal does not render ANSI
        .init();

    let config = AgentConfig {
        agent_id: std::env::var("AGENT_ID").unwrap_or_else(|_| "agent-001".into()),
        collector_url: std::env::var("COLLECTOR_URL")
            .unwrap_or_else(|_| "http://localhost:3000".into()),
        interval_secs: std::env::var("INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30),
        ping_targets: std::env::var("PING_TARGETS")
            .map(|v| v.split(',').map(str::to_string).collect())
            .unwrap_or_else(|_| vec!["8.8.8.8".into()]),
    };

    info!(
        agent_id = %config.agent_id,
        collector = %config.collector_url,
        "Starting fleet agent (service mode)"
    );

    let agent = Agent::new(config, "/var/lib/fleet-agent/state.db")
        .expect("failed to initialise agent");
    if let Err(e) = agent.run().await {
        eprintln!("Agent error: {e}");
        std::process::exit(1);
    }
}
