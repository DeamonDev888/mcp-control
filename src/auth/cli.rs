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
  pair <url>         appairer ce poste avec un moteur MCP Control (http://hôte:port)\n\
  list               clients appairés du moteur local\n\
  revoke <keyid>     révoquer un client (effet à la requête suivante)\n\
  fingerprint        empreintes de l'identité du serveur local\n\
  verify-log [--path <fichier>]  vérifier la chaîne du journal d'audit (auth-audit.log)";

/// Squelette d'URL accepté : `http://hôte[:port][/]`. Aucun `https` (TLS = lot
/// M4), aucun chemin (l'API d'appairage est à la racine du port UI).
pub fn parse_url(url: &str) -> Result<(String, u16), String> {
    let url = url.trim();
    let reste = match url.strip_prefix("http://") {
        Some(r) => r,
        None if url.starts_with("https://") => {
            return Err(
                "https:// non pris en charge : MCP Control n'écoute pas encore en TLS (lot M4)"
                    .to_string(),
            );
        }
        None => return Err(format!("URL attendue en http://hôte:port — reçu « {url} »")),
    };
    let reste = reste.trim_end_matches('/');
    if reste.contains('/') || reste.is_empty() {
        return Err(format!("URL attendue en http://hôte:port — reçu « {url} »"));
    }
    let (host, port) = match reste.rsplit_once(':') {
        Some((h, p)) => (
            h,
            p.parse::<u16>().map_err(|_| "port invalide".to_string())?,
        ),
        None => (reste, 80),
    };
    if host.is_empty() {
        return Err("hôte vide".to_string());
    }
    Ok((host.to_string(), port))
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

/// Requête HTTP minimale (une connexion par appel, `Connection: close`).
fn http(
    host: &str,
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> Result<(u16, String), String> {
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
    Ok((status, corps.to_string()))
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

fn action(v: Value) -> Result<Value, String> {
    let port = local_port()?;
    let (status, reponse) = http_json("127.0.0.1", port, "POST", "/api/action", Some(&v))?;
    if status != 200 {
        return Err(format!("le moteur a répondu {status}"));
    }
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
    let (host, port) = match parse_url(url) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("auth pair : {e}");
            return 2;
        }
    };
    let cle = litemcp::auth::sign::SigningKey::generate();
    let cle_pub = crate::crypto::hex(&cle.verifying_key().to_bytes());
    let keyid = format!("mcc-{}-{}", keyid_prefix(&hostname()), crate::random_hex(1));
    let corps = json!({
        "keyid": keyid,
        "label": hostname().chars().take(64).collect::<String>(),
        "public_key": cle_pub,
        "created": crate::types::now_ts(),
    });
    let (status, reponse) = match http_json(&host, port, "POST", "/auth/pair", Some(&corps)) {
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
    let code = comparison_code(&pair_id, &cle_pub, &server_pub, &tls_fp);
    println!("Demande d'appairage envoyée à {host}:{port}");
    println!("  keyid client : {keyid}");
    println!(
        "  empreinte    : {}",
        crate::auth::registry::fingerprint_of(&cle_pub).unwrap_or_default()
    );
    println!();
    println!("  CODE DE COMPARAISON : {code}");
    println!();
    println!("Vérifie que ce code est IDENTIQUE dans MCP Control (fenêtre locale, écran");
    println!("« Accès »), puis approuve la demande avec les droits voulus.");
    println!("Attente de l'approbation ({} s au plus)...", PAIR_TTL_SECS);

    let limite = Instant::now() + Duration::from_secs(PAIR_TTL_SECS + 10);
    let droits: Vec<String> = loop {
        if Instant::now() >= limite {
            eprintln!("auth pair : demande expirée sans décision — clé locale effacée, rien n'est appairé");
            return 1;
        }
        std::thread::sleep(Duration::from_secs(2));
        let (status, v) =
            match http_json(&host, port, "GET", &format!("/auth/pair/{pair_id}"), None) {
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
        // L'identité du serveur ne doit pas changer en cours d'appairage.
        if v.get("server_pub").and_then(|p| p.as_str()) != Some(server_pub.as_str()) {
            eprintln!("auth pair : l'identité du serveur a CHANGÉ pendant l'appairage — abandon");
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
        let chemin = config_dir()
            .join("auth")
            .join("clients")
            .join(format!("{keyid}.key"));
        if let Err(e) = secret::write_secret_file(&chemin, &bloc) {
            eprintln!("auth pair : clé approuvée mais non enregistrée ({e}) — ré-appaire");
            return 1;
        }
    }
    let pin = json!({
        "url": format!("http://{host}:{port}"),
        "server_keyid": reponse.get("server_keyid").cloned().unwrap_or(Value::Null),
        "server_pub": server_pub,
        "tls_fp": tls_fp,
        "keyid": keyid,
        "paired_at": crate::types::iso_utc(crate::types::now_ts()),
    });
    let pin_path = config_dir()
        .join("auth")
        .join("clients")
        .join(format!("{keyid}.pin.json"));
    if let Some(parent) = pin_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(
        &pin_path,
        serde_json::to_string_pretty(&pin).unwrap_or_default(),
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
/// `config.json` (§9). Renvoie le premier motif trouvé, sinon `None`.
pub fn premier_secret_du_fichier(texte: &str) -> Option<&'static str> {
    ["PRIVATE KEY", "BEGIN PRIVATE", "seed", "SEED"]
        .into_iter()
        .find(|motif| texte.contains(motif))
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
        let keyid = format!("mcc-{}-{}", keyid_prefix("DEAMON-PC"), crate::random_hex(1));
        assert!(
            crate::auth::registry::valid_keyid(&keyid),
            "keyid généré invalide : {keyid}"
        );
    }

    #[test]
    fn garde_fou_config_json() {
        assert!(premier_secret_du_fichier(r#"{"auth":{"clients":[]}}"#).is_none());
        assert_eq!(
            premier_secret_du_fichier("-----BEGIN LITEMCP ED25519 PRIVATE KEY-----"),
            Some("PRIVATE KEY")
        );
    }
}
