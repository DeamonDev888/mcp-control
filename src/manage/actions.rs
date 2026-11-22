// Dispatcher d'actions API (POST /api/action) + surface ACTIONS partagée avec
// les contrats de tests. Les familles pourront être extraites en sous-fonctions
// par thème (serveur, config, boutique, updates) — move pur ici.
use super::*;

// Surface du dispatcher : lue par les contrats de tests (ui.rs, manage/tests.rs)
// uniquement, d'où l'allow ciblé hors tests.
#[cfg_attr(not(test), allow(dead_code))]
pub const ACTIONS: &[&str] = &[
    "toggle",
    "refresh",
    "kill_stdio",
    "start_stdio",
    "start",
    "kill",
    "scan",
    "scan_network",
    "discover",
    "discover_peers",
    "import_peer",
    "add",
    "remove",
    // Lot S4 : arrêt explicite d'un process qui n'a PAS été lancé par l'app
    // (le port est tenu par un tiers) — geste distinct, après refus motivé de
    // `kill`. Jamais exposé au gateway.
    "stop_server_force",
    // Lot S4 : annonces litemcp du dossier de l'utilisateur courant, en
    // LECTURE SEULE (« détecté, non suivi »). Jamais exposée au gateway.
    "litemcp_announcements",
    "toggle_tool",
    "set_start_cmd",
    "set_host",
    "set_auto_restart",
    "set_favorite",
    "reorder",
    "import_mcp",
    "install_store",
    "ssh_save",
    "ssh_forget",
    "ssh_discover",
    "set_settings",
    "set_notify",
    "set_autostart",
    "reset_state",
    "reset_runtime",
    "check_update",
    "open_release_page",
    "download_update",
    "install_update",
    "open_path",
    "test_notify",
    // Lot M1 — actions LOCALES d'authentification (droit `admin`). Jamais
    // exposées au gateway : ni la liste des tools, ni un champ `action`
    // injecté ne peut les atteindre (test `injections_interdites`).
    "pair_list",
    "pair_approve",
    "pair_reject",
    "client_list",
    "client_set_scopes",
    "client_revoke",
    "client_rotate",
    // Lot M3 — écran « Accès » et décisions de confirmation (droit `admin`).
    "access_list",
    "confirm_approve",
    "confirm_deny",
    "grant_revoke",
    "call_tool",
];

/// Port HTTP LOCAL d'un serveur (None : stdio, ou serveur distant). Un hôte
/// explicite de loopback (127.0.0.1, ::1, localhost) reste LOCAL : les serveurs
/// découverts et la plupart des serveurs ajoutés portent cet hôte.
fn local_port_of(state: &Arc<AppState>, id: &str) -> Option<u16> {
    let servers = lockv(&state.servers);
    servers
        .iter()
        .find(|s| s.id == id)
        .and_then(|s| match &s.transport {
            Transport::Http { host, port, .. }
                if host.as_deref().map(crate::is_loopback_bind).unwrap_or(true) =>
            {
                Some(*port)
            }
            _ => None,
        })
}

/// Process lancé par l'APPLICATION pour ce serveur : `Some((pid, vivant))`.
/// `vivant` = handle encore ouvert (le watcher n'a pas récolté la sortie) ET
/// date de création identique à celle enregistrée au spawn — un PID recyclé
/// par l'OS n'est donc jamais pris pour le nôtre. `None` = l'app n'a rien lancé.
fn processus_lance(state: &Arc<AppState>, id: &str) -> Option<(u32, bool)> {
    let children = lockv(&state.children);
    let h = children.get(id)?;
    let en_cours = lockv(&h.exit).is_none();
    if !en_cours {
        return Some((h.pid, false));
    }
    let identique = h
        .started
        .map(|t| probe::process_creation(h.pid) == Some(t))
        .unwrap_or(true);
    Some((h.pid, identique))
}

/// Process qui écoutent sur `port` sans être celui lancé par l'application.
fn etrangers_sur_port(port: u16, pid_lance: u32, vivant: bool) -> Vec<(u32, String)> {
    probe::port_listeners(port)
        .into_iter()
        .filter(|(pid, _)| !(vivant && *pid == pid_lance))
        .collect()
}

/// « node.exe (pid 5508), python.exe (pid 4242) » — pour un refus lisible.
fn decrire_processus(v: &[(u32, String)]) -> String {
    v.iter()
        .map(|(pid, img)| format!("{img} (pid {pid})"))
        .collect::<Vec<_>>()
        .join(", ")
}
/// Asset de la release correspondant à (type de binaire, OS) — table blanche
/// pure et testée. `None` = plateforme inconnue (jamais d'URL fabriquée hors
/// de cette table). Les assets n'embarquent pas la version : l'URL
/// `releases/latest/download/<asset>` sert toujours LA dernière release.
pub fn handle_action(state: &Arc<AppState>, body: &str) -> Value {
    let req: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return json!({"ok": false, "error": format!("body invalide: {}", e)}),
    };
    let action = req
        .get("action")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let id = req.get("id").and_then(|x| x.as_str()).map(String::from);

    match action.as_str() {
        "toggle" | "refresh" | "kill_stdio" | "start_stdio" | "start" | "kill" => {
            let id = match id {
                Some(i) => i,
                None => return json!({"ok": false, "error": "id manquant"}),
            };
            match action.as_str() {
                "toggle" => {
                    let now_enabled = {
                        let mut servers = lockv(&state.servers);
                        match servers.iter_mut().find(|s| s.id == id) {
                            Some(s) => {
                                s.enabled = !s.enabled;
                                s.state = if s.enabled {
                                    SState::Down
                                } else {
                                    SState::Disabled
                                };
                                if s.enabled {
                                    s.start_cmd = start_cmd_for(s);
                                    s.auto_restart = true;
                                } else {
                                    s.auto_restart = false;
                                }
                                s.enabled
                            }
                            None => return json!({"ok": false, "error": "serveur inconnu"}),
                        }
                    };
                    if let Err(e) = save_config_checked(state) {
                        return json!({"ok": false, "error": format!("config non écrite : {e}")});
                    }
                    if !now_enabled {
                        // OFF réel : tuer le SEUL process lancé par MCP Control
                        // (identité vérifiée — jamais un process tiers qui
                        // aurait pris le port).
                        if let Some((pid, true)) = processus_lance(state, &id) {
                            probe::kill_tree(pid);
                        }
                        lockv(&state.children).remove(&id);
                        let mut servers = lockv(&state.servers);
                        if let Some(s) = servers.iter_mut().find(|s| s.id == id) {
                            s.pid = None;
                        }
                    }
                    add_log(
                        state,
                        "info",
                        &format!(
                            "[{}] {}",
                            id,
                            if now_enabled {
                                "activé"
                            } else {
                                "désactivé"
                            }
                        ),
                    );
                    if now_enabled {
                        // ON réel : démarrer/sonder immédiatement si une commande de lancement existe
                        poll_one(state, &id);
                    }
                    json!({"ok": true, "enabled": now_enabled})
                }
                "refresh" => {
                    poll_one(state, &id);
                    json!({"ok": true})
                }
                "start_stdio" | "start" => {
                    {
                        let mut servers = lockv(&state.servers);
                        match servers.iter_mut().find(|s| s.id == id) {
                            Some(s) => {
                                // Start explicite : réactive le serveur même désactivé
                                // (sinon poll_one sort sur !enabled et le bouton ne fait rien)
                                if !s.enabled {
                                    s.enabled = true;
                                    s.state = SState::Down;
                                }
                                if s.start_cmd.is_none() {
                                    s.start_cmd = start_cmd_for(s);
                                }
                                s.auto_restart = true;
                            }
                            None => return json!({"ok": false, "error": "serveur inconnu"}),
                        }
                    }
                    if let Err(e) = save_config_checked(state) {
                        return json!({"ok": false, "error": format!("config non écrite : {e}")});
                    }
                    poll_one(state, &id);
                    json!({"ok": true})
                }
                "kill_stdio" | "kill" => {
                    // Lot F point 5 : id inconnu = refus EXPLICITE.
                    if !lockv(&state.servers).iter().any(|s| s.id == id) {
                        return json!({"ok": false, "error": "serveur inconnu"});
                    }
                    let local_port = local_port_of(state, &id);
                    // AUDIT S4, point 1 : on ne tue QUE le process lancé par
                    // l'application (pid enregistré au spawn, vérifié vivant ET
                    // identique par sa date de création). Un process tiers qui
                    // tient le port n'est jamais tué : refus motivé, avec le nom,
                    // et action distincte `stop_server_force`.
                    let (pid_lance, vivant) = processus_lance(state, &id).unwrap_or((0, false));
                    if let Some(port) = local_port {
                        let etrangers = etrangers_sur_port(port, pid_lance, vivant);
                        if !etrangers.is_empty() {
                            let desc = decrire_processus(&etrangers);
                            add_log(
                                state,
                                "warn",
                                &format!(
                                    "[{id}] arrêt refusé : port {port} tenu par {desc} — ce process n'a pas été lancé par l'application (action « arrêter quand même » pour forcer)"
                                ),
                            );
                            return json!({
                                "ok": false,
                                "error": format!(
                                    "port {port} tenu par {desc} — process non lancé par l'application ; utilisez stop_server_force pour l'arrêter quand même"
                                ),
                                "needs_force": true,
                                "occupants": etrangers
                                    .iter()
                                    .map(|(p, i)| json!({"pid": p, "process": i}))
                                    .collect::<Vec<_>>(),
                            });
                        }
                    }
                    // Arrêt du process de l'app (arbre compris : lanceur → serveur).
                    if vivant {
                        probe::kill_tree(pid_lance);
                    }
                    lockv(&state.children).remove(&id);
                    // Attente bornée de la libération du port (le serveur réel met
                    // 1-3 s à sortir) ; s'il reste tenu par un AUTRE process, on
                    // le signale sans le tuer.
                    let mut restants: Vec<(u32, String)> = Vec::new();
                    if let Some(port) = local_port {
                        for _ in 0..10 {
                            restants = probe::port_listeners(port);
                            if restants.is_empty() {
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(500));
                        }
                    }
                    {
                        let mut servers = lockv(&state.servers);
                        if let Some(s) = servers.iter_mut().find(|s| s.id == id) {
                            s.auto_restart = false;
                            s.pid = None;
                            s.state = SState::Down;
                        }
                    }
                    if !restants.is_empty() {
                        let desc = decrire_processus(&restants);
                        add_log(
                            state,
                            "warn",
                            &format!("[{id}] port toujours tenu par {desc} — process non lancé par l'application, non tué"),
                        );
                        return json!({
                            "ok": false,
                            "error": format!(
                                "le process de l'application est arrêté, mais le port reste tenu par {desc} (non tué) ; utilisez stop_server_force pour l'arrêter quand même"
                            ),
                            "needs_force": true,
                            "occupants": restants
                                .iter()
                                .map(|(p, i)| json!({"pid": p, "process": i}))
                                .collect::<Vec<_>>(),
                        });
                    }
                    add_log(state, "info", &format!("[{id}] arrêté"));
                    json!({"ok": true})
                }
                _ => unreachable!(),
            }
        }
        // Lot S4, point 1 : « arrêter quand même » — SEUL chemin qui tue un
        // process que l'application n'a PAS lancé. Geste explicite, consécutif
        // au refus motivé de `kill` ; jamais exposé au gateway.
        "stop_server_force" => {
            let Some(id) = id else {
                return json!({"ok": false, "error": "id manquant"});
            };
            if !lockv(&state.servers).iter().any(|s| s.id == id) {
                return json!({"ok": false, "error": "serveur inconnu"});
            }
            let Some(port) = local_port_of(state, &id) else {
                return json!({
                    "ok": false,
                    "error": "serveur sans port local : aucun process à arrêter"
                });
            };
            let occupants = probe::port_listeners(port);
            let pids: Vec<u32> = occupants.iter().map(|(p, _)| *p).collect();
            if !pids.is_empty() {
                probe::kill_pids(&pids);
            }
            // le handle éventuel de l'app ne correspond plus au process du port
            lockv(&state.children).remove(&id);
            {
                let mut servers = lockv(&state.servers);
                if let Some(s) = servers.iter_mut().find(|s| s.id == id) {
                    s.auto_restart = false;
                    s.pid = None;
                    s.state = SState::Down;
                }
            }
            add_log(
                state,
                "warn",
                &format!(
                    "[{id}] arrêt FORCÉ : port {port} — {} process tué(s) : {}",
                    pids.len(),
                    decrire_processus(&occupants)
                ),
            );
            json!({
                "ok": true,
                "forced": true,
                "port": port,
                "killed": occupants
                    .iter()
                    .map(|(p, i)| json!({"pid": p, "process": i}))
                    .collect::<Vec<_>>(),
            })
        }
        // Lot S4, point 2 : annonces litemcp — LECTURE SEULE. Le dossier
        // `%TEMP%\litemcp-registry` n'est jamais écrit, aucun champ d'annonce
        // n'est exécuté, aucun serveur n'est créé ni adopté.
        "litemcp_announcements" => {
            let mut v = litemcp_announcements(state);
            if let Some(o) = v.as_object_mut() {
                o.insert("ok".into(), json!(true));
            }
            v
        }
        "add" => {
            let transport = req
                .get("transport")
                .and_then(|x| x.as_str())
                .unwrap_or("http");
            let srv = if transport == "stdio" {
                let program = req.get("program").and_then(|x| x.as_str()).unwrap_or("");
                if program.is_empty() {
                    return json!({"ok": false, "error": "program manquant"});
                }
                let sargs: Vec<String> = req
                    .get("args")
                    .and_then(|x| x.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|i| i.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let sworkdir = req
                    .get("workdir")
                    .and_then(|x| x.as_str())
                    .map(String::from);
                // env additionnel optionnel {"CLÉ": "valeur"} (lot 3, audit 03
                // constat 5) : atteint le process lancé via StartCmd.env —
                // jamais l'API (noms seuls via server_json).
                let senv = env_from_json(&req);
                Server {
                    id: format!("manual-stdio-{}", now_compact()),
                    label: req
                        .get("label")
                        .and_then(|x| x.as_str())
                        .unwrap_or(program)
                        .to_string(),
                    transport: Transport::Stdio {
                        program: program.to_string(),
                        args: sargs.clone(),
                        workdir: sworkdir.clone(),
                        env: senv.clone(),
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
                    via_ssh: false,
                    tools: vec![],
                    latency_ms: None,
                    last_seen_ts: None,
                    last_error: None,
                    start_cmd: Some(StartCmd {
                        program: program.to_string(),
                        args: sargs,
                        workdir: sworkdir,
                        env: senv,
                    }),
                    auto_restart: true,
                    favorite: false,
                    tools_disabled: vec![],
                }
            } else {
                let port = match req.get("port").and_then(|x| x.as_u64()) {
                    Some(p) if (1..=65535).contains(&p) => p as u16,
                    _ => return json!({"ok": false, "error": "port invalide"}),
                };
                // commande de lancement optionnelle → MCP Control peut Start/Stop ce HTTP
                let sstart = {
                    let program = req
                        .get("program")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    if program.is_empty() {
                        None
                    } else {
                        Some(StartCmd {
                            program,
                            args: req
                                .get("args")
                                .and_then(|x| x.as_array())
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|i| i.as_str().map(String::from))
                                        .collect()
                                })
                                .unwrap_or_default(),
                            workdir: req
                                .get("workdir")
                                .and_then(|x| x.as_str())
                                .map(String::from),
                            env: env_from_json(&req),
                        })
                    }
                };
                Server {
                    id: format!("manual-http-{}", now_compact()),
                    label: req
                        .get("label")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    transport: Transport::Http {
                        host: req.get("host").and_then(|x| x.as_str()).map(String::from),
                        port,
                        path: req
                            .get("path")
                            .and_then(|x| x.as_str())
                            .unwrap_or("/mcp")
                            .to_string(),
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
                    via_ssh: false,
                    tools: vec![],
                    latency_ms: None,
                    last_seen_ts: None,
                    last_error: None,
                    start_cmd: sstart,
                    auto_restart: true,
                    favorite: false,
                    tools_disabled: vec![],
                }
            };
            let label = srv.label.clone();
            let sid = srv.id.clone();
            lockv(&state.servers).push(srv);
            if let Err(e) = save_config_checked(state) {
                return json!({"ok": false, "error": format!("config non écrite : {e}")});
            }
            add_log(
                state,
                "info",
                &format!("[+] serveur ajouté : {} ({})", label, sid),
            );
            json!({"ok": true, "id": sid})
        }
        "remove" => {
            let id = match id {
                Some(i) => i,
                None => return json!({"ok": false, "error": "id manquant"}),
            };
            // durable = « Retirer durablement » sur un seed : l'id est consigné
            // dans settings.removed_seeds pour ne pas être réinjecté au boot
            let durable = req
                .get("durable")
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
            // (lot 3, audit 03 constat 9 / lot S4 point 1) un serveur lancé par
            // l'app via un lanceur qui détache son enfant (bin/launch.cjs)
            // survit au kill_tree. On arrête le process que l'app a lancé
            // (identité vérifiée) ; si le port reste tenu par un AUTRE process,
            // il n'est PAS tué — supprimer une entrée ne doit jamais tuer un
            // tiers, même pour libérer le port (action explicite
            // stop_server_force).
            let (pid_lance, vivant) = processus_lance(state, &id).unwrap_or((0, false));
            let local_port = local_port_of(state, &id);
            if vivant {
                probe::kill_tree(pid_lance);
            }
            lockv(&state.children).remove(&id);
            if let Some(p) = local_port {
                for _ in 0..4 {
                    if probe::port_listeners(p).is_empty() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                let restants = probe::port_listeners(p);
                if !restants.is_empty() {
                    add_log(
                        state,
                        "warn",
                        &format!(
                            "[{}] port {} toujours tenu par {} après suppression — process non lancé par l'application, non tué",
                            id,
                            p,
                            decrire_processus(&restants)
                        ),
                    );
                }
            }
            let removed = {
                let mut servers = lockv(&state.servers);
                let before = servers.len();
                servers.retain(|s| s.id != id);
                before != servers.len()
            };
            if durable && removed {
                let mut st = lockv(&state.settings);
                if !st.removed_seeds.iter().any(|x| x == &id) {
                    st.removed_seeds.push(id.clone());
                }
            }
            if removed {
                lockv(&state.settings).server_order.retain(|x| x != &id);
            }
            if let Err(e) = save_config_checked(state) {
                return json!({"ok": false, "error": format!("config non écrite : {e}")});
            }
            add_log(
                state,
                "info",
                &format!(
                    "[-] serveur supprimé : {} ({}){}",
                    id,
                    if removed { "ok" } else { "introuvable" },
                    if durable && removed {
                        " — retiré durablement (non réinjecté au boot)"
                    } else {
                        ""
                    }
                ),
            );
            if !removed {
                // Lot F point 5 : l'interface ne peut afficher qu'un « échec »
                // générique sur un {ok:false} sans `error` — motif explicite.
                return json!({"ok": false, "error": "serveur inconnu"});
            }
            json!({"ok": true})
        }
        "scan" => {
            let range = match (
                req.get("from").and_then(|x| x.as_u64()),
                req.get("to").and_then(|x| x.as_u64()),
            ) {
                (Some(a), Some(b))
                    if (1..=65535).contains(&a) && (1..=65535).contains(&b) && a <= b =>
                {
                    Some((a as u16, b as u16))
                }
                _ => None,
            };
            // (lot 3, audit 03 constat 10) plage plafonnée : au-delà du plafond,
            // refus CLAIR (le scan synchrone figeait le handler des heures sur
            // une plage large). scan_ports tronque défensivement les plages
            // issues des réglages (scan_from/scan_to hors plafond).
            if let Some((a, b)) = range {
                if (b - a + 1) as usize > SCAN_MAX_PORTS {
                    return json!({
                        "ok": false,
                        "error": format!(
                            "plage trop large : {} ports demandés (max {}) — resserrez from..to",
                            b - a + 1,
                            SCAN_MAX_PORTS
                        )
                    });
                }
            }
            let found = scan_ports(state, range);
            json!({"ok": true, "found": found})
        }
        "scan_network" => {
            let range = match (
                req.get("from").and_then(|x| x.as_u64()),
                req.get("to").and_then(|x| x.as_u64()),
            ) {
                (Some(a), Some(b))
                    if (1..=65535).contains(&a) && (1..=65535).contains(&b) && a <= b =>
                {
                    Some((a as u16, b as u16))
                }
                _ => None,
            };
            let st = state.clone();
            std::thread::spawn(move || scan_network(&st, range));
            json!({"ok": true, "started": true, "note": "scan asynchrone — voir le journal pour les détections"})
        }
        "discover_peers" => {
            discover_peers(state);
            json!({"ok": true})
        }
        // découverte locale à la demande (bouton ⟳ de l'UI) : tous ports en écoute
        "discover" => {
            scan_listening(state);
            json!({"ok": true})
        }
        "ssh_save" => {
            let (ip, user) = match (
                req.get("ip").and_then(|x| x.as_str()),
                req.get("user").and_then(|x| x.as_str()),
            ) {
                (Some(i), Some(u)) if !i.trim().is_empty() && !u.trim().is_empty() => {
                    (i.trim().to_string(), u.trim().to_string())
                }
                _ => return json!({"ok": false, "error": "ip et user requis"}),
            };
            let mut st = lockv(&state.settings);
            if let Some(h) = st.ssh_hosts.iter_mut().find(|h| h.ip == ip) {
                h.user = user;
            } else {
                st.ssh_hosts.push(crate::types::SshHost { ip, user });
            }
            drop(st);
            if let Err(e) = save_config_checked(state) {
                return json!({"ok": false, "error": format!("config non écrite : {e}")});
            }
            add_log(state, "info", "[machines] machine enregistrée");
            json!({"ok": true})
        }
        "set_start_cmd" => {
            // Un serveur local sans commande de lancement (découvert par scan de
            // ports) n'a ni Start/Stop ni autoboot : l'utilisateur peut poser la
            // commande à la main ici. Seeds refusés (la config par défaut
            // réécraserait) et serveurs distants refusés (le process appartient
            // à sa machine hôte, pas au moteur local).
            // `program` null/absent/chaîne vide = RETRAIT de la commande (lot A :
            // le schéma gateway est nullable depuis le lot 1 — le dispatcher
            // répondait « program manquant » au lieu de retirer) ; l'autoboot
            // est désactivé puisque le serveur n'a plus rien à lancer.
            let id = match id {
                Some(i) => i,
                None => return json!({"ok": false, "error": "id manquant"}),
            };
            let program = req
                .get("program")
                .and_then(|x| x.as_str())
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty());
            let args: Vec<String> = req
                .get("args")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|i| i.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let workdir = req
                .get("workdir")
                .and_then(|x| x.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let mut servers = lockv(&state.servers);
            let Some(srv) = servers.iter_mut().find(|s| s.id == id) else {
                return json!({"ok": false, "error": "serveur inconnu"});
            };
            if srv.origin == Origin::Seed {
                return json!({"ok": false, "error": "serveur seed : commande non modifiable (retire-le durablement puis réajoute-le)"});
            }
            if matches!(&srv.transport, Transport::Http { host: Some(_), .. }) {
                return json!({"ok": false, "error": "serveur distant : la commande de lancement appartient à sa machine hôte"});
            }
            let Some(program) = program else {
                // RETRAIT : plus de commande → plus d'autoboot (un
                // auto-restart sans commande ne spawnerait rien de vivant).
                // Le process DÉJÀ lancé n'est pas tué : le retrait de la
                // commande ne vaut pas un Stop (bouton dédié).
                srv.start_cmd = None;
                srv.auto_restart = false;
                drop(servers);
                if let Err(e) = save_config_checked(state) {
                    return json!({"ok": false, "error": format!("config non écrite : {e}")});
                }
                add_log(
                    state,
                    "info",
                    &format!("[set_start_cmd] {id} : commande retirée (autoboot désactivé)"),
                );
                return json!({"ok": true, "removed": true});
            };
            // l'env existant est PRÉSERVÉ : set_start_cmd édite programme/args/
            // workdir, pas les variables (les effacer couperait la clé d'API
            // importée — lot 3).
            let env = srv
                .start_cmd
                .as_ref()
                .map(|c| c.env.clone())
                .unwrap_or_default();
            srv.start_cmd = Some(StartCmd {
                program: program.clone(),
                args,
                workdir,
                env,
            });
            // Un serveur découvert (scan) n'est PAS persisté (redétecté au boot).
            // Donner une commande = l'utilisateur le revendique : promotion en
            // manual pour que commande et boutons survivent au redémarrage.
            srv.origin = Origin::Manual;
            drop(servers);
            if let Err(e) = save_config_checked(state) {
                return json!({"ok": false, "error": format!("config non écrite : {e}")});
            }
            add_log(
                state,
                "info",
                &format!("[set_start_cmd] {} : {}", id, program),
            );
            json!({"ok": true})
        }
        "set_host" => {
            // Glisser-déposer d'une ligne sur l'en-tête d'un groupe machine :
            // le host du transport devient la clé du groupe (re-host sans
            // réimporter). Un stdio n'a pas de host (toujours local) — refus.
            let id = match id {
                Some(i) => i,
                None => return json!({"ok": false, "error": "id manquant"}),
            };
            let host_arg = match req.get("host").and_then(|x| x.as_str()) {
                Some(h) if !h.trim().is_empty() => h.trim().to_string(),
                _ => {
                    return json!({"ok": false, "error": "host requis (IP ou hostname de la machine)"});
                }
            };
            let mut servers = lockv(&state.servers);
            let Some(srv) = servers.iter_mut().find(|s| s.id == id) else {
                return json!({"ok": false, "error": "serveur inconnu"});
            };
            if !matches!(srv.transport, Transport::Http { .. }) {
                return json!({"ok": false, "error": "set_host : serveurs stdio non concernés (toujours locaux)"});
            }
            let old = match &mut srv.transport {
                Transport::Http { host, .. } => host.take(),
                _ => unreachable!(),
            };
            if let Transport::Http { host, .. } = &mut srv.transport {
                *host = Some(host_arg.clone());
            }
            let is_seed = srv.origin == Origin::Seed;
            drop(servers);
            // Un serveur seed ne doit PAS être réécrit : au boot suivant la
            // config par défaut réimposerait son host. Refus + restauration.
            if let Err(e) = save_config_checked(state) {
                return json!({"ok": false, "error": format!("config non écrite : {e}")});
            }
            if is_seed {
                let mut servers = lockv(&state.servers);
                if let Some(srv) = servers.iter_mut().find(|s| s.id == id) {
                    if let Transport::Http { host, .. } = &mut srv.transport {
                        *host = old;
                    }
                }
                drop(servers);
                if let Err(e) = save_config_checked(state) {
                    return json!({"ok": false, "error": format!("config non écrite : {e}")});
                }
                add_log(
                    state,
                    "warn",
                    &format!("[set_host] refusé sur seed : {}", id),
                );
                return json!({"ok": false, "error": "serveur seed : host non modifiable (retire-le durablement puis réajoute-le)"});
            }
            add_log(
                state,
                "info",
                &format!(
                    "[set_host] {} : {} -> {}",
                    id,
                    old.as_deref().unwrap_or("local"),
                    host_arg
                ),
            );
            json!({"ok": true, "old_host": old})
        }
        "ssh_forget" => {
            let ip = req
                .get("ip")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let removed = {
                let mut st = lockv(&state.settings);
                let before = st.ssh_hosts.len();
                st.ssh_hosts.retain(|h| h.ip != ip);
                before != st.ssh_hosts.len()
            };
            if let Err(e) = save_config_checked(state) {
                return json!({"ok": false, "error": format!("config non écrite : {e}")});
            }
            json!({"ok": removed})
        }
        "open_path" => {
            // « Ouvrir dans l'explorateur » (menu contextuel de l'explorateur).
            // Chemin local passé en ARGUMENT (jamais via un shell) ; un fichier
            // est sélectionné dans son dossier, un dossier est ouvert.
            let path = req
                .get("path")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if path.is_empty() || path.chars().any(|c| c.is_control()) {
                return json!({"ok": false, "error": "chemin invalide"});
            }
            let pb = std::path::PathBuf::from(&path);
            if !pb.exists() {
                return json!({"ok": false, "error": "chemin introuvable"});
            }
            #[cfg(windows)]
            let mut cmd = {
                let mut c = std::process::Command::new(probe::system_tool("explorer"));
                if pb.is_file() {
                    c.arg(format!("/select,{}", pb.display()));
                } else {
                    c.arg(&pb);
                }
                c
            };
            #[cfg(not(windows))]
            let mut cmd = {
                let mut c = std::process::Command::new(if cfg!(target_os = "macos") {
                    "open"
                } else {
                    "xdg-open"
                });
                if pb.is_file() {
                    c.arg(pb.parent().unwrap_or(&pb));
                } else {
                    c.arg(&pb);
                }
                c
            };
            probe::hidden(&mut cmd);
            match cmd.spawn() {
                Ok(_) => json!({"ok": true}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            }
        }
        "reorder" => {
            // glisser-déposer de l'explorateur : nouvel ordre d'affichage et de
            // priorité de sondage. Ids inconnus filtrés, manquants ajoutés en fin.
            let Some(ids) = req.get("ids").and_then(|x| x.as_array()) else {
                return json!({"ok": false, "error": "ids requis"});
            };
            let mut new_order: Vec<String> = ids
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect();
            let known: Vec<String> = {
                let servers = lockv(&state.servers);
                servers.iter().map(|s| s.id.clone()).collect()
            };
            new_order.retain(|id| known.contains(id));
            for id in &known {
                if !new_order.contains(id) {
                    new_order.push(id.clone());
                }
            }
            lockv(&state.settings).server_order = new_order.clone();
            if let Err(e) = save_config_checked(state) {
                return json!({"ok": false, "error": format!("config non écrite : {e}")});
            }
            add_log(
                state,
                "info",
                &format!(
                    "[ordre] {} serveur(s) réordonné(s) (préférence / priorité de sondage)",
                    new_order.len()
                ),
            );
            json!({"ok": true, "count": new_order.len()})
        }
        "set_settings" => {
            // Réglages principaux du client (dialog ⚙) : sondage, scan, réseau.
            // Validation stricte : whitelist du bind, clamps, ports 1-65535,
            // scan_from <= scan_to. ui_bind est figé au boot (listener) : un
            // changement s'applique au prochain démarrage.
            let mut st = lockv(&state.settings);
            let mut changed: Vec<&str> = Vec::new();
            if let Some(x) = req.get("refresh_secs").and_then(|x| x.as_u64()) {
                let v = x.clamp(5, 3600);
                if st.refresh_secs != v {
                    st.refresh_secs = v;
                    changed.push("refresh");
                }
            }
            if let (Some(f), Some(t)) = (
                req.get("scan_from").and_then(|x| x.as_u64()),
                req.get("scan_to").and_then(|x| x.as_u64()),
            ) {
                let f = f.clamp(1, 65535) as u16;
                let t = t.clamp(1, 65535) as u16;
                let (f, t) = (f.min(t), f.max(t));
                if st.scan_from != f || st.scan_to != t {
                    st.scan_from = f;
                    st.scan_to = t;
                    changed.push("scan");
                }
            }
            if let Some(b) = req.get("ui_bind").and_then(|x| x.as_str()) {
                let b = match b {
                    "0.0.0.0" => "0.0.0.0",
                    _ => "127.0.0.1",
                };
                if st.ui_bind != b {
                    st.ui_bind = b.to_string();
                    changed.push("ui_bind");
                }
            }
            if let Some(en) = req.get("network_enabled").and_then(|x| x.as_bool()) {
                if st.network_enabled != en {
                    st.network_enabled = en;
                    changed.push("network");
                }
            }
            if let Some(ports) = req.get("network_ports").and_then(|x| x.as_array()) {
                let v: Vec<u16> = ports
                    .iter()
                    .filter_map(|p| p.as_u64())
                    .filter(|p| (1..=65535).contains(p))
                    .map(|p| p as u16)
                    .collect();
                if !v.is_empty() && st.network_ports != v {
                    st.network_ports = v;
                    changed.push("network_ports");
                }
            }
            // Découverte locale (lot S4, point 3) : « ne sonder que ces ports »
            // et « exclure ces ports ». Validation identique au chargement
            // (1-65535, doublons retirés, 512 max) ; un tableau VIDE efface le
            // réglage, contrairement à network_ports.
            if let Some(arr) = req.get("discovery_ports_only").and_then(|x| x.as_array()) {
                let v = parse_port_list(arr, 512);
                if st.discovery_ports_only != v {
                    st.discovery_ports_only = v;
                    changed.push("discovery_ports_only");
                }
            }
            if let Some(arr) = req
                .get("discovery_ports_excluded")
                .and_then(|x| x.as_array())
            {
                let v = parse_port_list(arr, 512);
                if st.discovery_ports_excluded != v {
                    st.discovery_ports_excluded = v;
                    changed.push("discovery_ports_excluded");
                }
            }
            drop(st);
            if let Err(e) = save_config_checked(state) {
                return json!({"ok": false, "error": format!("config non écrite : {e}")});
            }
            add_log(
                state,
                "info",
                &format!(
                    "[réglages] {}",
                    if changed.is_empty() {
                        "aucun changement".to_string()
                    } else {
                        changed.join(", ")
                    }
                ),
            );
            json!({"ok": true, "changed": changed})
        }
        "set_notify" => {
            // Configuration des notifications (dialog Réglages) :
            // - desktop : toast Windows
            // - webhook : URL + en-têtes (une par ligne "K: V") + méthode
            //   - états notifiés (DOWN, PORT-ONLY par défaut)
            //   - délai minimum entre deux notifications du même serveur
            let mut c = lockv(&state.settings).notify.clone();
            if let Some(b) = req.get("desktop").and_then(|x| x.as_bool()) {
                c.desktop = b;
            }
            if let Some(u) = req.get("webhook_url").and_then(|x| x.as_str()) {
                let u = u.trim();
                if !u.is_empty() && !crate::notify::valid_webhook_url(u) {
                    // LOT B point 3 : motif PRÉCIS du refus (https:// est refusé
                    // depuis le lot 5 — l'ancien message générique citait encore
                    // https:// comme une forme valide).
                    let motif = crate::notify::webhook_url_reject_reason(u)
                        .unwrap_or_else(|| "URL de webhook invalide".to_string());
                    return json!({"ok": false, "error": motif});
                }
                c.webhook_url = u.to_string();
            }
            if let Some(m) = req.get("method").and_then(|x| x.as_str()) {
                let m = m.trim().to_uppercase();
                if m == "POST" || m == "PUT" {
                    c.method = m;
                }
            }
            if let Some(h) = req.get("headers").and_then(|x| x.as_str()) {
                c.headers = h.to_string();
            }
            if let Some(arr) = req.get("states").and_then(|x| x.as_array()) {
                let v: Vec<String> = arr
                    .iter()
                    .filter_map(|x| x.as_str())
                    .map(|x| x.trim().to_uppercase())
                    .filter(|x| !x.is_empty())
                    .collect();
                if !v.is_empty() {
                    c.states = v;
                }
            }
            if let Some(i) = req.get("min_interval_secs").and_then(|x| x.as_u64()) {
                c.min_interval_secs = i.clamp(0, 86400);
            }
            lockv(&state.settings).notify = c.clone();
            if let Err(e) = save_config_checked(state) {
                return json!({"ok": false, "error": format!("config non écrite : {e}")});
            }
            add_log(state, "info", "[notif] configuration enregistrée");
            // LOT B point 2 : la réponse porte la forme MASQUÉE (to_api_json) —
            // les en-têtes du webhook ne sortent JAMAIS en clair, pas même à
            // l'appelant qui vient d'enregistrer (même surface que /api/state).
            json!({"ok": true, "notify": crate::notify::to_api_json(&c)})
        }
        "test_notify" => {
            // Bouton « Tester » du panneau Notifications (dialog Réglages) :
            // émet une notification de test sur chaque canal ACTIVÉ et rend le
            // résultat par canal. Non exposé au gateway (déclenche un toast /
            // une requête réseau vers l'URL configurée — surface de confiance).
            let cfg = lockv(&state.settings).notify.clone();
            if !cfg.desktop && cfg.webhook_url.trim().is_empty() {
                return json!({
                    "ok": false,
                    "error": "aucun canal actif : active le toast desktop ou renseigne une URL de webhook"
                });
            }
            let desktop = if cfg.desktop {
                Some(
                    match crate::notify::desktop_toast(
                        "MCP Control",
                        "notification de test — le canal desktop fonctionne",
                    ) {
                        Ok(()) => json!({"ok": true}),
                        Err(e) => json!({"ok": false, "error": e}),
                    },
                )
            } else {
                None
            };
            let webhook = if !cfg.webhook_url.trim().is_empty() {
                Some(
                    match crate::notify::send_webhook(
                        &cfg,
                        "test",
                        "TEST",
                        "notification de test émise depuis Réglages → Notifications",
                    ) {
                        Ok(()) => json!({"ok": true}),
                        Err(e) => json!({"ok": false, "error": e}),
                    },
                )
            } else {
                None
            };
            add_log(state, "info", "[notif] test émis depuis les réglages");
            json!({"ok": true, "desktop": desktop, "webhook": webhook})
        }
        "reset_runtime" => {
            // Effacer l'historique runtime (crashs, redémarrages) et les
            // compteurs d'échec — utile quand un serveur a été réparé à la
            // main et que l'app doit repartir sur des bases propres.
            let mut rt = lockv(&state.runtime);
            let n = rt.len();
            rt.clear();
            drop(rt);
            lockv(&state.spawn_fails).clear();
            if let Err(e) = save_config_checked(state) {
                return json!({"ok": false, "error": format!("config non écrite : {e}")});
            }
            add_log(
                state,
                "info",
                &format!("[runtime] historique effac\u{e9} ({n} serveur(s))"),
            );
            json!({"ok": true, "cleared": n})
        }
        "check_update" => {
            // Re-vérification forcée de la release GitHub — le thread de fond
            // ne tourne qu'au boot puis chaque heure. Le bouton « Vérifier » du
            // dialog Réglages obtient un résultat immédiat ET honnête :
            // `checked:false` = le check a échoué (réseau) — l'UI ne doit PAS
            // afficher « à jour » dans ce cas (c'était le clic mort d'avant).
            let verdict = check_update(state);
            let latest = lockv(&state.update_latest).clone();
            let checked = !matches!(verdict, UpdateCheck::Unreachable);
            let sha256 = check_update_sha256_json(
                checked,
                latest.as_deref(),
                running_asset(),
                expected_sha256,
            );
            json!({
                "ok": true,
                "checked": checked,
                "current": env!("CARGO_PKG_VERSION"),
                "latest": latest,
                "update_available": latest.is_some(),
                "sha256": sha256,
            })
        }
        "open_release_page" => {
            // Ouvre la page des releases dans le navigateur par défaut de la
            // machine qui fait tourner le moteur — FIX du clic mort : dans la
            // webview Tauri (desktop), `window.open` est neutralisé, donc le
            // badge « mise à jour » et le bouton des réglages passent par ici.
            // URL FIXE sans argument : aucune surface d'injection ; non exposé
            // au gateway (le serveur n'a pas à ouvrir des navigateurs).
            const RELEASES_URL: &str =
                "https://github.com/DeamonDev888/mcp-control/releases/latest";
            #[cfg(windows)]
            let mut cmd = {
                let mut c = std::process::Command::new(probe::system_tool("rundll32"));
                c.args(["url.dll,FileProtocolHandler", RELEASES_URL]);
                c
            };
            #[cfg(target_os = "macos")]
            let mut cmd = {
                let mut c = std::process::Command::new("open");
                c.arg(RELEASES_URL);
                c
            };
            #[cfg(all(unix, not(target_os = "macos")))]
            let mut cmd = {
                let mut c = std::process::Command::new("xdg-open");
                c.arg(RELEASES_URL);
                c
            };
            let opened = {
                #[cfg(any(windows, target_os = "macos", all(unix, not(target_os = "macos"))))]
                {
                    probe::probe_hidden_cmd(&mut cmd);
                    cmd.spawn().is_ok()
                }
                #[cfg(not(any(
                    windows,
                    target_os = "macos",
                    all(unix, not(target_os = "macos"))
                )))]
                {
                    false
                }
            };
            if opened {
                json!({"ok": true, "url": RELEASES_URL})
            } else {
                json!({
                    "ok": false,
                    "url": RELEASES_URL,
                    "error": "pas de navigateur lançable sur cette machine — ouvre l'URL à la main",
                })
            }
        }
        "install_update" => perform_auto_update(state),
        "download_update" => {
            // Bouton « ⬆ Mettre à jour » : téléchargement DIRECT du BON asset
            // (installateur desktop ou CLI, pour le BON OS) au lieu de la page
            // des releases. Détection : l'OS vient de la requête quand l'UI
            // connaît celui du NAVIGATEUR (la machine où atterrit le fichier —
            // cas UI servie par un moteur distant), sinon l'OS du binaire qui
            // tourne ; le type (desktop/cli) suit le binaire courant. L'URL est
            // construite UNIQUEMENT depuis la table blanche `release_asset` —
            // jamais depuis la requête. Non exposé au gateway (ouvre un
            // téléchargement).
            let kind = match std::env::current_exe()
                .ok()
                .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_lowercase()))
            {
                Some(exe) if exe.contains("desktop") => "desktop",
                _ => "cli",
            };
            let os = match req.get("os").and_then(Value::as_str) {
                Some(o @ ("windows" | "macos" | "linux")) => o,
                // os fourni mais hors whitelist → refus net : JAMAIS un
                // téléchargement pour une plateforme différente de celle
                // demandée (le repli silencieux sur le moteur serait une
                // surprise) ; aucun effet de bord avant ce contrôle.
                Some(other) => {
                    return json!({
                        "ok": false,
                        "error": format!("os inconnu : {other} (windows, macos ou linux attendus)"),
                    });
                }
                None => engine_os(),
            };
            let Some(asset) = release_asset(kind, os) else {
                return json!({
                    "ok": false,
                    "error": format!("plateforme inconnue : {kind}/{os}"),
                });
            };
            let url = format!(
                "https://github.com/DeamonDev888/mcp-control/releases/latest/download/{asset}"
            );
            #[cfg(windows)]
            let mut cmd = {
                let mut c = std::process::Command::new(probe::system_tool("rundll32"));
                c.args(["url.dll,FileProtocolHandler", &url]);
                c
            };
            #[cfg(target_os = "macos")]
            let mut cmd = {
                let mut c = std::process::Command::new("open");
                c.arg(&url);
                c
            };
            #[cfg(all(unix, not(target_os = "macos")))]
            let mut cmd = {
                let mut c = std::process::Command::new("xdg-open");
                c.arg(&url);
                c
            };
            let opened = {
                #[cfg(any(windows, target_os = "macos", all(unix, not(target_os = "macos"))))]
                {
                    probe::probe_hidden_cmd(&mut cmd);
                    cmd.spawn().is_ok()
                }
                #[cfg(not(any(
                    windows,
                    target_os = "macos",
                    all(unix, not(target_os = "macos"))
                )))]
                {
                    false
                }
            };
            add_log(
                state,
                "info",
                &format!("[maj] téléchargement {kind} pour {os} : {asset}"),
            );
            if opened {
                json!({"ok": true, "url": url, "asset": asset, "kind": kind, "os": os})
            } else {
                // l'URL reste rendue : l'UI (vrai navigateur) fait le repli
                // window.open elle-même
                json!({
                    "ok": false,
                    "url": url,
                    "asset": asset,
                    "kind": kind,
                    "os": os,
                    "error": "pas de navigateur lançable sur cette machine — ouvre l'URL à la main",
                })
            }
        }
        "reset_state" => {
            // Réinitialisation depuis le dialog Réglages :
            // - placement : ordre des serveurs (server_order vidé)
            // - detection : découverts + imports natifs litemcp retirés (ils
            //   reviendront à la prochaine annonce/sonde), pairs oubliés
            // - oublis   : seeds retirés durablement RÉINJECTÉS tout de suite
            //   (comme au boot) + machines SSH enregistrées oubliées
            let what = req.get("what").and_then(|x| x.as_str()).unwrap_or("");
            let mut removed_servers = 0usize;
            let mut reinjected = 0usize;
            let mut cleared: Vec<&str> = Vec::new();
            if what == "placement" || what == "tout" {
                lockv(&state.settings).server_order.clear();
                cleared.push("placement");
            }
            if what == "detection" || what == "tout" {
                {
                    let mut servers = lockv(&state.servers);
                    let before = servers.len();
                    servers.retain(|s| {
                        s.origin != Origin::Discovered && !s.id.starts_with("litemcp-")
                    });
                    removed_servers += before - servers.len();
                }
                lockv(&state.peers).clear();
                cleared.push("detection");
            }
            if what == "oublis" || what == "tout" {
                let removed = {
                    let mut st = lockv(&state.settings);
                    st.ssh_hosts.clear();
                    let removed = st.removed_seeds.clone();
                    st.removed_seeds.clear();
                    removed
                };
                // réinjection immédiate (même logique que le boot) : les
                // seeds retirés durablement redeviennent visibles sans
                // attendre un redémarrage
                {
                    let mut servers = lockv(&state.servers);
                    for s in default_servers() {
                        if removed.contains(&s.id) && !servers.iter().any(|x| x.id == s.id) {
                            servers.push(s);
                            reinjected += 1;
                        }
                    }
                }
                cleared.push("oublis");
            }
            if cleared.is_empty() {
                return json!({"ok": false, "error": "what inconnu (placement | detection | oublis | tout)"});
            }
            if let Err(e) = save_config_checked(state) {
                return json!({"ok": false, "error": format!("config non écrite : {e}")});
            }
            add_log(
                state,
                "info",
                &format!(
                    "[reset] {} — {} serveur(s) retiré(s), {} seed(s) réinjecté(s)",
                    cleared.join("+"),
                    removed_servers,
                    reinjected
                ),
            );
            json!({
                "ok": true,
                "cleared": cleared,
                "removed": removed_servers,
                "reinjected": reinjected,
            })
        }
        "set_autostart" => {
            let enable = req.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
            let confirm = req
                .get("confirm")
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
            // AUDIT S4, point 6 : une instance qui n'est PAS l'installation
            // enregistrée (build de dev, autre dossier, instance de test) ne
            // réécrit pas l'entrée Run d'une autre installation sans geste
            // explicite (`confirm:true`).
            let current = std::env::current_exe()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_default();
            let recorded = autostart_recorded_path();
            if autostart_needs_confirm(recorded.as_deref(), &current) && !confirm {
                return json!({
                    "ok": false,
                    "needs_confirm": true,
                    "recorded": recorded,
                    "error": "l'entrée de démarrage appartient à une autre installation : confirmez pour la réécrire (confirm:true)",
                });
            }
            lockv(&state.settings).autostart = enable;
            apply_autostart(state, enable);
            if let Err(e) = save_config_checked(state) {
                return json!({"ok": false, "error": format!("config non écrite : {e}")});
            }
            json!({"ok": true, "autostart": enable, "active": autostart_active()})
        }
        // Autoboot PAR SERVEUR : « ce serveur est relancé automatiquement au
        // démarrage de MCP Control (et après un crash) ». Persisté dans
        // config.json, indépendant de l'autostart OS de l'application.
        "set_auto_restart" => {
            let Some(id) = id else {
                return json!({"ok": false, "error": "id manquant"});
            };
            let enable = req.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
            let (label, found) = {
                let mut servers = lockv(&state.servers);
                match servers.iter_mut().find(|s| s.id == id) {
                    Some(srv) => {
                        srv.auto_restart = enable;
                        // réactiver l'autoboot après des échecs doit repartir de zéro
                        if enable {
                            lockv(&state.spawn_fails).remove(&id);
                        }
                        (srv.label.clone(), true)
                    }
                    None => (String::new(), false),
                }
            };
            if !found {
                return json!({"ok": false, "error": "serveur inconnu"});
            }
            if let Err(e) = save_config_checked(state) {
                return json!({"ok": false, "error": format!("config non écrite : {e}")});
            }
            add_log(
                state,
                "info",
                &format!(
                    "[{}] autoboot {}",
                    label,
                    if enable {
                        "activé (relance au démarrage et après crash)"
                    } else {
                        "désactivé (ne sera pas relancé automatiquement)"
                    }
                ),
            );
            json!({"ok": true, "id": id, "auto_restart": enable})
        }
        "set_favorite" => {
            // Favori (⭐) par serveur : la ligne est épinglée en haut de sa
            // section dans l'explorateur, quel que soit le tri. Persisté dans
            // config.json, indépendant de l'autoboot et de l'ordre posé au
            // glisser-déposer (l'épingle s'applique À L'AFFICHAGE).
            let Some(id) = id else {
                return json!({"ok": false, "error": "id manquant"});
            };
            let enable = req.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
            let (label, found) = {
                let mut servers = lockv(&state.servers);
                match servers.iter_mut().find(|s| s.id == id) {
                    Some(srv) => {
                        srv.favorite = enable;
                        (srv.label.clone(), true)
                    }
                    None => (String::new(), false),
                }
            };
            if !found {
                return json!({"ok": false, "error": "serveur inconnu"});
            }
            if let Err(e) = save_config_checked(state) {
                return json!({"ok": false, "error": format!("config non écrite : {e}")});
            }
            add_log(
                state,
                "info",
                &format!(
                    "[{}] favori {}",
                    label,
                    if enable { "épinglé" } else { "retiré" }
                ),
            );
            json!({"ok": true, "id": id, "favorite": enable})
        }
        "ssh_discover" => {
            let ip = req
                .get("ip")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            // user : fourni par la requête, sinon la machine enregistrée
            let user = req
                .get("user")
                .and_then(|x| x.as_str())
                .map(|u| u.trim().to_string())
                .filter(|u| !u.is_empty())
                .or_else(|| {
                    lockv(&state.settings)
                        .ssh_hosts
                        .iter()
                        .find(|h| h.ip == ip)
                        .map(|h| h.user.clone())
                });
            let Some(user) = user else {
                return json!({"ok": false, "error": "utilisateur SSH requis (ou enregistre la machine)"});
            };
            if ip.is_empty() {
                return json!({"ok": false, "error": "ip requise"});
            }
            // l'action peut prendre ~30 s (netstat distant + sonde des ports) :
            // on la détache pour ne pas bloquer le serveur HTTP, l'UI suit via
            // le journal et un rechargement d'état.
            let st = state.clone();
            let ip2 = ip.clone();
            std::thread::spawn(move || {
                ssh_discover(&st, &ip2, &user);
            });
            json!({"ok": true, "started": true, "note": "découverte SSH lancée — vois le journal, l'état se recharge"})
        }
        "import_peer" => {
            let ip = req.get("ip").and_then(|x| x.as_str()).unwrap_or("");
            if ip.is_empty() {
                return json!({"ok": false, "error": "ip manquante"});
            }
            import_peer(state, ip)
        }
        "call_tool" => {
            let sid = match req.get("id").and_then(|x| x.as_str()) {
                Some(x) if !x.is_empty() => x.to_string(),
                _ => return json!({"ok": false, "error": "id manquant"}),
            };
            let tool = match req.get("tool").and_then(|x| x.as_str()) {
                Some(x) if !x.is_empty() => x.to_string(),
                _ => return json!({"ok": false, "error": "tool manquant"}),
            };
            let arguments = match req.get("arguments") {
                None | Some(Value::Null) => json!({}),
                Some(v @ Value::Object(_)) => v.clone(),
                Some(_) => {
                    return json!({"ok": false, "error": "arguments doit être un objet JSON"});
                }
            };
            let target = {
                let servers = lockv(&state.servers);
                let Some(s) = servers.iter().find(|s| s.id == sid) else {
                    return json!({"ok": false, "error": "serveur inconnu"});
                };
                if s.state != SState::Up {
                    return json!({"ok": false, "error": "serveur hors ligne (UP requis)"});
                }
                if s.tools_disabled.iter().any(|t| t == &tool) {
                    return json!({"ok": false, "error": "outil désactivé"});
                }
                if s.via_ssh {
                    return json!({"ok": false, "error": "serveur SSH : exécution directe non supportée"});
                }
                match &s.transport {
                    Transport::Http { host, port, path } => {
                        let h = s
                            .bound_host
                            .as_deref()
                            .or(host.as_deref())
                            .unwrap_or("127.0.0.1")
                            .to_string();
                        (h, *port, path.clone(), s.protocol.clone())
                    }
                    Transport::Stdio { .. } => {
                        return json!({"ok": false, "error": "serveur stdio : exécution directe non supportée (HTTP uniquement)"});
                    }
                }
            };
            let (host, port, path, proto) = target;
            let timeout_ms: u64 = 15_000;
            add_log(state, "info", &format!("[ui] call_tool {sid}.{tool}"));
            match probe::call_http_tool(
                &host,
                port,
                &path,
                proto.as_deref(),
                &tool,
                arguments,
                timeout_ms,
            ) {
                Ok((res, elapsed_ms)) => {
                    json!({"ok": true, "result": res, "elapsed_ms": elapsed_ms})
                }
                Err(e) => json!({"ok": false, "error": e}),
            }
        }
        "toggle_tool" => {
            let sid = match req.get("id").and_then(|x| x.as_str()) {
                Some(x) => x.to_string(),
                None => return json!({"ok": false, "error": "id manquant"}),
            };
            let tool = match req.get("tool").and_then(|x| x.as_str()) {
                Some(x) => x.to_string(),
                None => return json!({"ok": false, "error": "tool manquant"}),
            };
            let (now_disabled, count) = {
                let mut servers = lockv(&state.servers);
                match servers.iter_mut().find(|s| s.id == sid) {
                    Some(s) => {
                        if let Some(pos) = s.tools_disabled.iter().position(|t| t == &tool) {
                            s.tools_disabled.remove(pos);
                            (false, tools_actifs(s))
                        } else {
                            s.tools_disabled.push(tool.clone());
                            (true, tools_actifs(s))
                        }
                    }
                    None => return json!({"ok": false, "error": "serveur inconnu"}),
                }
            };
            if let Err(e) = save_config_checked(state) {
                return json!({"ok": false, "error": format!("config non écrite : {e}")});
            }
            add_log(
                state,
                "info",
                &format!(
                    "[{}] tool '{}' {} ({} actifs)",
                    sid,
                    tool,
                    if now_disabled {
                        "DÉSACTIVÉ"
                    } else {
                        "réactivé"
                    },
                    count
                ),
            );
            json!({"ok": true, "disabled": now_disabled, "tools_actifs": count})
        }
        "install_store" => {
            let sid = req.get("id").and_then(|x| x.as_str()).unwrap_or("");
            match find_store_item(sid) {
                Some(it) => {
                    // args surchargés depuis la requête (ex. dossier du serveur Filesystem)
                    let mut args: Vec<String> = match req.get("args").and_then(|x| x.as_array()) {
                        Some(a) if !a.is_empty() => a
                            .iter()
                            .filter_map(|i| i.as_str().map(String::from))
                            .collect(),
                        _ => it.args.iter().map(|a| a.to_string()).collect(),
                    };
                    if args.is_empty() {
                        args = it.args.iter().map(|a| a.to_string()).collect();
                    }
                    // Filesystem : le dossier exposé doit être choisi EXPLICITEMENT —
                    // jamais un chemin personnel par défaut dans un binaire distribué.
                    if it.id == "filesystem" && filesystem_dossier_manquant(&args) {
                        return json!({"ok": false, "error": "précisez le dossier à exposer : {\"action\":\"install_store\",\"id\":\"filesystem\",\"args\":[\"-y\",\"@modelcontextprotocol/server-filesystem\",\"C:/chemin/autorisé\"]}"});
                    }
                    // env additionnel optionnel {"CLÉ": "valeur"} (lot A) : des
                    // items du catalogue ont besoin d'un jeton (GITHUB_TOKEN…).
                    // Mêmes garanties que l'import (lot 3) : les valeurs
                    // n'atteignent que le process lancé (StartCmd.env → spawn) —
                    // jamais l'API (noms seuls via server_json), le gateway
                    // (résultat sans env) ni le journal.
                    let env = env_from_json(&req);
                    let id = format!("store-{}", it.id);
                    let already = {
                        let mut servers = lockv(&state.servers);
                        let already = servers.iter().any(|s| s.id == id);
                        if !already {
                            servers.push(Server {
                                id: id.clone(),
                                label: it.name.to_string(),
                                transport: Transport::Stdio {
                                    program: it.program.to_string(),
                                    args: args.clone(),
                                    workdir: None,
                                    env: env.clone(),
                                },
                                origin: Origin::Manual,
                                enabled: true,
                                state: SState::Down,
                                auto_restart: true,
                                favorite: false,
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
                                via_ssh: false,
                                tools: vec![],
                                latency_ms: None,
                                last_seen_ts: None,
                                last_error: None,
                                start_cmd: Some(StartCmd {
                                    program: it.program.to_string(),
                                    args,
                                    workdir: None,
                                    env,
                                }),
                                tools_disabled: vec![],
                            });
                        }
                        already
                    };
                    if already {
                        return json!({"ok": true, "added": 0, "note": "déjà installé"});
                    }
                    if let Err(e) = save_config_checked(state) {
                        return json!({"ok": false, "error": format!("config non écrite : {e}")});
                    }
                    add_log(state, "info", &format!("[boutique] installé : {}", it.name));
                    poll_one(state, &id);
                    let st = {
                        let servers = lockv(&state.servers);
                        servers
                            .iter()
                            .find(|s| s.id == id)
                            .map(|s| s.state.as_str().to_string())
                            .unwrap_or_default()
                    };
                    json!({"ok": true, "added": 1, "id": id, "state": st})
                }
                None => json!({"ok": false, "error": "item inconnu"}),
            }
        }
        "import_mcp" => {
            let raw = req.get("text").and_then(|x| x.as_str()).unwrap_or("");
            import_mcp_text(state, raw)
        }
        "pair_list" => crate::auth::pair::list(state, crate::types::now_ts()),
        // ---- actions LOCALES d'authentification (droit `admin`, jamais au gateway)
        "pair_approve" => {
            let pair_id = req
                .get("pair_id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let rights: Vec<String> = req
                .get("rights")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            match crate::auth::pair::approve(
                state,
                &pair_id,
                rights.clone(),
                crate::types::now_ts(),
            ) {
                Ok(v) => {
                    if let Err(e) = save_config_checked(state) {
                        return json!({"ok": false, "error": format!("config non écrite : {e}")});
                    }
                    add_log(
                        state,
                        "info",
                        &format!(
                            "[auth] appairage approuvé : {} — droits {:?}",
                            v.get("keyid").and_then(|x| x.as_str()).unwrap_or("?"),
                            rights
                        ),
                    );
                    v
                }
                Err(e) => json!({"ok": false, "error": e}),
            }
        }
        "pair_reject" => {
            let pair_id = req.get("pair_id").and_then(|x| x.as_str()).unwrap_or("");
            match crate::auth::pair::reject(state, pair_id, crate::types::now_ts()) {
                Ok(v) => {
                    add_log(state, "info", "[auth] appairage refusé localement");
                    v
                }
                Err(e) => json!({"ok": false, "error": e}),
            }
        }
        "client_list" => clients_json(state),
        "client_set_scopes" => {
            let keyid = req.get("keyid").and_then(|x| x.as_str()).unwrap_or("");
            let rights: Vec<String> = req
                .get("rights")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            let res = lockv(&state.settings)
                .auth
                .set_rights(keyid, rights.clone());
            match res {
                Ok(()) => {
                    if let Err(e) = save_config_checked(state) {
                        return json!({"ok": false, "error": format!("config non écrite : {e}")});
                    }
                    add_log(
                        state,
                        "info",
                        &format!("[auth] droits de {} : {:?}", keyid, rights),
                    );
                    json!({"ok": true, "keyid": keyid, "rights": rights})
                }
                Err(e) => json!({"ok": false, "error": e.motif()}),
            }
        }
        "client_revoke" => {
            let keyid = req.get("keyid").and_then(|x| x.as_str()).unwrap_or("");
            let res = lockv(&state.settings).auth.revoke(keyid);
            match res {
                Ok(()) => {
                    let revoked_at = lockv(&state.settings)
                        .auth
                        .find(keyid)
                        .and_then(|c| c.revoked_at.clone())
                        .unwrap_or_default();
                    if let Err(e) = save_config_checked(state) {
                        return json!({"ok": false, "error": format!("config non écrite : {e}")});
                    }
                    add_log(
                        state,
                        "info",
                        &format!(
                            "[auth] client révoqué : {} — refus à la requête suivante",
                            keyid
                        ),
                    );
                    json!({"ok": true, "keyid": keyid, "revoked_at": revoked_at})
                }
                Err(e) => json!({"ok": false, "error": e.motif()}),
            }
        }
        "client_rotate" => {
            let keyid = req.get("keyid").and_then(|x| x.as_str()).unwrap_or("");
            let new_keyid = req
                .get("new_keyid")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let public_key = req
                .get("public_key")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let res = lockv(&state.settings)
                .auth
                .rotate(keyid, new_keyid, public_key);
            match res {
                Ok(rec) => {
                    if let Err(e) = save_config_checked(state) {
                        return json!({"ok": false, "error": format!("config non écrite : {e}")});
                    }
                    add_log(
                        state,
                        "info",
                        &format!("[auth] rotation {} → {}", keyid, rec.keyid),
                    );
                    json!({"ok": true, "keyid": rec.keyid, "rights": rec.rights, "rotated_from": keyid})
                }
                Err(e) => json!({"ok": false, "error": e.motif()}),
            }
        }
        // ---- Écran « Accès » (lot M3, docs/auth.md §5) ---------------------
        // Tout ce qui suit est LOCAL : `action_reservee_admin` refuse un appel
        // venu du LAN avant le dispatcher, et `confirm` refuse en plus un
        // acteur qui n'est pas la session locale (défense en profondeur).
        "access_list" => {
            let mut v = crate::auth::confirm::list(state, crate::auth::confirm::Actor::LocalAdmin);
            let pairs = crate::auth::pair::list(state, crate::types::now_ts());
            let (server, ui_bind, ui_port, tls_fp, tls_bind_refused, tls_notice) = {
                let st = lockv(&state.settings);
                (
                    st.auth.server.clone(),
                    st.ui_bind.clone(),
                    st.ui_port,
                    st.auth.tls_fp.clone(),
                    st.auth.tls_bind_refused,
                    st.auth.tls_notice.clone(),
                )
            };
            let keyring_block = server.as_ref().and_then(|_| {
                let svc = crate::auth::secret::service()?;
                crate::auth::secret::load_client_key(
                    Some(&svc),
                    crate::auth::secret::ACCOUNT_SERVER,
                )
                .ok()
                .flatten()
            });
            let server_identity = match server.as_ref() {
                Some(ident) => json!({
                    "keyid": ident.keyid,
                    "fingerprint": crate::auth::registry::fingerprint_of(&ident.public_key)
                        .unwrap_or_default(),
                    "fingerprint_court": crate::auth::registry::short_fingerprint_of(&ident.public_key)
                        .unwrap_or_default(),
                    "storage": server_storage_kind(
                        ident,
                        &state.config_path,
                        keyring_block.as_deref(),
                    ),
                }),
                None => json!({
                    "keyid": "",
                    "fingerprint": "",
                    "fingerprint_court": "",
                    "storage": "unavailable",
                }),
            };
            drop(keyring_block);
            let tls_active = !tls_fp.is_empty();
            let ui_url = current_ui_url(&ui_bind, ui_port, tls_active, tls_bind_refused);
            if let Value::Object(o) = &mut v {
                // Contrat des actions locales : `ok` explicite. Sans lui, l'écran
                // « Accès » s'arrête sur « lecture impossible » et n'affiche rien.
                o.insert("ok".into(), json!(true));
                o.insert(
                    "clients".into(),
                    clients_json(state)
                        .get("clients")
                        .cloned()
                        .unwrap_or(json!([])),
                );
                o.insert(
                    "pairs".into(),
                    pairs.get("pending").cloned().unwrap_or(json!([])),
                );
                o.insert("pairs_max".into(), pairs["pending_max"].clone());
                o.insert("pairs_ttl_secs".into(), pairs["ttl_secs"].clone());
                o.insert("server_pub".into(), pairs["server_pub"].clone());
                o.insert("server_identity".into(), server_identity);
                o.insert(
                    "sessions".into(),
                    json!({
                        "active": crate::auth::session::active_count(),
                        "max": crate::auth::session::MAX_SESSIONS,
                    }),
                );
                o.insert(
                    "tls".into(),
                    json!({
                        "compiled": crate::auth::net::tls_compiled(),
                        "active": tls_active,
                        "fingerprint": tls_fp,
                        "bind_refused": tls_bind_refused,
                        "notice": tls_notice,
                    }),
                );
                o.insert("ui_url".into(), json!(ui_url));
            }
            v
        }
        "confirm_approve" => {
            let id = req.get("request_id").and_then(|x| x.as_str()).unwrap_or("");
            let mode = req.get("mode").and_then(|x| x.as_str()).unwrap_or("once");
            match crate::auth::confirm::approve(
                state,
                id,
                mode,
                crate::auth::confirm::Actor::LocalAdmin,
            ) {
                Ok(v) => v,
                Err(e) => json!({"ok": false, "error": e}),
            }
        }
        "confirm_deny" => {
            let id = req.get("request_id").and_then(|x| x.as_str()).unwrap_or("");
            match crate::auth::confirm::deny(state, id, crate::auth::confirm::Actor::LocalAdmin) {
                Ok(v) => v,
                Err(e) => json!({"ok": false, "error": e}),
            }
        }
        "grant_revoke" => {
            let id = req.get("request_id").and_then(|x| x.as_str()).unwrap_or("");
            match crate::auth::confirm::revoke(state, id, crate::auth::confirm::Actor::LocalAdmin) {
                Ok(v) => v,
                Err(e) => json!({"ok": false, "error": e}),
            }
        }
        _ => json!({"ok": false, "error": format!("action inconnue: {}", action)}),
    }
}

/// Registre des clients, tel que l'écran « Accès » et `client_list` l'affichent
/// (clés PUBLIQUES et empreintes seulement — aucun secret, par construction).
fn clients_json(state: &Arc<AppState>) -> Value {
    let settings = lockv(&state.settings);
    let now = crate::types::now_ts();
    let clients: Vec<Value> = settings
        .auth
        .clients
        .iter()
        .map(|c| {
            let mut v = c.to_json();
            if let Value::Object(o) = &mut v {
                o.insert(
                    "fingerprint".into(),
                    json!(crate::auth::registry::fingerprint_of(&c.public_key).unwrap_or_default()),
                );
                o.insert(
                    "fingerprint_court".into(),
                    json!(crate::auth::registry::short_fingerprint_of(&c.public_key)
                        .unwrap_or_default()),
                );
                o.insert("stale_180d".into(), json!(c.stale_180d(now)));
            }
            v
        })
        .collect();
    json!({ "ok": true, "clients": clients })
}

/// Empreinte SHA-256 de l'actif courant pour `check_update` (S3 pt 5).
pub(super) fn check_update_sha256_json<F>(
    checked: bool,
    latest: Option<&str>,
    asset: Option<&'static str>,
    lookup: F,
) -> Option<Value>
where
    F: FnOnce(&str, &str) -> Result<Sha256Lookup, &'static str>,
{
    if !checked {
        return None;
    }
    let (v, a) = latest.zip(asset)?;
    let tag = format!("v{}", v.strip_prefix('v').unwrap_or(v));
    Some(match lookup(&tag, a) {
        Ok(Sha256Lookup::Found(s)) => json!({"asset": a, "sha256": s, "note": Value::Null}),
        Ok(_) => json!({
            "asset": a,
            "sha256": Value::Null,
            "note": "absent ou illisible dans SHA256SUMS.txt"
        }),
        Err(e) => json!({"asset": a, "sha256": Value::Null, "note": e}),
    })
}

/// Stockage effectif de la clé privée Ed25519 du serveur sans jamais exposer de
/// secret (`"keyring"` si la clé du trousseau correspond à l'identité publiée,
/// sinon `"file_fallback"` si `auth/server.key` existe, sinon `"unavailable"`).
pub(super) fn server_storage_kind(
    ident: &crate::auth::ServerIdentity,
    config_path: &str,
    keyring_secret_block: Option<&str>,
) -> &'static str {
    if let Some(block) = keyring_secret_block {
        if let Ok(key) = ::litemcp::auth::sign::SigningKey::import(block) {
            if crate::crypto::hex(&key.verifying_key().to_bytes()) == ident.public_key {
                return "keyring";
            }
        }
    }
    if !config_path.trim().is_empty() {
        let dir = std::path::Path::new(config_path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        if crate::auth::secret::server_key_path(dir).exists() {
            return "file_fallback";
        }
    }
    "unavailable"
}

/// URL courante de l'interface / appairage (`http(s)://hôte:port`), d'après
/// `ui_bind`, `ui_port` et TLS actif (repli `127.0.0.1` si `0.0.0.0` ou bind
/// refusé faute de feature `tls`).
pub(super) fn current_ui_url(
    ui_bind: &str,
    ui_port: u16,
    tls_active: bool,
    tls_bind_refused: bool,
) -> String {
    let scheme = if tls_active { "https" } else { "http" };
    let host = if tls_bind_refused {
        "127.0.0.1".to_string()
    } else {
        let b = ui_bind.trim();
        if b.is_empty() || matches!(b, "0.0.0.0" | "*" | "::" | "[::]") {
            "127.0.0.1".to_string()
        } else if b == "::1" || b == "[::1]" {
            "[::1]".to_string()
        } else if b.contains(':') && !b.starts_with('[') {
            format!("[{b}]")
        } else {
            b.to_string()
        }
    };
    format!("{scheme}://{host}:{ui_port}")
}
