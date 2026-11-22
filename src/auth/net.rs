//! TLS 1.3 réseau, épinglage sortant et appairage des serveurs LiteMCP verrouillés.
//!
//! Spécifié par `docs/auth.md` §3.4, §7, §8.1–§8.3, §9 et §10.
//!
//! - **Écoute hors loopback (DA5 = a)** : TLS 1.3 uniquement (`rustls` 0.23.46 sans
//!   `tls12`, les trois suites TLS 1.3 de `ring`, ALPN `http/1.1`), certificat
//!   auto-signé généré au premier lancement (`rcgen` 0.14.7) et persisté dans
//!   `<config_dir>/tls/server.crt`, clé privée au trousseau (`server-tls`) ou
//!   repli `<config_dir>/tls/server.key` à permissions restreintes (0600 / ACL
//!   utilisateur seul sous Windows), journalisé. Sans la feature `tls`, toute
//!   écoute hors loopback est refusée et le moteur reste en `127.0.0.1`.
//! - **Épinglage client (contrat C9)** : `PinnedServer` vérifie en temps constant
//!   l'empreinte `sha256:<64 hex>` du certificat feuille observé pendant la
//!   poignée de main TLS 1.3 ; toute différence ou tout repli HTTP est refusé.
//! - **Sondes et proxy signés** : vers un pair MCP Control (droit `lecture`) ou
//!   un serveur LiteMCP verrouillé (`401` + `WWW-Authenticate` de la lib), les
//!   requêtes sortantes sont signées avec `Signer` et vérifiées avec `PinnedServer`.
//! - **Action locale `server_pair` (droit `admin`)** : génère et range une clé
//!   cliente Ed25519 (trousseau `client-<hôte>:<port>` ou repli fichier
//!   `auth/clients/<keyid>.key`) et épingle le serveur dans
//!   `auth/clients/<keyid>.pin.json`.

use litemcp::auth::sign::{RequestToSign, Signer, SigningKey};
use litemcp::auth::PinnedServer;
use serde_json::{json, Value};
#[cfg(feature = "tls")]
use std::io::{Read, Write};
#[cfg(feature = "tls")]
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
#[cfg(feature = "tls")]
use std::time::Instant;

use crate::auth::secret::{self, SecretStore};
use crate::types::{iso_utc, lockv, now_ts, AppState, Server, Transport};

/// Compte du trousseau pour la clé privée TLS du serveur (`docs/auth.md` §9).
pub const ACCOUNT_TLS: &str = "server-tls";

/// En-tête `WWW-Authenticate` exact d'un serveur LiteMCP verrouillé (`litemcp/docs/auth.md` §2.7).
pub const LITEMCP_WWW_AUTH: &str = "Signature realm=\"litemcp\", tag=\"litemcp-auth-v1\"";

/// Motif affiché quand un serveur LiteMCP répond 401 Signature sans être appairé (lot M4 pt 4).
pub const LOCKED_SERVER_MSG: &str =
    "serveur LiteMCP verrouillé (401 Signature) — appairer avec l'action locale server_pair";

/// Message affiché dans le journal et l'interface quand `ui_bind` hors loopback
/// est refusé faute de feature Cargo `tls` (DA5 = a, lot M4 pt 1).
pub const TLS_MISSING_BIND_MSG: &str =
    "Écoute réseau hors loopback refusée : binaire compilé sans la feature `tls` (DA5). Le moteur reste en 127.0.0.1.";

/// La feature Cargo `tls` est-elle compilée dans ce binaire ?
#[must_use]
pub const fn tls_compiled() -> bool {
    cfg!(feature = "tls")
}

/// Reconnaît le défi `WWW-Authenticate` d'un serveur LiteMCP verrouillé
/// (`Signature realm="litemcp", tag="litemcp-auth-v1"`).
#[must_use]
pub fn is_litemcp_auth_challenge(www_auth: &str) -> bool {
    let t = www_auth.trim();
    let Some(rest) = t.get(..9) else {
        return false;
    };
    rest.eq_ignore_ascii_case("signature")
        && t.contains("realm=\"litemcp\"")
        && t.contains("tag=\"litemcp-auth-v1\"")
}

/// Vérifie la forme canonique d'une empreinte de certificat TLS :
/// `sha256:` + 64 caractères hexadécimaux minuscules (`docs/auth.md` §3.4 et §7).
#[must_use]
pub fn valid_tls_fp(fp: &str) -> bool {
    let Some(hex) = fp.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

// ---------- Dossier de configuration actif pour les sondes sortantes ----------

fn config_dir_slot() -> &'static Mutex<Option<PathBuf>> {
    static SLOT: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Enregistre le chemin du `config.json` actif pour que les sondes sortantes
/// (`probe_http`, `probe_peer`, `rpc_call_host`) trouvent `auth/clients/`.
pub fn set_config_path(config_path: &str) {
    let trimmed = config_path.trim();
    if trimmed.is_empty() {
        return;
    }
    if let Some(parent) = Path::new(trimmed).parent() {
        if !parent.as_os_str().is_empty() {
            *lockv(config_dir_slot()) = Some(parent.to_path_buf());
        }
    }
}

/// Dossier de configuration courant (`MCC_CONFIG` prioritaire s'il est posé,
/// sinon le dernier chemin enregistré par le moteur).
#[must_use]
pub fn current_config_dir() -> PathBuf {
    if let Ok(env_cfg) = std::env::var("MCC_CONFIG") {
        let t = env_cfg.trim();
        if !t.is_empty() {
            if let Some(p) = Path::new(t).parent() {
                if !p.as_os_str().is_empty() {
                    return p.to_path_buf();
                }
            }
        }
    }
    if let Some(dir) = lockv(config_dir_slot()).clone() {
        return dir;
    }
    let resolved = crate::manage::resolve_config_path(
        env!("CARGO_MANIFEST_DIR"),
        std::env::var("MCC_CONFIG").ok(),
    );
    Path::new(&resolved)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Chemin du certificat TLS auto-signé persisté : `<config_dir>/tls/server.crt`.
#[must_use]
pub fn tls_cert_path(config_dir: &Path) -> PathBuf {
    config_dir.join("tls").join("server.crt")
}

/// Chemin du repli fichier de la clé privée TLS : `<config_dir>/tls/server.key`.
#[must_use]
pub fn tls_key_path(config_dir: &Path) -> PathBuf {
    config_dir.join("tls").join("server.key")
}

/// Dossier des identités clientes sortantes : `<config_dir>/auth/clients`.
#[must_use]
pub fn clients_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("auth").join("clients")
}

#[cfg(feature = "tls")]
fn decode_der_hex(hex: &str) -> Result<Vec<u8>, String> {
    let hex = hex.trim();
    if hex.len() < 64
        || !hex.len().is_multiple_of(2)
        || !hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("matériel TLS illisible (hexadécimal minuscule attendu)".to_string());
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    for i in 0..(hex.len() / 2) {
        let pair = hex.get(i * 2..i * 2 + 2).ok_or("matériel TLS tronqué")?;
        let b = u8::from_str_radix(pair, 16).map_err(|_| "octet TLS invalide")?;
        out.push(b);
    }
    Ok(out)
}

#[cfg(feature = "tls")]
fn read_optional_file(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("lecture {} impossible : {}", path.display(), e)),
    }
}

// ---------- Serveur TLS 1.3 (feature `tls`) ----------

#[cfg(feature = "tls")]
pub struct LoadedServerTls {
    pub server_config: Arc<rustls::ServerConfig>,
    pub cert_der: Vec<u8>,
    pub fingerprint: String,
    pub store: SecretStore,
    pub created: bool,
}

/// Construit une configuration `rustls::ServerConfig` **TLS 1.3 uniquement**,
/// trois suites de `ring`, ALPN `http/1.1`, sans authentification de client TLS.
#[cfg(feature = "tls")]
pub fn build_rustls_server_config(
    cert_der: &[u8],
    key_der: &[u8],
) -> Result<Arc<rustls::ServerConfig>, String> {
    // Valide d'abord par `litemcp::tls::TlsConfig` (contrat C10).
    let _ = litemcp::tls::TlsConfig::from_der(cert_der.to_vec(), key_der.to_vec())
        .map_err(|e| format!("configuration TLS refusée : {}", e.reason()))?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|_| "version TLS 1.3 impossible".to_string())?
        .with_no_client_auth()
        .with_single_cert(
            vec![rustls::pki_types::CertificateDer::from(cert_der.to_vec())],
            rustls::pki_types::PrivateKeyDer::Pkcs8(key_der.to_vec().into()),
        )
        .map_err(|_| "certificat ou clé privée TLS incompatible".to_string())?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config.session_storage = rustls::server::ServerSessionMemoryCache::new(256);
    Ok(Arc::new(config))
}

/// Charge ou génère le certificat auto-signé TLS 1.3 et sa clé privée
/// (`docs/auth.md` §7 et §9).
///
/// - Clé privée : trousseau du système (compte `server-tls`), repli fichier
///   `<config_dir>/tls/server.key` à permissions restreintes (0600 / ACL).
/// - Certificat : `<config_dir>/tls/server.crt`.
/// - Échec fermé : un fichier corrompu ou désaccordé avec le certificat n'est
///   **jamais** écrasé silencieusement (il faut supprimer les deux fichiers
///   pour révoquer et régénérer, §7).
#[cfg(feature = "tls")]
pub fn load_or_create_server_tls(
    dir: &Path,
    service: Option<&str>,
    extra_sans: &[String],
) -> Result<LoadedServerTls, String> {
    let cert_file = tls_cert_path(dir);
    let key_file = tls_key_path(dir);
    let cert_opt = read_optional_file(&cert_file)?;
    let key_file_opt = read_optional_file(&key_file)?;
    let key_ring_opt = match service {
        Some(svc) => secret::load_client_key(Some(svc), ACCOUNT_TLS)
            .ok()
            .flatten(),
        None => None,
    };

    if let Some(cert_hex) = cert_opt {
        let cert_der = decode_der_hex(&cert_hex)?;
        let (key_hex, store) = if let Some(k) = key_ring_opt {
            (k, SecretStore::Keyring)
        } else if let Some(k) = key_file_opt {
            (k, SecretStore::File)
        } else {
            return Err(
                "certificat tls/server.crt présent mais clé privée introuvable (trousseau et tls/server.key)"
                    .to_string(),
            );
        };
        let key_der = decode_der_hex(&key_hex)?;
        let server_config = build_rustls_server_config(&cert_der, &key_der)?;
        let fingerprint = litemcp::tls::CertificateFingerprint::of_der(&cert_der).to_hex();
        return Ok(LoadedServerTls {
            server_config,
            cert_der,
            fingerprint,
            store,
            created: false,
        });
    }

    // Si `tls/server.key` existe seul sur disque sans `tls/server.crt`, on refuse
    // de l'écraser silencieusement (§7 : supprimer les deux fichiers pour régénérer).
    if key_file_opt.is_some() {
        return Err(
            "tls/server.key présent sans tls/server.crt — supprimer les deux fichiers pour régénérer"
                .to_string(),
        );
    }

    let mut sans: Vec<String> = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];
    for ip in extra_sans {
        let t = ip.trim();
        if !t.is_empty() && !sans.iter().any(|s| s == t) {
            sans.push(t.to_string());
        }
    }
    let keypair =
        rcgen::KeyPair::generate().map_err(|_| "génération clé TLS impossible".to_string())?;
    let mut params = rcgen::CertificateParams::new(sans)
        .map_err(|_| "paramètres SAN du certificat TLS invalides".to_string())?;
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "mcp-control");
    let cert = params
        .self_signed(&keypair)
        .map_err(|_| "signature du certificat TLS auto-signé impossible".to_string())?;
    let cert_der = cert.der().to_vec();
    let key_der = keypair.serialize_der();
    let server_config = build_rustls_server_config(&cert_der, &key_der)?;
    let fingerprint = litemcp::tls::CertificateFingerprint::of_der(&cert_der).to_hex();

    let cert_hex = crate::crypto::hex(&cert_der);
    let key_hex = crate::crypto::hex(&key_der);

    let mut store = SecretStore::File;
    if let Some(svc) = service {
        if secret::store_client_key(svc, ACCOUNT_TLS, &key_hex).is_ok() {
            store = SecretStore::Keyring;
        }
    }
    if store == SecretStore::File {
        secret::write_secret_file(&key_file, &key_hex)?;
    }
    secret::write_secret_file(&cert_file, &cert_hex)?;

    Ok(LoadedServerTls {
        server_config,
        cert_der,
        fingerprint,
        store,
        created: true,
    })
}

/// Termine la poignée de main TLS 1.3 côté serveur avant la lecture HTTP.
/// Tout client TLS ≤ 1.2, HTTP en clair ou ALPN incompatible échoue ici.
#[cfg(feature = "tls")]
pub fn accept_tls_stream(
    mut stream: TcpStream,
    config: &Arc<rustls::ServerConfig>,
    deadline: Instant,
) -> Result<rustls::StreamOwned<rustls::ServerConnection, TcpStream>, String> {
    let mut conn = rustls::ServerConnection::new(Arc::clone(config))
        .map_err(|e| format!(" session TLS : {e}"))?;
    while conn.is_handshaking() {
        let remain = deadline.saturating_duration_since(Instant::now());
        if remain.is_zero() {
            return Err("échéance dépassée pendant la poignée de main TLS".to_string());
        }
        let _ = stream.set_read_timeout(Some(remain));
        let _ = stream.set_write_timeout(Some(remain));
        if let Err(e) = conn.complete_io(&mut stream) {
            // Tenter d'écrire l'alerte TLS déjà préparée par rustls avant de fermer.
            let _ = conn.write_tls(&mut stream);
            let _ = stream.flush();
            return Err(format!("poignée de main TLS 1.3 refusée : {e}"));
        }
    }
    Ok(rustls::StreamOwned::new(conn, stream))
}

// ---------- Client TLS 1.3 et épinglage `PinnedServer` (feature `tls`) ----------

#[cfg(feature = "tls")]
#[derive(Debug)]
struct PinnedCertVerifier {
    expected: Option<PinnedServer>,
    observed: Arc<Mutex<Option<String>>>,
}

#[cfg(feature = "tls")]
impl rustls::client::danger::ServerCertVerifier for PinnedCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let fp = litemcp::tls::CertificateFingerprint::of_der(end_entity.as_ref()).to_hex();
        *lockv(&self.observed) = Some(fp.clone());
        if let Some(pinned) = &self.expected {
            if pinned.verify(&fp).is_err() {
                return Err(rustls::Error::General(
                    "empreinte du certificat TLS différente de l'épinglage (PinnedServer)".into(),
                ));
            }
        }
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        // TLS 1.2 est strictement interdit (docs/auth.md §7).
        Err(rustls::Error::PeerIncompatible(
            rustls::PeerIncompatible::Tls12NotOfferedOrEnabled,
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(feature = "tls")]
fn server_name_for(host: &str) -> Result<rustls::pki_types::ServerName<'static>, String> {
    let clean = host.trim().trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = clean.parse::<std::net::IpAddr>() {
        return Ok(rustls::pki_types::ServerName::IpAddress(ip.into()));
    }
    rustls::pki_types::ServerName::try_from(clean.to_string())
        .map_err(|_| format!("nom d'hôte TLS invalide : {host}"))
}

/// Exécute une requête HTTP/1.1 sur une connexion **TLS 1.3** (`ALPN http/1.1`).
///
/// - Si `expected_tls_fp` est `Some(fp)`, le certificat présenté par le serveur
///   est vérifié en temps constant par [`PinnedServer::verify`] pendant la
///   poignée de main : aucun octet HTTP n'est émis si l'empreinte diffère.
/// - Renvoie `(octets_reponse_http, empreinte_tls_observee)`.
#[cfg(feature = "tls")]
pub fn https_exchange_raw(
    host: &str,
    port: u16,
    raw_request: &[u8],
    timeout: Duration,
    expected_tls_fp: Option<&str>,
    max_read: usize,
) -> Result<(Vec<u8>, String), String> {
    let expected = match expected_tls_fp.map(str::trim).filter(|s| !s.is_empty()) {
        Some(fp) => {
            let p = PinnedServer::new(fp)
                .map_err(|_| format!("empreinte TLS épinglée mal formée : {fp}"))?;
            Some(p)
        }
        None => None,
    };
    let clean_host = host.trim().trim_start_matches('[').trim_end_matches(']');
    let addr: SocketAddr = (clean_host, port)
        .to_socket_addrs()
        .map_err(|e| format!("résolution {host}:{port} impossible : {e}"))?
        .next()
        .ok_or_else(|| format!("aucune adresse pour {host}:{port}"))?;
    let observed = Arc::new(Mutex::new(None));
    let verifier = Arc::new(PinnedCertVerifier {
        expected,
        observed: Arc::clone(&observed),
    });
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut client_cfg = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|_| "configuration client TLS 1.3 impossible".to_string())?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    client_cfg.alpn_protocols = vec![b"http/1.1".to_vec()];

    let sname = server_name_for(host)?;
    let mut conn = rustls::ClientConnection::new(Arc::new(client_cfg), sname)
        .map_err(|e| format!("session client TLS : {e}"))?;

    let mut tcp = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|e| format!("connexion {host}:{port} impossible : {e}"))?;
    let _ = tcp.set_read_timeout(Some(timeout));
    let _ = tcp.set_write_timeout(Some(timeout));

    // Si le serveur a déjà rejeté la connexion en clair avant la poignée de
    // main (503 d'admission TCP), lire ce refus sans émettre ClientHello
    // (sinon l'envoi sur socket fermée provoque un RST / 10053 sous Windows).
    let _ = tcp.set_nonblocking(true);
    let mut pre = [0u8; 4096];
    let pre_n = tcp.peek(&mut pre).unwrap_or(0);
    let _ = tcp.set_nonblocking(false);
    if pre_n >= 9 && pre[..pre_n].starts_with(b"HTTP/1.1 ") {
        let mut raw = Vec::new();
        let _ = tcp.set_read_timeout(Some(Duration::from_millis(200)));
        let mut chunk = [0u8; 4096];
        while raw.len() < 4096 {
            match tcp.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => raw.extend_from_slice(&chunk[..n]),
            }
        }
        return Err(format!(
            "{host}:{port} : refus pré-TLS : {}",
            String::from_utf8_lossy(&raw)
        ));
    }

    struct HandshakeTap<'a> {
        tcp: &'a mut TcpStream,
        captured: Vec<u8>,
    }
    impl Read for HandshakeTap<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.tcp.read(buf)?;
            if n > 0 && self.captured.len() < 4096 {
                let take = n.min(4096 - self.captured.len());
                self.captured.extend_from_slice(&buf[..take]);
            }
            Ok(n)
        }
    }
    impl Write for HandshakeTap<'_> {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.tcp.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.tcp.flush()
        }
    }

    let deadline = Instant::now() + timeout;
    let mut captured_head = Vec::new();
    while conn.is_handshaking() {
        let remain = deadline.saturating_duration_since(Instant::now());
        if remain.is_zero() {
            return Err(format!(
                "{host}:{port} : échéance poignée de main TLS dépassée"
            ));
        }
        let _ = tcp.set_read_timeout(Some(remain));
        let _ = tcp.set_write_timeout(Some(remain));
        let mut tap = HandshakeTap {
            tcp: &mut tcp,
            captured: std::mem::take(&mut captured_head),
        };
        let io_res = conn.complete_io(&mut tap);
        captured_head = tap.captured;
        if let Err(e) = io_res {
            if captured_head.starts_with(b"HTTP/1.1 ") {
                return Err(format!(
                    "{host}:{port} : refus pré-TLS : {}",
                    String::from_utf8_lossy(&captured_head)
                ));
            }
            return Err(format!("{host}:{port} : refus TLS 1.3 : {e}"));
        }
    }

    let observed_fp = lockv(&observed).clone().ok_or_else(|| {
        "aucun certificat serveur reçu pendant la poignée de main TLS".to_string()
    })?;
    if let Some(fp) = expected_tls_fp.map(str::trim).filter(|s| !s.is_empty()) {
        let p = PinnedServer::new(fp)
            .map_err(|_| format!("empreinte TLS épinglée mal formée : {fp}"))?;
        p.verify(&observed_fp)
            .map_err(|_| format!("certificat TLS de {host}:{port} substitué ou modifié (attendu {fp}, observé {observed_fp})"))?;
    }

    let mut tls_stream = rustls::StreamOwned::new(conn, tcp);
    tls_stream
        .write_all(raw_request)
        .map_err(|e| format!("{host}:{port} write TLS : {e}"))?;
    let _ = tls_stream.flush();

    let mut data = Vec::new();
    let mut tmp = [0u8; 16384];
    let mut header_len: Option<usize> = None;
    loop {
        let remain = deadline.saturating_duration_since(Instant::now());
        if remain.is_zero() {
            break;
        }
        if header_len.is_none() {
            if let Some(p) = crate::find_sub(&data, b"\r\n\r\n") {
                header_len = Some(p + 4);
            }
        }
        if let Some(hl) = header_len {
            let head = String::from_utf8_lossy(&data[..hl]).to_lowercase();
            let body_bytes = &data[hl..];
            let complete = if head.contains("transfer-encoding: chunked") {
                crate::find_sub(body_bytes, b"\r\n0\r\n").is_some()
            } else if let Some(cl) = head.lines().find_map(|l| {
                l.strip_prefix("content-length:")
                    .map(|v| v.trim().to_string())
            }) {
                body_bytes.len() >= cl.parse::<usize>().unwrap_or(usize::MAX)
            } else {
                false
            };
            if complete {
                break;
            }
        }
        let _ = tls_stream.sock.set_read_timeout(Some(remain));
        match tls_stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                data.extend_from_slice(&tmp[..n]);
                if data.len() > max_read {
                    break;
                }
            }
            Err(e) => {
                if data.is_empty() {
                    return Err(format!("{host}:{port} read TLS : {e}"));
                }
                break;
            }
        }
    }
    Ok((data, observed_fp))
}

#[cfg(not(feature = "tls"))]
pub fn https_exchange_raw(
    host: &str,
    port: u16,
    _raw_request: &[u8],
    _timeout: Duration,
    _expected_tls_fp: Option<&str>,
    _max_read: usize,
) -> Result<(Vec<u8>, String), String> {
    Err(format!(
        "connexion TLS vers {host}:{port} impossible : binaire compilé sans la feature `tls`"
    ))
}

// ---------- Identités clientes sortantes (pairs et serveurs LiteMCP verrouillés) ----------

/// Identité cliente épinglée pour un serveur distant (pair MCP Control ou
/// serveur LiteMCP verrouillé).
pub struct OutboundIdentity {
    pub keyid: String,
    pub signer: Signer,
    pub server_pub: String,
    pub tls_fp: String,
    pub pinned: Option<PinnedServer>,
}

impl OutboundIdentity {
    /// Signe une requête `POST` selon le profil RFC 9421 + RFC 9530 de `litemcp`.
    #[must_use]
    pub fn sign_headers(
        &self,
        method: &str,
        path: &str,
        authority: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Vec<(String, String)> {
        let req = RequestToSign::new(method, path, authority, headers, body);
        let signed = self.signer.headers(&req);
        signed
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }
}

fn normalize_host(h: &str) -> String {
    let clean = h.trim().trim_start_matches('[').trim_end_matches(']');
    if crate::is_loopback_bind(clean) || crate::is_loopback_bind(h.trim()) {
        "127.0.0.1".to_string()
    } else {
        clean.to_ascii_lowercase()
    }
}

fn pin_matches_host_port(v: &Value, host: &str, port: u16) -> bool {
    let target_norm = normalize_host(host);
    if let (Some(h), Some(p)) = (
        v.get("host").and_then(|x| x.as_str()),
        v.get("port").and_then(|x| x.as_u64()),
    ) {
        if p == u64::from(port) && normalize_host(h) == target_norm {
            return true;
        }
    }
    if let Some(url) = v.get("url").and_then(|x| x.as_str()) {
        let rest = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))
            .unwrap_or("")
            .trim_end_matches('/');
        if let Some((h, p_str)) = rest.rsplit_once(':') {
            if let Ok(p) = p_str.parse::<u16>() {
                if p == port && normalize_host(h) == target_norm {
                    return true;
                }
            }
        }
    }
    false
}

/// Liste les hôtes déjà épinglés dans `auth/clients/*.pin.json` (pour la
/// découverte de pairs appairés même hors cache ARP).
#[must_use]
pub fn pinned_peer_hosts(config_dir: &Path) -> Vec<String> {
    let dir = clients_dir(config_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut hosts = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".pin.json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let host = v
            .get("host")
            .and_then(|x| x.as_str())
            .map(String::from)
            .or_else(|| {
                let url = v.get("url")?.as_str()?;
                let rest = url
                    .strip_prefix("https://")
                    .or_else(|| url.strip_prefix("http://"))?
                    .trim_end_matches('/');
                rest.rsplit_once(':').map(|(h, _)| h.to_string())
            });
        if let Some(h) = host {
            let h = h.trim().to_string();
            if !h.is_empty() && !hosts.contains(&h) {
                hosts.push(h);
            }
        }
    }
    hosts
}

/// Retrouve le fichier `<keyid>.pin.json` le plus récent pour `(host, port)`.
#[must_use]
pub fn find_existing_pin(config_dir: &Path, host: &str, port: u16) -> Option<Value> {
    let dir = clients_dir(config_dir);
    let entries = std::fs::read_dir(&dir).ok()?;
    let mut best: Option<(String, std::time::SystemTime, Value)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".pin.json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if !pin_matches_host_port(&v, host, port) {
            continue;
        }
        let paired_at = v
            .get("paired_at")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        let replace = match &best {
            None => true,
            Some((prev_ts, prev_mt, _)) => {
                paired_at > *prev_ts || (paired_at == *prev_ts && mtime > *prev_mt)
            }
        };
        if replace {
            best = Some((paired_at, mtime, v));
        }
    }
    best.map(|(_, _, v)| v)
}

/// Vérifie qu'un serveur déjà épinglé pour `(host, port)` n'a changé ni de
/// clé publique Ed25519 (`server_pub`), ni d'empreinte de certificat TLS
/// (`tls_fp`), et n'a pas subi de repli de TLS vers HTTP (`docs/auth.md` §3.4 et §7).
pub fn verify_no_identity_change(
    config_dir: &Path,
    host: &str,
    port: u16,
    new_server_pub: &str,
    new_tls_fp: &str,
) -> Result<(), String> {
    let Some(existing) = find_existing_pin(config_dir, host, port) else {
        return Ok(());
    };
    let old_pub = existing
        .get("server_pub")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim();
    let old_tls = existing
        .get("tls_fp")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim();
    if !old_pub.is_empty()
        && !new_server_pub.is_empty()
        && !crate::crypto::ct_eq(old_pub.as_bytes(), new_server_pub.as_bytes())
    {
        return Err(format!(
            "l'identité Ed25519 du serveur {host}:{port} a CHANGÉ par rapport à l'épinglage existant — refus explicite (pas de contournement silencieux)"
        ));
    }
    if !old_tls.is_empty() {
        if new_tls_fp.trim().is_empty() {
            return Err(format!(
                "le serveur {host}:{port} avait une empreinte TLS épinglée ({old_tls}) et répond maintenant sans TLS — repli refusé"
            ));
        }
        let pinned = PinnedServer::new(old_tls).map_err(|_| {
            format!("empreinte TLS épinglée mal formée pour {host}:{port} : {old_tls}")
        })?;
        if pinned.verify(new_tls_fp.trim()).is_err() {
            return Err(format!(
                "le certificat TLS du serveur {host}:{port} a CHANGÉ par rapport à l'épinglage existant (attendu {old_tls}, reçu {new_tls_fp}) — refus explicite"
            ));
        }
    }
    Ok(())
}

/// Charge l'identité cliente sortante (`Signer` + `PinnedServer`) pour `(host, port)`
/// depuis `<config_dir>/auth/clients/`.
pub fn outbound_identity_for_dir(
    config_dir: &Path,
    host: &str,
    port: u16,
) -> Result<Option<OutboundIdentity>, String> {
    let Some(pin) = find_existing_pin(config_dir, host, port) else {
        return Ok(None);
    };
    let keyid = pin
        .get("keyid")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if !crate::auth::registry::valid_keyid(&keyid) {
        return Err(format!(
            "épinglage illisible pour {host}:{port} (keyid invalide)"
        ));
    }
    let tls_fp = pin
        .get("tls_fp")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let pinned = if tls_fp.is_empty() {
        None
    } else {
        let p = PinnedServer::new(&tls_fp)
            .map_err(|_| format!("empreinte TLS épinglée invalide pour {host}:{port}"))?;
        Some(p)
    };
    let server_pub = pin
        .get("server_pub")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let svc = secret::service();
    let mut candidates = vec![format!("client-{host}:{port}")];
    let norm = normalize_host(host);
    let norm_acct = format!("client-{norm}:{port}");
    if !candidates.contains(&norm_acct) {
        candidates.push(norm_acct);
    }
    let mut block: Option<String> = None;
    for acct in &candidates {
        if let Ok(Some(b)) = secret::load_client_key(svc.as_deref(), acct) {
            block = Some(b);
            break;
        }
    }
    let block = match block {
        Some(b) => b,
        None => {
            let key_path = clients_dir(config_dir).join(format!("{keyid}.key"));
            std::fs::read_to_string(&key_path).map_err(|_| {
                format!(
                    "clé privée cliente {keyid} introuvable pour {host}:{port} (trousseau et {})",
                    key_path.display()
                )
            })?
        }
    };
    let key = SigningKey::import(&block)
        .map_err(|_| format!("clé privée cliente {keyid} illisible pour {host}:{port}"))?;
    Ok(Some(OutboundIdentity {
        keyid: keyid.clone(),
        signer: Signer::new(key).with_keyid(keyid),
        server_pub,
        tls_fp,
        pinned,
    }))
}

/// Charge l'identité cliente sortante pour `(host, port)` dans le dossier de
/// configuration actif.
pub fn outbound_identity_for(host: &str, port: u16) -> Result<Option<OutboundIdentity>, String> {
    outbound_identity_for_dir(&current_config_dir(), host, port)
}

/// Un serveur supervisé est-il verrouillé ou appairé par signature LiteMCP ?
///
/// Vrai dans deux cas (badge cadenas sur UP/DOWN) :
/// 1. le serveur a répondu `401` + `WWW-Authenticate` de `litemcp` (son motif
///    d'erreur ou l'ensemble `locked_servers` le signale) ;
/// 2. MCP Control détient un épinglage client sortant (`auth/clients/*.pin.json`)
///    pour l'hôte et le port de ce serveur.
#[must_use]
pub fn is_server_locked(s: &Server) -> bool {
    if s.last_error
        .as_deref()
        .is_some_and(|e| e.contains("verrouillé (401 Signature)"))
    {
        return true;
    }
    if let Transport::Http { host, port, .. } = &s.transport {
        let h = s
            .bound_host
            .as_deref()
            .or(host.as_deref())
            .unwrap_or("127.0.0.1");
        if find_existing_pin(&current_config_dir(), h, *port).is_some() {
            return true;
        }
    }
    false
}

// ---------- Action locale `server_pair` (droit `admin`, lot M4 point 5) ----------

/// Action locale `server_pair` (droit `admin`, exclue du gateway) : génère une
/// paire de clés Ed25519 cliente pour un serveur LiteMCP verrouillé, range la
/// clé privée au trousseau (`client-<hôte>:<port>`) ou en repli fichier 0600
/// (`auth/clients/<keyid>.key`), et écrit l'épinglage `auth/clients/<keyid>.pin.json`.
pub fn handle_server_pair(state: &Arc<AppState>, body: &str) -> Value {
    set_config_path(&state.config_path);
    let req: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return json!({"ok": false, "error": format!("body invalide: {e}")}),
    };

    let id_param = req
        .get("id")
        .or_else(|| req.get("server"))
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);

    let mut target_host: Option<String> = req
        .get("host")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    let mut target_port: Option<u16> = req
        .get("port")
        .and_then(|x| x.as_u64())
        .filter(|p| (1..=65535).contains(p))
        .map(|p| p as u16);

    if let Some(url) = req.get("url").and_then(|x| x.as_str()) {
        match crate::auth::cli::parse_url_scheme(url) {
            Ok((_https, h, p)) => {
                target_host = Some(h);
                target_port = Some(p);
            }
            Err(e) => return json!({"ok": false, "error": e}),
        }
    }

    let mut resolved_id: Option<String> = None;
    if let Some(ref sid) = id_param {
        let servers = lockv(&state.servers);
        let Some(srv) = servers.iter().find(|s| s.id == *sid) else {
            return json!({"ok": false, "error": format!("serveur inconnu : {sid}")});
        };
        match &srv.transport {
            Transport::Http { host, port, .. } => {
                let h = srv
                    .bound_host
                    .clone()
                    .or_else(|| host.clone())
                    .unwrap_or_else(|| "127.0.0.1".to_string());
                target_host.get_or_insert(h);
                target_port.get_or_insert(*port);
                resolved_id = Some(srv.id.clone());
            }
            Transport::Stdio { .. } => {
                return json!({
                    "ok": false,
                    "error": "server_pair ne s'applique qu'aux serveurs HTTP"
                });
            }
        }
    }

    let (Some(host), Some(port)) = (target_host, target_port) else {
        return json!({
            "ok": false,
            "error": "cible manquante : fournir `id` (serveur HTTP connu) ou `host` + `port`"
        });
    };

    if resolved_id.is_none() {
        let norm = normalize_host(&host);
        let servers = lockv(&state.servers);
        resolved_id = servers.iter().find_map(|s| match &s.transport {
            Transport::Http {
                host: sh, port: sp, ..
            } if *sp == port => {
                let h = s
                    .bound_host
                    .as_deref()
                    .or(sh.as_deref())
                    .unwrap_or("127.0.0.1");
                if normalize_host(h) == norm {
                    Some(s.id.clone())
                } else {
                    None
                }
            }
            _ => None,
        });
    }

    let tls_fp = req
        .get("tls_fp")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if !tls_fp.is_empty() && !valid_tls_fp(&tls_fp) {
        return json!({
            "ok": false,
            "error": "empreinte tls_fp invalide (attendu sha256:<64 hex minuscules>)"
        });
    }

    let server_pub = req
        .get("server_pub")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if !server_pub.is_empty() && crate::auth::registry::decode_public_key(&server_pub).is_none() {
        return json!({
            "ok": false,
            "error": "server_pub invalide (attendu 64 hex minuscules Ed25519)"
        });
    }

    let config_dir = if state.config_path.trim().is_empty() {
        current_config_dir()
    } else {
        Path::new(&state.config_path)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(current_config_dir)
    };

    if let Err(e) = verify_no_identity_change(&config_dir, &host, port, &server_pub, &tls_fp) {
        return json!({"ok": false, "error": e});
    }

    let keyid = match req
        .get("keyid")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(k) => {
            if !crate::auth::registry::valid_keyid(k) {
                return json!({
                    "ok": false,
                    "error": "keyid hors grammaire (attendu mcc-<préfixe>-<16 hex>)"
                });
            }
            k.to_string()
        }
        None => match crate::random_hex(1) {
            Some(hex) => format!("mcc-srv-{hex}"),
            None => {
                return json!({
                    "ok": false,
                    "error": "entropie OS indisponible"
                });
            }
        },
    };

    let cle = SigningKey::generate();
    let vk = cle.verifying_key();
    let public_key = crate::crypto::hex(&vk.to_bytes());
    let public_key_block = vk.export();
    let fingerprint = vk.fingerprint().full();
    let fingerprint_court = vk.fingerprint().short();

    let compte = format!("client-{host}:{port}");
    let bloc_prive = cle.export();
    let mut store = SecretStore::File;
    if let Some(svc) = secret::service() {
        if secret::store_client_key(&svc, &compte, &bloc_prive).is_ok() {
            store = SecretStore::Keyring;
        }
    }
    let dir_clients = clients_dir(&config_dir);
    if store == SecretStore::File {
        let key_path = dir_clients.join(format!("{keyid}.key"));
        if let Err(e) = secret::write_secret_file(&key_path, &bloc_prive) {
            return json!({
                "ok": false,
                "error": format!("enregistrement de la clé privée impossible : {e}")
            });
        }
    }

    let scheme = if tls_fp.is_empty() { "http" } else { "https" };
    let pin = json!({
        "url": format!("{scheme}://{host}:{port}"),
        "host": host,
        "port": port,
        "server_id": resolved_id,
        "server_keyid": req.get("server_keyid").and_then(|x| x.as_str()).unwrap_or("litemcp-server"),
        "server_pub": server_pub,
        "tls_fp": tls_fp,
        "keyid": keyid,
        "public_key": public_key,
        "paired_at": iso_utc(now_ts()),
    });
    let pin_path = dir_clients.join(format!("{keyid}.pin.json"));
    let pin_text = serde_json::to_string_pretty(&pin).unwrap_or_default();
    if let Err(e) = secret::write_secret_file(&pin_path, &pin_text) {
        return json!({
            "ok": false,
            "error": format!("écriture de l'épinglage impossible : {e}")
        });
    }

    crate::manage::add_log(
        state,
        "info",
        &format!(
            "[auth] server_pair : clé cliente {} ({}) créée pour {}:{} — stockage : {}",
            keyid,
            fingerprint_court,
            host,
            port,
            store.as_str()
        ),
    );

    json!({
        "ok": true,
        "id": resolved_id,
        "host": host,
        "port": port,
        "keyid": keyid,
        "public_key": public_key,
        "public_key_block": public_key_block,
        "fingerprint": fingerprint,
        "fingerprint_court": fingerprint_court,
        "tls_fp": tls_fp,
        "store": store.as_str(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "mcc-auth-net-{}-{}-{}",
            std::process::id(),
            now_ts(),
            tag
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reconnaissance_defi_www_authenticate_litemcp() {
        assert!(is_litemcp_auth_challenge(LITEMCP_WWW_AUTH));
        assert!(is_litemcp_auth_challenge(
            "signature realm=\"litemcp\", tag=\"litemcp-auth-v1\""
        ));
        assert!(!is_litemcp_auth_challenge("Bearer realm=\"litemcp\""));
        assert!(!is_litemcp_auth_challenge("Signature realm=\"autre\""));
        assert!(!is_litemcp_auth_challenge(""));
    }

    #[test]
    fn validation_empreinte_tls_sha256() {
        let ok = format!("sha256:{}", "a0".repeat(32));
        assert!(valid_tls_fp(&ok));
        assert!(!valid_tls_fp("sha256:1234"));
        assert!(!valid_tls_fp(&format!("sha256:{}", "A0".repeat(32))));
        assert!(!valid_tls_fp(&"a0".repeat(32)));
    }

    #[test]
    fn changement_identite_ou_certificat_refuse_sans_contournement() {
        let dir = tmp_dir("pin-change");
        let clients = clients_dir(&dir);
        std::fs::create_dir_all(&clients).unwrap();
        let pub1 = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
        let pub2 = "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c";
        let fp1 = format!("sha256:{}", "11".repeat(32));
        let fp2 = format!("sha256:{}", "22".repeat(32));
        let pin = json!({
            "url": "https://192.0.2.50:5504",
            "host": "192.0.2.50",
            "port": 5504,
            "server_pub": pub1,
            "tls_fp": fp1,
            "keyid": "mcc-test-0000000000000001",
            "paired_at": "2026-11-20 12:00:00"
        });
        std::fs::write(
            clients.join("mcc-test-0000000000000001.pin.json"),
            serde_json::to_string(&pin).unwrap(),
        )
        .unwrap();

        // Même identité et même certificat : accepté.
        assert!(verify_no_identity_change(&dir, "192.0.2.50", 5504, pub1, &fp1).is_ok());
        // Clé publique Ed25519 changée : refus explicite.
        let err_pub = verify_no_identity_change(&dir, "192.0.2.50", 5504, pub2, &fp1).unwrap_err();
        assert!(err_pub.contains("CHANGÉ"), "{err_pub}");
        // Certificat TLS substitué : refus explicite.
        let err_tls = verify_no_identity_change(&dir, "192.0.2.50", 5504, pub1, &fp2).unwrap_err();
        assert!(err_tls.contains("CHANGÉ"), "{err_tls}");
        // Repli de TLS vers HTTP (tls_fp vide) : refus explicite.
        let err_downgrade =
            verify_no_identity_change(&dir, "192.0.2.50", 5504, pub1, "").unwrap_err();
        assert!(err_downgrade.contains("sans TLS"), "{err_downgrade}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "tls")]
    #[test]
    fn certificat_tls_persiste_recharge_et_refuse_tls12_ou_cert_substitue() {
        let dir = tmp_dir("tls-persist");
        let extra = vec!["127.0.0.1".to_string()];
        let first = load_or_create_server_tls(&dir, None, &extra).expect("création TLS");
        assert!(first.created);
        assert_eq!(first.store, SecretStore::File);
        assert!(valid_tls_fp(&first.fingerprint), "{}", first.fingerprint);
        assert!(tls_cert_path(&dir).exists());
        assert!(tls_key_path(&dir).exists());

        // Redémarrage : même certificat, même empreinte.
        let second = load_or_create_server_tls(&dir, None, &extra).expect("relecture TLS");
        assert!(!second.created);
        assert_eq!(first.fingerprint, second.fingerprint);
        assert_eq!(first.cert_der, second.cert_der);

        // Serveur TLS 1.3 éphémère sur socket locale : test poignée de main.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let srv_cfg = Arc::clone(&first.server_config);
        let th = std::thread::spawn(move || {
            for _ in 0..3 {
                if let Ok((s, _)) = listener.accept() {
                    let cfg = Arc::clone(&srv_cfg);
                    if let Ok(mut tls) =
                        accept_tls_stream(s, &cfg, Instant::now() + Duration::from_secs(5))
                    {
                        let mut buf = [0u8; 512];
                        let _ = tls.read(&mut buf);
                        let _ = tls.write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                        );
                        let _ = tls.flush();
                        tls.conn.send_close_notify();
                        let _ = tls.conn.complete_io(&mut tls.sock);
                    }
                }
            }
        });

        // 1) Client TLS 1.3 avec la BONNE empreinte épinglée → 200 OK.
        let req = b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
        let (bytes, obs_fp) = https_exchange_raw(
            "127.0.0.1",
            port,
            req,
            Duration::from_secs(5),
            Some(&first.fingerprint),
            65536,
        )
        .expect("TLS 1.3 épinglé doit réussir");
        assert_eq!(obs_fp, first.fingerprint);
        assert!(String::from_utf8_lossy(&bytes).starts_with("HTTP/1.1 200"));

        // 2) Client TLS 1.3 avec une AUTRE empreinte épinglée (certificat substitué) → refus.
        let faux_fp = format!("sha256:{}", "ff".repeat(32));
        let err_pin = https_exchange_raw(
            "127.0.0.1",
            port,
            req,
            Duration::from_secs(5),
            Some(&faux_fp),
            65536,
        )
        .unwrap_err();
        assert!(
            err_pin.contains("PinnedServer") || err_pin.contains("refus TLS"),
            "attendu refus PinnedServer, reçu : {err_pin}"
        );

        // 3) Client TLS 1.2 (ClientHello 0x0303 sans TLS 1.3 dans supported_versions) → refus.
        let mut tcp12 = TcpStream::connect(("127.0.0.1", port)).unwrap();
        tcp12
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let hello_tls12: [u8; 52] = [
            0x16, 0x03, 0x01, 0x00, 0x2f, // Record Handshake (47 octets)
            0x01, 0x00, 0x00, 0x2b, // ClientHello (43 octets)
            0x03, 0x03, // Version TLS 1.2 (0x0303)
            // 32 octets de random
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c,
            0x1d, 0x1e, 0x1f, 0x20, 0x00, // Session ID len = 0
            0x00, 0x02, 0xc0, 0x2f, // Cipher Suites: TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256
            0x01, 0x00, // Compression methods: null
            0x00, 0x00, // Extensions len = 0 (donc TLS 1.2 max)
        ];
        tcp12.write_all(&hello_tls12).unwrap();
        let mut rep12 = Vec::new();
        let _ = tcp12.read_to_end(&mut rep12);
        // La réponse ne contient AUCUN HTTP (soit une alerte TLS 0x15, soit EOF).
        assert!(
            !String::from_utf8_lossy(&rep12).contains("HTTP/1.1"),
            "TLS 1.2 ne doit jamais négocier HTTP : {rep12:?}"
        );
        if !rep12.is_empty() {
            assert_eq!(
                rep12[0], 0x15,
                "alerte TLS (0x15) attendue sur ClientHello 1.2"
            );
        }

        let _ = th.join();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
