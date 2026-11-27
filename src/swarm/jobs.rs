//! Suivi borné des tâches longues Swarm-control lancées depuis l'UI (`ask`,
//! `run_agents_parallel`, `run_agent` synchrone).
//!
//! Clé scopée par `Arc::as_ptr(state) as usize` (même pattern que
//! `manage/litemcp.rs` et `manage/scans.rs`) pour isoler les instances de test
//! sans alourdir `AppState`.

use crate::types::{lockv, now_ts, AppState};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const MAX_JOBS_PER_STATE: usize = 64;

static JOB_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
struct SwarmJob {
    id: String,
    kind: String,
    server_id: String,
    agent: Option<String>,
    prompt_preview: String,
    status: String,
    started_ts: u64,
    elapsed_ms: Option<u64>,
    result: Option<Value>,
    error: Option<String>,
}

impl SwarmJob {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "kind": self.kind,
            "server_id": self.server_id,
            "agent": self.agent,
            "prompt_preview": self.prompt_preview,
            "status": self.status,
            "started_ts": self.started_ts,
            "elapsed_ms": self.elapsed_ms,
            "result": self.result,
            "error": self.error,
        })
    }
}

fn jobs_store() -> &'static Mutex<HashMap<usize, VecDeque<SwarmJob>>> {
    static STORE: OnceLock<Mutex<HashMap<usize, VecDeque<SwarmJob>>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn truncate_preview(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, ch) in s.trim().chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

pub fn start_job(
    state: &Arc<AppState>,
    kind: &str,
    server_id: &str,
    agent: Option<String>,
    prompt: &str,
) -> String {
    let seq = JOB_SEQ.fetch_add(1, Ordering::Relaxed);
    let id = format!("swj-{seq}");
    let job = SwarmJob {
        id: id.clone(),
        kind: kind.to_string(),
        server_id: server_id.to_string(),
        agent,
        prompt_preview: truncate_preview(prompt, 120),
        status: "running".to_string(),
        started_ts: now_ts(),
        elapsed_ms: None,
        result: None,
        error: None,
    };
    let key = Arc::as_ptr(state) as usize;
    let mut map = lockv(jobs_store());
    let q = map.entry(key).or_default();
    q.push_front(job);
    while q.len() > MAX_JOBS_PER_STATE {
        q.pop_back();
    }
    id
}

pub fn finish_job(
    state: &Arc<AppState>,
    job_id: &str,
    elapsed_ms: u64,
    outcome: Result<Value, String>,
) {
    let key = Arc::as_ptr(state) as usize;
    let mut map = lockv(jobs_store());
    if let Some(q) = map.get_mut(&key) {
        if let Some(j) = q.iter_mut().find(|j| j.id == job_id) {
            j.elapsed_ms = Some(elapsed_ms);
            match outcome {
                Ok(v) => {
                    j.status = "done".to_string();
                    j.result = Some(v);
                }
                Err(e) => {
                    j.status = "error".to_string();
                    j.error = Some(e);
                }
            }
        }
    }
}

pub fn get_job(state: &Arc<AppState>, job_id: &str) -> Option<Value> {
    let key = Arc::as_ptr(state) as usize;
    let map = lockv(jobs_store());
    map.get(&key)
        .and_then(|q| q.iter().find(|j| j.id == job_id))
        .map(SwarmJob::to_json)
}

pub fn list_jobs(state: &Arc<AppState>) -> Vec<Value> {
    let key = Arc::as_ptr(state) as usize;
    let map = lockv(jobs_store());
    map.get(&key)
        .map(|q| q.iter().take(24).map(SwarmJob::to_json).collect())
        .unwrap_or_default()
}
