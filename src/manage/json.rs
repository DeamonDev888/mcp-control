// Builders JSON de l API : état, détail, runtime + décision LAN.
// (split v0.7.20 : manage.rs reste la racine — API stable via réexport)
use super::*;

pub(super) fn now_compact() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64 % 100_000_000)
        .unwrap_or(0)
}

pub fn start_cmd_for(srv: &Server) -> Option<StartCmd> {
    match &srv.transport {
        Transport::Stdio {
            program,
            args,
            workdir,
            env,
        } => Some(StartCmd {
            program: program.clone(),
            args: args.clone(),
            workdir: workdir.clone(),
            env: env.clone(),
        }),
        Transport::Http { .. } => srv.start_cmd.clone(),
    }
}

// ---------- sérialisation UI ----------

pub(super) fn server_json(s: &Server, managed: bool) -> Value {
    let (ttype, extra) = match &s.transport {
        Transport::Http { host, port, path } => {
            ("http", json!({"host": host, "port": port, "path": path}))
        }
        Transport::Stdio {
            program,
            args,
            workdir,
            env,
        } => (
            "stdio",
            json!({
                "program": program, "args": args, "workdir": workdir,
                // noms SEULS (lot 3) : les valeurs d'env ne sortent jamais par
                // /api/state ni /api/detail.
                "env": crate::types::env_names(env),
            }),
        ),
    };
    let origin = s.origin.as_str();
    let auth = crate::auth::net::server_auth_status(s);
    json!({
        "id": s.id, "label": s.label, "transport": ttype, "transport_info": extra,
        "origin": origin, "enabled": s.enabled, "state": s.state.as_str(),
        "pid": s.pid, "bound_host": s.bound_host,
        "server_name": s.server_name, "version": s.version, "protocol": s.protocol,
        "tools_count": tools_actifs(s), "tools_total": s.tools.len(),
        "tools_disabled": s.tools_disabled,
        "latency_ms": s.latency_ms,
        "last_seen": s.last_seen_ts, "last_error": s.last_error,
        "can_start": s.start_cmd.is_some(), "auto_restart": s.auto_restart,
        "favorite": s.favorite,
        "sse": s.sse, "sse_endpoint": s.sse_endpoint, "ws": s.ws, "lang": s.lang,
        "framework": s.framework,
        "locked": auth.locked,
        "paired": auth.paired,
        "auth_keyid": auth.keyid,
        "auth_public_key": auth.public_key,
        "auth_fingerprint": auth.fingerprint,
        "via_ssh": s.via_ssh,
        // contrôle du process : maintenu par CE moteur MCP Control (children) ?
        "managed": managed,
        // commande de lancement (Start/Stop) — panneau Démarrage de l'UI
        "start_cmd": s.start_cmd.as_ref().map(|c| json!({
            "program": c.program, "args": c.args, "workdir": c.workdir,
            // noms SEULS (lot 3) : les valeurs d'env ne sortent jamais par l'API.
            "env": crate::types::env_names(&c.env),
        })),
    })
}

/// Ids des process actuellement maintenus par MCP Control (copie —
/// évite d'imbriquer le lock children sous le lock servers).
pub(super) fn managed_ids(state: &Arc<AppState>) -> std::collections::HashSet<String> {
    lockv(&state.children).keys().cloned().collect()
}

/// Décision pure « le LAN est-il joignable ? » — testable sans réseau.
/// Un VPN fait répondre SA passerelle alors que le LAN est coupé : si tous les
/// serveurs distants échouent en réseau, on conclut LAN coupé même si le ping
/// de la passerelle passe.
pub fn lan_decision(gw: Option<bool>, distants_ko: bool) -> Option<bool> {
    match (gw, distants_ko) {
        (Some(true), true) => Some(false),
        (None, true) => Some(false),
        (g, _) => g,
    }
}

/// Mesure si le LAN répond et publie le résultat.
/// Deux signaux, car un VPN fait répondre SA passerelle tout en coupant le LAN :
/// 1. ping des passerelles par défaut (aucune ne répond = réseau coupé) ;
/// 2. état des serveurs DISTANTS : s'ils échouent tous avec une erreur réseau
///    (ssh/connect/timeout/injoignable), c'est le LAN qui est coupé, pas eux.
pub fn refresh_lan_state(state: &Arc<AppState>) {
    // P1 rang 4 : aucun serveur distant (hors loopback) ET découverte réseau
    // coupée → inutile de lancer `ipconfig.exe` + `ping.exe` toutes les 30 s.
    let net_enabled = lockv(&state.settings).network_enabled;
    let distants: Vec<(bool, Option<String>)> = {
        let servers = lockv(&state.servers);
        servers
            .iter()
            .filter(|s| {
                s.enabled
                    && matches!(
                        &s.transport,
                        Transport::Http { host: Some(h), .. } if !crate::is_loopback_bind(h)
                    )
            })
            .map(|s| (s.state == SState::Up, s.last_error.clone()))
            .collect()
    };
    if distants.is_empty() && !net_enabled {
        *lockv(&state.lan_ok) = None;
        return;
    }
    let gw = probe::lan_reachable();
    let distants_ko = !distants.is_empty()
        && distants.iter().all(|(up, err)| {
            !up && err
                .as_deref()
                .map(|e| {
                    let e = e.to_lowercase();
                    e.contains("ssh")
                        || e.contains("connect")
                        || e.contains("timed out")
                        || e.contains("injoignable")
                        || e.contains("unreachable")
                })
                .unwrap_or(false)
        });
    let reachable = lan_decision(gw, distants_ko);
    let mut slot = lockv(&state.lan_ok);
    let changed = *slot != reachable;
    *slot = reachable;
    drop(slot);
    if changed {
        match reachable {
            Some(false) => add_log(
                state,
                "warn",
                "réseau local injoignable (passerelle muette et serveurs distants en échec réseau) — \
                 VPN actif ou connexion coupée : les serveurs distants ne peuvent pas être sondés",
            ),
            Some(true) => add_log(state, "info", "réseau local joignable"),
            None => {}
        }
    }
}

pub fn state_json(state: &Arc<AppState>) -> Value {
    let managed = managed_ids(state);
    // chaque verrou est pris et relâché isolément pour éviter de maintenir
    // 5 mutex simultanément (réduit la contention et tout risque d'inversion d'ordre).
    let (order, autostart, settings_val) = {
        let settings = lockv(&state.settings);
        (
            settings.server_order.clone(),
            settings.autostart,
            json!({
                "refresh_secs": settings.refresh_secs,
                "ui_port": settings.ui_port,
                "ui_bind": settings.ui_bind,
                "scan_from": settings.scan_from,
                "scan_to": settings.scan_to,
                "network_enabled": settings.network_enabled,
                "network_ports": settings.network_ports,
                "discovery_ports_only": settings.discovery_ports_only,
                "discovery_ports_excluded": settings.discovery_ports_excluded,
                "ssh_hosts": settings.ssh_hosts.iter().map(|h| json!({
                    "ip": h.ip, "user": h.user
                })).collect::<Vec<_>>(),
                // config des notifications (toast/webhook) : l'UI locale en a
                // besoin pour son panneau Réglages → Notifications. JAMAIS
                // exposée au gateway MCP, et les en-têtes ne sortent JAMAIS
                // ici (ils peuvent porter un Authorization) : seul un booléen
                // `headers_set` est rendu (audit 05 #5 : /api/state était
                // lisible sans code en loopback).
                "notify": crate::notify::to_api_json(&settings.notify),
                "tls": {
                    "compiled": crate::auth::net::tls_compiled(),
                    "active": !settings.auth.tls_fp.is_empty(),
                    "fingerprint": settings.auth.tls_fp,
                    "bind_refused": settings.auth.tls_bind_refused,
                    "notice": settings.auth.tls_notice,
                },
            }),
        )
    };
    let (list, up, total, tools) = {
        let servers = lockv(&state.servers);
        // affichage : ordre préféré (glisser-déposer), non listés en dernier (stable)
        let mut list: Vec<(usize, Value)> = servers
            .iter()
            .map(|s| {
                let pos = order.iter().position(|x| x == &s.id).unwrap_or(usize::MAX);
                (pos, server_json(s, managed.contains(&s.id)))
            })
            .collect();
        list.sort_by_key(|(pos, _)| *pos);
        let list: Vec<Value> = list.into_iter().map(|(_, v)| v).collect();
        let up = servers.iter().filter(|s| s.state == SState::Up).count();
        let total = servers.len();
        // ne compte que les tools des serveurs effectivement UP (les tools mis en
        // cache d'un serveur DOWN/PORT-ONLY restent visibles dans le panneau détail
        // mais ne gonflent pas le compteur d'outils disponibles).
        let tools: usize = servers
            .iter()
            .filter(|s| s.state == SState::Up)
            .map(tools_actifs)
            .sum();
        (list, up, total, tools)
    };
    let logarr: Vec<Value> = {
        let log = lockv(&state.log);
        log.iter()
            .rev()
            .take(80)
            .map(|e| {
                json!({
                    "ts": e.ts, "level": e.level, "msg": e.msg
                })
            })
            .collect()
    };
    let peers_arr: Vec<Value> = {
        let peers = lockv(&state.peers);
        peers
            .values()
            .map(|p| {
                json!({
                    "ip": p.ip, "ui_port": p.ui_port, "last_seen": p.last_seen,
                    "stats": p.servers.get("stats"),
                    "servers": p.servers.get("servers"),
                })
            })
            .collect()
    };
    let latest = lockv(&state.update_latest).clone();
    let lan = *lockv(&state.lan_ok);
    // Écran « Accès » (lot M3) : compteurs SEULEMENT — le bandeau de l'UI a
    // besoin de savoir qu'une demande attend, sans transporter de texte venu
    // d'un client par ce canal (le détail vit dans `access_list`, action
    // locale). Aucun verrou de réglages n'est tenu ici (pair::list le prend).
    let access = crate::auth::confirm::summary(state);
    // Annonces LiteMCP non suivies : lecture seule (aucune adoption ni écriture).
    let announcements_count = litemcp_announcements(state)["announcements"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|a| a.get("followed").and_then(Value::as_bool) == Some(false))
                .count()
        })
        .unwrap_or(0);
    // Compteur seul : jamais de jeton, cookie ni secret de session ici.
    let sessions_active = crate::auth::session::active_count();
    // Identité publique Ed25519 de MCP Control (clé publique seule, jamais privée)
    // pour l'intégration rapide sur les sous-serveurs depuis le menu clic droit.
    let server_identity = {
        let st = lockv(&state.settings);
        st.auth.server.as_ref().map(|ident| {
            json!({
                "keyid": ident.keyid,
                "public_key": ident.public_key,
                "fingerprint": crate::auth::registry::fingerprint_of(&ident.public_key).unwrap_or_default(),
                "fingerprint_court": crate::auth::registry::short_fingerprint_of(&ident.public_key).unwrap_or_default(),
            })
        })
    };
    json!({
        "servers": list, "log": logarr,
        "latest_version": latest,
        "update_available": latest.is_some(),
        "autostart": autostart,
        // None = pas encore mesuré ; false = réseau local coupé (VPN le plus souvent)
        "lan_ok": lan,
        "stats": {"up": up, "total": total, "tools": tools},
        "settings": settings_val,
        "peers": peers_arr,
        "access": access,
        "announcements_count": announcements_count,
        "sessions_active": sessions_active,
        "server_identity": server_identity,
    })
}

pub fn detail_json(state: &Arc<AppState>, id: &str) -> Value {
    let managed = managed_ids(state);
    let servers = lockv(&state.servers);
    match servers.iter().find(|s| s.id == id) {
        Some(s) => {
            let mut o = match server_json(s, managed.contains(&s.id)) {
                Value::Object(o) => o,
                _ => Map::new(),
            };
            let tools: Vec<Value> = s
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name, "desc": t.desc,
                        "input_schema": t.input_schema,
                        "disabled": s.tools_disabled.contains(&t.name),
                    })
                })
                .collect();
            o.insert("tools_list".into(), json!(tools));
            Value::Object(o)
        }
        None => json!({"ok": false, "error": "serveur inconnu"}),
    }
}

/// Variante légère de `state_json` pour `gateway::list_servers` (P1 rang 5) :
/// évite de sérialiser `log`, `peers`, `settings`, `access` et de parcourir le
/// disque (`litemcp_announcements`), filtre avant sérialisation et ne clone pas
/// le tableau `servers`.
pub fn servers_summary_json(state: &Arc<AppState>, state_filter: Option<&str>) -> Value {
    let managed = managed_ids(state);
    let order = lockv(&state.settings).server_order.clone();
    let (list, up, total, tools) = {
        let servers = lockv(&state.servers);
        let up = servers.iter().filter(|s| s.state == SState::Up).count();
        let total = servers.len();
        let tools: usize = servers
            .iter()
            .filter(|s| s.state == SState::Up)
            .map(tools_actifs)
            .sum();
        let mut ordered: Vec<(usize, &Server)> = servers
            .iter()
            .filter(|s| match state_filter {
                Some(want) => s.state.as_str() == want,
                None => true,
            })
            .map(|s| {
                let pos = order.iter().position(|x| x == &s.id).unwrap_or(usize::MAX);
                (pos, s)
            })
            .collect();
        ordered.sort_by_key(|(pos, _)| *pos);
        let list: Vec<Value> = ordered
            .into_iter()
            .map(|(_, s)| server_json(s, managed.contains(&s.id)))
            .collect();
        (list, up, total, tools)
    };
    let lan = *lockv(&state.lan_ok);
    json!({
        "stats": {"up": up, "total": total, "tools": tools},
        "lan_ok": lan,
        "servers": list,
    })
}

/// Hash d'un serveur pour ETag `/api/state` et `/api/detail` (hors champs
/// volatils `latency_ms` et `last_seen_ts` qui changent à chaque sonde).
fn hash_server_visible<H: std::hash::Hasher>(
    s: &Server,
    managed: bool,
    include_tools_list: bool,
    h: &mut H,
) {
    use std::hash::Hash;
    s.id.hash(h);
    s.label.hash(h);
    match &s.transport {
        Transport::Http { host, port, path } => {
            0u8.hash(h);
            host.hash(h);
            port.hash(h);
            path.hash(h);
        }
        Transport::Stdio {
            program,
            args,
            workdir,
            env,
        } => {
            1u8.hash(h);
            program.hash(h);
            args.hash(h);
            workdir.hash(h);
            env.len().hash(h);
            for (k, _) in env {
                k.hash(h);
            }
        }
    }
    s.origin.as_str().hash(h);
    s.enabled.hash(h);
    s.state.as_str().hash(h);
    s.pid.hash(h);
    s.bound_host.hash(h);
    s.server_name.hash(h);
    s.version.hash(h);
    s.protocol.hash(h);
    tools_actifs(s).hash(h);
    s.tools.len().hash(h);
    s.tools_disabled.hash(h);
    s.last_error.hash(h);
    s.auto_restart.hash(h);
    s.favorite.hash(h);
    s.sse.hash(h);
    s.sse_endpoint.hash(h);
    s.ws.hash(h);
    s.lang.hash(h);
    s.framework.hash(h);
    let auth = crate::auth::net::server_auth_status(s);
    auth.locked.hash(h);
    auth.paired.hash(h);
    auth.keyid.hash(h);
    auth.public_key.hash(h);
    auth.fingerprint.hash(h);
    s.via_ssh.hash(h);
    managed.hash(h);
    match &s.start_cmd {
        Some(c) => {
            true.hash(h);
            c.program.hash(h);
            c.args.hash(h);
            c.workdir.hash(h);
            c.env.len().hash(h);
            for (k, _) in &c.env {
                k.hash(h);
            }
        }
        None => false.hash(h),
    }
    if include_tools_list {
        for t in &s.tools {
            t.name.hash(h);
            t.desc.hash(h);
            if let Some(ref sc) = t.input_schema {
                sc.to_string().hash(h);
            }
        }
    }
}

/// Empreinte stable (ETag) de `/api/state` hors horodatages volatils
/// (`latency_ms`, `last_seen_ts`, `peers[].last_seen`). Permet de répondre
/// `304 Not Modified` sans sérialiser le JSON quand rien de visible n'a changé
/// (P1 rang 3).
pub fn state_etag(state: &Arc<AppState>) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let managed = managed_ids(state);
    let order = {
        let settings = lockv(&state.settings);
        settings.refresh_secs.hash(&mut h);
        settings.ui_port.hash(&mut h);
        settings.ui_bind.hash(&mut h);
        settings.scan_from.hash(&mut h);
        settings.scan_to.hash(&mut h);
        settings.network_enabled.hash(&mut h);
        settings.network_ports.hash(&mut h);
        settings.discovery_ports_only.hash(&mut h);
        settings.discovery_ports_excluded.hash(&mut h);
        settings.ssh_hosts.len().hash(&mut h);
        for sh in &settings.ssh_hosts {
            sh.ip.hash(&mut h);
            sh.user.hash(&mut h);
        }
        settings.server_order.hash(&mut h);
        settings.autostart.hash(&mut h);
        settings.notify.desktop.hash(&mut h);
        settings.notify.webhook_url.hash(&mut h);
        if let Some(ref ident) = settings.auth.server {
            ident.keyid.hash(&mut h);
            ident.public_key.hash(&mut h);
        }
        settings.notify.method.hash(&mut h);
        settings.notify.min_interval_secs.hash(&mut h);
        settings.notify.states.hash(&mut h);
        settings.notify.headers.trim().is_empty().hash(&mut h);
        settings.auth.tls_fp.hash(&mut h);
        settings.auth.tls_bind_refused.hash(&mut h);
        settings.auth.tls_notice.hash(&mut h);
        settings.server_order.clone()
    };
    {
        let servers = lockv(&state.servers);
        servers.len().hash(&mut h);
        let mut idxs: Vec<(usize, usize)> = servers
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let pos = order.iter().position(|x| x == &s.id).unwrap_or(usize::MAX);
                (pos, i)
            })
            .collect();
        idxs.sort_by_key(|(pos, _)| *pos);
        for (_, i) in idxs {
            let s = &servers[i];
            hash_server_visible(s, managed.contains(&s.id), false, &mut h);
        }
    }
    {
        let log = lockv(&state.log);
        log.len().hash(&mut h);
        if let Some(last) = log.back() {
            last.ts.hash(&mut h);
            last.level.hash(&mut h);
            last.msg.hash(&mut h);
        }
        if let Some(first) = log.front() {
            first.ts.hash(&mut h);
            first.msg.hash(&mut h);
        }
    }
    {
        let peers = lockv(&state.peers);
        let mut keys: Vec<&String> = peers.keys().collect();
        keys.sort();
        keys.len().hash(&mut h);
        for k in keys {
            if let Some(p) = peers.get(k) {
                p.ip.hash(&mut h);
                p.ui_port.hash(&mut h);
                if let Some(st) = p.servers.get("stats") {
                    st.to_string().hash(&mut h);
                }
                if let Some(arr) = p.servers.get("servers").and_then(Value::as_array) {
                    arr.len().hash(&mut h);
                }
            }
        }
    }
    lockv(&state.update_latest).hash(&mut h);
    (*lockv(&state.lan_ok)).hash(&mut h);
    let access = crate::auth::confirm::summary(state);
    access.to_string().hash(&mut h);
    let announcements_count = litemcp_announcements(state)["announcements"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|a| a.get("followed").and_then(Value::as_bool) == Some(false))
                .count()
        })
        .unwrap_or(0);
    announcements_count.hash(&mut h);
    crate::auth::session::active_count().hash(&mut h);
    format!("\"s-{:016x}\"", h.finish())
}

/// Empreinte stable (ETag) de `/api/detail?id=...` hors horodatages volatils.
pub fn detail_etag(state: &Arc<AppState>, id: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let managed = managed_ids(state);
    let servers = lockv(&state.servers);
    match servers.iter().find(|s| s.id == id) {
        Some(s) => {
            true.hash(&mut h);
            hash_server_visible(s, managed.contains(&s.id), true, &mut h);
        }
        None => {
            false.hash(&mut h);
            id.hash(&mut h);
        }
    }
    format!("\"d-{:016x}\"", h.finish())
}
