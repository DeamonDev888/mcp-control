//! Session locale de la fenêtre — `docs/auth.md` §6 (DA2 = a, lot S1).
//!
//! Au lancement, le moteur tire un **jeton d'amorçage** de 32 octets du système
//! (base64url), valable {@link BOOTSTRAP_TTL} et **à usage unique**. Le shell
//! Tauri navigue vers `http://127.0.0.1:<port>/#t=<jeton>` (fragment : jamais
//! envoyé au serveur, jamais dans `Referer`, jamais dans `mcc.log`) ; `app.js`
//! l'échange sur `POST /api/session` contre le cookie `mcc_session` **et un
//! secret de session** que la page garde en MÉMOIRE (jamais cookie, jamais
//! localStorage/sessionStorage) et renvoie dans `X-MCC-Session`.
//!
//! Le cookie seul n'ouvre plus rien : le moteur exige cookie ET secret,
//! comparés en temps constant (`crypto::ct_eq`). C'est la correction de S-1 :
//! un cookie déposé sur un autre port loopback (ou rejoué hors navigateur)
//! ne suffit plus.
//!
//! Jusqu'à {@link MAX_SESSIONS} sessions simultanées (une par fenêtre), chacune
//! avec expiration à l'inactivité ({@link IDLE_TTL}) et durée absolue
//! ({@link ABSOLUTE_TTL}) ; `last_seen` est mis à jour à chaque requête
//! autorisée. `close_others` révoque toutes les autres (route locale
//! `POST /api/session/close-others`).
//!
//! Fenêtre rouverte sur un moteur déjà lancé : le shell demande un jeton par un
//! **canal fichier** dans le dossier de `config.json` — la demande ne contient
//! qu'un nonce, la réponse (`mcc-session.grant`) un jeton à usage unique de
//! {@link BOOTSTRAP_FILE_TTL}, les deux fichiers ont les permissions de
//! l'utilisateur seul (`manage::restrict_permissions`, 0600/ACL) et sont
//! supprimés dès lecture. Aucun port n'est ouvert pour ce canal, aucun jeton
//! n'est journalisé.
//!
//! Le cookie est `HttpOnly`, `SameSite=Strict`, `Path=/`, sans `Secure` (HTTP
//! loopback) et meurt avec le process : rien n'est écrit sur disque, il n'y a
//! aucun rafraîchissement silencieux.
//!
//! La session porte `admin` (elle ouvre `/api/*` en entier) et n'est **jamais**
//! valable sur `/mcp` : `handle_conn` ne la consulte que pour `/api/*`, et le
//! gateway ne vérifie que des signatures (test du lot M2).
//!
//! État : process-global (`OnceLock`), comme les nonces de `/api/hello` — le
//! moteur est instance unique par process et un champ ajouté à `AppState`
//! élargirait la surface du lot au-delà de `src/auth.rs` + `src/lib.rs`.

use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// Durée de vie du jeton d'amorçage affiché sur la console (§6). Passé ce
/// délai il faut relancer le moteur : aucun jeton ne traîne sur disque.
pub const BOOTSTRAP_TTL: Duration = Duration::from_secs(300);

/// Durée de vie du jeton rendu par le canal fichier (fenêtre rouverte) : le
/// fichier de réponse ne reste lisible que deux minutes.
pub const BOOTSTRAP_FILE_TTL: Duration = Duration::from_secs(120);

/// Nom du cookie de session (§6).
pub const COOKIE_NAME: &str = "mcc_session";

/// En-tête secret de session (S-1). Nom personnalisé : l'envoyer impose un
/// préflight CORS, que le serveur refuse — c'est la protection CSRF, avec le
/// cookie `SameSite=Strict` et les contrôles Host/Origin.
pub const SESSION_HEADER: &str = "x-mcc-session";

/// Nombre maximal de sessions UI durables simultanées (une par fenêtre, borné).
pub const MAX_SESSIONS: usize = 8;

/// Réserve dédiée aux sessions CLI éphémères (`auth revoke`, `auth list`, …)
/// afin que la révocation d'urgence reste toujours possible même si 8 fenêtres
/// UI sont ouvertes (AU-C07).
pub const MAX_EPHEMERAL_SESSIONS: usize = 4;

/// Expiration à l'inactivité : une session sans requête autorisée pendant 12 h
/// est morte (une fenêtre oubliée ne vit pas indéfiniment).
pub const IDLE_TTL: Duration = Duration::from_secs(12 * 3600);

/// Expiration à l'inactivité d'une session CLI éphémère (60 s — filet de
/// sécurité si le process CLI est tué avant `POST /api/session/close`).
pub const EPHEMERAL_IDLE_TTL: Duration = Duration::from_secs(60);

/// Durée absolue : une session meurt 24 h après son échange, même utilisée.
pub const ABSOLUTE_TTL: Duration = Duration::from_secs(24 * 3600);

/// Fichiers du canal de session (dossier de `config.json`).
pub const REQUEST_FILE: &str = "mcc-session.req";
pub const GRANT_FILE: &str = "mcc-session.grant";

/// Jeton d'amorçage en attente d'échange.
struct Bootstrap {
    token: String,
    issued: Instant,
    ttl: Duration,
}

/// Session vivante.
struct Session {
    /// Valeur du cookie `mcc_session`.
    id: String,
    /// Secret gardé en mémoire par la page, présenté dans `X-MCC-Session`.
    secret: String,
    /// Émission : borne la durée absolue.
    issued: Instant,
    /// Dernière requête autorisée : borne l'inactivité.
    last_seen: Instant,
    /// Durée d'inactivité propre à la session (`IDLE_TTL` ou `EPHEMERAL_IDLE_TTL`).
    idle_ttl: Duration,
}

#[derive(Default)]
struct Slot {
    bootstraps: Vec<Bootstrap>,
    sessions: Vec<Session>,
}

fn slot() -> MutexGuard<'static, Slot> {
    static SLOT: OnceLock<Mutex<Slot>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(Slot::default()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

// ---------- journal (jamais de secret) ----------

static LOG_PATH: OnceLock<Mutex<Option<String>>> = OnceLock::new();

/// Branche le journal de session sur `mcc.log` (appelé au boot par le moteur).
/// Aucune ligne écrite ici ne contient de jeton ou de secret.
pub fn set_log_path(path: &str) {
    let cell = LOG_PATH.get_or_init(|| Mutex::new(None));
    *cell.lock().unwrap_or_else(|p| p.into_inner()) = Some(path.to_string());
}

/// Ligne de journal : stderr + `mcc.log` si le chemin est connu. Le message
/// est toujours construit par ce module — aucun secret n'y entre.
fn journal(level: &str, message: &str) {
    let line = format!(
        "[{}] [{}] [session] {}\n",
        crate::types::iso_utc(crate::types::now_ts()),
        level,
        message
    );
    eprint!("{line}");
    let path = LOG_PATH
        .get()
        .and_then(|cell| cell.lock().unwrap_or_else(|p| p.into_inner()).clone());
    if let Some(path) = path {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

// ---------- aléa ----------

/// base64url sans padding (RFC 4648 §5) — l'encodage des jetons de §6. Ce n'est
/// pas de la cryptographie, seulement une mise en forme.
fn b64url(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(A[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(A[n as usize & 63] as char);
        }
    }
    out
}

/// Sonde de test : force l'échec de la source OS (S-3). Jamais compilée hors
/// des tests de la bibliothèque.
#[cfg(test)]
static OS_RANDOM_FAILS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `bytes` octets du système, en base64url. **Aucun repli** (S-3) : source OS
/// indisponible = `None` — l'appelant refuse la session et journalise.
fn fresh_token(bytes: usize) -> Option<String> {
    let mut buf = vec![0u8; bytes];
    #[cfg(test)]
    if OS_RANDOM_FAILS.load(std::sync::atomic::Ordering::SeqCst) {
        journal(
            "error",
            "entropie OS indisponible (simulée) : aucun jeton, aucune session",
        );
        return None;
    }
    if !crate::os_random(&mut buf) {
        journal(
            "error",
            "entropie OS indisponible : aucun jeton, aucune session (échec fermé)",
        );
        return None;
    }
    Some(b64url(&buf))
}

fn is_token(s: &str) -> bool {
    s.len() == 43
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

// ---------- cycle de vie ----------

/// Une session est morte au-delà de la durée absolue ou de l'inactivité.
fn expired_with_idle(issued_ago: Duration, idle_ago: Duration, idle_ttl: Duration) -> bool {
    issued_ago >= ABSOLUTE_TTL || idle_ago >= idle_ttl
}

#[cfg(test)]
fn expired(issued_ago: Duration, idle_ago: Duration) -> bool {
    expired_with_idle(issued_ago, idle_ago, IDLE_TTL)
}

fn prune(s: &mut Slot) {
    let now = Instant::now();
    s.sessions.retain(|x| {
        !expired_with_idle(
            now.duration_since(x.issued),
            now.duration_since(x.last_seen),
            x.idle_ttl,
        )
    });
    s.bootstraps
        .retain(|b| now.duration_since(b.issued) < b.ttl);
}

/// Tire un jeton d'amorçage avec la durée de vie donnée et l'installe dans le
/// slot. `None` = entropie OS indisponible (S-3) : aucun jeton n'existe.
fn mint_bootstrap_with_ttl(ttl: Duration) -> Option<String> {
    let token = fresh_token(32)?;
    let mut s = slot();
    prune(&mut s);
    s.bootstraps.push(Bootstrap {
        token: token.clone(),
        issued: Instant::now(),
        ttl,
    });
    // Borne : au plus 16 jetons en attente (évite d'évincer des CLI concurrentes).
    while s.bootstraps.len() > 16 {
        s.bootstraps.remove(0);
    }
    Some(token)
}

/// Tire un nouveau jeton d'amorçage (32 octets, usage unique, 5 minutes).
pub fn mint_bootstrap() -> Option<String> {
    mint_bootstrap_with_ttl(BOOTSTRAP_TTL)
}

/// URL à usage unique à ouvrir dans la fenêtre ou le navigateur : le jeton vit
/// dans le **fragment**, qui n'est jamais transmis au serveur.
pub fn bootstrap_url(base: &str) -> Option<String> {
    let token = mint_bootstrap()?;
    Some(format!("{}/#t={}", base.trim_end_matches('/'), token))
}

/// Session rendue par un échange réussi.
pub struct Issued {
    /// Valeur du cookie `mcc_session`.
    pub cookie: String,
    /// Secret de session (mémoire de la page uniquement).
    pub secret: String,
}

/// Échange du jeton contre une session. `None` = jeton inconnu, déjà consommé,
/// périmé, plafond de sessions atteint ou entropie OS indisponible (échec
/// fermé : aucun cookie n'est émis).
pub fn exchange(token: &str) -> Option<Issued> {
    exchange_with_mode(token, false)
}

/// Variante avec `ephemeral = true` (sessions CLI courtes, TTL 60 s).
pub fn exchange_with_mode(token: &str, ephemeral: bool) -> Option<Issued> {
    {
        let mut s = slot();
        prune(&mut s);
        let idx = s
            .bootstraps
            .iter()
            .position(|b| crate::crypto::ct_eq(b.token.as_bytes(), token.as_bytes()))?;
        if s.bootstraps[idx].issued.elapsed() >= s.bootstraps[idx].ttl {
            s.bootstraps.remove(idx);
            return None;
        }
        // AU-C07 : vérifier le plafond AVANT de consommer le jeton d'amorçage ;
        // les sessions CLI éphémères disposent d'une réserve séparée pour que
        // `mcp-control auth revoke` ne soit jamais bloqué par 8 onglets UI.
        if !ephemeral {
            let durable_count = s
                .sessions
                .iter()
                .filter(|x| x.idle_ttl != EPHEMERAL_IDLE_TTL)
                .count();
            if durable_count >= MAX_SESSIONS {
                drop(s);
                journal(
                    "warn",
                    "session refusée : le plafond de 8 sessions simultanées est atteint (fermer une fenêtre ou « fermer les autres sessions »)",
                );
                return None;
            }
        }
        s.bootstraps.remove(idx);
    }
    // Usage unique : le jeton est consommé AVANT de tirer les secrets, hors du
    // verrou `SLOT` afin qu'aucune E/S de `journal()` ne soit exécutée verrou tenu.
    let id = fresh_token(32)?;
    let secret = fresh_token(32)?;
    let mut s = slot();
    prune(&mut s);
    if ephemeral {
        let eph_count = s
            .sessions
            .iter()
            .filter(|x| x.idle_ttl == EPHEMERAL_IDLE_TTL)
            .count();
        if eph_count >= MAX_EPHEMERAL_SESSIONS {
            if let Some(pos) = s
                .sessions
                .iter()
                .position(|x| x.idle_ttl == EPHEMERAL_IDLE_TTL)
            {
                s.sessions.remove(pos);
            }
        }
    } else {
        let durable_count = s
            .sessions
            .iter()
            .filter(|x| x.idle_ttl != EPHEMERAL_IDLE_TTL)
            .count();
        if durable_count >= MAX_SESSIONS {
            drop(s);
            journal(
                "warn",
                "session refusée : le plafond de 8 sessions simultanées est atteint (fermer une fenêtre ou « fermer les autres sessions »)",
            );
            return None;
        }
    }
    let now = Instant::now();
    s.sessions.push(Session {
        id: id.clone(),
        secret: secret.clone(),
        issued: now,
        last_seen: now,
        idle_ttl: if ephemeral {
            EPHEMERAL_IDLE_TTL
        } else {
            IDLE_TTL
        },
    });
    Some(Issued { cookie: id, secret })
}

/// Nombre d'occurrences du cookie `mcc_session=` dans tous les en-têtes `Cookie`.
/// Permet au sas HTTP de rejeter en `400 Bad Request` tout envoi ambigu (R-3).
pub fn cookie_count(headers: &[(String, String)]) -> usize {
    headers
        .iter()
        .filter(|(n, _)| n == "cookie")
        .flat_map(|(_, v)| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .filter(|(name, _)| name.trim() == COOKIE_NAME)
        .count()
}

/// Valeur du cookie `mcc_session` présentée par la requête, s'il y en a une
/// seule (aucune ambiguïté).
pub fn cookie_value(headers: &[(String, String)]) -> Option<String> {
    if cookie_count(headers) != 1 {
        return None;
    }
    headers
        .iter()
        .filter(|(n, _)| n == "cookie")
        .flat_map(|(_, v)| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| name.trim() == COOKIE_NAME)
        .map(|(_, value)| value.trim().to_string())
}

/// Valeur d'un en-tête unique (nom déjà en minuscules côté `handle_conn`).
/// Retourne `None` si l'en-tête est absent ou dupliqué.
fn header_value(headers: &[(String, String)], name: &str) -> Option<String> {
    let mut vals = headers
        .iter()
        .filter(|(n, _)| n == name)
        .map(|(_, v)| v.trim().to_string());
    let first = vals.next()?;
    if vals.next().is_some() {
        return None;
    }
    Some(first)
}

/// Session vivante désignée par la requête : cookie ET secret doivent
/// correspondre (comparaison en temps constant), et la session doit être dans
/// ses bornes. `last_seen` est rafraîchi sur succès.
pub fn authorized_session(headers: &[(String, String)]) -> Option<String> {
    let cookie = cookie_value(headers)?;
    let secret = header_value(headers, SESSION_HEADER)?;
    let mut s = slot();
    prune(&mut s);
    let now = Instant::now();
    let mut idx = None;
    for (i, sess) in s.sessions.iter().enumerate() {
        if crate::crypto::ct_eq(sess.id.as_bytes(), cookie.as_bytes()) {
            idx = Some(i);
            break;
        }
    }
    let i = idx?;
    if !crate::crypto::ct_eq(s.sessions[i].secret.as_bytes(), secret.as_bytes()) {
        return None;
    }
    s.sessions[i].last_seen = now;
    Some(s.sessions[i].id.clone())
}

/// La requête porte-t-elle une session valide ? (`false` = cookie absent, secret
/// absent ou faux, session expirée ou d'un moteur précédent.)
pub fn authorized(headers: &[(String, String)]) -> bool {
    authorized_session(headers).is_some()
}

/// Nombre de sessions locales actives (dans leurs bornes d'inactivité et de
/// durée absolue). Lecture seule : aucun jeton, cookie ni secret n'est exposé.
pub fn active_count() -> usize {
    let s = slot();
    let now = Instant::now();
    s.sessions
        .iter()
        .filter(|x| {
            !expired_with_idle(
                now.duration_since(x.issued),
                now.duration_since(x.last_seen),
                x.idle_ttl,
            )
        })
        .count()
}

/// Révoque uniquement la session courante (utilisé par les commandes CLI
/// éphémères dès la fin de leur appel, évitant de saturer le plafond de 8).
pub fn close_session(current: &str) -> bool {
    let mut s = slot();
    prune(&mut s);
    let before = s.sessions.len();
    s.sessions
        .retain(|x| !crate::crypto::ct_eq(x.id.as_bytes(), current.as_bytes()));
    s.sessions.len() < before
}

/// Révoque toutes les sessions SAUF celle désignée par `current`. Rend le
/// nombre de sessions fermées (`None` = session courante invalide).
pub fn close_others(current: &str) -> Option<usize> {
    let mut s = slot();
    prune(&mut s);
    if !s
        .sessions
        .iter()
        .any(|x| crate::crypto::ct_eq(x.id.as_bytes(), current.as_bytes()))
    {
        return None;
    }
    let before = s.sessions.len();
    s.sessions
        .retain(|x| crate::crypto::ct_eq(x.id.as_bytes(), current.as_bytes()));
    Some(before - s.sessions.len())
}

/// Ligne `Set-Cookie` de la session (§6) : `HttpOnly`, `SameSite=Strict`,
/// `Path=/`, sans `Max-Age` (cookie de session) et sans `Secure` (HTTP
/// loopback). Aucune valeur n'est journalisée.
pub fn set_cookie_header(id: &str) -> String {
    format!("{COOKIE_NAME}={id}; HttpOnly; SameSite=Strict; Path=/")
}

// ---------- canal fichier (fenêtre rouverte sur un moteur déjà lancé) ----------
//
// Le second lancement ne peut pas parler au moteur (le port loopback n'est plus
// une zone de confiance, c'est tout l'objet de S-1). Le rendez-vous passe par
// deux fichiers du dossier de `config.json` : la demande ne porte qu'un nonce,
// la réponse un jeton à usage unique. Les deux sont restreints au compte
// courant (`restrict_permissions`) et supprimés dès lecture ; le jeton n'est
// jamais journalisé. Aucun autre compte ne peut écrire ni lire ces fichiers
// (dossier de configuration de l'utilisateur, ACL explicite).

fn sidecar(config_path: &str, name: &str) -> std::path::PathBuf {
    Path::new(config_path).with_file_name(name)
}

/// Crée le fichier, le restreint au compte courant, PUIS y écrit `bytes` :
/// aucun octet secret n'existe avec les ACL héritées. Si la restriction échoue,
/// le fichier est supprimé : aucun jeton ne reste lisible par un autre compte
/// (échec fermé).
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    // Fichier temporaire créé VIDE, restreint, rempli + sync_all, PUIS renommé
    // atomiquement : aucun octet secret n'existe avec les ACL héritées ET le
    // lecteur (`serve_session_requests` / `request_bootstrap`) ne voit jamais
    // un fichier vide ou tronqué en cours d'écriture.
    let mut tmp_os = path.as_os_str().to_os_string();
    tmp_os.push(format!(".{}.tmp", std::process::id()));
    let tmp = std::path::PathBuf::from(tmp_os);
    std::fs::File::create(&tmp)?;
    if let Err(e) = crate::manage::restrict_permissions(&tmp.to_string_lossy()) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    let write_res = (|| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        let _ = f.sync_all();
        Ok(())
    })();
    if let Err(e) = write_res {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Supprime un résidu de canal plus vieux que la durée de vie du jeton : un
/// fichier d'une exécution précédente ne doit jamais être servi.
fn remove_stale(path: &Path) {
    if let Ok(meta) = std::fs::metadata(path) {
        if meta
            .modified()
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > BOOTSTRAP_FILE_TTL)
        {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Côté MOTEUR : sert les demandes de jeton déposées par un second lancement ou
/// par les sous-commandes CLI. Thread dédié, une passe toutes les 200 ms.
/// Chaque demande utilise son propre couple `mcc-session-<id>.req` / `.grant`
/// (zéro course entre plusieurs CLI simultanées) ; la forme héritée
/// `mcc-session.req` / `.grant` reste également servie.
pub fn serve_session_requests(config_path: &str) {
    serve_session_requests_on_port(config_path, 0);
}

/// Variante scellant le port HTTP loopback réellement écouté (`ui_port > 0`)
/// sur la 3ᵉ ligne de `.grant` (protection anti-squat / repli `+1..+9`, AU-C02).
pub fn serve_session_requests_on_port(config_path: &str, ui_port: u16) {
    let config_path = config_path.to_string();
    std::thread::spawn(move || {
        let dir = Path::new(&config_path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let legacy_req = sidecar(&config_path, REQUEST_FILE);
        let legacy_grant = sidecar(&config_path, GRANT_FILE);
        remove_stale(&legacy_req);
        remove_stale(&legacy_grant);
        journal(
            "info",
            "canal de session fichier utilisateur actif (demande → jeton à usage unique 2 min, permissions utilisateur seul)",
        );
        loop {
            std::thread::sleep(Duration::from_millis(200));
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let fname = entry.file_name().to_string_lossy().to_string();
                let path = entry.path();
                if fname.starts_with("mcc-session") && fname.ends_with(".grant") {
                    remove_stale(&path);
                    continue;
                }
                let grant_path = if fname == REQUEST_FILE {
                    legacy_grant.clone()
                } else if let Some(rest) = fname.strip_prefix("mcc-session-") {
                    let Some(req_id) = rest.strip_suffix(".req") else {
                        continue;
                    };
                    if req_id.is_empty()
                        || req_id.len() > 32
                        || !req_id.chars().all(|c| c.is_ascii_hexdigit())
                    {
                        remove_stale(&path);
                        continue;
                    }
                    dir.join(format!("mcc-session-{req_id}.grant"))
                } else {
                    continue;
                };
                let Ok(texte) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let nonce = texte.trim().to_string();
                if !is_token(&nonce) {
                    let _ = std::fs::remove_file(&path);
                    journal(
                        "warn",
                        "demande de session ignorée (format de nonce invalide)",
                    );
                    continue;
                }
                // La demande est retirée AVANT de répondre : une seule réponse par demande.
                if std::fs::remove_file(&path).is_err() {
                    continue;
                }
                match mint_bootstrap_with_ttl(BOOTSTRAP_FILE_TTL) {
                    Some(token) => {
                        let corps = if ui_port > 0 {
                            format!("{nonce}\n{token}\n{ui_port}\n")
                        } else {
                            format!("{nonce}\n{token}\n")
                        };
                        match write_private(&grant_path, corps.as_bytes()) {
                            Ok(()) => journal(
                                "info",
                                "demande de session servie (canal fichier utilisateur, jeton à usage unique 2 min)",
                            ),
                            Err(e) => journal(
                                "warn",
                                &format!("réponse de session non écrite : {e}"),
                            ),
                        }
                    }
                    None => journal(
                        "error",
                        "demande de session refusée : entropie OS indisponible (aucun jeton)",
                    ),
                }
            }
        }
    });
}

/// Côté SHELL / CLI : demande un jeton d'amorçage au moteur par un couple
/// dédié `mcc-session-<id>.req` / `.grant` (sans course entre appels concurrents),
/// en retournant aussi le port loopback authentifié si le moteur l'a scellé.
pub fn request_bootstrap_with_port(
    config_path: &str,
    timeout: Duration,
) -> Result<(String, Option<u16>), String> {
    let nonce = fresh_token(32)
        .ok_or_else(|| "entropie OS indisponible : aucun jeton de session".to_string())?;
    let req_id = crate::random_hex(1)
        .ok_or_else(|| "entropie OS indisponible : aucun identifiant de demande".to_string())?;
    let req = sidecar(config_path, &format!("mcc-session-{req_id}.req"));
    let grant = sidecar(config_path, &format!("mcc-session-{req_id}.grant"));
    write_private(&req, nonce.as_bytes())
        .map_err(|e| format!("écriture de la demande de session impossible : {e}"))?;
    let fin = Instant::now() + timeout;
    loop {
        if let Ok(texte) = std::fs::read_to_string(&grant) {
            let mut lignes = texte.lines().map(str::trim);
            if lignes.next() == Some(nonce.as_str()) {
                if let Some(token) = lignes.next() {
                    let port = lignes
                        .next()
                        .and_then(|s| s.parse::<u16>().ok())
                        .filter(|&p| p > 0);
                    let _ = std::fs::remove_file(&grant);
                    if is_token(token) {
                        return Ok((token.to_string(), port));
                    }
                }
            }
        }
        if Instant::now() >= fin {
            let _ = std::fs::remove_file(&req);
            let _ = std::fs::remove_file(&grant);
            return Err(
                "aucune réponse du moteur au canal fichier (moteur antérieur au lot S1 ?)"
                    .to_string(),
            );
        }
        std::thread::sleep(Duration::from_millis(80));
    }
}

/// Côté SHELL / CLI : demande un jeton d'amorçage au moteur par un couple
/// dédié `mcc-session-<id>.req` / `.grant` (sans course entre appels concurrents).
pub fn request_bootstrap(config_path: &str, timeout: Duration) -> Result<String, String> {
    request_bootstrap_with_port(config_path, timeout).map(|(tok, _)| tok)
}

/// Réinitialise l'état (tests unitaires uniquement : un process de test ne doit
/// pas hériter du jeton d'un autre).
#[cfg(test)]
pub fn reset_for_tests() {
    let mut s = slot();
    s.bootstraps.clear();
    s.sessions.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    /// Les tests partagent l'état global du module : ils se sérialisent (cause
    /// de l'instabilité corrigée au lot S1 — deux tests échangeaient en
    /// parallèle et se volaient leurs sessions).
    fn guard() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let g = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        reset_for_tests();
        OS_RANDOM_FAILS.store(false, Ordering::SeqCst);
        g
    }

    fn hs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect()
    }

    fn open() -> Issued {
        exchange(&mint_bootstrap().expect("jeton")).expect("session")
    }

    fn with_session(cookie: &str, secret: &str) -> Vec<(String, String)> {
        hs(&[
            ("cookie", &format!("a=1; mcc_session={cookie}; b=2")),
            (SESSION_HEADER, secret),
        ])
    }

    #[test]
    fn base64url_forme_rfc_4648() {
        assert_eq!(b64url(b""), "");
        assert_eq!(b64url(b"f"), "Zg");
        assert_eq!(b64url(b"fo"), "Zm8");
        assert_eq!(b64url(b"foo"), "Zm9v");
        // 32 octets → 43 caractères, sans padding, alphabet url
        let t = b64url(&[0xff; 32]);
        assert_eq!(t.len(), 43, "{t}");
        assert!(t
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert!(!t.contains('='));
    }

    #[test]
    fn jeton_amorcage_32_octets_et_usage_unique() {
        let _g = guard();
        let token = mint_bootstrap().expect("jeton");
        assert_eq!(
            token.len(),
            43,
            "32 octets base64url = 43 caractères : {token}"
        );
        // premier échange : cookie + secret émis
        let s = exchange(&token).expect("jeton frais accepté");
        assert_eq!(s.cookie.len(), 43);
        assert_eq!(s.secret.len(), 43);
        // second échange du même jeton : refusé (usage unique)
        assert!(exchange(&token).is_none(), "jeton déjà consommé");
        // jeton inventé : refusé
        assert!(exchange(&"A".repeat(43)).is_none());
        // jeton vide : refusé
        assert!(exchange("").is_none());
    }

    #[test]
    fn cookie_et_secret_exiges_tous_les_deux() {
        let _g = guard();
        let s = open();
        let cookie = s.cookie.clone();
        let secret = s.secret.clone();
        // les deux → 200
        assert!(authorized(&with_session(&cookie, &secret)));
        // cookie seul → refus (S-1)
        assert!(
            !authorized(&hs(&[("cookie", &format!("mcc_session={cookie}"))])),
            "un cookie seul ne doit plus ouvrir /api/*"
        );
        // bon cookie + mauvais secret → refus
        assert!(!authorized(&with_session(&cookie, &"B".repeat(43))));
        // bon secret + mauvais cookie → refus
        assert!(!authorized(&with_session(&"C".repeat(43), &secret)));
        // secret absent, cookie d'un autre moteur, absence de tout
        assert!(!authorized(&hs(&[("cookie", "autre=1")])));
        assert!(!authorized(&[]));
        // le secret n'est jamais confondu avec le cookie
        assert!(!authorized(&hs(&[
            ("cookie", &format!("mcc_session={secret}")),
            (SESSION_HEADER, &cookie)
        ])));
    }

    #[test]
    fn plusieurs_sessions_simultanees_sans_invalider_les_precedentes() {
        let _g = guard();
        assert_eq!(active_count(), 0);
        let a = open();
        assert_eq!(active_count(), 1);
        let b = open();
        assert_eq!(active_count(), 2);
        assert_ne!(a.cookie, b.cookie);
        // la 2e fenêtre n'a pas invalidé la 1re
        assert!(authorized(&with_session(&a.cookie, &a.secret)));
        assert!(authorized(&with_session(&b.cookie, &b.secret)));
    }

    #[test]
    fn plafond_de_huit_sessions() {
        let _g = guard();
        let mut sessions = Vec::new();
        for _ in 0..MAX_SESSIONS {
            sessions.push(open());
        }
        for s in &sessions {
            assert!(authorized(&with_session(&s.cookie, &s.secret)));
        }
        // 9e session : refusée, mais les 8 premières restent vivantes
        let token = mint_bootstrap().expect("jeton");
        assert!(exchange(&token).is_none(), "plafond de 8 sessions");
        for s in &sessions {
            assert!(authorized(&with_session(&s.cookie, &s.secret)));
        }
    }

    #[test]
    fn fermer_les_autres_sessions_garde_la_courante() {
        let _g = guard();
        let courante = open();
        let autre1 = open();
        let autre2 = open();
        assert_eq!(active_count(), 3);
        assert_eq!(close_others(&courante.cookie), Some(2));
        assert_eq!(active_count(), 1);
        assert!(authorized(&with_session(
            &courante.cookie,
            &courante.secret
        )));
        assert!(!authorized(&with_session(&autre1.cookie, &autre1.secret)));
        assert!(!authorized(&with_session(&autre2.cookie, &autre2.secret)));
        // id inconnu / expiré : refus (aucune révocation au hasard)
        assert_eq!(close_others(&"D".repeat(43)), None);
    }

    #[test]
    fn bornes_de_vie_inactivite_et_duree_absolue() {
        // Expiration : pure, testable sans attendre 12 h.
        assert!(!expired(Duration::from_secs(0), Duration::from_secs(0)));
        assert!(!expired(
            ABSOLUTE_TTL - Duration::from_secs(1),
            IDLE_TTL / 2
        ));
        assert!(expired(ABSOLUTE_TTL, Duration::from_secs(0)));
        assert!(expired(Duration::from_secs(0), IDLE_TTL));
        assert!(!expired(
            Duration::from_secs(24 * 3600 - 1),
            Duration::from_secs(12 * 3600 - 1)
        ));
    }

    #[test]
    fn entropie_os_indisponible_refuse_jeton_et_session() {
        let _g = guard();
        OS_RANDOM_FAILS.store(true, Ordering::SeqCst);
        assert!(mint_bootstrap().is_none(), "aucun jeton sans aléa OS");
        assert!(bootstrap_url("http://127.0.0.1:5090").is_none());
        assert!(
            exchange(&"A".repeat(43)).is_none(),
            "aucune session sans aléa OS"
        );
        assert!(!authorized(&hs(&[
            ("cookie", "mcc_session=x"),
            (SESSION_HEADER, "y")
        ])));
        OS_RANDOM_FAILS.store(false, Ordering::SeqCst);
        // retour de l'aléa : une session est de nouveau possible
        assert!(mint_bootstrap().is_some());
    }

    #[test]
    fn set_cookie_porte_les_attributs_de_la_spec() {
        let c = set_cookie_header("valeur");
        for attendu in [
            "mcc_session=valeur",
            "HttpOnly",
            "SameSite=Strict",
            "Path=/",
        ] {
            assert!(c.contains(attendu), "{attendu} manquant dans {c}");
        }
        // cookie de session : pas de Max-Age ; HTTP loopback : pas de Secure
        assert!(!c.contains("Max-Age"));
        assert!(!c.contains("Secure"));
    }

    #[test]
    fn url_amorcage_met_le_jeton_dans_le_fragment() {
        let _g = guard();
        let url = bootstrap_url("http://127.0.0.1:5090").expect("url");
        assert!(url.starts_with("http://127.0.0.1:5090/#t="), "{url}");
        let token = url.rsplit("#t=").next().unwrap_or("").to_string();
        assert_eq!(token.len(), 43);
        // le fragment n'est jamais envoyé au serveur : aucun jeton dans la query
        assert!(!url.contains("?"));
        assert!(exchange(&token).is_some());
        // la barre oblique finale est absorbée (pas de double slash)
        let url2 = bootstrap_url("http://127.0.0.1:5090/").expect("url");
        assert!(url2.contains(":5090/#t="), "{url2}");
    }

    #[test]
    fn canal_fichier_sert_un_jeton_a_usage_unique() {
        let _g = guard();
        let dir = std::env::temp_dir().join(format!(
            "mcc-session-canal-{}-{}",
            std::process::id(),
            crate::types::now_ts()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("config.json").to_string_lossy().to_string();
        serve_session_requests(&config);
        let token = request_bootstrap(&config, Duration::from_secs(10)).expect("jeton du canal");
        assert_eq!(token.len(), 43);
        // la réponse a été consommée : le fichier de réponse a disparu
        assert!(!dir.join(GRANT_FILE).exists());
        assert!(!dir.join(REQUEST_FILE).exists());
        // le jeton s'échange normalement, une seule fois
        assert!(exchange(&token).is_some());
        assert!(exchange(&token).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn canal_fichier_scelle_le_port_authentifie() {
        let _g = guard();
        let dir = std::env::temp_dir().join(format!(
            "mcc-session-port-{}-{}",
            std::process::id(),
            crate::types::now_ts()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("config.json").to_string_lossy().to_string();
        serve_session_requests_on_port(&config, 5094);
        let (token, port) =
            request_bootstrap_with_port(&config, Duration::from_secs(10)).expect("grant avec port");
        assert_eq!(token.len(), 43);
        assert_eq!(port, Some(5094));
        assert!(exchange(&token).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn doublons_cookie_et_en_tete_session_refuses() {
        let _g = guard();
        let s = open();
        assert_eq!(
            cookie_count(&hs(&[
                ("cookie", &format!("mcc_session={}", s.cookie)),
                ("cookie", "mcc_session=autre"),
            ])),
            2
        );
        assert!(!authorized(&hs(&[
            ("cookie", &format!("mcc_session={}", s.cookie)),
            ("cookie", "mcc_session=autre"),
            (SESSION_HEADER, &s.secret),
        ])));
        assert!(!authorized(&hs(&[
            (
                "cookie",
                &format!("mcc_session={}; mcc_session=autre", s.cookie)
            ),
            (SESSION_HEADER, &s.secret),
        ])));
        assert!(!authorized(&hs(&[
            ("cookie", &format!("mcc_session={}", s.cookie)),
            (SESSION_HEADER, &s.secret),
            (SESSION_HEADER, "autre"),
        ])));
    }
}
