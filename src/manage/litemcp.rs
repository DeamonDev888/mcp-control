// Détection litemcp : annonces du dossier de l'utilisateur courant (lecture
// seule), pool de sondes persistant.
//
// AUDIT S4 (point 2) : une annonce trouvée sur le disque n'est PLUS adoptée ni
// écrite dans la configuration. Le boot d'une instance de test avait créé un
// serveur `litemcp-moltbook` (port 8787) dans le seul but de lire un fichier
// déposé dans %TEMP%. Désormais le fichier n'est que LISTÉ (« détecté, non
// suivi ») ; l'adoption reste un geste de l'utilisateur (lot U2).
use super::*;

/// Dossier d'annonces litemcp de l'UTILISATEUR COURANT : `%TEMP%` est per-user
/// sous Windows (`C:\Users\<moi>\AppData\Local\Temp`) ; sous Unix, `$TMPDIR`
/// ou `/tmp`. Aucun autre dossier n'est lu, aucun champ n'est exécuté.
fn registry_dir() -> std::path::PathBuf {
    // Surcharge de TEST : les scénarios écrivent leurs annonces dans un
    // dossier temporaire au lieu de %TEMP%\litemcp-registry (qui appartient
    // à l'utilisateur réel).
    if let Ok(dir) = std::env::var("MCC_LITEMP_REGISTRY") {
        if !dir.trim().is_empty() {
            return std::path::PathBuf::from(dir);
        }
    }
    let mut base = std::env::temp_dir();
    if base.as_os_str().is_empty() {
        base = std::path::PathBuf::from("/tmp");
    }
    base.join("litemcp-registry")
}

/// Bornes de lecture : 64 Kio par fichier, 32 annonces retenues au plus,
/// 512 fichiers `.json` examinés au maximum par appel (évite qu'un amas de
/// fichiers zombies morts n'éclipse des annonces vivantes tout en restant borné).
const ANNONCE_MAX_BYTES: u64 = 64 * 1024;
const ANNONCES_MAX: usize = 32;
const ANNONCES_SCAN_MAX: usize = 512;

/// Une annonce lue sur le disque. Champs d'AFFICHAGE seulement : `exe`, `args`,
/// `cwd` ne sont ni lus ni rendus (rien d'une annonce n'est exécuté).
#[derive(Debug, Clone, PartialEq)]
pub struct Annonce {
    pub name: String,
    pub port: Option<u16>,
    pub version: Option<String>,
    /// Un serveur de la liste porte-t-il déjà ce nom (ou ce port) ? Purement
    /// informatif — l'annonce n'est pas « suivie » pour autant.
    pub suivi: bool,
}

/// Décision d'acceptation selon le propriétaire du fichier.
/// Refus seulement quand l'appartenance à un AUTRE compte est PROUVÉE : un
/// système qui ne sait pas lire le propriétaire (macOS sans /proc) ne doit
/// pas rendre la liste inutilisable — les bornes de taille/nombre restent.
pub(super) fn owner_check_result(owned: Option<bool>) -> Result<(), String> {
    match owned {
        Some(true) | None => Ok(()),
        Some(false) => Err("appartient à un autre compte".into()),
    }
}

/// Le fichier est-il lisible comme annonce (fichier ordinaire, taille bornée,
/// propriétaire courant quand c'est vérifiable) ?
fn annonce_lisible(path: &std::path::Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("illisible: {e}"))?;
    if !meta.file_type().is_file() {
        return Err("n'est pas un fichier ordinaire".into());
    }
    if meta.len() > ANNONCE_MAX_BYTES {
        return Err(format!(
            "trop volumineux ({} octets > {ANNONCE_MAX_BYTES})",
            meta.len()
        ));
    }
    owner_check_result(probe::file_owned_by_current_user(path))
}

/// Liste les annonces du dossier litemcp de l'utilisateur courant. Lecture
/// seule : aucune adoption, aucune écriture de config, aucun démarrage.
/// `skipped` porte le motif de chaque fichier écarté (nom + raison).
pub fn litemcp_announcements(state: &Arc<AppState>) -> Value {
    let dir = registry_dir();
    let mut annonces: Vec<Annonce> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => {
            return json!({
                "dossier": dir.to_string_lossy(),
                "detected": 0,
                "announcements": [],
                "skipped": [],
            })
        }
    };
    let mut examines = 0usize;
    for entry in entries.flatten() {
        if annonces.len() >= ANNONCES_MAX || examines >= ANNONCES_SCAN_MAX {
            break;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.to_ascii_lowercase().ends_with(".json") {
            continue;
        }
        examines += 1;
        if let Err(motif) = annonce_lisible(&path) {
            skipped.push(json!({"name": name, "reason": motif}));
            continue;
        }
        let Ok(body) = std::fs::read(&path) else {
            skipped.push(json!({"name": name, "reason": "lecture impossible"}));
            continue;
        };
        let Ok(v) = serde_json::from_slice::<Value>(&body) else {
            skipped.push(json!({"name": name, "reason": "JSON invalide"}));
            continue;
        };
        if let Some(raw_pid) = v.get("pid").filter(|x| !x.is_null()) {
            let Some(pid) = raw_pid
                .as_u64()
                .and_then(|p| u32::try_from(p).ok())
                .filter(|&p| p > 0)
            else {
                skipped.push(json!({"name": name, "reason": "pid invalide"}));
                continue;
            };
            if !probe::process_alive(pid) {
                skipped.push(json!({
                    "name": name,
                    "reason": format!("processus inactif (pid {pid})"),
                }));
                continue;
            }
        }
        let nom = v
            .get("name")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| name.trim_end_matches(".json").to_string());
        let port = v
            .get("port")
            .and_then(|x| x.as_u64())
            .filter(|p| (1..=65535).contains(p))
            .map(|p| p as u16);
        let version = v
            .get("version")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        let suivi = {
            let servers = lockv(&state.servers);
            servers.iter().any(|s| {
                s.label.eq_ignore_ascii_case(&nom)
                    || matches!(&s.transport, Transport::Http { host, port: p, .. }
                        if host.as_deref().map(crate::is_loopback_bind).unwrap_or(true)
                            && Some(*p) == port)
            })
        };
        annonces.push(Annonce {
            name: nom,
            port,
            version,
            suivi,
        });
    }
    annonces.sort_by(|a, b| a.name.cmp(&b.name));
    let arr: Vec<Value> = annonces
        .iter()
        .map(|a| {
            let swarm_kind = crate::swarm::detect_announcement(&a.name).map(|k| k.as_str());
            json!({
                "name": a.name,
                "port": a.port,
                "version": a.version,
                "swarm_kind": swarm_kind,
                // jamais « adoptée » : l'interface l'affiche « détecté, non suivi »
                "detected": true,
                "followed": a.suivi,
            })
        })
        .collect();
    json!({
        "dossier": dir.to_string_lossy(),
        "detected": arr.len(),
        "announcements": arr,
        "skipped": skipped,
    })
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
static SONDAGE_EN_COURS: OnceLock<Mutex<HashMap<(usize, String), Instant>>> = OnceLock::new();
static LAST_WORKER_CACHE_SAVE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn sonde_en_cours_map() -> &'static Mutex<HashMap<(usize, String), Instant>> {
    SONDAGE_EN_COURS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Filtre les ids encore en sonde ET marque les autres en UNE seule section
/// verrouillée : le poller et le tool refresh appellent poll_all en concurrence
/// — sans l'atomicité, deux appelants verraient le même serveur libre et
/// empileraient deux sondes du même id. Clé scopée par `Arc::as_ptr(state)`
/// pour isoler les instances distinctes `[M-14]`.
fn filtrer_et_marquer_sondage(state: &Arc<AppState>, ids: Vec<String>) -> Vec<String> {
    let mut map = lockv(sonde_en_cours_map());
    map.retain(|_, t| t.elapsed().as_secs() < SONDAGE_MAX_SECS);
    let maintenant = Instant::now();
    let ptr = Arc::as_ptr(state) as usize;
    let mut retenus = Vec::with_capacity(ids.len());
    for id in ids {
        let key = (ptr, id.clone());
        if let std::collections::hash_map::Entry::Vacant(e) = map.entry(key) {
            e.insert(maintenant);
            retenus.push(id);
        }
    }
    retenus
}

/// Libère le marqueur à la fin d'un poll_one (branche normale OU panic —
/// le worker appelle après catch_unwind).
fn liberer_sondage(state: &Arc<AppState>, id: &str) {
    let key = (Arc::as_ptr(state) as usize, id.to_string());
    lockv(sonde_en_cours_map()).remove(&key);
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
            poll_one_bg(&state, &id);
        }));
        // le marqueur est libéré même en cas de panic : le serveur sera
        // re-sondé au cycle suivant (jamais sauté pour toujours)
        liberer_sondage(&state, &id);
        // [M-14] : persister le cache d'état à la fin des sondes (borné à 1/5 s)
        // afin que les nouveaux états observés par les workers survivent à l'arrêt.
        let now = crate::types::now_ts();
        let prev = LAST_WORKER_CACHE_SAVE.load(std::sync::atomic::Ordering::Relaxed);
        if now.saturating_sub(prev) >= 5
            && LAST_WORKER_CACHE_SAVE
                .compare_exchange(
                    prev,
                    now,
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                )
                .is_ok()
        {
            save_cache(&state);
        }
    }
}

/// Envoi des sondes au pool SANS ATTENTE (lot A) : chaque poll_one s'exécute
/// en parallèle chez un worker et publie l'état de SON serveur à sa fin ; le
/// cycle suivant part à l'heure même si une sonde (port muet, serveur lent)
/// tourne encore — ce serveur est simplement sauté (marqueur
/// SONDAGE_EN_COURS). L'ancien cycle attendait l'accusé de réception de
/// CHAQUE serveur (plafonné à 120 s) : la sonde la plus lente fixait la
/// période entière (30-48 s mesurés au lot 2 pour refresh 5 s).
pub(crate) fn cycle_via_pool(state: &Arc<AppState>, ids: &[String]) {
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
    // P-3 : invalide la table port → process du cycle précédent afin qu'au plus
    // un seul `netstat` + `tasklist` soit construit pour ce cycle.
    probe::begin_port_owner_cycle();
    // Les annonces litemcp NE SONT PLUS importées ici (audit S4, point 2) :
    // `litemcp_announcements` les liste à la demande, sans adoption ni écriture
    // de config. Le cycle ne lit donc plus %TEMP% et ne peut plus créer de
    // serveur que l'utilisateur n'a pas demandé.
    // priorité : l'ordre choisi par glisser-déposer gouverne l'ordre de sondage
    // (utile au boot où le premier cycle est séquentiel) — ids absents en dernier
    let order = lockv(&state.settings).server_order.clone();
    let mut ids: Vec<String> = lockv(&state.servers).iter().map(|s| s.id.clone()).collect();
    ids.sort_by_key(|id| order.iter().position(|x| x == id).unwrap_or(usize::MAX));
    // Un serveur encore en sonde (cycle précédent) est simplement SAUTÉ :
    // son poll_one publiera son état à sa fin, sans retarder ce cycle-ci ni
    // empiler un doublon (lot A — période des sains ≤ refresh + 2 s).
    let ids = filtrer_et_marquer_sondage(state, ids);
    // Sondes en parallèle via le pool persistant, sans attente de fin.
    // Chaque poll_one prend ses verrous brièvement et relâche avant d'écrire
    // au journal ; les spawns concurrents restent protégés par `state.spawning`.
    cycle_via_pool(state, &ids);
    save_cache(state);
}
