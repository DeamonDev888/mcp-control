// Import « collez n'importe quoi » : parsing des configs mcpServers/mcp_servers (porté de mcp-doc.ts).
// (split v0.7.20 : manage.rs reste la racine — API stable via réexport)
use super::*;

pub fn parse_servers_doc(raw: &str) -> Result<Vec<(String, Value)>, String> {
    let parsed: Value = serde_json::from_str(raw).map_err(|e| format!("JSON invalide: {}", e))?;
    let obj = parsed.as_object().ok_or("attendu un objet JSON")?;
    let map = obj
        .get("mcpServers")
        .or_else(|| obj.get("mcp_servers"))
        .and_then(|w| w.as_object())
        .unwrap_or(obj); // map brute
    Ok(map.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
}

pub(super) fn entry_to_server(id: String, name: &str, entry: &Value) -> Option<Server> {
    let command = entry.get("command").and_then(|x| x.as_str());
    let url = entry.get("url").and_then(|x| x.as_str());
    let args: Vec<String> = entry
        .get("args")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|i| i.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    // env du bloc mcpServers (`"env": {"CLÉ": "valeur"}`) : atteint le process
    // lancé via StartCmd.env → spawn_process/probe_stdio (lot 3, audit 03
    // constat 5 — la clé d'API ne doit plus être jetée à l'import).
    let env: Vec<(String, String)> = env_from_json(entry);
    if let Some(cmd) = command {
        let workdir = entry.get("cwd").and_then(|x| x.as_str()).map(String::from);
        return Some(Server {
            start_cmd: Some(StartCmd {
                program: cmd.to_string(),
                args: args.clone(),
                workdir: workdir.clone(),
                env: env.clone(),
            }),
            id,
            label: name.to_string(),
            transport: Transport::Stdio {
                program: cmd.to_string(),
                args,
                workdir,
                env,
            },
            origin: Origin::Manual,
            enabled: true,
            state: SState::Down,
            pid: None,
            bound_host: None,
            server_name: None,
            version: None,
            protocol: None,
            sse: false,
            sse_endpoint: false,
            ws: false,
            lang: None,
            framework: None,
            in_container: false,
            container_name: None,
            via_ssh: false,
            tools: vec![],
            latency_ms: None,
            last_seen_ts: None,
            last_error: None,
            auto_restart: true,
            favorite: false,
            tools_disabled: vec![],
        });
    }
    if let Some(u) = url {
        let (host, port, path) = parse_url_parts(u)?;
        return Some(Server {
            start_cmd: None,
            id,
            label: name.to_string(),
            transport: Transport::Http {
                host: Some(host),
                port,
                path,
            },
            origin: Origin::Manual,
            enabled: true,
            state: SState::Down,
            pid: None,
            bound_host: None,
            server_name: None,
            version: None,
            protocol: None,
            sse: false,
            sse_endpoint: false,
            ws: false,
            lang: None,
            framework: None,
            in_container: false,
            container_name: None,
            via_ssh: false,
            tools: vec![],
            latency_ms: None,
            last_seen_ts: None,
            last_error: None,
            auto_restart: true,
            favorite: false,
            tools_disabled: vec![],
        });
    }
    None
}

pub(super) fn parse_url_parts(u: &str) -> Option<(String, u16, String)> {
    // TLS non supporté par ce client (std sans rustls) : une URL https finirait en
    // sonde plaintext illisible et en DOWN permanent — on refuse proprement.
    // (import_mcp_text renvoie un message clair pour https://)
    let rest = u.strip_prefix("http://")?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, "/mcp".to_string()),
    };
    let (host, port) = if let Some(after_open) = hostport.strip_prefix('[') {
        let (h, after_close) = after_open.split_once(']')?;
        if h.is_empty() {
            return None;
        }
        let port = if let Some(p_str) = after_close.strip_prefix(':') {
            p_str.parse::<u16>().ok()?
        } else if after_close.is_empty() {
            80
        } else {
            return None;
        };
        (h.to_string(), port)
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) if !h.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => {
                (h.to_string(), p.parse().ok()?)
            }
            _ => (hostport.to_string(), 80),
        }
    };
    if host.is_empty() || port == 0 {
        return None;
    }
    Some((host, port, path))
}

fn sanitize_import_id(raw: &str) -> String {
    let clean: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = clean.trim_matches('-');
    if trimmed.is_empty() {
        "serveur".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Porté de mcp-import.ts : collez n'importe quoi — snippet mcp.json, commande
/// npx/uvx/docker, ligne `claude mcp add`, ou simple URL → serveurs ajoutés.
pub fn import_mcp_text(state: &Arc<AppState>, raw: &str) -> Value {
    let text = raw.trim();
    if text.is_empty() {
        return json!({"ok": false, "error": "texte vide"});
    }

    // 1. JSON (mcp.json)
    if text.starts_with('{') {
        match parse_servers_doc(text) {
            Ok(entries) if !entries.is_empty() => {
                let mut added = 0usize;
                let mut skipped: Vec<Value> = Vec::new();
                for (name, entry) in entries {
                    let id = format!("store-{}", sanitize_import_id(&name));
                    let mut servers = lockv(&state.servers);
                    if servers.iter().any(|s| s.id == id) {
                        continue;
                    }
                    if let Some(srv) = entry_to_server(id, &name, &entry) {
                        let label = srv.label.clone();
                        servers.push(srv);
                        drop(servers);
                        add_log(
                            state,
                            "info",
                            &format!("[boutique] import JSON : {}", label),
                        );
                        added += 1;
                    } else {
                        // (lot 3, audit 03 constat 6) une entrée {"url":"https://…"}
                        // était écartée EN SILENCE (le `?` de parse_url_parts
                        // renvoie None) et la réponse restait ok:true. Chaque
                        // entrée écartée est désormais listée avec son motif —
                        // même message que le collage d'une URL https directe.
                        let est_https = entry
                            .get("url")
                            .and_then(|u| u.as_str())
                            .map(|u| u.starts_with("https"))
                            .unwrap_or(false);
                        let motif = if est_https {
                            "TLS non supporté : MCP Control ne sonde que du http:// (passez par un tunnel http local pour un endpoint https)".to_string()
                        } else {
                            "entrée non reconnue (ni commande stdio, ni URL http://)".to_string()
                        };
                        add_log(
                            state,
                            "warn",
                            &format!("[boutique] import JSON : {} écarté ({})", name, motif),
                        );
                        skipped.push(json!({"name": name, "reason": motif}));
                        drop(servers);
                    }
                }
                if added > 0 {
                    save_config(state);
                }
                return json!({"ok": true, "added": added, "source": "json", "skipped": skipped});
            }
            Ok(_) => { /* json vide → continuer sur les autres pistes */ }
            Err(e) => return json!({"ok": false, "error": e}),
        }
    }

    // 2. `claude mcp add [--transport ...] [--scope ...] [-e K=V] <name> [--] <cmd> [args...]`
    let toks: Vec<&str> = text.split_whitespace().collect();
    if toks.len() >= 4 && toks[0] == "claude" && toks[1] == "mcp" && toks[2] == "add" {
        let mut i = 3usize;
        let mut positional: Vec<&str> = Vec::new();
        while i < toks.len() {
            if toks[i] == "--" {
                positional.extend_from_slice(&toks[i + 1..]);
                break;
            }
            if matches!(
                toks[i],
                "-e" | "--env" | "-s" | "--scope" | "-t" | "--transport"
            ) {
                i += 2;
                continue;
            }
            if toks[i].starts_with('-') {
                i += 1;
                continue;
            }
            positional.push(toks[i]);
            i += 1;
        }
        if positional.len() >= 2 {
            let name = sanitize_import_id(positional[0]);
            let program = positional[1].to_string();
            let args: Vec<String> = positional[2..].iter().map(|s| s.to_string()).collect();
            return push_imported(state, name, program, args);
        }
    }

    // 3. URL directe → serveur HTTP (https refusé : pas de TLS dans ce client std)
    if text.starts_with("https://") {
        return json!({"ok": false, "error": "TLS non supporté : MCP Control ne sonde que du http:// (passez par un tunnel http local pour un endpoint https)"});
    }
    if text.starts_with("http://") {
        if let Some((host, port, path)) = parse_url_parts(text) {
            let name = sanitize_import_id(&name_from_url(&host));
            let base_id = format!("store-{}", name);
            let mut servers = lockv(&state.servers);
            let already_same_endpoint = servers.iter().any(|s| {
                matches!(
                    &s.transport,
                    Transport::Http { host: Some(h), port: p, path: pa }
                    if h == &host && *p == port && pa == &path
                )
            });
            let id = if servers.iter().any(|s| s.id == base_id) {
                format!("store-{}-{}", name, port)
            } else {
                base_id
            };
            if !already_same_endpoint && !servers.iter().any(|s| s.id == id) {
                servers.push(Server {
                    start_cmd: None,
                    id: id.clone(),
                    label: name.clone(),
                    transport: Transport::Http {
                        host: Some(host),
                        port,
                        path,
                    },
                    origin: Origin::Manual,
                    enabled: true,
                    state: SState::Down,
                    pid: None,
                    bound_host: None,
                    server_name: None,
                    version: None,
                    protocol: None,
                    sse: false,
                    sse_endpoint: false,
                    ws: false,
                    lang: None,
                    framework: None,
                    in_container: false,
                    container_name: None,
                    via_ssh: false,
                    tools: vec![],
                    latency_ms: None,
                    last_seen_ts: None,
                    last_error: None,
                    auto_restart: true,
                    favorite: false,
                    tools_disabled: vec![],
                });
                drop(servers);
                save_config(state);
                add_log(state, "info", &format!("[boutique] import URL : {}", name));
                return json!({"ok": true, "added": 1, "source": "url"});
            }
            return json!({"ok": true, "added": 0, "source": "url", "note": "déjà présent"});
        }
    }

    // 4. Commande npx/uvx/docker → serveur stdio
    if let Some(first) = toks.first() {
        if ["npx", "uvx", "uv", "docker", "node", "python", "python3"].contains(first) {
            // premier token qui n'est pas une option (-y, --yes…) = le package
            let spec = toks
                .iter()
                .skip(1)
                .find(|t| !t.starts_with('-'))
                .copied()
                .unwrap_or("");
            let name = if spec.is_empty() {
                "serveur-importe".to_string()
            } else {
                cleanup_name(spec)
            };
            let name = format!("{}-{}", name, now_compact());
            let program = first.to_string();
            let args: Vec<String> = toks[1..].iter().map(|s| s.to_string()).collect();
            return push_imported(state, name, program, args);
        }
    }

    json!({"ok": false, "error": "rien de reconnu : colle un mcp.json, une commande npx/uvx, une URL, ou `claude mcp add ...`"})
}

pub(super) fn push_imported(
    state: &Arc<AppState>,
    name: String,
    program: String,
    args: Vec<String>,
) -> Value {
    let id = format!("store-{}", name);
    let mut servers = lockv(&state.servers);
    if servers.iter().any(|s| s.id == id) {
        return json!({"ok": true, "added": 0, "note": "déjà présent"});
    }
    servers.push(Server {
        start_cmd: Some(StartCmd {
            program: program.clone(),
            args: args.clone(),
            workdir: None,
            env: Vec::new(),
        }),
        id: id.clone(),
        label: name.clone(),
        transport: Transport::Stdio {
            program,
            args,
            workdir: None,
            env: Vec::new(),
        },
        origin: Origin::Manual,
        enabled: true,
        state: SState::Down,
        pid: None,
        bound_host: None,
        server_name: None,
        version: None,
        protocol: None,
        sse: false,
        sse_endpoint: false,
        ws: false,
        lang: None,
        framework: None,
        in_container: false,
        container_name: None,
        via_ssh: false,
        tools: vec![],
        latency_ms: None,
        last_seen_ts: None,
        last_error: None,
        auto_restart: true,
        favorite: false,
        tools_disabled: vec![],
    });
    drop(servers);
    save_config(state);
    add_log(
        state,
        "info",
        &format!("[boutique] import commande : {}", name),
    );
    json!({"ok": true, "added": 1, "source": "commande"})
}

/// Porté de mcp-import.ts nameFromUrl : "mcp.linear.app" → "linear"
/// (dégage api/mcp/www, garde la marque).
pub(super) fn name_from_url(host: &str) -> String {
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    let mut it: Vec<&str> = labels.to_vec();
    while it.len() > 1 && ["api", "mcp", "www"].contains(&it[0]) {
        it.remove(0);
    }
    it.first()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "server".to_string())
}
