use std::sync::{Arc, Mutex};
use tokio::time::{sleep, timeout, Duration};
use tracing::{error, info, warn};

use crate::collector_client::CollectorClient;
use crate::config::AgentConfig;
use crate::error::Result;
use crate::metrics::{probe_targets, Snapshot};
use crate::store::ConfigStore;

/// If a single collect+post cycle exceeds this multiple of the configured
/// interval, we abandon it and move on rather than blocking the next cycle.
const CYCLE_TIMEOUT_MULTIPLIER: u32 = 3;

pub struct Agent {
    config: Arc<Mutex<AgentConfig>>,
    store: ConfigStore,
    client: CollectorClient,
}

impl Agent {
    pub fn new(initial_config: AgentConfig, db_path: &str) -> Result<Self> {
        let store = ConfigStore::open(db_path)?;
        store.save(&initial_config)?;
        let client = CollectorClient::new(&initial_config.collector_url);
        Ok(Self {
            config: Arc::new(Mutex::new(initial_config)),
            store,
            client,
        })
    }

    pub async fn run(&self) -> Result<()> {
        loop {
            let cfg = self.config.lock().unwrap().clone();
            let cycle_deadline =
                Duration::from_secs(cfg.interval_secs * CYCLE_TIMEOUT_MULTIPLIER as u64);

            let work = async {
                let pings = probe_targets(&cfg.ping_targets).await;
                match Snapshot::collect(&cfg.agent_id, pings).await {
                    Ok(snapshot) => {
                        if let Err(e) = self.client.post_metrics(&snapshot).await {
                            warn!("Failed to post metrics: {e}");
                        } else {
                            info!("Metrics posted for agent {}", cfg.agent_id);
                        }
                    }
                    Err(e) => error!("Metrics collection error: {e}"),
                }

                match self.client.fetch_config(&cfg.agent_id).await {
                    Ok(Some(remote)) => self.apply_remote_config(remote, &cfg),
                    Ok(None) => {}
                    Err(e) => warn!("Config fetch failed: {e}"),
                }
            };

            if timeout(cycle_deadline, work).await.is_err() {
                warn!(
                    "Cycle exceeded {}s deadline — skipping to next interval",
                    cycle_deadline.as_secs()
                );
            }

            sleep(Duration::from_secs(cfg.interval_secs)).await;
        }
    }

    pub(crate) fn apply_remote_config(
        &self,
        remote: crate::collector_client::RemoteConfig,
        current: &AgentConfig,
    ) {
        let mut candidate = current.clone();
        if let Some(secs) = remote.interval_secs {
            candidate.interval_secs = secs;
        }
        if let Some(targets) = remote.ping_targets {
            candidate.ping_targets = targets;
        }

        match candidate.validate() {
            Ok(()) => {
                if candidate == *current {
                    return; // nothing changed, skip the write and log noise
                }
                if let Err(e) = self.store.save(&candidate) {
                    warn!("Could not persist new config: {e}");
                    return;
                }
                *self.config.lock().unwrap() = candidate;
                info!("Config updated successfully");
            }
            Err(e) => {
                warn!("Received invalid config ({e}), reverting to last known good");
                match self.store.load() {
                    Ok(Some(good)) => *self.config.lock().unwrap() = good,
                    Ok(None) => warn!("No stored config to revert to"),
                    Err(e) => error!("Store read failed during rollback: {e}"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector_client::RemoteConfig;

    fn make_agent(cfg: AgentConfig) -> Agent {
        Agent {
            config: Arc::new(Mutex::new(cfg.clone())),
            store: crate::store::ConfigStore::open(":memory:").unwrap(),
            client: CollectorClient::new(&cfg.collector_url),
        }
    }

    #[test]
    fn bad_remote_config_does_not_replace_good_config() {
        let good = AgentConfig::default(); // interval_secs = 30
        let agent = make_agent(good.clone());
        agent.store.save(&good).unwrap();

        // interval_secs: 0 is invalid — should trigger rollback.
        let bad = RemoteConfig {
            interval_secs: Some(0),
            ping_targets: None,
        };
        agent.apply_remote_config(bad, &good);

        let active = agent.config.lock().unwrap().clone();
        assert_eq!(active.interval_secs, 30, "agent must keep last known good config");
    }

    #[test]
    fn valid_remote_config_is_applied() {
        let initial = AgentConfig::default();
        let agent = make_agent(initial.clone());
        agent.store.save(&initial).unwrap();

        let update = RemoteConfig {
            interval_secs: Some(10),
            ping_targets: Some(vec!["1.1.1.1".into()]),
        };
        agent.apply_remote_config(update, &initial);

        let active = agent.config.lock().unwrap().clone();
        assert_eq!(active.interval_secs, 10);
        assert_eq!(active.ping_targets, vec!["1.1.1.1"]);
    }
}
