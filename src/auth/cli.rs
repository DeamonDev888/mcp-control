//! Ligne de commande `mcp-control auth ...` (lot M1, point 5).
//!
//! - `auth pair <url>` : génère la paire du client, envoie la clé publique,
//!   affiche le CODE DE COMPARAISON, attend l'approbation locale, puis range la
//!   clé privée au trousseau (repli fichier) et épingle l'identité du serveur.
//! - `auth list` : clients du moteur local (`client_list`, droit `admin`).
//! - `auth revoke <keyid>` : révocation immédiate (`client_revoke`).
//! - `auth fingerprint` : empreintes de l'identité du serveur, lues dans
//!   `config.json`. **Aucune clé privée n'est jamais exportée** (§9).
//!
//! Aucune dépendance : client HTTP minimal sur `std::net` (l'écoute est en
//! loopback ou en LAN HTTP jusqu'au lot M4 ; `https://` est refusé).

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::auth::pair::{comparison_code, PAIR_TTL_SECS};
use crate::auth::secret;

pub const USAGE: &str = "mcp-control auth <commande>\n\
  pair <url>         appairer ce poste avec un moteur MCP Control (http://127.0.0.1:port ou https://hôte:port)\n\
  pending            lister les demandes d'appairage en attente sur le moteur local\n\
  approve <pair_id> [droits...]  approuver une demande (défaut : lecture)\n\
  reject <pair_id>   refuser une demande d'appairage en attente\n\
  state              afficher un état JSON assaini du moteur local\n\
  list               clients appairés du moteur local\n\
  revoke <keyid>     révoquer un client (effet à la requête suivante)\n\
  fingerprint        empreintes de l'identité du serveur local\n\
  verify-log [--path <fichier>]  vérifier la chaîne du journal d'audit (auth-audit.log)";

/// Analyse une URL `http://hôte[:port][/]` ou `https://hôte[:port][/]`.
/// Renvoie `(is_https, host, port)`. Sans la feature `tls`, `https://` est refusé
/// avec un motif explicite.
pub fn parse_url_scheme(url: &str) -> Result<(bool, String, u16), String> {
    let url = url.trim();
    let (is_https, reste, def_port) = if let Some(r) = url.strip_prefix("http://") {
        (false, r, 80u16)
    } else if let Some(r) = url.strip_prefix("https://") {
        if !crate::auth::net::tls_compiled() {
            return Err(
                "https:// non pris en charge : binaire compilé sans la feature `tls`".to_string(),
            );
        }
        (true, r, 443u16)
    } else {
        return Err(format!(
            "URL attendue en http://hôte:port ou https://hôte:port — reçu « {url} »"
        ));
    };
    let reste = reste.trim_end_matches('/');
    if reste.contains('/') || reste.is_empty() {
        return Err(format!(
            "URL attendue en http(s)://hôte:port sans chemin — reçu « {url} »"
        ));
    }
    let (host, port) = if let Some(bracketed) = reste.strip_prefix('[') {
        match bracketed.split_once(']') {
            Some((h, "")) => (h, def_port),
            Some((h, after)) => match after.strip_prefix(':') {
                Some(p) => (
                    h,
                    p.parse::<u16>().map_err(|_| "port invalide".to_string())?,
                ),
                None => return Err("hôte IPv6 invalide".to_string()),
            },
            None => return Err("crochet IPv6 non fermé".to_string()),
        }
    } else {
        match reste.rsplit_once(':') {
            Some((h, p)) => (
                h,
                p.parse::<u16>().map_err(|_| "port invalide".to_string())?,
            ),
            None => (reste, def_port),
        }
    };
    if host.is_empty() {
        return Err("hôte vide".to_string());
    }
    Ok((is_https, host.to_string(), port))
}

/// Squelette d'URL HTTP locale (`http://hôte[:port][/]`), utilisé par le pont
/// stdio (`bridge.rs`).
pub fn parse_url(url: &str) -> Result<(String, u16), String> {
    let url = url.trim();
    if url.starts_with("https://") {
        return Err(
            "https:// non pris en charge sur cette commande HTTP locale (lot M4 : utiliser auth pair pour TLS)"
                .to_string(),
        );
    }
    let (_, host, port) = parse_url_scheme(url)?;
    Ok((host, port))
}

/// Préfixe lisible du `keyid` : 1 à 8 caractères `[a-z0-9]` tirés du nom de la
/// machine (grammaire du §3.1). Repli : `cli`.
fn keyid_prefix(nom: &str) -> String {
    let mut out: String = nom
        .chars()
        .filter_map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_alphanumeric() {
                Some(c)
            } else {
                None
            }
        })
        .take(8)
        .collect();
    if out.is_empty() {
        out.push_str("cli");
    }
    out
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "cli".to_string())
}

/// Requête HTTP minimale avec en-têtes additionnels (une connexion par appel,
/// `Connection: close`). Renvoie `(statut, en_tetes_bruts, corps)`.
fn http_with_headers(
    host: &str,
    port: u16,
    method: &str,
    path: &str,
    extra_headers: &[(&str, &str)],
    body: Option<&str>,
) -> Result<(u16, String, String), String> {
    let addr = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("résolution {host}:{port} impossible : {e}"))?
        .next()
        .ok_or_else(|| format!("aucune adresse pour {host}:{port}"))?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .map_err(|e| format!("connexion {host}:{port} impossible : {e}"))?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(15)));
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\nAccept: application/json\r\n"
    );
    for (name, value) in extra_headers {
        req.push_str(&format!("{name}: {value}\r\n"));
    }
    if let Some(b) = body {
        req.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            b.len()
        ));
    }
    req.push_str("\r\n");
    if let Some(b) = body {
        req.push_str(b);
    }
    s.write_all(req.as_bytes())
        .map_err(|e| format!("envoi impossible : {e}"))?;
    let mut data = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        match s.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => data.extend_from_slice(&tmp[..n]),
            Err(e) => return Err(format!("lecture impossible : {e}")),
        }
        if data.len() > 4 * 1024 * 1024 {
            break;
        }
    }
    let texte = String::from_utf8_lossy(&data).to_string();
    let (head, corps) = texte.split_once("\r\n\r\n").unwrap_or((&texte, ""));
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| "réponse HTTP illisible".to_string())?;
    Ok((status, head.to_string(), corps.to_string()))
}

/// Requête HTTP minimale (une connexion par appel, `Connection: close`).
fn http(
    host: &str,
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> Result<(u16, String), String> {
    let (status, _, corps) = http_with_headers(host, port, method, path, &[], body)?;
    Ok((status, corps))
}

fn http_json(
    host: &str,
    port: u16,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<(u16, Value), String> {
    let texte = body.map(|b| b.to_string());
    let (status, corps) = http(host, port, method, path, texte.as_deref())?;
    let v: Value = serde_json::from_str(&corps).unwrap_or(Value::Null);
    Ok((status, v))
}

/// Requête HTTP ou HTTPS (TLS 1.3) pour le protocole d'appairage (`auth pair`).
/// Renvoie `(statut_http, corps_json, empreinte_tls_observee)`.
fn pair_http_json(
    is_https: bool,
    host: &str,
    port: u16,
    method: &str,
    path: &str,
    body: Option<&Value>,
    expected_tls_fp: Option<&str>,
) -> Result<(u16, Value, String), String> {
    if !is_https {
        let (status, v) = http_json(host, port, method, path, body)?;
        return Ok((status, v, String::new()));
    }
    let texte = body.map(|b| b.to_string());
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\nAccept: application/json\r\n"
    );
    if let Some(ref b) = texte {
        req.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            b.len()
        ));
    }
    req.push_str("\r\n");
    if let Some(ref b) = texte {
        req.push_str(b);
    }
    let (data, observed_fp) = crate::auth::net::https_exchange_raw(
        host,
        port,
        req.as_bytes(),
        Duration::from_secs(15),
        expected_tls_fp,
        4 * 1024 * 1024,
    )?;
    let texte_rep = String::from_utf8_lossy(&data).to_string();
    let (head, corps) = texte_rep.split_once("\r\n\r\n").unwrap_or((&texte_rep, ""));
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| "réponse HTTPS illisible".to_string())?;
    let v: Value = serde_json::from_str(corps).unwrap_or(Value::Null);
    Ok((status, v, observed_fp))
}

/// Chemin de `config.json` de CE poste (même résolution que le moteur).
fn config_path() -> String {
    crate::manage::resolve_config_path(env!("CARGO_MANIFEST_DIR"), std::env::var("MCC_CONFIG").ok())
}

fn config_dir() -> PathBuf {
    Path::new(&config_path())
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Port UI du moteur local, lu dans `config.json` (aucune supposition).
fn local_port() -> Result<u16, String> {
    let texte = std::fs::read_to_string(config_path())
        .map_err(|e| format!("config.json illisible ({}) — lance d'abord MCP Control", e))?;
    let v: Value =
        serde_json::from_str(&texte).map_err(|e| format!("config.json invalide : {e}"))?;
    v.get("settings")
        .and_then(|s| s.get("ui_port"))
        .and_then(|p| p.as_u64())
        .map(|p| p as u16)
        .ok_or_else(|| "ui_port absent de config.json".to_string())
}

/// Extrait la valeur du cookie `mcc_session` des en-têtes `Set-Cookie`.
fn extract_session_cookie(head: &str) -> Option<String> {
    for line in head.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("set-cookie") {
            continue;
        }
        let first = value.trim().split(';').next().unwrap_or("");
        if let Some((cname, cval)) = first.split_once('=') {
            if cname.trim() == crate::auth::session::COOKIE_NAME {
                let cval = cval.trim();
                if !cval.is_empty() {
                    return Some(cval.to_string());
                }
            }
        }
    }
    None
}

/// Ouvre une session locale S1 via le canal fichier utilisateur puis l'échange
/// sur `POST /api/session` (R-5). Aucun jeton, cookie ni secret n'est affiché.
fn local_session(port: u16) -> Result<(String, String), String> {
    let token = crate::auth::session::request_bootstrap(&config_path(), Duration::from_secs(5))?;
    let body = json!({ "token": token }).to_string();
    let (status, head, corps) =
        http_with_headers("127.0.0.1", port, "POST", "/api/session", &[], Some(&body))?;
    if status != 200 {
        return Err(format!(
            "ouverture de session locale refusée (HTTP {status})"
        ));
    }
    let cookie = extract_session_cookie(&head)
        .ok_or_else(|| "cookie de session absent de la réponse /api/session".to_string())?;
    let v: Value = serde_json::from_str(&corps).unwrap_or(Value::Null);
    let secret = v
        .get("secret")
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "secret de session absent de la réponse /api/session".to_string())?
        .to_string();
    Ok((cookie, secret))
}

fn action(v: Value) -> Result<Value, String> {
    let port = local_port()?;
    let (cookie, secret) = local_session(port)?;
    let cookie_hdr = format!("{}={cookie}", crate::auth::session::COOKIE_NAME);
    let body = v.to_string();
    let (status, _, corps) = http_with_headers(
        "127.0.0.1",
        port,
        "POST",
        "/api/action",
        &[("Cookie", &cookie_hdr), ("X-MCC-Session", &secret)],
        Some(&body),
    )?;
    if status != 200 {
        return Err(format!("le moteur a répondu {status}"));
    }
    let reponse: Value = serde_json::from_str(&corps).unwrap_or(Value::Null);
    if reponse.get("ok").and_then(|o| o.as_bool()) != Some(true) {
        return Err(reponse
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("requête refusée par le moteur")
            .to_string());
    }
    Ok(reponse)
}

/// `auth pair <url>` — les cinq messages du §3.4, côté client.
fn cmd_pair(url: &str) -> i32 {
    let (is_https, host, port) = match parse_url_scheme(url) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("auth pair : {e}");
            return 2;
        }
    };
    if !is_https && !crate::is_loopback_bind(&host) {
        eprintln!(
            "auth pair : connexion HTTP en clair hors loopback refusée (DA5 : utiliser https://{host}:{port})"
        );
        return 1;
    }
    let cfg_dir = config_dir();
    // Si un épinglage existe déjà pour ce serveur avec TLS, tout repli HTTP ou
    // changement de certificat dès la connexion est refusé (§3.4 et §7).
    let pre_pinned_tls = crate::auth::net::find_existing_pin(&cfg_dir, &host, port)
        .and_then(|v| {
            v.get("tls_fp")
                .and_then(|t| t.as_str())
                .map(|s| s.trim().to_string())
        })
        .filter(|s| !s.is_empty());
    if !is_https && pre_pinned_tls.is_some() {
        eprintln!(
            "auth pair : le serveur {host}:{port} avait une empreinte TLS épinglée — repli HTTP en clair refusé"
        );
        return 1;
    }

    let cle = litemcp::auth::sign::SigningKey::generate();
    let cle_pub = crate::crypto::hex(&cle.verifying_key().to_bytes());
    let Some(suffix) = crate::random_hex(1) else {
        eprintln!("auth pair : entropie OS indisponible — abandon");
        return 1;
    };
    let keyid = format!("mcc-{}-{}", keyid_prefix(&hostname()), suffix);
    let corps = json!({
        "keyid": keyid,
        "label": hostname().chars().take(64).collect::<String>(),
        "public_key": cle_pub,
        "created": crate::types::now_ts(),
    });
    let (status, reponse, observed_tls_fp) = match pair_http_json(
        is_https,
        &host,
        port,
        "POST",
        "/auth/pair",
        Some(&corps),
        pre_pinned_tls.as_deref(),
    ) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("auth pair : {e}");
            return 1;
        }
    };
    if status != 200 {
        let motif = reponse
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("demande refusée");
        eprintln!("auth pair : {motif} (HTTP {status})");
        return 1;
    }
    if reponse.get("api").and_then(|a| a.as_str()) != Some(crate::auth::pair::PAIR_API) {
        eprintln!("auth pair : le serveur ne parle pas le protocole attendu — abandon");
        return 1;
    }
    let pair_id = reponse
        .get("pair_id")
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string();
    let server_pub = reponse
        .get("server_pub")
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string();
    let tls_fp = reponse
        .get("tls_fp")
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string();
    if pair_id.len() != 32 || crate::auth::registry::decode_public_key(&server_pub).is_none() {
        eprintln!("auth pair : réponse d'appairage incomplète — abandon");
        return 1;
    }
    // §3.4 message 2 : comparer `tls_fp` à l'empreinte observée pendant la
    // poignée de main TLS ; différentes → abandon explicite.
    if is_https {
        if tls_fp.is_empty() || !crate::auth::net::valid_tls_fp(&tls_fp) {
            eprintln!(
                "auth pair : le serveur TLS n'a pas annoncé d'empreinte tls_fp valide — abandon"
            );
            return 1;
        }
        if !litemcp::auth::PinnedServer::new(&observed_tls_fp)
            .is_ok_and(|p| p.verify(&tls_fp).is_ok())
        {
            eprintln!(
                "auth pair : l'empreinte TLS annoncée ({tls_fp}) diffère du certificat observé ({observed_tls_fp}) — certificat substitué ou MITM, abandon"
            );
            return 1;
        }
    } else if !tls_fp.is_empty() {
        eprintln!(
            "auth pair : le serveur annonce une empreinte TLS ({tls_fp}) sur une connexion HTTP en clair — utiliser https://{host}:{port}"
        );
        return 1;
    }
    // §7 : refuser un serveur dont l'identité Ed25519 ou le certificat TLS a
    // changé par rapport à un épinglage existant.
    if let Err(e) =
        crate::auth::net::verify_no_identity_change(&cfg_dir, &host, port, &server_pub, &tls_fp)
    {
        eprintln!("auth pair : {e}");
        return 1;
    }
    let code = comparison_code(&pair_id, &cle_pub, &server_pub, &tls_fp);
    println!("Demande d'appairage envoyée à {host}:{port}");
    println!("  keyid client : {keyid}");
    println!(
        "  empreinte    : {}",
        crate::auth::registry::fingerprint_of(&cle_pub).unwrap_or_default()
    );
    if !tls_fp.is_empty() {
        println!("  certificat   : {tls_fp}");
    }
    println!();
    println!("  CODE DE COMPARAISON : {code}");
    println!();
    println!("Vérifie que ce code est IDENTIQUE dans MCP Control (fenêtre locale, écran");
    println!("« Accès »), puis approuve la demande avec les droits voulus.");
    println!("Attente de l'approbation ({} s au plus)...", PAIR_TTL_SECS);

    let expected_poll_tls = if is_https {
        Some(tls_fp.as_str())
    } else {
        None
    };
    let limite = Instant::now() + Duration::from_secs(PAIR_TTL_SECS + 10);
    let droits: Vec<String> = loop {
        if Instant::now() >= limite {
            eprintln!("auth pair : demande expirée sans décision — clé locale effacée, rien n'est appairé");
            return 1;
        }
        std::thread::sleep(Duration::from_secs(2));
        let (status, v, _) = match pair_http_json(
            is_https,
            &host,
            port,
            "GET",
            &format!("/auth/pair/{pair_id}"),
            None,
            expected_poll_tls,
        ) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("auth pair : {e}");
                return 1;
            }
        };
        if status == 404 {
            eprintln!("auth pair : demande inconnue du serveur (expirée ou redémarrage) — abandon");
            return 1;
        }
        if status != 200 {
            eprintln!("auth pair : le serveur a répondu {status} — abandon");
            return 1;
        }
        // L'identité et l'empreinte TLS du serveur ne doivent pas changer en cours d'appairage.
        if v.get("server_pub").and_then(|p| p.as_str()) != Some(server_pub.as_str()) {
            eprintln!("auth pair : l'identité du serveur a CHANGÉ pendant l'appairage — abandon");
            return 1;
        }
        if v.get("tls_fp").and_then(|p| p.as_str()).unwrap_or("") != tls_fp {
            eprintln!(
                "auth pair : l'empreinte TLS du serveur a CHANGÉ pendant l'appairage — abandon"
            );
            return 1;
        }
        match v.get("status").and_then(|s| s.as_str()).unwrap_or("") {
            "pending" => {}
            "approved" => {
                break v
                    .get("rights")
                    .and_then(|r| r.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str())
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();
            }
            "rejected" => {
                eprintln!("auth pair : demande REFUSÉE localement — aucune clé conservée");
                return 1;
            }
            "expired" => {
                eprintln!(
                    "auth pair : demande EXPIRÉE ({} s) — aucune clé conservée",
                    PAIR_TTL_SECS
                );
                return 1;
            }
            autre => {
                eprintln!("auth pair : état inattendu « {autre} » — abandon");
                return 1;
            }
        }
    };

    // Approbation obtenue : on range la clé privée et on épingle le serveur.
    let compte = format!("client-{host}:{port}");
    let bloc = cle.export();
    let mut range = secret::SecretStore::File;
    if let Some(svc) = secret::service() {
        if secret::store_client_key(&svc, &compte, &bloc).is_ok() {
            range = secret::SecretStore::Keyring;
        }
    }
    if range == secret::SecretStore::File {
        let chemin = cfg_dir
            .join("auth")
            .join("clients")
            .join(format!("{keyid}.key"));
        if let Err(e) = secret::write_secret_file(&chemin, &bloc) {
            eprintln!("auth pair : clé approuvée mais non enregistrée ({e}) — ré-appaire");
            return 1;
        }
    }
    let scheme = if is_https { "https" } else { "http" };
    let pin = json!({
        "url": format!("{scheme}://{host}:{port}"),
        "host": host,
        "port": port,
        "server_keyid": reponse.get("server_keyid").cloned().unwrap_or(Value::Null),
        "server_pub": server_pub,
        "tls_fp": tls_fp,
        "keyid": keyid,
        "paired_at": crate::types::iso_utc(crate::types::now_ts()),
    });
    let pin_path = cfg_dir
        .join("auth")
        .join("clients")
        .join(format!("{keyid}.pin.json"));
    if let Some(parent) = pin_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = secret::write_secret_file(
        &pin_path,
        &serde_json::to_string_pretty(&pin).unwrap_or_default(),
    ) {
        eprintln!("auth pair : identité du serveur non épinglée ({e})");
        return 1;
    }
    println!();
    println!(
        "Appairé. keyid = {keyid} — droits : {}",
        if droits.is_empty() {
            "(aucun : clé inerte)".to_string()
        } else {
            droits.join(", ")
        }
    );
    println!("Clé privée : {}", range.as_str());
    println!("Serveur épinglé : {} ({})", server_pub, pin_path.display());
    0
}

fn cmd_pending() -> i32 {
    match action(json!({ "action": "pair_list" })) {
        Ok(v) => {
            let pending = v
                .get("pending")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();
            if pending.is_empty() {
                println!("Aucune demande d'appairage en attente.");
                return 0;
            }
            println!(
                "{:<34} {:<18} {:<26} {:<8} SOURCE",
                "PAIR_ID", "LIBELLÉ", "KEYID", "CODE"
            );
            for p in pending {
                println!(
                    "{:<34} {:<18} {:<26} {:<8} {}",
                    p.get("pair_id").and_then(|x| x.as_str()).unwrap_or(""),
                    p.get("label").and_then(|x| x.as_str()).unwrap_or(""),
                    p.get("keyid").and_then(|x| x.as_str()).unwrap_or(""),
                    p.get("code").and_then(|x| x.as_str()).unwrap_or(""),
                    p.get("paired_from").and_then(|x| x.as_str()).unwrap_or(""),
                );
            }
            0
        }
        Err(e) => {
            eprintln!("auth pending : {e}");
            1
        }
    }
}

pub(crate) fn parse_approve_rights(args: &[String]) -> Result<Vec<String>, String> {
    if args.is_empty() {
        return Ok(vec![crate::auth::RIGHT_LECTURE.to_string()]);
    }
    let mut out = Vec::new();
    for raw in args {
        let r = raw.trim();
        if !crate::auth::registry::is_grantable(r) {
            return Err(format!(
                "droit refusé « {r} » (attendu : lecture, pilotage ou execution)"
            ));
        }
        if !out.iter().any(|x| x == r) {
            out.push(r.to_string());
        }
    }
    Ok(out)
}

fn cmd_approve(pair_id: &str, right_args: &[String]) -> i32 {
    let pair_id = pair_id.trim();
    if pair_id.is_empty() {
        eprintln!("auth approve : pair_id manquant");
        return 2;
    }
    let rights = match parse_approve_rights(right_args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("auth approve : {e}");
            return 2;
        }
    };
    match action(json!({
        "action": "pair_approve",
        "pair_id": pair_id,
        "rights": rights,
    })) {
        Ok(v) => {
            println!(
                "Appairage approuvé : keyid={} — droits : {}",
                v.get("keyid").and_then(|x| x.as_str()).unwrap_or("?"),
                rights.join(", ")
            );
            0
        }
        Err(e) => {
            eprintln!("auth approve : {e}");
            1
        }
    }
}

fn cmd_reject(pair_id: &str) -> i32 {
    let pair_id = pair_id.trim();
    if pair_id.is_empty() {
        eprintln!("auth reject : pair_id manquant");
        return 2;
    }
    match action(json!({ "action": "pair_reject", "pair_id": pair_id })) {
        Ok(_) => {
            println!("Appairage {pair_id} refusé.");
            0
        }
        Err(e) => {
            eprintln!("auth reject : {e}");
            1
        }
    }
}

pub(crate) fn sanitize_cli_state(v: &Value) -> Value {
    json!({
        "app": v.get("app").cloned().unwrap_or_else(|| json!("mcp-control")),
        "version": v.get("version").cloned().unwrap_or_else(|| json!(env!("CARGO_PKG_VERSION"))),
        "stats": v.get("stats").cloned().unwrap_or_else(|| json!({})),
        "servers": v.get("servers").cloned().unwrap_or_else(|| json!([])),
    })
}

fn cmd_state() -> i32 {
    let port = match local_port() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("auth state : {e}");
            return 1;
        }
    };
    let (cookie, secret) = match local_session(port) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("auth state : {e}");
            return 1;
        }
    };
    let cookie_hdr = format!("{}={cookie}", crate::auth::session::COOKIE_NAME);
    let (status, _, corps) = match http_with_headers(
        "127.0.0.1",
        port,
        "GET",
        "/api/state",
        &[("Cookie", &cookie_hdr), ("X-MCC-Session", &secret)],
        None,
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("auth state : {e}");
            return 1;
        }
    };
    if status != 200 {
        eprintln!("auth state : le moteur a répondu HTTP {status}");
        return 1;
    }
    let v: Value = match serde_json::from_str(&corps) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("auth state : JSON illisible ({e})");
            return 1;
        }
    };
    println!("{}", sanitize_cli_state(&v));
    0
}

fn cmd_list() -> i32 {
    match action(json!({ "action": "client_list" })) {
        Ok(v) => {
            let clients = v
                .get("clients")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();
            if clients.is_empty() {
                println!("Aucun client appairé.");
                return 0;
            }
            println!(
                "{:<32} {:<20} {:<24} {:<8} EMPREINTE",
                "KEYID", "LIBELLÉ", "DROITS", "ÉTAT"
            );
            for c in clients {
                println!(
                    "{:<32} {:<20} {:<24} {:<8} {}",
                    c.get("keyid").and_then(|x| x.as_str()).unwrap_or(""),
                    c.get("label").and_then(|x| x.as_str()).unwrap_or(""),
                    c.get("rights")
                        .and_then(|r| r.as_array())
                        .map(|a| a
                            .iter()
                            .filter_map(|x| x.as_str())
                            .collect::<Vec<_>>()
                            .join(","))
                        .unwrap_or_default(),
                    c.get("status").and_then(|x| x.as_str()).unwrap_or(""),
                    c.get("fingerprint_court")
                        .and_then(|x| x.as_str())
                        .unwrap_or(""),
                );
            }
            0
        }
        Err(e) => {
            eprintln!("auth list : {e}");
            1
        }
    }
}

fn cmd_revoke(keyid: &str) -> i32 {
    if !crate::auth::registry::valid_keyid(keyid) {
        eprintln!("auth revoke : keyid invalide (attendu mcc-<préfixe>-<16 hex>)");
        return 2;
    }
    match action(json!({ "action": "client_revoke", "keyid": keyid })) {
        Ok(v) => {
            println!(
                "Client {keyid} révoqué — effectif à la requête suivante ({})",
                v.get("revoked_at").and_then(|x| x.as_str()).unwrap_or("-")
            );
            0
        }
        Err(e) => {
            eprintln!("auth revoke : {e}");
            1
        }
    }
}

fn cmd_fingerprint() -> i32 {
    let texte = match std::fs::read_to_string(config_path()) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("auth fingerprint : config.json illisible ({e})");
            return 1;
        }
    };
    let v: Value = match serde_json::from_str(&texte) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("auth fingerprint : config.json invalide ({e})");
            return 1;
        }
    };
    let auth = crate::auth::registry::AuthState::from_json(v.get("auth").unwrap_or(&Value::Null));
    let Some(server) = auth.server else {
        eprintln!("auth fingerprint : aucune identité de serveur dans config.json");
        return 1;
    };
    println!("keyid serveur : {}", server.keyid);
    println!(
        "empreinte     : {}",
        crate::auth::registry::fingerprint_of(&server.public_key).unwrap_or_default()
    );
    println!(
        "empreinte courte : {}",
        crate::auth::registry::short_fingerprint_of(&server.public_key).unwrap_or_default()
    );
    if let Some(interdit) = premier_secret_du_fichier(&texte) {
        eprintln!("auth fingerprint : ATTENTION, config.json contient « {interdit} » — ce fichier ne doit contenir aucune clé privée");
        return 1;
    }
    0
}

/// Garde-fou de `auth fingerprint` : aucun bloc privé ne doit traîner dans
/// `config.json` (§9). Inspecte à la fois les marqueurs PEM explicites et les
/// noms de clés JSON d'objet (`private_key`, `secret_key`, `seed`, `signing_key`,
/// `hmac_key`) afin de ne jamais déclencher de faux positif sur `"removed_seeds"`
/// ou un identifiant serveur `"seed-*"`.
pub fn premier_secret_du_fichier(texte: &str) -> Option<&'static str> {
    for motif in ["PRIVATE KEY", "BEGIN PRIVATE", "LITEMCP ED25519 PRIVATE"] {
        if texte.contains(motif) {
            return Some(motif);
        }
    }
    fn cle_secrete_json(v: &Value) -> Option<&'static str> {
        match v {
            Value::Object(map) => {
                for (k, val) in map {
                    let low = k.trim().to_ascii_lowercase();
                    let hit = match low.as_str() {
                        "private_key" | "privatekey" => Some("private_key"),
                        "secret_key" | "secretkey" => Some("secret_key"),
                        "seed" => Some("seed"),
                        "signing_key" | "signingkey" => Some("signing_key"),
                        "hmac_key" | "hmackey" => Some("hmac_key"),
                        _ => None,
                    };
                    if hit.is_some() {
                        return hit;
                    }
                    if let Some(sub) = cle_secrete_json(val) {
                        return Some(sub);
                    }
                }
                None
            }
            Value::Array(arr) => arr.iter().find_map(cle_secrete_json),
            _ => None,
        }
    }
    if let Ok(v) = serde_json::from_str::<Value>(texte) {
        return cle_secrete_json(&v);
    }
    None
}

/// `auth verify-log [--path <fichier>]` — recalcule la chaîne du journal
/// d'audit (`docs/auth.md` §11). Code 0 = intacte, 1 = falsifiée ou clé
/// introuvable, 2 = usage.
fn cmd_verify_log(chemin: Option<&str>) -> i32 {
    let path = match chemin {
        Some(p) => PathBuf::from(p),
        None => crate::audit::path_for(&config_path()),
    };
    if !path.exists() {
        println!(
            "verify-log : aucun journal d'audit ({}) — rien à vérifier.",
            path.display()
        );
        return 0;
    }
    let key_dir = match chemin {
        Some(_) => path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(config_dir),
        None => config_dir(),
    };
    let cle = match crate::auth::secret::load_audit_key(
        &key_dir,
        crate::auth::secret::service().as_deref(),
    ) {
        Ok(Some(k)) => k,
        Ok(None) => {
            eprintln!(
                "verify-log : clé du journal introuvable (trousseau `audit-hmac` et auth/audit.key) — la chaîne ne peut pas être vérifiée sur cette machine"
            );
            return 1;
        }
        Err(e) => {
            eprintln!("verify-log : {e}");
            return 1;
        }
    };
    match crate::audit::verify(&path, &cle) {
        Ok(r) => {
            println!(
                "verify-log : chaîne INTACTE — {} ligne(s), {} segment(s), seq {} → {}{}",
                r.lignes,
                r.segments,
                r.premier_seq,
                r.dernier_seq,
                if r.partiel {
                    " (chaîne partielle : segments antérieurs tournés)"
                } else {
                    ""
                }
            );
            0
        }
        Err(f) => {
            eprintln!("verify-log : {}", f.message());
            eprintln!("verify-log : journal FALSIFIÉ ({})", path.display());
            1
        }
    }
}

/// Point d'entrée : renvoie le code de sortie du process.
pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("pair") => match args.get(1) {
            Some(url) => cmd_pair(url),
            None => {
                eprintln!("auth pair : URL manquante");
                eprintln!("{USAGE}");
                2
            }
        },
        Some("pending") => cmd_pending(),
        Some("approve") => match args.get(1) {
            Some(pair_id) => cmd_approve(pair_id, &args[2..]),
            None => {
                eprintln!("auth approve : pair_id manquant");
                eprintln!("{USAGE}");
                2
            }
        },
        Some("reject") => match args.get(1) {
            Some(pair_id) => cmd_reject(pair_id),
            None => {
                eprintln!("auth reject : pair_id manquant");
                eprintln!("{USAGE}");
                2
            }
        },
        Some("state") => cmd_state(),
        Some("list") => cmd_list(),
        Some("revoke") => match args.get(1) {
            Some(keyid) => cmd_revoke(keyid),
            None => {
                eprintln!("auth revoke : keyid manquant");
                2
            }
        },
        Some("fingerprint") => cmd_fingerprint(),
        Some("verify-log") => {
            let mut reste = args.iter().skip(1);
            let chemin = match reste.next().map(String::as_str) {
                None => None,
                Some("-h") | Some("--help") => {
                    println!("{USAGE}");
                    return 0;
                }
                Some("--path") => match reste.next() {
                    Some(p) => Some(p.as_str()),
                    None => {
                        eprintln!("auth verify-log : --path attend un fichier");
                        return 2;
                    }
                },
                Some(p) if p.starts_with('-') => {
                    eprintln!("auth verify-log : option inconnue « {p} »");
                    eprintln!("{USAGE}");
                    return 2;
                }
                Some(p) => Some(p),
            };
            if let Some(extra) = reste.next() {
                eprintln!("auth verify-log : argument inattendu « {extra} »");
                return 2;
            }
            cmd_verify_log(chemin)
        }
        _ => {
            eprintln!("{USAGE}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_http_seulement_et_port_optionnel() {
        assert_eq!(
            parse_url("http://127.0.0.1:5501"),
            Ok(("127.0.0.1".to_string(), 5501))
        );
        assert_eq!(
            parse_url("http://mon-poste.local:5090/"),
            Ok(("mon-poste.local".to_string(), 5090))
        );
        assert_eq!(
            parse_url("http://exemple.test"),
            Ok(("exemple.test".to_string(), 80))
        );
        assert!(parse_url("https://exemple.test:5090")
            .unwrap_err()
            .contains("M4"));
        assert_eq!(
            parse_url_scheme("http://127.0.0.1:5504"),
            Ok((false, "127.0.0.1".to_string(), 5504))
        );
        #[cfg(feature = "tls")]
        {
            assert_eq!(
                parse_url_scheme("https://192.0.2.10:5504/"),
                Ok((true, "192.0.2.10".to_string(), 5504))
            );
            assert_eq!(
                parse_url_scheme("https://exemple.test"),
                Ok((true, "exemple.test".to_string(), 443))
            );
        }
        #[cfg(not(feature = "tls"))]
        {
            assert!(parse_url_scheme("https://192.0.2.10:5504")
                .unwrap_err()
                .contains("tls"));
        }
        assert!(parse_url("exemple.test:5090").is_err());
        assert!(parse_url("http://exemple.test:5090/api").is_err());
        assert!(parse_url("http://:5090").is_err());
        assert!(parse_url("http://exemple.test:pas-un-port").is_err());
    }

    #[test]
    fn prefixe_keyid_respecte_la_grammaire() {
        assert_eq!(keyid_prefix("DEAMON-PC"), "deamonpc");
        assert_eq!(keyid_prefix("a"), "a");
        assert_eq!(keyid_prefix("portable-de-test-tres-long"), "portable");
        assert_eq!(keyid_prefix("---"), "cli");
        let keyid = format!(
            "mcc-{}-{}",
            keyid_prefix("DEAMON-PC"),
            crate::random_hex(1).expect("entropie OS")
        );
        assert!(
            crate::auth::registry::valid_keyid(&keyid),
            "keyid généré invalide : {keyid}"
        );
    }

    #[test]
    fn extraction_cookie_session_dans_set_cookie() {
        let head = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nSet-Cookie: mcc_session=abc123_XYZ; HttpOnly; SameSite=Strict; Path=/\r\n";
        assert_eq!(extract_session_cookie(head).as_deref(), Some("abc123_XYZ"));
        assert_eq!(extract_session_cookie("HTTP/1.1 200 OK\r\n"), None);
    }

    #[test]
    fn garde_fou_config_json() {
        assert!(premier_secret_du_fichier(
            r#"{"servers":[{"id":"seed-news"}],"settings":{"removed_seeds":["seed-old"],"auth":{"clients":[]}}}"#
        )
        .is_none());
        assert_eq!(
            premier_secret_du_fichier("-----BEGIN LITEMCP ED25519 PRIVATE KEY-----"),
            Some("PRIVATE KEY")
        );
        assert_eq!(
            premier_secret_du_fichier(r#"{"auth":{"server":{"seed":"001122"}}}"#),
            Some("seed")
        );
        assert_eq!(
            premier_secret_du_fichier(r#"{"auth":{"private_key":"abcd"}}"#),
            Some("private_key")
        );
    }

    #[test]
    fn approve_rights_valides_et_rejet_admin() {
        assert_eq!(parse_approve_rights(&[]), Ok(vec!["lecture".to_string()]));
        assert_eq!(
            parse_approve_rights(&[
                "lecture".to_string(),
                "pilotage".to_string(),
                "lecture".to_string()
            ]),
            Ok(vec!["lecture".to_string(), "pilotage".to_string()])
        );
        assert!(parse_approve_rights(&["admin".to_string()]).is_err());
        assert!(parse_approve_rights(&["inconnu".to_string()]).is_err());
    }

    #[test]
    fn sanitize_cli_state_ne_laisse_que_champs_publics() {
        let raw = json!({
            "app": "mcp-control",
            "version": "0.8.8",
            "stats": {"total": 1, "up": 1},
            "servers": [{"id": "s1"}],
            "settings": {"api_access": "secret", "notify": {"webhook_url": "http://x"}},
            "log": [{"msg": "interne"}]
        });
        let out = sanitize_cli_state(&raw);
        assert_eq!(out["app"], json!("mcp-control"));
        assert_eq!(out["stats"]["total"], json!(1));
        assert_eq!(out["servers"][0]["id"], json!("s1"));
        assert!(out.get("settings").is_none());
        assert!(out.get("log").is_none());
    }
}
