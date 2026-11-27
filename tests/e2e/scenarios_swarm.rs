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
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
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
                                    if let Some(v) =
                                        line.to_ascii_lowercase().strip_prefix("content-length:")
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
                        continue;
                    }
                    let req = String::from_utf8_lossy(&raw);
                    let first = req.lines().next().unwrap_or("");
                    if first.starts_with("GET /health") {
                        let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
                        let _ = stream.write_all(resp.as_bytes());
                        continue;
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
                            continue;
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
        "create_agent" => json!({"created": args["name"], "runner": args["runner"]}),
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
        "prompt": "Tu es Bob."
    }));
    assert_eq!(cr["ok"], true);
    assert_eq!(cr["data"]["created"], "bob");

    let stop = eng.action(&json!({
        "action": "swarm_resident_stop",
        "server_id": res_id,
        "clearQueue": true
    }));
    assert_eq!(stop["ok"], true);
    assert_eq!(stop["data"]["stopped"], true);
}
