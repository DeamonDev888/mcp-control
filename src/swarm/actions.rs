//! Actions locales `swarm_*` (réservées `ADMIN_ACTIONS`, jamais exposées au
//! gateway `/mcp` ni au LAN).

use super::detect::{detect_server, SwarmKind};
use super::jobs;
use super::rpc::{call_bridge_loopback, call_server_mcp, unwrap_tool_json};
use crate::manage;
use crate::types::{lockv, AppState, SState, Server, Transport};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

/// Cache mémoire `(server_id | "agent:<nom>") -> swarm_class` scopé par `AppState`
/// et persisté sous `<config_dir>/cache/swarm-classes.json`.
fn class_store() -> &'static Mutex<HashMap<usize, HashMap<String, String>>> {
    static STORE: OnceLock<Mutex<HashMap<usize, HashMap<String, String>>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn class_loaded_set() -> &'static Mutex<HashSet<usize>> {
    static LOADED: OnceLock<Mutex<HashSet<usize>>> = OnceLock::new();
    LOADED.get_or_init(|| Mutex::new(HashSet::new()))
}

fn classes_cache_path(config_path: &str) -> Option<PathBuf> {
    let trimmed = config_path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let base = Path::new(trimmed)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())?;
    Some(base.join("cache").join("swarm-classes.json"))
}

fn ensure_classes_loaded(state: &Arc<AppState>) {
    let key = Arc::as_ptr(state) as usize;
    {
        let mut loaded = lockv(class_loaded_set());
        if !loaded.insert(key) {
            return;
        }
    }
    let Some(path) = classes_cache_path(&state.config_path) else {
        return;
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return;
    };
    let Ok(Value::Object(obj)) = serde_json::from_slice::<Value>(&bytes) else {
        return;
    };
    let mut store = lockv(class_store());
    let map = store.entry(key).or_default();
    for (k, v) in obj {
        if let Some(cls) = normalize_swarm_class(v.as_str()) {
            map.entry(k).or_insert(cls);
        }
    }
}

fn persist_classes(state: &Arc<AppState>) {
    let Some(path) = classes_cache_path(&state.config_path) else {
        return;
    };
    let key = Arc::as_ptr(state) as usize;
    let snap: Map<String, Value> = {
        let store = lockv(class_store());
        let Some(map) = store.get(&key) else {
            return;
        };
        map.iter()
            .filter(|(k, _)| k.starts_with("agent:"))
            .map(|(k, v)| (k.clone(), json!(v)))
            .collect()
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("json.tmp");
    if let Ok(bytes) = serde_json::to_vec_pretty(&Value::Object(snap)) {
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// Valide et normalise une classe d'agent (`status.class` ou `agent.class`)
/// contre la liste blanche des slugs servis par `ui::swarm_logo_svg` (hors
/// `logo`, réservé à la ruche). Toute classe inconnue retourne `None` afin que
/// l'UI retombe sur la devinette par le nom puis `custom`.
pub(crate) fn normalize_swarm_class(raw: Option<&str>) -> Option<String> {
    let slug = raw?.trim().to_ascii_lowercase();
    if slug.is_empty() || slug == "logo" {
        return None;
    }
    if crate::ui::swarm_logo_svg(&slug).is_some() {
        Some(slug)
    } else {
        None
    }
}

fn last_agents_cache() -> &'static std::sync::Mutex<HashMap<String, Vec<Value>>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Vec<Value>>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn extract_swarm_class(v: &Value) -> Option<String> {
    normalize_swarm_class(v.get("class").and_then(Value::as_str))
}

fn agent_class_key(agent_name: &str) -> String {
    format!("agent:{}", agent_name.trim().to_ascii_lowercase())
}

fn set_swarm_class(state: &Arc<AppState>, server_id: &str, cls: Option<&str>) {
    ensure_classes_loaded(state);
    let key = Arc::as_ptr(state) as usize;
    let mut store = lockv(class_store());
    let entry = store.entry(key).or_default();
    match normalize_swarm_class(cls) {
        Some(valid) => {
            entry.insert(server_id.to_string(), valid);
        }
        None => {
            entry.remove(server_id);
        }
    }
}

fn set_agent_class_by_name(state: &Arc<AppState>, agent_name: &str, cls: Option<&str>) {
    let trimmed = agent_name.trim();
    if trimmed.is_empty() {
        return;
    }
    ensure_classes_loaded(state);
    let akey = agent_class_key(trimmed);
    let matched_ids: Vec<String> = {
        let servers = lockv(&state.servers);
        servers
            .iter()
            .filter_map(|s| {
                let m = detect_server(s)?;
                if m.kind == SwarmKind::Resident
                    && m.agent
                        .as_deref()
                        .is_some_and(|a| a.eq_ignore_ascii_case(trimmed))
                {
                    Some(s.id.clone())
                } else {
                    None
                }
            })
            .collect()
    };
    {
        let key = Arc::as_ptr(state) as usize;
        let mut store = lockv(class_store());
        let entry = store.entry(key).or_default();
        match normalize_swarm_class(cls) {
            Some(valid) => {
                entry.insert(akey, valid.clone());
                for sid in matched_ids {
                    entry.insert(sid, valid.clone());
                }
            }
            None => {
                entry.remove(&akey);
                for sid in matched_ids {
                    entry.remove(&sid);
                }
            }
        }
    }
    persist_classes(state);
}

pub fn get_swarm_class(state: &Arc<AppState>, server_id: &str) -> Option<String> {
    ensure_classes_loaded(state);
    let key = Arc::as_ptr(state) as usize;
    let store = lockv(class_store());
    store.get(&key).and_then(|m| m.get(server_id)).cloned()
}

pub fn snapshot_swarm_classes(state: &Arc<AppState>) -> HashMap<String, String> {
    ensure_classes_loaded(state);
    let key = Arc::as_ptr(state) as usize;
    let store = lockv(class_store());
    store.get(&key).cloned().unwrap_or_default()
}

pub fn resolve_server_swarm_class<'a>(
    s: &Server,
    classes: &'a HashMap<String, String>,
) -> Option<&'a str> {
    if let Some(c) = classes.get(&s.id) {
        return Some(c.as_str());
    }
    let meta = detect_server(s)?;
    if meta.kind == SwarmKind::Resident {
        if let Some(ag) = meta.agent {
            return classes.get(&agent_class_key(&ag)).map(String::as_str);
        }
    }
    None
}

/// Enregistre automatiquement dans MCP Control l'URL HTTP d'un agent résident
/// nouvellement démarré (s'il n'est pas déjà suivi sur ce port).
fn auto_follow_resident_url(
    state: &Arc<AppState>,
    url: &str,
    fallback_agent: &str,
    explicit_label: Option<&str>,
    cls: Option<&str>,
) -> Option<String> {
    let (host, port, path) = parse_http_url_parts(url)?;
    let existing_id = {
        let servers = lockv(&state.servers);
        servers.iter().find_map(|s| match &s.transport {
            Transport::Http { port: p, .. } if *p == port => Some(s.id.clone()),
            _ => None,
        })
    };
    if let Some(id) = existing_id {
        if cls.is_some() {
            set_swarm_class(state, &id, cls);
        }
        return Some(id);
    }
    let label = explicit_label
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("swarm-{fallback_agent}"));
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
        let fid = added
            .get("id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        if let Some(ref id) = fid {
            if cls.is_some() {
                set_swarm_class(state, id, cls);
            }
        }
        fid
    } else {
        None
    }
}

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

    // Interrogation parallèle bornée (12 s max) des instances UP et résidents UP.
    let (instances_json, residents_json) = std::thread::scope(|scope| {
        let inst_handles: Vec<_> = instances_snap
            .iter()
            .map(|inst| {
                let st = Arc::clone(state);
                scope.spawn(move || {
                    let mut agents = Value::Array(vec![]);
                    let mut runs = Value::Array(vec![]);
                    let mut error: Option<String> = None;
                    let mut stale_agents = false;
                    if inst.state == SState::Up {
                        let (ag_res, runs_res) = std::thread::scope(|inner| {
                            let st1 = Arc::clone(&st);
                            let id1 = inst.id.clone();
                            let h_ag = inner.spawn(move || {
                                call_and_unwrap(
                                    &st1,
                                    &id1,
                                    "list_agents",
                                    json!({"details": true}),
                                    12_000,
                                )
                            });
                            let st2 = Arc::clone(&st);
                            let id2 = inst.id.clone();
                            let h_runs = inner.spawn(move || {
                                call_and_unwrap(
                                    &st2,
                                    &id2,
                                    "agent_control",
                                    json!({"action": "list"}),
                                    8_000,
                                )
                            });
                            (h_ag.join().ok(), h_runs.join().ok())
                        });
                        match ag_res {
                            Some(Ok((v, _))) => {
                                if let Some(arr) = v.get("agents").and_then(Value::as_array) {
                                    if let Ok(mut cache) = last_agents_cache().lock() {
                                        cache.insert(inst.id.clone(), arr.clone());
                                    }
                                    agents = Value::Array(arr.clone());
                                }
                            }
                            Some(Err(e)) => {
                                error = Some(e);
                                if let Ok(cache) = last_agents_cache().lock() {
                                    if let Some(prev) = cache.get(&inst.id) {
                                        agents = Value::Array(prev.clone());
                                        stale_agents = true;
                                    }
                                }
                            }
                            None => error = Some("échec interne list_agents".to_string()),
                        }
                        if let Some(Ok((v, _))) = runs_res {
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
                        "stale_agents": stale_agents,
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
                    let swarm_class = if r.state == SState::Up {
                        match call_and_unwrap(&st, &r.id, "status", json!({}), 8_000) {
                            Ok((v, _)) => {
                                let cls = extract_swarm_class(&v);
                                set_swarm_class(&st, &r.id, cls.as_deref());
                                status_val = v;
                                cls
                            }
                            Err(e) => {
                                error = Some(e);
                                get_swarm_class(&st, &r.id)
                            }
                        }
                    } else {
                        get_swarm_class(&st, &r.id)
                    };
                    json!({
                        "id": r.id,
                        "label": r.label,
                        "state": r.state.as_str(),
                        "url": r.url,
                        "version": r.version,
                        "agent": r.agent,
                        "swarm_class": swarm_class,
                        "bridge_port": r.bridge_port,
                        "locked": r.locked,
                        "paired": r.paired,
                        "status": status_val,
                        "error": error,
                    })
                })
            })
            .collect();

        let mut insts: Vec<Value> = inst_handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .collect();
        let mut ress: Vec<Value> = res_handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .collect();
        let saved_classes = snapshot_swarm_classes(state);
        // Enrichit `list_agents` avec les classes persistées localement et collecte
        // toutes les classes connues pour propager vers les résidents.
        let mut classes_by_agent: HashMap<String, String> = HashMap::new();
        for (k, v) in &saved_classes {
            if let Some(ag) = k.strip_prefix("agent:") {
                classes_by_agent.insert(ag.to_string(), v.clone());
            }
        }
        for inst in &mut insts {
            if let Some(arr) = inst.get_mut("agents").and_then(Value::as_array_mut) {
                for a in arr {
                    let ag_name = a
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_ascii_lowercase);
                    if let Some(cls) = extract_swarm_class(a) {
                        if let Some(ref low) = ag_name {
                            classes_by_agent.insert(low.clone(), cls);
                        }
                    } else if let Some(ref low) = ag_name {
                        if let Some(saved) = classes_by_agent.get(low) {
                            if let Some(obj) = a.as_object_mut() {
                                obj.insert("class".into(), json!(saved));
                            }
                        }
                    }
                }
            }
        }
        if !classes_by_agent.is_empty() {
            for r in &mut ress {
                let has_cls = r.get("swarm_class").and_then(Value::as_str).is_some();
                if !has_cls {
                    if let Some(ag) = r.get("agent").and_then(Value::as_str) {
                        if let Some(cls) = classes_by_agent.get(&ag.to_ascii_lowercase()) {
                            if let Some(id) = r.get("id").and_then(Value::as_str) {
                                set_swarm_class(state, id, Some(cls));
                            }
                            if let Some(obj) = r.as_object_mut() {
                                obj.insert("swarm_class".into(), json!(cls));
                            }
                        }
                    }
                }
            }
        }
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
            // NB : `create_agent` dans Swarm-control valide son entrée en `.strict()`
            // et n'accepte pas de champ `class` — on mémorise `class` côté MCP Control.
            let wanted_class = normalize_swarm_class(req_str(req, "class"));
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
            if let Some(b) = req.get("executable").and_then(Value::as_bool) {
                args.insert("executable".into(), json!(b));
            }
            manage::add_log(
                state,
                "info",
                &format!("[swarm] create_agent '{name}' sur {inst}"),
            );
            match call_and_unwrap(state, inst, "create_agent", Value::Object(args), 25_000) {
                Ok((data, elapsed_ms)) => {
                    if let Some(ref cls) = wanted_class {
                        set_agent_class_by_name(state, name, Some(cls));
                    }
                    let followed_id = data.get("resident").and_then(|r| {
                        let url = r.get("url")?.as_str()?;
                        let lbl = r.get("name").and_then(Value::as_str);
                        auto_follow_resident_url(state, url, name, lbl, wanted_class.as_deref())
                    });
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
                    set_agent_class_by_name(state, name, None);
                    // Si un serveur résident suivi correspondait à cet agent supprimé, le retirer proprement.
                    let stale_ids: Vec<String> = {
                        let servers = lockv(&state.servers);
                        servers
                            .iter()
                            .filter_map(|s| {
                                let m = detect_server(s)?;
                                if m.kind == SwarmKind::Resident
                                    && m.agent
                                        .as_deref()
                                        .is_some_and(|a| a.eq_ignore_ascii_case(name))
                                {
                                    Some(s.id.clone())
                                } else {
                                    None
                                }
                            })
                            .collect()
                    };
                    for sid in &stale_ids {
                        let _ = manage::handle_action(
                            state,
                            &json!({"action": "remove", "id": sid}).to_string(),
                        );
                    }
                    json!({
                        "ok": true,
                        "data": data,
                        "elapsed_ms": elapsed_ms,
                        "removed_residents": stale_ids,
                    })
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
                    let saved_cls = snapshot_swarm_classes(state)
                        .get(&agent_class_key(name))
                        .cloned();
                    let followed_id = if req.get("follow").and_then(Value::as_bool).unwrap_or(false)
                    {
                        data.get("url").and_then(Value::as_str).and_then(|url| {
                            auto_follow_resident_url(
                                state,
                                url,
                                name,
                                data.get("name").and_then(Value::as_str),
                                saved_cls.as_deref(),
                            )
                        })
                    } else {
                        None
                    };
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
                    let matching_ids: Vec<String> = {
                        let servers = lockv(&state.servers);
                        servers
                            .iter()
                            .filter_map(|s| {
                                let m = detect_server(s)?;
                                if m.kind == SwarmKind::Resident
                                    && m.agent
                                        .as_deref()
                                        .is_some_and(|a| a.eq_ignore_ascii_case(name))
                                {
                                    Some(s.id.clone())
                                } else {
                                    None
                                }
                            })
                            .collect()
                    };
                    for sid in &matching_ids {
                        let _ = manage::handle_action(
                            state,
                            &json!({"action": "refresh", "id": sid}).to_string(),
                        );
                    }
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
                    let swarm_class = extract_swarm_class(&data);
                    set_swarm_class(state, sid, swarm_class.as_deref());
                    json!({
                        "ok": true,
                        "data": data,
                        "swarm_class": swarm_class,
                        "elapsed_ms": elapsed_ms,
                    })
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

        "swarm_sync_logos" => {
            let detected_ver = {
                let servers = lockv(&state.servers);
                servers.iter().find_map(|s| {
                    if s.state == SState::Up && detect_server(s).is_some() {
                        s.version.clone().filter(|v| !v.trim().is_empty())
                    } else {
                        None
                    }
                })
            };
            let rep = super::logos::sync_now(state, detected_ver.as_deref());
            if rep.busy {
                json!({"ok": false, "error": "synchronisation des logos déjà en cours"})
            } else {
                manage::add_log(
                    state,
                    "info",
                    &format!(
                        "[swarm] sync logos manuelle : {} mis à jour, {} inchangés, {} erreur(s)",
                        rep.updated, rep.unchanged, rep.errors
                    ),
                );
                json!({
                    "ok": true,
                    "updated": rep.updated,
                    "unchanged": rep.unchanged,
                    "errors": rep.errors,
                    "rev": rep.rev,
                })
            }
        }

        "swarm_resident_history" => {
            let Some(sid) = req_str(req, "server_id") else {
                return Some(json!({"ok": false, "error": "server_id manquant"}));
            };
            let mut args = Map::new();
            if let Some(lim) = req.get("limit").and_then(Value::as_u64) {
                args.insert("limit".into(), json!(lim.clamp(1, 500)));
            }
            if let Some(before) = req.get("before").and_then(Value::as_u64) {
                args.insert("before".into(), json!(before));
            }
            match call_and_unwrap(state, sid, "history", Value::Object(args), 8_000) {
                Ok((data, elapsed_ms)) => {
                    json!({"ok": true, "data": data, "elapsed_ms": elapsed_ms})
                }
                Err(e) => json!({"ok": false, "error": e}),
            }
        }

        "swarm_resident_logs" => {
            let Some(sid) = req_str(req, "server_id") else {
                return Some(json!({"ok": false, "error": "server_id manquant"}));
            };
            let mut args = Map::new();
            if let Some(tail) = req.get("tail").and_then(Value::as_u64) {
                args.insert("tail".into(), json!(tail.clamp(1, 65_536)));
            }
            match call_and_unwrap(state, sid, "logs", Value::Object(args), 8_000) {
                Ok((data, elapsed_ms)) => {
                    json!({"ok": true, "data": data, "elapsed_ms": elapsed_ms})
                }
                Err(e) => json!({"ok": false, "error": e}),
            }
        }

        "swarm_resident_session" => {
            let Some(sid) = req_str(req, "server_id") else {
                return Some(json!({"ok": false, "error": "server_id manquant"}));
            };
            let op = req_str(req, "op").unwrap_or("show");
            if !matches!(op, "show" | "reset" | "set") {
                return Some(
                    json!({"ok": false, "error": "op invalide (show | reset | set attendu)"}),
                );
            }
            let mut args = Map::new();
            args.insert("action".into(), json!(op));
            if op == "set" {
                let Some(sess_id) = req_str(req, "session_id") else {
                    return Some(json!({"ok": false, "error": "session_id requis pour op=set"}));
                };
                args.insert("id".into(), json!(sess_id));
            }
            manage::add_log(
                state,
                "info",
                &format!("[swarm] resident session '{op}' sur {sid}"),
            );
            match call_and_unwrap(state, sid, "session", Value::Object(args), 8_000) {
                Ok((data, elapsed_ms)) => {
                    json!({"ok": true, "data": data, "elapsed_ms": elapsed_ms})
                }
                Err(e) => json!({"ok": false, "error": e}),
            }
        }

        "swarm_resident_config" => {
            let Some(sid) = req_str(req, "server_id") else {
                return Some(json!({"ok": false, "error": "server_id manquant"}));
            };
            match call_and_unwrap(state, sid, "config", json!({}), 8_000) {
                Ok((data, elapsed_ms)) => {
                    // Normalise `files: [{name, content}]` (outil résident `config`)
                    // au même format `{ name, runner, files: { ... } }` que `get_agent_configs`.
                    let mut files_map = Map::new();
                    if let Some(arr) = data.get("files").and_then(Value::as_array) {
                        for item in arr {
                            if let (Some(n), Some(c)) = (
                                item.get("name").and_then(Value::as_str),
                                item.get("content").and_then(Value::as_str),
                            ) {
                                let norm_name = if n == "prompt" { "prompt.md" } else { n };
                                files_map.insert(norm_name.to_string(), json!(c));
                            }
                        }
                    } else if let Some(obj) = data.get("files").and_then(Value::as_object) {
                        files_map = obj.clone();
                    }
                    let agent = data
                        .get("agent")
                        .or_else(|| data.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let runner = data.get("runner").and_then(Value::as_str);
                    json!({
                        "ok": true,
                        "data": {
                            "name": agent,
                            "runner": runner,
                            "files": files_map,
                            "read_only": true,
                        },
                        "elapsed_ms": elapsed_ms,
                    })
                }
                Err(e) => json!({"ok": false, "error": e}),
            }
        }

        "swarm_set_agent_class" => {
            let cls_raw = req.get("class").and_then(Value::as_str).map(str::trim);
            let clear = cls_raw.is_none_or(|s| s.is_empty() || s == "auto");
            let valid = if clear {
                None
            } else {
                let Some(c) = normalize_swarm_class(cls_raw) else {
                    return Some(json!({"ok": false, "error": "classe d'agent invalide"}));
                };
                Some(c)
            };
            let agent = req_str(req, "agent").map(ToOwned::to_owned);
            let sid = req_str(req, "server_id").map(ToOwned::to_owned);
            if agent.is_none() && sid.is_none() {
                return Some(json!({"ok": false, "error": "agent ou server_id requis"}));
            }
            if let Some(ref ag) = agent {
                set_agent_class_by_name(state, ag, valid.as_deref());
            }
            if let Some(ref id) = sid {
                set_swarm_class(state, id, valid.as_deref());
                if agent.is_none() {
                    let inferred = {
                        let servers = lockv(&state.servers);
                        servers
                            .iter()
                            .find(|s| &s.id == id)
                            .and_then(|s| detect_server(s).and_then(|m| m.agent))
                    };
                    if let Some(ag) = inferred {
                        set_agent_class_by_name(state, &ag, valid.as_deref());
                    }
                }
            }
            json!({"ok": true, "class": valid})
        }

        _ => return None,
    };
    Some(out)
}
