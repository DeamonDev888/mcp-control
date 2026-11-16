//! Session locale de la fenêtre — `docs/auth.md` §6 (DA2 = a).
//!
//! Au lancement, le moteur tire un **jeton d'amorçage** de 32 octets du système
//! (base64url), valable {@link BOOTSTRAP_TTL} et **à usage unique**. Le shell
//! Tauri navigue vers `http://127.0.0.1:<port>/#t=<jeton>` (fragment : jamais
//! envoyé au serveur, jamais dans `Referer`, jamais dans `mcc.log`) ; `app.js`
//! l'échange sur `POST /api/session` contre le cookie `mcc_session`, puis efface
//! le fragment. En mode headless, la même URL est imprimée sur la console.
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

use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// Durée de vie du jeton d'amorçage (§6). Passé ce délai il faut relancer le
/// moteur : aucun jeton ne traîne sur disque.
pub const BOOTSTRAP_TTL: Duration = Duration::from_secs(300);

/// Nom du cookie de session (§6).
pub const COOKIE_NAME: &str = "mcc_session";

/// En-tête anti-CSRF exigé sur toute requête qui change l'état (§6). Un en-tête
/// personnalisé impose un préflight CORS, que le serveur refuse (403).
pub const CSRF_HEADER: &str = "x-mcc-request";

/// Jeton d'amorçage en attente d'échange.
struct Bootstrap {
    token: String,
    issued: Instant,
}

/// Session vivante (une seule : la fenêtre et ses onglets partagent le cookie).
struct Session {
    id: String,
    #[allow(dead_code)] // trace de vie : le cookie meurt avec le process
    issued: Instant,
}

#[derive(Default)]
struct Slot {
    bootstrap: Option<Bootstrap>,
    session: Option<Session>,
}

fn slot() -> MutexGuard<'static, Slot> {
    static SLOT: OnceLock<Mutex<Slot>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(Slot::default()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

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

/// `bytes` octets du système, en base64url. Repli (jamais observé : les deux
/// sources OS sont toujours là) : le mélange de `random_hex`, qui reste
/// improvisible entre processus.
fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    if !crate::os_random(&mut buf) {
        let hex = crate::random_hex(bytes.div_ceil(8));
        for (i, slot) in buf.iter_mut().enumerate() {
            let pair = hex.get(i * 2..i * 2 + 2).unwrap_or("00");
            *slot = u8::from_str_radix(pair, 16).unwrap_or(0);
        }
    }
    b64url(&buf)
}

/// Tire un nouveau jeton d'amorçage (32 octets, usage unique, 5 minutes) et
/// l'installe dans le slot. Un jeton précédent non échangé devient invalide :
/// une seule URL à usage unique est vivante à la fois.
pub fn mint_bootstrap() -> String {
    let token = random_token(32);
    let mut s = slot();
    s.bootstrap = Some(Bootstrap {
        token: token.clone(),
        issued: Instant::now(),
    });
    token
}

/// URL à usage unique à ouvrir dans la fenêtre ou le navigateur : le jeton vit
/// dans le **fragment**, qui n'est jamais transmis au serveur.
pub fn bootstrap_url(base: &str) -> String {
    format!("{}/#t={}", base.trim_end_matches('/'), mint_bootstrap())
}

/// Échange du jeton contre une session. `None` = jeton inconnu, déjà consommé
/// ou périmé (échec fermé : aucun cookie n'est émis).
pub fn exchange(token: &str) -> Option<String> {
    let mut s = slot();
    let boot = s.bootstrap.as_ref()?;
    if boot.issued.elapsed() > BOOTSTRAP_TTL {
        s.bootstrap = None;
        return None;
    }
    if !crate::crypto::ct_eq(boot.token.as_bytes(), token.as_bytes()) {
        return None;
    }
    // Usage unique : le jeton est consommé AVANT de rendre la session.
    s.bootstrap = None;
    let id = random_token(32);
    s.session = Some(Session {
        id: id.clone(),
        issued: Instant::now(),
    });
    Some(id)
}

/// Valeur du cookie `mcc_session` présentée par la requête, s'il y en a une.
pub fn cookie_value(headers: &[(String, String)]) -> Option<String> {
    headers
        .iter()
        .filter(|(n, _)| n == "cookie")
        .flat_map(|(_, v)| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE_NAME)
        .map(|(_, value)| value.trim().to_string())
}

/// La requête porte-t-elle une session valide ? (`false` = pas de session, ou
/// session d'un moteur précédent, ou valeur forgée.)
pub fn authorized(headers: &[(String, String)]) -> bool {
    let Some(presented) = cookie_value(headers) else {
        return false;
    };
    let s = slot();
    match s.session.as_ref() {
        Some(session) => crate::crypto::ct_eq(session.id.as_bytes(), presented.as_bytes()),
        None => false,
    }
}

/// La requête porte-t-elle l'en-tête anti-CSRF de §6 ?
pub fn has_csrf_header(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .any(|(n, v)| n == CSRF_HEADER && v.trim() == "1")
}

/// Ligne `Set-Cookie` de la session (§6) : `HttpOnly`, `SameSite=Strict`,
/// `Path=/`, sans `Max-Age` (cookie de session) et sans `Secure` (HTTP
/// loopback). Aucune valeur n'est journalisée.
pub fn set_cookie_header(id: &str) -> String {
    format!("{COOKIE_NAME}={id}; HttpOnly; SameSite=Strict; Path=/")
}

/// Réinitialise l'état (tests unitaires uniquement : un process de test ne doit
/// pas hériter du jeton d'un autre).
#[cfg(test)]
pub fn reset_for_tests() {
    let mut s = slot();
    s.bootstrap = None;
    s.session = None;
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
        reset_for_tests();
        let token = mint_bootstrap();
        assert_eq!(
            token.len(),
            43,
            "32 octets base64url = 43 caractères : {token}"
        );
        // premier échange : cookie émis
        let cookie = exchange(&token).expect("jeton frais accepté");
        assert_eq!(cookie.len(), 43);
        // second échange du même jeton : refusé (usage unique)
        assert!(exchange(&token).is_none(), "jeton déjà consommé");
        // jeton inventé : refusé
        assert!(exchange(&"A".repeat(43)).is_none());
        // jeton vide : refusé
        assert!(exchange("").is_none());
    }

    #[test]
    fn cookie_valide_seulement_pour_la_session_vivante() {
        reset_for_tests();
        let cookie = exchange(&mint_bootstrap()).expect("session");
        assert!(authorized(&hs(&[(
            "cookie",
            &format!("a=1; mcc_session={cookie}; b=2")
        )])));
        assert!(authorized(&hs(&[(
            "cookie",
            &format!("mcc_session={cookie}")
        )])));
        // valeur forgée, session d'un autre moteur, absence de cookie
        assert!(!authorized(&hs(&[("cookie", "mcc_session=forgé")])));
        assert!(!authorized(&hs(&[("cookie", "autre=1")])));
        assert!(!authorized(&[]));
        // une nouvelle session invalide la précédente (usage unique du jeton)
        let frais = exchange(&mint_bootstrap()).expect("session 2");
        assert!(!authorized(&hs(&[(
            "cookie",
            &format!("mcc_session={cookie}")
        )])));
        assert!(authorized(&hs(&[(
            "cookie",
            &format!("mcc_session={frais}")
        )])));
    }

    #[test]
    fn csrf_header_exige_la_valeur_exacte() {
        assert!(has_csrf_header(&hs(&[("x-mcc-request", "1")])));
        assert!(has_csrf_header(&hs(&[("x-mcc-request", " 1 ")])));
        assert!(!has_csrf_header(&hs(&[("x-mcc-request", "0")])));
        assert!(!has_csrf_header(&hs(&[("x-mcc-request", "true")])));
        assert!(!has_csrf_header(&[]));
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
        reset_for_tests();
        let url = bootstrap_url("http://127.0.0.1:5090");
        assert!(url.starts_with("http://127.0.0.1:5090/#t="), "{url}");
        let token = url.rsplit("#t=").next().unwrap_or("").to_string();
        assert_eq!(token.len(), 43);
        // le fragment n'est jamais envoyé au serveur : le chemin seul reste /mcp
        // ou /api — ici on vérifie qu'aucun jeton ne fuit dans la query.
        assert!(!url.contains("?"));
        assert!(exchange(&token).is_some());
        // la barre oblique finale est absorbée (pas de double slash)
        let url2 = bootstrap_url("http://127.0.0.1:5090/");
        assert!(url2.contains(":5090/#t="), "{url2}");
    }
}
