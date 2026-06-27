use fleet_core::{agent::Agent, config::AgentConfig};
use tracing::info;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let config = AgentConfig::default();
    info!(
        agent_id = %config.agent_id,
        collector = %config.collector_url,
        interval_secs = config.interval_secs,
        "Starting fleet agent (CLI mode)"
    );

    let agent = Agent::new(config, "agent-cli.db").expect("failed to initialise agent");
    if let Err(e) = agent.run().await {
        eprintln!("Agent error: {e}");
        std::process::exit(1);
    }
}
