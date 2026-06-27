use serde::{Deserialize, Serialize};

use crate::error::{AgentError, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentConfig {
    /// How often the agent collects and reports metrics, in seconds.
    pub interval_secs: u64,
    /// Hosts to probe for latency (e.g. ["8.8.8.8", "1.1.1.1"]).
    pub ping_targets: Vec<String>,
    /// URL of the collector service.
    pub collector_url: String,
    /// Unique identifier for this agent instance.
    pub agent_id: String,
}

impl AgentConfig {
    pub fn validate(&self) -> Result<()> {
        if self.interval_secs == 0 {
            return Err(AgentError::Config(
                "interval_secs must be greater than 0".into(),
            ));
        }
        if self.collector_url.is_empty() {
            return Err(AgentError::Config("collector_url must not be empty".into()));
        }
        if self.agent_id.is_empty() {
            return Err(AgentError::Config("agent_id must not be empty".into()));
        }
        Ok(())
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            interval_secs: 30,
            ping_targets: vec!["8.8.8.8".into()],
            collector_url: "http://localhost:3000".into(),
            agent_id: "agent-001".into(),
        }
    }
}
