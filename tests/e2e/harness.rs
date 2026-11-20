//! Harnais e2e : lance le VRAI binaire `mcp-control` (CARGO_BIN_EXE) dans un
//! répertoire temporaire isolé et lui parle en HTTP brut — uniquement
//! std::net + serde_json, aucune dépendance externe.
//!
//! Isolation garantie :
//! - `MCC_CONFIG` pointe vers un config.json unique par moteur ; mcc.log et
//!   state-cache.json sont écrits à côté (même répertoire temporaire) ;
//! - `"autostart": false` — sinon le moteur écrit une VRAIE clé Run (HKCU) ;
//! - `"network_enabled": false` — sinon il scanne le LAN à +30 s ;
//! - `removed_seeds` liste les 8 seeds embarqués — sinon le premier cycle du
//!   poller AUTO-LANCE les vrais serveurs node de la machine (start_cmd) ;
//! - `MCC_API_ACCESS` sème le code d'accès quand le scénario en demande un.
//!
//! API partagée par tous les modules de scénarios (certains helpers ne servent
//! que scenarios_lifecycle / scenarios_scan) → allow(dead_code) assumé.

#![allow(dead_code)]

use litemcp::auth::sign::RequestToSign;
use litemcp::auth::sign::{Signer, SigningKey};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Compteur pour des répertoires temporaires uniques par process de test.
static NEXT_TMP: AtomicU64 = AtomicU64::new(0);

// ---------- sérialisation globale des scénarios ----------

/// Verrou global e2e : UN SEUL scénario actif à la fois, même quand
/// `cargo test --test e2e` tourne en parallèle (défaut de cargo). Chaque
/// scénario démarre bien SON moteur sur SON port, mais plusieurs scénarios
/// restent sensibles aux ressources partagées de la machine (netstat/scan
/// local, ARP, charge CPU, sonde de mise à jour…) — c'est la cause du flak
/// historique de `scenarios_boot::auth_matrix_lan` en mode parallèle.
/// Const-stable depuis Rust 1.63.
pub static E2E_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Prend le verrou de sérialisation pour toute la durée d'un scénario.
/// À appeler en PREMIÈRE ligne de chaque `#[test]` :
/// `let _garde = serial_guard();`. Tolère le poison (un scénario qui panique
/// en tenant le verrou ne doit pas faire échouer en cascade les suivants).
pub fn serial_guard() -> std::sync::MutexGuard<'static, ()> {
    match E2E_LOCK.lock() {
        Ok(g) => g,
        Err(poisonne) => poisonne.into_inner(),
    }
}

/// Les 8 seeds embarqués par `default_servers()` — chaque seed porte une vraie
/// `start_cmd` (node / hello.exe) + auto_restart : sans retrait explicite, le
/// poller du moteur les AUTO-LANCE au premier cycle. On les retire au chargement
/// (load_config filtre les seeds listés dans settings.removed_seeds).
pub const SEED_IDS: &[&str] = &[
    "seed-overmind",
    "seed-discord",
    "seed-postgresql",
    "seed-debats",
    "seed-x",
    "seed-news-btc",
    "seed-news",
    "seed-litemcp",
];

/// Configuration de démarrage d'un moteur de test.
pub struct EngineCfg {
    /// Port UI demandé (le moteur peut décaler de +1 à +9 si occupé —
    /// wait_ready détecte le port réel en sondant /api/hello).
    pub ui_port: u16,
    /// Bind UI : "127.0.0.1" (loopback) ou "0.0.0.0" (exposé LAN).
    pub ui_bind: &'static str,
    /// Code d'accès API (semé via config + env MCC_API_ACCESS). None = aucun.
    pub api_access: Option<String>,
}

impl Default for EngineCfg {
    fn default() -> Self {
        EngineCfg {
            ui_port: free_port(),
            ui_bind: "127.0.0.1",
            api_access: None,
        }
    }
}

/// Un moteur mcp-control vivant : process enfant + port réel détecté.
/// Drop = kill + wait (le tmp est CONSERVÉ : verrous de fichiers Windows).
pub struct Engine {
    child: Child,
    base_url: String,
    tmp: PathBuf,
    /// Code d'accès API passé au moteur (None si engine sans code).
    pub code: Option<String>,
    /// Port UI RÉEL (base ou base+offset après repli du moteur).
    pub port: u16,
    /// Bind demandé — conservé pour relancer le moteur (`restart`).
    bind: &'static str,
    /// Session locale (docs/auth.md §6) : le cookie obtenu en échangeant le
    /// jeton d'amorçage imprimé sur la console du moteur.
    session: Option<String>,
    /// Secret de session (S-1) : mémoire de la page, jamais cookie. Présenté
    /// dans `X-MCC-Session` sur chaque requête `/api/*` loopback.
    session_secret: Option<String>,
    /// Identité du harnais sur `/mcp` (paire Ed25519 appairée au démarrage,
    /// droits `lecture` + `pilotage` + `execution`). None = aucune identité.
    signer: Option<Arc<Signer>>,
    /// `keyid` de l'identité du harnais (vide tant qu'elle n'est pas appairée).
    keyid: String,
    /// Approbation automatique des demandes de confirmation (lot M3) — voir
    /// [`Engine::set_auto_approve`].
    auto_approve: bool,
}

/// Mode de signature d'une requête `/mcp`.
#[derive(Clone, Copy, PartialEq)]
pub enum SignMode {
    /// Signature automatique du client du harnais (loopback seulement).
    Auto,
    /// Aucune signature ajoutée (cas négatifs de la matrice).
    None,
}

/// Lance le binaire mcp-control avec une config isolée dans un tmp unique.
pub fn spawn(cfg: EngineCfg) -> std::io::Result<Engine> {
    let n = NEXT_TMP.fetch_add(1, Ordering::SeqCst);
    spawn_in(
        cfg,
        std::env::temp_dir().join(format!("mcc-e2e-{}-{}", std::process::id(), n)),
    )
}

/// Variante interne : répertoire de données imposé (utilisée par `restart` pour
/// repartir sur le MÊME `config.json`, registre d'authentification compris).
fn spawn_in(cfg: EngineCfg, tmp: PathBuf) -> std::io::Result<Engine> {
    std::fs::create_dir_all(&tmp)?;
    let cfg_path = tmp.join("config.json");

    // Config générée : les trois réglages SÛRS sont obligatoires —
    // autostart false (jamais de clé Run réelle), network_enabled false
    // (jamais de scan LAN à +30 s), refresh_secs 5 (plancher) pour des
    // cycles de sondage rapides. removed_seeds neutralise les seeds embarqués.
    // Un `data_dir` réutilisé (redémarrage) garde SON config.json : le registre
    // d'authentification doit survivre, c'est précisément ce qu'on vérifie.
    let doc = json!({
        "servers": [],
        "settings": {
            "refresh_secs": 5,
            "scan_from": 3000,
            "scan_to": 3000,
            "ui_port": cfg.ui_port,
            "ui_bind": cfg.ui_bind,
            "network_enabled": false,
            "network_ports": [5090, 5091],
            "api_access": cfg.api_access,
            "allowed_origins": [],
            "removed_seeds": SEED_IDS,
            "server_order": [],
            "autostart": false,
            "ssh_hosts": []
        }
    });
    if !cfg_path.exists() {
        std::fs::write(
            &cfg_path,
            serde_json::to_string_pretty(&doc).unwrap_or_default(),
        )?;
    }

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mcp-control"));
    // mcc.log + state-cache.json sont écrits À CÔTÉ du config.json → tmp isolé.
    cmd.env("MCC_CONFIG", &cfg_path);
    // Trousseau : les moteurs de test ne doivent JAMAIS écrire dans le
    // gestionnaire d'identifiants de l'utilisateur (une entrée par moteur, et
    // une identité de test à la place de la sienne). Valeur vide = repli
    // fichier 0600, le chemin qu'on veut précisément éprouver ici.
    cmd.env("MCC_KEYRING_SERVICE", "");
    match &cfg.api_access {
        Some(code) => {
            cmd.env("MCC_API_ACCESS", code);
        }
        // on NE hérite PAS d'un MCC_API_ACCESS du shell de l'opérateur
        None => {
            cmd.env_remove("MCC_API_ACCESS");
        }
    }
    // Durée du permis de confirmation (lot M3, `docs/auth.md` §5) : dix minutes
    // en production, quelques secondes pour éprouver l'expiration. Le test la
    // pose dans SON environnement (`MCC_E2E_CONFIRM_SECS`) — les scénarios E2E
    // sont sérialisés, aucun autre moteur ne la lit.
    match std::env::var("MCC_E2E_CONFIRM_SECS") {
        Ok(v) => {
            cmd.env("MCC_CONFIRM_SECS", v);
        }
        Err(_) => {
            cmd.env_remove("MCC_CONFIRM_SECS");
        }
    }
    // stdout/stderr vers des fichiers du tmp : débogage facile, aucun risque
    // de blocage pipe-buffer (on ne les lit jamais en temps réel).
    let out_log = std::fs::File::create(tmp.join("mcc-stdout.log"))?;
    let err_log = std::fs::File::create(tmp.join("mcc-stderr.log"))?;
    cmd.stdout(Stdio::from(out_log))
        .stderr(Stdio::from(err_log));
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW : aucune console ne clignote pendant les tests
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let child = cmd.spawn()?;
    Ok(Engine {
        child,
        base_url: format!("http://127.0.0.1:{}", cfg.ui_port),
        tmp,
        code: cfg.api_access,
        port: cfg.ui_port,
        bind: cfg.ui_bind,
        session: None,
        session_secret: None,
        signer: None,
        keyid: String::new(),
        auto_approve: true,
    })
}

impl Engine {
    /// Port demandé à la config (avant repli éventuel).
    pub fn tmp_dir(&self) -> &PathBuf {
        &self.tmp
    }

    /// Arrête ce moteur et le relance sur le MÊME répertoire de données (donc le
    /// même `config.json` : registre des clients, identité du serveur). Lot M1 :
    /// c'est ce qui prouve qu'un client appairé le reste après redémarrage.
    pub fn restart(&mut self) -> Result<(), String> {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let cfg = EngineCfg {
            ui_port: self.port,
            ui_bind: self.bind,
            api_access: self.code.clone(),
        };
        let neuf = spawn_in(cfg, self.tmp.clone()).map_err(|e| e.to_string())?;
        *self = neuf;
        self.wait_ready(Duration::from_secs(30))
    }

    /// Attend que le moteur réponde GET /api/hello → 200 avec un nonce, sur le
    /// port de base puis les ports +1..+9 (repli anti-collision du moteur).
    /// Vrai HTTP via TcpStream — jamais un simple sleep.
    pub fn wait_ready(&mut self, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            // moteur mort → inutile d'attendre : message avec le tmp (logs dedans)
            if let Ok(Some(st)) = self.child.try_wait() {
                return Err(format!(
                    "le moteur est mort (code {:?}) — logs dans {}",
                    st.code(),
                    self.tmp.display()
                ));
            }
            for port in ports_candidats(self.port) {
                if try_hello(port) == Some(true) {
                    self.port = port;
                    self.base_url = format!("http://127.0.0.1:{}", port);
                    // Lot M2 : le harnais s'appaire au démarrage (session locale
                    // puis client Ed25519 au registre) et signe ensuite ses
                    // requêtes `/mcp` — plus aucun scénario n'appelle le gateway
                    // sans identité.
                    return self.authenticate();
                }
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "moteur pas prêt après {:?} (ports {}..{} sondés, tmp {})",
                    timeout,
                    self.port,
                    self.port.saturating_add(9),
                    self.tmp.display()
                ));
            }
            thread::sleep(Duration::from_millis(150));
        }
    }

    /// URL loopback du moteur (port réel après wait_ready).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// IP LAN non-loopback de la machine (astuce UDP : connect("8.8.8.8:80")
    /// n'émet AUCUN paquet, il sélectionne juste l'interface de routage).
    /// None si indisponible (pas d'interface LAN, sandbox…).
    pub fn lan_ip(&self) -> Option<String> {
        let s = UdpSocket::bind("0.0.0.0:0").ok()?;
        s.connect("8.8.8.8:80").ok()?;
        let ip = s.local_addr().ok()?.ip();
        if ip.is_loopback() {
            return None;
        }
        Some(ip.to_string())
    }

    /// URL LAN du moteur (même port que loopback ; bind 0.0.0.0 requis côté moteur).
    pub fn lan_base_url(&self) -> Option<String> {
        self.lan_ip()
            .map(|ip| format!("http://{}:{}", ip, self.port))
    }

    /// Requête HTTP arbitraire vers `host:port` (Host: dérivé de host, sauf si
    /// les en-têtes en contiennent déjà un). Retourne une Resp même en cas
    /// d'échec de connexion (status 0) pour que les assertions parlent clair.
    ///
    /// Lot M2 : la session locale est posée sur les requêtes **loopback** et le
    /// client du harnais signe les `POST /mcp` loopback. Une cible LAN ne reçoit
    /// ni cookie ni signature — comme un client distant réel.
    pub fn request(
        &self,
        host: &str,
        port: u16,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Resp {
        self.send(host, port, method, path, headers, body, SignMode::Auto)
    }

    /// Idem avec contrôle explicite de la signature (matrice du lot M2) :
    /// `SignMode::None` envoie la requête telle quelle, même vers `/mcp`.
    // Lot M3 : si le moteur répond « confirmation en attente » à un `POST /mcp`
    // signé par le harnais, l'approbation locale est donnée puis la requête est
    // renvoyée UNE fois (nouvelle signature, donc nouveau nonce). C'est
    // exactement le geste de l'utilisateur dans l'écran « Accès », et cela
    // laisse chaque scénario maître de son sujet ; `set_auto_approve(false)`
    // rend la main aux scénarios de confirmation.
    #[allow(clippy::too_many_arguments)] // miroir du harnais de transport
    pub fn send(
        &self,
        host: &str,
        port: u16,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
        sign: SignMode,
    ) -> Resp {
        let r = self.send_once(host, port, method, path, headers, body, sign);
        if !self.auto_approve
            || sign != SignMode::Auto
            || !is_loopback_host(host)
            || !path.starts_with("/mcp")
            || !demande_confirmation(&r)
        {
            return r;
        }
        self.approuver_en_attente();
        self.send_once(host, port, method, path, headers, body, sign)
    }

    /// Approuve (une fois) toutes les demandes en attente, tous clients confondus.
    ///
    /// C'est le geste de l'utilisateur dans l'écran « Accès », rejoué par le
    /// harnais : les scénarios E2E sont sérialisés et n'ont qu'un seul moteur
    /// vivant, donc aucune demande étrangère ne traîne. Aucune assertion ici :
    /// un scénario qui veut prouver un refus ne passe pas par cette fonction.
    pub fn approuver_en_attente(&self) {
        let l = self.action(&json!({ "action": "access_list" }));
        let ids: Vec<String> = l["pending"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| p["id"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        for id in ids {
            let _ = self.action(&json!({
                "action": "confirm_approve", "request_id": id, "mode": "once",
            }));
        }
    }

    /// Corps de [`Engine::send`], sans la reprise après approbation.
    #[allow(clippy::too_many_arguments)] // miroir du harnais de transport
    fn send_once(
        &self,
        host: &str,
        port: u16,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
        sign: SignMode,
    ) -> Resp {
        let addr: SocketAddr = match format!("{}:{}", host, port).parse() {
            Ok(a) => a,
            Err(e) => return Resp::dead(format!("adresse invalide {}:{} : {}", host, port, e)),
        };
        let local = is_loopback_host(host);
        // Liste finale des en-têtes : Host, ceux de l'appelant, Content-Type,
        // CSRF, cookie de session, puis signature. La signature couvre
        // exactement cette liste (content-type et mcp-* compris).
        let mut all: Vec<(String, String)> = Vec::new();
        if !headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("host")) {
            all.push(("Host".to_string(), format!("{}:{}", host, port)));
        }
        for (n, v) in headers {
            all.push(((*n).to_string(), (*v).to_string()));
        }
        let has = |all: &Vec<(String, String)>, name: &str| {
            all.iter().any(|(n, _)| n.eq_ignore_ascii_case(name))
        };
        if body.is_some() && !has(&all, "content-type") {
            all.push(("Content-Type".to_string(), "application/json".to_string()));
        }
        // En-têtes de routage SEP-2243 : `Mcp-Method` (obligatoire pour la
        // signature : c'est un composant couvert exigé) et `Mcp-Name` quand la
        // méthode route sur un nom. Un vrai client les envoie ; le noyau
        // vérifie leur cohérence avec le corps (-32020).
        if path.starts_with("/mcp") && !has(&all, "mcp-method") {
            if let Some(corps) = body {
                for (nom, valeur) in mcp_routing(corps) {
                    if !has(&all, nom) {
                        all.push((nom.to_string(), valeur));
                    }
                }
            }
        }
        // CSRF (S-1) : plus d'en-tête constant `X-MCC-Request`. La session
        // exige cookie ET secret (`X-MCC-Session`), comme `app.js`.
        if local {
            if let Some(cookie) = &self.session {
                if !has(&all, "cookie") {
                    all.push(("Cookie".to_string(), format!("mcc_session={cookie}")));
                }
            }
            if let Some(secret) = &self.session_secret {
                if !has(&all, "x-mcc-session") {
                    all.push(("X-MCC-Session".to_string(), secret.clone()));
                }
            }
        }
        if sign == SignMode::Auto
            && local
            && path.starts_with("/mcp")
            && !has(&all, "signature-input")
        {
            if let Some(signer) = &self.signer {
                let servis: Vec<(&str, &str)> =
                    all.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
                let authority = all
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case("host"))
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("");
                let corps = body.unwrap_or(b"");
                let a_signer = RequestToSign::new(method, path, authority, &servis, corps);
                let signes = signer.headers(&a_signer);
                for (n, v) in signes.iter() {
                    all.push((n.to_string(), v.to_string()));
                }
            }
        }
        let mut req = Vec::with_capacity(512 + body.map(|b| b.len()).unwrap_or(0));
        req.extend_from_slice(format!("{} {} HTTP/1.1\r\n", method, path).as_bytes());
        for (n, v) in &all {
            req.extend_from_slice(format!("{}: {}\r\n", n, v).as_bytes());
        }
        match body {
            Some(b) => {
                req.extend_from_slice(format!("Content-Length: {}\r\n", b.len()).as_bytes());
                req.extend_from_slice(b"Connection: close\r\n\r\n");
                // LE CORPS : annoncé par Content-Length, il DOIT suivre — sinon
                // le moteur attend (30 s de read timeout) et la réponse n'arrive
                // jamais du côté client.
                req.extend_from_slice(b);
            }
            None => req.extend_from_slice(b"Connection: close\r\n\r\n"),
        }
        let mut stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
            Ok(s) => s,
            Err(e) => return Resp::dead(format!("connexion {}:{} : {}", host, port, e)),
        };
        // lecture 60 s : certaines actions sont SYNCHRONES côté moteur
        // (« scan », « discover » : sondage réel des ports) et peuvent
        // dépasser largement 30 s sous charge — un timeout trop court
        // tronquait la réponse et faisait échouer le POST (ok:false fantôme).
        // Cas observé (lot F) : la découverte locale périodique sonde TOUS les
        // ports en écoute de la machine (34 ici) et sature le pool de sondes ;
        // le scan d'un seul port a alors dépassé 30 s et le client a rendu une
        // « réponse vide » alors que le moteur travaillait normalement (aucune
        // erreur au journal). 60 s laisse la marge d'un cycle complet.
        stream.set_read_timeout(Some(Duration::from_secs(60))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
        if stream.write_all(&req).is_err() {
            return Resp::dead(format!("écriture {}:{} échouée", host, port));
        }
        let mut data = Vec::new();
        let mut tmp = [0u8; 16384];
        loop {
            match stream.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => data.extend_from_slice(&tmp[..n]),
                Err(_) => break,
            }
            if data.len() > 64 * 1024 * 1024 {
                break;
            }
        }
        if data.is_empty() {
            // connecté mais fermé sans UN SEUL octet : à distinguer d'un échec
            // de connexion (les assertions affichent ce message via .text())
            eprintln!("[harnais] réponse vide de {}:{}", host, port);
            return Resp::dead(format!(
                "réponse vide : {}:{} a fermé sans répondre",
                host, port
            ));
        }
        Resp::parse(&data)
    }

    // ---------- session locale + appairage (docs/auth.md §6 et §3.4) ----------

    /// Cookie de session (None avant l'échange du jeton d'amorçage).
    pub fn session_cookie(&self) -> Option<&str> {
        self.session.as_deref()
    }

    /// Secret de session (S-1), `None` avant l'échange.
    pub fn session_secret(&self) -> Option<&str> {
        self.session_secret.as_deref()
    }

    /// Signeur du harnais sur `/mcp` (None = aucune identité).
    pub fn signer(&self) -> Option<&Arc<Signer>> {
        self.signer.as_ref()
    }

    /// Approbation automatique des confirmations (lot M3, `docs/auth.md` §5).
    ///
    /// Active par défaut : elle tient le rôle de l'utilisateur qui clique
    /// « Accepter une fois » dans l'écran « Accès », pour que les scénarios des
    /// lots précédents (qui appellent des outils `execution` par `/mcp`)
    /// continuent de vérifier leur propre sujet et non la confirmation. Un
    /// scénario qui ÉTUDIE la confirmation la coupe d'abord.
    pub fn set_auto_approve(&mut self, on: bool) {
        self.auto_approve = on;
    }

    /// `keyid` du client appairé par le harnais ("" s'il n'y en a pas).
    pub fn client_keyid(&self) -> &str {
        &self.keyid
    }

    /// Session locale + client appairé : appelé à la fin de `wait_ready`.
    fn authenticate(&mut self) -> Result<(), String> {
        self.open_session()?;
        self.pair_harness_client()?;
        // rejeu de la requête de garde : la session doit maintenant passer
        if self.get("/api/state").status != 200 {
            return Err("session locale refusée par /api/state".to_string());
        }
        Ok(())
    }

    /// Échange le jeton d'amorçage imprimé sur la console du moteur (§6) contre
    /// le cookie `mcc_session`. Exactement ce que fait `app.js` dans la fenêtre.
    fn open_session(&mut self) -> Result<(), String> {
        let token = self.bootstrap_token()?;
        let corps = json!({ "token": token }).to_string();
        let r = self.send(
            "127.0.0.1",
            self.port,
            "POST",
            "/api/session",
            &[],
            Some(corps.as_bytes()),
            SignMode::None,
        );
        if r.status != 200 {
            return Err(format!("POST /api/session → {} : {}", r.status, r.text()));
        }
        let set_cookie = r
            .header("set-cookie")
            .ok_or("Set-Cookie absent de /api/session")?;
        let valeur = set_cookie
            .split(';')
            .next()
            .and_then(|c| c.trim().strip_prefix("mcc_session="))
            .ok_or("cookie de session absent de Set-Cookie")?;
        if valeur.len() != 43 {
            return Err(format!(
                "cookie de session inattendu ({} car.)",
                valeur.len()
            ));
        }
        // S-1 : le secret de session arrive dans le corps, la page le garde en
        // MÉMOIRE et le présente dans `X-MCC-Session`.
        let secret = r
            .json()
            .get("secret")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if secret.len() != 43 {
            return Err(format!(
                "secret de session absent de /api/session ({} car.)",
                secret.len()
            ));
        }
        let cookie = valeur.to_string();
        self.session = Some(cookie);
        self.session_secret = Some(secret);
        Ok(())
    }

    /// Ouvre une SECONDE session (autre fenêtre) sans toucher à la courante, en
    /// passant par le canal fichier du moteur (S-1 pt 3) : c'est exactement le
    /// chemin d'une fenêtre rouverte. Rend `(cookie, secret)`.
    pub fn open_extra_session(&self) -> Result<(String, String), String> {
        let config = self.tmp.join("config.json").to_string_lossy().to_string();
        let token =
            mcp_control::auth::session::request_bootstrap(&config, Duration::from_secs(15))?;
        let corps = json!({ "token": token }).to_string();
        let r = self.send(
            "127.0.0.1",
            self.port,
            "POST",
            "/api/session",
            &[],
            Some(corps.as_bytes()),
            SignMode::None,
        );
        if r.status != 200 {
            return Err(format!(
                "POST /api/session (2e) → {} : {}",
                r.status,
                r.text()
            ));
        }
        let set_cookie = r
            .header("set-cookie")
            .ok_or("Set-Cookie absent de /api/session (2e)")?;
        let cookie = set_cookie
            .split(';')
            .next()
            .and_then(|c| c.trim().strip_prefix("mcc_session="))
            .ok_or("cookie de session absent (2e)")?
            .to_string();
        let secret = r
            .json()
            .get("secret")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if cookie.len() != 43 || secret.len() != 43 {
            return Err("session 2 : cookie ou secret de taille inattendue".to_string());
        }
        Ok((cookie, secret))
    }

    /// Lit `[session] http://…/#t=<jeton>` dans la console du moteur (stdout
    /// redirigé dans le tmp). Le jeton n'apparaît nulle part ailleurs.
    fn bootstrap_token(&self) -> Result<String, String> {
        let log = self.tmp.join("mcc-stdout.log");
        let limite = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(texte) = std::fs::read_to_string(&log) {
                for ligne in texte.lines() {
                    if let Some(reste) = ligne.split("#t=").nth(1) {
                        let jeton = reste.trim().to_string();
                        if jeton.len() == 43 {
                            return Ok(jeton);
                        }
                    }
                }
            }
            if Instant::now() >= limite {
                return Err(format!(
                    "jeton d'amorçage introuvable dans {} (le moteur n'imprime pas l'URL de session ?)",
                    log.display()
                ));
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// Appaire le client du harnais (droits `lecture`+`pilotage`+`execution`) au
    /// démarrage. La clé et le keyid vivent dans le tmp : au `restart`, le même
    /// client est réutilisé (le registre a survécu dans config.json).
    fn pair_harness_client(&mut self) -> Result<(), String> {
        let cle_path = self.tmp.join("e2e-client.key");
        let id_path = self.tmp.join("e2e-client.id");
        let (cle, keyid) = if cle_path.exists() && id_path.exists() {
            let texte = std::fs::read_to_string(&cle_path).map_err(|e| e.to_string())?;
            let keyid = std::fs::read_to_string(&id_path)
                .map_err(|e| e.to_string())?
                .trim()
                .to_string();
            (
                SigningKey::import(&texte).map_err(|e| format!("clé client illisible : {e}"))?,
                keyid,
            )
        } else {
            let cle = SigningKey::generate();
            let keyid = nouveau_keyid();
            std::fs::write(&cle_path, cle.export()).map_err(|e| e.to_string())?;
            std::fs::write(&id_path, &keyid).map_err(|e| e.to_string())?;
            (cle, keyid)
        };
        let signer = Signer::new(cle).with_keyid(keyid.clone());
        self.signer = Some(Arc::new(signer));
        self.keyid = keyid.clone();
        if self.client_known(&keyid) {
            return Ok(());
        }
        let publique = hex_bytes(&self.signer.as_ref().unwrap().verifying_key().to_bytes());
        let demande = json!({
            "keyid": keyid,
            "label": "harnais e2e",
            "public_key": publique,
            "created": now_unix(),
        })
        .to_string();
        let r = self.send(
            "127.0.0.1",
            self.port,
            "POST",
            "/auth/pair",
            &[],
            Some(demande.as_bytes()),
            SignMode::None,
        );
        if r.status != 200 {
            return Err(format!("POST /auth/pair → {} : {}", r.status, r.text()));
        }
        let pair_id = r
            .json()
            .get("pair_id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or("pair_id absent de la réponse d'appairage")?;
        let approbation = self.action(&json!({
            "action": "pair_approve",
            "pair_id": pair_id,
            "rights": ["lecture", "pilotage", "execution"],
        }));
        if approbation["ok"] != json!(true) {
            return Err(format!("approbation d'appairage refusée : {approbation}"));
        }
        Ok(())
    }

    /// Le client du harnais est-il déjà au registre ? (après un redémarrage)
    fn client_known(&self, keyid: &str) -> bool {
        self.action(&json!({ "action": "client_list" }))["clients"]
            .as_array()
            .map(|arr| arr.iter().any(|c| c["keyid"] == json!(keyid)))
            .unwrap_or(false)
    }

    /// GET simple sur le moteur (loopback).
    pub fn get(&self, path: &str) -> Resp {
        self.request("127.0.0.1", self.port, "GET", path, &[], None)
    }

    /// GET avec en-têtes additionnels (X-MCC-Access, Origin, …).
    pub fn get_with(&self, path: &str, headers: &[(&str, &str)]) -> Resp {
        self.request("127.0.0.1", self.port, "GET", path, headers, None)
    }

    /// POST JSON sur le moteur (loopback).
    pub fn post_json(&self, path: &str, v: &Value) -> Resp {
        self.request(
            "127.0.0.1",
            self.port,
            "POST",
            path,
            &[],
            Some(v.to_string().as_bytes()),
        )
    }

    /// POST /api/action → corps JSON parsé (ok:false en Value si erreur moteur).
    pub fn action(&self, v: &Value) -> Value {
        self.post_json("/api/action", v).json()
    }

    /// Écriture TCP BRUTE (fuzz) : octets envoyés TELS QUELS, lecture jusqu'à
    /// EOF avec 5 s de timeout. Les erreurs d'écriture sont tolérées (le moteur
    /// peut fermer la connexion pendant l'envoi d'un flux hostile).
    /// port_offset permet de visiter les ports de repli du moteur (+1..+9).
    pub fn raw(&self, request_bytes: &[u8], port_offset: u16) -> Resp {
        let port = self.port.saturating_add(port_offset);
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
            Ok(s) => s,
            Err(e) => return Resp::dead(format!("connexion :{} : {}", port, e)),
        };
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
        let _ = stream.write_all(request_bytes); // erreur possible = fuite d'info utile
        let mut data = Vec::new();
        let mut tmp = [0u8; 16384];
        loop {
            match stream.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => data.extend_from_slice(&tmp[..n]),
                Err(_) => break,
            }
            if data.len() > 8 * 1024 * 1024 {
                break;
            }
        }
        if data.is_empty() {
            // aucune réponse : connexion fermée / refusée à sec (status 0)
            Resp {
                status: 0,
                headers: vec![],
                body: b"(aucune reponse)".to_vec(),
            }
        } else {
            Resp::parse(&data)
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // kill + wait : pas de zombie. Le tmp est conservé (verrous Windows +
        // pistage post-mortem des logs mcc-*.log).
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------- utilitaires de signature (lot M2) ----------

/// `host` désigne-t-il la machine locale ? (cookie de session et signature
/// automatiques ne partent que vers le loopback ; une cible LAN est traitée
/// comme un client distant réel.)
pub fn is_loopback_host(h: &str) -> bool {
    matches!(h, "127.0.0.1" | "localhost" | "::1" | "[::1]")
}

/// La réponse est-elle le refus « confirmation en attente » du lot M3
/// (`docs/auth.md` §5 : `403`, `-32023`, `data.reason = "approval_pending"`,
/// `retryable`, `retryAfterMs`) ?
pub fn demande_confirmation(r: &Resp) -> bool {
    if r.status != 403 {
        return false;
    }
    let v = r.json();
    v["error"]["code"] == json!(-32023)
        && v["error"]["data"]["reason"].as_str() == Some("approval_pending")
        && v["error"]["data"]["retryable"] == json!(true)
        && v["error"]["data"]["retryAfterMs"].is_number()
}

/// En-têtes de routage SEP-2243 déduits du corps JSON-RPC : `Mcp-Method`, plus
/// `Mcp-Name` pour les méthodes qui routent sur un nom (tools/call → `name`,
/// resources/read → `uri`).
pub fn mcp_routing(body: &[u8]) -> Vec<(&'static str, String)> {
    let Ok(v) = serde_json::from_slice::<Value>(body) else {
        return Vec::new();
    };
    let Some(methode) = v.get("method").and_then(Value::as_str) else {
        return Vec::new();
    };
    let mut out = vec![("Mcp-Method", methode.to_string())];
    let champ = match methode {
        "tools/call" | "prompts/get" => Some("name"),
        "resources/read" => Some("uri"),
        _ => None,
    };
    if let Some(champ) = champ {
        if let Some(nom) = v
            .get("params")
            .and_then(|p| p.get(champ))
            .and_then(Value::as_str)
        {
            out.push(("Mcp-Name", nom.to_string()));
        }
    }
    out
}

/// Hexadécimal minuscule — la forme des clés publiques de `docs/auth.md` §3.2.
pub fn hex_bytes(b: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(b.len() * 2);
    for octet in b {
        let _ = write!(s, "{:02x}", octet);
    }
    s
}

/// Horodatage Unix en secondes (paramètre `created` de la signature).
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `keyid` de test : `^mcc-[a-z0-9]{1,8}-[0-9a-f]{16}$` (§3.1). Unique par
/// moteur de test (nanosecondes + pid + compteur).
pub fn nouveau_keyid() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let graine = nanos ^ (u64::from(std::process::id()) << 32) ^ n;
    format!("mcc-e2e-{:016x}", graine)
}

/// Sonde /api/hello rapide (wait_ready) : Some(true) = 200 + nonce ;
/// Some(false) = port ouvert mais pas notre hello ; None = port mort.
fn try_hello(port: u16) -> Option<bool> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(300)).ok()?;
    s.set_read_timeout(Some(Duration::from_millis(700))).ok();
    let _ = s.write_all(b"GET /api/hello HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    let mut data = Vec::new();
    let mut tmp = [0u8; 2048];
    loop {
        match s.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => data.extend_from_slice(&tmp[..n]),
            Err(_) => break,
        }
        if data.len() > 65536 {
            break;
        }
    }
    let text = String::from_utf8_lossy(&data);
    Some(text.starts_with("HTTP/1.1 200") && text.contains("nonce"))
}

// ---------- réponse HTTP ----------

/// Réponse HTTP parsée : status 0 = pas de réponse (connexion fermée à sec,
/// refus de connexion…). Headers en minuscules (noms).
pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Resp {
    fn dead(msg: String) -> Resp {
        Resp {
            status: 0,
            headers: vec![],
            body: msg.into_bytes(),
        }
    }

    /// Parse une réponse HTTP brute (status + headers + corps).
    fn parse(raw: &[u8]) -> Resp {
        let sep = find_sub(raw, b"\r\n\r\n").unwrap_or(raw.len());
        let head = String::from_utf8_lossy(&raw[..sep]).to_string();
        let mut lines = head.split("\r\n");
        let status = lines
            .next()
            .unwrap_or("")
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);
        let mut headers = Vec::new();
        for l in lines {
            if let Some((n, v)) = l.split_once(':') {
                headers.push((n.trim().to_ascii_lowercase(), v.trim().to_string()));
            }
        }
        let body = raw[sep.saturating_add(4).min(raw.len())..].to_vec();
        Resp {
            status,
            headers,
            body,
        }
    }

    /// Corps parsé en JSON (tolérant : Value::Null si non-JSON).
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.text()).unwrap_or(Value::Null)
    }

    /// Corps en texte (UTF-8 lossy).
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    /// Premier en-tête correspondant (nom insensible à la casse).
    pub fn header(&self, name: &str) -> Option<String> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.clone())
    }

    /// Le status correspond-il ?
    pub fn status_is(&self, s: u16) -> bool {
        self.status == s
    }
}

fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

// ---------- utilitaires d'attente ----------

/// Ports candidats de `wait_ready` : base, base+1 … base+9, SATURÉS à u16.
/// L'arithmétique saturante est obligatoire : un port éphémère Windows peut
/// dépasser 65 526, et `base + 9` débordait en u16 (panique « attempt to add
/// with overflow » du harnais, sans aucun rapport avec le moteur).
fn ports_candidats(base: u16) -> Vec<u16> {
    (0..=9u16).map(|off| base.saturating_add(off)).collect()
}

/// Un port libre (bind :0 → port → drop). Course possible mais improbable ;
/// le repli +1..+9 du moteur couvre le cas.
/// Le port rendu laisse TOUJOURS la marge de 9 du repli : au-delà de 65 526,
/// `base + 9` déborderait (cas couvert aussi par la saturation ci-dessus).
pub fn free_port() -> u16 {
    let mut dernier = 0u16;
    for _ in 0..32 {
        dernier = TcpListener::bind(("127.0.0.1", 0))
            .expect("bind 127.0.0.1:0")
            .local_addr()
            .expect("local_addr")
            .port();
        if dernier <= u16::MAX - 9 {
            return dernier;
        }
    }
    // 32 tirages de suite au-delà de 65 526 : jamais observé (plage éphémère
    // Windows 49152-65535) — la saturation de wait_ready couvre ce cas.
    dernier
}

/// Sonde `f` toutes les ~150 ms jusqu'au délai : Some(true) → succès ;
/// Some(false)/None → on continue ; échéance → false. TOUTE attente e2e est
/// deadline-bounded par ce helper.
pub fn wait_until(timeout: Duration, mut f: impl FnMut() -> Option<bool>) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if f() == Some(true) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(150));
    }
}

/// Moteur loopback sans code, prêt (wait_ready 15 s) — le cas le plus courant.
pub fn spawn_loopback() -> Engine {
    let mut e = spawn(EngineCfg::default()).expect("spawn moteur loopback");
    e.wait_ready(Duration::from_secs(15))
        .expect("moteur loopback prêt");
    e
}

/// Moteur loopback avec code d'accès, prêt.
pub fn spawn_loopback_with_code(code: &str) -> Engine {
    let cfg = EngineCfg {
        ui_port: free_port(),
        ui_bind: "127.0.0.1",
        api_access: Some(code.to_string()),
    };
    let mut e = spawn(cfg).expect("spawn moteur loopback+code");
    e.wait_ready(Duration::from_secs(15))
        .expect("moteur loopback+code prêt");
    e
}

/// Moteur exposé au LAN (bind 0.0.0.0) avec code d'accès, prêt.
pub fn spawn_lan_with_code(code: &str) -> Engine {
    let cfg = EngineCfg {
        ui_port: free_port(),
        ui_bind: "0.0.0.0",
        api_access: Some(code.to_string()),
    };
    let mut e = spawn(cfg).expect("spawn moteur LAN");
    e.wait_ready(Duration::from_secs(15))
        .expect("moteur LAN prêt");
    e
}

// ---------- faux serveur MCP (HTTP JSON-RPC en Rust pur) ----------

/// Mode d'un serveur MCP factice.
#[derive(Clone, Copy, PartialEq)]
pub enum FakeMode {
    /// initialize → serverInfo + capabilities ; tools/list → 2 outils ;
    /// notifications/… → result vide ; TOUT le reste (GET /health, /mcp, /sse…)
    /// répond la même réponse initialize — path-agnostique, toute stratégie de
    /// sonde du moteur aboutit.
    Mcp,
    /// Toujours {"result":{}} : JSON-RPC correct mais PAS un serveur MCP
    /// (validate_initialize → NotMcp) — le moteur doit rester à PORT-ONLY.
    ResultEmpty,
    /// Accepte la connexion et ne répond JAMAIS (serveur mort-faux).
    Stall,
}

/// Serveur MCP factice en mémoire : TcpListener + thread d'acceptation,
/// un thread par connexion, réponses 200 JSON + Connection: close.
pub struct Fixture {
    port: u16,
    stop: Arc<AtomicBool>,
    th: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    /// Faux MCP complet (initialize valide + tools/list).
    pub fn spawn_mcp() -> Fixture {
        Self::spawn(FakeMode::Mcp)
    }

    /// Faux serveur qui répond toujours {"result":{}} (pas un MCP).
    pub fn spawn_result_empty() -> Fixture {
        Self::spawn(FakeMode::ResultEmpty)
    }

    /// Faux serveur muet : accepte, ne répond jamais.
    pub fn spawn_stall() -> Fixture {
        Self::spawn(FakeMode::Stall)
    }

    fn spawn(mode: FakeMode) -> Fixture {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind fixture 127.0.0.1:0");
        let port = listener.local_addr().expect("local_addr fixture").port();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let th = thread::Builder::new()
            .name("mcc-fake-mcp".into())
            .spawn(move || {
                let _ = listener.set_nonblocking(true);
                loop {
                    if stop2.load(Ordering::SeqCst) {
                        break;
                    }
                    match listener.accept() {
                        Ok((s, _)) => {
                            // thread détaché : les connexions stall peuvent durer
                            let _ = thread::Builder::new()
                                .name("mcc-fake-conn".into())
                                .spawn(move || fake_conn(s, mode));
                        }
                        Err(_) => thread::sleep(Duration::from_millis(25)),
                    }
                }
                // listener droppé ici → port libéré
            })
            .expect("spawn thread fixture");
        Fixture {
            port,
            stop,
            th: Some(th),
        }
    }

    /// Port éphémère du faux serveur.
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // join du thread d'acceptation → listener droppé → port fermé
        if let Some(h) = self.th.take() {
            let _ = h.join();
        }
    }
}

/// Initialise MCP valide (passe validate_initialize : serverInfo.name présent).
fn init_json() -> String {
    json!({
        "jsonrpc": "2.0", "id": 1,
        "result": {
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "fake-e2e-mcp", "version": "0.2.0"}
        }
    })
    .to_string()
}

/// tools/list : deux outils, comme un vrai serveur.
fn tools_json() -> String {
    json!({
        "jsonrpc": "2.0", "id": 2,
        "result": {"tools": [
            {"name": "ping", "description": "ping tool", "inputSchema": {"type": "object"}},
            {"name": "echo", "description": "echo tool", "inputSchema": {"type": "object"}}
        ]}
    })
    .to_string()
}

/// Traite UNE connexion du faux serveur : lit l'en-tête + le corps, répond
/// selon le mode. Path-agnostique : le moteur sonde /mcp, /, /api/mcp, /health.
fn fake_conn(mut s: TcpStream, mode: FakeMode) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let head_end = loop {
        match s.read(&mut tmp) {
            Ok(0) => return,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if let Some(p) = find_sub(&buf, b"\r\n\r\n") {
                    break p;
                }
                if buf.len() > 65536 {
                    return;
                }
            }
            Err(_) => return,
        }
    };
    // Content-Length (première occurrence — nos clients n'en envoient qu'un)
    let head = String::from_utf8_lossy(&buf[..head_end]).to_ascii_lowercase();
    let mut cl = 0usize;
    for line in head.lines().skip(1) {
        if let Some(v) = line.strip_prefix("content-length:") {
            cl = v.trim().parse().unwrap_or(0);
            break;
        }
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < cl {
        match s.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&tmp[..n]),
            Err(_) => break,
        }
    }
    body.truncate(cl);

    if mode == FakeMode::Stall {
        // accepté mais JAMAIS de réponse : on avale jusqu'à la fin (le client
        // fermant ou le timeout de lecture met fin au thread)
        let mut sink = Vec::new();
        let _ = s.read_to_end(&mut sink);
        return;
    }

    let payload = match mode {
        FakeMode::Mcp => {
            let parsed: Option<Value> = serde_json::from_slice(&body).ok();
            let method = parsed
                .as_ref()
                .and_then(|v| v.get("method").and_then(|m| m.as_str()).map(String::from))
                .unwrap_or_default();
            match method.as_str() {
                "tools/list" => tools_json(),
                // tools/call : le gateway MCP de la console proxyfie les
                // appels jusqu'ici — la fixture répond un vrai résultat avec
                // structuredContent, en écho au nom de l'outil appelé
                "tools/call" => {
                    let appel = parsed
                        .as_ref()
                        .and_then(|v| v["params"]["name"].as_str())
                        .unwrap_or("?")
                        .to_string();
                    let rpc_id = parsed.as_ref().map(|v| v["id"].clone()).unwrap_or(json!(9));
                    json!({
                        "jsonrpc": "2.0", "id": rpc_id,
                        "result": {
                            "content": [{"type": "text", "text": format!("pong:{}", appel)}],
                            "structuredContent": { "pong": true, "tool": appel }
                        }
                    })
                    .to_string()
                }
                // notifications/initialized et toute autre méthode : result vide
                "notifications/initialized" => {
                    json!({"jsonrpc": "2.0", "id": 3, "result": {}}).to_string()
                }
                // initialize (et GET sans corps : /health, /, /sse…) → initialize
                _ => init_json(),
            }
        }
        FakeMode::ResultEmpty => json!({"jsonrpc": "2.0", "id": 1, "result": {}}).to_string(),
        FakeMode::Stall => unreachable!(),
    };
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        payload.len(),
        payload
    );
    let _ = s.write_all(resp.as_bytes());
    let _ = s.flush();
}

// ---------- tests du harnais lui-même ----------

/// Non-régression de l'échec aléatoire du lot B : `port + off` en u16 paniquait
/// (« attempt to add with overflow ») quand `free_port()` rendait ≥ 65 527.
#[test]
fn harnais_ports_candidats_ne_debordent_pas() {
    assert_eq!(ports_candidats(5432), (5432..=5441).collect::<Vec<u16>>());
    // base au ras du plafond : les 10 candidats restent dans u16
    assert_eq!(ports_candidats(u16::MAX), vec![u16::MAX; 10]);
    assert_eq!(
        ports_candidats(u16::MAX - 5).last().copied(),
        Some(u16::MAX)
    );
}

/// `free_port()` doit laisser la marge de 9 du repli moteur : sinon
/// `wait_ready` sonde des ports hors u16 (l'ancien panic).
#[test]
fn harnais_free_port_laisse_la_marge_du_repli() {
    for _ in 0..64 {
        let p = free_port();
        assert!(
            p <= u16::MAX - 9,
            "free_port a rendu {} : la marge de 9 du repli moteur déborde",
            p
        );
    }
}
