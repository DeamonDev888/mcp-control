// Détection litemcp : registre %TEMP%, annonces, import natif, boucle de sondage.
// (split v0.7.20 : manage.rs reste la racine — API stable via réexport)
use super::*;

/// Lit une annonce du registre litemcp (`%TEMP%/litemcp-registry/<nom>.json`).
/// Écrites par les serveurs litemcp intégrés (ex : moltbook) au bind réussi —
/// contient exe/args/cwd/port : tout pour Start/Stop natifs sans scan ni
/// saisie manuelle. Aucune erreur : fichier absent/corrompu = annonce absente.
fn read_litemcp_registry(name: &str) -> Option<Value> {
    let mut base = std::env::temp_dir();
    if base.as_os_str().is_empty() {
        base = std::path::PathBuf::from("/tmp");
    }
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return None;
    }
    let path = base.join("litemcp-registry").join(format!("{name}.json"));
    let body = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&body).ok()
}

/// Applique une annonce litemcp à l'état : création si aucun serveur HTTP
/// LOCAL du même nom n'existe (exe/args/cwd deviennent la commande de
/// lancement → Start/Stop/autoboot natifs), sinon resync du port et
/// **adoption** — un serveur découvert par scan (sans commande) reçoit celui
/// de l'annonce et est promu `manual` (un discovered n'est pas persisté, la
/// commande serait perdue au reboot). Un serveur avec déjà une commande la
/// garde : l'annonce ne réécrit jamais un choix de l'utilisateur.
/// Renvoie true si l'état a changé.
pub(super) fn apply_announcement(state: &Arc<AppState>, ann: &Value) -> bool {
    let port = ann.get("port").and_then(|x| x.as_u64()).unwrap_or(0) as u16;
    let exe = ann.get("exe").and_then(|x| x.as_str()).unwrap_or("");
    if port == 0 || exe.is_empty() {
        return false;
    }
    let args: Vec<String> = ann
        .get("args")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|i| i.as_str().map(String::from))
                .skip(1) // args[0] = chemin de l'exe lui-même
                .collect()
        })
        .unwrap_or_default();
    let workdir = ann.get("cwd").and_then(|x| x.as_str()).map(String::from);
    let label = ann
        .get("name")
        .and_then(|x| x.as_str())
        .unwrap_or("litemcp")
        .to_string();
    let version = ann
        .get("version")
        .and_then(|x| x.as_str())
        .map(String::from);
    let ts = ann.get("ts").and_then(|x| x.as_u64()).unwrap_or(0);

    let mut servers = lockv(&state.servers);
    let existing = servers.iter_mut().find(|s| {
        matches!(&s.transport, Transport::Http { host: None, .. })
            && s.label.eq_ignore_ascii_case(&label)
    });
    match existing {
        Some(srv) => {
            let mut changed = false;
            // resync du port si le serveur a changé de port d'écoute
            if let Transport::Http {
                port: ref mut p, ..
            } = srv.transport
            {
                if *p != port {
                    add_log(
                        state,
                        "info",
                        &format!("[litemcp] {} : port {} -> {} (annonce)", srv.id, *p, port),
                    );
                    *p = port;
                    changed = true;
                }
            }
            // adoption : découvert par scan sans commande → commande native
            if srv.start_cmd.is_none() {
                srv.start_cmd = Some(StartCmd {
                    program: exe.to_string(),
                    args,
                    workdir,
                    env: Vec::new(),
                });
                // un discovered n'est pas persisté : promotion en manual pour
                // que la commande adoptée survive au redémarrage
                srv.origin = Origin::Manual;
                if srv.framework.is_none() {
                    srv.framework = Some("litemcp".into());
                }
                if version.is_some() && srv.version.is_none() {
                    srv.version = version;
                }
                add_log(
                    state,
                    "info",
                    &format!(
                        "[litemcp] adoption : {} reçoit sa commande de lancement",
                        srv.id
                    ),
                );
                changed = true;
            } else if version.is_some() && srv.version.is_none() {
                srv.version = version;
                changed = true;
            }
            let seen = srv.last_seen_ts.unwrap_or(0);
            if ts > seen {
                srv.last_seen_ts = Some(ts);
                changed = true;
            }
            changed
        }
        None => {
            let id = format!("litemcp-{label}");
            add_log(
                state,
                "info",
                &format!("[litemcp] import natif : {} (port {port})", label),
            );
            servers.push(Server {
                id,
                label,
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
                version,
                protocol: None,
                sse: false,
                sse_endpoint: false,
                ws: false,
                lang: Some("rust".into()),
                framework: Some("litemcp".into()),
                via_ssh: false,
                tools: vec![],
                latency_ms: None,
                last_seen_ts: Some(ts),
                last_error: None,
                start_cmd: Some(StartCmd {
                    program: exe.to_string(),
                    args,
                    workdir,
                    env: Vec::new(),
                }),
                auto_restart: true,
                favorite: false,
                tools_disabled: vec![],
            });
            true
        }
    }
}

/// Import auto des serveurs litemcp annoncés (`%TEMP%/litemcp-registry/`).
/// Appelé à chaque cycle de `poll_all` : le CC et le serveur litemcp restent
/// synchronisés (port, commande, vie) sans intervention.
pub fn import_litemcp_registry(state: &Arc<AppState>) {
    let names = ["moltbook"];
    for name in names {
        let Some(ann) = read_litemcp_registry(name) else {
            continue;
        };
        if apply_announcement(state, &ann) {
            save_config(state);
        }
    }
}

// ---------- pool de sondes persistant (audit 03, constat 2) ----------

use std::collections::{HashMap, VecDeque};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Instant;

/// Nombre de workers du pool de sondes (persistant). Chaque poll_one est borné
/// par ses propres timeouts ; 12 workers en parallèle couvrent largement les
/// installations typiques, sans recréer de threads à chaque cycle. Un serveur
/// lent, muet ou en cours de démarrage retarde au plus SON worker — jamais les
/// autres serveurs ni le cycle suivant (l'ancien découpage en paquets de 8
/// joints par `thread::scope` imposait une barrière par paquet : le serveur le
/// plus lent du paquet retardait tous les suivants, et la re-sonde de 30 s
/// après un spawn bloquait le cycle entier — périodes mesurées à 48-90 s).
const POLL_WORKERS: usize = 12;
/// Durée au-delà de laquelle une entrée « en sonde » est considérée
/// abandonnée (un poll_one est borné par ses propres timeouts — l'entrée
/// est normalement libérée par le worker à sa fin). Garde-fou : si un
/// worker mourait sans libérer, le serveur serait sauté pour toujours.
const SONDAGE_MAX_SECS: u64 = 120;

/// Un travail de sondage : état applicatif + id de serveur.
struct Travail {
    state: Arc<AppState>,
    id: String,
}

static FILE_TRAVAIL: OnceLock<Mutex<VecDeque<Travail>>> = OnceLock::new();
static FILE_SIGNAL: OnceLock<Condvar> = OnceLock::new();
static POOL_DEMARRE: OnceLock<()> = OnceLock::new();
/// Serveurs actuellement en sonde (marqués à l'envoi, libérés par le worker
/// à la fin du poll_one). Le cycle suivant SAUTE un serveur encore en sonde
/// au lieu d'attendre la sonde la plus lente (lot A) ou d'empiler des sondes
/// doublons du même serveur muet.
static SONDAGE_EN_COURS: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

fn sonde_en_cours_map() -> &'static Mutex<HashMap<String, Instant>> {
    SONDAGE_EN_COURS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Filtre les ids encore en sonde ET marque les autres en UNE seule section
/// verrouillée : le poller et le tool refresh appellent poll_all en concurrence
/// — sans l'atomicité, deux appelants verraient le même serveur libre et
/// empileraient deux sondes du même id.
fn filtrer_et_marquer_sondage(ids: Vec<String>) -> Vec<String> {
    let mut map = lockv(sonde_en_cours_map());
    map.retain(|_, t| t.elapsed().as_secs() < SONDAGE_MAX_SECS);
    let maintenant = Instant::now();
    let mut retenus = Vec::with_capacity(ids.len());
    for id in ids {
        if !map.contains_key(&id) {
            map.insert(id.clone(), maintenant);
            retenus.push(id);
        }
    }
    retenus
}

/// Libère le marqueur à la fin d'un poll_one (branche normale OU panic —
/// le worker appelle après catch_unwind).
fn liberer_sondage(id: &str) {
    lockv(sonde_en_cours_map()).remove(id);
}

fn file_travail() -> (&'static Mutex<VecDeque<Travail>>, &'static Condvar) {
    (
        FILE_TRAVAIL.get_or_init(|| Mutex::new(VecDeque::new())),
        FILE_SIGNAL.get_or_init(Condvar::new),
    )
}

/// Worker persistant : prend un travail, l'exécute, accuse réception, recommence.
/// poll_one ne doit jamais tuer le worker : catch_unwind en filet (un panic
/// serait déjà anormal — lib.rs en journalise un au niveau du cycle entier).
fn worker_sondes() {
    let (file, signal) = file_travail();
    loop {
        let travail = {
            let mut q = lockv(file);
            loop {
                if let Some(t) = q.pop_front() {
                    break t;
                }
                q = match signal.wait(q) {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };
            }
        };
        let Travail { state, id } = travail;
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            poll_one(&state, &id);
        }));
        // le marqueur est libéré même en cas de panic : le serveur sera
        // re-sondé au cycle suivant (jamais sauté pour toujours)
        liberer_sondage(&id);
    }
}

/// Envoi des sondes au pool SANS ATTENTE (lot A) : chaque poll_one s'exécute
/// en parallèle chez un worker et publie l'état de SON serveur à sa fin ; le
/// cycle suivant part à l'heure même si une sonde (port muet, serveur lent)
/// tourne encore — ce serveur est simplement sauté (marqueur
/// SONDAGE_EN_COURS). L'ancien cycle attendait l'accusé de réception de
/// CHAQUE serveur (plafonné à 120 s) : la sonde la plus lente fixait la
/// période entière (30-48 s mesurés au lot 2 pour refresh 5 s).
fn cycle_via_pool(state: &Arc<AppState>, ids: &[String]) {
    if ids.is_empty() {
        return;
    }
    let (file, signal) = file_travail();
    POOL_DEMARRE.get_or_init(|| {
        for i in 0..POLL_WORKERS {
            let _ = std::thread::Builder::new()
                .name(format!("mcc-poll-{i}"))
                .spawn(worker_sondes);
        }
    });
    // marque DÉJÀ posé par filtrer_et_marquer_sondage (atomique avec le
    // filtre) : un cycle suivant ne peut jamais empiler un doublon pour un
    // serveur dont la sonde n'a pas encore démarré
    {
        let mut q = lockv(file);
        for id in ids {
            q.push_back(Travail {
                state: state.clone(),
                id: id.clone(),
            });
        }
    }
    signal.notify_all();
}

pub fn poll_all(state: &Arc<AppState>) {
    // Import natif des serveurs litemcp (moltbook) : l'annonce écrite au bind
    // fournit exe/args/cwd/port — gestion Start/Stop/autoboot native, même
    // hors plage de scan (moltbook 8787 est hors 3000-3200).
    import_litemcp_registry(state);
    // priorité : l'ordre choisi par glisser-déposer gouverne l'ordre de sondage
    // (utile au boot où le premier cycle est séquentiel) — ids absents en dernier
    let order = lockv(&state.settings).server_order.clone();
    let mut ids: Vec<String> = lockv(&state.servers).iter().map(|s| s.id.clone()).collect();
    ids.sort_by_key(|id| order.iter().position(|x| x == id).unwrap_or(usize::MAX));
    // Un serveur encore en sonde (cycle précédent) est simplement SAUTÉ :
    // son poll_one publiera son état à sa fin, sans retarder ce cycle-ci ni
    // empiler un doublon (lot A — période des sains ≤ refresh + 2 s).
    let ids = filtrer_et_marquer_sondage(ids);
    // Sondes en parallèle via le pool persistant, sans attente de fin.
    // Chaque poll_one prend ses verrous brièvement et relâche avant d'écrire
    // au journal ; les spawns concurrents restent protégés par `state.spawning`.
    cycle_via_pool(state, &ids);
    save_cache(state);
}
