use reqwest::Client;
use serde::Deserialize;

use crate::error::Result;
use crate::metrics::Snapshot;

#[derive(Debug, Deserialize)]
pub struct RemoteConfig {
    pub interval_secs: Option<u64>,
    pub ping_targets: Option<Vec<String>>,
}

pub struct CollectorClient {
    client: Client,
    base_url: String,
}

impl CollectorClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    pub async fn post_metrics(&self, snapshot: &Snapshot) -> Result<()> {
        self.client
            .post(format!("{}/metrics", self.base_url))
            .json(snapshot)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// Fetches a remote config update for this agent. Returns None if the
    /// collector has no override for this agent.
    pub async fn fetch_config(&self, agent_id: &str) -> Result<Option<RemoteConfig>> {
        let resp = self
            .client
            .get(format!("{}/config/{}", self.base_url, agent_id))
            .send()
            .await?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let cfg = resp.error_for_status()?.json::<RemoteConfig>().await?;
        Ok(Some(cfg))
    }
}
