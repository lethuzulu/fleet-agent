use std::sync::{Arc, Mutex};
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

use crate::collector_client::CollectorClient;
use crate::config::AgentConfig;
use crate::error::Result;
use crate::metrics::{probe_targets, Snapshot};
use crate::store::ConfigStore;

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

            // Collect metrics and ship them.
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

            // Poll for a config update.
            match self.client.fetch_config(&cfg.agent_id).await {
                Ok(Some(remote)) => self.apply_remote_config(remote, &cfg),
                Ok(None) => {}
                Err(e) => warn!("Config fetch failed: {e}"),
            }

            sleep(Duration::from_secs(cfg.interval_secs)).await;
        }
    }

    fn apply_remote_config(
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
