// Pairs MCP Control du LAN et machines distantes (SSH).
// (split v0.7.20 : manage.rs reste la racine — API stable via réexport)
use super::*;

fn ssh_status_slot() -> &'static std::sync::Mutex<
    std::collections::HashMap<(usize, String), crate::types::SshHostStatus>,
> {
    static SLOT: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<(usize, String), crate::types::SshHostStatus>>,
    > = std::sync::OnceLock::new();
    SLOT.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

pub fn get_ssh_status(state: &Arc<AppState>, ip: &str) -> Option<crate::types::SshHostStatus> {
    let key = (Arc::as_ptr(state) as usize, ip.to_string());
    lockv(ssh_status_slot()).get(&key).cloned()
}

pub fn set_ssh_status(state: &Arc<AppState>, ip: &str, status: crate::types::SshHostStatus) {
    let key = (Arc::as_ptr(state) as usize, ip.to_string());
    lockv(ssh_status_slot()).insert(key, status);
}

pub fn clear_ssh_status_one(state: &Arc<AppState>, ip: &str) {
    let key = (Arc::as_ptr(state) as usize, ip.to_string());
    lockv(ssh_status_slot()).remove(&key);
}

pub fn clear_ssh_status_all(state: &Arc<AppState>) {
    let key_ptr = Arc::as_ptr(state) as usize;
    lockv(ssh_status_slot()).retain(|(ptr, _), _| *ptr != key_ptr);
}

/// Ne saute notre propre port UI QUE sur les adresses loopback locales — jamais
/// sur une IP distante du LAN (deux machines avec le même port 5090 par défaut).
pub(crate) fn should_skip_peer_port(ip: &str, port: u16, own_port: u16) -> bool {
    port == own_port && crate::is_loopback_bind(ip)
}

pub fn discover_peers(state: &Arc<AppState>) {
    if !lockv(&state.settings).network_enabled {
        return;
    }
    crate::auth::net::set_config_path(&state.config_path);
    let ports = lockv(&state.settings).network_ports.clone();
    let own_port = lockv(&state.settings).ui_port;
    let prefixes = network_prefixes();
    let my_ips: Vec<String> = crate::manage::local_ips();
    let mut hosts: Vec<String> = vec!["127.0.0.1".to_string()]; // autres instances locales
    if std::env::var_os("MCC_NO_LOCAL_DISCOVERY").is_none() {
        for (prefix, self_ip) in &prefixes {
            for h in probe::arp_hosts(prefix, self_ip) {
                if !hosts.contains(&h) {
                    hosts.push(h);
                }
            }
        }
    }
    // une cible qui est une de NOS ips sur NOTRE port UI = nous-mêmes -> skip
    hosts.retain(|h| !(my_ips.contains(h)));
    // Pairs déjà appairés (`auth/clients/*.pin.json`) : sondés même s'ils ne
    // sont pas dans le cache ARP ou s'ils utilisent un port UI non standard.
    let pinned_targets =
        crate::auth::net::pinned_peer_targets(&crate::auth::net::current_config_dir());
    for (h, _) in &pinned_targets {
        if !hosts.contains(h) {
            hosts.push(h.clone());
        }
    }
    add_log(
        state,
        "info",
        &format!(
            "pairs : sondage {} cibles (ARP + loopback) sur ports UI {:?}",
            hosts.len(),
            ports
        ),
    );

    use std::sync::atomic::{AtomicU32, Ordering};
    let new_peers = std::sync::Arc::new(AtomicU32::new(0));
    // secret partagé : la sonde de pairs présente NOTRE code d'accès (le pair doit
    // avoir la même valeur dans son config.json pour accepter /api/state)
    let access = lockv(&state.settings).api_access.clone();
    let mut handles = Vec::new();
    for ip in hosts {
        let mut target_ports = ports.clone();
        for (ph, pp) in &pinned_targets {
            if ph == &ip && !target_ports.contains(pp) {
                target_ports.push(*pp);
            }
        }
        let my_ips = my_ips.clone();
        let state = state.clone();
        let new_peers = new_peers.clone();
        let access = access.clone();
        handles.push(std::thread::spawn(move || {
            for port in &target_ports {
                if should_skip_peer_port(&ip, *port, own_port)
                    || (*port == own_port && my_ips.contains(&ip))
                {
                    continue;
                } // ne jamais se sonder soi-même sur notre port
                let pr = probe::probe_peer(&ip, *port, access.as_deref());
                if pr.alive {
                    if let Some(servers) = pr.servers {
                        let count = servers
                            .get("stats")
                            .and_then(|s| s.get("total"))
                            .and_then(|x| x.as_u64())
                            .unwrap_or(0);
                        let mut peers = lockv(&state.peers);
                        let is_new = !peers.contains_key(&ip);
                        peers.insert(
                            ip.clone(),
                            crate::types::Peer {
                                ip: ip.clone(),
                                ui_port: *port,
                                last_seen: crate::types::now_ts(),
                                servers,
                                via_ssh: false,
                            },
                        );
                        drop(peers);
                        if is_new {
                            new_peers.fetch_add(1, Ordering::Relaxed);
                        }
                        add_log(
                            &state,
                            "info",
                            &format!(
                                "[pairs] MCP Control détecté sur {}:{} ({} serveurs)",
                                ip, port, count
                            ),
                        );
                        break;
                    }
                }
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
    let total = lockv(&state.peers).len();
    let nouveaux = new_peers.load(Ordering::Relaxed);
    add_log(
        state,
        "info",
        &format!(
            "pairs : {} peer(s) connu(s) ({} nouveau(x))",
            total, nouveaux
        ),
    );
}

/// Importe depuis un pair tous les serveurs HTTP qu'on n'a pas encore (host:ip).
#[allow(unused_must_use)]
pub fn import_peer(state: &Arc<AppState>, ip: &str) -> Value {
    let snapshot = {
        let peers = lockv(&state.peers);
        peers
            .get(ip)
            .map(|p| (p.ui_port, p.via_ssh, p.servers.clone()))
    };
    let (ui_port, peer_via_ssh, snapshot) = match snapshot {
        Some(x) => x,
        None => return json!({"ok": false, "error": "pair inconnu"}),
    };
    let in_ssh_hosts = lockv(&state.settings).ssh_hosts.iter().any(|h| h.ip == ip);
    let mut added = 0usize;
    if let Some(arr) = snapshot.get("servers").and_then(|x| x.as_array()) {
        for s in arr {
            if s.get("transport").and_then(|x| x.as_str()) != Some("http") {
                continue;
            }
            if s.get("enabled") == Some(&json!(false)) {
                continue;
            }
            let (raw_host, port, path, name, version) = (
                s.get("transport_info")
                    .and_then(|t| t.get("host"))
                    .and_then(|x| x.as_str())
                    .map(String::from),
                s.get("transport_info")
                    .and_then(|t| t.get("port"))
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0) as u16,
                s.get("transport_info")
                    .and_then(|t| t.get("path"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("/mcp")
                    .to_string(),
                s.get("server_name")
                    .and_then(|x| x.as_str())
                    .map(String::from),
                s.get("version").and_then(|x| x.as_str()).map(String::from),
            );
            let in_container = s
                .get("in_container")
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
            let container_name = s
                .get("container_name")
                .and_then(|x| x.as_str())
                .map(String::from);
            if port == 0 {
                continue;
            }
            let is_local_on_peer = raw_host
                .as_deref()
                .map(|h| crate::is_loopback_bind(h) || matches!(h, "0.0.0.0" | "::" | "[::]" | "*"))
                .unwrap_or(true);
            // host local côté pair → réécrire vers l'IP du pair si ce n'est pas notre machine
            let host = if is_local_on_peer {
                if crate::is_loopback_bind(ip) {
                    continue; // même machine : localhost reste localhost
                }
                ip.to_string()
            } else {
                raw_host.unwrap_or_else(|| ip.to_string())
            };
            let import_via_ssh = peer_via_ssh || (is_local_on_peer && in_ssh_hosts);
            let mut servers = lockv(&state.servers);
            let already = servers.iter().any(|srv| {
                matches!(&srv.transport,
                Transport::Http { host: Some(h), port: p, .. } if h == &host && p == &port)
            });
            if already {
                continue;
            }
            let id = format!("net-{}.{}", host, port);
            if servers.iter().any(|s| s.id == id) {
                continue;
            }
            servers.push(Server {
                id,
                label: name.clone().unwrap_or_else(|| format!("{}:{}", host, port)),
                transport: Transport::Http {
                    host: Some(host.clone()),
                    port,
                    path,
                },
                origin: Origin::Discovered,
                enabled: true,
                state: SState::Down,
                auto_restart: true,
                favorite: false,
                pid: None,
                bound_host: None,
                server_name: name,
                version,
                protocol: None,
                sse: false,
                sse_endpoint: false,
                ws: false,
                lang: None,
                framework: None,
                in_container,
                container_name,
                via_ssh: import_via_ssh,
                tools: vec![],
                latency_ms: None,
                last_seen_ts: None,
                last_error: None,
                start_cmd: None,
                tools_disabled: vec![],
            });
            added += 1;
        }
    }
    drop(snapshot);
    save_config(state);
    add_log(
        state,
        "info",
        &format!(
            "[pairs] import depuis {} (UI {}) : {} serveur(s) ajouté(s)",
            ip, ui_port, added
        ),
    );
    json!({"ok": true, "added": added})
}

// ---------- découverte SSH et bootstrap CLI d'une machine distante ----------

/// Premier contact / bootstrap SSH avec une machine distante :
/// 1. Inspecte OS, architecture, présence et état du CLI `mcp-control` ;
/// 2. Si absent et `auto_install_cli` (ou `force_install`), télécharge et installe
///    le binaire CLI officiel vérifié par SHA-256 ;
/// 3. S'assure que le moteur CLI tourne sur la machine distante ;
/// 4. Remonte son catalogue dans `state.peers` (`via_ssh: true`).
pub fn ssh_bootstrap(
    state: &Arc<AppState>,
    host_cfg: &crate::types::SshHost,
    force_install: bool,
) -> Value {
    let ip = host_cfg.ip.as_str();
    let user = host_cfg.user.as_str();
    let target = probe::SshTarget::from_host(host_cfg);
    add_log(
        state,
        "info",
        &format!("[ssh] connexion à {user}@{ip} — inspection de la machine et du CLI…"),
    );
    let env = match probe::ssh_inspect_remote(&target, 8) {
        Ok(e) => e,
        Err(e) => {
            let st = crate::types::SshHostStatus {
                last_check_ts: Some(crate::types::now_ts()),
                last_error: Some(e.clone()),
                ..Default::default()
            };
            set_ssh_status(state, ip, st);
            add_log(state, "warn", &e);
            return json!({"ok": false, "ssh_ok": false, "error": e});
        }
    };

    let mut status = crate::types::SshHostStatus {
        os: Some(env.os.clone()),
        arch: Some(env.arch.clone()),
        cli_installed: env.cli_path.is_some(),
        cli_path: env.cli_path.clone(),
        cli_version: env.cli_version.clone(),
        cli_running: env.cli_running,
        cli_port: env.cli_port,
        last_check_ts: Some(crate::types::now_ts()),
        last_error: None,
    };
    set_ssh_status(state, ip, status.clone());

    if !status.cli_installed && (host_cfg.auto_install_cli || force_install) {
        add_log(
            state,
            "info",
            &format!(
                "[ssh {ip}] CLI mcp-control absent ({}/{}) — recherche du binaire officiel…",
                env.os, env.arch
            ),
        );
        match latest_cli_download(&env.os, &env.arch) {
            Ok((tag, url, sha256)) => {
                add_log(
                    state,
                    "info",
                    &format!(
                        "[ssh {ip}] installation du CLI mcp-control {tag} ({}/{}) avec vérification SHA-256…",
                        env.os, env.arch
                    ),
                );
                match probe::ssh_install_cli(&target, &env.os, &url, &sha256) {
                    Ok(installed_path) => {
                        status.cli_installed = true;
                        status.cli_path = Some(installed_path);
                        status.cli_version =
                            Some(tag.strip_prefix('v').unwrap_or(&tag).to_string());
                        status.last_error = None;
                        add_log(
                            state,
                            "info",
                            &format!(
                                "[ssh {ip}] CLI mcp-control {tag} installé et vérifié ({})",
                                status.cli_path.as_deref().unwrap_or("")
                            ),
                        );
                    }
                    Err(e) => {
                        status.last_error = Some(e.clone());
                        add_log(
                            state,
                            "warn",
                            &format!("[ssh {ip}] installation auto du CLI échouée : {e}"),
                        );
                    }
                }
            }
            Err(e) => {
                status.last_error = Some(e.clone());
                add_log(
                    state,
                    "info",
                    &format!("[ssh {ip}] installation auto du CLI ignorée : {e}"),
                );
            }
        }
        status.last_check_ts = Some(crate::types::now_ts());
        set_ssh_status(state, ip, status.clone());
    }

    if let Some(ref cli_path) = status.cli_path.clone() {
        if !status.cli_running {
            match probe::ssh_ensure_cli_running(&target, &env.os, cli_path) {
                Ok(port) => {
                    status.cli_running = true;
                    status.cli_port = Some(port);
                    status.last_error = None;
                    add_log(
                        state,
                        "info",
                        &format!("[ssh {ip}] moteur CLI mcp-control démarré sur 127.0.0.1:{port}"),
                    );
                }
                Err(e) => {
                    status.last_error = Some(e.clone());
                    add_log(
                        state,
                        "warn",
                        &format!("[ssh {ip}] démarrage du CLI distant impossible : {e}"),
                    );
                }
            }
            status.last_check_ts = Some(crate::types::now_ts());
            set_ssh_status(state, ip, status.clone());
        }
    }

    let cfg_dir_fallback = env.cli_config_dir.clone().or_else(|| {
        status
            .cli_path
            .as_deref()
            .and_then(|p| p.rsplit_once(['/', '\\']).map(|(d, _)| d.to_string()))
    });

    if let Some(port) = status.cli_port.filter(|_| status.cli_running) {
        if let Some(snap) = probe::ssh_read_remote_peer_state(
            &target,
            &env.os,
            status.cli_path.as_deref(),
            cfg_dir_fallback.as_deref(),
            port,
        ) {
            let count = snap
                .get("stats")
                .and_then(|s| s.get("total"))
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let mut peers = lockv(&state.peers);
            peers.insert(
                ip.to_string(),
                crate::types::Peer {
                    ip: ip.to_string(),
                    ui_port: port,
                    last_seen: crate::types::now_ts(),
                    servers: snap,
                    via_ssh: true,
                },
            );
            drop(peers);
            add_log(
                state,
                "info",
                &format!(
                    "[ssh {ip}] instance MCP Control distante détectée via SSH (:{port}, {count} serveur(s))"
                ),
            );
        }
    }

    if force_install
        && status.last_error.is_some()
        && (!status.cli_installed || !status.cli_running)
    {
        return json!({
            "ok": false,
            "ssh_ok": true,
            "os": status.os,
            "arch": status.arch,
            "cli_installed": status.cli_installed,
            "cli_path": status.cli_path,
            "cli_version": status.cli_version,
            "cli_running": status.cli_running,
            "cli_port": status.cli_port,
            "error": status.last_error,
        });
    }

    json!({
        "ok": true,
        "ssh_ok": true,
        "os": status.os,
        "arch": status.arch,
        "cli_installed": status.cli_installed,
        "cli_path": status.cli_path,
        "cli_version": status.cli_version,
        "cli_running": status.cli_running,
        "cli_port": status.cli_port,
        "error": status.last_error,
    })
}

/// Découverte SSH complète d'une machine distante enregistrée (`SshHost`).
pub fn ssh_discover_host(state: &Arc<AppState>, host_cfg: &crate::types::SshHost) -> Value {
    let ip = host_cfg.ip.as_str();
    let target = probe::SshTarget::from_host(host_cfg);

    let boot = ssh_bootstrap(state, host_cfg, false);
    if boot.get("ok").and_then(|x| x.as_bool()) != Some(true) {
        return boot;
    }

    let cli_port = get_ssh_status(state, ip).and_then(|s| s.cli_port);
    let mut ports = match probe::ssh_netstat_target(&target, 8) {
        Ok(p) => p,
        Err(e) => {
            add_log(state, "warn", &e);
            return json!({"ok": false, "error": e});
        }
    };
    if let Some(cp) = cli_port {
        ports.retain(|&p| p != cp);
    }

    let mut candidate_paths: Vec<String> = vec!["/mcp".to_string()];
    {
        let peers = lockv(&state.peers);
        if let Some(p) = peers.get(ip) {
            if let Some(arr) = p.servers.get("servers").and_then(|x| x.as_array()) {
                for s in arr {
                    if let Some(path) = s
                        .get("transport_info")
                        .and_then(|t| t.get("path"))
                        .and_then(|x| x.as_str())
                        .map(str::trim)
                        .filter(|x| !x.is_empty())
                    {
                        if !candidate_paths.iter().any(|c| c == path) {
                            candidate_paths.push(path.to_string());
                        }
                    }
                }
            }
        }
    }
    {
        let servers = lockv(&state.servers);
        for s in servers.iter() {
            if s.via_ssh {
                if let Transport::Http {
                    host: Some(ref h),
                    ref path,
                    ..
                } = s.transport
                {
                    if h == ip
                        && !path.trim().is_empty()
                        && !candidate_paths.iter().any(|c| c == path)
                    {
                        candidate_paths.push(path.clone());
                    }
                }
            }
        }
    }
    let path_refs: Vec<&str> = candidate_paths.iter().map(String::as_str).collect();

    add_log(
        state,
        "info",
        &format!(
            "[ssh {ip}] {} port(s) en écoute (système écarté) — sonde MCP depuis la machine…",
            ports.len()
        ),
    );
    let found = match probe::ssh_probe_mcp_batch_target(&target, &ports, &path_refs, 4) {
        Ok(f) => f,
        Err(e) => {
            add_log(state, "warn", &e);
            return json!({"ok": false, "error": e});
        }
    };
    let mut serveurs: Vec<Value> = Vec::new();
    let mut imported = 0usize;
    let mut to_poll: Vec<String> = Vec::new();
    for (port, path, v) in &found {
        let res = v.get("result").cloned().unwrap_or(json!({}));
        let si = res.get("serverInfo").cloned().unwrap_or(json!({}));
        let raw_name = si.get("name").and_then(|x| x.as_str());
        let name = raw_name
            .map(String::from)
            .unwrap_or_else(|| format!("{ip}:{port}"));
        let version = si.get("version").and_then(|x| x.as_str()).map(String::from);
        let protocol = res
            .get("protocolVersion")
            .and_then(|x| x.as_str())
            .map(String::from);
        let lang = probe::detect_lang_http(&probe::RespHead::default(), raw_name);
        let framework = crate::frameworks::detect(
            raw_name,
            protocol.as_deref(),
            false,
            false,
            false,
            lang.as_deref(),
        )
        .map(|f| f.id.to_string());
        let id = format!("net-{ip}.{port}");
        let mut servers = lockv(&state.servers);
        // DÉ-DUPLICATION SSH : un même serveur (même nom + même machine)
        // ne doit créer qu'UNE entrée. On cherche d'abord l'exact
        // (hôte+port), puis le même nom sur la même machine (le serveur a
        // changé de port) — sinon chaque import recrée un doublon.
        let existing_idx = {
            let mut hit: Option<usize> = None;
            for (i, sv) in servers.iter().enumerate() {
                // une entrée désactivée par l'utilisateur ne doit jamais être
                // détournée ni rafraîchie par la découverte SSH
                if !sv.enabled {
                    continue;
                }
                let same_port = matches!(&sv.transport,
                    Transport::Http { host: Some(h), port: sp, .. } if h == ip && sp == port);
                let same_name = raw_name.is_some()
                    && sv
                        .server_name
                        .as_deref()
                        .map(|n| n.eq_ignore_ascii_case(raw_name.unwrap_or("")))
                        .unwrap_or(false)
                    && matches!(&sv.transport, Transport::Http { host: Some(h), .. } if h == ip);
                if same_port || same_name {
                    hit = Some(i);
                    break;
                }
            }
            hit
        };
        if let Some(idx) = existing_idx {
            let existing = &mut servers[idx];
            let existing_id = existing.id.clone();
            // les entrées manuelles gardent leur mode de sonde : on ne force
            // pas via_ssh (audit 2026-10-26 — sinon elles échouent ensuite
            // « machine SSH non enregistrée » dès l'oubli de l'hôte)
            if existing.origin != Origin::Manual {
                existing.via_ssh = true;
            }
            // le port et le chemin peuvent avoir changé : on suit le réel sondé
            if let Transport::Http {
                port: ref mut sp,
                path: ref mut spath,
                ..
            } = existing.transport
            {
                if *sp != *port {
                    *sp = *port;
                }
                if spath != path {
                    *spath = path.clone();
                }
            }
            existing.state = SState::Up;
            // surcharge CONDITIONNELLE (audit 2026-10-26) : une sonde validée
            // sans serverInfo/version ne doit pas effacer ce qu'on savait déjà
            if let Some(n) = raw_name {
                existing.server_name = Some(n.to_string());
                // le label fabriqué « ip:port » suit le nom annoncé ; un label
                // saisi par l'utilisateur est conservé
                if existing.label.trim().is_empty() || existing.label.contains(':') {
                    existing.label = n.to_string();
                }
            }
            if version.is_some() {
                existing.version = version.clone();
            }
            if protocol.is_some() {
                existing.protocol = protocol.clone();
            }
            if lang.is_some() {
                existing.lang = lang.clone();
            }
            if framework.is_some() {
                existing.framework = framework.clone();
            }
            existing.last_seen_ts = Some(crate::types::now_ts());
            let should_poll = existing.via_ssh && existing.tools.is_empty();
            drop(servers);
            if should_poll && !to_poll.contains(&existing_id) {
                to_poll.push(existing_id);
            }
            serveurs.push(json!({"port": port, "path": path, "name": name, "imported": false}));
            continue;
        }
        servers.push(Server {
            id: id.clone(),
            label: name.clone(),
            transport: Transport::Http {
                host: Some(ip.to_string()),
                port: *port,
                path: path.clone(),
            },
            origin: Origin::Discovered,
            enabled: true,
            state: SState::Up,
            auto_restart: true,
            favorite: false,
            pid: None,
            bound_host: Some(ip.to_string()),
            server_name: raw_name.map(String::from),
            version: version.clone(),
            protocol,
            sse: false,
            sse_endpoint: false,
            ws: false,
            lang,
            framework,
            in_container: false,
            container_name: None,
            via_ssh: true,
            tools: vec![],
            latency_ms: None,
            last_seen_ts: Some(crate::types::now_ts()),
            last_error: None,
            start_cmd: None,
            tools_disabled: vec![],
        });
        drop(servers);
        imported += 1;
        to_poll.push(id.clone());
        serveurs.push(json!({"port": port, "path": path, "name": name, "imported": true}));
        add_log(
            state,
            "info",
            &format!(
                "[ssh {ip}] serveur MCP importé (via ssh) : {}:{}{} — {} {}",
                ip,
                port,
                path,
                name,
                version.as_deref().unwrap_or("")
            ),
        );
    }
    // fusion des doublons (même nom + même machine) : la découverte SSH
    // pouvait créer plusieurs entrées pour un même serveur
    merge_duplicate_servers(state);
    if !serveurs.is_empty() {
        save_config(state);
    }
    for sid in &to_poll {
        poll_one(state, sid);
    }
    add_log(
        state,
        "info",
        &format!(
            "[ssh {ip}] terminé : {} serveur(s) MCP trouvé(s), {} importé(s)",
            serveurs.len(),
            imported
        ),
    );
    json!({"ok": true, "ports": ports.len(), "found": imported, "serveurs": serveurs})
}

/// Compatibilité : connexion SSH simple par `(ip, user)`, en reprenant les options
/// éventuelles enregistrées dans `settings.ssh_hosts`.
#[allow(dead_code)]
pub fn ssh_discover(state: &Arc<AppState>, ip: &str, user: &str) -> Value {
    let cfg = {
        let st = lockv(&state.settings);
        st.ssh_hosts
            .iter()
            .find(|h| h.ip == ip)
            .cloned()
            .unwrap_or(crate::types::SshHost {
                ip: ip.to_string(),
                user: user.to_string(),
                port: None,
                identity_file: None,
                auto_install_cli: true,
            })
    };
    let cfg = crate::types::SshHost {
        user: if user.trim().is_empty() {
            cfg.user
        } else {
            user.to_string()
        },
        ..cfg
    };
    ssh_discover_host(state, &cfg)
}
