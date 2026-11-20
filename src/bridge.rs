//! Pont **stdio** pour les clients MCP standards (DA6 = a, lot M5 point 1 ; `docs/auth.md` §8.1).
//!
//! Claude Desktop, Claude Code et Cursor parlent MCP sur stdin/stdout et ne
//! savent pas signer une requête HTTP. Le pont est le maillon qui détient la
//! clé : il lit un message JSON-RPC par ligne sur stdin, le POSTe sur `/mcp`
//! en **signant** avec l'identité du client local (celle de
//! `mcp-control auth pair`, contrat C9 : `Signer` de la lib), puis écrit la
//! réponse sur stdout.
//!
//! Règles du pont :
//!
//! - **stdout ne porte que du JSON-RPC**, un message par ligne ; tout
//!   diagnostic part sur stderr. Aucun bandeau, aucune bannière.
//! - La réponse rendue est celle du moteur, avec le **même `id`** (le serveur
//!   le reprend ; le pont ne le réécrit jamais sur un 200).
//! - Les notifications (pas d'`id`) ne produisent rien : le moteur répond 202
//!   sans corps, le pont n'écrit rien.
//! - Un refus d'identité (401) devient une erreur JSON-RPC `-32023` explicite
//!   qui dit quoi faire (`mcp-control auth pair <url>`), jamais un silence.
//! - Aucun secret n'est journalisé : ni clé, ni nonce, ni signature.
//!
//! La clé privée du client n'est **jamais** lue depuis `config.json` : elle
//! vient du trousseau (§9, compte `client-<hôte>:<port>`) ou de son repli
//! fichier `auth/clients/<keyid>.key`, et l'identité du serveur vient du
//! fichier d'épinglage `auth/clients/<keyid>.pin.json` écrit par
//! `auth pair`.

use litemcp::auth::sign::{RequestToSign, Signer, SigningKey};
use serde_json::{json, Value};
use std::io::{BufRead, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const USAGE: &str = "mcp-control bridge [--url http://127.0.0.1:<port>]\n\
  parle MCP sur stdin/stdout et relaie vers /mcp en signant avec l'identité\n\
  du client local (appairé par `mcp-control auth pair <url>`).\n\
  Sans --url : le port UI du config.json local.";

/// Identité locale utilisable pour signer : `keyid` + clé privée + épinglage.
struct Identite {
    keyid: String,
    signer: Signer,
}

/// Point d'entrée du binaire. Rend le code de sortie du process.
pub fn run(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        eprintln!("{USAGE}");
        return 0;
    }
    let url = match option_url(args) {
        Ok(Some(u)) => u,
        Ok(None) => match url_locale() {
            Ok(u) => u,
            Err(e) => {
                eprintln!("bridge : {e}");
                return 2;
            }
        },
        Err(e) => {
            eprintln!("bridge : {e}");
            eprintln!("{USAGE}");
            return 2;
        }
    };
    let (host, port) = match crate::auth::cli::parse_url(&url) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("bridge : {e}");
            return 2;
        }
    };
    let authority = format!("{host}:{port}");
    let identite = match identite_locale(&host, port) {
        Ok(i) => i,
        Err(message) => {
            eprintln!("{message}");
            return 2;
        }
    };
    eprintln!(
        "bridge : stdio → http://{authority}/mcp (keyid {})",
        identite.keyid
    );
    let stdin = std::io::stdin();
    let mut sortie = std::io::stdout().lock();
    for ligne in stdin.lock().lines() {
        let ligne = match ligne {
            Ok(l) => l,
            Err(_) => break,
        };
        if ligne.trim().is_empty() {
            continue;
        }
        let reponse = traiter_ligne(&ligne, &identite, &host, port, &authority, &url);
        if let Some(texte) = reponse {
            if writeln!(sortie, "{texte}").is_err() {
                return 0;
            }
            let _ = sortie.flush();
        }
    }
    0
}

fn option_url(args: &[String]) -> Result<Option<String>, String> {
    let mut i = 0;
    let mut url: Option<String> = None;
    while i < args.len() {
        let a = &args[i];
        if a == "--url" {
            let val = args
                .get(i + 1)
                .cloned()
                .ok_or_else(|| "--url attend une valeur".to_string())?;
            if url.replace(val).is_some() {
                return Err("--url fourni plus d'une fois".to_string());
            }
            i += 2;
            continue;
        }
        if let Some(v) = a.strip_prefix("--url=") {
            if v.is_empty() {
                return Err("--url= attend une valeur".to_string());
            }
            if url.replace(v.to_string()).is_some() {
                return Err("--url fourni plus d'une fois".to_string());
            }
            i += 1;
            continue;
        }
        return Err(format!("argument inconnu : {a}"));
    }
    Ok(url)
}

/// Port UI du moteur local, lu dans `config.json` (aucune supposition).
fn url_locale() -> Result<String, String> {
    let port = port_local()?;
    Ok(format!("http://127.0.0.1:{port}"))
}

fn config_path() -> String {
    crate::manage::resolve_config_path(env!("CARGO_MANIFEST_DIR"), std::env::var("MCC_CONFIG").ok())
}

fn config_dir() -> PathBuf {
    Path::new(&config_path())
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn port_local() -> Result<u16, String> {
    let texte = std::fs::read_to_string(config_path()).map_err(|e| {
        format!("config.json illisible ({e}) — lance d'abord MCP Control, ou donne --url")
    })?;
    let v: Value =
        serde_json::from_str(&texte).map_err(|e| format!("config.json invalide : {e}"))?;
    v.get("settings")
        .and_then(|s| s.get("ui_port"))
        .and_then(|p| p.as_u64())
        .map(|p| p as u16)
        .ok_or_else(|| "ui_port absent de config.json — donne --url".to_string())
}

/// Retrouve l'identité du client local pour ce serveur : épinglage puis clé
/// privée (trousseau, sinon repli fichier). Message actionnable sinon.
fn identite_locale(host: &str, port: u16) -> Result<Identite, String> {
    let dir = config_dir().join("auth").join("clients");
    let voulu = format!("http://{host}:{port}");
    let pin = trouver_pin(&dir, &voulu).ok_or_else(|| {
        format!(
            "bridge : ce poste n'est pas appairé avec {voulu}.\n\
             Lance : mcp-control auth pair {voulu}\n\
             puis approuve la demande dans la fenêtre locale de MCP Control."
        )
    })?;
    if let Some(tls_fp) = pin
        .get("tls_fp")
        .and_then(|t| t.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        // Contrat C9 : l'empreinte épinglée interdit un repli HTTP en clair.
        let _ = litemcp::auth::PinnedServer::new(tls_fp);
        return Err(format!(
            "bridge : {voulu} a une empreinte TLS épinglée — connexion HTTP en clair refusée (lot M4 requis)"
        ));
    }
    let keyid = pin
        .get("keyid")
        .and_then(|k| k.as_str())
        .unwrap_or("")
        .to_string();
    if !crate::auth::registry::valid_keyid(&keyid) {
        return Err(format!(
            "bridge : épinglage illisible pour {voulu} — relance : mcp-control auth pair {voulu}"
        ));
    }
    let compte = format!("client-{host}:{port}");
    let bloc = match crate::auth::secret::load_client_key(
        crate::auth::secret::service().as_deref(),
        &compte,
    ) {
        Ok(Some(b)) => Some(b),
        _ => None,
    };
    let bloc = match bloc {
        Some(b) => b,
        None => std::fs::read_to_string(dir.join(format!("{keyid}.key"))).map_err(|_| {
            format!(
                "bridge : clé privée de {keyid} introuvable (trousseau et fichier) — relance : mcp-control auth pair {voulu}"
            )
        })?,
    };
    let cle = SigningKey::import(&bloc).map_err(|_| {
        format!(
            "bridge : clé privée de {keyid} illisible — relance : mcp-control auth pair {voulu}"
        )
    })?;
    Ok(Identite {
        keyid: keyid.clone(),
        signer: Signer::new(cle).with_keyid(keyid),
    })
}

/// Épinglage le plus récent dont l'URL correspond exactement (`host:port`).
fn trouver_pin(dir: &Path, url: &str) -> Option<Value> {
    let entrees = std::fs::read_dir(dir).ok()?;
    let mut meilleur: Option<(String, std::time::SystemTime, Value)> = None;
    for e in entrees.flatten() {
        let nom = e.file_name().to_string_lossy().to_string();
        if !nom.ends_with(".pin.json") {
            continue;
        }
        let texte = match std::fs::read_to_string(e.path()) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let v: Value = match serde_json::from_str(&texte) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("url").and_then(|u| u.as_str()) == Some(url) {
            let paired_at = v
                .get("paired_at")
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .to_string();
            let mtime = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            let remplace = match &meilleur {
                None => true,
                Some((prev_ts, prev_mt, _)) => {
                    paired_at > *prev_ts || (paired_at == *prev_ts && mtime > *prev_mt)
                }
            };
            if remplace {
                meilleur = Some((paired_at, mtime, v));
            }
        }
    }
    meilleur.map(|(_, _, v)| v)
}

// ---------- un message ----------

fn construire_requete(
    identite: &Identite,
    authority: &str,
    v: &Value,
    methode: &str,
) -> Result<String, &'static str> {
    let corps = serde_json::to_string(v).map_err(|_| "invalid request: body")?;
    // En-têtes du profil : `Mcp-Method` toujours (composant couvert exigé),
    // `Mcp-Name` seulement là où le transport l'exige. `Host` est écrit une
    // seule fois dans l'en-tête HTTP ci-dessous (jamais dupliqué).
    let mut entetes: Vec<(String, String)> = vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("Accept".to_string(), "application/json".to_string()),
        ("Mcp-Method".to_string(), methode.to_string()),
    ];
    if matches!(methode, "tools/call" | "prompts/get" | "resources/read") {
        let nom = if methode == "resources/read" {
            v.pointer("/params/uri").and_then(|n| n.as_str())
        } else {
            v.pointer("/params/name").and_then(|n| n.as_str())
        };
        if let Some(nom) = nom.filter(|n| !n.trim().is_empty()) {
            entetes.push(("Mcp-Name".to_string(), nom.to_string()));
        }
    }
    let refs: Vec<(&str, &str)> = entetes
        .iter()
        .map(|(n, v)| (n.as_str(), v.as_str()))
        .collect();
    let a_signer = RequestToSign::new("POST", "/mcp", authority, &refs, corps.as_bytes());
    let signes = identite.signer.headers(&a_signer);
    if !signes.is_complete() {
        return Err("invalid request: méthode MCP non signable");
    }
    let mut requete = format!("POST /mcp HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n");
    for (n, valeur) in entetes
        .iter()
        .map(|(n, v)| (n.as_str(), v.as_str()))
        .chain(signes.iter())
    {
        requete.push_str(&format!("{n}: {valeur}\r\n"));
    }
    requete.push_str(&format!("Content-Length: {}\r\n\r\n", corps.len()));
    requete.push_str(&corps);
    Ok(requete)
}

fn traiter_ligne(
    ligne: &str,
    identite: &Identite,
    host: &str,
    port: u16,
    authority: &str,
    url: &str,
) -> Option<String> {
    let v: Value = match serde_json::from_str(ligne) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("bridge : ligne d'entrée illisible (JSON attendu) — erreur -32700 rendue");
            return Some(erreur_json(&Value::Null, -32700, "parse error", None));
        }
    };
    let est_notification = v.get("id").is_none();
    let id = v.get("id").cloned().unwrap_or(Value::Null);
    let Some(methode) = v.get("method").and_then(|m| m.as_str()) else {
        eprintln!("bridge : message sans méthode — erreur -32600 rendue");
        return (!est_notification)
            .then(|| erreur_json(&id, -32600, "invalid request: method", None));
    };
    let requete = match construire_requete(identite, authority, &v, methode) {
        Ok(r) => r,
        Err(msg) => {
            eprintln!("bridge : requête non signable ({msg})");
            return (!est_notification).then(|| erreur_json(&id, -32600, msg, None));
        }
    };

    let (statut, entete_reponse, corps_reponse) = match envoyer(host, port, requete.as_bytes()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("bridge : {e}");
            return (!est_notification).then(|| {
                erreur_json(
                    &id,
                    -32603,
                    &format!("moteur injoignable ({e}) — MCP Control est-il lancé ?"),
                    None,
                )
            });
        }
    };
    match statut {
        // Notification (pas d'id) : 202 sans corps, rien à rendre.
        202 if corps_reponse.trim().is_empty() => None,
        200 => (!est_notification).then(|| compact(&corps_reponse)),
        401 => {
            eprintln!("bridge : identité refusée par le moteur (401)");
            (!est_notification).then(|| {
                erreur_json(
                    &id,
                    -32023,
                    &format!("non autorisé — ré-appaire ce poste : mcp-control auth pair {url}"),
                    Some("unauthenticated"),
                )
            })
        }
        429 => {
            let delai = entete_reponse
                .lines()
                .find_map(|l| {
                    let (n, v) = l.split_once(':')?;
                    n.eq_ignore_ascii_case("retry-after")
                        .then(|| v.trim().to_string())
                })
                .unwrap_or_else(|| "?".to_string());
            eprintln!("bridge : moteur limité en débit (429, Retry-After {delai})");
            (!est_notification).then(|| {
                erreur_json(
                    &id,
                    -32023,
                    &format!("trop d'échecs d'authentification — réessayez dans {delai} s"),
                    Some("rate_limited"),
                )
            })
        }
        autre => {
            // 403 (-32023), 400, 415, 500… : le corps du moteur est déjà une
            // enveloppe JSON-RPC, on la rend telle quelle.
            if est_notification {
                eprintln!("bridge : moteur a répondu {autre} à une notification");
                None
            } else if serde_json::from_str::<Value>(&corps_reponse).is_ok() {
                Some(compact(&corps_reponse))
            } else {
                eprintln!("bridge : moteur a répondu {autre} sans corps JSON-RPC");
                Some(erreur_json(
                    &id,
                    -32603,
                    &format!("moteur : HTTP {autre}"),
                    None,
                ))
            }
        }
    }
}

/// Requête HTTP/1.1 minimale, une connexion par message (`Connection: close`).
fn envoyer(host: &str, port: u16, requete: &[u8]) -> Result<(u16, String, String), String> {
    let addr = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("résolution {host}:{port} impossible : {e}"))?
        .next()
        .ok_or_else(|| format!("aucune adresse pour {host}:{port}"))?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .map_err(|e| format!("connexion {host}:{port} impossible : {e}"))?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(60)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(5)));
    s.write_all(requete)
        .map_err(|e| format!("envoi impossible : {e}"))?;
    let mut data = Vec::new();
    let mut tmp = [0u8; 16384];
    loop {
        match s.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => data.extend_from_slice(&tmp[..n]),
            Err(e) => return Err(format!("lecture impossible : {e}")),
        }
        if data.len() > 64 * 1024 * 1024 {
            break;
        }
    }
    if data.is_empty() {
        return Err(format!("{host}:{port} a fermé sans répondre"));
    }
    let texte = String::from_utf8_lossy(&data).to_string();
    let (entete, corps) = texte.split_once("\r\n\r\n").unwrap_or((&texte, ""));
    let statut = entete
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| "réponse HTTP illisible".to_string())?;
    Ok((statut, entete.to_string(), corps.to_string()))
}

/// Une ligne JSON compacte : le protocole stdio est délimité par les retours à
/// la ligne, un corps multiligne casserait le flux.
fn compact(corps: &str) -> String {
    match serde_json::from_str::<Value>(corps) {
        Ok(v) => v.to_string(),
        Err(_) => corps.replace(['\r', '\n'], " ").trim().to_string(),
    }
}

fn erreur_json(id: &Value, code: i64, message: &str, raison: Option<&str>) -> String {
    let mut err = json!({ "code": code, "message": message });
    if let Some(raison) = raison {
        err["data"] = json!({ "reason": raison });
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": err }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_optionnelle_et_formes_acceptees() {
        assert_eq!(option_url(&[]).unwrap(), None);
        assert_eq!(
            option_url(&["--url".into(), "http://127.0.0.1:5505".into()]).unwrap(),
            Some("http://127.0.0.1:5505".to_string())
        );
        assert_eq!(
            option_url(&["--url=http://127.0.0.1:5505".into()]).unwrap(),
            Some("http://127.0.0.1:5505".to_string())
        );
        assert!(option_url(&["--url".into()]).is_err());
        assert!(option_url(&["--inconnu".into()]).is_err());
    }

    #[test]
    fn requete_signee_a_un_seul_host_et_les_trois_entetes() {
        let cle = SigningKey::generate();
        let ident = Identite {
            keyid: "mcc-test-0000000000000001".to_string(),
            signer: Signer::new(cle).with_keyid("mcc-test-0000000000000001"),
        };
        let v = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "list_servers", "arguments": {} }
        });
        let req = construire_requete(&ident, "127.0.0.1:5505", &v, "tools/call").unwrap();
        let (head, _) = req.split_once("\r\n\r\n").unwrap();
        let host_count = head
            .lines()
            .filter(|l| l.to_ascii_lowercase().starts_with("host:"))
            .count();
        assert_eq!(host_count, 1, "un seul Host attendu :\n{head}");
        assert!(head.contains("Content-Digest: sha-256=:"));
        assert!(head.contains("Signature-Input: sig1="));
        assert!(head.contains("Signature: sig1=:"));
        assert!(head.contains("Mcp-Method: tools/call"));
        assert!(head.contains("Mcp-Name: list_servers"));
    }

    #[test]
    fn trouver_pin_choisit_le_plus_recent() {
        let dir = std::env::temp_dir().join(format!("mcc-bridge-pin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("mcc-a-0000000000000001.pin.json"),
            json!({
                "url": "http://127.0.0.1:5505",
                "keyid": "mcc-a-0000000000000001",
                "paired_at": "2026-11-19 10:00:00",
                "tls_fp": ""
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            dir.join("mcc-a-0000000000000002.pin.json"),
            json!({
                "url": "http://127.0.0.1:5505",
                "keyid": "mcc-a-0000000000000002",
                "paired_at": "2026-11-20 10:00:00",
                "tls_fp": ""
            })
            .to_string(),
        )
        .unwrap();
        let choisi = trouver_pin(&dir, "http://127.0.0.1:5505").unwrap();
        assert_eq!(choisi["keyid"], "mcc-a-0000000000000002");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn erreur_json_a_la_forme_jsonrpc() {
        let v: Value = serde_json::from_str(&erreur_json(
            &json!(7),
            -32023,
            "non",
            Some("unauthenticated"),
        ))
        .unwrap();
        assert_eq!(v["jsonrpc"], json!("2.0"));
        assert_eq!(v["id"], json!(7));
        assert_eq!(v["error"]["code"], json!(-32023));
        assert_eq!(v["error"]["data"]["reason"], json!("unauthenticated"));
        assert!(
            !v.to_string().contains('\n'),
            "une erreur doit tenir sur une ligne"
        );
    }

    #[test]
    fn compact_tient_sur_une_ligne() {
        assert_eq!(compact("{\n \"a\": 1\n}"), "{\"a\":1}");
        assert_eq!(
            compact("pas du json\nsur deux lignes"),
            "pas du json sur deux lignes"
        );
    }
}
