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

use litemcp::auth::sign::{RequestToSign, Signer};
use serde_json::{json, Value};
use std::io::{BufRead, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const USAGE: &str = "mcp-control bridge [--url http(s)://127.0.0.1:<port>[/mcp/s/<id>]]\n\
  parle MCP sur stdin/stdout et relaie vers /mcp (ou /mcp/s/<id> en mode\n\
  hybride stdio) en signant avec l'identité du client local (appairé par\n\
  `mcp-control auth pair <url>`, avec vérification PinnedServer en TLS 1.3).\n\
  Sans --url : le port UI du config.json local.";

/// Identité locale utilisable pour signer : `keyid` + clé privée + épinglage TLS éventuel.
pub(crate) struct Identite {
    pub(crate) keyid: String,
    pub(crate) signer: Signer,
}

fn format_authority(host: &str, port: u16) -> String {
    let clean = host.trim().trim_start_matches('[').trim_end_matches(']');
    if clean.contains(':') {
        format!("[{clean}]:{port}")
    } else {
        format!("{clean}:{port}")
    }
}

/// Analyse une URL de pont `http(s)://host[:port][path]` :
/// - `""`, `"/"`, `"/mcp"`, `"/mcp/"` → chemin `"/mcp"` ;
/// - `"/mcp/s/<id>"` (slash final toléré, `<id>` ASCII `[A-Za-z0-9._-]+` sans `..`)
///   → chemin `"/mcp/s/<id>"` ;
/// - tout autre chemin ou schéma est refusé explicitement.
pub(crate) fn parse_bridge_url_full(url: &str) -> Result<(bool, String, u16, String), String> {
    let url = url.trim();
    let (is_https, reste) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return Err(format!(
            "URL attendue en http(s)://hôte[:port][/mcp/s/<id>] — reçu « {url} »"
        ));
    };
    let (authority_part, path_part) = match reste.find('/') {
        Some(idx) => (&reste[..idx], &reste[idx..]),
        None => (reste, ""),
    };
    if authority_part.is_empty()
        || authority_part.contains('?')
        || authority_part.contains('#')
        || path_part.contains('?')
        || path_part.contains('#')
    {
        return Err(format!(
            "URL bridge invalide (requête ou fragment interdit) : « {url} »"
        ));
    }
    let scheme = if is_https { "https" } else { "http" };
    let (_, host, port) =
        crate::auth::cli::parse_url_scheme(&format!("{scheme}://{authority_part}"))?;
    let target_path = if path_part.is_empty()
        || path_part == "/"
        || path_part == "/mcp"
        || path_part == "/mcp/"
    {
        "/mcp".to_string()
    } else if let Some(raw_id) = path_part.strip_prefix("/mcp/s/") {
        let id = raw_id.trim_end_matches('/');
        if id.is_empty()
            || id.contains('/')
            || id.contains('?')
            || id.contains('#')
            || id.contains("..")
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Err(format!(
                "identifiant serveur invalide dans /mcp/s/<id> : « {url} »"
            ));
        }
        format!("/mcp/s/{id}")
    } else {
        return Err(format!(
            "chemin non pris en charge dans l'URL bridge (attendu /, /mcp ou /mcp/s/<id>) : « {url} »"
        ));
    };
    Ok((is_https, host, port, target_path))
}

#[cfg(test)]
pub(crate) fn parse_bridge_url(url: &str) -> Result<(String, u16, String), String> {
    let (_, host, port, path) = parse_bridge_url_full(url)?;
    Ok((host, port, path))
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
    let (mut is_https, host, port, target_path) = match parse_bridge_url_full(&url) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("bridge : {e}");
            return 2;
        }
    };
    if !is_https && !crate::is_loopback_bind(&host) {
        eprintln!(
            "bridge : connexion HTTP en clair hors loopback refusée (DA5 : utiliser https://{}{target_path})",
            format_authority(&host, port)
        );
        return 2;
    }
    let (mut identite, mut tls_fp) = match identite_locale(is_https, &host, port, &target_path) {
        Ok(v) => v,
        Err(message) => {
            eprintln!("{message}");
            return 2;
        }
    };
    if !tls_fp.is_empty() {
        is_https = true;
    }
    if is_https && !crate::auth::net::tls_compiled() {
        eprintln!(
            "bridge : connexion HTTPS impossible — ce binaire est compilé sans la feature `tls`"
        );
        return 2;
    }
    let scheme = if is_https { "https" } else { "http" };
    let authority = format_authority(&host, port);
    let pair_url = format!("{scheme}://{authority}");
    eprintln!(
        "bridge : stdio → {scheme}://{authority}{target_path} (keyid {})",
        identite.keyid
    );
    const MAX_BRIDGE_LINE: usize = 32 * 1024 * 1024;
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let mut sortie = std::io::stdout().lock();
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        let n = match reader
            .by_ref()
            .take((MAX_BRIDGE_LINE + 1) as u64)
            .read_until(b'\n', &mut buf)
        {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        if n > MAX_BRIDGE_LINE {
            // GW-C04 : purger le reste de la ligne surdimensionnée sans OOM.
            let mut drain = [0u8; 4096];
            loop {
                match reader.read(&mut drain) {
                    Ok(0) | Err(_) => break,
                    Ok(k) if drain[..k].contains(&b'\n') => break,
                    Ok(_) => {}
                }
            }
            let err = erreur_json(
                &Value::Null,
                -32700,
                "message JSON-RPC trop volumineux (> 32 Mio)",
                None,
            );
            if writeln!(sortie, "{err}").is_err() {
                return 0;
            }
            let _ = sortie.flush();
            continue;
        }
        let ligne = String::from_utf8_lossy(&buf);
        if ligne.trim().is_empty() {
            continue;
        }
        let (reponse, got_401) = traiter_ligne_avec_statut(
            &ligne,
            &identite,
            is_https,
            tls_fp.as_str(),
            &host,
            port,
            &authority,
            &target_path,
            &pair_url,
        );
        let reponse = if got_401 {
            // GW-C04 : si la clé a été tournée/ré-appairée sur disque, recharger
            // une fois et retenter la requête avant de remonter 401.
            if let Ok((new_id, new_fp)) = identite_locale(is_https, &host, port, &target_path) {
                if new_id.keyid != identite.keyid {
                    eprintln!(
                        "bridge : identité rechargée après 401 ({} → {})",
                        identite.keyid, new_id.keyid
                    );
                    identite = new_id;
                    if !new_fp.is_empty() {
                        tls_fp = new_fp;
                    }
                    traiter_ligne_avec_statut(
                        &ligne,
                        &identite,
                        is_https,
                        tls_fp.as_str(),
                        &host,
                        port,
                        &authority,
                        &target_path,
                        &pair_url,
                    )
                    .0
                } else {
                    reponse
                }
            } else {
                reponse
            }
        } else {
            reponse
        };
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
/// Si un épinglage local porte déjà une empreinte TLS (`tls_fp`), bascule
/// automatiquement en `https://127.0.0.1:<port>` (compatibilité TLS 1.3).
fn url_locale() -> Result<String, String> {
    let port = port_local()?;
    let use_https = crate::auth::net::find_existing_pin(&config_dir(), "127.0.0.1", port)
        .and_then(|v| {
            v.get("tls_fp")
                .and_then(|t| t.as_str())
                .map(|s| !s.trim().is_empty())
        })
        .unwrap_or(false);
    let scheme = if use_https { "https" } else { "http" };
    Ok(format!("{scheme}://127.0.0.1:{port}"))
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
        .and_then(|p| u16::try_from(p).ok())
        .filter(|&p| p >= 1)
        .ok_or_else(|| "ui_port absent ou invalide dans config.json — donne --url".to_string())
}

/// Retrouve l'identité du client local pour ce serveur : épinglage (avec
/// normalisation loopback `localhost` ↔ `127.0.0.1` ↔ `::1`) puis clé privée
/// (trousseau, sinon repli fichier). Vérifie le contrat C9 (jamais de repli
/// HTTP en clair si une empreinte TLS est épinglée).
fn identite_locale(
    is_https: bool,
    host: &str,
    port: u16,
    target_path: &str,
) -> Result<(Identite, String), String> {
    let authority = format_authority(host, port);
    let scheme = if is_https { "https" } else { "http" };
    let voulu = format!("{scheme}://{authority}");
    let out = crate::auth::net::outbound_identity_for_dir(&config_dir(), host, port)
        .map_err(|e| format!("bridge : {e} — relance : mcp-control auth pair {voulu}"))?
        .ok_or_else(|| {
            format!(
                "bridge : ce poste n'est pas appairé avec {voulu}.\n\
                 Lance : mcp-control auth pair {voulu}\n\
                 puis approuve la demande dans la fenêtre locale de MCP Control."
            )
        })?;
    if !out.tls_fp.is_empty() && !is_https {
        return Err(format!(
            "bridge : {authority} a une empreinte TLS épinglée — connexion HTTP en clair refusée (utiliser --url https://{authority}{target_path})"
        ));
    }
    if is_https && out.tls_fp.is_empty() {
        return Err(format!(
            "bridge : {voulu} n'a aucune empreinte TLS épinglée — relance : mcp-control auth pair https://{authority}"
        ));
    }
    Ok((
        Identite {
            keyid: out.keyid,
            signer: out.signer,
        },
        out.tls_fp,
    ))
}

#[cfg(test)]
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

pub(crate) fn construire_requete(
    identite: &Identite,
    authority: &str,
    target_path: &str,
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
    let a_signer = RequestToSign::new("POST", target_path, authority, &refs, corps.as_bytes());
    let signes = identite.signer.headers(&a_signer);
    if !signes.is_complete() {
        return Err("invalid request: méthode MCP non signable");
    }
    let mut requete =
        format!("POST {target_path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n");
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

#[allow(clippy::too_many_arguments, dead_code)]
fn traiter_ligne(
    ligne: &str,
    identite: &Identite,
    is_https: bool,
    tls_fp: &str,
    host: &str,
    port: u16,
    authority: &str,
    target_path: &str,
    url: &str,
) -> Option<String> {
    traiter_ligne_avec_statut(
        ligne,
        identite,
        is_https,
        tls_fp,
        host,
        port,
        authority,
        target_path,
        url,
    )
    .0
}

#[allow(clippy::too_many_arguments)]
fn traiter_ligne_avec_statut(
    ligne: &str,
    identite: &Identite,
    is_https: bool,
    tls_fp: &str,
    host: &str,
    port: u16,
    authority: &str,
    target_path: &str,
    url: &str,
) -> (Option<String>, bool) {
    let v: Value = match serde_json::from_str(ligne) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("bridge : ligne d'entrée illisible (JSON attendu) — erreur -32700 rendue");
            return (
                Some(erreur_json(&Value::Null, -32700, "parse error", None)),
                false,
            );
        }
    };
    let est_notification = v.get("id").is_none();
    let id = v.get("id").cloned().unwrap_or(Value::Null);
    let Some(methode) = v.get("method").and_then(|m| m.as_str()) else {
        eprintln!("bridge : message sans méthode — erreur -32600 rendue");
        return (
            (!est_notification).then(|| erreur_json(&id, -32600, "invalid request: method", None)),
            false,
        );
    };
    let requete = match construire_requete(identite, authority, target_path, &v, methode) {
        Ok(r) => r,
        Err(msg) => {
            eprintln!("bridge : requête non signable ({msg})");
            return (
                (!est_notification).then(|| erreur_json(&id, -32600, msg, None)),
                false,
            );
        }
    };

    let (statut, entete_reponse, corps_reponse) =
        match envoyer(is_https, tls_fp, host, port, requete.as_bytes()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("bridge : {e}");
                return (
                    (!est_notification).then(|| {
                        erreur_json(
                            &id,
                            -32603,
                            &format!("moteur injoignable ({e}) — MCP Control est-il lancé ?"),
                            None,
                        )
                    }),
                    false,
                );
            }
        };
    match statut {
        // Notification (pas d'id) : 202 sans corps, rien à rendre.
        202 if corps_reponse.trim().is_empty() => (None, false),
        200 => ((!est_notification).then(|| compact(&corps_reponse)), false),
        401 => {
            eprintln!("bridge : identité refusée par le moteur (401)");
            (
                (!est_notification).then(|| {
                    erreur_json(
                        &id,
                        -32023,
                        &format!(
                            "non autorisé — ré-appaire ce poste : mcp-control auth pair {url}"
                        ),
                        Some("unauthenticated"),
                    )
                }),
                true,
            )
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
            (
                (!est_notification).then(|| {
                    erreur_json(
                        &id,
                        -32023,
                        &format!("trop d'échecs d'authentification — réessayez dans {delai} s"),
                        Some("rate_limited"),
                    )
                }),
                false,
            )
        }
        autre => {
            // 403 (-32023), 400, 415, 500… : le corps du moteur est déjà une
            // enveloppe JSON-RPC, on la rend telle quelle.
            let resp = if est_notification {
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
            };
            (resp, false)
        }
    }
}

/// Requête HTTP/1.1 minimale (en clair en loopback ou TLS 1.3 + `PinnedServer`
/// si `is_https`), une connexion par message (`Connection: close`).
fn envoyer(
    is_https: bool,
    tls_fp: &str,
    host: &str,
    port: u16,
    requete: &[u8],
) -> Result<(u16, String, String), String> {
    let data = if is_https {
        let expected = (!tls_fp.trim().is_empty()).then_some(tls_fp.trim());
        let (raw, _) = crate::auth::net::https_exchange_raw(
            host,
            port,
            requete,
            Duration::from_secs(60),
            expected,
            64 * 1024 * 1024,
        )?;
        raw
    } else {
        let clean_host = host.trim().trim_start_matches('[').trim_end_matches(']');
        let addr = (clean_host, port)
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
        data
    };
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
    use litemcp::auth::sign::SigningKey;

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
        let req = construire_requete(&ident, "127.0.0.1:5505", "/mcp", &v, "tools/call").unwrap();
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
    fn parse_bridge_url_accepte_racine_mcp_et_hybride_boutique() {
        assert_eq!(
            parse_bridge_url("http://127.0.0.1:5090").unwrap(),
            ("127.0.0.1".to_string(), 5090, "/mcp".to_string())
        );
        assert_eq!(
            parse_bridge_url("http://127.0.0.1:5090/").unwrap(),
            ("127.0.0.1".to_string(), 5090, "/mcp".to_string())
        );
        assert_eq!(
            parse_bridge_url("http://127.0.0.1:5090/mcp").unwrap(),
            ("127.0.0.1".to_string(), 5090, "/mcp".to_string())
        );
        assert_eq!(
            parse_bridge_url("http://127.0.0.1:5090/mcp/").unwrap(),
            ("127.0.0.1".to_string(), 5090, "/mcp".to_string())
        );
        assert_eq!(
            parse_bridge_url("http://127.0.0.1:5090/mcp/s/store-github").unwrap(),
            (
                "127.0.0.1".to_string(),
                5090,
                "/mcp/s/store-github".to_string()
            )
        );
        assert_eq!(
            parse_bridge_url("http://127.0.0.1:5090/mcp/s/store-github/").unwrap(),
            (
                "127.0.0.1".to_string(),
                5090,
                "/mcp/s/store-github".to_string()
            )
        );
        #[cfg(feature = "tls")]
        {
            assert_eq!(
                parse_bridge_url_full("https://127.0.0.1:5090/mcp/s/store-github").unwrap(),
                (
                    true,
                    "127.0.0.1".to_string(),
                    5090,
                    "/mcp/s/store-github".to_string()
                )
            );
            assert_eq!(
                parse_bridge_url_full("https://[::1]:5090/mcp").unwrap(),
                (true, "::1".to_string(), 5090, "/mcp".to_string())
            );
        }
        assert!(parse_bridge_url("ftp://127.0.0.1:5090/mcp").is_err());
        assert!(parse_bridge_url("http://127.0.0.1:5090/mcp/s/").is_err());
        assert!(parse_bridge_url("http://127.0.0.1:5090/mcp/s/a/b").is_err());
        assert!(parse_bridge_url("http://127.0.0.1:5090/mcp/s/..").is_err());
        assert!(parse_bridge_url("http://127.0.0.1:5090/mcp/s/store-github?x=1").is_err());
        assert!(parse_bridge_url("http://127.0.0.1:5090/mcp/s/store-github#frag").is_err());
        assert!(parse_bridge_url("http://127.0.0.1:5090/api/state").is_err());
    }

    #[test]
    fn requete_signee_hybride_poste_et_signe_le_chemin_serveur() {
        use litemcp::auth::sign::{ClientKey, NonceCache, SignatureVerifier, TestKeyStore};
        use litemcp::auth::{VerifiedRequest, Verifier};
        use std::sync::Arc;

        let cle = SigningKey::generate();
        let vk = cle.verifying_key();
        let keyid = "mcc-hyb-0000000000000001";
        let ident = Identite {
            keyid: keyid.to_string(),
            signer: Signer::new(cle).with_keyid(keyid),
        };
        let v = json!({
            "jsonrpc": "2.0",
            "id": 42,
            "method": "tools/call",
            "params": { "name": "search_repositories", "arguments": { "query": "mcp" } }
        });
        let req = construire_requete(
            &ident,
            "127.0.0.1:5090",
            "/mcp/s/store-github",
            &v,
            "tools/call",
        )
        .unwrap();
        let (head, body) = req.split_once("\r\n\r\n").unwrap();
        assert!(
            head.starts_with("POST /mcp/s/store-github HTTP/1.1\r\n"),
            "ligne de requête inattendue :\n{head}"
        );
        let hdrs: Vec<(String, String)> = head
            .lines()
            .skip(1)
            .filter_map(|l| {
                let (n, val) = l.split_once(':')?;
                Some((n.trim().to_ascii_lowercase(), val.trim().to_string()))
            })
            .collect();
        let refs: Vec<(&str, &str)> = hdrs.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();

        let store = Arc::new(TestKeyStore::new());
        store.insert(
            keyid,
            ClientKey::new("bridge-test", vk.to_bytes()).with_right("execution"),
        );
        let verifier = SignatureVerifier::new(store, NonceCache::default());

        let ok = verifier.verify(&VerifiedRequest::new(
            "POST",
            "/mcp/s/store-github",
            "127.0.0.1:5090",
            &refs,
            body.as_bytes(),
        ));
        assert!(ok.is_ok(), "signature hybride valide attendue : {ok:?}");

        let wrong_path = verifier.verify(&VerifiedRequest::new(
            "POST",
            "/mcp",
            "127.0.0.1:5090",
            &refs,
            body.as_bytes(),
        ));
        assert!(
            wrong_path.is_err(),
            "la signature liée à /mcp/s/store-github ne doit pas passer sur /mcp"
        );
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
