//! Actions locales `swarm_*` (réservées `ADMIN_ACTIONS`, jamais exposées au
//! gateway `/mcp` ni au LAN).

use super::detect::{detect_server, SwarmKind};
use super::jobs;
use super::rpc::{call_bridge_loopback, call_server_mcp, unwrap_tool_json};
use crate::manage;
use crate::types::{lockv, AppState, SState, Transport};
use serde_json::{json, Map, Value};
use std::sync::Arc;
use std::time::Instant;

fn req_str<'a>(req: &'a Value, key: &str) -> Option<&'a str> {
    req.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn server_url_str(transport: &Transport, bound_host: Option<&str>) -> Option<String> {
    match transport {
        Transport::Http { host, port, path } => {
            let h = bound_host.or(host.as_deref()).unwrap_or("127.0.0.1");
            let p = if path.starts_with('/') {
                path.as_str()
            } else {
                "/mcp"
            };
            Some(format!("http://{h}:{port}{p}"))
        }
        Transport::Stdio { .. } => None,
    }
}

fn parse_http_url_parts(url: &str) -> Option<(Option<String>, u16, String)> {
    let rest = url
        .trim()
        .strip_prefix("http://")
        .or_else(|| url.trim().strip_prefix("https://"))?;
    let (hostport, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/mcp"),
    };
    let (host, port_s) = hostport.rsplit_once(':')?;
    let port = port_s.parse::<u16>().ok()?;
    let h = if crate::is_loopback_bind(host) {
        None
    } else {
        Some(host.to_string())
    };
    Some((h, port, path.to_string()))
}

fn call_and_unwrap(
    state: &Arc<AppState>,
    server_id: &str,
    tool: &str,
    args: Value,
    timeout_ms: u64,
) -> Result<(Value, u64), String> {
    let (raw, elapsed_ms) = call_server_mcp(state, server_id, tool, args, timeout_ms)?;
    let data = unwrap_tool_json(&raw)?;
    Ok((data, elapsed_ms))
}

fn build_overview(state: &Arc<AppState>) -> Value {
    struct InstanceSnap {
        id: String,
        label: String,
        state: SState,
        url: Option<String>,
        version: Option<String>,
        instance: Option<String>,
        workspace: Option<String>,
        locked: bool,
        paired: bool,
    }
    struct ResidentSnap {
        id: String,
        label: String,
        state: SState,
        url: Option<String>,
        version: Option<String>,
        agent: Option<String>,
        bridge_port: Option<u16>,
        locked: bool,
        paired: bool,
    }

    let (instances_snap, residents_snap, mcp_targets) = {
        let servers = lockv(&state.servers);
        let mut insts = Vec::new();
        let mut ress = Vec::new();
        let mut targets = Vec::new();
        for s in servers.iter() {
            let url = server_url_str(&s.transport, s.bound_host.as_deref());
            let auth = crate::auth::net::server_auth_status(s);
            let meta = detect_server(s);
            if let Some(ref u) = url {
                targets.push(json!({
                    "id": s.id,
                    "label": s.label,
                    "url": u,
                    "state": s.state.as_str(),
                    "swarm_kind": meta.as_ref().map(|m| m.kind.as_str()),
                }));
            }
            if let Some(m) = meta {
                match m.kind {
                    SwarmKind::Instance => insts.push(InstanceSnap {
                        id: s.id.clone(),
                        label: s.label.clone(),
                        state: s.state,
                        url,
                        version: s.version.clone(),
                        instance: m.instance,
                        workspace: m.workspace,
                        locked: auth.locked,
                        paired: auth.paired,
                    }),
                    SwarmKind::Resident => ress.push(ResidentSnap {
                        id: s.id.clone(),
                        label: s.label.clone(),
                        state: s.state,
                        url,
                        version: s.version.clone(),
                        agent: m.agent,
                        bridge_port: m.bridge_port,
                        locked: auth.locked,
                        paired: auth.paired,
                    }),
                }
            }
        }
        (insts, ress, targets)
    };

    // Interrogation parallèle bornée (3,5 s max) des instances UP et résidents UP.
    let (instances_json, residents_json) = std::thread::scope(|scope| {
        let inst_handles: Vec<_> = instances_snap
            .iter()
            .map(|inst| {
                let st = Arc::clone(state);
                scope.spawn(move || {
                    let mut agents = Value::Array(vec![]);
                    let mut runs = Value::Array(vec![]);
                    let mut error: Option<String> = None;
                    if inst.state == SState::Up {
                        match call_and_unwrap(
                            &st,
                            &inst.id,
                            "list_agents",
                            json!({"details": true}),
                            3500,
                        ) {
                            Ok((v, _)) => {
                                if let Some(arr) = v.get("agents").and_then(Value::as_array) {
                                    agents = Value::Array(arr.clone());
                                }
                            }
                            Err(e) => error = Some(e),
                        }
                        if let Ok((v, _)) = call_and_unwrap(
                            &st,
                            &inst.id,
                            "agent_control",
                            json!({"action": "list"}),
                            3000,
                        ) {
                            if let Some(arr) = v
                                .get("runs")
                                .or_else(|| v.get("agents"))
                                .and_then(Value::as_array)
                            {
                                runs = Value::Array(arr.clone());
                            } else if v.is_array() {
                                runs = v;
                            }
                        }
                    }
                    json!({
                        "id": inst.id,
                        "label": inst.label,
                        "state": inst.state.as_str(),
                        "url": inst.url,
                        "version": inst.version,
                        "instance": inst.instance,
                        "workspace": inst.workspace,
                        "locked": inst.locked,
                        "paired": inst.paired,
                        "agents": agents,
                        "runs": runs,
                        "error": error,
                    })
                })
            })
            .collect();

        let res_handles: Vec<_> = residents_snap
            .iter()
            .map(|r| {
                let st = Arc::clone(state);
                scope.spawn(move || {
                    let mut status_val = Value::Null;
                    let mut error: Option<String> = None;
                    if r.state == SState::Up {
                        match call_and_unwrap(&st, &r.id, "status", json!({}), 3000) {
                            Ok((v, _)) => status_val = v,
                            Err(e) => error = Some(e),
                        }
                    }
                    json!({
                        "id": r.id,
                        "label": r.label,
                        "state": r.state.as_str(),
                        "url": r.url,
                        "version": r.version,
                        "agent": r.agent,
                        "bridge_port": r.bridge_port,
                        "locked": r.locked,
                        "paired": r.paired,
                        "status": status_val,
                        "error": error,
                    })
                })
            })
            .collect();

        let insts: Vec<Value> = inst_handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .collect();
        let ress: Vec<Value> = res_handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .collect();
        (insts, ress)
    });

    // Annonces LiteMCP Swarm non suivies (lecture seule, Audit S4).
    let ann_all = manage::litemcp_announcements(state);
    let announcements: Vec<Value> = ann_all["announcements"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|a| a.get("swarm_kind").and_then(Value::as_str).is_some())
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    json!({
        "ok": true,
        "instances": instances_json,
        "residents": residents_json,
        "announcements": announcements,
        "jobs": jobs::list_jobs(state),
        "mcp_targets": mcp_targets,
    })
}

pub fn handle_swarm_action(state: &Arc<AppState>, action: &str, req: &Value) -> Option<Value> {
    let out = match action {
        "swarm_overview" => build_overview(state),

        "swarm_create_agent" => {
            let Some(inst) = req_str(req, "instance_id") else {
                return Some(json!({"ok": false, "error": "instance_id manquant"}));
            };
            let Some(name) = req_str(req, "name") else {
                return Some(json!({"ok": false, "error": "nom d'agent manquant"}));
            };
            let Some(prompt) = req_str(req, "prompt") else {
                return Some(json!({"ok": false, "error": "prompt manquant"}));
            };
            let mut args = Map::new();
            args.insert("name".into(), json!(name));
            args.insert("prompt".into(), json!(prompt));
            for k in ["runner", "model", "copyEnvFrom", "cloneFrom", "config"] {
                if let Some(v) = req_str(req, k) {
                    args.insert(k.into(), json!(v));
                }
            }
            if let Some(b) = req.get("resident").and_then(Value::as_bool) {
                args.insert("resident".into(), json!(b));
            }
            manage::add_log(
                state,
                "info",
                &format!("[swarm] create_agent '{name}' sur {inst}"),
            );
            match call_and_unwrap(state, inst, "create_agent", Value::Object(args), 20_000) {
                Ok((data, elapsed_ms)) => {
                    json!({"ok": true, "data": data, "elapsed_ms": elapsed_ms})
                }
                Err(e) => json!({"ok": false, "error": e}),
            }
        }

        "swarm_delete_agent" => {
            let Some(inst) = req_str(req, "instance_id") else {
                return Some(json!({"ok": false, "error": "instance_id manquant"}));
            };
            let Some(name) = req_str(req, "name") else {
                return Some(json!({"ok": false, "error": "nom d'agent manquant"}));
            };
            let mut args = Map::new();
            args.insert("name".into(), json!(name));
            for k in ["runner", "config"] {
                if let Some(v) = req_str(req, k) {
                    args.insert(k.into(), json!(v));
                }
            }
            manage::add_log(
                state,
                "info",
                &format!("[swarm] delete_agent '{name}' sur {inst}"),
            );
            match call_and_unwrap(state, inst, "delete_agent", Value::Object(args), 15_000) {
                Ok((data, elapsed_ms)) => {
                    json!({"ok": true, "data": data, "elapsed_ms": elapsed_ms})
                }
                Err(e) => json!({"ok": false, "error": e}),
            }
        }

        "swarm_get_configs" => {
            let Some(inst) = req_str(req, "instance_id") else {
                return Some(json!({"ok": false, "error": "instance_id manquant"}));
            };
            let Some(name) = req_str(req, "name") else {
                return Some(json!({"ok": false, "error": "nom d'agent manquant"}));
            };
            let mut args = Map::new();
            args.insert("name".into(), json!(name));
            for k in ["runner", "config"] {
                if let Some(v) = req_str(req, k) {
                    args.insert(k.into(), json!(v));
                }
            }
            match call_and_unwrap(
                state,
                inst,
                "get_agent_configs",
                Value::Object(args),
                10_000,
            ) {
                Ok((data, elapsed_ms)) => {
                    json!({"ok": true, "data": data, "elapsed_ms": elapsed_ms})
                }
                Err(e) => json!({"ok": false, "error": e}),
            }
        }

        "swarm_update_config" => {
            let Some(inst) = req_str(req, "instance_id") else {
                return Some(json!({"ok": false, "error": "instance_id manquant"}));
            };
            let Some(name) = req_str(req, "name") else {
                return Some(json!({"ok": false, "error": "nom d'agent manquant"}));
            };
            let mut args = Map::new();
            args.insert("name".into(), json!(name));
            for k in ["runner", "model", "prompt", "file", "content", "config"] {
                if let Some(v) = req.get(k).and_then(Value::as_str) {
                    args.insert(k.into(), json!(v));
                }
            }
            if let Some(arr) = req.get("mcpServers").and_then(Value::as_array) {
                args.insert("mcpServers".into(), Value::Array(arr.clone()));
            }
            if let Some(obj) = req.get("env").and_then(Value::as_object) {
                args.insert("env".into(), Value::Object(obj.clone()));
            }
            manage::add_log(
                state,
                "info",
                &format!("[swarm] update_agent_config '{name}' sur {inst}"),
            );
            match call_and_unwrap(
                state,
                inst,
                "update_agent_config",
                Value::Object(args),
                15_000,
            ) {
                Ok((data, elapsed_ms)) => {
                    json!({"ok": true, "data": data, "elapsed_ms": elapsed_ms})
                }
                Err(e) => json!({"ok": false, "error": e}),
            }
        }

        "swarm_agent_start" => {
            let Some(inst) = req_str(req, "instance_id") else {
                return Some(json!({"ok": false, "error": "instance_id manquant"}));
            };
            let Some(name) = req_str(req, "name") else {
                return Some(json!({"ok": false, "error": "nom d'agent manquant"}));
            };
            let mut args = Map::new();
            args.insert("name".into(), json!(name));
            for k in ["runner", "config"] {
                if let Some(v) = req_str(req, k) {
                    args.insert(k.into(), json!(v));
                }
            }
            manage::add_log(
                state,
                "info",
                &format!("[swarm] agent_start '{name}' sur {inst}"),
            );
            match call_and_unwrap(state, inst, "agent_start", Value::Object(args), 20_000) {
                Ok((data, elapsed_ms)) => {
                    let mut followed_id: Option<String> = None;
                    if req.get("follow").and_then(Value::as_bool).unwrap_or(false) {
                        if let Some(url) = data.get("url").and_then(Value::as_str) {
                            if let Some((host, port, path)) = parse_http_url_parts(url) {
                                let label = data
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .map(ToOwned::to_owned)
                                    .unwrap_or_else(|| format!("swarm-{name}"));
                                let already = {
                                    let servers = lockv(&state.servers);
                                    servers.iter().any(|s| {
                                        matches!(&s.transport, Transport::Http { port: p, .. } if *p == port)
                                    })
                                };
                                if !already {
                                    let mut add_req = json!({
                                        "action": "add",
                                        "label": label,
                                        "transport": "http",
                                        "port": port,
                                        "path": path,
                                    });
                                    if let Some(h) = host {
                                        add_req["host"] = json!(h);
                                    }
                                    let added = manage::handle_action(state, &add_req.to_string());
                                    if added.get("ok").and_then(Value::as_bool) == Some(true) {
                                        followed_id = added
                                            .get("id")
                                            .and_then(Value::as_str)
                                            .map(ToOwned::to_owned);
                                    }
                                }
                            }
                        }
                    }
                    json!({
                        "ok": true,
                        "data": data,
                        "elapsed_ms": elapsed_ms,
                        "followed_id": followed_id,
                    })
                }
                Err(e) => json!({"ok": false, "error": e}),
            }
        }

        "swarm_agent_stop" => {
            let Some(inst) = req_str(req, "instance_id") else {
                return Some(json!({"ok": false, "error": "instance_id manquant"}));
            };
            let Some(name) = req_str(req, "name") else {
                return Some(json!({"ok": false, "error": "nom d'agent manquant"}));
            };
            let mut args = Map::new();
            args.insert("name".into(), json!(name));
            for k in ["runner", "config"] {
                if let Some(v) = req_str(req, k) {
                    args.insert(k.into(), json!(v));
                }
            }
            manage::add_log(
                state,
                "info",
                &format!("[swarm] agent_stop '{name}' sur {inst}"),
            );
            match call_and_unwrap(state, inst, "agent_stop", Value::Object(args), 15_000) {
                Ok((data, elapsed_ms)) => {
                    json!({"ok": true, "data": data, "elapsed_ms": elapsed_ms})
                }
                Err(e) => json!({"ok": false, "error": e}),
            }
        }

        "swarm_resident_status" => {
            let Some(sid) = req_str(req, "server_id") else {
                return Some(json!({"ok": false, "error": "server_id manquant"}));
            };
            match call_and_unwrap(state, sid, "status", json!({}), 5_000) {
                Ok((data, elapsed_ms)) => {
                    json!({"ok": true, "data": data, "elapsed_ms": elapsed_ms})
                }
                Err(e) => json!({"ok": false, "error": e}),
            }
        }

        "swarm_resident_stop" => {
            let Some(sid) = req_str(req, "server_id") else {
                return Some(json!({"ok": false, "error": "server_id manquant"}));
            };
            let clear = req
                .get("clearQueue")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            manage::add_log(
                state,
                "info",
                &format!("[swarm] resident stop sur {sid} (clearQueue={clear})"),
            );
            match call_and_unwrap(state, sid, "stop", json!({"clearQueue": clear}), 10_000) {
                Ok((data, elapsed_ms)) => {
                    json!({"ok": true, "data": data, "elapsed_ms": elapsed_ms})
                }
                Err(e) => json!({"ok": false, "error": e}),
            }
        }

        "swarm_resident_ask" => {
            let Some(sid) = req_str(req, "server_id").map(ToOwned::to_owned) else {
                return Some(json!({"ok": false, "error": "server_id manquant"}));
            };
            let Some(prompt) = req_str(req, "prompt").map(ToOwned::to_owned) else {
                return Some(json!({"ok": false, "error": "prompt manquant"}));
            };
            let mut args = Map::new();
            args.insert("prompt".into(), json!(prompt));
            args.insert(
                "from".into(),
                json!(req_str(req, "from").unwrap_or("mcp-control")),
            );
            if let Some(m) = req_str(req, "model") {
                args.insert("model".into(), json!(m));
            }
            if let Some(via) = req.get("via").and_then(Value::as_array) {
                args.insert("via".into(), Value::Array(via.clone()));
            }
            let req_timeout = req
                .get("timeoutMs")
                .and_then(Value::as_u64)
                .unwrap_or(180_000)
                .clamp(5_000, 600_000);
            args.insert("timeoutMs".into(), json!(req_timeout));

            let agent_name = {
                let servers = lockv(&state.servers);
                servers
                    .iter()
                    .find(|s| s.id == sid)
                    .and_then(|s| detect_server(s).and_then(|m| m.agent))
            };
            let job_id = jobs::start_job(state, "ask", &sid, agent_name, &prompt);
            manage::add_log(
                state,
                "info",
                &format!("[swarm] ask lancé sur {sid} ({job_id})"),
            );
            let st = Arc::clone(state);
            let jid = job_id.clone();
            let http_timeout = req_timeout.saturating_add(15_000).clamp(30_000, 615_000);
            std::thread::spawn(move || {
                let t0 = Instant::now();
                let res = call_and_unwrap(&st, &sid, "ask", Value::Object(args), http_timeout)
                    .map(|(v, _)| v);
                jobs::finish_job(&st, &jid, t0.elapsed().as_millis() as u64, res);
            });
            json!({"ok": true, "job_id": job_id})
        }

        "swarm_run_agent" => {
            let Some(inst) = req_str(req, "instance_id").map(ToOwned::to_owned) else {
                return Some(json!({"ok": false, "error": "instance_id manquant"}));
            };
            let Some(runner) = req_str(req, "runner").map(ToOwned::to_owned) else {
                return Some(json!({"ok": false, "error": "runner manquant"}));
            };
            let Some(prompt) = req_str(req, "prompt").map(ToOwned::to_owned) else {
                return Some(json!({"ok": false, "error": "prompt manquant"}));
            };
            let background = req
                .get("background")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let mut args = Map::new();
            args.insert("runner".into(), json!(runner));
            args.insert("prompt".into(), json!(prompt));
            args.insert("background".into(), json!(background));
            for k in ["agentName", "model", "path", "sessionId", "config"] {
                if let Some(v) = req_str(req, k) {
                    args.insert(k.into(), json!(v));
                }
            }
            if let Some(ar) = req.get("autoResume").and_then(Value::as_bool) {
                args.insert("autoResume".into(), json!(ar));
            }
            let req_timeout = req
                .get("timeoutMs")
                .and_then(Value::as_u64)
                .unwrap_or(180_000)
                .clamp(5_000, 600_000);
            args.insert("timeoutMs".into(), json!(req_timeout));

            manage::add_log(
                state,
                "info",
                &format!("[swarm] run_agent ({runner}, bg={background}) sur {inst}"),
            );
            if background {
                match call_and_unwrap(state, &inst, "run_agent", Value::Object(args), 20_000) {
                    Ok((data, elapsed_ms)) => {
                        json!({"ok": true, "data": data, "elapsed_ms": elapsed_ms})
                    }
                    Err(e) => json!({"ok": false, "error": e}),
                }
            } else {
                let agent_name = req_str(req, "agentName").map(ToOwned::to_owned);
                let job_id = jobs::start_job(state, "run_agent", &inst, agent_name, &prompt);
                let st = Arc::clone(state);
                let jid = job_id.clone();
                let http_timeout = req_timeout.saturating_add(15_000).clamp(30_000, 615_000);
                std::thread::spawn(move || {
                    let t0 = Instant::now();
                    let res =
                        call_and_unwrap(&st, &inst, "run_agent", Value::Object(args), http_timeout)
                            .map(|(v, _)| v);
                    jobs::finish_job(&st, &jid, t0.elapsed().as_millis() as u64, res);
                });
                json!({"ok": true, "job_id": job_id})
            }
        }

        "swarm_run_parallel" => {
            let Some(inst) = req_str(req, "instance_id").map(ToOwned::to_owned) else {
                return Some(json!({"ok": false, "error": "instance_id manquant"}));
            };
            let Some(agents) = req.get("agents").and_then(Value::as_array).cloned() else {
                return Some(json!({"ok": false, "error": "agents (tableau) manquant"}));
            };
            if agents.is_empty() || agents.len() > 10 {
                return Some(
                    json!({"ok": false, "error": "agents doit contenir entre 1 et 10 tâches"}),
                );
            }
            let wait_all = req.get("waitAll").and_then(Value::as_bool).unwrap_or(true);
            let count = agents.len();
            let args = json!({
                "agents": agents,
                "waitAll": wait_all,
            });
            let job_id = jobs::start_job(
                state,
                "run_parallel",
                &inst,
                None,
                &format!("{count} agents en parallèle"),
            );
            manage::add_log(
                state,
                "info",
                &format!("[swarm] run_agents_parallel ({count}) sur {inst} ({job_id})"),
            );
            let st = Arc::clone(state);
            let jid = job_id.clone();
            std::thread::spawn(move || {
                let t0 = Instant::now();
                let res = call_and_unwrap(&st, &inst, "run_agents_parallel", args, 300_000)
                    .map(|(v, _)| v);
                jobs::finish_job(&st, &jid, t0.elapsed().as_millis() as u64, res);
            });
            json!({"ok": true, "job_id": job_id})
        }

        "swarm_agent_control" => {
            let Some(inst) = req_str(req, "instance_id") else {
                return Some(json!({"ok": false, "error": "instance_id manquant"}));
            };
            let Some(op) = req_str(req, "op") else {
                return Some(
                    json!({"ok": false, "error": "op manquant (status|stream|wait|kill|list)"}),
                );
            };
            let mut args = Map::new();
            args.insert("action".into(), json!(op));
            for k in ["agentName", "runId", "runner", "config"] {
                if let Some(v) = req_str(req, k) {
                    args.insert(k.into(), json!(v));
                }
            }
            if let Some(t) = req.get("timeoutMs").and_then(Value::as_u64) {
                args.insert("timeoutMs".into(), json!(t));
            }
            if let Some(ts) = req.get("sinceTimestamp").and_then(Value::as_u64) {
                args.insert("sinceTimestamp".into(), json!(ts));
            }
            match call_and_unwrap(state, inst, "agent_control", Value::Object(args), 15_000) {
                Ok((data, elapsed_ms)) => {
                    json!({"ok": true, "data": data, "elapsed_ms": elapsed_ms})
                }
                Err(e) => json!({"ok": false, "error": e}),
            }
        }

        "swarm_bridge_action" => {
            let Some(port) = req
                .get("port")
                .and_then(Value::as_u64)
                .filter(|p| (1..=65535).contains(p))
                .map(|p| p as u16)
            else {
                return Some(json!({"ok": false, "error": "port bridge invalide"}));
            };
            let op = req_str(req, "op").unwrap_or("status");
            match call_bridge_loopback(port, op, 5_000) {
                Ok(data) => json!({"ok": true, "data": data}),
                Err(e) => json!({"ok": false, "error": e}),
            }
        }

        "swarm_job_status" => {
            if let Some(jid) = req_str(req, "job_id") {
                match jobs::get_job(state, jid) {
                    Some(job) => json!({"ok": true, "job": job}),
                    None => json!({"ok": false, "error": "tâche Swarm inconnue"}),
                }
            } else {
                json!({"ok": true, "jobs": jobs::list_jobs(state)})
            }
        }

        _ => return None,
    };
    Some(out)
}
