// Découverte : scan de ports, auto-scan (netstat), scan réseau, dédoublonnage.
// (split v0.7.20 : manage.rs reste la racine — API stable via réexport)
use super::*;

/// Fenêtre accordée à un démarrage (spawn réussi → UP réellement observé).
/// Un lanceur qui construit avant d'écouter (bin/launch.cjs fait `npm run
/// build`) a ce délai ; passé l'échéance sans UP, le démarrage compte comme
/// un échec (note_spawn_failure). Vérifié aux CYCLES suivants, jamais en
/// bloquant le cycle (audit 03, constats 2 et 3).
const START_WINDOW_SECS: u64 = 30;

/// Durée de vie du cache `tools/list` pour un serveur déjà UP dont ni le nom
/// ni la version n'ont changé (P1 rang 2 : évite 455 `tools/list`/min au repos).
const TOOLS_REFRESH_TTL: std::time::Duration = std::time::Duration::from_secs(300);

fn tools_cache_slot(
) -> &'static std::sync::Mutex<std::collections::HashMap<(usize, String), std::time::Instant>> {
    static SLOT: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<(usize, String), std::time::Instant>>,
    > = std::sync::OnceLock::new();
    SLOT.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Invalide le cache `tools/list` pour tous les serveurs de cet `AppState`
/// (appelé sur un rafraîchissement manuel explicite).
pub fn invalidate_tools_cache(state: &Arc<AppState>) {
    let key_ptr = Arc::as_ptr(state) as usize;
    lockv(tools_cache_slot()).retain(|(ptr, _), _| *ptr != key_ptr);
}

fn tools_cache_fresh(state: &Arc<AppState>, id: &str) -> bool {
    let key = (Arc::as_ptr(state) as usize, id.to_string());
    lockv(tools_cache_slot())
        .get(&key)
        .is_some_and(|t| t.elapsed() < TOOLS_REFRESH_TTL)
}

fn tools_cache_mark(state: &Arc<AppState>, id: &str) {
    let key = (Arc::as_ptr(state) as usize, id.to_string());
    lockv(tools_cache_slot()).insert(key, std::time::Instant::now());
}

fn tools_cache_clear_one(state: &Arc<AppState>, id: &str) {
    let key = (Arc::as_ptr(state) as usize, id.to_string());
    lockv(tools_cache_slot()).remove(&key);
}

/// Sonde manuelle d'un serveur : force le rechargement complet (`tools/list`).
pub fn poll_one(state: &Arc<AppState>, id: &str) {
    poll_one_inner(state, id, true);
}

/// Sonde de fond (pool du poller) : réutilise `tools/list` tant que le serveur
/// reste UP, que ses `name`/`version` n'ont pas changé et que le TTL n'est pas échu.
pub(crate) fn poll_one_bg(state: &Arc<AppState>, id: &str) {
    poll_one_inner(state, id, false);
}

#[allow(unused_must_use)]
/// Serveurs réellement confirmés UP pendant cette session (évite les fausses
/// alertes de crash au boot depuis un `state-cache.json` périmé) `[M-14]`.
fn confirmed_up_set() -> &'static std::sync::Mutex<std::collections::HashSet<(usize, String)>> {
    static SET: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<(usize, String)>>> =
        std::sync::OnceLock::new();
    SET.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

fn mark_confirmed_up(state: &Arc<AppState>, id: &str) {
    let key = (Arc::as_ptr(state) as usize, id.to_string());
    lockv(confirmed_up_set()).insert(key);
}

fn take_confirmed_up(state: &Arc<AppState>, id: &str) -> bool {
    let key = (Arc::as_ptr(state) as usize, id.to_string());
    lockv(confirmed_up_set()).remove(&key)
}

fn poll_one_inner(state: &Arc<AppState>, id: &str, force_full: bool) {
    crate::auth::net::set_config_path(&state.config_path);
    let (transport, enabled, via_ssh, was_up, _has_tools, prev_name, prev_version) = {
        let servers = lockv(&state.servers);
        match servers.iter().find(|s| s.id == id) {
            Some(s) => (
                s.transport.clone(),
                s.enabled,
                s.via_ssh,
                s.state == SState::Up,
                !s.tools.is_empty(),
                s.server_name.clone(),
                s.version.clone(),
            ),
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
            if force_full
                && !via_ssh
                && host.as_deref().map(crate::is_loopback_bind).unwrap_or(true)
            {
                probe::begin_port_owner_cycle();
            }
            // MC-O02 : autoriser le fast-path `ProbeHint` même pour les
            // serveurs MCP valides à 0 outil (ex. resources/prompts uniquement).
            let can_reuse =
                !force_full && was_up && prev_name.is_some() && tools_cache_fresh(state, id);
            let hint = probe::ProbeHint {
                reuse_metadata_if_unchanged: can_reuse,
                prev_name: prev_name.as_deref(),
                prev_version: prev_version.as_deref(),
            };
            let ssh_host_cfg = if via_ssh {
                host.as_deref().and_then(|h| {
                    lockv(&state.settings)
                        .ssh_hosts
                        .iter()
                        .find(|x| x.ip == h)
                        .cloned()
                })
            } else {
                None
            };
            let mut res = if via_ssh {
                match (&host, &ssh_host_cfg) {
                    (Some(_), Some(sh)) => probe::ssh_probe_server_target(
                        &probe::SshTarget::from_host(sh),
                        port,
                        &path,
                        6,
                    ),
                    (Some(_), None) => probe::McpProbe::fail(
                        "machine SSH non enregistrée — réenregistre-la dans Machines".to_string(),
                        true,
                    ),
                    (None, _) => {
                        probe::McpProbe::fail("serveur via SSH sans hôte".to_string(), true)
                    }
                }
            } else {
                match &host {
                    // budget de sonde 8 s (lot A) : un serveur réellement lent
                    // mais vivant (réponse en ~6 s) doit rester classé UP —
                    // le port muet, lui, est coupé dès la 1re forme sans octet
                    // (probe_http, bail précoce) au lieu d'encaisser 3 timeout.
                    Some(h) => probe::probe_http_host_with(h, port, &path, 8000, &hint),
                    None => probe::probe_http_with(port, &path, 8000, &hint),
                }
            };
            // Auto-start : serveur HTTP LOCAL éteint OU serveur via_ssh enregistré
            // avec commande de lancement configurée (jamais pour un serveur LAN non-SSH,
            // ni pour un serveur déjà en écoute mais verrouillé 401).
            let auto_start_eligible = host.is_none() || (via_ssh && ssh_host_cfg.is_some());
            if !res.mcp_up && !res.locked && auto_start_eligible {
                // État du process lancé par nous, s'il existe : vivant ? code de sortie ?
                let (child_alive, exit_code) = if via_ssh {
                    (false, None)
                } else {
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
                let (echec_demarrage, ssh_still_starting) = {
                    let mut rt = lockv(&state.runtime);
                    let entry = rt.entry(id.to_string()).or_default();
                    let echeance = entry.starting_until;
                    if via_ssh {
                        if echeance != 0 && now >= echeance {
                            entry.starting_until = 0;
                            (true, false)
                        } else {
                            (false, echeance > now)
                        }
                    } else if echeance != 0 && (!child_alive || now >= echeance) {
                        entry.starting_until = 0;
                        (true, false)
                    } else {
                        (false, false)
                    }
                };
                if echec_demarrage {
                    // [H-12] : si le lanceur HTTP est toujours vivant après expiration
                    // de START_WINDOW_SECS sans jamais répondre MCP, le tuer pour ne
                    // pas accumuler de processus bloqués.
                    if !via_ssh && child_alive {
                        if let Some(h) = lockv(&state.children).remove(id) {
                            let same = match h.started {
                                Some(ts) => probe::process_creation(h.pid) == Some(ts),
                                None => true,
                            };
                            if same && h.pid != 0 {
                                probe::kill_tree(h.pid);
                            }
                        }
                    }
                    let motif = if child_alive || via_ssh {
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
                    && !ssh_still_starting
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
                        if let Some(ref sh) = ssh_host_cfg {
                            let os_hint = get_ssh_status(state, &sh.ip).and_then(|st| st.os);
                            let target = probe::SshTarget::from_host(sh);
                            match probe::ssh_spawn_remote(&target, os_hint.as_deref(), &cmd) {
                                Ok(pid_opt) => {
                                    lockv(&state.spawn_fails).remove(id);
                                    if let Some(pid) = pid_opt {
                                        let mut servers = lockv(&state.servers);
                                        if let Some(srv) = servers.iter_mut().find(|s| s.id == id) {
                                            srv.pid = Some(pid);
                                        }
                                    }
                                    add_log(
                                        state,
                                        "info",
                                        &format!(
                                            "[{}] ssh démarré ({} — re-sonde)",
                                            id,
                                            pid_opt
                                                .map(|p| format!("pid {p}"))
                                                .unwrap_or_else(|| "détaché".to_string())
                                        ),
                                    );
                                    {
                                        let mut rt = lockv(&state.runtime);
                                        rt.entry(id.to_string()).or_default().starting_until =
                                            crate::types::now_ts() + START_WINDOW_SECS;
                                    }
                                    std::thread::sleep(std::time::Duration::from_millis(400));
                                    let mut res2 =
                                        probe::ssh_probe_server_target(&target, port, &path, 3);
                                    if res2.mcp_up && res2.pid.is_none() {
                                        res2.pid = pid_opt;
                                    }
                                    res = res2;
                                }
                                Err(e) => {
                                    note_spawn_failure(state, id);
                                    add_log(
                                        state,
                                        "warn",
                                        &format!("[{}] démarrage SSH KO : {}", id, e),
                                    );
                                }
                            }
                        } else {
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
            let mut gw_changed = false;
            let mut incident: Option<(String, &'static str, String)> = None;
            let mut kill_orphan_child = false;
            {
                let mut servers = lockv(&state.servers);
                match servers.iter_mut().find(|s| s.id == id) {
                    Some(srv) if !srv.enabled => {
                        // [H-12] : désactivé pendant que la sonde tournait — ne pas
                        // écraser son état et nettoyer un éventuel enfant fraîchement lancé.
                        kill_orphan_child = true;
                    }
                    Some(srv) => {
                        if res.mcp_up {
                            let changed = srv.server_name != res.name || srv.state != SState::Up;
                            let tools_changed =
                                !res.metadata_reused && srv.tools.len() != res.tools.len();
                            if changed || tools_changed {
                                gw_changed = true;
                            }
                            srv.state = SState::Up;
                            mark_confirmed_up(state, id);
                            srv.bound_host = res.host.clone();
                            srv.server_name = res.name.clone();
                            srv.version = res.version.clone();
                            srv.protocol = res.protocol.clone();
                            if !res.metadata_reused {
                                srv.sse = res.sse;
                                srv.sse_endpoint = res.sse_endpoint;
                                srv.ws = res.ws;
                                srv.lang = res.lang.clone();
                                srv.framework = res.framework.clone();
                                srv.in_container = res.in_container;
                                srv.container_name = res.container_name.clone();
                                srv.tools = res.tools.clone();
                            }
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
                                let new_state = if res.port_open {
                                    SState::PortOnly
                                } else {
                                    SState::Down
                                };
                                if srv.state != new_state {
                                    gw_changed = true;
                                }
                                srv.state = new_state;
                                if res.locked {
                                    if res.host.is_some() {
                                        srv.bound_host = res.host.clone();
                                    }
                                    if res.framework.is_some() {
                                        srv.framework = res.framework.clone();
                                    }
                                    if res.lang.is_some() {
                                        srv.lang = res.lang.clone();
                                    }
                                    if res.in_container {
                                        srv.in_container = true;
                                        if res.container_name.is_some() {
                                            srv.container_name = res.container_name.clone();
                                        }
                                    }
                                }
                                srv.last_error = res.error.clone();
                                srv.latency_ms = None;
                                if !res.port_open && (host.is_none() || via_ssh) {
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
                                    if take_confirmed_up(state, id) {
                                        incident = Some((
                                            srv.label.clone(),
                                            st_lbl,
                                            res.error.clone().unwrap_or_else(|| "?".to_string()),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    None => {
                        kill_orphan_child = true;
                    }
                }
            }
            if kill_orphan_child {
                if let Some(h) = lockv(&state.children).remove(id) {
                    let same = match h.started {
                        Some(ts) => probe::process_creation(h.pid) == Some(ts),
                        None => true,
                    };
                    if same && h.pid != 0 {
                        probe::kill_tree(h.pid);
                    }
                }
                return;
            }
            if let Some((lbl, st_lbl, det)) = incident {
                notify_incident(state, &lbl, st_lbl, &det);
            }
            if res.mcp_up {
                if !res.metadata_reused {
                    tools_cache_mark(state, id);
                }
            } else {
                // Serveur tombé ou redémarré : force le rechargement de tools/list
                // dès son retour UP.
                tools_cache_clear_one(state, id);
            }
            if gw_changed {
                crate::gateway::notify_change();
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
                let mut gw_changed = false;
                let mut stdio_incident: Option<String> = None;
                {
                    let mut servers = lockv(&state.servers);
                    if let Some(srv) = servers.iter_mut().find(|s| s.id == id) {
                        if srv.state == SState::Up {
                            gw_changed = true;
                            if take_confirmed_up(state, id) {
                                stdio_incident = Some(srv.label.clone());
                            }
                        }
                        srv.state = SState::Down;
                        srv.pid = None;
                        if srv.last_error.is_none() {
                            srv.last_error = Some(detail.clone());
                        }
                    }
                }
                if let Some(lbl) = stdio_incident {
                    notify_incident(state, &lbl, "DOWN", &detail);
                }
                if gw_changed {
                    crate::gateway::notify_change();
                }
            }
            let (enabled, has_cmd) = {
                let servers = lockv(&state.servers);
                match servers.iter().find(|s| s.id == id) {
                    Some(s) => (s.enabled, s.start_cmd.is_some() && s.auto_restart),
                    None => return,
                }
            };
            if alive {
                mark_confirmed_up(state, id);
                let rpc_opt = if force_full {
                    lockv(&state.children).get(id).and_then(|h| h.rpc.clone())
                } else {
                    None
                };
                if let Some(rpc) = rpc_opt {
                    match probe::list_stdio_tools(&rpc, 10_000) {
                        Ok((tools, ms)) => {
                            let mut gw_changed = false;
                            {
                                let mut servers = lockv(&state.servers);
                                if let Some(srv) = servers.iter_mut().find(|s| s.id == id) {
                                    if srv.state != SState::Up || srv.tools.len() != tools.len() {
                                        gw_changed = true;
                                    }
                                    srv.state = SState::Up;
                                    if !tools.is_empty() || srv.tools.is_empty() {
                                        srv.tools = tools;
                                    }
                                    srv.latency_ms = Some(ms);
                                    srv.last_seen_ts = Some(crate::types::now_ts());
                                    srv.last_error = None;
                                }
                            }
                            if gw_changed {
                                crate::gateway::notify_change();
                            }
                        }
                        Err(e) => {
                            let pipe_dead = e.contains("stdout fermé")
                                || e.contains("écriture stdio")
                                || e.contains("flush stdio");
                            let mut gw_changed = false;
                            if pipe_dead {
                                lockv(&state.children).remove(id);
                                let mut servers = lockv(&state.servers);
                                if let Some(srv) = servers.iter_mut().find(|s| s.id == id) {
                                    if srv.state == SState::Up {
                                        gw_changed = true;
                                    }
                                    srv.state = SState::Down;
                                    srv.pid = None;
                                    srv.last_error = Some(e.clone());
                                }
                            } else {
                                let mut servers = lockv(&state.servers);
                                if let Some(srv) = servers.iter_mut().find(|s| s.id == id) {
                                    if srv.state != SState::Up {
                                        gw_changed = true;
                                    }
                                    srv.state = SState::Up;
                                    srv.last_seen_ts = Some(crate::types::now_ts());
                                }
                            }
                            if gw_changed {
                                crate::gateway::notify_change();
                            }
                        }
                    }
                } else {
                    let mut gw_changed = false;
                    {
                        let mut servers = lockv(&state.servers);
                        if let Some(srv) = servers.iter_mut().find(|s| s.id == id) {
                            if srv.state != SState::Up {
                                gw_changed = true;
                            }
                            srv.state = SState::Up;
                            srv.last_seen_ts = Some(crate::types::now_ts());
                        }
                    }
                    if gw_changed {
                        crate::gateway::notify_change();
                    }
                }
            } else if enabled && has_cmd {
                let now = crate::types::now_ts();
                let backoff_ok = if force_full {
                    true
                } else {
                    let mut rt = lockv(&state.runtime);
                    rt.entry(id.to_string()).or_default().can_attempt(now)
                };
                if !backoff_ok {
                    return;
                }
                // (re)démarrage auto avec handshake complet — même garde anti-doublon
                if !lockv(&state.spawning).insert(id.to_string()) {
                    return;
                }
                let cmd = {
                    let mut servers = lockv(&state.servers);
                    if let Some(srv) = servers.iter_mut().find(|s| s.id == id) {
                        srv.last_error = Some("démarrage en cours…".to_string());
                        srv.start_cmd.clone()
                    } else {
                        None
                    }
                };
                if let Some(cmd) = cmd {
                    let timeout_ms = stdio_start_timeout_ms(&cmd.program, &cmd.args);
                    match probe::probe_stdio(
                        &cmd.program,
                        &cmd.args,
                        cmd.workdir.as_deref(),
                        timeout_ms,
                        true,
                        &cmd.env,
                    ) {
                        Ok((p, handle)) => {
                            if let Some(h) = handle {
                                lockv(&state.children).insert(id.to_string(), h);
                            }
                            lockv(&state.spawn_fails).remove(id);
                            let mut gw_changed = false;
                            {
                                let mut servers = lockv(&state.servers);
                                if let Some(srv) = servers.iter_mut().find(|s| s.id == id) {
                                    if srv.state != SState::Up || srv.tools.len() != p.tools.len() {
                                        gw_changed = true;
                                    }
                                    srv.state = SState::Up;
                                    srv.pid = p.pid;
                                    srv.server_name = p.name.clone();
                                    srv.version = p.version.clone();
                                    srv.protocol = p.protocol.clone();
                                    srv.sse = p.sse;
                                    srv.sse_endpoint = p.sse_endpoint;
                                    srv.lang = p.lang.clone();
                                    srv.framework = p.framework.clone();
                                    srv.in_container = p.in_container;
                                    srv.container_name = p.container_name.clone();
                                    srv.tools = p.tools.clone();
                                    srv.latency_ms = Some(p.latency_ms);
                                    srv.last_seen_ts = Some(crate::types::now_ts());
                                    srv.last_error = None;
                                }
                            }
                            if gw_changed {
                                crate::gateway::notify_change();
                            }
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
                            let mut gw_changed = false;
                            {
                                let mut servers = lockv(&state.servers);
                                if let Some(srv) = servers.iter_mut().find(|s| s.id == id) {
                                    if srv.state == SState::Up {
                                        gw_changed = true;
                                    }
                                    srv.state = SState::Down;
                                    srv.last_error = Some(e.clone());
                                    srv.pid = None;
                                }
                            }
                            if gw_changed {
                                crate::gateway::notify_change();
                            }
                            note_spawn_failure(state, id);
                            add_log(state, "warn", &format!("[{}] stdio KO : {}", id, e));
                        }
                    }
                }
                // libère la garde anti-doublon quoi qu'il arrive
                lockv(&state.spawning).remove(id);
            } else {
                let mut gw_changed = false;
                {
                    let mut servers = lockv(&state.servers);
                    if let Some(srv) = servers.iter_mut().find(|s| s.id == id) {
                        if srv.state == SState::Up {
                            gw_changed = true;
                        }
                        srv.state = SState::Down;
                    }
                }
                if gw_changed {
                    crate::gateway::notify_change();
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
/// Découverte locale DÉSACTIVÉE par l'environnement (`MCC_NO_LOCAL_DISCOVERY=1`).
/// Réservé aux instances de test : sans cela, un moteur de test importe les
/// serveurs réels de la machine (scan local au boot + scan réseau à +30 s).
/// Délai du handshake `initialize` au démarrage d'un serveur stdio.
///
/// Un lanceur de paquets (npx, uvx…) télécharge le paquet au PREMIER
/// lancement : couper ce téléchargement à 45 s laissait un cache npx à moitié
/// installé (ex. mcp-mermaid → playwright-core sans index.js), et chaque
/// démarrage suivant plantait. D'où :
/// - npx avec paquet déjà en cache → 60 s (démarrage normal) ;
/// - uvx avec paquet déjà en cache → 90 s (environnement uv prêt) ;
/// - npx/uvx sans cache ou depuis git → 5 min (installation complète) ;
/// - bunx/pnpx/dlx → 3 min ;
/// - binaire direct → 30 s.
///
/// Un process qui meurt est signalé tout de suite (probe_stdio), donc un long
/// délai ne fait attendre que les serveurs réellement bloqués.
pub(super) fn stdio_start_timeout_ms(program: &str, args: &[String]) -> u64 {
    let stem = std::path::Path::new(program)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(program)
        .to_ascii_lowercase();
    match stem.as_str() {
        "npx" => match npx_package_name(args) {
            Some(pkg)
                if !pkg.starts_with("github:")
                    && !pkg.starts_with("git+")
                    && !pkg.starts_with("http://")
                    && !pkg.starts_with("https://")
                    && npx_package_cached(&npx_cache_dirs(), &pkg) =>
            {
                60_000
            }
            _ => 300_000,
        },
        "uvx" | "uv" => {
            if args.iter().any(|a| a.contains("git+")) {
                300_000
            } else {
                match uv_package_name(args) {
                    Some(pkg) if uv_package_cached(&uv_cache_dirs(), &pkg) => 90_000,
                    _ => 300_000,
                }
            }
        }
        "bunx" | "pnpx" | "dlx" => 180_000,
        _ => 30_000,
    }
}

/// Normalise un nom de paquet Python selon PEP 503 (`_` / `.` / `-` -> `-`, minuscules).
fn normalize_pep503(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_dash = false;
    for ch in name.chars() {
        if matches!(ch, '-' | '_' | '.') {
            if !prev_dash && !out.is_empty() {
                out.push('-');
                prev_dash = true;
            }
        } else {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Nom du paquet PyPI lancé par uvx (`--from pkg@latest bin` → `Some("pkg")`).
pub(super) fn uv_package_name(args: &[String]) -> Option<String> {
    let mut it = args.iter();
    let mut spec = None;
    while let Some(a) = it.next() {
        if a == "--from" {
            spec = it.next().cloned();
            break;
        }
        if let Some(v) = a.strip_prefix("--from=") {
            spec = Some(v.to_string());
            break;
        }
        if matches!(
            a.as_str(),
            "--python"
                | "-p"
                | "--with"
                | "--with-editable"
                | "--with-requirements"
                | "--index-url"
                | "--default-index"
                | "--extra-index-url"
                | "--find-links"
                | "--python-preference"
                | "--cache-dir"
                | "-C"
                | "--directory"
        ) {
            let _ = it.next();
            continue;
        }
        if !a.starts_with('-') {
            spec = Some(a.clone());
            break;
        }
    }
    let spec = spec?;
    if spec.contains("git+") || spec.contains("://") {
        return None;
    }
    let base = spec
        .split(['@', '=', '<', '>', '!', '~', '['])
        .next()
        .unwrap_or("")
        .trim();
    let norm = normalize_pep503(base);
    (!norm.is_empty()).then_some(norm)
}

/// Dossiers de cache uv candidats (`UV_CACHE_DIR`, `LOCALAPPDATA/uv/cache`, `.cache/uv`).
fn uv_cache_dirs() -> Vec<std::path::PathBuf> {
    let mut v = Vec::new();
    if let Ok(c) = std::env::var("UV_CACHE_DIR") {
        if !c.trim().is_empty() {
            v.push(std::path::PathBuf::from(c));
        }
    }
    if let Ok(l) = std::env::var("LOCALAPPDATA") {
        v.push(std::path::PathBuf::from(l).join("uv").join("cache"));
    }
    if let Ok(x) = std::env::var("XDG_CACHE_HOME") {
        v.push(std::path::PathBuf::from(x).join("uv"));
    }
    if let Some(h) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        v.push(std::path::PathBuf::from(h).join(".cache").join("uv"));
    }
    v
}

/// Le paquet PyPI est-il déjà présent dans un des caches `wheels-v*/pypi/<pkg>` d'uv ?
pub(super) fn uv_package_cached(dirs: &[std::path::PathBuf], pkg: &str) -> bool {
    let norm = normalize_pep503(pkg);
    if norm.is_empty() {
        return false;
    }
    dirs.iter().any(|d| {
        std::fs::read_dir(d)
            .map(|rd| {
                rd.flatten().any(|e| {
                    let name = e.file_name();
                    let name = name.to_string_lossy();
                    name.starts_with("wheels-") && e.path().join("pypi").join(&norm).is_dir()
                })
            })
            .unwrap_or(false)
    })
}

/// Nom du paquet lancé par npx, sans version (`@scope/nom@1.2` → `@scope/nom`).
pub(super) fn npx_package_name(args: &[String]) -> Option<String> {
    let mut it = args.iter();
    let mut spec = None;
    while let Some(a) = it.next() {
        if a == "-p" || a == "--package" {
            spec = it.next().cloned();
            break;
        }
        if let Some(v) = a.strip_prefix("--package=") {
            spec = Some(v.to_string());
            break;
        }
        if !a.starts_with('-') {
            spec = Some(a.clone());
            break;
        }
    }
    let spec = spec?;
    // la version suit le DERNIER « @ » qui n'est pas en tête (portée @scope)
    let nom = match spec.rfind('@').filter(|&i| i > 0) {
        Some(i) => spec[..i].to_string(),
        None => spec,
    };
    (!nom.is_empty()).then_some(nom)
}

/// Dossiers `_npx` candidats (variable npm_config_cache, sinon défaut OS).
fn npx_cache_dirs() -> Vec<std::path::PathBuf> {
    let mut v = Vec::new();
    if let Ok(c) = std::env::var("npm_config_cache") {
        v.push(std::path::PathBuf::from(c).join("_npx"));
    }
    if let Ok(l) = std::env::var("LOCALAPPDATA") {
        v.push(std::path::PathBuf::from(l).join("npm-cache").join("_npx"));
    }
    if let Some(h) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        v.push(std::path::PathBuf::from(h).join(".npm").join("_npx"));
    }
    v
}

/// Le paquet est-il déjà installé dans un des caches npx ?
pub(super) fn npx_package_cached(dirs: &[std::path::PathBuf], pkg: &str) -> bool {
    let rel: std::path::PathBuf = pkg.split('/').collect();
    dirs.iter().any(|d| {
        std::fs::read_dir(d)
            .map(|rd| {
                rd.flatten().any(|e| {
                    e.path()
                        .join("node_modules")
                        .join(&rel)
                        .join("package.json")
                        .is_file()
                })
            })
            .unwrap_or(false)
    })
}

/// Résultat de la purge des fichiers téléchargés pour un paquet de la Boutique.
#[derive(Default, Debug, Clone)]
pub(super) struct PurgeReport {
    pub removed_paths: Vec<String>,
    pub uv_cache_cleaned: bool,
    pub warnings: Vec<String>,
}

/// Clé canonique `(lanceur, paquet)` pour éviter de purger un cache partagé
/// avec un autre serveur configuré.
pub(super) fn store_package_key(program: &str, args: &[String]) -> Option<(String, String)> {
    let stem = std::path::Path::new(program)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(program)
        .to_ascii_lowercase();
    match stem.as_str() {
        "npx" => npx_package_name(args).map(|p| ("npx".to_string(), p)),
        "uvx" | "uv" => uv_purge_targets(args)
            .into_iter()
            .next()
            .map(|p| ("uvx".to_string(), p)),
        _ => None,
    }
}

/// Cibles normalisées (PEP 503) pour nettoyer les caches `uv` d'un serveur `uvx`.
pub(super) fn uv_purge_targets(args: &[String]) -> Vec<String> {
    if let Some(norm) = uv_package_name(args) {
        return vec![norm];
    }
    let mut out: Vec<String> = Vec::new();
    let mut it = args.iter();
    let mut from_spec: Option<String> = None;
    let mut bin_name: Option<String> = None;
    while let Some(a) = it.next() {
        if a == "--from" {
            from_spec = it.next().cloned();
            continue;
        }
        if let Some(v) = a.strip_prefix("--from=") {
            from_spec = Some(v.to_string());
            continue;
        }
        if matches!(
            a.as_str(),
            "--python"
                | "-p"
                | "--with"
                | "--with-editable"
                | "--with-requirements"
                | "--index-url"
                | "--default-index"
                | "--extra-index-url"
                | "--find-links"
                | "--python-preference"
                | "--cache-dir"
                | "-C"
                | "--directory"
        ) {
            let _ = it.next();
            continue;
        }
        if !a.starts_with('-') && bin_name.is_none() {
            bin_name = Some(a.clone());
        }
    }
    if let Some(spec) = from_spec {
        let trimmed = spec.split(['#', '?']).next().unwrap_or(&spec);
        let seg = trimmed.rsplit('/').next().unwrap_or(trimmed);
        let seg = seg.strip_suffix(".git").unwrap_or(seg);
        let seg = seg.split('@').next().unwrap_or(seg).trim();
        let norm = normalize_pep503(seg);
        if !norm.is_empty() && !out.contains(&norm) {
            out.push(norm);
        }
    }
    if let Some(b) = bin_name {
        let base = b
            .split(['@', '=', '<', '>', '!', '~', '['])
            .next()
            .unwrap_or("")
            .trim();
        let norm = normalize_pep503(base);
        if !norm.is_empty() && !out.contains(&norm) {
            out.push(norm);
        }
    }
    out
}

fn npx_spec_pkg_name(spec: &str) -> String {
    if spec.starts_with("github:") || spec.contains("://") {
        return spec.to_string();
    }
    match spec.rfind('@').filter(|&i| i > 0) {
        Some(i) => spec[..i].to_string(),
        None => spec.to_string(),
    }
}

fn npx_hash_dir_matches_pkg(hash_dir: &std::path::Path, pkg: &str) -> bool {
    let pkg_json_path = hash_dir.join("package.json");
    let parsed: Option<Value> = std::fs::read_to_string(&pkg_json_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    let mut deps_len = 0usize;
    if let Some(ref v) = parsed {
        if let Some(deps) = v.get("dependencies").and_then(|d| d.as_object()) {
            deps_len = deps.len();
            if deps_len > 1 {
                return false;
            }
            if deps.contains_key(pkg) {
                return true;
            }
            if pkg.starts_with("github:") {
                let gh_slug = pkg
                    .trim_start_matches("github:")
                    .split('#')
                    .next()
                    .unwrap_or("");
                if deps.values().any(|val| {
                    val.as_str().is_some_and(|s| {
                        s == pkg
                            || s.starts_with(pkg)
                            || (!gh_slug.is_empty() && s.contains(gh_slug))
                    })
                }) {
                    return true;
                }
            }
        }
        if let Some(arr) = v.pointer("/_npx/packages").and_then(|a| a.as_array()) {
            if arr.len() == 1 && arr[0].as_str().map(npx_spec_pkg_name).as_deref() == Some(pkg) {
                return true;
            }
        }
    }
    if !pkg.starts_with("github:") && !pkg.contains("://") && (parsed.is_none() || deps_len == 0) {
        let rel: std::path::PathBuf = pkg.split('/').collect();
        if hash_dir.join("node_modules").join(rel).is_dir() {
            return true;
        }
    }
    false
}

fn clear_readonly_recursive(path: &std::path::Path) {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return;
        }
        let mut perms = meta.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = perms.mode();
            let add = if meta.is_dir() { 0o700 } else { 0o600 };
            if mode & add != add {
                perms.set_mode(mode | add);
                let _ = std::fs::set_permissions(path, perms);
            }
        }
        #[cfg(not(unix))]
        {
            if perms.readonly() {
                #[allow(clippy::permissions_set_readonly_false)]
                perms.set_readonly(false);
                let _ = std::fs::set_permissions(path, perms);
            }
        }
        if meta.is_dir() {
            if let Ok(rd) = std::fs::read_dir(path) {
                for entry in rd.flatten() {
                    clear_readonly_recursive(&entry.path());
                }
            }
        }
    }
}

fn remove_dir_all_retry(path: &std::path::Path) -> std::io::Result<()> {
    for attempt in 0..5 {
        match std::fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                if e.kind() == std::io::ErrorKind::PermissionDenied {
                    clear_readonly_recursive(path);
                }
                if attempt == 4 {
                    return Err(e);
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
        }
    }
    Ok(())
}

/// Purge les caches `npx`/`uvx` d'un paquet sur disque dans les répertoires fournis.
pub(super) fn purge_store_package_files_in_dirs(
    program: &str,
    args: &[String],
    npx_dirs: &[std::path::PathBuf],
    uv_dirs: &[std::path::PathBuf],
    run_uv_cli: bool,
) -> PurgeReport {
    let mut report = PurgeReport::default();
    let stem = std::path::Path::new(program)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(program)
        .to_ascii_lowercase();
    match stem.as_str() {
        "npx" => {
            if let Some(pkg) = npx_package_name(args) {
                for d in npx_dirs {
                    let Ok(rd) = std::fs::read_dir(d) else {
                        continue;
                    };
                    for entry in rd.flatten() {
                        let p = entry.path();
                        let Ok(meta) = std::fs::symlink_metadata(&p) else {
                            continue;
                        };
                        if meta.file_type().is_symlink() || !meta.file_type().is_dir() {
                            continue;
                        }
                        if npx_hash_dir_matches_pkg(&p, &pkg) {
                            match remove_dir_all_retry(&p) {
                                Ok(()) => {
                                    report.removed_paths.push(p.to_string_lossy().to_string())
                                }
                                Err(e) => report.warnings.push(format!("{}: {e}", p.display())),
                            }
                        }
                    }
                }
            }
        }
        "uvx" | "uv" => {
            let targets = uv_purge_targets(args);
            if run_uv_cli && !targets.is_empty() {
                let mut cmd = Command::new("uv");
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    cmd.creation_flags(0x0800_0000);
                }
                cmd.args(["cache", "clean", "--force"])
                    .args(&targets)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                if let Ok(st) = cmd.status() {
                    if st.success() {
                        report.uv_cache_cleaned = true;
                    }
                }
            }
            if !targets.is_empty() {
                for d in uv_dirs {
                    let Ok(rd) = std::fs::read_dir(d) else {
                        continue;
                    };
                    for entry in rd.flatten() {
                        let bucket = entry.path();
                        let Ok(meta) = std::fs::symlink_metadata(&bucket) else {
                            continue;
                        };
                        if meta.file_type().is_symlink() || !meta.file_type().is_dir() {
                            continue;
                        }
                        let bname = entry.file_name();
                        let bname = bname.to_string_lossy();
                        let pypi = bucket.join("pypi");
                        if bname.starts_with("wheels-") || bname.starts_with("sdists-") {
                            for norm in &targets {
                                let pkg_dir = pypi.join(norm);
                                if std::fs::symlink_metadata(&pkg_dir)
                                    .map(|m| !m.file_type().is_symlink() && m.file_type().is_dir())
                                    .unwrap_or(false)
                                {
                                    match remove_dir_all_retry(&pkg_dir) {
                                        Ok(()) => report
                                            .removed_paths
                                            .push(pkg_dir.to_string_lossy().to_string()),
                                        Err(e) => report
                                            .warnings
                                            .push(format!("{}: {e}", pkg_dir.display())),
                                    }
                                }
                            }
                        } else if bname.starts_with("simple-") {
                            for norm in &targets {
                                let rkyv = pypi.join(format!("{norm}.rkyv"));
                                if std::fs::symlink_metadata(&rkyv)
                                    .map(|m| !m.file_type().is_symlink() && m.file_type().is_file())
                                    .unwrap_or(false)
                                {
                                    match std::fs::remove_file(&rkyv) {
                                        Ok(()) => report
                                            .removed_paths
                                            .push(rkyv.to_string_lossy().to_string()),
                                        Err(e) => {
                                            report.warnings.push(format!("{}: {e}", rkyv.display()))
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
    report
}

/// Purge les fichiers locaux du paquet lancé par un item de la Boutique.
pub(super) fn purge_store_package_files(program: &str, args: &[String]) -> PurgeReport {
    purge_store_package_files_in_dirs(program, args, &npx_cache_dirs(), &uv_cache_dirs(), true)
}

pub fn local_discovery_disabled() -> bool {
    std::env::var("MCC_NO_LOCAL_DISCOVERY")
        .map(|v| v.trim() == "1")
        .unwrap_or(false)
}

/// Ce port doit-il être sondé par la découverte locale ?
/// `excluded` gagne toujours ; `only` vide = tous les ports.
pub(super) fn discovery_port_allowed(port: u16, only: &[u16], excluded: &[u16]) -> bool {
    !excluded.contains(&port) && (only.is_empty() || only.contains(&port))
}

/// Réglages de découverte lus en une fois (only, excluded).
fn discovery_ports(state: &Arc<AppState>) -> (Vec<u16>, Vec<u16>) {
    let s = lockv(&state.settings);
    (
        s.discovery_ports_only.clone(),
        s.discovery_ports_excluded.clone(),
    )
}

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
    if local_discovery_disabled() {
        add_log(
            state,
            "info",
            "scan local ignoré : MCC_NO_LOCAL_DISCOVERY=1 (instance de test)",
        );
        return 0;
    }
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
    let (seulement, exclus) = discovery_ports(state);
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
        .filter(|(p, _)| {
            // « ports exclus » vaut aussi pour une plage explicite ; « ports à
            // sonder » ne s'applique qu'au balayage des réglages (range=None),
            // sinon une plage explicite ne trouverait jamais rien.
            !exclus.contains(p)
                && (range.is_some() || seulement.is_empty() || seulement.contains(p))
        })
        .collect();
    let (mtx, mrx) = std::sync::mpsc::channel::<(u16, String, &'static str, probe::McpProbe)>();
    if !candidats.is_empty() {
        probe::begin_port_owner_cycle();
        let workers = SCAN_WORKERS.min(candidats.len()).max(1);
        let next = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..workers {
                let mtx = mtx.clone();
                let next = &next;
                let candidats = &candidats;
                scope.spawn(move || loop {
                    let idx = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((port, host)) = candidats.get(idx) else {
                        break;
                    };
                    if let Some((path, p)) = probe::probe_mcp_any_path(*port, 4000) {
                        let _ = mtx.send((*port, host.clone(), path, p));
                    }
                });
            }
        });
    }
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
                state: if p.mcp_up {
                    SState::Up
                } else {
                    SState::PortOnly
                },
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
                in_container: p.in_container,
                container_name: p.container_name.clone(),
                via_ssh: false,
                tools: p.tools,
                latency_ms: if p.mcp_up { Some(p.latency_ms) } else { None },
                last_seen_ts: Some(crate::types::now_ts()),
                last_error: p.error.clone(),
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
    if found > 0 {
        crate::gateway::notify_change();
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
    if local_discovery_disabled() {
        add_log(
            state,
            "info",
            "scan local ignoré : MCC_NO_LOCAL_DISCOVERY=1 (instance de test)",
        );
        return;
    }
    let (seulement, exclus) = discovery_ports(state);
    let ports: Vec<u16> = probe::local_listening_ports()
        .into_iter()
        .filter(|p| discovery_port_allowed(*p, &seulement, &exclus))
        .collect();
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
    // Sondes PARALLÈLES sur pool borné (S-10 : plafond SCAN_LISTENING_WORKERS
    // au lieu d'un thread par port en écoute). Chaque port est sondé sur les
    // chemins MCP usuels (/mcp, /, /api/mcp).
    const SCAN_LISTENING_WORKERS: usize = 16;
    let results: Vec<(u16, Option<(&'static str, probe::McpProbe)>)> = if candidates.is_empty() {
        Vec::new()
    } else {
        probe::begin_port_owner_cycle();
        let workers = SCAN_LISTENING_WORKERS.min(candidates.len()).max(1);
        let next = std::sync::atomic::AtomicUsize::new(0);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            for _ in 0..workers {
                let tx = tx.clone();
                let next = &next;
                let candidates = &candidates;
                scope.spawn(move || loop {
                    let idx = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(&port) = candidates.get(idx) else {
                        break;
                    };
                    let _ = tx.send((port, probe::probe_mcp_any_path(port, 4000)));
                });
            }
        });
        drop(tx);
        let mut v: Vec<_> = rx.into_iter().collect();
        v.sort_by_key(|(p, _)| *p);
        v
    };
    for (port, probe_res) in results {
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
            state: if p.mcp_up {
                SState::Up
            } else {
                SState::PortOnly
            },
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
            in_container: p.in_container,
            container_name: p.container_name.clone(),
            via_ssh: false,
            tools: p.tools,
            latency_ms: if p.mcp_up { Some(p.latency_ms) } else { None },
            last_seen_ts: Some(crate::types::now_ts()),
            last_error: p.error.clone(),
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
    if found > 0 {
        crate::gateway::notify_change();
    }
    add_log(
        state,
        "info",
        &format!("scan local terminé : {} nouveau(x) serveur(s) MCP", found),
    );
}

// ---------- scan réseau local (MCP distants) ----------

/// Garde anti-réentrance sur `scan_network` (P-5 / P1 rang 4) : empêche deux
/// scans réseau concurrents de saturer le LAN et la table de serveurs.
static NETWORK_SCAN_RUNNING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub(super) struct NetworkScanGuard;

impl NetworkScanGuard {
    pub(super) fn try_acquire() -> Option<Self> {
        NETWORK_SCAN_RUNNING
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .ok()
            .map(|_| NetworkScanGuard)
    }
}

impl Drop for NetworkScanGuard {
    fn drop(&mut self) {
        NETWORK_SCAN_RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

#[allow(unused_must_use)]
pub fn scan_network(state: &Arc<AppState>, range: Option<(u16, u16)>) {
    let Some(_guard) = NetworkScanGuard::try_acquire() else {
        add_log(state, "info", "scan réseau déjà en cours — appel ignoré");
        return;
    };
    if local_discovery_disabled() {
        add_log(
            state,
            "info",
            "scan réseau ignoré : MCC_NO_LOCAL_DISCOVERY=1 (instance de test)",
        );
        return;
    }
    let (from, to) = match range {
        Some(r) => r,
        None => {
            let s = lockv(&state.settings);
            (s.scan_from, s.scan_to)
        }
    };
    if from == 0 || to < from || (to as u32 - from as u32) >= SCAN_MAX_PORTS as u32 {
        add_log(
            state,
            "warn",
            &format!(
                "scan réseau refusé : plage {}..{} invalide (max {} ports)",
                from, to, SCAN_MAX_PORTS
            ),
        );
        return;
    }
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
        let mut c = Command::new(probe::system_tool("ipconfig"));
        probe::hidden(&mut c).output()
    } else {
        let mut c = Command::new(probe::system_tool("hostname"));
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

    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
    let found = AtomicU32::new(0);
    let tested = AtomicU32::new(0);
    let targets: Vec<String> = (1u16..=254)
        .map(|last| format!("{}.{}", prefix, last))
        .filter(|ip| ip != self_ip)
        .collect();
    let workers = SCAN_WORKERS.min(targets.len()).max(1);
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let ports = &ports;
            let known = &known_hosts_ports;
            let targets = &targets;
            let found = &found;
            let tested = &tested;
            let next = &next;
            scope.spawn(move || loop {
                let idx = next.fetch_add(1, Ordering::Relaxed);
                let Some(ip) = targets.get(idx) else {
                    break;
                };
                let mut hit = false;
                for port in ports {
                    if !probe::port_open(ip, *port, std::time::Duration::from_millis(400)) {
                        continue;
                    }
                    hit = true;
                    if known
                        .iter()
                        .any(|(h, p)| h.as_deref() == Some(ip.as_str()) && p == port)
                    {
                        continue;
                    }
                    let p = probe::probe_http_host(ip, *port, "/mcp", 3000);
                    if p.mcp_up {
                        found.fetch_add(1, Ordering::Relaxed);
                        let label = p.name.clone().unwrap_or_else(|| format!("{}:{}", ip, port));
                        let mut servers = lockv(&state.servers);
                        // 1. Déjà connu sur ce même hôte+port (manuel, seed ou découvert) :
                        //    mise à jour des métadonnées sans recréer ni écraser l'origine.
                        if let Some(s) = servers.iter_mut().find(|s| {
                            matches!(
                                &s.transport,
                                Transport::Http { host: Some(h), port: sp, .. } if h == ip && sp == port
                            )
                        }) {
                            if p.name.is_some() {
                                s.server_name = p.name.clone();
                            }
                            if p.version.is_some() {
                                s.version = p.version.clone();
                            }
                            s.framework = p.framework.clone();
                            s.lang = p.lang.clone();
                            s.in_container = p.in_container;
                            s.container_name = p.container_name.clone();
                            s.tools = p.tools;
                            s.latency_ms = Some(p.latency_ms);
                            s.last_seen_ts = Some(crate::types::now_ts());
                            drop(servers);
                            continue;
                        }
                        // 2. Remplacement d'un port fantôme : UNIQUEMENT parmi les entrées
                        //    `Origin::Discovered` portant un `server_name` non vide, et en
                        //    conservant l'indice RÉEL dans `servers` (jamais un indice filtré
                        //    qui supprimerait un serveur Stdio ou Manuel par décalage).
                        let replaceable: Vec<_> = servers
                                .iter()
                                .enumerate()
                                .filter_map(|(real_idx, s)| {
                                    if s.origin != Origin::Discovered
                                        || !s
                                            .server_name
                                            .as_deref()
                                            .is_some_and(|n| !n.trim().is_empty())
                                    {
                                        return None;
                                    }
                                    match &s.transport {
                                        Transport::Http { host, port, .. } => Some((
                                            real_idx,
                                            (s.server_name.clone(), host.clone(), *port),
                                        )),
                                        _ => None,
                                    }
                                })
                                .collect();
                        let known_srv: Vec<(Option<String>, Option<String>, u16)> =
                            replaceable.iter().map(|(_, entry)| entry.clone()).collect();
                        let decision = crate::validate::dedup_decide(
                            &known_srv,
                            p.name.as_deref(),
                            Some(ip.as_str()),
                            *port,
                        );
                        let mut replace_log: Option<String> = None;
                        let target_id = match decision {
                            crate::validate::DedupDecision::Duplicate => {
                                drop(servers);
                                continue;
                            }
                            crate::validate::DedupDecision::Replace(sub_idx) => {
                                let (real_idx, (_, _, old_port)) = replaceable[sub_idx];
                                let old = servers.remove(real_idx);
                                replace_log = Some(format!(
                                    "[réseau] {} a changé de port ({} → {}) — entrée fantôme remplacée",
                                    label, old_port, port
                                ));
                                old.id
                            }
                            crate::validate::DedupDecision::Add => format!("net-{}.{}", ip, port),
                        };
                        let name_log = p.name.clone();
                        let ver_log = p.version.clone();
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
                            in_container: p.in_container,
                            container_name: p.container_name.clone(),
                            via_ssh: false,
                            tools: p.tools,
                            latency_ms: Some(p.latency_ms),
                            last_seen_ts: Some(crate::types::now_ts()),
                            last_error: None,
                            start_cmd: None,
                            tools_disabled: vec![],
                        });
                        drop(servers);
                        crate::gateway::notify_change();
                        if let Some(msg) = replace_log {
                            add_log(state, "info", &msg);
                        }
                        add_log(
                            state,
                            "info",
                            &format!(
                                "[réseau] serveur MCP distant découvert : {}:{} — {} v{}",
                                ip,
                                port,
                                name_log.as_deref().unwrap_or("?"),
                                ver_log.as_deref().unwrap_or("?")
                            ),
                        );
                    }
                }
                if hit {
                    tested.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    });
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
            in_container: false,
            container_name: None,
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
    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn npx_package_name_retire_options_et_version() {
        assert_eq!(
            npx_package_name(&v(&["-y", "mcp-mermaid"])).as_deref(),
            Some("mcp-mermaid")
        );
        assert_eq!(
            npx_package_name(&v(&["-y", "@modelcontextprotocol/server-gdrive@0.6.2"])).as_deref(),
            Some("@modelcontextprotocol/server-gdrive")
        );
        assert_eq!(
            npx_package_name(&v(&["--yes", "pkg@latest", "--flag"])).as_deref(),
            Some("pkg")
        );
        assert_eq!(
            npx_package_name(&v(&["-p", "@s/a@1", "bin"])).as_deref(),
            Some("@s/a")
        );
        assert_eq!(
            npx_package_name(&v(&["--package=x@2"])).as_deref(),
            Some("x")
        );
        assert_eq!(npx_package_name(&v(&["-y"])), None);
        assert_eq!(npx_package_name(&v(&[""])), None);
    }

    #[test]
    fn delai_demarrage_long_si_paquet_npx_absent_du_cache() {
        let tmp = std::env::temp_dir().join(format!("mcc-npx-{}", std::process::id()));
        let pkg = tmp
            .join("abc123")
            .join("node_modules")
            .join("@s")
            .join("ok");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("package.json"), "{}").unwrap();
        let dirs = vec![tmp.clone()];
        assert!(npx_package_cached(&dirs, "@s/ok"));
        assert!(!npx_package_cached(&dirs, "@s/absent"));
        let _ = std::fs::remove_dir_all(&tmp);
        assert_eq!(stdio_start_timeout_ms("node", &v(&["x.js"])), 30_000);
        assert_eq!(stdio_start_timeout_ms("bunx", &v(&["mcp-x"])), 180_000);
        // Chemin complet classé par file_stem : séparateurs natifs par OS
        // (Path::file_stem ne découpe pas les `\` sous unix).
        #[cfg(windows)]
        assert_eq!(
            stdio_start_timeout_ms(r"C:\x\uvx.exe", &v(&["paquet-uv-inexistant-mcc-zz9"])),
            300_000
        );
        #[cfg(not(windows))]
        assert_eq!(
            stdio_start_timeout_ms("/usr/local/bin/uvx", &v(&["paquet-uv-inexistant-mcc-zz9"])),
            300_000
        );
        assert_eq!(
            stdio_start_timeout_ms("npx", &v(&["-y", "paquet-inexistant-mcc-zz9"])),
            300_000
        );
        assert_eq!(
            stdio_start_timeout_ms("npx", &v(&["-y", "github:org/repo"])),
            300_000
        );
    }

    #[test]
    fn delai_demarrage_long_pour_uvx_git() {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let tmp_uv = std::env::temp_dir().join(format!("mcc-uv-{}", std::process::id()));
        let wheel_dir = tmp_uv.join("wheels-v6").join("pypi").join("mcp-simple");
        std::fs::create_dir_all(&wheel_dir).unwrap();
        let uv_dirs = vec![tmp_uv.clone()];
        assert!(uv_package_cached(&uv_dirs, "mcp_simple"));
        assert!(!uv_package_cached(&uv_dirs, "absent-pkg"));
        let _ = std::fs::remove_dir_all(&tmp_uv);

        assert_eq!(
            uv_package_name(&v(&[
                "--from",
                "redis-mcp-server@latest",
                "redis-mcp-server"
            ])),
            Some("redis-mcp-server".to_string())
        );
        assert_eq!(
            uv_package_name(&v(&["--python", "3.13", "android_mcp"])),
            Some("android-mcp".to_string())
        );
        assert_eq!(
            stdio_start_timeout_ms(
                "uvx",
                &v(&["--from", "git+https://github.com/org/repo.git", "mcp-repo"])
            ),
            300_000
        );
        // Chemin complet classé par file_stem : séparateurs natifs par OS
        // (Path::file_stem ne découpe pas les `\` sous unix).
        #[cfg(windows)]
        assert_eq!(
            stdio_start_timeout_ms(
                r"C:\tools\uvx.exe",
                &v(&["--from", "git+https://github.com/org/repo.git", "mcp-repo"])
            ),
            300_000
        );
        #[cfg(not(windows))]
        assert_eq!(
            stdio_start_timeout_ms(
                "/usr/local/bin/uvx",
                &v(&["--from", "git+https://github.com/org/repo.git", "mcp-repo"])
            ),
            300_000
        );
        assert_eq!(
            stdio_start_timeout_ms("uvx", &v(&["paquet-uv-absent-mcc-zz9"])),
            300_000
        );
        assert_eq!(
            stdio_start_timeout_ms(
                "uv",
                &v(&[
                    "run",
                    "--no-project",
                    "--with",
                    "git+https://example.com/r",
                    "python",
                    "-m",
                    "m"
                ])
            ),
            300_000
        );
    }

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
                probe::system_tool("powershell")
                    .to_string_lossy()
                    .to_string(),
                vec![
                    "-NoProfile".to_string(),
                    "-NonInteractive".to_string(),
                    "-Command".to_string(),
                    "Start-Sleep -Seconds 15".to_string(),
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
