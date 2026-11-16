//! Notifications : desktop (Windows) + webhook HTTP sur incident serveur.
//!
//! L'app surveille en continu ; quand un serveur passe de UP à DOWN (ou
//! PORT-ONLY), l'opérateur doit être prévenu — sans regarder l'écran.
//!
//! Deux canaux :
//! - **desktop** : toast Windows (PowerShell + BurntToast si présent, sinon
//!   fallback `msg`/notification) — sans dépendance externe obligatoire ;
//! - **webhook** : POST JSON vers une URL (Slack/Discord/n8n/own endpoint).
//!
//! Règles : un seul déclenchement par transition (pas de spam à chaque
//! cycle), et jamais d'erreur bloquante — une notification ratée ne doit
//! jamais faire tomber le poller.

use serde_json::{json, Value};
use std::net::IpAddr;

// ---------- identité de CETTE instance (audit 05 #4) ----------

/// Points d'écoute de cette instance : (port UI réel, IP locales de la machine).
/// Enregistré par run_headless au boot ; une URL de webhook qui vise l'un de
/// ces points est CETTE instance → refusée à l'enregistrement comme à l'envoi.
static SELF_LISTEN: std::sync::OnceLock<(u16, Vec<String>)> = std::sync::OnceLock::new();

/// Chemin du journal mcc.log, branché par run_headless : send_webhook n'a pas
/// d'AppState mais doit tracer chaque envoi (cible + résultat).
static WEBHOOK_LOG_PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Branche le chemin du journal (mcc.log) pour la journalisation des envois
/// webhook. Idempotent : le premier appel gagne (le boot ne s'exécute qu'une
/// fois par processus ; en tests unitaires sans boot, aucun fichier écrit).
pub fn set_webhook_log_path(path: &str) {
    let _ = WEBHOOK_LOG_PATH.set(path.to_string());
}

/// Enregistre le port UI réel (après repli +1..+9) et les IP locales de la
/// machine. Idempotent : le premier appel gagne (le boot ne s'exécute qu'une
/// fois par processus).
pub fn note_self_listen(ui_port: u16, local_ips: &[String], ui_bind: &str) {
    let mut hosts: Vec<String> = Vec::new();
    if !matches!(ui_bind.trim(), "" | "0.0.0.0" | "*" | "::") {
        hosts.push(ui_bind.trim().to_ascii_lowercase());
    }
    for ip in local_ips {
        hosts.push(ip.trim().to_ascii_lowercase());
    }
    let _ = SELF_LISTEN.set((ui_port, hosts));
}

/// Hôte d'URL normalisé (crochets IPv6 retirés, minuscules).
fn url_host_only(h: &str) -> String {
    let h = h.trim();
    let h = h.strip_prefix('[').unwrap_or(h);
    let h = h.split(']').next().unwrap_or(h);
    h.to_ascii_lowercase()
}

/// Cette URL (hôte:port) vise-t-elle cette instance elle-même ? Uniquement si
/// le port correspond au port UI réel, et que l'hôte est de boucle
/// (127.x, ::1), localhost, 0.0.0.0, ou une des IP locales de la machine.
fn is_own_instance(host: &str, port: u16) -> bool {
    let Some((self_port, hosts)) = SELF_LISTEN.get() else {
        return false;
    };
    if *self_port != port {
        return false;
    }
    let h = url_host_only(host);
    if h.is_empty() || h == "localhost" || h == "0.0.0.0" {
        return true;
    }
    if let Ok(ip) = h.parse::<IpAddr>() {
        if ip.is_loopback() {
            return true;
        }
    }
    hosts.contains(&h)
}

/// Configuration des notifications (persistée dans config.json).
#[derive(Clone, Debug)]
pub struct NotifyConfig {
    /// Active les notifications desktop.
    pub desktop: bool,
    /// URL du webhook (vide = désactivé).
    pub webhook_url: String,
    /// Méthode HTTP du webhook (POST par défaut).
    pub method: String,
    /// En-têtes additionnels (ex. Authorization) — format "K: V" par ligne.
    pub headers: String,
    /// Ne notifier que ces états (par défaut : DOWN et PORT-ONLY).
    pub states: Vec<String>,
    /// Délai minimum entre deux notifications du même serveur (secondes).
    pub min_interval_secs: u64,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        NotifyConfig {
            desktop: false,
            webhook_url: String::new(),
            method: "POST".to_string(),
            headers: String::new(),
            states: vec!["DOWN".into(), "PORT-ONLY".into()],
            min_interval_secs: 300,
        }
    }
}

impl NotifyConfig {
    /// L'état doit-il déclencher une notification ?
    pub fn wants(&self, state: &str) -> bool {
        self.states.iter().any(|s| s.eq_ignore_ascii_case(state))
    }

    /// En-têtes du webhook, parsés depuis "K: V" (une par ligne).
    pub fn header_pairs(&self) -> Vec<(String, String)> {
        self.headers
            .lines()
            .filter_map(|l| {
                let l = l.trim();
                if l.is_empty() {
                    return None;
                }
                let (k, v) = l.split_once(':')?;
                let k = k.trim();
                if k.is_empty() {
                    return None;
                }
                Some((k.to_string(), v.trim().to_string()))
            })
            .collect()
    }
}

/// Charge la config de notification depuis la section `notify` de la config.
pub fn from_json(v: &Value) -> NotifyConfig {
    let mut c = NotifyConfig::default();
    let Some(o) = v.as_object() else { return c };
    if let Some(b) = o.get("desktop").and_then(|x| x.as_bool()) {
        c.desktop = b;
    }
    if let Some(u) = o.get("webhook_url").and_then(|x| x.as_str()) {
        c.webhook_url = u.trim().to_string();
    }
    if let Some(m) = o.get("method").and_then(|x| x.as_str()) {
        let m = m.trim().to_uppercase();
        if m == "POST" || m == "PUT" {
            c.method = m;
        }
    }
    if let Some(h) = o.get("headers").and_then(|x| x.as_str()) {
        c.headers = h.to_string();
    }
    if let Some(arr) = o.get("states").and_then(|x| x.as_array()) {
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
    if let Some(i) = o.get("min_interval_secs").and_then(|x| x.as_u64()) {
        c.min_interval_secs = i.clamp(0, 86400);
    }
    c
}

/// Sérialise la config de notification pour L'EXPOSITION API (/api/state) :
/// les en-têtes ne sortent JAMAIS en clair (ils peuvent porter un
/// Authorization, lisible sans code en loopback — audit 05 #5) ; un booléen
/// `headers_set` suffit à l'UI (« champ vide = inchangé »).
pub fn to_api_json(c: &NotifyConfig) -> Value {
    json!({
        "desktop": c.desktop,
        "webhook_url": c.webhook_url,
        "method": c.method,
        "headers_set": !c.headers.trim().is_empty(),
        "states": c.states,
        "min_interval_secs": c.min_interval_secs,
    })
}

/// Sérialise la config de notification (PERSISTANCE config.json uniquement —
/// l'exposition API passe par `to_api_json`, qui masque les en-têtes).
pub fn to_json(c: &NotifyConfig) -> Value {
    json!({
        "desktop": c.desktop,
        "webhook_url": c.webhook_url,
        "method": c.method,
        "headers": c.headers,
        "states": c.states,
        "min_interval_secs": c.min_interval_secs,
    })
}

/// Corps du webhook (compatible Slack/Discord : un objet `text` suffit, et le
/// contenu est aussi lisible par un endpoint maison).
pub fn webhook_body(server: &str, state: &str, detail: &str) -> Value {
    json!({
        "text": format!("MCP — {server} est {state} : {detail}"),
        "server": server,
        "state": state,
        "detail": detail,
        "app": "mcp-command-center",
        "ts": crate::types::now_ts(),
    })
}

/// Raison PRÉCISE du refus d'une URL de webhook — None = acceptée. Appliquée
/// à l'enregistrement (set_notify → valid_webhook_url) comme à l'envoi
/// (send_webhook). Règles :
/// - https:// REFUSÉ : l'app n'embarque aucune pile TLS (Cargo.toml =
///   serde_json + litemcp) — l'ancien comportement acceptait l'URL puis
///   partait en HTTP brut sur le port 443, sans jamais aboutir (audit 05 #4) ;
/// - une URL qui vise cette instance elle-même est refusée (boucle sur soi) ;
/// - les AUTRES cibles locales ou privées restent AUTORISÉES : un webhook vers
///   un service du LAN est un usage normal.
pub fn webhook_url_reject_reason(url: &str) -> Option<String> {
    let u = url.trim();
    if u.is_empty() {
        return Some("URL vide".to_string());
    }
    if u.len() > 2000 {
        return Some("URL trop longue (max 2000 caractères)".to_string());
    }
    if u.starts_with("https://") {
        return Some("https:// non pris en charge : l'app n'embarque aucune pile TLS, l'envoi partait en HTTP brut sur le port 443 et n'aboutissait jamais — utilisez une URL http://".to_string());
    }
    if !u.starts_with("http://") {
        return Some("URL de webhook invalide (http:// attendu)".to_string());
    }
    match split_url(u) {
        Ok((host, port, _path)) => {
            if is_own_instance(&host, port) {
                Some(format!(
                    "cette URL vise cette instance elle-même (port UI {port}) : boucle interdite"
                ))
            } else {
                None
            }
        }
        Err(e) => Some(e),
    }
}

/// Valide une URL de webhook (http:// seul, longueur bornée, pas cette
/// instance). Le message PRÉCIS du refus est dans `webhook_url_reject_reason`.
pub fn valid_webhook_url(url: &str) -> bool {
    webhook_url_reject_reason(url).is_none()
}

/// Envoie le webhook (HTTP brut, sans dépendance : on réutilise le client
/// maison du projet). Retourne Ok(()) ou le message d'erreur.
pub fn send_webhook(
    cfg: &NotifyConfig,
    server: &str,
    state: &str,
    detail: &str,
) -> Result<(), String> {
    if cfg.webhook_url.trim().is_empty() {
        return Err("aucune URL de webhook configurée".to_string());
    }
    // mêmes règles qu'à l'enregistrement, avec le message précis — un ancien
    // https:// persisté dans config.json est refusé ici aussi
    if let Some(raison) = webhook_url_reject_reason(&cfg.webhook_url) {
        return Err(format!("webhook refusé : {raison}"));
    }
    let body = webhook_body(server, state, detail).to_string();
    let (host, port, path) = split_url(&cfg.webhook_url)?;
    // les paires d'en-têtes doivent VIVRE jusqu'à l'appel : on les garde
    // dans des variables (un Vec<&str> vers des temporaires ne compile pas).
    let pairs = cfg.header_pairs();
    let mut headers: Vec<(&str, &str)> = Vec::new();
    for (k, v) in &pairs {
        headers.push((k.as_str(), v.as_str()));
    }
    let cible = format!("http://{host}:{port}{path}");
    let resultat =
        crate::probe::http_post_raw(&host, port, &path, &body, &headers, &cfg.method, 8000);
    // JOURNAL (audit 05 #4) : chaque envoi trace sa cible et son résultat —
    // le webhook embarque des données d'incident vers l'extérieur.
    let (ok, outcome) = match &resultat {
        Ok(()) => (true, "ok".to_string()),
        Err(e) => (false, format!("erreur : {e}")),
    };
    append_webhook_log(&webhook_log_line(
        server, state, detail, &cible, ok, &outcome,
    ));
    resultat.map(|_| ()).map_err(|e| format!("webhook : {e}"))
}

/// Ligne de journal d'un envoi de webhook (pure, testable) — format aligné sur
/// le journal disque : "[ISO] [niveau] message".
fn webhook_log_line(
    server: &str,
    state: &str,
    detail: &str,
    cible: &str,
    ok: bool,
    outcome: &str,
) -> String {
    format!(
        "[{}] [{}] [webhook] envoi vers {} — serveur « {} » → {} ({}) : {}",
        crate::types::iso_utc(crate::types::now_ts()),
        if ok { "info" } else { "warn" },
        cible,
        server,
        state,
        detail,
        outcome
    )
}

/// Écrit la ligne dans mcc.log si le chemin a été branché au boot (tests
/// unitaires sans boot : aucun fichier écrit). Volume négligeable — la
/// rotation 512 Ko du journal reste assurée par `write_log_file`.
fn append_webhook_log(line: &str) {
    if let Some(path) = WEBHOOK_LOG_PATH.get() {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            use std::io::Write;
            let _ = writeln!(f, "{line}");
        }
    }
}

/// Découpe une URL en (hôte, port, chemin) — le client maison n'accepte pas
/// les URL complètes.
fn split_url(url: &str) -> Result<(String, u16, String), String> {
    let u = url.trim();
    let rest = if let Some(r) = u.strip_prefix("https://") {
        r
    } else if let Some(r) = u.strip_prefix("http://") {
        r
    } else {
        return Err("URL de webhook invalide (http:// ou https:// attendu)".to_string());
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let authority = authority.split('@').next_back().unwrap_or(authority);
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => (
            h.to_string(),
            p.parse::<u16>().map_err(|_| "port invalide".to_string())?,
        ),
        _ => (
            authority.to_string(),
            if u.starts_with("https://") { 443 } else { 80 },
        ),
    };
    if host.is_empty() {
        return Err("hôte vide dans l'URL".to_string());
    }
    Ok((host, port, path.to_string()))
}

/// Script PowerShell FIXE du toast : aucune donnée n'y est concaténée — le
/// titre et le message arrivent par les variables d'environnement
/// ($env:MCC_TOAST_TITLE / $env:MCC_TOAST_TEXT). L'ancien code passait
/// `-Command` trois fois (PowerShell échouait toujours : canal desktop mort,
/// audit 05 #7) et assemblait le script par concaténation : une apostrophe
/// dans un label de serveur injectait du PowerShell.
const TOAST_SCRIPT: &str = "\
[void][System.Reflection.Assembly]::LoadWithPartialName('System.Windows.Forms');
[void][System.Reflection.Assembly]::LoadWithPartialName('System.Drawing');
$n = New-Object System.Windows.Forms.NotifyIcon;
$n.Icon = [System.Drawing.SystemIcons]::Warning;
$n.BalloonTipTitle = $env:MCC_TOAST_TITLE;
$n.BalloonTipText = $env:MCC_TOAST_TEXT;
$n.Visible = $true;
Start-Sleep -Milliseconds 50;
$n.ShowBalloonTip(8000);
Start-Sleep -Seconds 9;
$n.Dispose()";

/// Ce que dit la notification d'une **demande de confirmation** (lot M3,
/// `docs/auth.md` §5) : libellé et empreinte courte du client, adresse source,
/// outil et effet exact. Rendu pur (titre, texte) pour être testable sans
/// afficher de toast — et pour prouver qu'aucun secret n'y entre.
pub fn approval_pending_message(
    label: &str,
    fingerprint: &str,
    address: &str,
    tool: &str,
    effect: &str,
) -> (String, String) {
    let qui = if label.trim().is_empty() {
        fingerprint.to_string()
    } else {
        format!("{} ({})", label.trim(), fingerprint)
    };
    let effet: String = if effect.chars().count() > 200 {
        effect.chars().take(200).collect::<String>() + "…"
    } else {
        effect.to_string()
    };
    (
        "MCP Control — confirmation demandée".to_string(),
        format!("{qui} depuis {address} — {tool} : {effet} — ouvrez Accès pour approuver"),
    )
}

/// Toast desktop Windows. Silencieux si indisponible (le poller ne doit pas
/// tomber). Titre et message transmis PAR L'ENVIRONNEMENT, jamais dans le
/// script : apostrophes et guillemets sont inertes (aucune concaténation).
pub fn desktop_toast(title: &str, message: &str) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("notifications desktop : non supporté hors Windows".to_string());
    }
    let mut c = std::process::Command::new("powershell");
    // UN SEUL -Command, script constant.
    c.args(["-NoProfile", "-NonInteractive", "-Command", TOAST_SCRIPT]);
    c.env("MCC_TOAST_TITLE", title);
    c.env("MCC_TOAST_TEXT", message);
    let out = c.output().map_err(|e| format!("toast desktop : {e}"))?;
    if !out.status.success() {
        return Err("toast desktop : PowerShell a échoué".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_de_demande_de_confirmation_sans_secret() {
        let (titre, texte) = approval_pending_message(
            "atelier",
            "ed25519:3f9a1c07b2d84e51",
            "192.168.40.7",
            "start_server",
            "node srv.js",
        );
        assert!(titre.contains("confirmation"));
        for attendu in [
            "atelier",
            "3f9a1c07b2d84e51",
            "192.168.40.7",
            "start_server",
            "node srv.js",
        ] {
            assert!(texte.contains(attendu), "{texte}");
        }
        // libellé vide : l'empreinte seule identifie le client
        let (_, texte) = approval_pending_message("", "ed25519:abc", "127.0.0.1", "x", "y");
        assert!(texte.starts_with("ed25519:abc "), "{texte}");
        // effet très long : tronqué, jamais un toast de 10 000 caractères
        let (_, texte) = approval_pending_message("a", "b", "c", "d", &"x".repeat(5_000));
        assert!(texte.chars().count() < 400, "{}", texte.chars().count());
    }

    #[test]
    fn config_par_defaut_sans_webhook() {
        let c = NotifyConfig::default();
        assert!(!c.desktop);
        assert!(c.webhook_url.is_empty());
        assert!(c.wants("DOWN"));
        assert!(c.wants("PORT-ONLY"));
        assert!(!c.wants("UP"));
        assert!(!c.wants("DISABLED"));
    }

    #[test]
    fn lecture_json_respecte_les_champs() {
        let c = from_json(&json!({
            "desktop": true,
            "webhook_url": "https://example.com/hook",
            "method": "put",
            "headers": "Authorization: Bearer abc\nX-Test: 1",
            "states": ["down"],
            "min_interval_secs": 60
        }));
        assert!(c.desktop);
        assert_eq!(c.webhook_url, "https://example.com/hook");
        assert_eq!(c.method, "PUT", "méthode normalisée en majuscules");
        assert_eq!(c.states, vec!["DOWN"]);
        assert_eq!(c.min_interval_secs, 60);
        let pairs = c.header_pairs();
        assert_eq!(pairs.len(), 2);
        assert_eq!(
            pairs[0],
            ("Authorization".to_string(), "Bearer abc".to_string())
        );
        assert_eq!(pairs[1], ("X-Test".to_string(), "1".to_string()));
    }

    #[test]
    fn json_invalide_retombe_sur_les_defauts() {
        let c = from_json(&json!("pas un objet"));
        assert!(!c.desktop);
        assert!(c.webhook_url.is_empty());
        // une méthode non autorisée est ignorée
        let c2 = from_json(&json!({"method": "DELETE"}));
        assert_eq!(c2.method, "POST");
    }

    #[test]
    fn url_webhook_https_refusee_message_tls() {
        // audit 05 #4 : https:// partait en HTTP brut sur le port 443 (aucune
        // pile TLS) — refusé, avec un message qui explique pourquoi
        let r = webhook_url_reject_reason("https://example.com/x").unwrap();
        assert!(r.contains("TLS"), "message TLS attendu, obtenu : {r}");
        assert!(!valid_webhook_url("https://example.com/x"));
        assert!(webhook_url_reject_reason("ftp://example.com").is_some());
        assert!(webhook_url_reject_reason("example.com").is_some());
        assert!(webhook_url_reject_reason("").is_some());
        // les autres cibles locales ou privées restent un usage NORMAL
        assert!(valid_webhook_url("http://192.168.1.5:8080/hook"));
        assert!(valid_webhook_url("http://10.0.0.7/hook"));
        // un service local SUR UN AUTRE PORT que le nôtre : usage normal aussi
        assert!(webhook_url_reject_reason("http://127.0.0.1:57596/hook").is_none());
    }

    #[test]
    fn url_webhook_vise_soi_meme_refusee() {
        // canon du processus de tests (port fixe, TEST-NET) : idempotent,
        // le premier appel gagne — tous les tests unitaires partagent la valeur
        note_self_listen(
            57595,
            &["203.0.113.5".to_string(), "198.51.100.5".to_string()],
            "0.0.0.0",
        );
        for h in ["127.0.0.1", "localhost", "[::1]", "203.0.113.5", "0.0.0.0"] {
            let url = format!("http://{h}:57595/api/state");
            let r = webhook_url_reject_reason(&url);
            assert!(
                r.as_deref().is_some_and(|m| m.contains("instance")),
                "auto-instance refusée pour {h}, obtenu : {r:?}"
            );
        }
        // une IP non locale sur NOTRE port n'est pas cette instance
        assert!(webhook_url_reject_reason("http://192.0.2.10:57595/hook").is_none());
        // sans enregistrement boot (autre processus), pas de détection
    }

    #[test]
    fn decoupe_url() {
        let (h, p, path) = split_url("https://example.com/a/b").unwrap();
        assert_eq!(h, "example.com");
        assert_eq!(p, 443);
        assert_eq!(path, "/a/b");
        let (h, p, path) = split_url("http://192.168.1.5:8080").unwrap();
        assert_eq!(h, "192.168.1.5");
        assert_eq!(p, 8080);
        assert_eq!(path, "/");
        assert!(split_url("pas-url").is_err());
    }

    #[test]
    fn corps_webhook_lisible() {
        let b = webhook_body("mon-api", "DOWN", "connexion refusée");
        assert_eq!(b["server"], "mon-api");
        assert_eq!(b["state"], "DOWN");
        assert!(b["text"].as_str().unwrap().contains("mon-api"));
        assert!(b["text"].as_str().unwrap().contains("DOWN"));
    }

    #[test]
    fn webhook_vide_ou_invalide_refuse() {
        let c = NotifyConfig::default();
        assert!(send_webhook(&c, "x", "DOWN", "y").is_err());
        let c2 = NotifyConfig {
            webhook_url: "ftp://bad".into(),
            ..NotifyConfig::default()
        };
        assert!(send_webhook(&c2, "x", "DOWN", "y").is_err());
        // un ancien https:// persisté avant le correctif est refusé À L'ENVOI
        // aussi, avec le message TLS clair (aucun envoi HTTP brut sur 443)
        let c3 = NotifyConfig {
            webhook_url: "https://example.com/hook".into(),
            ..NotifyConfig::default()
        };
        let e = send_webhook(&c3, "x", "DOWN", "y").unwrap_err();
        assert!(
            e.contains("TLS"),
            "message TLS attendu à l'envoi, obtenu : {e}"
        );
    }

    #[test]
    fn exposition_api_masque_les_entetes() {
        // audit 05 #5 : /api/state ne doit plus transporter les en-têtes du
        // webhook (un Authorization y vivait en clair, lisible sans code)
        let c = NotifyConfig {
            headers: "Authorization: Bearer jeton-secret-999".into(),
            ..NotifyConfig::default()
        };
        let v = to_api_json(&c);
        assert_eq!(v["headers_set"], json!(true));
        assert!(v.get("headers").is_none(), "headers en clair interdit");
        let s = v.to_string();
        assert!(
            !s.contains("jeton-secret-999"),
            "aucune valeur d'en-tête ne sort"
        );
        // en-têtes absents ou blancs → headers_set false
        let blanc = NotifyConfig {
            headers: "   ".into(),
            ..NotifyConfig::default()
        };
        assert_eq!(to_api_json(&blanc)["headers_set"], json!(false));
        let vide = NotifyConfig {
            headers: String::new(),
            ..NotifyConfig::default()
        };
        assert_eq!(to_api_json(&vide)["headers_set"], json!(false));
        // la persistance (to_json) garde la valeur — seul /api/state la masque
        assert_eq!(to_json(&vide)["headers"], json!(String::new()));
        let c2 = NotifyConfig {
            headers: "X-Jeton: abc".into(),
            ..NotifyConfig::default()
        };
        assert_eq!(to_json(&c2)["headers"], json!("X-Jeton: abc"));
    }

    #[test]
    fn ligne_de_journal_webhook_cible_et_resultat() {
        let ok = webhook_log_line(
            "mon-api",
            "DOWN",
            "connexion refusée",
            "http://192.168.1.5:8080/hook",
            true,
            "ok",
        );
        assert!(ok.contains("[info] [webhook] envoi vers http://192.168.1.5:8080/hook"));
        assert!(ok.contains("serveur « mon-api » → DOWN"));
        assert!(ok.contains("(connexion refusée) : ok"));
        let ko = webhook_log_line(
            "srv",
            "PORT-ONLY",
            "détail",
            "http://h:1/p",
            false,
            "erreur : HTTP 500",
        );
        assert!(ko.contains("[warn] [webhook] envoi vers http://h:1/p"));
        assert!(ko.contains("erreur : HTTP 500"));
    }

    #[test]
    #[cfg(windows)]
    fn toast_desktop_titre_et_message_hostiles_via_env() {
        // le titre/message passent par l'environnement, jamais dans le script :
        // une apostrophe ou un guillemet ne peut plus casser ni injecter du
        // PowerShell. Réel : PowerShell doit sortir 0.
        let r = desktop_toast(
            "l'apostrophe \"les guillemets\" 'tous'",
            "message d'essai avec ' et \" et $(calc) et ${x}",
        );
        assert!(r.is_ok(), "toast desktop doit réussir, obtenu : {r:?}");
    }
}
