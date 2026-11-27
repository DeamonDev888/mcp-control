//! Couche RPC vers les serveurs Swarm-control (MCP HTTP/SSH/stdio + bridge
//! HTTP loopback optionnel `--bridge`).

use crate::probe;
use crate::types::{lockv, AppState, SState, Transport};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

/// Appelle un outil MCP sur un serveur suivi par MCP Control (HTTP local/LAN,
/// SSH ou stdio hybride), en réutilisant la pile `probe::call_*_tool` (y
/// compris la signature Ed25519 RFC 9421 via `auth::net`).
pub fn call_server_mcp(
    state: &Arc<AppState>,
    server_id: &str,
    tool: &str,
    arguments: Value,
    timeout_ms: u64,
) -> Result<(Value, u64), String> {
    let target = {
        let servers = lockv(&state.servers);
        let Some(s) = servers.iter().find(|s| s.id == server_id) else {
            return Err(format!("serveur inconnu : {server_id}"));
        };
        if s.state != SState::Up {
            return Err("serveur hors ligne (UP requis)".to_string());
        }
        if s.tools_disabled.iter().any(|t| t == tool) {
            return Err(format!("outil '{tool}' désactivé sur ce serveur"));
        }
        match &s.transport {
            Transport::Http { host, port, path } => {
                let h = s
                    .bound_host
                    .as_deref()
                    .or(host.as_deref())
                    .unwrap_or("127.0.0.1")
                    .to_string();
                (
                    Some((h, *port, path.clone(), s.protocol.clone(), s.via_ssh)),
                    false,
                )
            }
            Transport::Stdio { .. } => {
                if s.via_ssh {
                    return Err("serveur stdio SSH non supporté".to_string());
                }
                (None, true)
            }
        }
    };

    if let Some((host, port, path, proto, via_ssh)) = target.0 {
        if via_ssh {
            let sh = lockv(&state.settings)
                .ssh_hosts
                .iter()
                .find(|x| x.ip == host)
                .cloned();
            let Some(cfg) = sh else {
                return Err(
                    "machine SSH non enregistrée — réenregistre-la dans Machines".to_string(),
                );
            };
            let timeout_s = timeout_ms.div_ceil(1000).clamp(2, 600);
            probe::ssh_call_tool(
                &probe::SshTarget::from_host(&cfg),
                port,
                &path,
                proto.as_deref(),
                tool,
                arguments,
                timeout_s,
            )
        } else {
            probe::call_http_tool(
                &host,
                port,
                &path,
                proto.as_deref(),
                tool,
                arguments,
                timeout_ms,
            )
        }
    } else {
        let rpc = lockv(&state.children)
            .get(server_id)
            .and_then(|h| h.rpc.clone());
        match rpc {
            Some(rpc) => probe::call_stdio_tool(&rpc, tool, arguments, timeout_ms),
            None => Err("pont stdio hybride non prêt — redémarrez ou sondez le serveur".into()),
        }
    }
}

/// Déplie la réponse `tools/call` d'un serveur LiteMCP (`structuredContent` ou
/// `content[0].text` JSON) et propage proprement `isError: true`.
pub fn unwrap_tool_json(result: &Value) -> Result<Value, String> {
    let is_err = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let first_text = result
        .get("content")
        .and_then(Value::as_array)
        .and_then(|arr| {
            arr.iter().find_map(|item| {
                if item.get("type").and_then(Value::as_str) == Some("text") {
                    item.get("text").and_then(Value::as_str)
                } else {
                    None
                }
            })
        });
    if is_err {
        let msg = first_text
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| result.to_string());
        return Err(msg);
    }
    if let Some(sc) = result.get("structuredContent").filter(|v| !v.is_null()) {
        return Ok(sc.clone());
    }
    if let Some(txt) = first_text {
        let trimmed = txt.trim();
        if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
            return Ok(parsed);
        }
        return Ok(json!({ "text": txt }));
    }
    Ok(result.clone())
}

/// Appelle un endpoint du bridge HTTP optionnel (`swarm-control --bridge <port>`)
/// STRICTEMENT sur `127.0.0.1` avec une liste blanche de routes.
pub fn call_bridge_loopback(port: u16, op: &str, timeout_ms: u64) -> Result<Value, String> {
    let (method, path) = match op {
        "status" => ("GET", "/status"),
        "health" => ("GET", "/health"),
        "reset" => ("POST", "/reset"),
        _ => return Err(format!("opération bridge inconnue : {op}")),
    };
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let timeout = Duration::from_millis(timeout_ms.clamp(500, 15_000));
    let mut stream = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|e| format!("bridge 127.0.0.1:{port} injoignable : {e}"))?;
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|e| format!("écriture bridge : {e}"))?;

    let mut raw = Vec::new();
    const MAX_BRIDGE_BYTES: usize = 512 * 1024;
    let mut limited = stream.take((MAX_BRIDGE_BYTES + 1) as u64);
    let _ = limited.read_to_end(&mut raw);
    if raw.len() > MAX_BRIDGE_BYTES {
        return Err("réponse bridge trop volumineuse".to_string());
    }
    let sep = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "réponse HTTP bridge invalide".to_string())?;
    let head = String::from_utf8_lossy(&raw[..sep]);
    let status_code = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    let body_str = String::from_utf8_lossy(&raw[sep + 4..]).trim().to_string();
    if !(200..300).contains(&status_code) {
        return Err(format!("bridge HTTP {status_code}: {body_str}"));
    }
    if let Ok(v) = serde_json::from_str::<Value>(&body_str) {
        Ok(v)
    } else {
        Ok(json!({ "text": body_str }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwrap_deplie_json_text_et_erreurs() {
        let ok = json!({
            "content": [{"type": "text", "text": "{\"count\":2,\"agents\":[]}"}]
        });
        assert_eq!(unwrap_tool_json(&ok).unwrap()["count"], 2);

        let err = json!({
            "isError": true,
            "content": [{"type": "text", "text": "agent introuvable"}]
        });
        assert_eq!(unwrap_tool_json(&err).unwrap_err(), "agent introuvable");
    }
}
