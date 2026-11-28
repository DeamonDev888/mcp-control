//! Scénarios e2e — support natif Swarm-control (détection instance admin +
//! agent résident, actions `swarm_*`, ressource gateway `mcp-control://swarm`,
//! assets `/swarm.js` & `/swarm.css`).

use crate::harness::{serial_guard, spawn_loopback, wait_until};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Serveur LiteMCP factice (instance admin `swarm-control-1` ou résident
/// `swarm-alice`) répondant à `GET /health`, `initialize`, `tools/list` et
/// `tools/call`.
struct MockSwarmServer {
    port: u16,
    stop: Arc<AtomicBool>,
}

impl Drop for MockSwarmServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(("127.0.0.1", self.port));
    }
}

fn spawn_mock_swarm(
    server_name: &'static str,
    tools: Value,
    responder: fn(&str, &Value) -> Value,
) -> MockSwarmServer {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind mock swarm");
    let port = listener.local_addr().unwrap().port();
    let _ = listener.set_nonblocking(true);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_bg = Arc::clone(&stop);

    thread::spawn(move || {
        while !stop_bg.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let tools = tools.clone();
                    thread::spawn(move || {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        let mut raw = Vec::new();
                        let mut tmp = [0u8; 4096];
                        let mut header_end: Option<usize> = None;
                        let mut content_len: usize = 0;
                        loop {
                            let n = stream.read(&mut tmp).unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            raw.extend_from_slice(&tmp[..n]);
                            if header_end.is_none() {
                                if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                                    header_end = Some(pos + 4);
                                    let h = String::from_utf8_lossy(&raw[..pos]);
                                    for line in h.lines() {
                                        if let Some(v) = line
                                            .to_ascii_lowercase()
                                            .strip_prefix("content-length:")
                                        {
                                            content_len = v.trim().parse().unwrap_or(0);
                                        }
                                    }
                                }
                            }
                            if let Some(he) = header_end {
                                if raw.len() >= he + content_len {
                                    break;
                                }
                            }
                        }
                        if raw.is_empty() {
                            return;
                        }
                        let req = String::from_utf8_lossy(&raw);
                        let first = req.lines().next().unwrap_or("");
                        if first.starts_with("GET /health") {
                            let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
                            let _ = stream.write_all(resp.as_bytes());
                            return;
                        }
                        let body = header_end
                            .map(|he| String::from_utf8_lossy(&raw[he..]).to_string())
                            .unwrap_or_default();
                        let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                        let id = v.get("id").cloned().unwrap_or(json!(1));
                        let method = v.get("method").and_then(Value::as_str).unwrap_or("");
                        let reply = match method {
                            "initialize" => json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "protocolVersion": "2026-07-28",
                                    "capabilities": {"tools": {}},
                                    "serverInfo": {"name": server_name, "version": "0.1.2"}
                                }
                            }),
                            "notifications/initialized" => {
                                let resp = "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                                let _ = stream.write_all(resp.as_bytes());
                                return;
                            }
                            "tools/list" => json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {"tools": tools}
                            }),
                            "tools/call" => {
                                let tname = v["params"]["name"].as_str().unwrap_or("");
                                let targs = &v["params"]["arguments"];
                                let payload = responder(tname, targs);
                                json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": {
                                        "content": [{"type": "text", "text": payload.to_string()}]
                                    }
                                })
                            }
                            _ => json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "error": {"code": -32601, "message": "method not found"}
                            }),
                        };
                        let bytes = reply.to_string();
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            bytes.len(),
                            bytes
                        );
                        let _ = stream.write_all(resp.as_bytes());
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    });

    MockSwarmServer { port, stop }
}

fn admin_responder(tool: &str, args: &Value) -> Value {
    match tool {
        "list_agents" => json!({
            "count": 1,
            "agents": [{
                "name": "alice",
                "runner": "claude",
                "hasSession": true,
                "running": false,
                "resident": {"instance": "swarm-control-1", "url": "http://127.0.0.1:23081/mcp"}
            }]
        }),
        "agent_control" => json!({"runs": []}),
        "create_agent" => {
            assert!(
                args.get("class").is_none(),
                "create_agent (.strict()) ne doit pas recevoir de champ 'class'"
            );
            json!({
                "created": args["name"],
                "runner": args["runner"],
                "executable": args.get("executable").and_then(Value::as_bool).unwrap_or(false)
            })
        }
        "agent_stop" => json!({"stopped": args["name"]}),
        _ => json!({"ok": true}),
    }
}

fn resident_responder(tool: &str, args: &Value) -> Value {
    match tool {
        "status" => json!({
            "agent": "alice",
            "runner": "claude",
            "name": "swarm-alice",
            "state": "idle",
            "queueLength": 0,
            "turnsDone": 3
        }),
        "ask" => json!({
            "agent": "alice",
            "reply": format!("echo: {}", args["prompt"].as_str().unwrap_or("")),
            "sessionId": "sess-42",
            "queuedMs": 0,
            "durationMs": 12,
            "via": ["mcp-control"]
        }),
        "stop" => json!({"agent": "alice", "stopped": true, "cleared": 0}),
        _ => json!({"ok": true}),
    }
}

#[test]
fn swarm_e2e_detection_overview_ask_et_assets() {
    let _garde = serial_guard();
    let mock_admin = spawn_mock_swarm(
        "swarm-control-1",
        json!([
            {"name": "list_agents", "description": "list"},
            {"name": "create_agent", "description": "create"},
            {"name": "agent_start", "description": "start"},
            {"name": "agent_stop", "description": "stop"},
            {"name": "agent_control", "description": "ctrl"}
        ]),
        admin_responder,
    );
    let mock_res = spawn_mock_swarm(
        "swarm-alice",
        json!([
            {"name": "ask", "description": "ask"},
            {"name": "status", "description": "status"},
            {"name": "stop", "description": "stop"}
        ]),
        resident_responder,
    );

    let eng = spawn_loopback();

    // 1. Assets dédiés servis avec 200
    assert_eq!(eng.get("/swarm.js").status, 200);
    assert_eq!(eng.get("/swarm.css").status, 200);
    assert_eq!(eng.get("/swarm-logo/logo.svg").status, 200);
    assert_eq!(eng.get("/swarm-logo/coordinator.svg").status, 200);
    assert_eq!(eng.get("/swarm-logo/coder.svg").status, 200);

    // 2. Ajout des deux serveurs Swarm
    let add_admin = eng.action(&json!({
        "action": "add",
        "label": "Swarm Admin",
        "transport": "http",
        "port": mock_admin.port,
        "path": "/mcp"
    }));
    assert_eq!(add_admin["ok"], true);
    let admin_id = add_admin["id"].as_str().unwrap().to_string();

    let add_res = eng.action(&json!({
        "action": "add",
        "label": "Alice",
        "transport": "http",
        "port": mock_res.port,
        "path": "/mcp"
    }));
    assert_eq!(add_res["ok"], true);
    let res_id = add_res["id"].as_str().unwrap().to_string();

    // 3. Attendre que les deux serveurs soient sondés UP et détectés Swarm
    eng.action(&json!({"action": "refresh", "id": admin_id}));
    eng.action(&json!({"action": "refresh", "id": res_id}));
    let ok_up = wait_until(Duration::from_secs(10), || {
        let st = eng.get("/api/state").json();
        Some(
            st["swarm_stats"]["instances"] == 1
                && st["swarm_stats"]["residents"] == 1
                && st["swarm_stats"]["up"] == 2,
        )
    });
    assert!(ok_up, "les deux serveurs Swarm doivent être UP et détectés");

    let st = eng.get("/api/state").json();
    assert_eq!(st["swarm_stats"]["instances"], 1);
    assert_eq!(st["swarm_stats"]["residents"], 1);
    let servers = st["servers"].as_array().unwrap();
    let s_admin = servers.iter().find(|s| s["id"] == admin_id).unwrap();
    let s_res = servers.iter().find(|s| s["id"] == res_id).unwrap();
    assert_eq!(s_admin["swarm_kind"], "instance");
    assert_eq!(s_admin["framework"], "litemcp");
    assert_eq!(s_res["swarm_kind"], "resident");
    assert_eq!(s_res["swarm_agent"], "alice");

    // 4. swarm_overview agrège instances + agents + statut résident
    let ov = eng.action(&json!({"action": "swarm_overview"}));
    assert_eq!(ov["ok"], true);
    assert_eq!(ov["instances"].as_array().unwrap().len(), 1);
    assert_eq!(ov["instances"][0]["agents"][0]["name"], "alice");
    assert_eq!(ov["residents"][0]["status"]["turnsDone"], 3);

    // 5. swarm_resident_ask asynchrone + suivi via swarm_job_status
    let ask = eng.action(&json!({
        "action": "swarm_resident_ask",
        "server_id": res_id,
        "prompt": "bonjour swarm"
    }));
    assert_eq!(ask["ok"], true);
    let jid = ask["job_id"].as_str().unwrap().to_string();
    let mut final_job = Value::Null;
    for _ in 0..40 {
        let poll = eng.action(&json!({"action": "swarm_job_status", "job_id": jid}));
        if poll["job"]["status"] == "done" {
            final_job = poll["job"].clone();
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(final_job["status"], "done");
    assert_eq!(final_job["result"]["reply"], "echo: bonjour swarm");
    assert_eq!(final_job["result"]["sessionId"], "sess-42");

    // 6. swarm_create_agent & swarm_resident_stop
    let cr = eng.action(&json!({
        "action": "swarm_create_agent",
        "instance_id": admin_id,
        "name": "bob",
        "runner": "hermes",
        "prompt": "Tu es Bob.",
        "class": "explorer",
        "executable": true
    }));
    assert_eq!(cr["ok"], true);
    assert_eq!(cr["data"]["created"], "bob");
    assert_eq!(cr["data"]["executable"], true);

    let stop = eng.action(&json!({
        "action": "swarm_resident_stop",
        "server_id": res_id,
        "clearQueue": true
    }));
    assert_eq!(stop["ok"], true);
    assert_eq!(stop["data"]["stopped"], true);
}

fn resident_seven_tools() -> Value {
    json!([
        {"name": "ask", "description": "ask"},
        {"name": "status", "description": "status"},
        {"name": "stop", "description": "stop"},
        {"name": "history", "description": "history"},
        {"name": "logs", "description": "logs"},
        {"name": "config", "description": "config"},
        {"name": "session", "description": "session"}
    ])
}

fn sniperbot_discord_responder(tool: &str, args: &Value) -> Value {
    match tool {
        "status" => json!({
            "agent": "sniperbot_analyst",
            "class": "discord",
            "runner": "claude",
            "state": "idle",
            "queueLength": 0,
            "turnsDone": 1
        }),
        "history" => json!({
            "agent": "sniperbot_analyst",
            "total": 1,
            "path": "bridge/history/claude-sniperbot_analyst.jsonl",
            "entries": [{
                "ts": 1700000000000u64,
                "prompt": "ping",
                "reply": "pong",
                "sessionId": "sess-99",
                "durationMs": 42,
                "queuedMs": 0,
                "runner": "claude",
                "agent": "sniperbot_analyst"
            }]
        }),
        "logs" => json!({
            "output": "derniere ligne stdout\n",
            "bytes": 22,
            "live": false
        }),
        "session" => match args.get("action").and_then(Value::as_str).unwrap_or("") {
            "show" => json!({"action": "show", "session": "sess-99"}),
            "reset" => json!({"action": "reset", "hadSession": true, "session": null}),
            "set" => json!({"action": "set", "session": args["id"]}),
            _ => json!({"error": "bad action"}),
        },
        "config" => json!({
            "agent": "sniperbot_analyst",
            "runner": "claude",
            "files": [
                {"name": "prompt", "content": "Tu es Sniperbot."},
                {"name": "settings.json", "content": "{}"}
            ]
        }),
        _ => json!({"ok": true}),
    }
}

fn resident_no_class_responder(tool: &str, _args: &Value) -> Value {
    match tool {
        "status" => json!({
            "agent": "coder_helper",
            "runner": "claude",
            "state": "idle",
            "queueLength": 0,
            "turnsDone": 0
        }),
        _ => json!({"ok": true}),
    }
}

fn resident_unknown_class_responder(tool: &str, _args: &Value) -> Value {
    match tool {
        "status" => json!({
            "agent": "rogue_agent",
            "class": "../../etc/passwd",
            "runner": "claude",
            "state": "idle",
            "queueLength": 0,
            "turnsDone": 0
        }),
        _ => json!({"ok": true}),
    }
}

#[test]
fn swarm_e2e_classe_agent_logo_et_detection_sept_outils() {
    let _garde = serial_guard();
    // 1. Résident nommé `sniperbot_analyst` (sans préfixe `swarm-` : détecté par
    //    sa signature de 7 outils) dont le `status` porte `"class": "discord"`.
    let mock_discord = spawn_mock_swarm(
        "sniperbot_analyst",
        resident_seven_tools(),
        sniperbot_discord_responder,
    );
    // 2. Résident sans champ `class` dans son `status` : garde la devinette par nom.
    let mock_noclass = spawn_mock_swarm(
        "swarm-coder_helper",
        resident_seven_tools(),
        resident_no_class_responder,
    );
    // 3. Résident avec classe inconnue / traversal : ignorée (jamais propagée).
    let mock_unknown = spawn_mock_swarm(
        "swarm-rogue_agent",
        resident_seven_tools(),
        resident_unknown_class_responder,
    );

    let eng = spawn_loopback();

    // Assets logo : slugs connus servis en 200, classe inconnue / traversal -> 404.
    assert_eq!(eng.get("/swarm-logo/discord.svg").status, 200);
    assert_eq!(eng.get("/swarm-logo/custom.svg").status, 200);
    assert_eq!(eng.get("/swarm-logo/inconnue.svg").status, 404);
    assert_eq!(eng.get("/swarm-logo/..%2Fetc%2Fpasswd.svg").status, 404);

    let id_discord = eng.action(&json!({
        "action": "add",
        "label": "Sniperbot Analyst",
        "transport": "http",
        "port": mock_discord.port,
        "path": "/mcp"
    }))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let id_noclass = eng.action(&json!({
        "action": "add",
        "label": "Coder Helper",
        "transport": "http",
        "port": mock_noclass.port,
        "path": "/mcp"
    }))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let id_unknown = eng.action(&json!({
        "action": "add",
        "label": "Rogue Agent",
        "transport": "http",
        "port": mock_unknown.port,
        "path": "/mcp"
    }))["id"]
        .as_str()
        .unwrap()
        .to_string();

    for id in [&id_discord, &id_noclass, &id_unknown] {
        eng.action(&json!({"action": "refresh", "id": id}));
    }

    let ok_up = wait_until(Duration::from_secs(10), || {
        let st = eng.get("/api/state").json();
        Some(st["swarm_stats"]["residents"] == 3 && st["swarm_stats"]["up"] == 3)
    });
    assert!(
        ok_up,
        "les 3 résidents (dont celui détecté par 7 outils) doivent être UP et détectés"
    );

    // `swarm_overview` interroge `status` et propage `swarm_class`.
    let ov = eng.action(&json!({"action": "swarm_overview"}));
    assert_eq!(ov["ok"], true);
    let ress = ov["residents"].as_array().unwrap();
    let r_discord = ress.iter().find(|r| r["id"] == id_discord).unwrap();
    let r_noclass = ress.iter().find(|r| r["id"] == id_noclass).unwrap();
    let r_unknown = ress.iter().find(|r| r["id"] == id_unknown).unwrap();

    assert_eq!(r_discord["agent"], "sniperbot_analyst");
    assert_eq!(r_discord["swarm_class"], "discord");
    assert_eq!(r_noclass["swarm_class"], Value::Null);
    assert_eq!(r_unknown["swarm_class"], Value::Null);

    // `/api/state` et `/api/detail` reflètent aussi `swarm_class` à côté de `swarm_kind`/`swarm_agent`.
    let st = eng.get("/api/state").json();
    let servers = st["servers"].as_array().unwrap();
    let s_discord = servers.iter().find(|s| s["id"] == id_discord).unwrap();
    let s_noclass = servers.iter().find(|s| s["id"] == id_noclass).unwrap();
    let s_unknown = servers.iter().find(|s| s["id"] == id_unknown).unwrap();

    assert_eq!(s_discord["swarm_kind"], "resident");
    assert_eq!(s_discord["swarm_agent"], "sniperbot_analyst");
    assert_eq!(s_discord["swarm_class"], "discord");
    assert_eq!(s_noclass["swarm_class"], Value::Null);
    assert_eq!(s_unknown["swarm_class"], Value::Null);

    let det = eng.get(&format!("/api/detail?id={id_discord}")).json();
    assert_eq!(det["swarm_class"], "discord");

    // 4. Nouveaux outils résidents (history, logs, session, config) + changement de classe
    let hist = eng.action(&json!({
        "action": "swarm_resident_history",
        "server_id": id_discord,
        "limit": 10
    }));
    assert_eq!(hist["ok"], true);
    assert_eq!(hist["data"]["entries"][0]["reply"], "pong");

    let logs = eng.action(&json!({
        "action": "swarm_resident_logs",
        "server_id": id_discord,
        "tail": 4096
    }));
    assert_eq!(logs["ok"], true);
    assert_eq!(logs["data"]["bytes"], 22);

    let sess_set = eng.action(&json!({
        "action": "swarm_resident_session",
        "server_id": id_discord,
        "op": "set",
        "session_id": "sess-custom-7"
    }));
    assert_eq!(sess_set["ok"], true);
    assert_eq!(sess_set["data"]["session"], "sess-custom-7");

    let sess_rst = eng.action(&json!({
        "action": "swarm_resident_session",
        "server_id": id_discord,
        "op": "reset"
    }));
    assert_eq!(sess_rst["ok"], true);
    assert_eq!(sess_rst["data"]["hadSession"], true);

    let rcfg = eng.action(&json!({
        "action": "swarm_resident_config",
        "server_id": id_discord
    }));
    assert_eq!(rcfg["ok"], true);
    assert_eq!(rcfg["data"]["read_only"], true);
    assert_eq!(rcfg["data"]["files"]["prompt.md"], "Tu es Sniperbot.");

    let set_cls = eng.action(&json!({
        "action": "swarm_set_agent_class",
        "server_id": id_noclass,
        "class": "verifier"
    }));
    assert_eq!(set_cls["ok"], true);
    assert_eq!(set_cls["class"], "verifier");
    let det_nc = eng.get(&format!("/api/detail?id={id_noclass}")).json();
    assert_eq!(det_nc["swarm_class"], "verifier");
}
