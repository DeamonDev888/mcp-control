pub mod audit;
pub mod auth;
pub mod bridge;
pub mod crypto;
pub mod frameworks;
pub mod gateway;
pub mod logos;
pub mod manage;
pub mod notify;
pub mod probe;
pub mod runtime;
pub mod store_aitmpl;
pub mod store_github;
pub mod store_glama;
pub mod types;
pub mod ui;
pub mod validate;

use crate::types::{lockv, AppState};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Corps de requête max (anti-DoS : l'ancien code suivait un Content-Length
/// choisi par le client sans plafond → gonflement mémoire infini).
const MAX_BODY: usize = 2 * 1024 * 1024;
/// Connexions simultanées max (au-delà : refus EXPLICITE en 503, cf. admit_conn).
const MAX_CONNS: usize = 64;
/// Part de MAX_CONNS réservée aux connexions LOCALES : le LAN ne peut jamais
/// occuper plus de MAX_CONNS − LOOPBACK_RESERVE slots, la fenêtre locale
/// (desktop, navigateur de la machine) reste joignable quoi qu'il arrive
/// (audit 05 #1 : 64 sockets muettes du LAN coupaient tout, y compris local).
const LOOPBACK_RESERVE: usize = 16;
/// Connexions simultanées max par adresse IP HORS loopback : une seule machine
/// du LAN ne peut plus, à elle seule, saturer la part non réservée.
const MAX_CONNS_PER_ADDR: usize = 16;
/// Échéance TOTALE d'une requête (en-têtes PUIS corps) : au-delà, la connexion
/// est fermée. L'ancien read timeout de 30 s se réarmait à chaque octet reçu —
/// un client muet-lent monopolisait un thread indéfiniment (slowloris).
const REQUEST_DEADLINE: Duration = Duration::from_secs(5);
/// CSP de la page UI (servie en en-tête HTTP par CE serveur — le webview Tauri
/// charge une URL externe, la config `csp` de tauri.conf.json ne s'y applique
/// pas). Lot S2 : plus AUCUN script ni style en ligne — la page ne porte que
/// des attributs data-act traités par la délégation d'événements de app.js,
/// et les couleurs dynamiques sont des classes CSS. 'self' seul, ni
/// 'unsafe-inline' ni 'unsafe-eval', aucun nonce de contournement. Une XSS
/// future ne peut ni s'exécuter en ligne, ni charger du code externe, ni
/// exfiltrer (connect-src 'self').
const UI_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self' ipc: http://ipc.localhost; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
/// En-têtes de durcissement appliqués à TOUTES les réponses HTTP (HTML, JSON,
/// assets et refus précoces 413/503).
const SECURITY_HEADERS: &str = "X-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nCross-Origin-Opener-Policy: same-origin\r\nCross-Origin-Resource-Policy: same-origin\r\n";

/// Version de spec annoncée par litemcp — premier signal du fingerprint :
/// litemcp répond initialize avec 2026-07-28 même quand le client demande
/// 2024-11-05 (stateless strict, pas de négociation).
pub const LITEMCP_PROTOCOL: &str = "2026-07-28";

static OPEN_CONNS: AtomicUsize = AtomicUsize::new(0);
/// Connexions simultanées ouvertes par des clients HORS loopback — comparé à
/// MAX_CONNS − LOOPBACK_RESERVE pour que le LAN ne prenne jamais la part locale.
static OPEN_LAN_CONNS: AtomicUsize = AtomicUsize::new(0);

/// Connexions ouvertes par adresse IP (hors loopback uniquement).
fn per_addr_slot() -> &'static Mutex<HashMap<IpAddr, usize>> {
    static SLOT: std::sync::OnceLock<Mutex<HashMap<IpAddr, usize>>> = std::sync::OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Garde de connexion : relâche TOUTES les réservations (total, hors-loopback,
/// par adresse) quand la connexion se termine — y compris sur panic du handler.
struct ConnGuard {
    lan: bool,
    addr: Option<IpAddr>,
}
impl Drop for ConnGuard {
    fn drop(&mut self) {
        OPEN_CONNS.fetch_sub(1, Ordering::SeqCst);
        if self.lan {
            OPEN_LAN_CONNS.fetch_sub(1, Ordering::SeqCst);
            if let Some(addr) = self.addr.take() {
                let mut m = per_addr_slot().lock().unwrap_or_else(|p| p.into_inner());
                if let Some(c) = m.get_mut(&addr) {
                    *c -= 1;
                    if *c == 0 {
                        m.remove(&addr);
                    }
                }
            }
        }
    }
}

/// Décision d'admission d'une connexion TCP entrante (pure, testable). Les
/// compteurs incluent DÉJÀ la connexion candidate :
/// - total > MAX_CONNS → saturation ;
/// - hors loopback : part locale réservée — le LAN s'arrête à
///   MAX_CONNS − LOOPBACK_RESERVE (la fenêtre locale reste joignable) ;
/// - hors loopback : plafond par adresse ; le loopback est exempté du plafond
///   par adresse et de la réserve (machine de confiance, même utilisateur).
pub(crate) fn conn_admission(
    total_open: usize,
    lan_open: usize,
    addr_open: usize,
    peer_loopback: bool,
) -> Result<(), &'static str> {
    if total_open > MAX_CONNS {
        return Err(
            "serveur saturé (plafond de connexions simultanées) — réessayez dans un instant",
        );
    }
    if peer_loopback {
        return Ok(());
    }
    if lan_open > MAX_CONNS - LOOPBACK_RESERVE {
        return Err(
            "plafond de connexions hors loopback atteint (part réservée aux clients locaux)",
        );
    }
    if addr_open > MAX_CONNS_PER_ADDR {
        return Err("trop de connexions simultanées depuis votre adresse");
    }
    Ok(())
}

/// Réservation comptable puis décision : chaque plafond est vérifié au fur et
/// à mesure, avec annulation propre des réservations déjà prises en cas de
/// refus. La réservation totale est atomique (pas de course entre threads).
fn admit_conn(peer_ip: Option<IpAddr>) -> Result<ConnGuard, &'static str> {
    let peer_loopback = peer_ip.is_none_or(|ip| ip.is_loopback());
    let total = OPEN_CONNS.fetch_add(1, Ordering::SeqCst) + 1;
    if let Err(r) = conn_admission(total, 0, 0, peer_loopback) {
        OPEN_CONNS.fetch_sub(1, Ordering::SeqCst);
        return Err(r);
    }
    if peer_loopback {
        return Ok(ConnGuard {
            lan: false,
            addr: None,
        });
    }
    let Some(addr) = peer_ip else {
        OPEN_CONNS.fetch_sub(1, Ordering::SeqCst);
        return Err("adresse client indéterminable");
    };
    let lan = OPEN_LAN_CONNS.fetch_add(1, Ordering::SeqCst) + 1;
    if let Err(r) = conn_admission(total, lan, 0, false) {
        OPEN_LAN_CONNS.fetch_sub(1, Ordering::SeqCst);
        OPEN_CONNS.fetch_sub(1, Ordering::SeqCst);
        return Err(r);
    }
    let after_addr = {
        let mut m = per_addr_slot().lock().unwrap_or_else(|p| p.into_inner());
        let c = m.entry(addr).or_insert(0);
        *c += 1;
        *c
    };
    if let Err(r) = conn_admission(total, lan, after_addr, false) {
        {
            let mut m = per_addr_slot().lock().unwrap_or_else(|p| p.into_inner());
            if let Some(c) = m.get_mut(&addr) {
                *c -= 1;
                if *c == 0 {
                    m.remove(&addr);
                }
            }
        }
        OPEN_LAN_CONNS.fetch_sub(1, Ordering::SeqCst);
        OPEN_CONNS.fetch_sub(1, Ordering::SeqCst);
        return Err(r);
    }
    Ok(ConnGuard {
        lan: true,
        addr: Some(addr),
    })
}

/// Réponse de refus explicite (503) écrite directement par la boucle d'accept,
/// sans thread dédié — plus de fermeture muette : le client voit POURQUOI.
fn write_conn_rejected(mut stream: TcpStream, status: u16, message: &str) {
    let j = serde_json::json!({"ok": false, "error": message}).to_string();
    let resp = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{}Cache-Control: no-store\r\nConnection: close\r\n\r\n{}",
        status,
        reason_phrase(status),
        j.len(),
        SECURITY_HEADERS,
        j
    );
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
    // Drain NON bloquant des octets de requête déjà arrivés : fermer avec des
    // données non lues dans le tampon de réception fait émettre un RST par
    // Windows — le client voit alors « connection reset » AU LIEU du 503
    // (constaté avec curl pendant le lot 5). En avalant ce qui est déjà là,
    // la fermeture reste un FIN propre et le 503 est lisible. Borne 16×4 KiB :
    // ni blocage de la boucle d'accept, ni lecture illimitée d'un client
    // hostile (au-delà, le RST résiduel reste acceptable).
    let _ = stream.set_nonblocking(true);
    let mut poubelle = [0u8; 4096];
    for _ in 0..16 {
        match stream.read(&mut poubelle) {
            Ok(0) | Err(_) => break,
            Ok(_) => continue,
        }
    }
}

/// Vrai si le bind n'expose que la machine locale.
pub fn is_loopback_bind(s: &str) -> bool {
    matches!(s, "127.0.0.1" | "localhost" | "::1" | "[::1]")
}

/// Le code d'accès est-il exigé pour CETTE connexion ? Uniquement quand l'UI
/// est exposée au réseau (bind non-loopback) ET que le client vient du réseau :
/// les connexions locales (fenêtre desktop, navigateur de la machine) n'ont
/// JAMAIS de code à saisir. Les contrôles Host/Origin (anti CSRF/rebinding)
/// restent appliqués à toutes les connexions, locales incluses.
///
/// **Lot M2 (DA2 = a)** : cette exemption locale n'est plus une autorisation.
/// Le code hérité ne sert plus qu'à la tolérance en LECTURE SEULE de §10 ;
/// l'accès à `/api/*` passe par la session locale (`auth::session`) et `/mcp`
/// par une signature, loopback compris. La fonction reste exportée parce que
/// c'est elle qui décrit la portée historique du code partagé (§10) — elle
/// n'est plus consultée par `check_access`.
pub fn effective_require_access(bind_is_loopback: bool, peer_is_loopback: bool) -> bool {
    !bind_is_loopback && !peer_is_loopback
}

/// Hex aléatoire de `words` × 64 bits — CSPRNG de l'OS exclusivement (R-7).
/// Aucun repli PRNG non cryptographique : si la source OS échoue, retourne
/// `None` (échec fermé) pour que les appelants renvoient `503`.
fn random_hex_with<F: FnMut(&mut [u8]) -> bool>(words: usize, mut rng: F) -> Option<String> {
    let mut out = String::with_capacity(words * 16);
    let mut buf = [0u8; 8];
    for _ in 0..words {
        if !rng(&mut buf) {
            return None;
        }
        out.push_str(&format!("{:016x}", u64::from_le_bytes(buf)));
    }
    Some(out)
}

pub(crate) fn random_hex(words: usize) -> Option<String> {
    random_hex_with(words, os_random)
}

/// Remplit `buf` avec des octets cryptographiquement aléatoires de l'OS.
/// `true` = rempli ; `false` = source indisponible (l'appelant a son repli).
/// `pub(crate)` depuis le lot M2 : les jetons d'amorçage et le cookie de session
/// (`auth::session`) tirent 32 octets ici — même source que les nonces.
pub(crate) fn os_random(buf: &mut [u8]) -> bool {
    #[cfg(windows)]
    {
        #[link(name = "bcrypt")]
        extern "system" {
            fn BCryptGenRandom(
                hAlgorithm: usize,
                pBuffer: *mut u8,
                cbBuffer: u32,
                dwFlags: u32,
            ) -> i32;
        }
        // BCRYPT_USE_SYSTEM_PREFERRED_RNG : le RNG préféré du système.
        // NTSTATUS 0 = succès.
        unsafe { BCryptGenRandom(0, buf.as_mut_ptr(), buf.len() as u32, 0x0000_0002) == 0 }
    }
    #[cfg(not(windows))]
    {
        use std::io::Read;
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(buf))
            .is_ok()
    }
}

/// Code d'accès aléatoire 128 bits (hex) — conservé pour le test unitaire
/// d'entropie ; plus aucun code partagé n'est généré au démarrage (docs/auth.md §10).
#[cfg_attr(not(test), allow(dead_code))]
fn generate_access() -> String {
    random_hex(2).expect("entropie OS")
}

// ---------- preuve de pair (audit 2026-10-24 S-B) ----------
//
// Avant : discover_peers envoyait NOTRE code d'accès en clair (X-MCC-Access) à
// toutes les cibles sondées du LAN — n'importe quel hôte à l'écoute récupérait
// le secret. Désormais : le sondeur récupère un nonce à usage unique sur
// GET /api/hello (préauthentifié, aucune donnée) et signe ce nonce avec le code
// (HMAC-SHA256, cf. crypto.rs). Le pair vérifie la signature avec SA copie du
// code : un observateur réseau ne récupère qu'une preuve morte après usage.

/// Nonces émis par GET /api/hello → date d'ÉMISSION (l'échéance est
/// émission + HELLO_NONCE_TTL ; usage unique conservé via la consommation).
fn hello_nonces_slot() -> &'static std::sync::Mutex<std::collections::HashMap<String, u64>> {
    static SLOT: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, u64>>> =
        std::sync::OnceLock::new();
    SLOT.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

const HELLO_NONCE_TTL: u64 = 120;
const HELLO_NONCE_MAX: usize = 1024;
/// Limiteur de débit /api/hello par adresse (hors loopback) : sans lui, une
/// adresse peut à elle seule remplir la table et noyer les nonces des pairs
/// (audit 05 #6 : 1 100 requêtes en 1,1 s invalidaient toutes les preuves).
const HELLO_RATE_MAX: usize = 30;
/// Fenêtre glissante du limiteur de débit (secondes).
const HELLO_RATE_WINDOW_SECS: u64 = 60;

fn issue_hello_nonce() -> Option<String> {
    let nonce = random_hex(4)?;
    let mut slot = hello_nonces_slot()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let now = crate::types::now_ts();
    slot.retain(|_, issued| *issued + HELLO_NONCE_TTL > now);
    if slot.len() >= HELLO_NONCE_MAX {
        // Inondation : on évince les plus ANCIENTS au lieu de vider la table
        // (l'ancien slot.clear() invalidait TOUTES les preuves en vol) — les
        // preuves récentes des autres adresses restent valables.
        evict_oldest_nonces(&mut slot, HELLO_NONCE_MAX - 1);
    }
    slot.insert(nonce.clone(), now);
    Some(nonce)
}

/// Évince les nonces émis le plus anciennement jusqu'à repasser sous `max`.
/// Renvoie le nombre d'entrées évincées (pur, testable sans état global).
fn evict_oldest_nonces(map: &mut HashMap<String, u64>, max: usize) -> usize {
    let mut evicted = 0;
    while map.len() > max {
        let Some(oldest) = map
            .iter()
            .min_by_key(|(_, issued)| **issued)
            .map(|(k, _)| k.clone())
        else {
            break;
        };
        map.remove(&oldest);
        evicted += 1;
    }
    evicted
}

fn hello_rate_slot() -> &'static Mutex<HashMap<IpAddr, Vec<u64>>> {
    static SLOT: std::sync::OnceLock<Mutex<HashMap<IpAddr, Vec<u64>>>> = std::sync::OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Décision pure d'un limiteur à fenêtre glissante : à partir des horodatages
/// précédents, renvoie (décision, liste à conserver). Partagé par `/api/hello`
/// (30 req/min) et `/auth/pair` (5 req/min) — même algorithme, seaux distincts.
pub(crate) fn rate_decide(
    now: u64,
    prior: &[u64],
    max: usize,
    window_secs: u64,
) -> (Result<(), ()>, Vec<u64>) {
    let vivants: Vec<u64> = prior
        .iter()
        .copied()
        .filter(|t| now.saturating_sub(*t) < window_secs)
        .collect();
    if vivants.len() >= max {
        return (Err(()), vivants);
    }
    let mut next = vivants;
    next.push(now);
    (Ok(()), next)
}

/// Décision pure du limiteur de débit /api/hello : à partir des horodatages
/// précédents, renvoie (décision, liste à conserver). La liste est d'abord purgée
/// des timestamps hors fenêtre, puis comparée à `HELLO_RATE_MAX`.
fn hello_rate_decide(now: u64, prior: &[u64]) -> (Result<(), ()>, Vec<u64>) {
    rate_decide(now, prior, HELLO_RATE_MAX, HELLO_RATE_WINDOW_SECS)
}

/// Limiteur de débit /api/hello par adresse : true = requête admise (et
/// comptabilisée), false = au-delà de HELLO_RATE_MAX requêtes dans la fenêtre.
/// Table bornée à 1024 entrées (éviction des plus anciennes) `[M-04]`.
fn hello_rate_ok(ip: IpAddr) -> bool {
    const HELLO_RATE_KEYS_MAX: usize = 1024;
    let now = crate::types::now_ts();
    let mut slot = hello_rate_slot().lock().unwrap_or_else(|p| p.into_inner());
    let (decision, next) = hello_rate_decide(now, slot.get(&ip).map(Vec::as_slice).unwrap_or(&[]));
    slot.insert(ip, next);
    if slot.len() > HELLO_RATE_KEYS_MAX {
        if let Some(oldest) = slot
            .iter()
            .min_by_key(|(_, stamps)| stamps.last().copied().unwrap_or(0))
            .map(|(k, _)| *k)
        {
            slot.remove(&oldest);
        }
    }
    decision.is_ok()
}

/// Vérifie une preuve de pair : le nonce doit avoir été émis par NOUS (et pas
/// encore consommé), et le HMAC doit correspondre à NOTRE code d'accès.
fn verify_proof(nonce: &str, proof: &str, api_access: &str) -> bool {
    if nonce.len() != 64 || !nonce.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    let issued = hello_nonces_slot()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(nonce)
        .is_some();
    if !issued {
        return false;
    }
    let expected = crate::crypto::hmac_sha256_hex(
        api_access.as_bytes(),
        format!("mcc-proof-v1:{}", nonce).as_bytes(),
    );
    crate::crypto::ct_eq(expected.as_bytes(), proof.as_bytes())
}

/// Extrait un paramètre de query d'un chemin (ex. access= pour /api/detail).
fn query_param(path: &str, name: &str) -> Option<String> {
    let q = path.split_once('?')?.1;
    for pair in q.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        if k == name {
            return Some(urldecode(v));
        }
    }
    None
}

/// Contrôle d'accès central (pur, testable) :
/// - OPTIONS → 403 : les préflights CORS meurent ici, aucun cross-site ne passe ;
/// - Host hors liste → 403 (anti DNS-rebinding : un domaine rebindexé n'est pas le nôtre) ;
/// - Origin hors liste → 403 (anti CSRF cross-site) ;
/// - /api/* hors loopback → 401 sans le bon code (header X-MCC-Access ou ?access=),
///   SAUF /api/hello (nonce de préauthentification, aucune donnée) et SAUF si une
///   preuve de pair valide a déjà été vérifiée (proof_ok, cf. verify_proof) ;
/// - POST /api/action → 415 si Content-Type ≠ application/json (bloque les POST
///   « simples » text/plain des sites web, qui s'exécutent sans préflight).
///
/// Extrait la partie hote (sans port, sans crochets) d'un header Host ou d'une origine.
/// Gere l'IPv6 brackete et les ports numeriques ; minuscules pour comparaison.
pub fn host_only_of(h: &str) -> String {
    let h = h.trim();
    if let Some(rest) = h.strip_prefix('[') {
        if let Some(idx) = rest.find(']') {
            return rest[..idx].to_ascii_lowercase();
        }
    }
    if let Some((host, port)) = h.rsplit_once(':') {
        if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) {
            let host = host.trim_matches(|c| c == '[' || c == ']');
            return host.to_ascii_lowercase();
        }
    }
    h.trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase()
}

/// Extrait le port numérique explicite d'un en-tête `Host`, s'il est présent.
/// `Ok(None)` = pas de port explicite ; `Ok(Some(p))` = port valide ; `Err` =
/// syntaxe de port invalide (R-1).
pub fn host_port_of(h: &str) -> Result<Option<u16>, &'static str> {
    let h = h.trim();
    if let Some(rest) = h.strip_prefix('[') {
        let Some(idx) = rest.find(']') else {
            return Err("Bad Request");
        };
        let after = &rest[idx + 1..];
        if after.is_empty() {
            return Ok(None);
        }
        let Some(port_s) = after.strip_prefix(':') else {
            return Err("Bad Request");
        };
        if port_s.is_empty() || !port_s.bytes().all(|b| b.is_ascii_digit()) {
            return Err("Bad Request");
        }
        let port = port_s.parse::<u16>().map_err(|_| "Bad Request")?;
        if port == 0 {
            return Err("Bad Request");
        }
        return Ok(Some(port));
    }
    if let Some((host, port_s)) = h.rsplit_once(':') {
        // IPv6 sans crochets : plusieurs ':' → pas de port exploitable.
        if host.contains(':') {
            return Ok(None);
        }
        if port_s.is_empty() || !port_s.bytes().all(|b| b.is_ascii_digit()) {
            return Err("Bad Request");
        }
        let port = port_s.parse::<u16>().map_err(|_| "Bad Request")?;
        if port == 0 {
            return Err("Bad Request");
        }
        return Ok(Some(port));
    }
    Ok(None)
}

/// Vérifie que le port de l'en-tête `Host` correspond au `ui_port` écouté par
/// cette instance (R-1 : ferme le rejeu inter-ports / inter-instances sur `/mcp`).
/// - Un port explicite différent de `ui_port` est toujours refusé (`403`).
/// - Sur `/mcp`, un `Host` sans port n'est accepté que si `ui_port` est 80/443.
pub fn check_host_port(
    headers: &[(String, String)],
    ui_port: u16,
    is_mcp: bool,
) -> Result<(), (u16, &'static str)> {
    let Some((_, h)) = headers.iter().find(|(n, _)| n == "host") else {
        return Err((400, "Bad Request"));
    };
    match host_port_of(h) {
        Err(reason) => Err((400, reason)),
        Ok(Some(p)) if p != ui_port => Err((403, "Forbidden")),
        Ok(None) if is_mcp && ui_port != 80 && ui_port != 443 => Err((403, "Forbidden")),
        Ok(_) => Ok(()),
    }
}

/// Hote normalise extrait d'une origine HTTP complete (schema://hote[:port][/...]).
fn origin_host(o: &str) -> String {
    let r = o.split("://").nth(1).unwrap_or(o);
    let r = r.split('/').next().unwrap_or(r);
    host_only_of(r)
}

/// Fenêtre minimale entre deux recalculs des listes anti-rebinding : une
/// rafale de Hosts inconnus ne doit pas forcer une énumération d'interfaces
/// (sous-process ipconfig/hostname) à chaque requête (lot B point 4).
const HOSTS_RECALC_INTERVAL: Duration = Duration::from_secs(5);

/// Dernier recalcul des listes d'hôtes autorisés (limite de fréquence).
static HOSTS_RECALC_LAST: Mutex<Option<Instant>> = Mutex::new(None);

/// Listes anti DNS-rebinding (origines + hôtes) pour le port UI courant :
/// boucle + IP locales actuelles + hôtes des origines additionnelles
/// (settings.allowed_origins). Pure et testée — partagée par le boot et le
/// recalcul à chaud déclenché par un Host inconnu (lot B point 4).
fn listes_hosts_origins(
    ui_port: u16,
    local_ips: &[String],
    extra_origins: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut origins: Vec<String> = vec![
        format!("http://127.0.0.1:{ui_port}"),
        format!("https://127.0.0.1:{ui_port}"),
        format!("http://localhost:{ui_port}"),
        format!("https://localhost:{ui_port}"),
        format!("http://[::1]:{ui_port}"),
        format!("https://[::1]:{ui_port}"),
    ];
    let mut hosts: Vec<String> = vec![
        "127.0.0.1".to_string(),
        "localhost".to_string(),
        "::1".to_string(),
    ];
    for ip in local_ips {
        let host_url = if ip.contains(':') && !ip.starts_with('[') {
            format!("[{ip}]")
        } else {
            ip.clone()
        };
        origins.push(format!("http://{host_url}:{ui_port}"));
        origins.push(format!("https://{host_url}:{ui_port}"));
        hosts.push(ip.clone());
    }
    for o in extra_origins {
        origins.push(o.clone());
        hosts.push(origin_host(o));
    }
    (origins, hosts)
}

/// Limite de fréquence du recalcul (pure, testée) : un seul recalcul par
/// fenêtre — une adresse hostile ne peut pas forcer une énumération
/// d'interfaces à chaque requête.
fn recalc_permis(dernier: Option<Instant>, maintenant: Instant) -> bool {
    dernier
        .map(|t| maintenant.duration_since(t) >= HOSTS_RECALC_INTERVAL)
        .unwrap_or(true)
}

/// Un Host inconnu se présente : recalcule les IP locales (l'adresse a pu
/// apparaître après le boot — VPN, carte virtuelle, changement de Wi-Fi) et
/// remplace les listes si elles ont changé. Retourne true si les listes ont
/// bougé (le rejet 403 doit être rejugé). AUCUN affaiblissement : un Host ne
/// peut entrer dans les listes que s'il EST l'adresse d'une interface actuelle
/// de la machine — un nom ou une IP étrangère reste refusé. Borné en
/// fréquence (HOSTS_RECALC_INTERVAL).
fn rafraichir_hosts_sur_host_inconnu(state: &Arc<AppState>) -> bool {
    let maintenant = Instant::now();
    {
        let mut last = lockv(&HOSTS_RECALC_LAST);
        if !recalc_permis(*last, maintenant) {
            return false;
        }
        *last = Some(maintenant);
    }
    let (ui_port, extra) = {
        let st = lockv(&state.settings);
        (st.ui_port, st.allowed_origins.clone())
    };
    let (origins, hosts) = listes_hosts_origins(ui_port, &manage::local_ips(), &extra);
    let change = {
        let mut h = lockv(&state.allowed_hosts);
        let mut o = lockv(&state.allowed_origins);
        let change = *h != hosts || *o != origins;
        if change {
            *h = hosts;
            *o = origins;
        }
        change
    };
    if change {
        manage::add_log(
            state,
            "info",
            "[sécurité] Host inconnu : adresses locales recalculées — une nouvelle interface (VPN, Wi-Fi, carte virtuelle) est acceptée comme Host ; les hôtes étrangers restent refusés",
        );
    }
    change
}

/// Contrôle d'accès central de `/api/*` (pur, testable). L'ordre est imposé :
/// les contrôles gratuits passent AVANT toute vérification d'identité.
///
/// 1. `OPTIONS` → 403 : les préflights CORS meurent ici, aucun cross-site ne passe ;
/// 2. Host absent → 400 ; Host hors liste → 403 (anti DNS-rebinding) ;
/// 3. cadrage ambigu (Content-Length dupliqué, Transfer-Encoding) → 400 ;
/// 4. Origin hors liste → 403 (anti CSRF) ;
/// 5. `/api/*` sans session locale valide et sans code hérité valide → 401.
///    C'est le sas du lot M2 : le loopback n'est plus une zone de confiance
///    (DA2 = a). Exceptions : `/api/hello` (pré-authentifié, nonce seul) et
///    `/api/session` (c'est elle qui crée la session) ;
/// 6. un appelant HÉRITÉ (`X-MCC-Access` ou preuve de pair, §10) n'obtient que
///    la LECTURE : `POST /api/action` → 403 ;
/// 7. toute écriture (`POST /api/action`, `POST /api/session`, `POST
///    /api/session/close-others`) exige `application/json` → 415 sinon.
///
/// CSRF (S-1) : il n'y a plus d'en-tête constant `X-MCC-Request`. Toute requête
/// `/api/*` authentifiée porte désormais `X-MCC-Session`, un en-tête SECRET à
/// nom personnalisé : une page tierce ne peut pas le fabriquer, et l'envoyer
/// impose un préflight CORS que le serveur refuse (403, règle 1). Le cookie
/// `SameSite=Strict` et les contrôles Host/Origin restent la seconde barrière.
///
/// `/mcp` n'est plus filtré ici : sa porte est la **signature** (`gateway::serve`,
/// §2.2). Le code hérité n'y est plus exigé — sinon un client signé du LAN sans
/// code serait refusé avant vérification.
pub fn check_access(
    allowed_hosts: &[String],
    allowed_origins: &[String],
    method: &str,
    path: &str,
    headers: &[(String, String)],
    session_ok: bool,
    legacy_ok: bool,
) -> Result<(), (u16, &'static str)> {
    if method == "OPTIONS" {
        return Err((403, "Forbidden"));
    }
    // HTTP/1.1 : exactement un en-tête Host requis (0 ou ≥ 2 → 400, R-3).
    // Sinon le contrôle anti DNS-rebinding ci-dessous serait sauté ou ambigu.
    let host_count = headers.iter().filter(|(n, _)| n == "host").count();
    if host_count != 1 {
        return Err((400, "Bad Request"));
    }
    if let Some((_, h)) = headers.iter().find(|(n, _)| n == "host") {
        let host_only = host_only_of(h);
        if !allowed_hosts
            .iter()
            .any(|a| a.to_ascii_lowercase() == host_only)
        {
            return Err((403, "Forbidden"));
        }
    }
    // anti request-smuggling & désynchronisation (R-3) : refuser les en-têtes
    // critiques dupliqués ou mal formés.
    let mut cl_count = 0usize;
    for (n, v) in headers {
        if n == "content-length" {
            cl_count += 1;
            if cl_count > 1
                || v.is_empty()
                || !v.bytes().all(|b| b.is_ascii_digit())
                || v.parse::<usize>().is_err()
            {
                return Err((400, "Bad Request"));
            }
        }
    }
    if headers.iter().any(|(n, _)| n == "transfer-encoding") {
        return Err((400, "Bad Request"));
    }
    if headers.iter().filter(|(n, _)| n == "origin").count() > 1 {
        return Err((400, "Bad Request"));
    }
    if headers
        .iter()
        .filter(|(n, _)| n == crate::auth::session::SESSION_HEADER)
        .count()
        > 1
    {
        return Err((400, "Bad Request"));
    }
    if crate::auth::session::cookie_count(headers) > 1 {
        return Err((400, "Bad Request"));
    }
    if let Some((_, o)) = headers.iter().find(|(n, _)| n == "origin") {
        if !allowed_origins.iter().any(|a| a == o) {
            return Err((403, "Forbidden"));
        }
    }
    let bare_path = path.split('?').next().unwrap_or("");
    // /api/hello est PRÉ-authentifié : il ne délivre qu'un nonce à usage unique,
    // aucune donnée — c'est l'amorce de la preuve de pair (cf. verify_proof).
    // /api/session est la seule route qui CRÉE une session : elle n'en exige pas.
    let is_hello = bare_path == "/api/hello";
    let is_session = bare_path == "/api/session";
    if bare_path.starts_with("/api") && !is_hello && !is_session && !session_ok {
        // Code hérité = LECTURE SEULE (§10, DA3 = a). Aucune autre forme
        // d'accès n'existe plus : ni `?access=` (supprimé au lot M2), ni
        // l'exemption loopback (DA2 = a).
        if !legacy_ok {
            return Err((401, "Unauthorized"));
        }
        if method == "POST" && bare_path == "/api/action" {
            return Err((403, "Forbidden"));
        }
        // CSRF (S-1) : plus d'en-tête constant. L'en-tête secret `X-MCC-Session`
        // est exigé par `auth::session::authorized_session` (cookie ET secret,
        // temps constant) : son nom personnalisé impose un préflight, que le
        // serveur refuse — une page web étrangère ne peut pas l'envoyer.
    }
    if method == "POST"
        && (bare_path == "/api/action"
            || bare_path == "/api/session"
            || bare_path == "/api/session/close-others")
    {
        let ok = headers
            .iter()
            .find(|(n, _)| n == "content-type")
            .map(|(_, v)| v.to_ascii_lowercase().starts_with("application/json"))
            .unwrap_or(false);
        if !ok {
            return Err((415, "Unsupported Media Type"));
        }
    }
    Ok(())
}

/// Réponse complète d'une route : statut, type de contenu, corps et en-têtes
/// additionnels (le `Set-Cookie` de la session, le `WWW-Authenticate` d'un 401).
/// Le socket reste écrit à la main (`handle_conn`) ; ce type transporte juste ce
/// qu'une route a à dire en plus du triptyque historique.
pub struct Reply {
    pub status: u16,
    pub ctype: &'static str,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

impl Reply {
    pub fn new(status: u16, ctype: &'static str, body: Vec<u8>) -> Self {
        Self {
            status,
            ctype,
            body,
            headers: Vec::new(),
        }
    }

    /// Réponse JSON (le cas courant de `/api/*`).
    pub fn json(status: u16, body: serde_json::Value) -> Self {
        Self::new(status, "application/json", body.to_string().into_bytes())
    }

    /// Ajoute un en-tête de réponse (`Set-Cookie`, `WWW-Authenticate`…).
    pub fn with_header(mut self, name: &str, value: String) -> Self {
        self.headers.push((name.to_string(), value));
        self
    }

    /// Les deux champs d'une réponse à en-têtes constants (`Content-Type`
    /// seulement) — pour `route`, dont la signature ne change pas.
    pub fn into_parts(self) -> (u16, &'static str, Vec<u8>) {
        (self.status, self.ctype, self.body)
    }
}

/// Compatibilité avec les routes qui rendent déjà le triptyque historique
/// (`/auth/pair`, appairage du lot M1) : aucun en-tête additionnel.
impl From<(u16, &'static str, Vec<u8>)> for Reply {
    fn from((status, ctype, body): (u16, &'static str, Vec<u8>)) -> Self {
        Self::new(status, ctype, body)
    }
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        202 => "Accepted",
        304 => "Not Modified",
        400 => "Bad Request",
        200 => "OK",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        500 => "Internal Server Error",
        _ => "Bad Request",
    }
}

/// Contrôle interruptible des threads de fond (P-4 / P1 rang 4) : un changement
/// de `refresh_secs` ou une demande d'arrêt réveille immédiatement les boucles
/// sans attendre 5 à 30 s.
struct BgControl {
    wake_seq: u64,
    shutdown: bool,
}

fn bg_control() -> &'static (Mutex<BgControl>, std::sync::Condvar) {
    static CTRL: std::sync::OnceLock<(Mutex<BgControl>, std::sync::Condvar)> =
        std::sync::OnceLock::new();
    CTRL.get_or_init(|| {
        (
            Mutex::new(BgControl {
                wake_seq: 0,
                shutdown: false,
            }),
            std::sync::Condvar::new(),
        )
    })
}

/// Réveille immédiatement les threads de fond (ex. après changement de `refresh_secs`).
pub fn wake_bg_poller() {
    let (m, cv) = bg_control();
    let mut g = m.lock().unwrap_or_else(|p| p.into_inner());
    g.wake_seq = g.wake_seq.wrapping_add(1);
    cv.notify_all();
}

/// Signale l'arrêt propre aux threads de fond.
pub fn shutdown_bg() {
    let (m, cv) = bg_control();
    let mut g = m.lock().unwrap_or_else(|p| p.into_inner());
    g.shutdown = true;
    cv.notify_all();
}

/// Dort au plus `dur`, réveillé immédiatement par `wake_bg_poller()` ou `shutdown_bg()`.
/// Renvoie `true` si la boucle doit continuer, `false` si l'arrêt est demandé.
fn wait_bg(dur: Duration) -> bool {
    let (m, cv) = bg_control();
    let guard = m.lock().unwrap_or_else(|p| p.into_inner());
    if guard.shutdown {
        return false;
    }
    let start_seq = guard.wake_seq;
    let deadline = Instant::now() + dur;
    let mut g = guard;
    while !g.shutdown && g.wake_seq == start_seq {
        let remain = deadline.saturating_duration_since(Instant::now());
        if remain.is_zero() {
            break;
        }
        let (next, res) = cv
            .wait_timeout(g, remain)
            .unwrap_or_else(|p| p.into_inner());
        g = next;
        if res.timed_out() {
            break;
        }
    }
    !g.shutdown
}

/// Compare un en-tête `If-None-Match` à un `ETag` serveur (RFC 9110 §13.1.2 :
/// gère `*`, les listes séparées par virgules et le préfixe faible `W/`).
pub(crate) fn etag_matches(if_none_match: &str, etag: &str) -> bool {
    fn norm(s: &str) -> &str {
        let t = s.trim();
        t.strip_prefix("W/").unwrap_or(t).trim()
    }
    let target = norm(etag);
    if target.is_empty() {
        return false;
    }
    for part in if_none_match.split(',') {
        let p = part.trim();
        if p == "*" || norm(p) == target {
            return true;
        }
    }
    false
}

/// Résultat de `run_headless` : l'URL de l'UI, et si un moteur TOURNAIT DÉJÀ
/// (instance unique — cf. la logique de reconnexion dans run_headless).
pub struct RunOutcome {
    pub url: String,
    /// URL à usage unique de la session locale (`http://…/#t=<jeton>`, §6).
    /// Vide quand le moteur a été RÉUTILISÉ : le shell demande alors un jeton
    /// par le canal fichier (`auth::session::request_bootstrap`).
    pub session_url: String,
    pub reused: bool,
    /// Chemin du `config.json` de CE lancement — le shell en déduit le dossier
    /// du canal de session quand un moteur tournait déjà (lot S1).
    pub config_path: String,
}

pub fn run_headless() -> RunOutcome {
    let config_path =
        manage::resolve_config_path(env!("CARGO_MANIFEST_DIR"), std::env::var("MCC_CONFIG").ok());
    let config_path_out = config_path.clone();
    // ROBUSTESSE (audit 2026-10-25) : un panic dans un thread (poller, sonde)
    // était TOTALEMENT silencieux en mode desktop (windows_subsystem=windows,
    // aucune console) et tuait la supervision sans trace. Le hook écrit le
    // panic sur disque (mcc.log) et sur stderr.
    {
        let log_file = std::path::Path::new(&config_path)
            .with_file_name("mcc.log")
            .to_string_lossy()
            .to_string();
        // JOURNAL webhook (audit 05 #4) : send_webhook trace chaque envoi
        // (cible + résultat) dans mcc.log — il n'a pas d'AppState, le chemin
        // lui est branché ici, au seul endroit qui le connaît.
        crate::notify::set_webhook_log_path(&log_file);
        // Journal de session (S-1) : les refus d'entropie et les demandes du
        // canal fichier sont tracés dans le même mcc.log, sans aucun secret.
        crate::auth::session::set_log_path(&log_file);
        std::panic::set_hook(Box::new(move |info| {
            let ts = crate::types::iso_utc(crate::types::now_ts());
            let payload = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "?".to_string());
            let loc = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_default();
            let line = format!("[{}] [panic] {} ({})\n", ts, payload, loc);
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_file)
            {
                let _ = f.write_all(line.as_bytes());
            }
            eprint!("{}", line);
        }));
    }
    let (servers, settings) = manage::load_config(&config_path);
    let state = Arc::new(AppState {
        servers: Mutex::new(servers),
        log: Mutex::new(std::collections::VecDeque::new()),
        settings: Mutex::new(settings.clone()),
        children: Mutex::new(std::collections::HashMap::new()),
        peers: Mutex::new(std::collections::HashMap::new()),
        allowed_origins: Mutex::new(Vec::new()),
        allowed_hosts: Mutex::new(Vec::new()),
        spawn_fails: Mutex::new(std::collections::HashMap::new()),
        runtime: Mutex::new(manage::load_runtime(&config_path)),
        spawning: Mutex::new(std::collections::HashSet::new()),
        update_latest: Mutex::new(None),
        lan_ok: Mutex::new(None),
        config_path,
    });
    *active_state_slot()
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some(Arc::clone(&state));
    // cache de démarrage : restaure le dernier état connu AVANT la première
    // sonde — l'UI est peuplée instantanément, les sondes corrigent ensuite
    manage::load_cache(&state);
    // IDENTITÉ DU SERVEUR (lot M1, docs/auth.md §9) : créée au premier
    // lancement, clé privée au trousseau du système (repli fichier 0600
    // journalisé), clé PUBLIQUE persistée dans config.json (`auth.server`).
    // Avant le listener : un `auth pair` reçu au premier instant trouve une
    // identité prête.
    crate::auth::secret::ensure_server_identity(&state);
    crate::auth::net::set_config_path(&state.config_path);

    // P1 rang 5 : en bind loopback (cas par défaut), on ne bloque PAS avant
    // `TcpListener::bind` sur `manage::local_ips()` (qui lance `ipconfig.exe`
    // et provoquait des pics TCP à 1,5 s). Les IP LAN ne sont requises avant
    // le bind que pour un bind réseau (certificat TLS SANs).
    let mut ui_port = settings.ui_port;
    let requested_bind = settings.ui_bind.clone();
    let loopback_requested = is_loopback_bind(&requested_bind);
    let local_ips_list = if loopback_requested {
        Vec::new()
    } else {
        manage::local_ips()
    };
    #[cfg(feature = "tls")]
    let mut tls_server_config: Option<Arc<rustls::ServerConfig>> = None;

    // Lot M4 (DA5 = a, docs/auth.md §7) : hors loopback, TLS 1.3 est obligatoire.
    // Sans la feature `tls` (ou si le certificat/la clé sont invalides), le moteur
    // REFUSE d'écouter hors loopback, reste en 127.0.0.1 et le dit dans le
    // journal et dans l'interface. Aucun repli silencieux vers HTTP.
    let ui_bind = if is_loopback_bind(&requested_bind) {
        requested_bind
    } else {
        #[cfg(not(feature = "tls"))]
        {
            let msg = crate::auth::net::TLS_MISSING_BIND_MSG.to_string();
            {
                let mut st = lockv(&state.settings);
                st.auth.tls_bind_refused = true;
                st.auth.tls_notice = Some(msg.clone());
            }
            eprintln!("[TLS] {msg}");
            manage::add_log(&state, "warn", &format!("[tls] {msg}"));
            "127.0.0.1".to_string()
        }
        #[cfg(feature = "tls")]
        {
            let cfg_dir = std::path::Path::new(&state.config_path)
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let svc = crate::auth::secret::service();
            match crate::auth::net::load_or_create_server_tls(
                &cfg_dir,
                svc.as_deref(),
                &local_ips_list,
            ) {
                Ok(loaded) => {
                    {
                        let mut st = lockv(&state.settings);
                        st.auth.tls_fp = loaded.fingerprint.clone();
                        st.auth.tls_bind_refused = false;
                        st.auth.tls_notice = None;
                    }
                    if loaded.created {
                        manage::add_log(
                            &state,
                            "info",
                            &format!(
                                "[tls] certificat TLS 1.3 auto-signé créé ({}) — clé privée : {}",
                                loaded.fingerprint,
                                loaded.store.as_str()
                            ),
                        );
                    } else if loaded.store == crate::auth::secret::SecretStore::File {
                        manage::add_log(
                            &state,
                            "warn",
                            &format!(
                                "[tls] clé privée TLS lue depuis un fichier à permissions restreintes (trousseau indisponible) : {} — empreinte {}",
                                crate::auth::net::tls_key_path(&cfg_dir).display(),
                                loaded.fingerprint
                            ),
                        );
                    } else {
                        manage::add_log(
                            &state,
                            "info",
                            &format!(
                                "[tls] certificat TLS 1.3 chargé ({}) — clé privée : {}",
                                loaded.fingerprint,
                                loaded.store.as_str()
                            ),
                        );
                    }
                    tls_server_config = Some(loaded.server_config);
                    requested_bind
                }
                Err(e) => {
                    let msg = format!(
                        "Écoute réseau ({requested_bind}) refusée : configuration TLS invalide ({e}). Le moteur reste en 127.0.0.1."
                    );
                    {
                        let mut st = lockv(&state.settings);
                        st.auth.tls_fp.clear();
                        st.auth.tls_bind_refused = true;
                        st.auth.tls_notice = Some(msg.clone());
                    }
                    eprintln!("[TLS] {msg}");
                    manage::add_log(&state, "error", &format!("[tls] {msg}"));
                    "127.0.0.1".to_string()
                }
            }
        }
    };
    let listener = loop {
        match TcpListener::bind((ui_bind.as_str(), ui_port)) {
            Ok(l) => break l,
            Err(_) => {
                // port occupé : si un AUTRE moteur MCP Control y tourne déjà,
                // on s'y RECONNECTE au lieu d'en lancer un second (instance
                // unique — deux moteurs sur la même config se disputeraient les
                // spawns, réécriraient config.json et doublerraient les scans).
                // Occupant étranger → port suivant (comportement historique).
                // Un moteur peut être détecté puis mourir dans la seconde
                // (relance pendant un build, arrêt manuel) : on ne réutilise
                // une URL que si elle répond DEUX fois de suite, espacées de
                // 300 ms. Un moteur moribond ne survivra pas à la seconde
                // mesure — sinon la fenêtre s'ouvre sur un port mort.
                // Le moteur à détecter écoute sur le bind de la config : on
                // interroge CETTE adresse (un bind 0.0.0.0 ou une IP LAN
                // précise n'est pas joignable par défaut sur 127.0.0.1).
                let bind_host: Option<&str> = match ui_bind.as_str() {
                    "" | "0.0.0.0" | "*" => None,
                    h => Some(h),
                };
                let mut probe_hosts: Vec<&str> = Vec::new();
                if let Some(h) = bind_host {
                    probe_hosts.push(h);
                }
                for h in ["127.0.0.1", "::1"] {
                    if !probe_hosts.contains(&h) {
                        probe_hosts.push(h);
                    }
                }
                let mut dead_motor = false;
                for _ in 0..2 {
                    if let Some(url) = probe::control_ui_at_hosts(ui_port, &probe_hosts) {
                        thread::sleep(Duration::from_millis(300));
                        if probe::control_ui_at_hosts(ui_port, &probe_hosts).is_some() {
                            manage::add_log(
                                &state,
                                "info",
                                &format!(
                                    "moteur déjà en cours sur {} — réutilisé : aucun second moteur lancé (sondes, autostart et scan restent à l'instance existante)",
                                    url
                                ),
                            );
                            return RunOutcome {
                                url,
                                session_url: String::new(),
                                reused: true,
                                config_path: config_path_out,
                            };
                        }
                        dead_motor = true;
                        break;
                    }
                    thread::sleep(Duration::from_millis(250));
                }
                if dead_motor {
                    // Le moteur détecté était en train de mourir : on attend
                    // que le port se libère pour lancer NOTRE moteur, sinon
                    // la fenêtre pointerait vers un port mort.
                    manage::add_log(
                        &state,
                        "warn",
                        &format!(
                            "moteur précédent sur le port {} : arrêt en cours, attente de libération",
                            ui_port
                        ),
                    );
                    for _ in 0..12 {
                        if TcpListener::bind((ui_bind.as_str(), ui_port)).is_ok() {
                            break;
                        }
                        thread::sleep(Duration::from_millis(250));
                    }
                }
                manage::add_log(
                    &state,
                    "warn",
                    &format!("port {} occupé (étranger), essai {}", ui_port, ui_port + 1),
                );
                ui_port += 1;
                if ui_port > settings.ui_port + 9 {
                    eprintln!(
                        "[fatal] aucun port UI libre ({}..{})",
                        settings.ui_port,
                        settings.ui_port + 9
                    );
                    std::process::exit(1);
                }
            }
        }
    };
    manage::add_log(
        &state,
        "info",
        &format!("mcp-control v{} démarré", env!("CARGO_PKG_VERSION")),
    );
    {
        let mut st = lockv(&state.settings);
        st.ui_port = ui_port;
    }
    // SÉCURITÉ (docs/auth.md §10, DA3 = a) : aucune nouvelle valeur de code
    // partagé `api_access` n'est générée au boot ; si une valeur héritée est
    // présente dans config.json, elle est conservée en lecture seule pour 0.8.x.
    {
        let has_legacy = lockv(&state.settings)
            .api_access
            .as_deref()
            .is_some_and(|s| !s.is_empty());
        if has_legacy {
            manage::add_log(
                &state,
                "warn",
                "[auth] code hérité api_access présent dans config.json — toléré en LECTURE SEULE (DA3, 0.8.x) ; migrer les pairs vers mcp-control auth pair",
            );
        }
    }
    {
        let extra = lockv(&state.settings).allowed_origins.clone();
        let (origins, hosts) = listes_hosts_origins(ui_port, &local_ips_list, &extra);
        *lockv(&state.allowed_origins) = origins;
        *lockv(&state.allowed_hosts) = hosts;
    }
    // SÉCURITÉ (audit 05 #4) : enregistre les points d'écoute de CETTE instance
    // (port UI réel après repli +1..+9, IP locales, bind) — une URL de webhook
    // qui les vise est refusée à l'enregistrement comme à l'envoi.
    crate::notify::note_self_listen(ui_port, &local_ips_list, &ui_bind);
    // Complète les IP locales + aligne autostart en arrière-plan (sans retarder
    // l'écoute TCP ni la disponibilité du gateway).
    {
        let st = state.clone();
        let bind_bg = ui_bind.clone();
        thread::spawn(move || {
            if loopback_requested {
                let ips = manage::local_ips();
                let extra = lockv(&st.settings).allowed_origins.clone();
                let (origins, hosts) = listes_hosts_origins(ui_port, &ips, &extra);
                *lockv(&st.allowed_origins) = origins;
                *lockv(&st.allowed_hosts) = hosts;
                crate::notify::note_self_listen(ui_port, &ips, &bind_bg);
            }
            manage::sync_autostart_at_boot(&st);
        });
    }
    let ui_host = if ui_bind == "0.0.0.0" || ui_bind == "::" {
        "<ip-lan>".to_string()
    } else if ui_bind.contains(':') && !ui_bind.starts_with('[') {
        format!("[{ui_bind}]")
    } else {
        ui_bind.clone()
    };
    // Si le bind est une IP LAN précise (hors loopback et hors wildcard 0.0.0.0 / ::),
    // ouvrir un écouteur auxiliaire local sur 127.0.0.1:<ui_port> afin que la
    // webview Tauri et la CLI `auth` joignent toujours l'UI en loopback HTTP.
    let local_aux_listener =
        if !is_loopback_bind(&ui_bind) && ui_bind != "0.0.0.0" && ui_bind != "::" {
            TcpListener::bind(("127.0.0.1", ui_port)).ok()
        } else {
            None
        };
    let ui_url_host = if !is_loopback_bind(&ui_bind)
        && (ui_bind == "0.0.0.0" || ui_bind == "::" || local_aux_listener.is_some())
    {
        "127.0.0.1".to_string()
    } else if ui_bind.contains(':') && !ui_bind.starts_with('[') {
        format!("[{ui_bind}]")
    } else {
        ui_bind.clone()
    };
    let ui_url = format!("http://{}:{}", ui_url_host, ui_port);
    println!("[UI] {}  (Ctrl+C pour arrêter)", ui_url);
    // SESSION LOCALE (docs/auth.md §6, DA2 = a) : un jeton d'amorçage de
    // 32 octets, à usage unique, 5 minutes, ne vit que dans le FRAGMENT de
    // l'URL — jamais envoyé au serveur, jamais dans Referer, jamais dans
    // mcc.log (on n'écrit donc PAS ce jeton dans le journal : seule cette
    // ligne de console le porte). Le shell Tauri navigue vers cette URL ; en
    // headless, l'utilisateur l'ouvre une fois dans son navigateur.
    // S-3 : sans entropie OS, AUCUN jeton n'est tiré (pas de repli) — le
    // moteur démarre sans session et le dit dans le journal.
    let session_url = match crate::auth::session::bootstrap_url(&ui_url) {
        Some(u) => {
            println!("[session] {}", u);
            u
        }
        None => {
            manage::add_log(
                &state,
                "error",
                "session locale : entropie OS indisponible — aucun jeton d'amorçage, aucune session ne sera ouverte (échec fermé)",
            );
            String::new()
        }
    };
    // Fenêtre rouverte sur un moteur déjà lancé (S-1 pt 3) : le shell dépose
    // une demande dans le dossier de config, ce thread y répond par un jeton
    // à usage unique de 2 min. Fichiers restreints à l'utilisateur, supprimés
    // dès lecture, jamais journalisés.
    crate::auth::session::serve_session_requests(&state.config_path);
    manage::add_log(
        &state,
        "info",
        "session locale : URL à usage unique affichée sur la console (5 min, un seul échange) — /api/* l'exige, /mcp exige une signature",
    );
    manage::add_log(
        &state,
        "info",
        &format!(
            "UI prête sur http://{}:{} (bind {})",
            ui_host, ui_port, ui_bind
        ),
    );

    // poller : première passe immédiate, ensuite refresh_secs (interruptible via Condvar, P-4).
    // catch_unwind : un panic dans un cycle de sondage ne doit JAMAIS tuer la
    // supervision (c'était le cas avant l'audit 2026-10-25 — le thread mourait
    // en silence et l'app affichait un état figé).
    {
        let st = state.clone();
        thread::spawn(move || loop {
            let cycle =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| manage::poll_all(&st)));
            if cycle.is_err() {
                manage::add_log(
                    &st,
                    "error",
                    "panic pendant un cycle de sondage — le poller a survécu (détail dans mcc.log)",
                );
            }
            let secs = lockv(&st.settings).refresh_secs;
            if !wait_bg(Duration::from_secs(secs)) {
                break;
            }
        });
    }
    // veille réseau : la passerelle LAN répond-elle ? (VPN, câble, Wi-Fi)
    // Mesuré tout de suite puis toutes les 30 s (uniquement si serveurs distants
    // ou network_enabled ; sommeil interruptible).
    {
        let st = state.clone();
        thread::spawn(move || loop {
            manage::refresh_lan_state(&st);
            if !wait_bg(Duration::from_secs(30)) {
                break;
            }
        });
    }
    // AUTO-SCAN au boot : tous les ports en écoute de la machine (netstat/ss),
    // plus de plage à deviner — un serveur MCP sur n'importe quel port (8787
    // compris) est découvert même si scan_from/scan_to ne le couvre pas.
    {
        let st = state.clone();
        thread::spawn(move || {
            if !wait_bg(Duration::from_millis(1200)) {
                return;
            }
            manage::scan_listening(&st);
        });
    }
    // vérification de mise à jour (release GitHub) : au boot puis chaque heure
    {
        let st = state.clone();
        thread::spawn(move || {
            if !wait_bg(Duration::from_secs(8)) {
                return;
            }
            loop {
                manage::check_update(&st);
                if !wait_bg(Duration::from_secs(3600)) {
                    break;
                }
            }
        });
    }
    // découverte des pairs MCP Control + réseau (après le scan local)
    {
        let st = state.clone();
        thread::spawn(move || {
            if !wait_bg(Duration::from_secs(30)) {
                return;
            }
            manage::discover_peers(&st);
            let enabled = crate::types::lockv(&st.settings).network_enabled;
            if enabled {
                let (from, to) = {
                    let s = crate::types::lockv(&st.settings);
                    (s.scan_from, s.scan_to)
                };
                manage::scan_network(&st, Some((from, to)));
            }
        });
    }

    // GATEWAY MCP : la console est elle-même un serveur MCP — POST /mcp sur
    // le port UI, toute la surface de l'app en tools + proxy vers les
    // serveurs détectés (cf. gateway::serve). Le snapshot des ressources est
    // reconstruit sur événement de changement (Condvar, P-4 ; filet 60 s).
    let gw = gateway::install(&state);
    {
        let msg = format!(
            "gateway MCP prêt : {} outils de supervision + proxy call_server_tool — les clients MCP joignent POST {}/mcp{}",
            gateway::tool_count(&gw.snapshot()),
            ui_url,
            if is_loopback_bind(&ui_bind) {
                String::new()
            } else {
                " (code d'accès exigé hors loopback, comme /api/*)".to_string()
            }
        );
        println!("[MCP] {msg}");
        manage::add_log(&state, "info", &msg);
    }
    {
        let st = state.clone();
        thread::spawn(move || {
            let mut last_fp = gateway::fingerprint(&st);
            let mut last_seq = 0u64;
            loop {
                last_seq = gateway::wait_change(last_seq, Duration::from_secs(60));
                let now = gateway::fingerprint(&st);
                if now != last_fp {
                    last_fp = now;
                    gateway::refresh(&st);
                }
            }
        });
    }

    if let Some(aux_listener) = local_aux_listener {
        let aux_state = Arc::clone(&state);
        thread::spawn(move || {
            for stream in aux_listener.incoming() {
                match stream {
                    Ok(s) => match admit_conn(s.peer_addr().ok().map(|a| a.ip())) {
                        Ok(guard) => {
                            let st = aux_state.clone();
                            thread::spawn(move || {
                                #[cfg(feature = "tls")]
                                handle_conn(s, st, guard, None);
                                #[cfg(not(feature = "tls"))]
                                handle_conn(s, st, guard);
                            });
                        }
                        Err(raison) => write_conn_rejected(s, 503, raison),
                    },
                    Err(e) => eprintln!("[accept-loopback] {}", e),
                }
            }
        });
    }
    // serveur HTTP/TLS en tâche de fond : run_headless retourne, l'app hôte décide
    // (main = park forever, desktop = boucle d'événements Tauri)
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(s) => {
                    // admission AVANT tout thread dédié : les refus (503
                    // explicite) sont écrits ici, sans spawn — une tempête de
                    // connexions ne crée plus un thread par rejet.
                    match admit_conn(s.peer_addr().ok().map(|a| a.ip())) {
                        Ok(guard) => {
                            let st = state.clone();
                            #[cfg(feature = "tls")]
                            let tls_cfg = tls_server_config.clone();
                            thread::spawn(move || {
                                #[cfg(feature = "tls")]
                                handle_conn(s, st, guard, tls_cfg);
                                #[cfg(not(feature = "tls"))]
                                handle_conn(s, st, guard);
                            });
                        }
                        Err(raison) => write_conn_rejected(s, 503, raison),
                    }
                }
                Err(e) => eprintln!("[accept] {}", e),
            }
        }
    });
    RunOutcome {
        url: ui_url,
        session_url,
        reused: false,
        config_path: config_path_out,
    }
}

fn active_state_slot() -> &'static Mutex<Option<Arc<AppState>>> {
    static SLOT: std::sync::OnceLock<Mutex<Option<Arc<AppState>>>> = std::sync::OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Arrêt propre du moteur (appelé depuis le menu « Quitter » du tray Desktop et
/// avant un redémarrage d'auto-mise à jour) : stoppe les threads de fond et
/// termine tous les sous-processus supervisés appartenant à cette instance.
pub fn shutdown_engine() {
    shutdown_bg();
    let state_opt = active_state_slot()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take();
    if let Some(state) = state_opt {
        let handles: Vec<probe::StdioHandle> = {
            let mut ch = lockv(&state.children);
            ch.drain().map(|(_, h)| h).collect()
        };
        for h in handles {
            let same_proc = match h.started {
                Some(ts) => probe::process_creation(h.pid) == Some(ts),
                None => true,
            };
            if same_proc && h.pid != 0 {
                probe::kill_tree(h.pid);
            }
        }
    }
}

/// Bloque le thread courant indéfiniment (mode CLI headless).
pub fn park_forever() {
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}

pub fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

enum ConnStream {
    Plain(TcpStream),
    #[cfg(feature = "tls")]
    Tls(Box<rustls::StreamOwned<rustls::ServerConnection, TcpStream>>),
}

impl ConnStream {
    fn set_read_timeout(&self, dur: Option<Duration>) -> std::io::Result<()> {
        match self {
            ConnStream::Plain(s) => s.set_read_timeout(dur),
            #[cfg(feature = "tls")]
            ConnStream::Tls(s) => s.sock.set_read_timeout(dur),
        }
    }

    fn set_write_timeout(&self, dur: Option<Duration>) -> std::io::Result<()> {
        match self {
            ConnStream::Plain(s) => s.set_write_timeout(dur),
            #[cfg(feature = "tls")]
            ConnStream::Tls(s) => s.sock.set_write_timeout(dur),
        }
    }

    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            ConnStream::Plain(s) => s.read(buf),
            #[cfg(feature = "tls")]
            ConnStream::Tls(s) => s.read(buf),
        }
    }

    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        match self {
            ConnStream::Plain(s) => s.write_all(buf),
            #[cfg(feature = "tls")]
            ConnStream::Tls(s) => s.write_all(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            ConnStream::Plain(s) => s.flush(),
            #[cfg(feature = "tls")]
            ConnStream::Tls(s) => {
                let res = s.flush();
                s.conn.send_close_notify();
                let _ = s.conn.complete_io(&mut s.sock);
                let _ = s.sock.flush();
                res
            }
        }
    }

    /// Fermeture propre après un rejet anticipé (RFC 9112 §9.6) : envoie FIN
    /// côté écriture puis draine brièvement les octets entrants (borné à 200 ms
    /// et 4 Mio) afin qu'un RST TCP sous Windows n'efface pas la réponse `413`
    /// avant sa lecture par le client.
    fn finish_early_reject(&mut self) {
        let _ = self.flush();
        match self {
            ConnStream::Plain(s) => {
                let _ = s.shutdown(std::net::Shutdown::Write);
            }
            #[cfg(feature = "tls")]
            ConnStream::Tls(s) => {
                let _ = s.sock.shutdown(std::net::Shutdown::Write);
            }
        }
        let drain_until = Instant::now() + Duration::from_millis(200);
        let mut drained = 0usize;
        let mut sink = [0u8; 16384];
        while drained < 4 * 1024 * 1024 && Instant::now() < drain_until {
            let rem = drain_until.saturating_duration_since(Instant::now());
            let _ = self.set_read_timeout(Some(rem.min(Duration::from_millis(50))));
            match self.read(&mut sink) {
                Ok(0) | Err(_) => break,
                Ok(n) => drained += n,
            }
        }
    }
}

#[cfg(feature = "tls")]
pub(crate) fn handle_conn(
    tcp_stream: TcpStream,
    state: Arc<AppState>,
    guard: ConnGuard,
    tls_cfg: Option<Arc<rustls::ServerConfig>>,
) {
    let _guard = guard;
    let deadline = Instant::now() + REQUEST_DEADLINE;
    let peer_ip = tcp_stream.peer_addr().ok().map(|a| a.ip());
    let peer_is_loopback = peer_ip.map(|ip| ip.is_loopback()).unwrap_or(false);

    // Lot M4 (DA5 = a, docs/auth.md §7) :
    // - hors loopback (!peer_is_loopback) : TLS 1.3 est OBLIGATOIRE ; une
    //   requête HTTP en clair ou TLS <= 1.2 échoue à la poignée de main ;
    // - en loopback (127.0.0.1 / ::1) : reste en HTTP, mais accepte aussi TLS 1.3
    //   si le client démarre par un ClientHello TLS (0x16) et que TLS est actif.
    let stream = if !peer_is_loopback {
        let Some(cfg) = tls_cfg else {
            return;
        };
        match crate::auth::net::accept_tls_stream(tcp_stream, &cfg, deadline) {
            Ok(tls) => ConnStream::Tls(Box::new(tls)),
            Err(_) => return,
        }
    } else if let Some(cfg) = tls_cfg {
        let _ = tcp_stream.set_read_timeout(Some(REQUEST_DEADLINE));
        let mut first = [0u8; 1];
        let is_tls_client_hello = tcp_stream
            .peek(&mut first)
            .map(|n| n == 1 && first[0] == 0x16)
            .unwrap_or(false);
        if is_tls_client_hello {
            match crate::auth::net::accept_tls_stream(tcp_stream, &cfg, deadline) {
                Ok(tls) => ConnStream::Tls(Box::new(tls)),
                Err(_) => return,
            }
        } else {
            ConnStream::Plain(tcp_stream)
        }
    } else {
        ConnStream::Plain(tcp_stream)
    };
    handle_conn_stream(stream, peer_ip, peer_is_loopback, deadline, state);
}

#[cfg(not(feature = "tls"))]
pub(crate) fn handle_conn(tcp_stream: TcpStream, state: Arc<AppState>, guard: ConnGuard) {
    let _guard = guard;
    let deadline = Instant::now() + REQUEST_DEADLINE;
    let peer_ip = tcp_stream.peer_addr().ok().map(|a| a.ip());
    let peer_is_loopback = peer_ip.map(|ip| ip.is_loopback()).unwrap_or(false);
    // DA5 = a : sans la feature `tls`, aucune connexion hors loopback n'est
    // jamais servie (échec fermé en sus du bind 127.0.0.1).
    if !peer_is_loopback {
        return;
    }
    handle_conn_stream(
        ConnStream::Plain(tcp_stream),
        peer_ip,
        peer_is_loopback,
        deadline,
        state,
    );
}

fn handle_conn_stream(
    mut stream: ConnStream,
    peer_ip: Option<IpAddr>,
    peer_is_loopback: bool,
    deadline: Instant,
    state: Arc<AppState>,
) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 16384];
    let mut head_end = None;
    loop {
        if Instant::now() >= deadline {
            break;
        }
        let _ = stream.set_read_timeout(Some(deadline.saturating_duration_since(Instant::now())));
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                let prev_len = buf.len();
                buf.extend_from_slice(&tmp[..n]);
                let search_from = prev_len.saturating_sub(3);
                if let Some(rel) = find_sub(&buf[search_from..], b"\r\n\r\n") {
                    head_end = Some(search_from + rel);
                    break;
                }
                if buf.len() > 1_048_576 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let head_end = match head_end {
        Some(p) => p,
        None => return,
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let req_line = lines.next().unwrap_or("");
    let mut parts = req_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let mut content_length = 0usize;
    let mut headers: Vec<(String, String)> = Vec::new();
    for l in lines {
        if let Some((raw_name, value)) = l.split_once(':') {
            // RFC 9112 §5.1 : aucun espace n'est autorisé dans le nom d'en-tête
            // ni avant `:`. Un nom malformé est rejeté par 400 (anti-smuggling).
            if raw_name.is_empty() || raw_name.bytes().any(|b| b.is_ascii_whitespace()) {
                let j = serde_json::json!({"ok": false, "error": "Bad Request"}).to_string();
                let resp = format!(
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{}Cache-Control: no-store\r\nConnection: close\r\n\r\n{}",
                    j.len(),
                    SECURITY_HEADERS,
                    j
                );
                let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
                return;
            }
            headers.push((raw_name.to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    let mut cl_seen = false;
    for (name, value) in &headers {
        if name == "content-length" {
            let valid = !cl_seen && !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit());
            let parsed = if valid {
                value.parse::<usize>().ok()
            } else {
                None
            };
            match parsed {
                Some(n) => {
                    cl_seen = true;
                    content_length = n;
                }
                None => {
                    let j = serde_json::json!({"ok": false, "error": "Bad Request"}).to_string();
                    let resp = format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{}Cache-Control: no-store\r\nConnection: close\r\n\r\n{}",
                        j.len(),
                        SECURITY_HEADERS,
                        j
                    );
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.flush();
                    return;
                }
            }
        }
    }
    if content_length > MAX_BODY {
        let j = serde_json::json!({"ok": false, "error": "body trop grand (max 2 Mo)"}).to_string();
        let resp = format!(
            "HTTP/1.1 413 Payload Too Large\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{}Cache-Control: no-store\r\nConnection: close\r\n\r\n{}",
            j.len(),
            SECURITY_HEADERS,
            j
        );
        let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
        let _ = stream.write_all(resp.as_bytes());
        stream.finish_early_reject();
        return;
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < content_length {
        if Instant::now() >= deadline {
            break;
        }
        let _ = stream.set_read_timeout(Some(deadline.saturating_duration_since(Instant::now())));
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                body.extend_from_slice(&tmp[..n]);
                if body.len() > MAX_BODY {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    if body.len() > MAX_BODY {
        let j = serde_json::json!({"ok": false, "error": "body trop grand (max 2 Mo)"}).to_string();
        let resp = format!(
            "HTTP/1.1 413 Payload Too Large\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{}Cache-Control: no-store\r\nConnection: close\r\n\r\n{}",
            j.len(),
            SECURITY_HEADERS,
            j
        );
        let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
        let _ = stream.write_all(resp.as_bytes());
        stream.finish_early_reject();
        return;
    }
    if body.len() < content_length {
        let j = serde_json::json!({"ok": false, "error": "corps HTTP incomplet"}).to_string();
        let resp = format!(
            "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{}Cache-Control: no-store\r\nConnection: close\r\n\r\n{}",
            j.len(),
            SECURITY_HEADERS,
            j
        );
        let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
        let _ = stream.write_all(resp.as_bytes());
        let _ = stream.flush();
        return;
    }
    body.truncate(content_length);

    // contrôle d'accès (session locale, code hérité, Origin, Host, cadrage) puis
    // routage. Lot M2 : le loopback n'est plus une zone de confiance (DA2 = a).
    let (api_access, ui_port) = {
        let st = lockv(&state.settings);
        (st.api_access.clone(), st.ui_port)
    };
    let (allowed_hosts, allowed_origins) = (
        lockv(&state.allowed_hosts).clone(),
        lockv(&state.allowed_origins).clone(),
    );
    // preuve de pair : X-MCC-Nonce + X-MCC-Proof (HMAC du nonce avec le code
    // partagé) authentifie sans que le code n'ait jamais voyagé en clair
    let proof_ok = {
        let nonce = headers
            .iter()
            .find(|(n, _)| n == "x-mcc-nonce")
            .map(|(_, v)| v.as_str());
        let proof = headers
            .iter()
            .find(|(n, _)| n == "x-mcc-proof")
            .map(|(_, v)| v.as_str());
        match (nonce, proof, api_access.as_deref()) {
            (Some(n), Some(p), Some(code)) if !code.is_empty() => verify_proof(n, p, code),
            _ => false,
        }
    };
    // Session locale de la fenêtre (docs/auth.md §6) : cookie `mcc_session`
    // + en-tête secret `X-MCC-Session` (S-1), émis par POST /api/session.
    // Valable sur /api/* UNIQUEMENT en loopback — jamais sur /mcp ni depuis le LAN [H-06].
    let session_id = if peer_is_loopback {
        crate::auth::session::authorized_session(&headers)
    } else {
        None
    };
    let session_ok = session_id.is_some();
    // Code hérité (§10, DA3 = a) : toléré en LECTURE SEULE pendant 0.8.x.
    // `?access=` n'existe plus (supprimé au lot M2).
    let legacy_code_ok = api_access
        .as_deref()
        .filter(|s| !s.is_empty())
        .zip(
            headers
                .iter()
                .find(|(n, _)| n == "x-mcc-access")
                .map(|(_, v)| v.as_str()),
        )
        .is_some_and(|(expected, got)| crate::crypto::ct_eq(expected.as_bytes(), got.as_bytes()));
    let legacy_ok = legacy_code_ok || proof_ok;
    let legacy_reason = if legacy_code_ok {
        Some("legacy_access")
    } else if proof_ok {
        Some("legacy_proof")
    } else {
        None
    };
    // Chaque usage RÉEL du code hérité écrit un avertissement (§10) :
    // - sur /api/*, il ne sert que si la session locale ne couvre pas la requête ;
    // - sur /mcp, la session n'ouvre rien : un code hérité sans signature est un
    //   usage, avec signature c'est la signature qui décide.
    let bare_route = path.split('?').next().unwrap_or(&path);
    let is_mcp_route = bare_route == "/mcp" || bare_route.starts_with("/mcp/s/");
    let signature_presente = headers
        .iter()
        .any(|(n, _)| n == "signature-input" || n == "signature");
    let usage_legacy = match legacy_reason {
        Some(_) if is_mcp_route => !signature_presente,
        Some(_) => !session_ok,
        None => false,
    };
    if let (Some(motif), true) = (legacy_reason, usage_legacy) {
        manage::add_log(
            &state,
            "warn",
            &format!(
                "[auth] {motif} : accès en LECTURE SEULE sur {bare_route} — client hérité, migrer vers un client appairé (docs/auth.md §10)"
            ),
        );
    }
    let gated = check_access(
        &allowed_hosts,
        &allowed_origins,
        &method,
        &path,
        &headers,
        session_ok,
        legacy_ok,
    );
    // LOT B point 4 : Host inconnu ≠ Host étranger. Une adresse apparue après
    // le boot (VPN, carte virtuelle, changement de Wi-Fi) est acceptée sans
    // redémarrage : on recalcule les IP locales (borné en fréquence) et on
    // rejuge UNE fois avec les listes à jour. Un Host qui n'est l'adresse
    // d'aucune interface de la machine (DNS-rebinding, forgery) reste 403 —
    // aucune liste n'accepte jamais un nom ou une IP étrangère à la machine.
    let gated = match gated {
        Err((403, _)) if method != "OPTIONS" => {
            let host_inconnu = headers
                .iter()
                .find(|(n, _)| n == "host")
                .map(|(_, h)| host_only_of(h))
                .is_some_and(|host_only| {
                    !allowed_hosts
                        .iter()
                        .any(|a| a.to_ascii_lowercase() == host_only)
                });
            if host_inconnu && rafraichir_hosts_sur_host_inconnu(&state) {
                let (hosts_frais, origins_fraiches) = (
                    lockv(&state.allowed_hosts).clone(),
                    lockv(&state.allowed_origins).clone(),
                );
                check_access(
                    &hosts_frais,
                    &origins_fraiches,
                    &method,
                    &path,
                    &headers,
                    session_ok,
                    legacy_ok,
                )
            } else {
                gated
            }
        }
        autre => autre,
    };
    // R-1 : le port de `Host` doit correspondre au `ui_port` de cette instance
    // (ferme le rejeu inter-ports / inter-instances sur `/mcp` et `/mcp/s/*`).
    let gated = gated.and_then(|()| check_host_port(&headers, ui_port, is_mcp_route));
    // limiteur de débit /api/hello (audit 05 #6) : hors loopback, une adresse
    // ne peut plus remplir la table de nonces à outrance. Le loopback est
    // exempté : la fenêtre locale et le harnais e2e sondent /api/hello en rafale.
    let hello_throttled = {
        let bare = path.split('?').next().unwrap_or("");
        bare == "/api/hello"
            && method == "GET"
            && !peer_is_loopback
            && match peer_ip {
                Some(ip) => !hello_rate_ok(ip),
                None => false,
            }
    };
    let reply = match gated {
        Err((code, reason)) => Reply::json(code, serde_json::json!({"ok": false, "error": reason})),
        Ok(()) if hello_throttled => Reply::json(
            429,
            serde_json::json!({
                "ok": false,
                "error": "taux de /api/hello dépassé pour cette adresse — réessayez plus tard"
            }),
        ),
        // Gateway MCP : la console elle-même est un serveur MCP — POST /mcp
        // (émule le transport Streamable HTTP stateless de LiteMCP sur notre
        // socket déjà durci ; cf. gateway::serve). `legacy_ok` n'ouvre que la
        // LECTURE (code hérité, §10) — la signature, elle, est obligatoire.
        Ok(()) if is_mcp_route => {
            // Anti-force brute (lot M5, docs/auth.md §1.4) : le `keyid`
            // revendiqué est une donnée PUBLIQUE (jamais une autorisation) ;
            // la décision est prise AVANT toute cryptographie, et le loopback
            // n'est jamais freiné ni bloqué par des échecs venus du LAN.
            let keyid_revendique = crate::audit::claimed_keyid(&headers);
            let verdict = crate::auth::throttle::check(peer_ip, keyid_revendique.as_deref());
            match verdict {
                crate::auth::throttle::Verdict::Block(secondes) => Reply::json(
                    429,
                    serde_json::json!({
                        "ok": false,
                        "error": "trop d'échecs d'authentification — réessayez plus tard"
                    }),
                )
                .with_header("Retry-After", secondes.to_string()),
                autre => {
                    if let crate::auth::throttle::Verdict::Delay(delai) = autre {
                        thread::sleep(delai);
                    }
                    // Journal d'audit chaîné (docs/auth.md §11) : la décision
                    // du hook C4 est posée sur ce fil, le transport y ajoute
                    // l'adresse source et le keyid revendiqué.
                    crate::audit::begin_mcp_request();
                    let reply = if let Some(raw_sid) = bare_route.strip_prefix("/mcp/s/") {
                        let sid = raw_sid.trim_end_matches('/');
                        if sid.is_empty() || sid.contains('/') {
                            Reply::json(
                                400,
                                serde_json::json!({
                                    "jsonrpc": "2.0",
                                    "id": null,
                                    "error": {
                                        "code": -32600,
                                        "message": "identifiant serveur invalide dans /mcp/s/<id>"
                                    }
                                }),
                            )
                        } else {
                            gateway::serve_hybrid_stdio(
                                &state, sid, &method, &path, &headers, &body, legacy_ok, peer_ip,
                            )
                        }
                    } else {
                        gateway::serve(&method, &path, &headers, &body, legacy_ok, peer_ip)
                    };
                    crate::audit::note_http(
                        &state,
                        &headers,
                        &method,
                        reply.status,
                        peer_ip,
                        keyid_revendique.as_deref(),
                    );
                    // Seuls un 401 (échec d'auth) ou un succès d'auth signée
                    // (200/202/403) mettent à jour le compteur anti-force brute :
                    // un rejet pré-auth (405, 415, 406, 400) ne remet jamais à zéro.
                    if reply.status == 401 {
                        crate::auth::throttle::record(peer_ip, keyid_revendique.as_deref(), true);
                    } else if signature_presente && matches!(reply.status, 200 | 202 | 403) {
                        crate::auth::throttle::record(peer_ip, keyid_revendique.as_deref(), false);
                    }
                    reply
                }
            }
        }
        Ok(()) => {
            let if_none_match = headers
                .iter()
                .find(|(n, _)| n == "if-none-match")
                .map(|(_, v)| v.as_str());
            route_peer_session_cond(
                &state,
                &method,
                &path,
                &body,
                peer_ip,
                session_id.as_deref(),
                if_none_match,
            )
        }
    };
    let Reply {
        status,
        ctype,
        body: payload,
        headers: extra,
    } = reply;
    // ACAO dynamique : on reflète UNIQUEMENT une origine autorisée (jamais *),
    // sinon aucun site tiers ne peut lire nos réponses (/api/state inclu).
    // RFC 9110 §15.4.5 : une réponse 304 Not Modified n'emporte aucun corps.
    let mut resp = if status == 304 {
        format!("HTTP/1.1 304 Not Modified\r\n{}", SECURITY_HEADERS)
    } else {
        format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n{}",
            status,
            reason_phrase(status),
            ctype,
            payload.len(),
            SECURITY_HEADERS
        )
    };
    // En-têtes portés par la route (lot M2) : `Set-Cookie` de la session,
    // `WWW-Authenticate` du 401 de litemcp et `ETag` (P1 rang 3). Leurs noms
    // et valeurs sont construits par le serveur, jamais recopiés du client.
    for (name, value) in &extra {
        resp.push_str(&format!("{}: {}\r\n", name, value));
    }
    if ctype == "application/json" || ctype == "text/event-stream" {
        resp.push_str("Cache-Control: no-store\r\n");
    }
    // Coquille HTML : no-cache — chaque navigation doit repartir du lien
    // ?v=VERSION à jour vers les assets.
    if ctype.starts_with("text/html") {
        resp.push_str(&format!("Content-Security-Policy: {}\r\n", UI_CSP));
        resp.push_str("Cache-Control: no-cache\r\n");
    }
    // Assets UI : no-cache + ETag/304 (revalidation légère, pas de pages
    // périmées en dev).
    if ctype.starts_with("text/css") || ctype.starts_with("text/javascript") {
        resp.push_str("Cache-Control: no-cache\r\n");
    }
    if let Some((_, o)) = headers.iter().find(|(n, _)| n == "origin") {
        if allowed_origins.iter().any(|a| a == o) {
            resp.push_str(&format!("Access-Control-Allow-Origin: {}\r\n", o));
        }
    }
    resp.push_str("Connection: close\r\n\r\n");
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let _ = stream.write_all(resp.as_bytes());
    if status != 304 && !payload.is_empty() {
        let _ = stream.write_all(&payload);
    }
    let _ = stream.flush();
}

/// Asset UI pré-versionné + ETag stable dérivé de la version et du contenu du binaire.
struct StaticAsset {
    bytes: Vec<u8>,
    etag: String,
}

fn build_static_asset(src: &str) -> StaticAsset {
    build_static_bytes(
        src.replace("__MCC_VERSION__", env!("CARGO_PKG_VERSION"))
            .as_bytes(),
    )
}

fn build_static_bytes(src: &[u8]) -> StaticAsset {
    use std::hash::{Hash, Hasher};
    let bytes = src.to_vec();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    env!("CARGO_PKG_VERSION").hash(&mut h);
    bytes.hash(&mut h);
    let etag = format!("\"a-{}-{:016x}\"", env!("CARGO_PKG_VERSION"), h.finish());
    StaticAsset { bytes, etag }
}

fn serve_static_asset(
    asset: &StaticAsset,
    ctype: &'static str,
    if_none_match: Option<&str>,
) -> Reply {
    if if_none_match.is_some_and(|inm| etag_matches(inm, &asset.etag)) {
        return Reply::new(304, ctype, Vec::new()).with_header("ETag", asset.etag.clone());
    }
    Reply::new(200, ctype, asset.bytes.clone()).with_header("ETag", asset.etag.clone())
}

fn asset_app_css() -> &'static StaticAsset {
    static ASSET: std::sync::OnceLock<StaticAsset> = std::sync::OnceLock::new();
    ASSET.get_or_init(|| build_static_asset(ui::APP_CSS))
}

fn asset_app_boot_js() -> &'static StaticAsset {
    static ASSET: std::sync::OnceLock<StaticAsset> = std::sync::OnceLock::new();
    ASSET.get_or_init(|| build_static_asset(ui::APP_BOOT_JS))
}

fn asset_app_js() -> &'static StaticAsset {
    static ASSET: std::sync::OnceLock<StaticAsset> = std::sync::OnceLock::new();
    ASSET.get_or_init(|| build_static_asset(ui::APP_JS))
}

fn asset_favicon_svg() -> &'static StaticAsset {
    static ASSET: std::sync::OnceLock<StaticAsset> = std::sync::OnceLock::new();
    ASSET.get_or_init(|| build_static_asset(ui::FAVICON_SVG))
}

fn asset_favicon_ico() -> &'static StaticAsset {
    static ASSET: std::sync::OnceLock<StaticAsset> = std::sync::OnceLock::new();
    ASSET.get_or_init(|| build_static_bytes(ui::FAVICON_ICO))
}

pub fn route(
    state: &Arc<AppState>,
    method: &str,
    path: &str,
    body: &[u8],
) -> (u16, &'static str, Vec<u8>) {
    route_peer(state, method, path, body, None).into_parts()
}

/// Routage avec l'adresse de l'appelant — seul `route_peer_session` la
/// connaît, et trois décisions en dépendent : le quota d'appairage (par
/// adresse), le refus des actions `admin` depuis le LAN (elles sont LOCALES par
/// construction) et l'échange du jeton de session (`POST /api/session`,
/// loopback seulement). Variante sans session pour `route()` et les tests
/// historiques (aucune route ne l'exige).
pub fn route_peer(
    state: &Arc<AppState>,
    method: &str,
    path: &str,
    body: &[u8],
    peer: Option<IpAddr>,
) -> Reply {
    route_peer_session(state, method, path, body, peer, None)
}

/// Même routage, avec l'identifiant de la session VIVANTE déjà validée par
/// `handle_conn` (cookie ET secret) : la route `close-others` en a besoin pour
/// garder la courante et révoquer les autres.
pub fn route_peer_session(
    state: &Arc<AppState>,
    method: &str,
    path: &str,
    body: &[u8],
    peer: Option<IpAddr>,
    session: Option<&str>,
) -> Reply {
    route_peer_session_cond(state, method, path, body, peer, session, None)
}

/// Routage avec `If-None-Match` (ETag / 304 Not Modified, P1 rang 3) — appelé
/// uniquement APRÈS tous les contrôles de sécurité (`check_access`, `check_host_port`).
pub fn route_peer_session_cond(
    state: &Arc<AppState>,
    method: &str,
    path: &str,
    body: &[u8],
    peer: Option<IpAddr>,
    session: Option<&str>,
    if_none_match: Option<&str>,
) -> Reply {
    let bare_path = path.split('?').next().unwrap_or(path);
    // ---- Appairage (docs/auth.md §3.4) : les deux seules routes non
    // authentifiées ajoutées par le lot M1. Le sas de `check_access` ne les
    // couvre pas (elles ne sont ni /api ni /mcp) : leur protection est le quota
    // par adresse, l'expiration à 2 minutes et le fait qu'elles n'accordent RIEN.
    if bare_path == "/auth/pair" {
        if method != "POST" {
            return Reply::new(
                405,
                "application/json",
                br#"{"ok":false,"error":"method not allowed"}"#.to_vec(),
            );
        }
        return crate::auth::pair::handle_post(state, body, peer, crate::types::now_ts()).into();
    }
    if let Some(pair_id) = bare_path.strip_prefix("/auth/pair/") {
        if method != "GET" {
            return Reply::new(
                405,
                "application/json",
                br#"{"ok":false,"error":"method not allowed"}"#.to_vec(),
            );
        }
        return crate::auth::pair::handle_get(state, pair_id, peer, crate::types::now_ts()).into();
    }
    // ---- Session locale (docs/auth.md §6) : le jeton d'amorçage (32 octets,
    // usage unique, 5 min) s'échange ici contre le cookie `mcc_session`. Route
    // LOCALE par construction : le jeton ne quitte jamais la machine, un
    // échange venu du LAN est refusé même avec un jeton valide.
    if method == "POST" && bare_path == "/api/session" {
        if !peer.is_some_and(|ip| ip.is_loopback()) {
            return Reply::json(
                403,
                serde_json::json!({"ok": false, "error": "session locale uniquement"}),
            );
        }
        let parsed_body = serde_json::from_slice::<serde_json::Value>(body).ok();
        let token = parsed_body
            .as_ref()
            .and_then(|v| v.get("token").and_then(|t| t.as_str()).map(String::from))
            .unwrap_or_default();
        let ephemeral = parsed_body
            .as_ref()
            .and_then(|v| v.get("ephemeral").and_then(|b| b.as_bool()))
            .unwrap_or(false);
        return match crate::auth::session::exchange_with_mode(&token, ephemeral) {
            Some(issued) => Reply::json(
                200,
                serde_json::json!({"ok": true, "secret": issued.secret}),
            )
            .with_header(
                "Set-Cookie",
                crate::auth::session::set_cookie_header(&issued.cookie),
            ),
            None => Reply::json(
                401,
                serde_json::json!({
                    "ok": false,
                    "error": "jeton d'amorçage refusé (inconnu, déjà utilisé, expiré ou plafond de sessions atteint)"
                }),
            ),
        };
    }
    // ---- Fermeture de la session courante (sessions CLI éphémères) : loopback uniquement.
    if method == "POST" && bare_path == "/api/session/close" {
        if !peer.is_some_and(|ip| ip.is_loopback()) {
            return Reply::json(
                403,
                serde_json::json!({"ok": false, "error": "session locale uniquement"}),
            );
        }
        return match session.filter(|s| crate::auth::session::close_session(s)) {
            Some(_) => Reply::json(200, serde_json::json!({"ok": true})),
            None => Reply::json(
                401,
                serde_json::json!({"ok": false, "error": "session absente ou expirée"}),
            ),
        };
    }
    // ---- Fermeture des autres sessions (S-1) : route locale de la fenêtre.
    // Elle n'existe PAS dans le dispatcher d'actions (`manage/actions.rs`) :
    // c'est un contrôle de session, pas une action métier.
    if method == "POST" && bare_path == "/api/session/close-others" {
        if !peer.is_some_and(|ip| ip.is_loopback()) {
            return Reply::json(
                403,
                serde_json::json!({"ok": false, "error": "session locale uniquement"}),
            );
        }
        return match session.and_then(crate::auth::session::close_others) {
            Some(closed) => Reply::json(200, serde_json::json!({"ok": true, "closed": closed})),
            None => Reply::json(
                401,
                serde_json::json!({"ok": false, "error": "session absente ou expirée"}),
            ),
        };
    }
    if method == "GET" && bare_path == "/" {
        // SÉCURITÉ (audit 2026-10-24) : le code d'accès n'est PLUS injecté dans la
        // page — quiconque joint le port pouvait le lire sur / et contourner toute
        // l'auth. L'UI le demande à l'utilisateur (login), le garde en localStorage
        // et le présente sur chaque appel /api/* (bind loopback : jamais demandé).
        let html = ui::INDEX_HTML.replace("__MCC_VERSION__", env!("CARGO_PKG_VERSION"));
        return Reply::new(200, "text/html; charset=utf-8", html.into_bytes());
    }
    // Assets de l'UI (split v0.7.19) : servis PRÉ-authentification comme la
    // coquille — ils ne contiennent aucun secret (le code d'accès ne quitte
    // jamais le serveur) et la page de login en a besoin pour s'afficher.
    // P1 rang 3 : ETag stable + 304 Not Modified sur If-None-Match.
    if method == "GET" && bare_path == "/app.css" {
        return serve_static_asset(asset_app_css(), "text/css; charset=utf-8", if_none_match);
    }
    if method == "GET" && bare_path == "/app-boot.js" {
        return serve_static_asset(
            asset_app_boot_js(),
            "text/javascript; charset=utf-8",
            if_none_match,
        );
    }
    if method == "GET" && bare_path == "/app.js" {
        return serve_static_asset(
            asset_app_js(),
            "text/javascript; charset=utf-8",
            if_none_match,
        );
    }
    if method == "GET" && bare_path == "/favicon.svg" {
        return serve_static_asset(
            asset_favicon_svg(),
            "image/svg+xml; charset=utf-8",
            if_none_match,
        );
    }
    if method == "GET" && bare_path == "/favicon.ico" {
        return serve_static_asset(asset_favicon_ico(), "image/x-icon", if_none_match);
    }
    if method == "GET" && bare_path == "/api/hello" {
        // amorce de la preuve de pair : identité + nonce à usage unique, aucun secret
        let Some(nonce) = issue_hello_nonce() else {
            return Reply::json(
                503,
                serde_json::json!({
                    "ok": false,
                    "error": "entropie OS indisponible"
                }),
            );
        };
        return Reply::json(
            200,
            serde_json::json!({
                "app": "mcp-control",
                "version": env!("CARGO_PKG_VERSION"),
                "nonce": nonce,
            }),
        );
    }
    if (method == "GET" || method == "POST") && bare_path == "/api/state" {
        let etag = manage::state_etag(state);
        if method == "GET" && if_none_match.is_some_and(|inm| etag_matches(inm, &etag)) {
            return Reply::new(304, "application/json", Vec::new()).with_header("ETag", etag);
        }
        return Reply::json(200, manage::state_json(state)).with_header("ETag", etag);
    }
    if method == "GET" && bare_path == "/api/detail" {
        if let Some(id) = query_param(path, "id") {
            let etag = manage::detail_etag(state, &id);
            if if_none_match.is_some_and(|inm| etag_matches(inm, &etag)) {
                return Reply::new(304, "application/json", Vec::new()).with_header("ETag", etag);
            }
            return Reply::json(200, manage::detail_json(state, &id)).with_header("ETag", etag);
        }
    }
    // Historique runtime : crashs, redémarrages, échecs de démarrage et
    // backoff — l'outil de supervision en lecture seule.
    if method == "GET" && bare_path == "/api/runtime" {
        return Reply::json(200, manage::runtime_json(state));
    }
    if method == "GET" && bare_path == "/api/store" {
        return Reply::json(200, manage::store_json());
    }
    if method == "GET" && bare_path == "/api/store-logos" {
        return Reply::new(
            200,
            "application/json",
            logos::LOGOS_JSON.as_bytes().to_vec(),
        );
    }
    // Table des frameworks MCP connus (id, label, langage) — l'UI en dérive
    // les badges sans dupliquer la liste côté JavaScript.
    if method == "GET" && bare_path == "/api/frameworks" {
        let arr: Vec<serde_json::Value> = crate::frameworks::FRAMEWORKS
            .iter()
            .map(|f| serde_json::json!({"id": f.id, "label": f.label, "lang": f.lang}))
            .collect();
        return Reply::json(200, serde_json::json!({"frameworks": arr}));
    }
    if method == "POST" && bare_path == "/api/action" {
        // Les actions d'authentification sont LOCALES (droit `admin`, §4) : un
        // appel venu du LAN est refusé AVANT le dispatcher, même avec une
        // session valide (le cookie SameSite=Strict ne circule jamais en
        // cross-site, mais on ne s'en remet pas à lui seul).
        let nom = serde_json::from_slice::<serde_json::Value>(body)
            .ok()
            .and_then(|v| v.get("action").and_then(|a| a.as_str()).map(String::from))
            .unwrap_or_default();
        let local = peer.is_some_and(|ip| ip.is_loopback());
        // [H-06] : `/api/action` est l'API de contrôle de la fenêtre locale.
        // Toute requête provenant d'une adresse réseau non-loopback est refusée.
        if peer.is_some_and(|ip| !ip.is_loopback())
            || (crate::auth::action_reservee_admin(&nom) && !local)
        {
            return Reply::new(
                403,
                "application/json",
                br#"{"ok":false,"error":"action locale uniquement"}"#.to_vec(),
            );
        }
        let text = String::from_utf8_lossy(body).to_string();
        if nom == "server_pair" {
            let res = crate::auth::net::handle_server_pair(state, &text);
            gateway::notify_change();
            return Reply::json(200, res);
        }
        if nom == "refresh" {
            manage::invalidate_tools_cache(state);
        }
        let res = manage::handle_action(state, &text);
        if nom == "set_settings" || nom == "settings" || nom == "save_settings" {
            wake_bg_poller();
        }
        gateway::notify_change();
        return Reply::json(200, res);
    }
    Reply::new(
        404,
        "application/json",
        br#"{"ok":false,"error":"not found"}"#.to_vec(),
    )
}

pub fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect()
    }

    /// Appel de `check_access` avec les listes de test (Host/Origin du port
    /// 5090) : `session` = cookie de session valide, `legacy` = code hérité
    /// (`X-MCC-Access` / preuve de pair) valide.
    fn acc(
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        session: bool,
        legacy: bool,
    ) -> Result<(), (u16, &'static str)> {
        check_access(
            &["127.0.0.1".to_string()],
            &["http://127.0.0.1:5090".to_string()],
            method,
            path,
            &hs(headers),
            session,
            legacy,
        )
    }

    #[test]
    fn check_access_options_est_toujours_refuse() {
        let r = acc(
            "OPTIONS",
            "/api/action",
            &[("host", "127.0.0.1:5090")],
            false,
            false,
        );
        assert_eq!(r, Err((403, "Forbidden")));
    }

    #[test]
    fn check_access_host_inconnu_refuse_dns_rebinding() {
        let hosts = vec![
            "127.0.0.1".to_string(),
            "192.0.2.10".to_string(),
            "::1".to_string(),
        ];
        let origins = vec![];
        let check = |h: &str| {
            check_access(
                &hosts,
                &origins,
                "GET",
                "/api/state",
                &hs(&[("host", h)]),
                true,
                false,
            )
        };
        assert_eq!(check("evil.example.com:5090"), Err((403, "Forbidden")));
        // host autorisé, avec ou sans port, IPv6 entre crochets
        for h in ["127.0.0.1:5090", "127.0.0.1", "[::1]:5090"] {
            assert!(check(h).is_ok(), "host {h} doit passer");
        }
    }

    #[test]
    fn check_access_origin_inconnue_refuse() {
        let r = acc(
            "POST",
            "/api/action",
            &[
                ("host", "127.0.0.1:5090"),
                ("origin", "https://malveille.example"),
            ],
            true,
            false,
        );
        assert_eq!(r, Err((403, "Forbidden")));
        let ok = acc(
            "GET",
            "/api/state",
            &[
                ("host", "127.0.0.1:5090"),
                ("origin", "http://127.0.0.1:5090"),
            ],
            true,
            false,
        );
        assert!(ok.is_ok());
    }

    /// Lot M2 (DA2 = a) : le loopback n'est plus une zone de confiance.
    /// `?access=` n'existe plus (D5 clos) ; `X-MCC-Access` ne donne que la
    /// LECTURE ; sans session ni code hérité, `/api/*` rend 401 — d'où que
    /// vienne la requête.
    #[test]
    fn check_access_api_exige_la_session_locale_meme_en_loopback() {
        let base = &[("host", "127.0.0.1:5090")] as &[(&str, &str)];
        // aucune identité → 401
        assert_eq!(
            acc("GET", "/api/state", base, false, false),
            Err((401, "Unauthorized"))
        );
        // `?access=` ne donne plus rien (supprimé au lot M2)
        assert_eq!(
            acc("GET", "/api/detail?id=x&access=abc", base, false, false),
            Err((401, "Unauthorized"))
        );
        // session valide → OK
        assert!(acc("GET", "/api/state", base, true, false).is_ok());
        // code hérité : la LECTURE passe, l'écriture est refusée
        assert!(acc("GET", "/api/state", base, false, true).is_ok());
        assert_eq!(
            acc(
                "POST",
                "/api/action",
                &[
                    ("host", "127.0.0.1:5090"),
                    ("content-type", "application/json"),
                ],
                false,
                true,
            ),
            Err((403, "Forbidden"))
        );
        // /api/hello reste pré-authentifié ; /api/session crée la session
        assert!(acc("GET", "/api/hello", base, false, false).is_ok());
        // S-1 : plus d'en-tête CSRF constant — l'en-tête SECRET `X-MCC-Session`
        // est vérifié en amont (`session_ok`) ; une session valide + JSON passe.
        assert!(acc(
            "POST",
            "/api/action",
            &[
                ("host", "127.0.0.1:5090"),
                ("content-type", "application/json"),
            ],
            true,
            false,
        )
        .is_ok());
        // `/mcp` n'est plus filtré ici : sa porte est la signature (§2.2)
        assert!(acc(
            "POST",
            "/mcp",
            &[
                ("host", "127.0.0.1:5090"),
                ("content-type", "application/json")
            ],
            false,
            false,
        )
        .is_ok());
    }

    #[test]
    fn check_access_action_post_exige_content_type_json() {
        let base = &[("host", "127.0.0.1:5090")] as &[(&str, &str)];
        let with_ct = |ct: Option<&'static str>| {
            let mut h: Vec<(&str, &str)> = base.to_vec();
            if let Some(ct) = ct {
                h.push(("content-type", ct));
            }
            acc("POST", "/api/action", &h, true, false)
        };
        // text/plain = requête « simple » sans préflight depuis un site web → 415
        assert_eq!(
            with_ct(Some("text/plain")),
            Err((415, "Unsupported Media Type"))
        );
        // application/json (+paramètres) → OK ; absent → 415
        assert!(with_ct(Some("application/json; charset=utf-8")).is_ok());
        assert_eq!(with_ct(None), Err((415, "Unsupported Media Type")));
        // GET /api/state sans content-type : pas concerné
        assert!(acc("GET", "/api/state", base, true, false).is_ok());
    }

    #[test]
    fn is_loopback_bind_cas_connus() {
        for s in ["127.0.0.1", "localhost", "::1", "[::1]"] {
            assert!(is_loopback_bind(s), "{} = loopback", s);
        }
        for s in ["0.0.0.0", "", "192.0.2.10"] {
            assert!(!is_loopback_bind(s), "{} = PAS loopback", s);
        }
    }

    #[test]
    fn code_acces_jamais_exige_aux_clients_locaux() {
        // bind loopback : aucun code, quel que soit le client
        assert!(!effective_require_access(true, true));
        assert!(!effective_require_access(true, false));
        // bind 0.0.0.0 : client LOCAL (desktop, navigateur de la machine) → pas de code
        assert!(
            !effective_require_access(false, true),
            "une connexion venue de la machine n'exige jamais le code"
        );
        // bind 0.0.0.0 + client du LAN → code exigé
        assert!(effective_require_access(false, false));
    }

    #[test]
    fn query_param_extrait_et_decode() {
        assert_eq!(
            query_param("/api/detail?id=a%20b&access=c+d", "access"),
            Some("c d".to_string())
        );
        assert_eq!(query_param("/api/detail?id=x", "access"), None);
        assert_eq!(query_param("/api/state", "access"), None);
    }

    #[test]
    fn generate_access_fait_32_hex_et_est_unique() {
        let a = generate_access();
        let b = generate_access();
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b, "deux tirages RandomState ne doivent pas coller");
    }

    #[test]
    fn random_hex_nonce_est_corrige_et_non_correle() {
        // CSPRNG obligatoire : les mots successifs d'un nonce (et d'un appel au
        // suivant) ne doivent montrer AUCUNE corrélation — RandomState suivait
        // un compteur thread-local, prévisible après observation.
        let a = random_hex(4).expect("entropie OS");
        let b = random_hex(4).expect("entropie OS");
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
        // mots distincts à l'intérieur d'un même tirage (64 bits chacun)
        let mots: Vec<&str> = a
            .as_bytes()
            .chunks(16)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect();
        assert_eq!(mots.len(), 4);
        assert!(
            mots.iter().collect::<std::collections::HashSet<_>>().len() == 4,
            "mots identiques dans un même nonce : entropie insuffisante"
        );
        // os_random répond (source OS toujours disponible en pratique)
        let mut buf = [0u8; 8];
        assert!(os_random(&mut buf), "source d'entropie OS indisponible");
    }

    #[test]
    fn urldecode_classique() {
        assert_eq!(urldecode("seed-overmind"), "seed-overmind");
        assert_eq!(urldecode("a%20b+c"), "a b c");
        assert_eq!(urldecode("caf%C3%A9"), "caf\u{e9}");
        assert_eq!(urldecode("%2Fapi%3Fid%3D1"), "/api?id=1");
    }

    #[test]
    fn urldecode_tolerant_aux_sequences_invalides() {
        assert_eq!(urldecode("100%"), "100%");
        assert_eq!(urldecode("%ZZ"), "%ZZ");
        assert_eq!(urldecode("%A"), "%A");
    }

    #[test]
    fn find_sub_cas_limites() {
        assert_eq!(find_sub(b"\r\n\r\n", b"\r\n\r\n"), Some(0));
        assert_eq!(find_sub(&[13u8, 10u8], b"\r\n\r\n"), None);
    }

    #[test]
    fn check_access_hello_preauth_et_legacy_en_lecture_seule() {
        let base = &[("host", "127.0.0.1:5090")] as &[(&str, &str)];
        // /api/hello est pré-authentifié (nonce seulement), sans session
        assert!(acc("GET", "/api/hello", base, false, false).is_ok());
        // une preuve de pair valide (code hérité, §10) donne la LECTURE
        assert!(acc("GET", "/api/state", base, false, true).is_ok());
        // sans preuve, sans code, sans session → 401
        assert_eq!(
            acc("GET", "/api/state", base, false, false),
            Err((401, "Unauthorized"))
        );
    }

    #[test]
    fn nonce_hello_usage_unique_et_verif_proof() {
        let nonce = issue_hello_nonce().expect("entropie OS");
        assert_eq!(nonce.len(), 64);
        let proof = crate::crypto::hmac_sha256_hex(
            b"code-partage",
            format!("mcc-proof-v1:{nonce}").as_bytes(),
        );
        assert!(verify_proof(&nonce, &proof, "code-partage"));
        // usage unique : la même preuve ne repasse pas
        assert!(!verify_proof(&nonce, &proof, "code-partage"));
        // mauvais code partagé → rejet (et le nonce est consommé par le 1er essai)
        let nonce2 = issue_hello_nonce().expect("entropie OS");
        let proof2 = crate::crypto::hmac_sha256_hex(
            b"code-partage",
            format!("mcc-proof-v1:{nonce2}").as_bytes(),
        );
        assert!(!verify_proof(&nonce2, &proof2, "autre-code"));
        // nonce inconnu / format invalide → rejet sans panique
        assert!(!verify_proof("inconnu", "00", "code-partage"));
        assert!(!verify_proof(&"a".repeat(64), &proof, "code-partage"));
    }

    fn test_state_with_access(code: Option<&str>) -> std::sync::Arc<crate::types::AppState> {
        std::sync::Arc::new(crate::types::AppState {
            servers: Mutex::new(vec![]),
            log: Mutex::new(std::collections::VecDeque::new()),
            settings: Mutex::new(crate::types::Settings {
                api_access: code.map(String::from),
                ..crate::types::Settings::default()
            }),
            children: Mutex::new(std::collections::HashMap::new()),
            config_path: String::new(),
            peers: Mutex::new(std::collections::HashMap::new()),
            allowed_origins: Mutex::new(vec![]),
            allowed_hosts: Mutex::new(vec!["127.0.0.1".to_string()]),
            spawn_fails: Mutex::new(std::collections::HashMap::new()),
            runtime: Mutex::new(crate::runtime::RuntimeLog::new()),
            spawning: Mutex::new(std::collections::HashSet::new()),
            update_latest: Mutex::new(None),
            lan_ok: Mutex::new(None),
        })
    }

    /// (lot M1) Les deux routes d'appairage sont joignables SANS identité (c'est
    /// leur objet), bornées par le quota d'appairage, et les actions `admin`
    /// sont refusées dès qu'elles viennent du LAN.
    #[test]
    fn appairage_ouvert_mais_borne_et_actions_admin_interdites_au_lan() {
        let state = test_state_with_access(None);
        lockv(&state.settings).auth.server = Some(crate::auth::ServerIdentity {
            keyid: crate::auth::SERVER_KEYID.to_string(),
            public_key: "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
                .to_string(),
            created_at: "2026-11-15 12:00:00".to_string(),
        });
        let loopback = Some("127.0.0.1".parse().unwrap());
        let corps = serde_json::json!({
            "keyid": "mcc-a-0000000000000001",
            "label": "portable",
            "public_key": "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
            "created": crate::types::now_ts(),
        })
        .to_string();
        let (s, ctype, body) =
            route_peer(&state, "POST", "/auth/pair", corps.as_bytes(), loopback).into_parts();
        assert_eq!(
            s,
            200,
            "POST /auth/pair doit être ouvert : {}",
            String::from_utf8_lossy(&body)
        );
        assert_eq!(ctype, "application/json");
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["api"], "mcc-auth-v1");
        assert_eq!(v["server_keyid"], crate::auth::SERVER_KEYID);
        assert_eq!(v["tls_fp"], "");
        let pair_id = v["pair_id"].as_str().unwrap_or("").to_string();
        assert_eq!(pair_id.len(), 32);
        // le statut se lit sans identité ; un identifiant inconnu rend 404
        let (s, _, body) = route_peer(
            &state,
            "GET",
            &format!("/auth/pair/{pair_id}"),
            b"",
            loopback,
        )
        .into_parts();
        assert_eq!(s, 200);
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["status"], "pending");
        assert_eq!(
            route_peer(
                &state,
                "GET",
                &format!("/auth/pair/{}", "0".repeat(32)),
                b"",
                loopback
            )
            .status,
            404
        );
        // méthodes inattendues : refus explicite, jamais un dispatch
        assert_eq!(
            route_peer(&state, "GET", "/auth/pair", b"", loopback).status,
            405
        );
        assert_eq!(
            route_peer(
                &state,
                "POST",
                &format!("/auth/pair/{pair_id}"),
                b"{}",
                loopback
            )
            .status,
            405
        );

        // actions `admin` : la session locale n'existe pas encore (M2), mais un
        // appel venu du LAN est déjà refusé AVANT le dispatcher.
        let action = br#"{"action":"client_list"}"#;
        assert_eq!(
            route_peer(&state, "POST", "/api/action", action, loopback).status,
            200
        );
        assert_eq!(
            route_peer(
                &state,
                "POST",
                "/api/action",
                action,
                Some("192.0.2.30".parse().unwrap())
            )
            .status,
            403,
            "une action d'authentification ne doit jamais être joignable du LAN"
        );
        // [H-06] : toute action POST /api/action venue du LAN est refusée par 403
        // (l'API de contrôle appartient exclusivement à la session locale).
        assert_eq!(
            route_peer(
                &state,
                "POST",
                "/api/action",
                br#"{"action":"client_inconnu_xyz"}"#,
                Some("192.0.2.30".parse().unwrap())
            )
            .status,
            403
        );
        assert_eq!(
            route_peer(
                &state,
                "POST",
                "/api/action",
                br#"{"action":"client_inconnu_xyz"}"#,
                loopback
            )
            .status,
            200
        );
    }

    #[test]
    fn csp_stricte_sans_script_ni_style_en_ligne() {
        // Lot S2 : la page ne doit plus rien pouvoir exécuter « en ligne ».
        // Une régression qui réintroduit 'unsafe-inline' ou un nonce casse ici.
        assert!(UI_CSP.contains("script-src 'self'"), "{UI_CSP}");
        assert!(UI_CSP.contains("style-src 'self'"), "{UI_CSP}");
        assert!(!UI_CSP.contains("unsafe-inline"), "{UI_CSP}");
        assert!(!UI_CSP.contains("unsafe-eval"), "{UI_CSP}");
        assert!(!UI_CSP.contains("nonce-"), "{UI_CSP}");
        // coquille : ni <style>, ni <script> en ligne (uniquement des src=)
        assert!(
            !crate::ui::INDEX_HTML.contains("<style"),
            "<style> en ligne"
        );
        assert!(
            !crate::ui::INDEX_HTML.contains("<script>"),
            "<script> en ligne"
        );
        for h in [
            "X-Content-Type-Options: nosniff",
            "Referrer-Policy: no-referrer",
            "Cross-Origin-Opener-Policy: same-origin",
            "Cross-Origin-Resource-Policy: same-origin",
        ] {
            assert!(SECURITY_HEADERS.contains(h), "en-tête manquant : {h}");
        }
    }

    #[test]
    fn route_racine_ne_revele_jamais_le_code_acces() {
        // Régression P0 (audit 2026-10-24) : la page / embarquait le code d'accès,
        // quiconque joint le port le lisait et contournait toute l'auth API.
        let state = test_state_with_access(Some("code-secret-123"));
        let (status, ctype, body) = route(&state, "GET", "/", b"");
        assert_eq!(status, 200);
        assert!(ctype.starts_with("text/html"));
        let html = String::from_utf8_lossy(&body).to_string();
        assert!(
            !html.contains("code-secret-123"),
            "la page / ne doit PAS contenir le code d'accès"
        );
        assert!(
            html.contains("loginDlg"),
            "la page doit proposer le déverrouillage (loginDlg)"
        );
    }

    #[test]
    fn les_assets_ui_sont_servis_avec_type_et_version() {
        let state = test_state_with_access(None);
        let version = env!("CARGO_PKG_VERSION");
        for (path, ctype_attendu, contenu) in [
            ("/app.css", "text/css", "--accent"),
            ("/app-boot.js", "text/javascript", "dataset.theme"),
            ("/app.js", "text/javascript", "const MCC_VERSION"),
        ] {
            let (status, ctype, body) = route(&state, "GET", path, b"");
            assert_eq!(status, 200, "{path} doit être servi");
            assert_eq!(ctype, format!("{ctype_attendu}; charset=utf-8"), "{path}");
            let text = String::from_utf8_lossy(&body).to_string();
            assert!(text.contains(contenu), "{path} : contenu attendu absent");
            assert!(
                !text.contains("__MCC_VERSION__"),
                "{path} : placeholder de version NON substitué"
            );
        }
        // la coquille pointe vers les assets VERSIONNÉS (substitution faite)
        let (_, _, shell) = route(&state, "GET", "/", b"");
        let html = String::from_utf8_lossy(&shell).to_string();
        assert!(
            html.contains(&format!("/app.css?v={version}")),
            "lien css non versionné"
        );
        assert!(
            html.contains(&format!("/app.js?v={version}")),
            "lien js non versionné"
        );
        // asset inconnu → pas de 200 travesti (le 404 du routeur prend le relais)
        let (status, _, _) = route(&state, "GET", "/app.inexistant", b"");
        assert_ne!(status, 200);
    }

    #[test]
    fn route_api_hello_renvoie_un_nonce_sans_secret() {
        let state = test_state_with_access(Some("code-secret-123"));
        let (status, ctype, body) = route(&state, "GET", "/api/hello", b"");
        assert_eq!(status, 200);
        assert!(ctype.starts_with("application/json"));
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["app"], "mcp-control");
        assert_eq!(v["nonce"].as_str().unwrap().len(), 64);
        let text = String::from_utf8_lossy(&body).to_string();
        assert!(!text.contains("code-secret-123"), "aucun secret dans hello");
    }

    /// Le `?access=` de l'ancien code est SUPPRIMÉ (lot M2, clôt D5 et l'audit
    /// 05 #9) : la query ne donne plus aucun accès, et elle ne pollue pas non
    /// plus la lecture d'`id`.
    #[test]
    fn query_access_ne_donne_aucun_acces_et_ne_pollue_pas_l_id() {
        let base = &[("host", "127.0.0.1:5090")] as &[(&str, &str)];
        assert_eq!(
            acc(
                "GET",
                "/api/state?access=code-secret-123",
                base,
                false,
                false
            ),
            Err((401, "Unauthorized")),
            "?access= ne doit plus rien ouvrir"
        );
        let state = test_state_with_access(Some("code-secret-123"));
        // /api/detail avec ?id=...&access=... ne doit pas polluer l'id avec "&access=..."
        state.servers.lock().unwrap().push(crate::types::Server {
            id: "seed-overmind".into(),
            label: "Overmind".into(),
            transport: crate::types::Transport::Http {
                host: None,
                port: 3333,
                path: "/mcp".into(),
            },
            origin: crate::types::Origin::Seed,
            enabled: true,
            auto_restart: true,
            favorite: false,
            state: crate::types::SState::Up,
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
            start_cmd: None,
            tools_disabled: vec![],
        });
        let expected_id = "seed-overmind";
        let path = format!("/api/detail?id={}&access=code-secret-123", expected_id);
        let (st2, _, body2) = route(&state, "GET", &path, b"");
        assert_eq!(st2, 200);
        let v2: serde_json::Value = serde_json::from_slice(&body2).unwrap();
        assert_eq!(v2["id"], serde_json::json!(expected_id));
    }

    // ---------- lot B : recalcul des hôtes autorisés ----------

    #[test]
    fn listes_hosts_origins_boucle_ips_et_extras() {
        let (origins, hosts) = listes_hosts_origins(
            5392,
            &["10.2.0.2".to_string(), "192.168.1.5".to_string()],
            &["http://mon-ui.local:8080".to_string()],
        );
        assert!(origins.contains(&"http://127.0.0.1:5392".to_string()));
        assert!(origins.contains(&"https://127.0.0.1:5392".to_string()));
        assert!(origins.contains(&"http://localhost:5392".to_string()));
        assert!(origins.contains(&"https://localhost:5392".to_string()));
        assert!(origins.contains(&"http://[::1]:5392".to_string()));
        assert!(origins.contains(&"https://[::1]:5392".to_string()));
        assert!(origins.contains(&"http://10.2.0.2:5392".to_string()));
        assert!(origins.contains(&"https://10.2.0.2:5392".to_string()));
        assert!(origins.contains(&"http://mon-ui.local:8080".to_string()));
        for h in [
            "127.0.0.1",
            "localhost",
            "::1",
            "10.2.0.2",
            "192.168.1.5",
            "mon-ui.local",
        ] {
            assert!(hosts.contains(&h.to_string()), "host {h} attendu");
        }
    }

    #[test]
    fn recalc_limite_de_frequence() {
        let maintenant = Instant::now();
        assert!(recalc_permis(None, maintenant), "jamais calculé : permis");
        let ancien = maintenant - HOSTS_RECALC_INTERVAL;
        assert!(
            recalc_permis(Some(ancien), maintenant),
            "fenêtre écoulée : permis"
        );
        let recent = maintenant - Duration::from_millis(500);
        assert!(
            !recalc_permis(Some(recent), maintenant),
            "dans la fenêtre : refusé (pas d'énumération d'interfaces à chaque requête)"
        );
    }

    // ---------- lot 5 : sécurité HTTP ----------

    #[test]
    fn admission_connexions_plafonds_et_reserve_loopback() {
        use super::{conn_admission, MAX_CONNS, MAX_CONNS_PER_ADDR};
        // sous les plafonds : admis
        assert!(conn_admission(10, 5, 3, false).is_ok());
        // 64e connexion totale : encore admise ; 65e : saturation
        assert!(conn_admission(MAX_CONNS, 0, 0, true).is_ok());
        assert_eq!(
            conn_admission(MAX_CONNS + 1, 0, 0, true),
            Err("serveur saturé (plafond de connexions simultanées) — réessayez dans un instant")
        );
        // hors loopback : 48 = part LAN exactement pleine (48e LAN admis) ;
        // 49e LAN → refus, même si le total est loin du plafond global
        assert!(conn_admission(48, 48, 1, false).is_ok());
        assert_eq!(
            conn_admission(49, 49, 1, false),
            Err("plafond de connexions hors loopback atteint (part réservée aux clients locaux)")
        );
        // le loopback IGNORE la réserve : 48 LAN + 1 local = le local passe
        assert!(conn_admission(49, 48, 1, true).is_ok());
        // plafond par adresse, hors loopback uniquement — le compteur inclut
        // DÉJÀ la connexion candidate (admit_conn incrémente avant de décider)
        // : la 16e passe, la 17e est refusée
        assert!(conn_admission(20, 10, MAX_CONNS_PER_ADDR, false).is_ok());
        assert_eq!(
            conn_admission(20, 10, MAX_CONNS_PER_ADDR + 1, false),
            Err("trop de connexions simultanées depuis votre adresse")
        );
        assert!(conn_admission(20, 10, MAX_CONNS_PER_ADDR - 1, false).is_ok());
        // le loopback n'a pas de plafond par adresse
        assert!(conn_admission(20, 0, usize::MAX, true).is_ok());
    }

    #[test]
    fn check_access_sans_host_rejete_en_400() {
        // audit 05 #8 : sans en-tête Host, le contrôle anti DNS-rebinding était
        // sauté (requête acceptée) — désormais 400, même en loopback
        let hosts = vec!["127.0.0.1".to_string()];
        for (meth, chemin) in [
            ("GET", "/api/state"),
            ("GET", "/api/hello"),
            ("GET", "/"),
            ("POST", "/api/action"),
        ] {
            let r = check_access(&hosts, &[], meth, chemin, &hs(&[]), false, false);
            assert_eq!(r, Err((400, "Bad Request")), "{meth} {chemin} sans Host");
        }
        // avec Host présent : comportements inchangés
        assert!(check_access(
            &hosts,
            &[],
            "GET",
            "/api/state",
            &hs(&[("host", "127.0.0.1:5090")]),
            true,
            false
        )
        .is_ok());
    }

    #[test]
    fn eviction_hello_garde_les_plus_recents() {
        use super::evict_oldest_nonces;
        let mut m: HashMap<String, u64> = HashMap::new();
        for i in 0..8u64 {
            m.insert(format!("n{i:03}"), 1000 + i); // n000 le plus ancien
        }
        // table déjà sous le max : rien à évincer
        assert_eq!(evict_oldest_nonces(&mut m, 8), 0);
        // max 5 : les 3 plus anciens sortent, les récents restent
        assert_eq!(evict_oldest_nonces(&mut m, 5), 3);
        assert!(!m.contains_key("n000"));
        assert!(!m.contains_key("n002"));
        assert!(m.contains_key("n007"), "le plus récent reste valide");
        assert_eq!(m.len(), 5);
        // max 0 : vidage propre, pas de boucle infinie
        assert_eq!(evict_oldest_nonces(&mut m, 0), 5);
        assert!(m.is_empty());
    }

    #[test]
    fn limiteur_debit_hello_fenetre_glissante() {
        use super::{hello_rate_decide, HELLO_RATE_MAX, HELLO_RATE_WINDOW_SECS};
        let mut hist: Vec<u64> = Vec::new();
        for _ in 0..HELLO_RATE_MAX {
            let (d, next) = hello_rate_decide(1000, &hist);
            assert!(
                d.is_ok(),
                "les {} premières requêtes passent",
                HELLO_RATE_MAX
            );
            hist = next;
        }
        // au-delà du plafond → refus, l'historique reste conservé pour la fenêtre
        let (d, next) = hello_rate_decide(1000, &hist);
        assert!(
            d.is_err(),
            "la requête {} doit être refusée",
            HELLO_RATE_MAX + 1
        );
        assert_eq!(next.len(), HELLO_RATE_MAX);
        // timestamps sortis de la fenêtre : purgés → de nouveau admis
        let (d2, next2) = hello_rate_decide(1000 + HELLO_RATE_WINDOW_SECS, &hist);
        assert!(d2.is_ok(), "fenêtre purgée : requête de nouveau admise");
        assert_eq!(next2, vec![1000 + HELLO_RATE_WINDOW_SECS]);
    }

    #[test]
    fn check_host_port_bloque_rejeu_inter_ports_et_mcp_sans_port() {
        // Port exact → OK (MCP et autres routes)
        assert!(check_host_port(&hs(&[("host", "127.0.0.1:5591")]), 5591, true).is_ok());
        assert!(check_host_port(&hs(&[("host", "[::1]:5591")]), 5591, true).is_ok());
        assert!(check_host_port(&hs(&[("host", "127.0.0.1:5591")]), 5591, false).is_ok());
        // Port d'une autre instance → 403 (ferme le rejeu inter-instances R-1)
        assert_eq!(
            check_host_port(&hs(&[("host", "127.0.0.1:5590")]), 5591, true),
            Err((403, "Forbidden"))
        );
        assert_eq!(
            check_host_port(&hs(&[("host", "127.0.0.1:5590")]), 5591, false),
            Err((403, "Forbidden"))
        );
        // /mcp sans port explicite : refusé hors 80/443, autorisé sur 80/443
        assert_eq!(
            check_host_port(&hs(&[("host", "127.0.0.1")]), 5591, true),
            Err((403, "Forbidden"))
        );
        assert!(check_host_port(&hs(&[("host", "127.0.0.1")]), 80, true).is_ok());
        assert!(check_host_port(&hs(&[("host", "127.0.0.1")]), 443, true).is_ok());
        // Hors /mcp sans port explicite : compatibilité des sondes simples
        assert!(check_host_port(&hs(&[("host", "127.0.0.1")]), 5591, false).is_ok());
        // Port mal formé → 400
        assert_eq!(
            check_host_port(&hs(&[("host", "127.0.0.1:abc")]), 5591, true),
            Err((400, "Bad Request"))
        );
    }

    #[test]
    fn check_access_rejette_entetes_dupliques_et_content_length_invalide() {
        // 1. Host dupliqué → 400
        assert_eq!(
            acc(
                "GET",
                "/",
                &[("host", "127.0.0.1:5090"), ("host", "evil.example:5090")],
                false,
                false,
            ),
            Err((400, "Bad Request"))
        );
        // 2. Origin dupliqué → 400
        assert_eq!(
            acc(
                "GET",
                "/",
                &[
                    ("host", "127.0.0.1:5090"),
                    ("origin", "http://127.0.0.1:5090"),
                    ("origin", "https://evil.example"),
                ],
                false,
                false,
            ),
            Err((400, "Bad Request"))
        );
        // 3. X-MCC-Session dupliqué → 400
        assert_eq!(
            acc(
                "GET",
                "/api/state",
                &[
                    ("host", "127.0.0.1:5090"),
                    ("x-mcc-session", "a"),
                    ("x-mcc-session", "b"),
                ],
                true,
                false,
            ),
            Err((400, "Bad Request"))
        );
        // 4. Plusieurs cookies mcc_session= (dans un ou plusieurs en-têtes Cookie) → 400
        assert_eq!(
            acc(
                "GET",
                "/api/state",
                &[
                    ("host", "127.0.0.1:5090"),
                    ("cookie", "mcc_session=a"),
                    ("cookie", "mcc_session=b"),
                ],
                true,
                false,
            ),
            Err((400, "Bad Request"))
        );
        assert_eq!(
            acc(
                "GET",
                "/api/state",
                &[
                    ("host", "127.0.0.1:5090"),
                    ("cookie", "mcc_session=a; mcc_session=b"),
                ],
                true,
                false,
            ),
            Err((400, "Bad Request"))
        );
        // 5. Content-Length non numérique / négatif / vide → 400
        for bad_cl in ["abc", "-5", "", "+10", "999999999999999999999999999999"] {
            assert_eq!(
                acc(
                    "POST",
                    "/api/action",
                    &[
                        ("host", "127.0.0.1:5090"),
                        ("content-type", "application/json"),
                        ("content-length", bad_cl),
                    ],
                    true,
                    false,
                ),
                Err((400, "Bad Request")),
                "Content-Length: {bad_cl} doit rendre 400"
            );
        }
    }

    #[test]
    fn random_hex_echoue_ferme_si_entropie_os_indisponible() {
        assert_eq!(random_hex_with(2, |_| false), None);
        assert_eq!(
            random_hex_with(1, |b| {
                b.copy_from_slice(&[0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]);
                true
            }),
            Some("efcdab8967452301".to_string())
        );
    }

    #[test]
    fn p1_etag_matches_et_304_api_et_statiques() {
        assert!(etag_matches("\"s-abc\"", "\"s-abc\""));
        assert!(etag_matches("W/\"s-abc\"", "\"s-abc\""));
        assert!(etag_matches("\"other\", \"s-abc\"", "\"s-abc\""));
        assert!(etag_matches("*", "\"s-abc\""));
        assert!(!etag_matches("\"s-def\"", "\"s-abc\""));

        let state = test_state_with_access(None);
        state.servers.lock().unwrap().push(crate::types::Server {
            id: "s1".into(),
            label: "S1".into(),
            transport: crate::types::Transport::Http {
                host: Some("127.0.0.1".into()),
                port: 19995,
                path: "/mcp".into(),
            },
            origin: crate::types::Origin::Manual,
            enabled: true,
            state: crate::types::SState::Up,
            pid: None,
            bound_host: Some("127.0.0.1".into()),
            server_name: Some("demo".into()),
            version: Some("1.0".into()),
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
            latency_ms: Some(5),
            last_seen_ts: Some(100),
            last_error: None,
            start_cmd: None,
            auto_restart: false,
            favorite: false,
            tools_disabled: vec![],
        });

        let loopback = Some("127.0.0.1".parse().unwrap());

        // 1. /api/state → 200 + ETag
        let r1 = route_peer_session_cond(&state, "GET", "/api/state", b"", loopback, None, None);
        assert_eq!(r1.status, 200);
        let etag1 = r1
            .headers
            .iter()
            .find(|(k, _)| k == "ETag")
            .map(|(_, v)| v.clone())
            .expect("ETag manquant sur /api/state");
        assert!(
            etag1.starts_with("\"s-"),
            "ETag de /api/state attendu (préfixe s-) : {etag1}"
        );

        // 2. Même état + changements volatils seuls (latency_ms, last_seen_ts) → 304 sans corps.
        // L'ETag de /api/state couvre aussi, à juste titre, des compteurs GLOBAUX au
        // processus (confirmations, sessions actives, statut de mise à jour, annonces
        // LiteMCP) que les autres tests, parallèles dans le même binaire, modifient
        // sans arrêt : un 200 peut donc être légitime (« left: 200, right: 304 »
        // intermittent en CI). On ne valide que des FENÊTRES STABLES : deux lectures
        // directes de l'ETag, avant/après le seul changement volatil, doivent être
        // identiques (une fuite de latency_ms/last_seen_ts les ferait différer à
        // chaque essai), et le 304 HTTP est ensuite exigé dans cette fenêtre.
        let mut r2 = None;
        for _ in 0..200 {
            {
                let mut srvs = lockv(&state.servers);
                srvs[0].latency_ms = Some(5);
                srvs[0].last_seen_ts = Some(100);
            }
            let e1 = manage::state_etag(&state);
            {
                let mut srvs = lockv(&state.servers);
                srvs[0].latency_ms = Some(42);
                srvs[0].last_seen_ts = Some(999);
            }
            let e2 = manage::state_etag(&state);
            if e1 != e2 {
                std::thread::sleep(std::time::Duration::from_millis(10));
                continue; // la partie globale a bougé : fenêtre invalide, on recommence
            }
            let r = route_peer_session_cond(
                &state,
                "GET",
                "/api/state",
                b"",
                loopback,
                None,
                Some(&e2),
            );
            if r.status == 304 {
                r2 = Some(r);
                break;
            }
            assert_eq!(r.status, 200, "état inchangé : 200 ou 304 attendus");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let r2 = r2.expect(
            "aucune fenêtre stable obtenue : soit latency_ms/last_seen_ts fuient dans \
             l'ETag, soit les compteurs globaux n'ont pas cessé de changer pendant le test",
        );
        assert!(r2.body.is_empty(), "304 ne doit porter aucun corps");
        let baseline = manage::state_etag(&state);

        // 3. Changement réel (state → Down) → 200 + nouvel ETag
        {
            lockv(&state.servers)[0].state = crate::types::SState::Down;
        }
        let r3 = route_peer_session_cond(
            &state,
            "GET",
            "/api/state",
            b"",
            loopback,
            None,
            Some(&baseline),
        );
        assert_eq!(r3.status, 200);
        let etag3 = r3
            .headers
            .iter()
            .find(|(k, _)| k == "ETag")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert_ne!(baseline, etag3);

        // 4. /api/detail?id=s1 → 200 + ETag → 304 sur If-None-Match
        let d1 = route_peer_session_cond(
            &state,
            "GET",
            "/api/detail?id=s1",
            b"",
            loopback,
            None,
            None,
        );
        assert_eq!(d1.status, 200);
        let detag = d1
            .headers
            .iter()
            .find(|(k, _)| k == "ETag")
            .map(|(_, v)| v.clone())
            .expect("ETag manquant sur /api/detail");
        let d2 = route_peer_session_cond(
            &state,
            "GET",
            "/api/detail?id=s1",
            b"",
            loopback,
            None,
            Some(&detag),
        );
        assert_eq!(d2.status, 304);
        assert!(d2.body.is_empty());

        // 5. Assets statiques → 200 + ETag → 304 sur If-None-Match
        for path in ["/app.css", "/app-boot.js", "/app.js"] {
            let a1 = route_peer_session_cond(&state, "GET", path, b"", loopback, None, None);
            assert_eq!(a1.status, 200, "{path} doit rendre 200");
            let aetag = a1
                .headers
                .iter()
                .find(|(k, _)| k == "ETag")
                .map(|(_, v)| v.clone())
                .expect("ETag manquant sur asset statique");
            let a2 =
                route_peer_session_cond(&state, "GET", path, b"", loopback, None, Some(&aetag));
            assert_eq!(a2.status, 304, "{path} doit rendre 304");
            assert!(a2.body.is_empty());
        }
    }
}
