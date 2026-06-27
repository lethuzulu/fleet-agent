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
    /// "icmp" when CAP_NET_RAW is available, "tcp" otherwise.
    pub method: String,
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
    pub async fn collect(agent_id: &str, ping_results: Vec<PingResult>) -> Result<Self> {
        let mut sys = System::new_all();
        sys.refresh_cpu_usage();
        // sysinfo computes CPU usage as a delta between two samples; without
        // this pause the first reading is always 0%.
        tokio::time::sleep(Duration::from_millis(200)).await;
        sys.refresh_cpu_usage();
        sys.refresh_memory();

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
        results.push(probe_one(target).await);
    }
    results
}

async fn probe_one(host: &str) -> PingResult {
    // Try ICMP first; it gives the most accurate RTT but needs CAP_NET_RAW.
    if let Some(rtt_ms) = icmp_ping(host).await {
        return PingResult { target: host.to_string(), rtt_ms: Some(rtt_ms), method: "icmp".into() };
    }

    // Fall back to TCP connect latency — no special permissions required.
    let rtt_ms = tcp_rtt(host).await;
    PingResult { target: host.to_string(), rtt_ms, method: "tcp".into() }
}

async fn icmp_ping(host: &str) -> Option<f64> {
    use std::net::IpAddr;
    let ip: IpAddr = host.parse().ok()?;
    let payload = [0u8; 8];
    let client = surge_ping::Client::new(&surge_ping::Config::default()).ok()?;
    let mut pinger = client.pinger(ip, surge_ping::PingIdentifier(1)).await;
    pinger.timeout(Duration::from_secs(2));
    match pinger.ping(surge_ping::PingSequence(0), &payload).await {
        Ok((_reply, rtt)) => Some(rtt.as_secs_f64() * 1000.0),
        Err(_) => None,
    }
}

/// Measures TCP handshake RTT to port 80, then 443 as a fallback.
/// Works without any elevated permissions.
async fn tcp_rtt(host: &str) -> Option<f64> {
    for port in [80u16, 443] {
        let addr = format!("{host}:{port}");
        let start = std::time::Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            tokio::net::TcpStream::connect(&addr),
        )
        .await;
        if let Ok(Ok(_)) = result {
            return Some(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_serialises_and_deserialises() {
        let snap = Snapshot {
            agent_id: "test-agent".into(),
            timestamp_ms: 1_000_000,
            cpu: CpuMetrics { core_usage: vec![12.5, 34.0], global_usage: 23.25 },
            memory: MemoryMetrics {
                total_bytes: 8 * 1024 * 1024 * 1024,
                used_bytes: 4 * 1024 * 1024 * 1024,
                available_bytes: 4 * 1024 * 1024 * 1024,
            },
            interfaces: vec![NetworkInterface {
                name: "eth0".into(),
                bytes_sent: 1000,
                bytes_recv: 2000,
                is_up: true,
            }],
            pings: vec![PingResult {
                target: "8.8.8.8".into(),
                rtt_ms: Some(12.5),
                method: "tcp".into(),
            }],
        };

        let json = serde_json::to_string(&snap).expect("serialise");
        let back: Snapshot = serde_json::from_str(&json).expect("deserialise");

        assert_eq!(back.agent_id, snap.agent_id);
        assert_eq!(back.cpu.global_usage, snap.cpu.global_usage);
        assert_eq!(back.pings[0].method, "tcp");
    }
}
