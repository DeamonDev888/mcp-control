// Chargement/sauvegarde de config.json, parsing des serveurs et réglages, serveurs par défaut.
// (split v0.7.20 : manage.rs reste la racine — API stable via réexport)
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Parse le champ `"env": {"CLÉ": "valeur", …}` d'une entrée (config.json ou
/// import mcpServers). Seules les valeurs STRING sont retenues ; champ absent →
/// vide (compat config v0.7.20 — CRITÈRE de non-régression du lot 3).
pub fn env_from_json(s: &Value) -> Vec<(String, String)> {
    s.get("env")
        .and_then(|x| x.as_object())
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Ligne de boot dans mcc.log (à côté du config.json) : `load_config` tourne
/// AVANT la création de l'AppState — sans fichier, aucun support possible pour
/// un config.json corrompu renommé (lot 3, audit 03 constat 8).
fn journal_boot(config_path: &str, msg: &str) {
    use std::io::Write;
    println!("[config] {msg}");
    let p = std::path::Path::new(config_path).with_file_name("mcc.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
    {
        let _ = writeln!(
            f,
            "[{}] [warn] {}",
            crate::types::iso_utc(crate::types::now_ts()),
            msg
        );
    }
}

/// Normalise une liste JSON de ports : entiers 1-65535 seulement, doublons
/// retirés (premier vu), plafonnée à `max` entrées. Un tableau vide est
/// significatif (« aucun port imposé » / « aucun port exclu ») : l'appelant
/// l'affecte tel quel quand la clé est présente.
pub fn parse_port_list(arr: &[Value], max: usize) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::new();
    for v in arr {
        let Some(p) = v.as_u64() else { continue };
        if !(1..=65535).contains(&p) {
            continue;
        }
        let p = p as u16;
        if !out.contains(&p) {
            out.push(p);
        }
        if out.len() >= max {
            break;
        }
    }
    out
}

/// Serveurs de base d'une installation neuve : AUCUN (lot S4, point 8).
///
/// L'ancienne liste embarquait huit serveurs avec des chemins propres à la
/// machine de développement (`C:/Users/<user>/...`) : une installation neuve
/// démarrait avec des serveurs fantômes, et l'app pouvait lancer des
/// programmes qui n'existent pas chez l'utilisateur. La découverte (ports en
/// écoute, scan réseau, import mcpServers) remplit la liste à la place.
///
/// MIGRATION : un `config.json` existant garde ses entrées `seed-*` telles
/// quelles — elles sont relues comme n'importe quel serveur manuel (label,
/// transport, commande de lancement). Rien n'est réinjecté au démarrage ;
/// `settings.removed_seeds` reste lu pour ne réintroduire aucun identifiant.
pub(super) fn default_servers() -> Vec<Server> {
    Vec::new()
}

/// Résolution du chemin de config au runtime (voir main.rs) :
/// 1. override explicite (MCC_CONFIG)
/// 2. dossier de l'exe / config.json s'il existe (installé : l'app a SA config)
/// 3. CARGO_MANIFEST_DIR/config.json s'il existe (dev local — l'exe de dev vit
///    dans target/, l'installé à côté de sa config ; un exe installé ne doit
///    JAMAIS réécrire la config du dossier de dev)
/// 4. dossier de l'exe / config.json (premier boot d'une install fraîche)
pub fn resolve_config_path(manifest_dir: &str, override_env: Option<String>) -> String {
    if let Some(p) = override_env {
        if !p.is_empty() {
            return p;
        }
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from(manifest_dir));
    let exe_config = exe_dir.join("config.json");
    if exe_config.exists() {
        return exe_config.to_string_lossy().to_string();
    }
    let dev_path = std::path::Path::new(manifest_dir).join("config.json");
    if dev_path.exists() {
        return dev_path.to_string_lossy().to_string();
    }
    exe_config.to_string_lossy().to_string()
}

/// Charge le journal runtime persisté (backoff d'autoboot + historique des
/// crashs et redémarrages) — appelé au boot : le compteur d'échecs survit
/// ainsi au redémarrage de l'application.
pub fn load_runtime(path: &str) -> crate::runtime::RuntimeLog {
    let Ok(file) = std::fs::read_to_string(path) else {
        return crate::runtime::RuntimeLog::new();
    };
    let Ok(v) = serde_json::from_str::<Value>(&file) else {
        return crate::runtime::RuntimeLog::new();
    };
    v.get("runtime")
        .map(crate::runtime::from_json)
        .unwrap_or_default()
}

/// Historique runtime formaté pour l'API /api/runtime : une liste plate
/// d'événements (du plus récent au plus ancien) + un résumé par serveur.
pub fn runtime_json(state: &Arc<AppState>) -> Value {
    let rt = lockv(&state.runtime);
    let mut events: Vec<crate::runtime::RuntimeEvent> = Vec::new();
    let mut summary: Vec<Value> = Vec::new();
    let mut ids: Vec<&String> = rt.keys().collect();
    ids.sort();
    for id in ids {
        let Some(s) = rt.get(id) else { continue };
        events.extend(s.events.iter().cloned());
        summary.push(json!({
            "id": id,
            "label": s.events.last().map(|e| e.label.clone()).unwrap_or_else(|| id.clone()),
            "fails": s.fails,
            "restarts": s.restarts,
            "crashes": s.crashes,
            "backoff_secs": s.backoff_secs(),
            "last_success_ts": s.last_success_ts,
        }));
    }
    events.sort_by_key(|a| std::cmp::Reverse(a.ts));
    json!({
        "events": events.iter().take(200).map(|e| e.to_json()).collect::<Vec<_>>(),
        "servers": summary,
    })
}
pub fn load_config(path: &str) -> (Vec<Server>, crate::types::Settings) {
    let defaults = default_servers();
    let settings = crate::types::Settings::default();
    // Temporaires orphelins d'une écriture interrompue (kill, coupure) : un
    // `.tmp` complet contient les secrets de la config avec l'ACL héritée du
    // dossier. Purgés AVANT toute écriture (audit S4, point 4).
    let purged = purge_orphan_tmp(path);
    // ACL des fichiers de données existants : config, cache, journal.
    secure_data_files(path);
    if purged > 0 {
        journal_boot(
            path,
            &format!("{purged} temporaire(s) d'écriture orphelin(s) supprimé(s)"),
        );
    }
    let file = match std::fs::read_to_string(path) {
        Ok(f) => f,
        Err(_) => return (defaults, settings),
    };
    let v: Value = match serde_json::from_str(&file) {
        Ok(v) => v,
        Err(_) => {
            // (lot 3, audit 03 constat 8) config.json ILLISIBLE (JSON invalide,
            // vide, BOM PowerShell…) : le repli silencieux sur les seeds
            // écrasait la config à la première sauvegarde sans laisser de
            // trace. On renomme le fichier (horodatage au format sûr Windows,
            // sans « : »), on journalise, puis on repart des défauts. Un
            // fichier ABSENT reste un premier boot normal : aucun renommage.
            let dest = format!("{}.corrompu-{}", path, crate::types::now_ts());
            let msg = match std::fs::rename(path, &dest) {
                Ok(()) => format!(
                    "config illisible (JSON invalide ou vide, {} octets) — renommé en {} — redémarrage sur les serveurs par défaut",
                    file.len(),
                    dest
                ),
                Err(e) => format!(
                    "config illisible (JSON invalide ou vide, {} octets) — renommage en {} impossible ({}) — redémarrage sur les serveurs par défaut",
                    file.len(),
                    dest,
                    e
                ),
            };
            journal_boot(path, &msg);
            return (defaults, settings);
        }
    };
    let mut servers = defaults;
    if let Some(arr) = v.get("servers").and_then(|x| x.as_array()) {
        for s in arr {
            let id = s
                .get("id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                continue;
            }
            if let Some(existing) = servers.iter_mut().find(|e| e.id == id) {
                apply_json(existing, s);
            } else if let Some(mut srv) = from_json(s) {
                srv.origin = Origin::Manual;
                servers.push(srv);
            }
        }
    }
    // reconstruit start_cmd pour tout serveur stdio (config rechargée -> auto-start au boot)
    for s in servers.iter_mut() {
        if s.start_cmd.is_none() {
            if let Transport::Stdio {
                program,
                args,
                workdir,
                env,
            } = s.transport.clone()
            {
                if !program.is_empty() {
                    s.start_cmd = Some(StartCmd {
                        program,
                        args,
                        workdir,
                        env,
                    });
                }
            }
        }
    }
    let mut settings = parse_settings(&v, settings);
    // (lot A) plage de scan inversée (config.json édité à la main) :
    // silencieusement ignorée par scan_ports (0 port rendu) — remise dans
    // l'ordre au chargement, avec une ligne au journal de boot.
    if settings.scan_from > settings.scan_to {
        let (from, to) = (settings.scan_from, settings.scan_to);
        std::mem::swap(&mut settings.scan_from, &mut settings.scan_to);
        journal_boot(
            path,
            &format!("plage de scan inversée ({from} > {to}) — remise dans l'ordre au chargement"),
        );
    }
    // « Retirer durablement » : les seeds explicitement retirés ne sont pas
    // réinjectés. Depuis le lot S4, `default_servers()` est VIDE : les entrées
    // `seed-*` d'un config.json existant sont relues comme des serveurs
    // manuels (migration — rien n'est réinjecté au démarrage), et ce filtre ne
    // peut donc plus retirer que des seeds qui n'existent plus.
    let removed: Vec<String> = settings.removed_seeds.clone();
    servers.retain(|s| !(s.origin == Origin::Seed && removed.contains(&s.id)));
    (servers, settings)
}

#[allow(unused_must_use)]
pub(super) fn apply_json(srv: &mut Server, s: &Value) {
    if let Some(b) = s.get("enabled").and_then(|x| x.as_bool()) {
        srv.enabled = b;
        srv.state = if b { SState::Down } else { SState::Disabled };
    }
    if let Some(t) = s.get("transport").and_then(|x| x.as_str()) {
        if t == "http" {
            if let (Some(p), Some(path)) = (
                s.get("port")
                    .and_then(|x| x.as_u64())
                    .and_then(|p| u16::try_from(p).ok())
                    .filter(|&p| p >= 1),
                s.get("path").and_then(|x| x.as_str()),
            ) {
                srv.transport = Transport::Http {
                    host: s.get("host").and_then(|x| x.as_str()).map(String::from),
                    port: p,
                    path: path.to_string(),
                };
                if let Some(v) = s.get("via_ssh").and_then(|x| x.as_bool()) {
                    srv.via_ssh = v;
                }
            }
        } else if t == "stdio" {
            let args: Vec<String> = s
                .get("args")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|i| i.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            srv.transport = Transport::Stdio {
                program: s
                    .get("program")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                args,
                workdir: s.get("workdir").and_then(|x| x.as_str()).map(String::from),
                env: env_from_json(s),
            };
        }
    }
    // commande de lancement optionnelle pour un serveur HTTP (Start/Stop par MCP Control)
    if let Some(program) = s.get("program").and_then(|x| x.as_str()) {
        if !program.is_empty() {
            let args: Vec<String> = s
                .get("args")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|i| i.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            srv.start_cmd = Some(StartCmd {
                program: program.to_string(),
                args,
                workdir: s.get("workdir").and_then(|x| x.as_str()).map(String::from),
                env: env_from_json(s),
            });
        }
    }
    if let Some(l) = s.get("label").and_then(|x| x.as_str()) {
        srv.label = l.to_string();
    }
    if let Some(arr) = s.get("tools_disabled").and_then(|x| x.as_array()) {
        srv.tools_disabled = arr
            .iter()
            .filter_map(|i| i.as_str().map(String::from))
            .collect();
    }
}

pub(super) fn from_json(s: &Value) -> Option<Server> {
    let id = s.get("id").and_then(|x| x.as_str())?.to_string();
    let label = s
        .get("label")
        .and_then(|x| x.as_str())
        .unwrap_or(&id)
        .to_string();
    let transport = match s.get("transport").and_then(|x| x.as_str()) {
        Some("stdio") => Transport::Stdio {
            program: s
                .get("program")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            args: s
                .get("args")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|i| i.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            workdir: s.get("workdir").and_then(|x| x.as_str()).map(String::from),
            env: env_from_json(s),
        },
        _ => Transport::Http {
            host: s.get("host").and_then(|x| x.as_str()).map(String::from),
            port: s
                .get("port")
                .and_then(|x| x.as_u64())
                .and_then(|p| u16::try_from(p).ok())
                .filter(|&p| p >= 1)?,
            path: s
                .get("path")
                .and_then(|x| x.as_str())
                .unwrap_or("/mcp")
                .to_string(),
        },
    };
    let enabled = s.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true);
    // Autoboot par serveur : défaut true (comportement historique), false = le
    // serveur n'est pas relancé automatiquement (bouton ⚡ de l'UI).
    let auto_restart = s
        .get("auto_restart")
        .and_then(|x| x.as_bool())
        .unwrap_or(true);
    let via_ssh = s.get("via_ssh").and_then(|x| x.as_bool()).unwrap_or(false);
    // commande de lancement optionnelle (HTTP lancé par MCP Control)
    let start_cmd = {
        let program = s
            .get("program")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if program.is_empty() {
            None
        } else {
            Some(StartCmd {
                program,
                args: s
                    .get("args")
                    .and_then(|x| x.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|i| i.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                workdir: s.get("workdir").and_then(|x| x.as_str()).map(String::from),
                env: env_from_json(s),
            })
        }
    };
    Some(Server {
        id,
        label,
        transport,
        origin: Origin::Manual,
        enabled,
        favorite: s.get("favorite").and_then(|x| x.as_bool()).unwrap_or(false),
        auto_restart,
        state: if enabled {
            SState::Down
        } else {
            SState::Disabled
        },
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
        via_ssh,
        tools: vec![],
        latency_ms: None,
        last_seen_ts: None,
        last_error: None,
        start_cmd,
        tools_disabled: s
            .get("tools_disabled")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|i| i.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

pub(super) fn parse_settings(v: &Value, mut st: crate::types::Settings) -> crate::types::Settings {
    if let Some(s) = v.get("settings") {
        if let Some(n) = s.get("notify") {
            st.notify = crate::notify::from_json(n);
        }
        if let Some(x) = s.get("refresh_secs").and_then(|x| x.as_u64()) {
            // clamp : 0 = busy-loop CPU, trop long = état périmé à l'écran
            st.refresh_secs = x.clamp(5, 3600);
        }
        if let Some(p) = s
            .get("scan_from")
            .and_then(|x| x.as_u64())
            .and_then(|p| u16::try_from(p).ok())
            .filter(|&p| p >= 1)
        {
            st.scan_from = p;
        }
        if let Some(p) = s
            .get("scan_to")
            .and_then(|x| x.as_u64())
            .and_then(|p| u16::try_from(p).ok())
            .filter(|&p| p >= 1)
        {
            st.scan_to = p;
        }
        if let Some(p) = s
            .get("ui_port")
            .and_then(|x| x.as_u64())
            .and_then(|p| u16::try_from(p).ok())
            .filter(|&p| p >= 1)
        {
            st.ui_port = p;
        }
        if let Some(x) = s.get("ui_bind").and_then(|x| x.as_str()) {
            st.ui_bind = x.to_string();
        }
        if let Some(x) = s.get("network_enabled").and_then(|x| x.as_bool()) {
            st.network_enabled = x;
        }
        if let Some(arr) = s.get("network_ports").and_then(|x| x.as_array()) {
            let ports: Vec<u16> = arr
                .iter()
                .filter_map(|x| x.as_u64())
                .filter(|p| *p >= 1 && *p <= 65535)
                .map(|p| p as u16)
                .collect();
            if !ports.is_empty() {
                st.network_ports = ports;
            }
        }
        // Découverte locale (lot S4, point 3) : ports imposés / exclus. Un
        // tableau vide EFFACE le réglage (contrairement à network_ports, dont
        // le vide est ignoré pour ne pas casser une config ancienne).
        if let Some(arr) = s.get("discovery_ports_only").and_then(|x| x.as_array()) {
            st.discovery_ports_only = parse_port_list(arr, 512);
        }
        if let Some(arr) = s.get("discovery_ports_excluded").and_then(|x| x.as_array()) {
            st.discovery_ports_excluded = parse_port_list(arr, 512);
        }
        if let Some(x) = s.get("api_access").and_then(|x| x.as_str()) {
            st.api_access = if x.is_empty() {
                None
            } else {
                Some(x.to_string())
            };
        }
        if let Some(arr) = s.get("allowed_origins").and_then(|x| x.as_array()) {
            let o: Vec<String> = arr
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect();
            if !o.is_empty() {
                st.allowed_origins = o;
            }
        }
        if let Some(arr) = s.get("removed_seeds").and_then(|x| x.as_array()) {
            st.removed_seeds = arr
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect();
        }
        if let Some(x) = s.get("autostart").and_then(|x| x.as_bool()) {
            st.autostart = x;
        }
        if let Some(arr) = s.get("server_order").and_then(|x| x.as_array()) {
            st.server_order = arr
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect();
        }
        if let Some(arr) = s.get("ssh_hosts").and_then(|x| x.as_array()) {
            st.ssh_hosts = arr
                .iter()
                .filter_map(|h| {
                    let ip = h.get("ip")?.as_str()?.trim();
                    let user = h.get("user")?.as_str()?.trim();
                    if ip.is_empty() || user.is_empty() {
                        return None;
                    }
                    let port = h
                        .get("port")
                        .and_then(|x| x.as_u64())
                        .and_then(|p| u16::try_from(p).ok())
                        .filter(|&p| p > 0 && p != 22);
                    let identity_file = h
                        .get("identity_file")
                        .and_then(|x| x.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(String::from);
                    let auto_install_cli = h
                        .get("auto_install_cli")
                        .and_then(|x| x.as_bool())
                        .unwrap_or(true);
                    Some(crate::types::SshHost {
                        ip: ip.to_string(),
                        user: user.to_string(),
                        port,
                        identity_file,
                        auto_install_cli,
                    })
                })
                .collect();
        }
    }
    // Authentification forte (lot M1) : identité PUBLIQUE du serveur et registre
    // des clients, à la racine du document (`auth.server`, `auth.clients[]`).
    if let Some(a) = v.get("auth") {
        st.auth = crate::auth::AuthState::from_json(a);
    }
    st
}

pub(super) fn ssh_host_json(h: &crate::types::SshHost) -> Value {
    let mut o = Map::new();
    o.insert("ip".into(), json!(h.ip));
    o.insert("user".into(), json!(h.user));
    if let Some(p) = h.port {
        o.insert("port".into(), json!(p));
    }
    if let Some(ref k) = h.identity_file {
        o.insert("identity_file".into(), json!(k));
    }
    if !h.auto_install_cli {
        o.insert("auto_install_cli".into(), json!(false));
    }
    Value::Object(o)
}

/// Verrou global d'écriture de config.json (lot 3, audit 03 constat 7) :
/// sérialise clone→écriture pour qu'un snapshot ancien n'écrase jamais un plus
/// récent. Aucun autre chemin ne prend ce verrou, et save_config est appelée
/// hors de tout verrou state (elle re-prend servers/settings/runtime dessous),
/// donc aucun ordre de verrous nouveau.
static SAVE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
/// Séquence pour un fichier temporaire UNIQUE par écriture.
static SAVE_SEQ: AtomicU64 = AtomicU64::new(0);

fn save_lock() -> &'static Mutex<()> {
    SAVE_LOCK.get_or_init(|| Mutex::new(()))
}

/// Écrit la configuration sur disque. Signature historique : un échec est
/// journalisé et AVALÉ — réservé aux chemins sans réponse à rendre (boot,
/// poller). Les actions API utilisent [`save_config_checked`] et remontent
/// l'échec (audit S4, point 4 : plus de `ok:true` mensonger).
pub fn save_config(state: &AppState) {
    let _ = save_config_checked(state);
}

/// Écriture atomique de config.json (tmp unique dans le même dossier, puis
/// renommage), ACL explicite, temporaire supprimé en cas d'échec.
///
/// `Err` = le fichier n'a PAS été remplacé ; le message porte la cause. Les
/// actions qui viennent de modifier l'état doivent le rendre à l'appelant, au
/// lieu de répondre `ok:true` sur une écriture ratée.
pub fn save_config_checked(state: &AppState) -> Result<(), String> {
    if state.config_path.is_empty() {
        return Ok(());
    }
    let _garde = lockv(save_lock());
    let servers = lockv(&state.servers).clone();
    let settings = lockv(&state.settings).clone();
    let mut arr = Vec::new();
    for s in &servers {
        // les serveurs découverts (scan local/réseau) ne sont PAS persistés :
        // redécouverts au boot — SAUF les via_ssh : leur re-découverte exige une
        // connexion SSH réussie, on garde donc le résultat.
        if s.origin == Origin::Discovered && !s.via_ssh {
            continue;
        }
        let mut o = Map::new();
        o.insert("id".into(), json!(s.id));
        o.insert("label".into(), json!(s.label));
        o.insert("enabled".into(), json!(s.enabled));
        // Autoboot par serveur : on n'écrit que la DÉSACTIVATION (le défaut est
        // true) — config.json reste lisible et le bouton ⚡ survit au redémarrage.
        if !s.auto_restart {
            o.insert("auto_restart".into(), json!(false));
        }
        // Favori (⭐) : on n'écrit que l'ACTIVATION (le défaut est false) —
        // config.json reste lisible et l'épingle survit au redémarrage.
        if s.favorite {
            o.insert("favorite".into(), json!(true));
        }
        if !s.tools_disabled.is_empty() {
            o.insert("tools_disabled".into(), json!(s.tools_disabled));
        }
        match &s.transport {
            Transport::Http { host, port, path } => {
                o.insert("transport".into(), json!("http"));
                if let Some(h) = host {
                    o.insert("host".into(), json!(h));
                }
                o.insert("port".into(), json!(port));
                o.insert("path".into(), json!(path));
                if s.via_ssh {
                    o.insert("via_ssh".into(), json!(true));
                }
                // commande de lancement (Start/Stop par MCP Control)
                if let Some(c) = &s.start_cmd {
                    o.insert("program".into(), json!(c.program));
                    o.insert("args".into(), json!(c.args));
                    if let Some(w) = &c.workdir {
                        o.insert("workdir".into(), json!(w));
                    }
                    if !c.env.is_empty() {
                        let m: Map<String, Value> =
                            c.env.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
                        o.insert("env".into(), Value::Object(m));
                    }
                }
            }
            Transport::Stdio {
                program,
                args,
                workdir,
                env,
            } => {
                o.insert("transport".into(), json!("stdio"));
                o.insert("program".into(), json!(program));
                o.insert("args".into(), json!(args));
                if let Some(w) = workdir {
                    o.insert("workdir".into(), json!(w));
                }
                // env de l'import mcpServers : les VALEURS restent dans ce fichier
                // local (le process en a besoin au boot) — jamais dans l'API ni
                // le journal (noms seulement, cf. json.rs::server_json).
                if !env.is_empty() {
                    let m: Map<String, Value> =
                        env.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
                    o.insert("env".into(), Value::Object(m));
                }
            }
        }
        arr.push(Value::Object(o));
    }
    let doc = json!({
        "servers": arr,
        "settings": {
            "refresh_secs": settings.refresh_secs,
            "scan_from": settings.scan_from,
            "scan_to": settings.scan_to,
            "ui_port": settings.ui_port,
            "ui_bind": settings.ui_bind,
            "network_enabled": settings.network_enabled,
            "network_ports": settings.network_ports,
            "discovery_ports_only": settings.discovery_ports_only,
            "discovery_ports_excluded": settings.discovery_ports_excluded,
            "api_access": settings.api_access,
            "allowed_origins": settings.allowed_origins,
            "removed_seeds": settings.removed_seeds,
            "server_order": settings.server_order,
            "autostart": settings.autostart,
            "notify": crate::notify::to_json(&settings.notify),
            "ssh_hosts": settings
                .ssh_hosts
                .iter()
                .map(ssh_host_json)
                .collect::<Vec<_>>(),
        },
        // runtime : backoff d'autoboot + historique des crashs (persiste)
        "runtime": crate::runtime::to_json(&lockv(&state.runtime)),
        // Authentification forte (lot M1) : clé PUBLIQUE du serveur et registre
        // des clients. Aucune clé privée, aucune graine, aucun secret ici — la
        // clé privée vit au trousseau (`auth::secret`).
        "auth": settings.auth.to_json(),
    });
    let pretty = serde_json::to_string_pretty(&doc).unwrap_or_default();
    if let Err(msg) = write_atomic_with(&state.config_path, "config", |tmp| {
        std::fs::write(tmp, &pretty)
    }) {
        add_log(state, "error", &msg);
        return Err(msg);
    }
    secure_data_files(&state.config_path);
    Ok(())
}

/// Écriture atomique vers `path` via un temporaire unique (`<path>.<pid>.<seq>.tmp`)
/// dans le même dossier, puis renommage. Sur échec (disque plein, lecture seule…),
/// le temporaire est supprimé pour ne laisser aucun secret sur disque (audit S4,
/// point 4).
pub(super) fn write_atomic_with<F>(path: &str, label: &str, write_fn: F) -> Result<(), String>
where
    F: FnOnce(&str) -> std::io::Result<()>,
{
    let tmp = format!(
        "{}.{}.{}.tmp",
        path,
        std::process::id(),
        SAVE_SEQ.fetch_add(1, Ordering::Relaxed)
    );
    if let Err(e) = write_fn(&tmp) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{label} : écriture impossible ({tmp}): {e}"));
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{label} : remplacement impossible ({path}): {e}"));
    }
    Ok(())
}

/// Applique l'ACL explicite aux fichiers de données posés à côté du
/// config.json : la configuration, le cache de démarrage et les journaux
/// (audit S4, point 4). L'ACL contient TOUJOURS le compte courant,
/// SYSTEM et Administrateurs (voir [`restrict_permissions`]).
pub fn secure_data_files(config_path: &str) {
    if config_path.is_empty() {
        return;
    }
    let cfg = std::path::Path::new(config_path);
    if cfg.exists() {
        let _ = restrict_permissions(config_path);
    }
    let Some(dir) = cfg.parent() else {
        return;
    };
    for name in [
        "config.json",
        "state-cache.json",
        "mcc.log",
        "auth-audit.log",
    ] {
        let p = dir.join(name);
        // Le journal applicatif est créé ici s'il n'existe pas encore : il reçoit
        // ainsi l'ACL explicite dès le premier boot, sans attendre la première
        // ligne écrite par un autre module.
        if name == "mcc.log" && !p.exists() {
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&p);
        }
        if p.exists() {
            let _ = restrict_permissions(&p.to_string_lossy());
        }
    }
}

/// Supprime les temporaires ORPHELINS d'écriture (`<config>.<pid>.<seq>.tmp`,
/// `state-cache.json.<pid>.<seq>.tmp`) laissés par un arrêt brutal pendant une
/// écriture. Appelé au boot, avant toute écriture. Renvoie le nombre de
/// fichiers purgés.
pub fn purge_orphan_tmp(config_path: &str) -> usize {
    if config_path.is_empty() {
        return 0;
    }
    let cfg = std::path::Path::new(config_path);
    let Some(dir) = cfg.parent() else {
        return 0;
    };
    let cfg_prefix = cfg
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .map(|s| format!("{s}."));
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut purged = 0usize;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let candidat = name.ends_with(".tmp")
            && (name.starts_with("state-cache.json.")
                || cfg_prefix.as_deref().is_some_and(|p| name.starts_with(p)));
        if candidat && std::fs::remove_file(e.path()).is_ok() {
            purged += 1;
        }
    }
    purged
}

/// Restreint un fichier au compte courant, SYSTEM et Administrateurs : `0600`
/// sous Unix, ACL `icacls` explicite sous Windows.
///
/// `/inheritance:r` coupe les ACE héritées (dont BUILTIN\Users), puis trois
/// autorisations explicites sont posées : compte courant, SYSTEM et le groupe
/// Administrateurs. Les deux derniers sont désignés par SID (`*S-1-5-18`,
/// `*S-1-5-32-544`) : les noms traduits (`Système`, `Administrateurs`)
/// changeraient l'ACL selon la langue de Windows. L'ancienne version les
/// RETIRAIT, ce qui cassait les sauvegardes et l'administration du fichier.
pub fn restrict_permissions(path: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
    }
    #[cfg(windows)]
    {
        // Aucune crate : l'outil système `icacls` (chemin absolu, point 5).
        let user = probe::current_user_sid().ok_or_else(|| {
            std::io::Error::other("SID du compte courant introuvable (OpenProcessToken)")
        })?;
        let out = probe::system_command("icacls")
            .arg(path)
            .arg("/inheritance:r")
            .arg("/grant:r")
            .arg(format!("*{user}:F"))
            .arg("/grant:r")
            .arg("*S-1-5-18:F") // SYSTEM — AUTORITE NT\Système
            .arg("/grant:r")
            .arg("*S-1-5-32-544:F") // BUILTIN\Administrateurs
            .output()?;
        if out.status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "icacls a échoué ({}) : {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            )))
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Ok(())
    }
}
