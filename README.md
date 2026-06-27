# fleet-agent

A cross-platform system monitoring agent written in Rust. A background process runs on each device, continuously collects system and network health metrics, and reports them to a central collector service. Config updates are pushed from the collector and applied live — no restart, no SSH.

## The problem this solves

Managing software on a large fleet of remote devices is hard in a specific way: when something goes wrong, you usually can't reach the device directly. A bad config update that crashes the agent leaves you blind. Fleet-agent is built around two constraints that follow from this:

1. **Config updates must be safe to apply remotely.** If a config fails validation, the agent reverts to the last known-good state persisted on disk rather than crashing or hanging.
2. **The collection loop must not stall.** A slow network call (dead collector, high latency) cannot block the agent indefinitely. Each cycle runs inside a hard timeout; the agent logs a warning and moves on.

---

## Architecture

```
┌──────────────────────────────────────────┐
│              Cargo Workspace             │
│                                          │
│  ┌─────────────┐    ┌─────────────────┐  │
│  │  fleet-core │    │    collector    │  │
│  │  (library)  │    │  (Axum server)  │  │
│  └──────┬──────┘    └─────────────────┘  │
│         │                                │
│  ┌──────┴──────┐   ┌────────────────┐    │
│  │  agent-cli  │   │ agent-service  │    │
│  │ (terminal)  │   │ (systemd/prod) │    │
│  └─────────────┘   └────────────────┘    │
└──────────────────────────────────────────┘
```

| Crate | Role |
|---|---|
| `fleet-core` | Shared library: metric collection, config, SQLite store, HTTP client, run loop |
| `collector` | Axum HTTP server that receives metrics and serves config overrides |
| `agent-cli` | Terminal binary — useful for development and demos |
| `agent-service` | Systemd service binary — reads all config from environment variables |

The same core crate powers both binaries. The only difference between them is how config is loaded and how the process is supervised.

---

## What the agent collects

Each cycle the agent collects and ships a `Snapshot`:

- **CPU** — per-core utilisation % (two sysinfo samples 200ms apart to compute a real delta, not a zero-value first reading)
- **Memory** — total / used / available bytes
- **Network interfaces** — bytes sent and received per interface
- **Latency probes** — RTT to each configured target. Tries ICMP first; falls back to TCP connect latency (port 80 then 443) if `CAP_NET_RAW` is not available. The probe result is tagged with the method used (`"icmp"` or `"tcp"`) so the collector knows what it's looking at.

---

## Config polling and rollback

```
Agent                              Collector
  │                                    │
  │── GET /config/agent-001 ──────────>│
  │<─ { interval_secs: 10,             │
  │     ping_targets: ["1.1.1.1"] } ──│
  │                                    │
  │   validate candidate config        │
  │   ┌─ ok  → persist to SQLite,      │
  │   │         apply in memory,        │
  │   │         no restart needed       │
  │   └─ err → load last known-good    │
  │             from SQLite, continue  │
  │                                    │
  │── POST /metrics ──────────────────>│
  │<─ 200 OK ─────────────────────────│
```

The last-known-good config survives process restarts via SQLite (`rusqlite`, bundled). If the store itself is unreadable during a rollback, the agent logs the error and continues on whatever is in memory — it does not crash.

---

## How to run

Requires Rust 1.75+.

**Terminal (two shells):**

```bash
# Shell 1 — start the collector
cargo run -p collector

# Shell 2 — start the agent
RUST_LOG=info cargo run -p agent-cli
```

The collector listens on `0.0.0.0:3000`. The agent defaults to `http://localhost:3000`.

**Push a config override:**

```bash
curl -X PUT http://localhost:3000/config/agent-001 \
  -H "Content-Type: application/json" \
  -d '{"interval_secs": 10, "ping_targets": ["8.8.8.8", "1.1.1.1"]}'
```

The agent picks this up on its next poll and applies it without restarting.

**View the dashboard:**

```bash
curl http://localhost:3000/dashboard | jq
```

Returns the latest snapshot for every known agent.

**As a systemd service:**

```bash
# Build the service binary
cargo build --release -p agent-service

# Install
sudo cp target/release/agent-service /usr/local/bin/
sudo cp deploy/fleet-agent.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now fleet-agent
```

Override config via a drop-in or environment file:

```
Environment=AGENT_ID=device-42
Environment=COLLECTOR_URL=http://collector.internal:3000
Environment=INTERVAL_SECS=30
Environment=PING_TARGETS=8.8.8.8,1.1.1.1
```

---

## Collector endpoints

| Method | Path | Description |
|---|---|---|
| `POST` | `/metrics` | Receive a `Snapshot` from an agent |
| `GET` | `/config/:agent_id` | Serve the current config override for an agent (404 if none) |
| `PUT` | `/config/:agent_id` | Set or update a config override for an agent |
| `GET` | `/dashboard` | Latest snapshot per agent |

---

## Tests

```bash
cargo test -p fleet-core
```

8 tests covering:
- Config validation rejects zero interval, empty agent ID, empty collector URL
- Default config is valid out of the box
- A bad remote config does not replace the active config (rollback assertion)
- A valid remote config is applied and persisted
- `Snapshot` survives a JSON serialise/deserialise round-trip
- SQLite store persists and reloads config correctly

---

## Design decisions

**Why SQLite for local state?**
The agent needs to survive a crash and recover to a known-good config without any network access. SQLite is a single file, has no daemon, and `rusqlite` bundles the C library so there are no system dependencies. The trade-off is a small binary size increase; for a monitoring agent this is acceptable.

**Why config polling instead of push?**
Push (the collector sending updates to the agent) requires the collector to know each agent's address and maintain an open connection. On a fleet of remote devices behind NAT, this is often impossible. Polling inverts the dependency: the agent only needs outbound HTTP access to one known endpoint.

**Why a cycle timeout instead of per-call timeouts?**
Individual calls (ping, HTTP post, config fetch) already have their own timeouts. The cycle timeout is a belt-and-suspenders guard: if the code path acquires a Tokio task that never resolves due to a bug or an OS-level hang, the agent still wakes up after `3 × interval_secs` and logs a warning. This is the difference between an agent that occasionally misses a cycle and an agent that silently hangs forever on a device with no operator access.

**Why one core library shared between `agent-cli` and `agent-service`?**
The binaries differ only in how config is loaded (hardcoded defaults vs. environment variables) and how stdout is formatted (ANSI vs. plain for journald). All behaviour — collection, polling, rollback — lives in `fleet-core` and is tested there. Adding a Windows service target later means writing one new `main.rs`, not porting logic.

---

## What's not here yet

- **TLS** — all HTTP is plaintext. Production would need mTLS between agent and collector so neither side accepts impersonation.
- **Authentication** — the `PUT /config/:agent_id` endpoint is unauthenticated. In production this needs at minimum a shared secret header, ideally agent certificates.
- **Windows service wrapper** — `agent-service` compiles on Windows but doesn't register with the Service Control Manager. The `windows-service` crate covers this.
- **Metrics storage limits** — the collector accumulates rows indefinitely. A TTL-based purge job is needed for any deployment longer than a day.
