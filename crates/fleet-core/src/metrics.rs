use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use sysinfo::System;

use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuMetrics {
    /// Per-core utilisation as a percentage (0.0–100.0).
    pub core_usage: Vec<f32>,
    pub global_usage: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryMetrics {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterface {
    pub name: String,
    pub bytes_sent: u64,
    pub bytes_recv: u64,
    pub is_up: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingResult {
    pub target: String,
    /// Round-trip time in milliseconds, or None if the host was unreachable.
    pub rtt_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub agent_id: String,
    pub timestamp_ms: u64,
    pub cpu: CpuMetrics,
    pub memory: MemoryMetrics,
    pub interfaces: Vec<NetworkInterface>,
    pub pings: Vec<PingResult>,
}

impl Snapshot {
    pub fn collect(agent_id: &str, ping_results: Vec<PingResult>) -> Result<Self> {
        let mut sys = System::new_all();
        sys.refresh_all();

        let core_usage: Vec<f32> = sys.cpus().iter().map(|c| c.cpu_usage()).collect();
        let global_usage = if core_usage.is_empty() {
            0.0
        } else {
            core_usage.iter().sum::<f32>() / core_usage.len() as f32
        };

        let interfaces = sysinfo::Networks::new_with_refreshed_list()
            .iter()
            .map(|(name, data)| NetworkInterface {
                name: name.clone(),
                bytes_sent: data.total_transmitted(),
                bytes_recv: data.total_received(),
                is_up: true,
            })
            .collect();

        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_millis() as u64;

        Ok(Self {
            agent_id: agent_id.to_string(),
            timestamp_ms,
            cpu: CpuMetrics {
                core_usage,
                global_usage,
            },
            memory: MemoryMetrics {
                total_bytes: sys.total_memory(),
                used_bytes: sys.used_memory(),
                available_bytes: sys.available_memory(),
            },
            interfaces,
            pings: ping_results,
        })
    }
}

pub async fn probe_targets(targets: &[String]) -> Vec<PingResult> {
    let mut results = Vec::with_capacity(targets.len());

    for target in targets {
        let rtt_ms = ping_once(target).await;
        results.push(PingResult {
            target: target.clone(),
            rtt_ms,
        });
    }

    results
}

async fn ping_once(host: &str) -> Option<f64> {
    use std::net::IpAddr;

    let ip: IpAddr = match host.parse() {
        Ok(ip) => ip,
        Err(_) => return None,
    };

    let payload = [0u8; 8];
    let client = surge_ping::Client::new(&surge_ping::Config::default()).ok()?;
    let mut pinger = client.pinger(ip, surge_ping::PingIdentifier(1)).await;
    pinger.timeout(Duration::from_secs(2));

    match pinger.ping(surge_ping::PingSequence(0), &payload).await {
        Ok((_reply, rtt)) => Some(rtt.as_secs_f64() * 1000.0),
        Err(_) => None,
    }
}
