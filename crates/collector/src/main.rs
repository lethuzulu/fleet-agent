use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tracing::info;

use fleet_core::metrics::Snapshot;

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ConfigOverride {
    interval_secs: Option<u64>,
    ping_targets: Option<Vec<String>>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let conn = Connection::open("collector.db").expect("failed to open database");
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS metrics (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id   TEXT NOT NULL,
            ts_ms      INTEGER NOT NULL,
            payload    TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS config_overrides (
            agent_id TEXT PRIMARY KEY,
            payload  TEXT NOT NULL
        );",
    )
    .expect("schema migration failed");

    let state = AppState {
        db: Arc::new(Mutex::new(conn)),
    };

    let app = Router::new()
        .route("/metrics", post(receive_metrics))
        .route("/config/:agent_id", get(get_config).put(put_config))
        .route("/dashboard", get(dashboard))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .expect("failed to bind");
    info!("Collector listening on 0.0.0.0:3000");
    axum::serve(listener, app).await.unwrap();
}

async fn receive_metrics(
    State(state): State<AppState>,
    Json(snapshot): Json<Snapshot>,
) -> impl IntoResponse {
    let payload = match serde_json::to_string(&snapshot) {
        Ok(p) => p,
        Err(_) => return StatusCode::BAD_REQUEST,
    };
    let db = state.db.lock().unwrap();
    match db.execute(
        "INSERT INTO metrics (agent_id, ts_ms, payload) VALUES (?1, ?2, ?3)",
        params![snapshot.agent_id, snapshot.timestamp_ms as i64, payload],
    ) {
        Ok(_) => {
            info!("Received metrics from {}", snapshot.agent_id);
            StatusCode::OK
        }
        Err(e) => {
            tracing::error!("DB insert failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

async fn get_config(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let db = state.db.lock().unwrap();
    let mut stmt = db
        .prepare("SELECT payload FROM config_overrides WHERE agent_id = ?1")
        .unwrap();
    let mut rows = stmt.query(params![agent_id]).unwrap();

    match rows.next().unwrap() {
        Some(row) => {
            let json_str: String = row.get(0).unwrap();
            let val: Value = serde_json::from_str(&json_str).unwrap_or(json!({}));
            (StatusCode::OK, Json(val)).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn put_config(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<ConfigOverride>,
) -> impl IntoResponse {
    let payload = match serde_json::to_string(&body) {
        Ok(p) => p,
        Err(_) => return StatusCode::BAD_REQUEST,
    };
    let db = state.db.lock().unwrap();
    match db.execute(
        "INSERT INTO config_overrides (agent_id, payload) VALUES (?1, ?2)
         ON CONFLICT(agent_id) DO UPDATE SET payload = excluded.payload",
        params![agent_id, payload],
    ) {
        Ok(_) => {
            info!("Config override set for {agent_id}");
            StatusCode::OK
        }
        Err(e) => {
            tracing::error!("DB write failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

async fn dashboard(State(state): State<AppState>) -> impl IntoResponse {
    let db = state.db.lock().unwrap();
    let mut stmt = db
        .prepare(
            "SELECT agent_id, payload FROM metrics
             WHERE ts_ms = (SELECT MAX(ts_ms) FROM metrics m2 WHERE m2.agent_id = metrics.agent_id)
             GROUP BY agent_id",
        )
        .unwrap();

    let rows: Vec<Value> = stmt
        .query_map([], |row| {
            let agent_id: String = row.get(0)?;
            let payload: String = row.get(1)?;
            Ok((agent_id, payload))
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .map(|(agent_id, payload)| {
            let snapshot: Value = serde_json::from_str(&payload).unwrap_or(json!({}));
            json!({ "agent_id": agent_id, "last_snapshot": snapshot })
        })
        .collect();

    Json(json!({ "agents": rows }))
}
