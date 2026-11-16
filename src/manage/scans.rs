// Découverte : scan de ports, auto-scan (netstat), scan réseau, dédoublonnage.
// (split v0.7.20 : manage.rs reste la racine — API stable via réexport)
use super::*;

/// Fenêtre accordée à un démarrage (spawn réussi → UP réellement observé).
/// Un lanceur qui construit avant d'écouter (bin/launch.cjs fait `npm run
/// build`) a ce délai ; passé l'échéance sans UP, le démarrage compte comme
/// un échec (note_spawn_failure). Vérifié aux CYCLES suivants, jamais en
/// bloquant le cycle (audit 03, constats 2 et 3).
const START_WINDOW_SECS: u64 = 30;

#[allow(unused_must_use)]
pub fn poll_one(state: &Arc<AppState>, id: &str) {
    let (transport, enabled) = {
        let servers = lockv(&state.servers);
        match servers.iter().find(|s| s.id == id) {
            Some(s) => (s.transport.clone(), s.enabled),
            None => return,
        }
    };
    if !enabled {
        return;
    }
    match transport {
        Transport::Http { host, port, path } => {
            // serveurs via_ssh : la sonde tourne DEPUIS la machine (curl interne
            // sur 127.0.0.1 via la connexion ssh) — l'extérieur est souvent
            // bloqué (bind localhost, ufw)
            let via_ssh = lockv(&state.servers)
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.via_ssh)
                .unwrap_or(false);
            let mut res = if via_ssh {
                match &host {
                    Some(h) => {
                        let user = lockv(&state.settings)
                            .ssh_hosts
                            .iter()
                            .find(|x| x.ip == *h)
                            .map(|x| x.user.clone());
                        match user {
                            Some(u) => probe::ssh_probe_server(h, &u, port, &path, 6),
                            None => probe::McpProbe::fail(
                                "machine SSH non enregistrée — réenregistre-la dans Machines"
                                    .to_string(),
                                true,
                            ),
                        }
                    }
                    None => probe::McpProbe::fail("serveur via SSH sans hôte".to_string(), true),
                }
            } else {
                match &host {
                    // budget de sonde 8 s (lot A) : un serveur réellement lent
                    // mais vivant (réponse en ~6 s) doit rester classé UP —
                    // le port muet, lui, est coupé dès la 1re forme sans octet
                    // (probe_http, bail précoce) au lieu d'encaisser 3 timeout.
                    Some(h) => probe::probe_http_host(h, port, &path, 8000),
                    None => probe::probe_http(port, &path, 8000),
                }
            };
            // Auto-start : serveur HTTP LOCAL éteint + commande de lancement configurée
            // (jamais pour un serveur distant, host.is_none() gate)
            if !res.mcp_up && host.is_none() {
                // État du process lancé par nous, s'il existe : vivant ? code de sortie ?
                let (child_alive, exit_code) = {
                    let children = lockv(&state.children);
                    match children.get(id) {
                        Some(h) => {
                            let code = *h.exit.lock().unwrap();
                            // watcher : Some dès que le process est sorti —
                            // déterministe (aucune course WNOHANG/ECHILD).
                            (code.is_none(), code)
                        }
                        None => (false, None),
                    }
                };
                // ÉCHEC DE DÉMARRAGE (audit 03, constat 3) : `start-ok` n'est plus
                // journalisé au spawn — un démarrage ne compte comme succès que
                // lorsqu'un UP est réellement observé (branche UP ci-dessous).
                // Un process sorti sans avoir répondu MCP, ou une échéance de
                // démarrage dépassée, comptent un échec via note_spawn_failure
                // (compteur persisté, backoff croissant, abandon à MAX_ATTEMPTS) :
                // la relance n'est plus infinie.
                let now = crate::types::now_ts();
                let echec_demarrage = {
                    let mut rt = lockv(&state.runtime);
                    let entry = rt.entry(id.to_string()).or_default();
                    let echeance = entry.starting_until;
                    if echeance != 0 && (!child_alive || now >= echeance) {
                        entry.starting_until = 0;
                        true
                    } else {
                        false
                    }
                };
                if echec_demarrage {
                    let motif = if child_alive {
                        format!(
                            "échéance de démarrage ({} s) dépassée sans réponse MCP",
                            START_WINDOW_SECS
                        )
                    } else {
                        format!(
                            "process sorti sans avoir répondu MCP{}",
                            exit_code
                                .map(|c| format!(" (code de sortie {c})"))
                                .unwrap_or_default()
                        )
                    };
                    add_log(state, "warn", &format!("[{id}] démarrage KO : {motif}"));
                    note_spawn_failure(state, id);
                }
                // can_auto relu APRÈS le comptage d'échec : le 5ᵉ échec désactive
                // l'auto-restart (note_spawn_failure) et doit empêcher le spawn
                // qui suivrait dans le même cycle.
                let (can_auto, backoff_ok) = {
                    let can = lockv(&state.servers)
                        .iter()
                        .find(|s| s.id == id)
                        .map(|s| s.start_cmd.is_some() && s.auto_restart)
                        .unwrap_or(false);
                    let now = crate::types::now_ts();
                    let ok = {
                        let mut rt = lockv(&state.runtime);
                        rt.entry(id.to_string()).or_default().can_attempt(now)
                    };
                    (can, ok)
                };
                if can_auto
                    && !child_alive
                    && backoff_ok
                    && lockv(&state.spawning).insert(id.to_string())
                {
                    // garde posée : le poller et une action API ne peuvent pas
                    // doubler le spawn du même serveur (course refresh vs poll)
                    let cmd = {
                        let servers = lockv(&state.servers);
                        servers
                            .iter()
                            .find(|s| s.id == id)
                            .and_then(|s| s.start_cmd.clone())
                    };
                    if let Some(cmd) = cmd {
                        match probe::spawn_process(
                            &cmd.program,
                            &cmd.args,
                            cmd.workdir.as_deref(),
                            &cmd.env,
                        ) {
                            Ok(h) => {
                                let pid = h.pid;
                                lockv(&state.children).insert(id.to_string(), h);
                                lockv(&state.spawn_fails).remove(id);
                                {
                                    let mut servers = lockv(&state.servers);
                                    if let Some(srv) = servers.iter_mut().find(|s| s.id == id) {
                                        srv.pid = Some(pid);
                                    }
                                }
                                add_log(
                                    state,
                                    "info",
                                    &format!("[{}] http démarré (pid {}) — re-sonde", id, pid),
                                );
                                // Marqueur « démarrage en cours » avec échéance :
                                // les cycles suivants re-vérifient l'état du serveur
                                // (UP réel → start-ok ; process sorti ou échéance
                                // dépassée → échec) au lieu de bloquer ici.
                                {
                                    let mut rt = lockv(&state.runtime);
                                    rt.entry(id.to_string()).or_default().starting_until =
                                        crate::types::now_ts() + START_WINDOW_SECS;
                                }
                                // Re-sonde UNIQUE et courte : un lanceur qui
                                // construit avant d'écouter est revérifié aux
                                // cycles suivants — le cycle n'attend plus 30 s
                                // (audit 03, constat 2 : c'était la cause principale
                                // des périodes mesurées à 48-90 s).
                                let mut res2 = probe::probe_http(port, &path, 2000);
                                if res2.mcp_up && res2.pid.is_none() {
                                    res2.pid = Some(pid);
                                }
                                res = res2;
                            }
                            Err(e) => {
                                note_spawn_failure(state, id);
                                add_log(
                                    state,
                                    "warn",
                                    &format!("[{}] démarrage HTTP KO : {}", id, e),
                                )
                            }
                        }
                    }
                    // libère la garde anti-doublon quoi qu'il arrive
                    lockv(&state.spawning).remove(id);
                }
            }
            // UP RÉELLEMENT OBSERVÉ : le seul endroit où le compteur d'échecs
            // retombe à zéro (runtime_event start-ok/recovered). Consommé AVANT
            // le verrou servers — note_spawn_failure prend servers sous runtime,
            // l'ordre inverse ici déboucherait sur un interblocage.
            let (etait_en_demarrage, avait_echecs, en_demarrage) = if res.mcp_up {
                let mut rt = lockv(&state.runtime);
                let entry = rt.entry(id.to_string()).or_default();
                let etait = entry.starting_until != 0;
                entry.starting_until = 0;
                (etait, entry.fails > 0, 0)
            } else {
                // marqueur lu (et relâché) AVANT le verrou servers : jamais
                // runtime sous servers ici non plus
                let en_cours = {
                    let rt = lockv(&state.runtime);
                    rt.get(id).map(|e| e.starting_until).unwrap_or(0)
                };
                (false, false, en_cours)
            };
            let mut log_event: Option<(&'static str, String)> = None;
            {
                let mut servers = lockv(&state.servers);
                if let Some(srv) = servers.iter_mut().find(|s| s.id == id) {
                    if res.mcp_up {
                        let changed = srv.server_name != res.name || srv.state != SState::Up;
                        srv.state = SState::Up;
                        srv.bound_host = res.host.clone();
                        srv.server_name = res.name.clone();
                        srv.version = res.version.clone();
                        srv.protocol = res.protocol.clone();
                        srv.sse = res.sse;
                        srv.sse_endpoint = res.sse_endpoint;
                        srv.ws = res.ws;
                        srv.lang = res.lang.clone();
                        srv.framework = res.framework.clone();
                        srv.tools = res.tools.clone();
                        srv.latency_ms = Some(res.latency_ms);
                        srv.last_seen_ts = Some(crate::types::now_ts());
                        srv.last_error = None;
                        if res.pid.is_some() {
                            srv.pid = res.pid;
                        }
                        if changed {
                            log_event = Some((
                                "info",
                                format!(
                                    "[{}] UP : {} v{} — {} tools ({} ms)",
                                    srv.label,
                                    res.name.as_deref().unwrap_or("?"),
                                    res.version.as_deref().unwrap_or("?"),
                                    tools_actifs(srv),
                                    res.latency_ms
                                ),
                            ));
                        }
                    } else {
                        // Démarrage en cours (marqueur posé au spawn, échéance non
                        // atteinte) : ni DOWN ni PORT-ONLY, pas d'incident, pid
                        // conservé — le serveur est revérifié aux cycles suivants.
                        if en_demarrage > crate::types::now_ts() {
                            srv.last_error = Some("démarrage en cours…".to_string());
                        } else {
                            let was_up = srv.state == SState::Up;
                            // port fermé = DOWN ; port ouvert sans MCP = PORT-ONLY (zombie)
                            srv.state = if res.port_open {
                                SState::PortOnly
                            } else {
                                SState::Down
                            };
                            srv.last_error = res.error.clone();
                            srv.latency_ms = None;
                            if !res.port_open && host.is_none() {
                                srv.pid = None;
                            }
                            if was_up {
                                let st_lbl = if res.port_open { "PORT-ONLY" } else { "DOWN" };
                                log_event = Some((
                                    "warn",
                                    format!(
                                        "[{}] {} : {}",
                                        srv.label,
                                        st_lbl,
                                        res.error.as_deref().unwrap_or("?")
                                    ),
                                ));
                                // NOTIFICATION d'incident (desktop + webhook) :
                                // déclenchée une seule fois, à la transition.
                                let lbl = srv.label.clone();
                                let det = res.error.clone().unwrap_or_else(|| "?".to_string());
                                notify_incident(state, &lbl, st_lbl, &det);
                            }
                        }
                    }
                }
            }
            if etait_en_demarrage {
                runtime_event(
                    state,
                    id,
                    "start-ok",
                    &format!(
                        "pid {}",
                        res.pid
                            .map(|p| p.to_string())
                            .unwrap_or_else(|| "?".to_string())
                    ),
                );
            } else if avait_echecs {
                runtime_event(state, id, "recovered", "UP observé après échec(s)");
            }
            if let Some((lvl, msg)) = log_event {
                add_log(state, lvl, &msg);
            }
        }
        Transport::Stdio { .. } => {
            // Le child stdio peut être MORT : on note le crash (code de sortie)
            // et on garde l'information pour l'historique runtime.
            let (alive, exit_code) = {
                let mut children = lockv(&state.children);
                let mut still = Vec::new();
                let mut code = None;
                for (cid, h) in children.iter() {
                    if cid == id {
                        let c = *h.exit.lock().unwrap();
                        if c.is_some() {
                            still.push(cid.clone());
                            code = c;
                        }
                    }
                }
                for cid in still {
                    children.remove(&cid);
                }
                (children.contains_key(id), code)
            };
            if let Some(code) = exit_code {
                let detail = match code {
                    0 => "arrêté proprement (code 0)".to_string(),
                    c => format!("crash : code de sortie {c}"),
                };
                add_log(state, "warn", &format!("[{id}] {detail}"));
                runtime_event(state, id, "crash", &detail);
            }
            let (enabled, has_cmd) = {
                let servers = lockv(&state.servers);
                match servers.iter().find(|s| s.id == id) {
                    Some(s) => (s.enabled, s.start_cmd.is_some() && s.auto_restart),
                    None => return,
                }
            };
            if alive {
                let mut servers = lockv(&state.servers);
                if let Some(srv) = servers.iter_mut().find(|s| s.id == id) {
                    srv.state = SState::Up;
                    srv.last_seen_ts = Some(crate::types::now_ts());
                }
            } else if enabled && has_cmd {
                // (re)démarrage auto avec handshake complet — même garde anti-doublon
                if !lockv(&state.spawning).insert(id.to_string()) {
                    return;
                }
                let cmd = {
                    let mut servers = lockv(&state.servers);
                    servers
                        .iter_mut()
                        .find(|s| s.id == id)
                        .and_then(|s| s.start_cmd.clone())
                };
                if let Some(cmd) = cmd {
                    match probe::probe_stdio(
                        &cmd.program,
                        &cmd.args,
                        cmd.workdir.as_deref(),
                        15000,
                        true,
                        &cmd.env,
                    ) {
                        Ok((p, handle)) => {
                            if let Some(h) = handle {
                                lockv(&state.children).insert(id.to_string(), h);
                            }
                            lockv(&state.spawn_fails).remove(id);
                            let mut servers = lockv(&state.servers);
                            if let Some(srv) = servers.iter_mut().find(|s| s.id == id) {
                                srv.state = SState::Up;
                                srv.pid = p.pid;
                                srv.server_name = p.name.clone();
                                srv.version = p.version.clone();
                                srv.protocol = p.protocol.clone();
                                srv.sse = p.sse;
                                srv.sse_endpoint = p.sse_endpoint;
                                srv.lang = p.lang.clone();
                                srv.framework = p.framework.clone();
                                srv.tools = p.tools.clone();
                                srv.latency_ms = Some(p.latency_ms);
                                srv.last_seen_ts = Some(crate::types::now_ts());
                                srv.last_error = None;
                            }
                            drop(servers);
                            add_log(
                                state,
                                "info",
                                &format!(
                                    "[{}] stdio démarré (pid {}) : {} — {} tools",
                                    id,
                                    p.pid.map(|x| x.to_string()).unwrap_or_else(|| "?".into()),
                                    p.name.as_deref().unwrap_or("?"),
                                    p.tools.len()
                                ),
                            );
                            runtime_event(state, id, "start-ok", "stdio");
                        }
                        Err(e) => {
                            let mut servers = lockv(&state.servers);
                            if let Some(srv) = servers.iter_mut().find(|s| s.id == id) {
                                srv.state = SState::Down;
                                srv.last_error = Some(e.clone());
                                srv.pid = None;
                            }
                            drop(servers);
                            note_spawn_failure(state, id);
                            add_log(state, "warn", &format!("[{}] stdio KO : {}", id, e));
                        }
                    }
                }
                // libère la garde anti-doublon quoi qu'il arrive
                lockv(&state.spawning).remove(id);
            } else {
                let mut servers = lockv(&state.servers);
                if let Some(srv) = servers.iter_mut().find(|s| s.id == id) {
                    srv.state = SState::Down;
                }
            }
        }
    }
}

// ---------- scan / découverte ----------

/// Plafond du scan par plage (lot 3, audit 03 constat 10) : au-delà, le
/// handler refuse une plage explicite et le scan tronque défensivement une
/// plage issue des réglages — un balayage synchrone séquentiel de plage large
/// figeait l'API (~313 ms/port mesurés, soit 5 min pour 1 000 ports).
pub const SCAN_MAX_PORTS: usize = 512;

/// Taille du pool std éphémère du balayage. Le pool PERSISTANT du cycle de vie
/// (poll_one) reste inchangé — ici un pool est créé et joint à chaque scan.
const SCAN_WORKERS: usize = 32;

/// Plafonne une plage de scan à SCAN_MAX_PORTS ports. Pur et testé.
pub(super) fn scan_plafonne(from: u16, to: u16) -> (u16, u16) {
    if (to as u32 - from as u32) < SCAN_MAX_PORTS as u32 {
        return (from, to);
    }
    let head = from as u32 + SCAN_MAX_PORTS as u32 - 1;
    if head <= u16::MAX as u32 {
        (from, head as u16)
    } else {
        // en haut de l'espace u16, on garde 512 ports collés à la borne haute
        ((u16::MAX as u32 - SCAN_MAX_PORTS as u32 + 1) as u16, to)
    }
}

#[allow(unused_must_use)]
pub fn scan_ports(state: &Arc<AppState>, range: Option<(u16, u16)>) -> u32 {
    let brut = match range {
        Some(r) => r,
        None => {
            let s = lockv(&state.settings);
            (s.scan_from, s.scan_to)
        }
    };
    if brut.0 > brut.1 {
        add_log(
            state,
            "info",
            &format!("scan {}..{} vide : rien à faire", brut.0, brut.1),
        );
        return 0;
    }
    let (from, to) = scan_plafonne(brut.0, brut.1);
    if (from, to) != brut {
        add_log(
            state,
            "warn",
            &format!(
                "scan {}..{} tronqué à {} ports ({}..{}) — réglez scan_from/scan_to",
                brut.0, brut.1, SCAN_MAX_PORTS, from, to
            ),
        );
    }
    add_log(
        state,
        "info",
        &format!(
            "scan {}..{} sur [::1]/127.0.0.1 ({} sondes parallèles)…",
            from, to, SCAN_WORKERS
        ),
    );
    // BALAYAGE PARALLÈLE (pool std éphémère par segments contigus) : avant, le
    // balayage séquentiel coûtait le budget complet 2 × 150 ms par port mesuré
    // (~313 s pour 1 000 ports) et figeait le handler HTTP. Chaque thread sonde
    // son segment, les résultats arrivent par canal — même approche std que le
    // reste du dépôt, sans toucher au pool persistant du cycle.
    let total = (to - from + 1) as usize;
    let workers = SCAN_WORKERS.min(total).max(1);
    let chunk = total.div_ceil(workers);
    let (rtx, rrx) = std::sync::mpsc::channel::<(u16, String)>();
    std::thread::scope(|scope| {
        let mut start = from as u32;
        let mut rest = total;
        for _ in 0..workers {
            let count = chunk.min(rest);
            if count == 0 {
                break;
            }
            let seg_start = start as u16;
            let seg_end = (start + count as u32 - 1) as u16;
            let rtx = rtx.clone();
            scope.spawn(move || {
                for port in seg_start..=seg_end {
                    if probe::port_open("::1", port, std::time::Duration::from_millis(150)) {
                        let _ = rtx.send((port, "::1".into()));
                    } else if probe::port_open(
                        "127.0.0.1",
                        port,
                        std::time::Duration::from_millis(150),
                    ) {
                        let _ = rtx.send((port, "127.0.0.1".into()));
                    }
                }
            });
            start += count as u32;
            rest -= count;
        }
    });
    drop(rtx);
    let mut open: Vec<(u16, String)> = rrx.into_iter().collect();
    open.sort();
    open.dedup();
    let own_ui = lockv(&state.settings).ui_port;
    let known: Vec<u16> = lockv(&state.servers)
        .iter()
        .filter_map(|s| match &s.transport {
            Transport::Http { host, port, .. }
                if host.as_deref().map(crate::is_loopback_bind).unwrap_or(true) =>
            {
                Some(*port)
            }
            _ => None,
        })
        .collect();
    // sondes MCP PARALLÈLES (même approche std que scan_listening) : un port
    // muet coûte 4 s — en séquentiel sur plusieurs ports ouverts, le handler
    // refigeait. Insertion sérialisée ensuite (verrou servers), triée par port
    // pour un ordre d'insertion déterministe (comportement d'avant conservé).
    let mut found = 0u32;
    let candidats: Vec<(u16, String)> = open
        .into_iter()
        .filter(|(p, _)| *p != own_ui && !known.contains(p))
        .collect();
    let (mtx, mrx) = std::sync::mpsc::channel::<(u16, String, &'static str, probe::McpProbe)>();
    std::thread::scope(|scope| {
        for (port, host) in candidats {
            let mtx = mtx.clone();
            scope.spawn(move || {
                if let Some((path, p)) = probe::probe_mcp_any_path(port, 4000) {
                    let _ = mtx.send((port, host, path, p));
                }
            });
        }
    });
    drop(mtx);
    let mut decouverts: Vec<(u16, String, &'static str, probe::McpProbe)> =
        mrx.into_iter().collect();
    decouverts.sort_by_key(|(p, _, _, _)| *p);
    for (port, host, path, p) in decouverts {
        let id = format!("disc-{}", port);
        let label = p.name.clone().unwrap_or_else(|| format!("port-{}", port));
        let mut servers = lockv(&state.servers);
        if !servers.iter().any(|s| {
            s.id == id
                || matches!(&s.transport,
                        Transport::Http { host, port: sp, .. }
                        if *sp == port
                            && host.as_deref().map(crate::is_loopback_bind).unwrap_or(true))
        }) {
            found += 1;
            servers.push(Server {
                id: id.clone(),
                label,
                transport: Transport::Http {
                    host: None,
                    port,
                    path: path.to_string(),
                },
                origin: Origin::Discovered,
                enabled: true,
                state: SState::Up,
                auto_restart: true,
                favorite: false,
                pid: p.pid,
                bound_host: p.host.clone().or(Some(host)),
                server_name: p.name.clone(),
                version: p.version.clone(),
                protocol: p.protocol.clone(),
                sse: p.sse,
                sse_endpoint: p.sse_endpoint,
                ws: p.ws,
                lang: p.lang.clone(),
                framework: p.framework.clone(),
                via_ssh: false,
                tools: p.tools,
                latency_ms: Some(p.latency_ms),
                last_seen_ts: Some(crate::types::now_ts()),
                last_error: None,
                start_cmd: None,
                tools_disabled: vec![],
            });
            drop(servers);
            add_log(
                state,
                "info",
                &format!(
                    "[scan] serveur MCP découvert sur port {} : {} v{}",
                    port,
                    p.name.as_deref().unwrap_or("?"),
                    p.version.as_deref().unwrap_or("?")
                ),
            );
        }
    }
    add_log(
        state,
        "info",
        &format!("scan terminé : {} nouveaux serveurs MCP", found),
    );
    found
}

/// Découverte locale — candidats = ports en écoute MOINS les serveurs locaux
/// déjà connus et notre propre port UI. Pur, testé.
pub(super) fn discovery_candidates(ports: &[u16], known_local: &[u16], own_ui: u16) -> Vec<u16> {
    ports
        .iter()
        .copied()
        .filter(|p| *p != own_ui && !known_local.contains(p))
        .collect()
}

/// AUTO-SCAN (boot + bouton ⟳) : sonde TOUS les ports en écoute de la machine
/// — netstat/ss, l'OS sait déjà ce qui écoute, aucune plage à deviner. Un
/// serveur sur 8787 est trouvé même avec scan_from/scan_to = 3000-3200 (le cas
/// qui rendait des serveurs invisibles). Les répondeurs deviennent des serveurs
/// découverts (non persistés : re-découverts à chaque boot, comme le range scan).
#[allow(unused_must_use)]
pub fn scan_listening(state: &Arc<AppState>) {
    let ports = probe::local_listening_ports();
    let own_ui = lockv(&state.settings).ui_port;
    let known_local: Vec<u16> = lockv(&state.servers)
        .iter()
        .filter_map(|s| match &s.transport {
            Transport::Http { host, port, .. }
                if host.as_deref().map(crate::is_loopback_bind).unwrap_or(true) =>
            {
                Some(*port)
            }
            _ => None,
        })
        .collect();
    let candidates = discovery_candidates(&ports, &known_local, own_ui);
    add_log(
        state,
        "info",
        &format!(
            "scan local : {} port(s) en écoute, {} inconnu(s) à sonder (tous ports — aucune plage)",
            ports.len(),
            candidates.len()
        ),
    );
    let mut found = 0u32;
    // sondes PARALLÈLES : un service silencieux (ports RPC Windows & co)
    // encaisse la connexion sans jamais répondre et coûte son timeout entier
    // (4 s) — en séquentiel, une trentaine de ports bavards paralysait la
    // découverte des minutes durant. Ajouts sérialisés ensuite (verrou servers).
    // Chaque port est sondé sur les chemins MCP usuels (/mcp, /, /api/mcp).
    let handles: Vec<_> = candidates
        .into_iter()
        .map(|port| std::thread::spawn(move || (port, probe::probe_mcp_any_path(port, 4000))))
        .collect();
    for h in handles {
        let (port, probe_res) = match h.join() {
            Ok(x) => x,
            Err(_) => continue,
        };
        let Some((path, p)) = probe_res else {
            continue;
        };
        let id = format!("disc-{}", port);
        let mut servers = lockv(&state.servers);
        if servers.iter().any(|s| {
            s.id == id
                || matches!(&s.transport,
                    Transport::Http { host, port: sp, .. }
                    if *sp == port
                        && host.as_deref().map(crate::is_loopback_bind).unwrap_or(true))
        }) {
            continue;
        }
        let label = p.name.clone().unwrap_or_else(|| format!("port-{}", port));
        servers.push(Server {
            id: id.clone(),
            label,
            transport: Transport::Http {
                host: None,
                port,
                path: path.to_string(),
            },
            origin: Origin::Discovered,
            enabled: true,
            state: SState::Up,
            auto_restart: true,
            favorite: false,
            pid: p.pid,
            bound_host: p.host.clone(),
            server_name: p.name.clone(),
            version: p.version.clone(),
            protocol: p.protocol.clone(),
            sse: p.sse,
            sse_endpoint: p.sse_endpoint,
            ws: p.ws,
            lang: p.lang.clone(),
            framework: p.framework.clone(),
            via_ssh: false,
            tools: p.tools,
            latency_ms: Some(p.latency_ms),
            last_seen_ts: Some(crate::types::now_ts()),
            last_error: None,
            start_cmd: None,
            tools_disabled: vec![],
        });
        drop(servers);
        found += 1;
        add_log(
            state,
            "info",
            &format!(
                "[scan] serveur MCP découvert sur port {} : {} v{}",
                port,
                p.name.as_deref().unwrap_or("?"),
                p.version.as_deref().unwrap_or("?")
            ),
        );
    }
    add_log(
        state,
        "info",
        &format!("scan local terminé : {} nouveau(x) serveur(s) MCP", found),
    );
}

// ---------- scan réseau local (MCP distants) ----------

#[allow(unused_must_use)]
pub fn scan_network(state: &Arc<AppState>, range: Option<(u16, u16)>) {
    let (from, to) = match range {
        Some(r) => r,
        None => {
            let s = lockv(&state.settings);
            (s.scan_from, s.scan_to)
        }
    };
    let prefixes = network_prefixes();
    if prefixes.is_empty() {
        add_log(state, "warn", "scan réseau : aucune interface IPv4 trouvée");
        return;
    }
    let total_found = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    for (prefix, self_ip) in &prefixes {
        add_log(
            state,
            "info",
            &format!(
                "scan réseau {}.1-254 (self={}) ports {}..{} (parallèle)",
                prefix, self_ip, from, to
            ),
        );
        let (found, _tested) = scan_network_range(state, prefix, self_ip, from, to);
        total_found.fetch_add(found, std::sync::atomic::Ordering::Relaxed);
    }
    let found = total_found.load(std::sync::atomic::Ordering::Relaxed);
    add_log(
        state,
        "info",
        &format!(
            "scan réseau terminé : {} nouveau(x) serveur(s) MCP distant(s)",
            found
        ),
    );
    prune_dead_network_servers(state);
    merge_duplicate_servers(state);
}

/// FUSION des doublons historiques : les imports répétés (SSH, scan) ont pu
/// créer plusieurs entrées pour le MÊME serveur (même nom annoncé, même
/// machine, port changé). On ne garde que la plus RÉCENTE (last_seen_ts max).
///
/// Périmètre volontairement RESTREINT (audit 2026-10-26, P1) : seules les
/// entrées DÉCOUVERTES (origin Discovered), HTTP avec hôte explicite et nom
/// annoncé non vide sont fusionnables. Jamais un serveur manuel, seed ou
/// importé d'un pair ; jamais une entrée sans nom annoncé — l'ancien repli
/// sur le label fusionnait deux serveurs distincts (deux instances légitimes
/// sur des ports différents, ou deux ajouts sans label) et la suppression
/// était persistée.
pub(super) fn merge_duplicate_servers(state: &Arc<AppState>) -> usize {
    let mut servers = lockv(&state.servers);
    let before = servers.len();
    // clé = nom ANNONCÉ normalisé + hôte (identité faible : le port peut avoir changé)
    let mut groups: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (i, s) in servers.iter().enumerate() {
        if s.origin != Origin::Discovered {
            continue;
        }
        let Transport::Http { host: Some(h), .. } = &s.transport else {
            continue;
        };
        // seul le nom annoncé (serverInfo.name) est une identité fiable :
        // un label (fabriqué « ip:port » ou saisi par l'utilisateur) ne doit
        // JAMAIS servir de clé de fusion
        let Some(name) = s
            .server_name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        else {
            continue;
        };
        let key = format!("{}@{}", name.to_lowercase(), h);
        groups.entry(key).or_default().push(i);
    }
    let mut to_remove: Vec<usize> = Vec::new();
    for (_key, mut idxs) in groups {
        if idxs.len() < 2 {
            continue;
        }
        // le plus récemment vu gagne
        idxs.sort_by_key(|i| std::cmp::Reverse(servers[*i].last_seen_ts.unwrap_or(0)));
        to_remove.extend(idxs[1..].iter().copied());
    }
    if !to_remove.is_empty() {
        to_remove.sort_unstable();
        to_remove.dedup();
        for i in to_remove.iter().rev() {
            servers.remove(*i);
        }
    }
    let removed = before - servers.len();
    drop(servers);
    if removed > 0 {
        add_log(
            state,
            "info",
            &format!("[réseau] {removed} doublon(s) de serveur fusionné(s)"),
        );
    }
    removed
}

/// NETTOYAGE : après un scan complet, un serveur réseau découvert dont la
/// machine n'a pas répondu pendant CE scan est un fantôme (hôte éteint, IP
/// recyclée). On ne supprime que ce qui est **découvert via le réseau**
/// (`net-<ip>.<port>`) — jamais un serveur manuel, seed ou importé d'un pair
/// (une machine éteinte ne doit pas effacer sa configuration).
///
/// Le scan marque les entrées qu'il a vues (`last_seen_ts` rafraîchi) : le
/// prune retire celles qui n'ont pas été rafraîchies pendant le scan.
pub(super) fn prune_dead_network_servers(state: &Arc<AppState>) {
    // fenêtre du scan : on ne purge que ce qui n'a pas été vu depuis 2 min
    let horizon = crate::types::now_ts() - 120;
    let mut servers = lockv(&state.servers);
    let before = servers.len();
    servers.retain(|s| {
        let is_net = s.id.starts_with("net-")
            && matches!(&s.transport, Transport::Http { host: Some(_), .. });
        if !is_net {
            return true;
        }
        // entrée réseau : gardée si elle a été vue récemment (le scan ou le
        // poll l'a sondée) ; sinon c'est un fantôme de machine disparue
        matches!(s.last_seen_ts, Some(ts) if ts > horizon)
    });
    let removed = before - servers.len();
    drop(servers);
    if removed > 0 {
        add_log(
            state,
            "info",
            &format!("[réseau] {} entrée(s) fantôme(s) retirée(s)", removed),
        );
    }
}

/// Toutes les IPv4 locales (toutes interfaces), dédupliquées.
/// Sources : ipconfig (Windows) / hostname -I (unix) + fallback UDP (interface de routage).
/// Le fallback UDP seul peut renvoyer l'IP VPN — d'où le parse des interfaces en priorité.
pub fn local_ips() -> Vec<String> {
    let mut ips = std::collections::BTreeSet::new();
    let output = if cfg!(windows) {
        let mut c = Command::new("ipconfig");
        probe::hidden(&mut c).output()
    } else {
        let mut c = Command::new("hostname");
        probe::hidden(&mut c).arg("-I").output()
    };
    if let Ok(o) = output {
        let text = String::from_utf8_lossy(&o.stdout).to_string();
        if cfg!(windows) {
            // ne prendre que les lignes "IPv4" (exclut masques et lignes "Passerelle par défaut"/"Default Gateway")
            // — le marqueur "ipv4" existe en FR ("Adresse IPv4") comme en EN ("IPv4 Address")
            for line in text.lines() {
                if !line.to_lowercase().contains("ipv4") {
                    continue;
                }
                for tok in line.split_whitespace() {
                    let tok = tok.trim_end_matches(',').trim_start_matches('.');
                    if crate::probe::looks_like_ip(tok) {
                        ips.insert(tok.to_string());
                    }
                }
            }
        } else {
            // `hostname -I` sort des IP brutes séparées par espaces — SANS marqueur
            // "ipv4" : l'ancien parse rejetait tout et le fallback UDP ne renvoyait
            // qu'une seule interface. On parse chaque token.
            for tok in text.split_whitespace() {
                if crate::probe::looks_like_ip(tok) {
                    ips.insert(tok.to_string());
                }
            }
        }
    }
    // fallback UDP : interface de routage sortante (paquet non émis)
    if let Ok(s) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if s.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = s.local_addr() {
                ips.insert(addr.ip().to_string());
            }
        }
    }
    ips.into_iter().collect()
}

/// Préfixes /24 distincts des interfaces locales.
pub fn network_prefixes() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for ip in local_ips() {
        if let Some((prefix, _)) = ip.rsplit_once('.') {
            if !out.iter().any(|(p, _)| p == prefix) {
                out.push((prefix.to_string(), ip));
            }
        }
    }
    out
}

/// Parallélisé : 254 hôtes × plage de ports. Retourne (nouveauserveurs, hôtes testés).
#[allow(unused_must_use)]
pub fn scan_network_range(
    state: &Arc<AppState>,
    prefix: &str,
    self_ip: &str,
    from: u16,
    to: u16,
) -> (u32, u32) {
    let ports: Vec<u16> = (from..=to).collect();
    let known_hosts_ports: Vec<(Option<String>, u16)> = lockv(&state.servers)
        .iter()
        .filter_map(|s| match &s.transport {
            Transport::Http { host, port, .. } => Some((host.clone(), *port)),
            _ => None,
        })
        .collect();

    use std::sync::atomic::{AtomicU32, Ordering};
    let found = std::sync::Arc::new(AtomicU32::new(0));
    let tested = std::sync::Arc::new(AtomicU32::new(0));
    let mut handles = Vec::new();
    for last in 1u16..=254 {
        let ip = format!("{}.{}", prefix, last);
        if ip == self_ip {
            continue;
        } // ne pas se scanner soi-même (scan loopback déjà couvert)
        let ports = ports.clone();
        let known = known_hosts_ports.clone();
        let state = state.clone();
        let found = found.clone();
        let tested = tested.clone();
        handles.push(std::thread::spawn(move || {
            let mut hit = false;
            for port in &ports {
                if !probe::port_open(&ip, *port, std::time::Duration::from_millis(400)) {
                    continue;
                }
                hit = true;
                if known
                    .iter()
                    .any(|(h, p)| h.as_deref() == Some(ip.as_str()) && p == port)
                {
                    continue;
                }
                let p = probe::probe_http_host(&ip, *port, "/mcp", 3000);
                if p.mcp_up {
                    found.fetch_add(1, Ordering::Relaxed);
                    let label = p.name.clone().unwrap_or_else(|| format!("{}:{}", ip, port));
                    let mut servers = lockv(&state.servers);
                    // DÉ-DUPLICATION : un serveur déjà connu (même hôte+port) est
                    // ignoré ; un serveur qui a CHANGÉ DE PORT (même nom, même
                    // hôte) remplace l'entrée fantôme au lieu de créer un doublon.
                    let known: Vec<(Option<String>, Option<String>, u16)> = servers
                        .iter()
                        .filter_map(|s| match &s.transport {
                            Transport::Http { host, port, .. } => {
                                Some((s.server_name.clone(), host.clone(), *port))
                            }
                            _ => None,
                        })
                        .collect();
                    let decision = crate::validate::dedup_decide(
                        &known,
                        p.name.as_deref(),
                        Some(ip.as_str()),
                        *port,
                    );
                    let target_id = match decision {
                        crate::validate::DedupDecision::Duplicate => {
                            // déjà connu : on met à jour ses métadonnées (framework,
                            // tools) sans créer d'entrée
                            if let Some(s) = servers.iter_mut().find(|s| {
                                matches!(&s.transport,
                                    Transport::Http { host: Some(h), port: sp, .. } if h == &ip && sp == port)
                            }) {
                                s.server_name = p.name.clone();
                                s.version = p.version.clone();
                                s.framework = p.framework.clone();
                                s.lang = p.lang.clone();
                                s.tools = p.tools;
                                s.latency_ms = Some(p.latency_ms);
                                s.last_seen_ts = Some(crate::types::now_ts());
                            }
                            drop(servers);
                            continue;
                        }
                        crate::validate::DedupDecision::Replace(idx) => {
                            // l'ancien port est fantôme : on le remplace
                            let old = servers.remove(idx);
                            let id = old.id;
                            add_log(
                                &state,
                                "info",
                                &format!(
                                    "[réseau] {} a changé de port ({} → {}) — entrée fantôme remplacée",
                                    label,
                                    known.get(idx).map(|(_, _, p)| *p).unwrap_or(0),
                                    port
                                ),
                            );
                            id
                        }
                        crate::validate::DedupDecision::Add => format!("net-{}.{}", ip, port),
                    };
                    if !servers.iter().any(|s| {
                        matches!(&s.transport,
                        Transport::Http { host: Some(h), port: sp, .. } if h == &ip && sp == port)
                    }) {
                        servers.push(Server {
                            id: target_id,
                            label,
                            transport: Transport::Http {
                                host: Some(ip.clone()),
                                port: *port,
                                path: "/mcp".into(),
                            },
                            origin: Origin::Discovered,
                            enabled: true,
                            state: SState::Up,
                            auto_restart: true,
                            favorite: false,
                            pid: None,
                            bound_host: Some(ip.clone()),
                            server_name: p.name.clone(),
                            version: p.version.clone(),
                            protocol: p.protocol.clone(),
                            sse: p.sse,
                            sse_endpoint: p.sse_endpoint,
                            ws: p.ws,
                            lang: p.lang.clone(),
                            framework: p.framework.clone(),
                            via_ssh: false,
                            tools: p.tools,
                            latency_ms: Some(p.latency_ms),
                            last_seen_ts: Some(crate::types::now_ts()),
                            last_error: None,
                            start_cmd: None,
                            tools_disabled: vec![],
                        });
                        drop(servers);
                        add_log(
                            &state,
                            "info",
                            &format!(
                                "[réseau] serveur MCP distant découvert : {}:{} — {} v{}",
                                ip,
                                port,
                                p.name.as_deref().unwrap_or("?"),
                                p.version.as_deref().unwrap_or("?")
                            ),
                        );
                    }
                }
            }
            if hit {
                tested.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
    (
        found.load(Ordering::Relaxed),
        tested.load(Ordering::Relaxed),
    )
}

// ---------- tests : cycle de démarrage, relances, échéance (lot 2) ----------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::RuntimeLog;
    use crate::types::{AppState, Origin, SState, Server, Settings, StartCmd, Transport};
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};

    fn serveur_local(id: &str, port: u16, cmd: Option<(String, Vec<String>)>) -> Server {
        Server {
            id: id.to_string(),
            label: id.to_string(),
            transport: Transport::Http {
                host: None,
                port,
                path: "/mcp".into(),
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
            start_cmd: cmd.map(|(program, args)| StartCmd {
                program,
                args,
                workdir: None,
                env: Vec::new(),
            }),
            auto_restart: true,
            favorite: false,
            tools_disabled: vec![],
        }
    }

    fn etat_test(servers: Vec<Server>) -> Arc<AppState> {
        Arc::new(AppState {
            servers: Mutex::new(servers),
            log: Mutex::new(VecDeque::new()),
            settings: Mutex::new(Settings::default()),
            children: Mutex::new(HashMap::new()),
            // config_path vide : save_config ne doit JAMAIS écrire un fichier
            // (cf. manage/tests.rs — la garde vide est testée)
            config_path: String::new(),
            peers: Mutex::new(HashMap::new()),
            allowed_origins: Mutex::new(Vec::new()),
            allowed_hosts: Mutex::new(Vec::new()),
            spawn_fails: Mutex::new(HashMap::new()),
            runtime: Mutex::new(RuntimeLog::new()),
            spawning: Mutex::new(HashSet::new()),
            update_latest: Mutex::new(None),
            lan_ok: Mutex::new(None),
        })
    }

    /// Commande qui meurt aussitôt (code de sortie 1) — cross-platform.
    fn commande_exit_immediat() -> (String, Vec<String>) {
        #[cfg(windows)]
        {
            (
                "cmd.exe".to_string(),
                vec!["/C".to_string(), "exit".to_string(), "1".to_string()],
            )
        }
        #[cfg(not(windows))]
        {
            (
                "sh".to_string(),
                vec!["-c".to_string(), "exit 1".to_string()],
            )
        }
    }

    /// Attend (borné à 5 s) que le watcher publie la sortie du process lancé :
    /// `sh`/`cmd` mettent quelques ms à mourir — sans cette attente, le poll
    /// suivant peut lire exit=None (process vivant) et rater l'échec.
    fn attendre_sortie(state: &Arc<AppState>, id: &str) {
        for _ in 0..500 {
            let finie = match lockv(&state.children).get(id) {
                Some(h) => h.exit.lock().unwrap().is_some(),
                None => true,
            };
            if finie {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Commande qui vit une quinzaine de secondes (pour tester l'échéance
    /// avec un process encore vivant malgré la charge parallèle des tests)
    /// — cross-platform.
    fn commande_vit_quelques_secondes() -> (String, Vec<String>) {
        #[cfg(windows)]
        {
            (
                "cmd.exe".to_string(),
                vec![
                    "/C".to_string(),
                    "ping".to_string(),
                    "-n".to_string(),
                    "15".to_string(),
                    "127.0.0.1".to_string(),
                ],
            )
        }
        #[cfg(not(windows))]
        {
            (
                "sh".to_string(),
                vec!["-c".to_string(), "sleep 15".to_string()],
            )
        }
    }

    fn port_libre() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind 127.0.0.1:0")
            .local_addr()
            .expect("local_addr")
            .port()
    }

    fn runtime_de(state: &AppState, id: &str) -> crate::runtime::RuntimeState {
        lockv(&state.runtime).get(id).cloned().unwrap_or_default()
    }

    fn journal(state: &AppState) -> Vec<String> {
        lockv(&state.log).iter().map(|e| e.msg.clone()).collect()
    }

    /// Faux serveur MCP local (initialize valide pour toute requête) — sert
    /// aux tests d'UP réellement observé.
    fn fixture_mcp() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind fixture");
        let port = listener.local_addr().expect("local_addr fixture").port();
        std::thread::spawn(move || {
            let reponse =
                "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2024-11-05\",\"serverInfo\":{\"name\":\"test-mcp\",\"version\":\"1.0\"}}}";
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { return };
                let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                let mut buf = Vec::new();
                let mut tmp = [0u8; 8192];
                // lecture bornée : la requête de sonde tient dans quelques blocs
                for _ in 0..8 {
                    match s.read(&mut tmp) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                if !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    continue; // sonde de port (TCP nu) : rien à répondre
                }
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    reponse.len()
                );
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(reponse.as_bytes());
            }
        });
        port
    }

    #[test]
    fn spawn_sans_up_ne_remet_pas_le_compteur_a_zero() {
        // Régression audit 03 constat 3 : au spawn, plus aucun `start-ok` —
        // le compteur d'échecs ne peut plus être remis à zéro par un simple
        // lancement de process. Un marqueur de démarrage avec échéance est
        // posé à la place, et l'état reste « démarrage en cours ».
        let port = port_libre();
        let (prog, args) = commande_exit_immediat();
        let state = etat_test(vec![serveur_local("crash-x", port, Some((prog, args)))]);
        poll_one(&state, "crash-x");
        let rt = runtime_de(&state, "crash-x");
        assert_eq!(rt.fails, 0, "aucun échec au spawn : {rt:?}");
        assert!(
            rt.starting_until > crate::types::now_ts(),
            "marqueur de démarrage posé avec échéance : {rt:?}"
        );
        assert!(
            rt.events.iter().all(|e| e.kind != "start-ok"),
            "plus de start-ok au spawn : {:?}",
            rt.events
        );
        let servers = lockv(&state.servers);
        let srv = servers.iter().find(|s| s.id == "crash-x").unwrap();
        assert!(srv.pid.is_some(), "pid conservé pendant le démarrage");
        assert_eq!(srv.state, SState::Down, "pas de fausse remontée UP");
        assert_eq!(
            srv.last_error.as_deref(),
            Some("démarrage en cours…"),
            "motif d'attente posé : {:?}",
            srv.last_error
        );
    }

    #[test]
    fn mort_immediate_compte_un_echec_puis_abandon_a_cinq() {
        // Régression audit 03 constat 3 : un serveur qui sort aussitôt compte
        // UN échec par tentative (note_spawn_failure), le backoff croît et à
        // MAX_ATTEMPTS l'auto-restart est désactivé — la relance n'est plus
        // infinie.
        let port = port_libre();
        let (prog, args) = commande_exit_immediat();
        let state = etat_test(vec![serveur_local("crash-y", port, Some((prog, args)))]);
        let mut backoffs = Vec::new();
        for _ in 0..crate::runtime::MAX_ATTEMPTS {
            poll_one(&state, "crash-y"); // spawn (ou tentative) — marqueur posé
            attendre_sortie(&state, "crash-y"); // le watcher publie la sortie
            poll_one(&state, "crash-y"); // process sorti → échec compté
            let rt = runtime_de(&state, "crash-y");
            backoffs.push(rt.backoff_secs());
            // on neutralise le backoff pour enchaîner les tentatives du test
            lockv(&state.runtime)
                .entry("crash-y".to_string())
                .or_default()
                .last_attempt_ts = 0;
        }
        let rt = runtime_de(&state, "crash-y");
        assert_eq!(rt.fails, crate::runtime::MAX_ATTEMPTS, "fails = 5 : {rt:?}");
        for fen in backoffs.windows(2) {
            assert!(fen[1] > fen[0], "backoff croissant : {backoffs:?}");
        }
        assert_eq!(backoffs[0], crate::runtime::BASE_BACKOFF_SECS);
        assert_eq!(
            rt.events.iter().filter(|e| e.kind == "start-fail").count(),
            crate::runtime::MAX_ATTEMPTS as usize
        );
        assert!(
            rt.events.iter().all(|e| e.kind != "start-ok"),
            "aucun start-ok sur une série d'échecs : {:?}",
            rt.events
        );
        let servers = lockv(&state.servers);
        let srv = servers.iter().find(|s| s.id == "crash-y").unwrap();
        assert!(!srv.auto_restart, "auto-restart désactivé à 5 échecs");
        drop(servers);
        assert!(
            journal(&state)
                .iter()
                .any(|m| m.contains("auto-restart désactivé après 5 échecs")),
            "ligne de journal d'abandon claire : {:?}",
            journal(&state)
        );
        // APRÈS abandon : plus aucun spawn — le marqueur ne repose pas, le
        // compteur n'évolue plus (la relance infinie est corrigée).
        let enfants_avant = lockv(&state.children).len();
        poll_one(&state, "crash-y");
        poll_one(&state, "crash-y");
        let rt2 = runtime_de(&state, "crash-y");
        assert_eq!(
            rt2.fails,
            crate::runtime::MAX_ATTEMPTS,
            "plus d'échec compté après abandon"
        );
        assert_eq!(rt2.starting_until, 0, "aucun nouveau marqueur posé");
        assert_eq!(
            lockv(&state.children).len(),
            enfants_avant,
            "aucun spawn après abandon"
        );
        assert!(
            lockv(&state.spawning).is_empty(),
            "garde anti-doublon relâchée"
        );
    }

    #[test]
    fn echeance_depassee_compte_un_echec_meme_process_vivant() {
        // Un lanceur qui ne rend jamais la main (build interminable, process
        // bloqué) est déclaré en échec à l'échéance de la fenêtre de démarrage.
        let port = port_libre();
        let (prog, args) = commande_vit_quelques_secondes();
        let state = etat_test(vec![serveur_local("lent", port, None)]);
        let h = probe::spawn_process(&prog, &args, None, &[]).expect("spawn process vivant");
        lockv(&state.children).insert("lent".to_string(), h);
        {
            let mut rt = lockv(&state.runtime);
            let entry = rt.entry("lent".to_string()).or_default();
            entry.starting_until = crate::types::now_ts().saturating_sub(1);
        }
        poll_one(&state, "lent");
        let rt = runtime_de(&state, "lent");
        assert_eq!(rt.fails, 1, "échec compté à l'échéance : {rt:?}");
        assert_eq!(rt.starting_until, 0, "marqueur consommé");
        assert!(
            journal(&state)
                .iter()
                .any(|m| m.contains("échéance de démarrage")),
            "motif d'échéance dans le journal : {:?}",
            journal(&state)
        );
        let servers = lockv(&state.servers);
        let srv = servers.iter().find(|s| s.id == "lent").unwrap();
        assert!(srv.auto_restart, "un seul échec n'abandonne pas");
        drop(servers);
        // nettoyage : tue le process encore vivant
        // nettoyage : tue le process encore vivant (le watcher récoltera)
        let restant = lockv(&state.children).remove("lent");
        if let Some(h) = restant {
            probe::kill_tree(h.pid);
        }
    }

    #[test]
    fn up_observe_remet_le_compteur_a_zero() {
        // Le compteur ne retombe à zéro QUE sur un UP réellement observé :
        // start-ok est journalisé à cette occasion (plus au spawn).
        let port = fixture_mcp();
        let state = etat_test(vec![serveur_local("revient", port, None)]);
        {
            let mut rt = lockv(&state.runtime);
            let entry = rt.entry("revient".to_string()).or_default();
            entry.fails = 3;
            entry.starting_until = crate::types::now_ts() + 30;
        }
        poll_one(&state, "revient");
        let rt = runtime_de(&state, "revient");
        assert_eq!(rt.fails, 0, "UP observé → compteur remis à zéro : {rt:?}");
        assert_eq!(rt.starting_until, 0, "marqueur consommé");
        assert_eq!(rt.last_success_ts, crate::types::now_ts());
        assert!(
            rt.events.iter().any(|e| e.kind == "start-ok"),
            "start-ok journalisé à l'UP réel : {:?}",
            rt.events
        );
        let servers = lockv(&state.servers);
        let srv = servers.iter().find(|s| s.id == "revient").unwrap();
        assert_eq!(srv.state, SState::Up);
        assert_eq!(srv.last_error, None);
    }

    #[test]
    fn up_apres_echecs_emet_recovered() {
        // UP sans marqueur de démarrage mais après des échecs comptés :
        // événement `recovered` (compteur remis à zéro, redémarrage compté).
        let port = fixture_mcp();
        let state = etat_test(vec![serveur_local("retabli", port, None)]);
        {
            let mut rt = lockv(&state.runtime);
            let entry = rt.entry("retabli".to_string()).or_default();
            entry.fails = 3;
        }
        poll_one(&state, "retabli");
        let rt = runtime_de(&state, "retabli");
        assert_eq!(rt.fails, 0, "{rt:?}");
        assert_eq!(rt.restarts, 1, "recovered compte un redémarrage : {rt:?}");
        assert!(rt.events.iter().any(|e| e.kind == "recovered"));
    }
}
