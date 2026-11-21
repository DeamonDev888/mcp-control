//! Scénarios E2E du lot M1 — **identités, registre des clients, appairage**.
//!
//! Critère de fin de `TODO-AUTH.md` : appairage complet entre deux process,
//! demande non approuvée = aucun accès, demande expirée, tempête de 500
//! demandes bornée avec l'instance toujours debout, `config.json` sans aucune
//! clé privée, redémarrage → le client reste appairé.
//!
//! Rien n'est simulé : le VRAI binaire est lancé (harness) et, pour le premier
//! scénario, la VRAIE sous-commande `mcp-control auth pair` tourne dans un
//! SECOND process. Les moteurs de test forcent le repli fichier
//! (`MCC_KEYRING_SERVICE=""`, posé par le harnais) : aucun test n'écrit dans le
//! gestionnaire d'identifiants de l'utilisateur.

use crate::harness::{
    demande_confirmation, free_port, hex_bytes, nouveau_keyid, serial_guard, spawn, Engine,
    EngineCfg, Resp, SignMode,
};
use litemcp::auth::sign::{RequestToSign, Signer, SigningKey};
use serde_json::{json, Value};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Clés PUBLIQUES seulement (vecteurs RFC 8032 §7.1) : le lot M1 ne vérifie
/// encore aucune signature, un test n'a donc pas besoin de clé privée pour
/// exiger une place dans la file d'appairage.
const CLE_PUB: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn engine() -> Engine {
    let mut e = spawn(EngineCfg::default()).expect("spawn du moteur");
    e.wait_ready(Duration::from_secs(30)).expect("moteur prêt");
    e
}

fn corps(keyid: &str) -> Value {
    json!({
        "keyid": keyid,
        "label": "poste e2e",
        "public_key": CLE_PUB,
        "created": now(),
    })
}

/// Envoie une demande d'appairage et rend (statut, corps JSON).
fn demander(e: &Engine, keyid: &str) -> (u16, Value) {
    let r: Resp = e.post_json("/auth/pair", &corps(keyid));
    (r.status, r.json())
}

fn statut_pair(e: &Engine, pair_id: &str) -> (u16, Value) {
    let r = e.get(&format!("/auth/pair/{pair_id}"));
    (r.status, r.json())
}

/// Clients du registre — HORS client du harnais : depuis le lot M2 le harnais
/// s'appaire lui-même au démarrage (`harness::Engine::client_keyid`), ces
/// scénarios comptent donc les appairages qu'ILS ont provoqués.
fn clients(e: &Engine) -> Vec<Value> {
    let harnais = e.client_keyid().to_string();
    e.action(&json!({ "action": "client_list" }))["clients"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c["keyid"].as_str() != Some(harnais.as_str()))
        .collect()
}

/// (critère 1) Appairage complet entre DEUX process : le moteur, et le VRAI
/// `mcp-control auth pair <url>` lancé à part. Le test approuve comme le ferait
/// la fenêtre locale, puis vérifie que le CLI a bien reçu l'approbation.
#[test]
fn appairage_complet_entre_deux_process() {
    let _garde = serial_guard();
    let e = engine();
    let client_dir =
        std::env::temp_dir().join(format!("mcc-e2e-client-{}-{}", std::process::id(), e.port));
    std::fs::create_dir_all(&client_dir).unwrap();
    let client_cfg = client_dir.join("config.json");

    let mut enfant = Command::new(env!("CARGO_BIN_EXE_mcp-control"))
        .args(["auth", "pair", e.base_url()])
        .env("MCC_CONFIG", &client_cfg)
        // aucun test n'écrit dans le trousseau de l'utilisateur
        .env("MCC_KEYRING_SERVICE", "")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("lancement du CLI d'appairage");

    // La demande doit apparaître dans pair_list AVANT toute approbation.
    let limite = Instant::now() + Duration::from_secs(20);
    let pair = loop {
        let v = e.action(&json!({ "action": "pair_list" }));
        if let Some(p) = v["pending"].as_array().and_then(|a| a.first()) {
            break p.clone();
        }
        assert!(
            Instant::now() < limite,
            "aucune demande d'appairage reçue du CLI : {v}"
        );
        std::thread::sleep(Duration::from_millis(200));
    };
    let keyid = pair["keyid"].as_str().unwrap_or("").to_string();
    let pair_id = pair["pair_id"].as_str().unwrap_or("").to_string();
    let code = pair["code"].as_str().unwrap_or("").to_string();
    assert!(
        keyid.starts_with("mcc-"),
        "keyid généré par le CLI hors grammaire : {keyid}"
    );
    assert_eq!(pair_id.len(), 32);
    assert!(
        code.len() == 7 && code.chars().nth(3) == Some(' '),
        "code de comparaison attendu en six chiffres groupés : {code:?}"
    );
    assert_eq!(pair["paired_from"], "127.0.0.1");
    // Rien n'est accordé avant l'approbation locale.
    assert!(clients(&e).is_empty(), "aucun client avant approbation");

    let ok = e.action(&json!({
        "action": "pair_approve",
        "pair_id": pair_id,
        "rights": ["lecture", "pilotage"],
    }));
    assert_eq!(ok["ok"], json!(true), "approbation refusée : {ok}");

    // Le CLI sort en 0 une fois l'approbation lue (poll toutes les 2 s).
    let limite = Instant::now() + Duration::from_secs(40);
    let sortie = loop {
        if let Some(st) = enfant.try_wait().expect("état du CLI") {
            let mut out = String::new();
            if let Some(mut s) = enfant.stdout.take() {
                let _ = s.read_to_string(&mut out);
            }
            break (st, out);
        }
        assert!(
            Instant::now() < limite,
            "le CLI d'appairage n'a pas rendu la main après approbation"
        );
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(
        sortie.0.success(),
        "le CLI doit sortir en 0 après approbation (code {:?})",
        sortie.0.code()
    );
    assert!(
        sortie.1.contains("Appairé"),
        "le CLI doit annoncer l'appairage : {}",
        sortie.1
    );
    assert!(
        sortie.1.contains(&keyid),
        "le CLI doit rendre le keyid appairé {keyid} : {}",
        sortie.1
    );
    assert!(
        sortie.1.contains(&code),
        "le CLI doit afficher le MÊME code de comparaison que la fenêtre ({code}) : {}",
        sortie.1
    );
    // Le client a rangé sa clé privée de son côté (fichier 0600, repli forcé).
    let cle_client = client_dir
        .join("auth")
        .join("clients")
        .join(format!("{keyid}.key"));
    assert!(
        cle_client.exists(),
        "clé privée du client absente : {}",
        cle_client.display()
    );

    // Côté serveur : la ligne est au registre, active, avec les droits choisis.
    let liste = clients(&e);
    assert_eq!(liste.len(), 1, "{liste:?}");
    assert_eq!(liste[0]["keyid"], keyid);
    assert_eq!(liste[0]["status"], "active");
    assert_eq!(liste[0]["rights"], json!(["lecture", "pilotage"]));
    assert!(
        liste[0]["fingerprint"]
            .as_str()
            .unwrap_or("")
            .starts_with("ed25519:"),
        "empreinte absente : {}",
        liste[0]
    );
    let _ = std::fs::remove_dir_all(&client_dir);
}

/// (critère 2) Demande non approuvée = AUCUN accès : la demande n'existe que
/// dans la file, jamais au registre ; un refus la clôt définitivement.
#[test]
fn demande_non_approuvee_n_accorde_aucun_acces() {
    let _garde = serial_guard();
    let e = engine();
    let keyid = "mcc-e2e-00000000000000aa";
    let (s, v) = demander(&e, keyid);
    assert_eq!(s, 200, "{v}");
    let pair_id = v["pair_id"].as_str().unwrap_or("").to_string();
    assert_eq!(v["expires_in"], 120);
    assert_eq!(v["api"], "mcc-auth-v1");
    assert_eq!(v["server_keyid"], "mcc-server");
    assert_eq!(v["tls_fp"], "", "loopback : pas d'empreinte TLS");

    // en attente, et RIEN au registre
    let (s, v) = statut_pair(&e, &pair_id);
    assert_eq!(s, 200);
    assert_eq!(v["status"], "pending", "{v}");
    assert!(clients(&e).is_empty(), "une demande ne crée aucun accès");

    // la demande ne peut pas s'auto-approuver par une route réseau : il n'y en a
    // pas ; on vérifie qu'aucune action du gateway ne l'atteint non plus.
    let r = e.post_json(
        "/mcp",
        &json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "client_list", "arguments": {} }
        }),
    );
    let v: Value = r.json();
    assert!(
        v.get("error").is_some(),
        "aucun tool d'authentification ne doit exister sur /mcp : {v}"
    );

    // refus local : définitif
    let ko = e.action(&json!({ "action": "pair_reject", "pair_id": pair_id }));
    assert_eq!(ko["ok"], json!(true), "{ko}");
    let (_, v) = statut_pair(&e, &pair_id);
    assert_eq!(v["status"], "rejected", "{v}");
    let ko = e.action(&json!({
        "action": "pair_approve", "pair_id": pair_id, "rights": ["lecture"],
    }));
    assert_eq!(
        ko["ok"],
        json!(false),
        "une demande refusée reste refusée : {ko}"
    );
    assert!(clients(&e).is_empty());

    // un pair_id inconnu (32 hex) rend 404 : rien n'est énumérable
    assert_eq!(e.get(&format!("/auth/pair/{}", "0".repeat(32))).status, 404);
    assert_eq!(e.get("/auth/pair/pas-un-id").status, 404);
    // et aucune clé n'a été créée dans le fichier de config
    let cfg = std::fs::read_to_string(e.tmp_dir().join("config.json")).unwrap();
    assert!(
        !cfg.contains(keyid),
        "aucun client ne doit être persisté : {cfg}"
    );
}

/// (critère 3) Demande expirée : au-delà de 2 minutes elle n'est plus
/// approuvable, le client lit `expired`, et rien n'est entré au registre.
#[test]
fn demande_expiree() {
    let _garde = serial_guard();
    let e = engine();
    let keyid = "mcc-e2e-00000000000000bb";
    let (s, v) = demander(&e, keyid);
    assert_eq!(s, 200, "{v}");
    let pair_id = v["pair_id"].as_str().unwrap_or("").to_string();

    // La spec fixe 2 minutes : on attend la vraie constante, pas une version
    // abrégée (aucune variable d'environnement ne raccourcit ce délai).
    std::thread::sleep(Duration::from_secs(122));

    let (s, v) = statut_pair(&e, &pair_id);
    assert_eq!(s, 200, "une demande expirée se lit encore (statut expired)");
    assert_eq!(v["status"], "expired", "{v}");
    let ko = e.action(&json!({
        "action": "pair_approve", "pair_id": pair_id, "rights": ["lecture"],
    }));
    assert_eq!(
        ko["ok"],
        json!(false),
        "approuver une demande expirée : {ko}"
    );
    assert!(
        ko["error"].as_str().unwrap_or("").contains("expirée"),
        "motif attendu : {ko}"
    );
    assert!(clients(&e).is_empty());
}

/// (critère 4) Tempête de 500 demandes : la file est bornée par le quota par
/// adresse, l'instance répond toujours en local, et `config.json` ne grossit pas.
#[test]
fn tempete_de_500_demandes_bornee_et_instance_vivante() {
    let _garde = serial_guard();
    let e = engine();
    let mut acceptees = 0usize;
    let mut refusees = 0usize;
    let mut autres: Vec<u16> = Vec::new();
    let debut = Instant::now();
    for i in 0..500 {
        let keyid = format!("mcc-flood-{i:016x}");
        let (s, _) = demander(&e, &keyid);
        match s {
            200 => acceptees += 1,
            429 => refusees += 1,
            autre => autres.push(autre),
        }
    }
    let duree = debut.elapsed();
    assert!(autres.is_empty(), "statuts inattendus : {autres:?}");
    // Le quota est de 5 demandes par adresse et par minute ; l'appairage du
    // harnais (lot M2) en a consommé une au démarrage — la propriété vérifiée
    // est le PLAFOND (jamais plus de 5 admises), pas le compte exact.
    assert!(
        acceptees <= 5,
        "le quota d'appairage (5/min/Adresse) doit borner la tempête : {acceptees} admises"
    );
    assert!(
        refusees >= 495,
        "les 500 demandes doivent être refusées au-delà du quota : {refusees} refus"
    );
    assert_eq!(acceptees + refusees, 500);
    assert!(
        duree < Duration::from_secs(120),
        "les refus doivent être rendus vite (mesuré {duree:?})"
    );

    // la file reste bornée (16 au plus) et l'instance répond en local
    let v = e.action(&json!({ "action": "pair_list" }));
    let pending = v["pending"].as_array().cloned().unwrap_or_default();
    assert!(
        pending.len() <= 16,
        "file d'attente non bornée : {} demandes",
        pending.len()
    );
    let r = e.get("/api/state");
    assert_eq!(r.status, 200, "l'instance doit rester debout");
    assert!(
        r.text().len() > 10,
        "l'état local doit rester servi après la tempête"
    );
    assert_eq!(e.get("/api/hello").status, 200);
    // après une rafale, seul le premier lot a été accepté : aucun client au
    // registre (aucune approbation n'a eu lieu)
    assert!(clients(&e).is_empty());
}

/// (critère 5) `config.json` ne contient AUCUNE clé privée — vérifié en
/// relisant le fichier après un appairage complet.
#[test]
fn config_json_ne_contient_aucune_cle_privee() {
    let _garde = serial_guard();
    let e = engine();
    let keyid = "mcc-e2e-00000000000000cc";
    let (s, v) = demander(&e, keyid);
    assert_eq!(s, 200, "{v}");
    let pair_id = v["pair_id"].as_str().unwrap_or("").to_string();
    let ok = e.action(&json!({
        "action": "pair_approve", "pair_id": pair_id, "rights": ["lecture"],
    }));
    assert_eq!(ok["ok"], json!(true), "{ok}");

    let chemin = e.tmp_dir().join("config.json");
    let texte = std::fs::read_to_string(&chemin).unwrap();
    // la clé PUBLIQUE du serveur et celle du client sont là
    assert!(texte.contains("\"public_key\""), "{texte}");
    assert!(
        texte.contains(keyid),
        "le client appairé doit être persisté"
    );
    // et AUCUN bloc privé, sous quelque forme que ce soit
    for interdit in [
        "PRIVATE KEY",
        "BEGIN PRIVATE",
        "BEGIN OPENSSH",
        "LITEMCP ED25519 PRIVATE",
    ] {
        assert!(
            !texte.contains(interdit),
            "config.json contient « {interdit} » — interdit par docs/auth.md §9"
        );
    }
    // La clé privée du serveur vit à côté, dans `auth/server.key` (repli forcé
    // pour les tests), avec des permissions réduites à l'utilisateur.
    let cle_serveur = e.tmp_dir().join("auth").join("server.key");
    assert!(
        cle_serveur.exists(),
        "la clé privée du serveur doit vivre hors de config.json : {}",
        cle_serveur.display()
    );
    let bloc = std::fs::read_to_string(&cle_serveur).unwrap();
    assert!(
        bloc.contains("PRIVATE KEY"),
        "bloc privé attendu au trousseau/repli"
    );
    // le contenu de la clé privée n'apparaît nulle part dans config.json
    for ligne in bloc.lines().filter(|l| l.len() > 32) {
        assert!(
            !texte.contains(ligne),
            "un fragment de clé privée a fui dans config.json"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&cle_serveur)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "clé privée : 0600 attendu, vu {mode:o}");
    }
    #[cfg(windows)]
    {
        let out = std::process::Command::new("icacls")
            .arg(&cle_serveur)
            .output()
            .expect("icacls");
        let acl = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(
            !acl.contains("(I)"),
            "la clé privée ne doit pas hériter des droits du dossier : {acl}"
        );
    }
}

/// (critère 6) Redémarrage : le client reste appairé (registre persisté), et
/// l'identité du serveur ne change pas.
#[test]
fn redemarrage_le_client_reste_appaire() {
    let _garde = serial_guard();
    let mut e = engine();
    let keyid = "mcc-e2e-00000000000000dd";
    let (s, v) = demander(&e, keyid);
    assert_eq!(s, 200, "{v}");
    let pair_id = v["pair_id"].as_str().unwrap_or("").to_string();
    let ok = e.action(&json!({
        "action": "pair_approve", "pair_id": pair_id, "rights": ["lecture"],
    }));
    assert_eq!(ok["ok"], json!(true), "{ok}");
    let avant = clients(&e);
    assert_eq!(avant.len(), 1);
    let empreinte_avant = avant[0]["fingerprint"].clone();
    let cfg_avant = std::fs::read_to_string(e.tmp_dir().join("config.json")).unwrap();

    e.restart().expect("redémarrage sur la même config");

    let apres = clients(&e);
    assert_eq!(apres.len(), 1, "le client doit rester appairé : {apres:?}");
    assert_eq!(apres[0]["keyid"], keyid);
    assert_eq!(apres[0]["status"], "active");
    assert_eq!(
        apres[0]["fingerprint"], empreinte_avant,
        "l'identité du serveur ne doit pas changer au redémarrage"
    );
    assert_eq!(
        apres[0]["rights"],
        json!(["lecture"]),
        "les droits accordés doivent survivre"
    );
    // même identité de serveur dans le fichier (clé publique inchangée)
    let cfg_apres = std::fs::read_to_string(e.tmp_dir().join("config.json")).unwrap();
    let cle_avant: Value = serde_json::from_str(&cfg_avant).unwrap();
    let cle_apres: Value = serde_json::from_str(&cfg_apres).unwrap();
    assert_eq!(
        cle_avant["auth"]["server"]["public_key"], cle_apres["auth"]["server"]["public_key"],
        "la clé publique du serveur doit survivre au redémarrage"
    );
    // la demande d'appairage, elle, n'est pas persistée : 404 après redémarrage
    assert_eq!(e.get(&format!("/auth/pair/{pair_id}")).status, 404);
}

// =====================================================================
// Lot M2 — le sas : matrice {identité} × {transport} × {surface}
// =====================================================================

/// Corps JSON-RPC d'un `tools/list` numéroté.
fn corps_tools_list(id: u64) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/list" })
}

/// Corps JSON-RPC d'un `tools/call`.
fn corps_tool(outil: &str, args: Value, id: u64) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "tools/call",
        "params": { "name": outil, "arguments": args }
    })
}

/// Appaire un client de test avec exactement `droits` et rend son signeur.
/// Le keyid est neuf à chaque appel (aucune collision avec le harnais).
fn client_appaire(e: &Engine, droits: &[&str]) -> Signer {
    client_appaire_label(e, droits, &format!("matrice {}", droits.join("+")))
}

/// Idem avec un libellé imposé (le libellé est un texte VENU DU CLIENT : la page
/// doit l'échapper, `docs/auth.md` §5 et test `libelle_hostile_jamais_dans_le_html`).
fn client_appaire_label(e: &Engine, droits: &[&str], label: &str) -> Signer {
    let cle = SigningKey::generate();
    let keyid = nouveau_keyid();
    let publique = hex_bytes(&cle.verifying_key().to_bytes());
    let demande = json!({
        "keyid": keyid,
        "label": label,
        "public_key": publique,
        "created": now(),
    });
    let r = e.post_json("/auth/pair", &demande);
    assert_eq!(r.status, 200, "appairage refusé : {}", r.text());
    let pair_id = r.json()["pair_id"].as_str().expect("pair_id").to_string();
    let ok = e.action(&json!({
        "action": "pair_approve", "pair_id": pair_id, "rights": droits,
    }));
    assert_eq!(ok["ok"], json!(true), "approbation refusée : {ok}");
    Signer::new(cle).with_keyid(keyid)
}

/// `POST /mcp` signé par `signer`. `created`/`nonce` imposés quand la matrice
/// teste l'horloge ou le rejeu. `SignMode::None` : c'est NOUS qui signons, le
/// harnais n'ajoute rien.
fn post_mcp_signe(
    e: &Engine,
    signer: &Signer,
    host: &str,
    corps: &Value,
    created: Option<u64>,
    nonce: Option<[u8; 16]>,
) -> Resp {
    let authority = format!("{}:{}", host, e.port);
    let body = corps.to_string();
    let methode = corps["method"].as_str().unwrap_or("");
    let mut base: Vec<(String, String)> =
        vec![("Content-Type".to_string(), "application/json".to_string())];
    base.push(("Mcp-Method".to_string(), methode.to_string()));
    if matches!(methode, "tools/call" | "prompts/get") {
        if let Some(nom) = corps["params"]["name"].as_str() {
            base.push(("Mcp-Name".to_string(), nom.to_string()));
        }
    }
    let refs0: Vec<(&str, &str)> = base.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
    let a_signer = RequestToSign::new("POST", "/mcp", &authority, &refs0, body.as_bytes());
    let signes = match (created, nonce) {
        // nonce imposé (rejeu) : `created` fourni ou courant — les deux envois
        // doivent produire des octets IDENTIQUES pour que le rejeu soit réel.
        (c, Some(n)) => signer.headers_at(&a_signer, c.unwrap_or_else(now), &n),
        // `created` imposé seul (horloge décalée) : nonce neuf
        (Some(c), None) => signer.headers_at(&a_signer, c, &nonce_neuf()),
        (None, None) => signer.headers(&a_signer),
    };
    let mut all = base;
    for (n, v) in signes.iter() {
        all.push((n.to_string(), v.to_string()));
    }
    let refs: Vec<(&str, &str)> = all.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
    e.send(
        host,
        e.port,
        "POST",
        "/mcp",
        &refs,
        Some(body.as_bytes()),
        SignMode::None,
    )
}

/// `POST /mcp` SANS signature (ni clé, ni cookie) : le cas « process local sans
/// identité » du critère de fin.
fn post_mcp_sans_signature(e: &Engine, host: &str, corps: &Value) -> Resp {
    let body = corps.to_string();
    let methode = corps["method"].as_str().unwrap_or("");
    let mut entetes: Vec<(String, String)> = vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("Mcp-Method".to_string(), methode.to_string()),
        // pas de cookie : on ne veut pas de la session locale sur /mcp
        ("Cookie".to_string(), "-".to_string()),
    ];
    if matches!(methode, "tools/call" | "prompts/get") {
        if let Some(nom) = corps["params"]["name"].as_str() {
            entetes.push(("Mcp-Name".to_string(), nom.to_string()));
        }
    }
    let refs: Vec<(&str, &str)> = entetes
        .iter()
        .map(|(n, v)| (n.as_str(), v.as_str()))
        .collect();
    e.send(
        host,
        e.port,
        "POST",
        "/mcp",
        &refs,
        Some(body.as_bytes()),
        SignMode::None,
    )
}

/// Un refus d'authentification : 401, corps constant de la lib et
/// `WWW-Authenticate` présent.
fn est_refus_401(r: &Resp) -> bool {
    r.status == 401
        && r.header("www-authenticate")
            .is_some_and(|v| v.contains("Signature") && v.contains("litemcp-auth-v1"))
        && r.text().contains("unauthorized")
}

/// Un refus de droit : 403 `-32023` (le handler n'a pas tourné).
fn est_refus_droit(r: &Resp) -> bool {
    if r.status != 403 {
        return false;
    }
    let v = r.json();
    v["error"]["code"] == json!(-32023)
        && v["error"]["data"]["reason"].as_str() == Some("right_missing")
}

/// Nonce de 16 octets pour une signature imposée (jamais réutilisé dans un
/// même cas : un nonce rejoué serait refusé pour une autre raison).
fn nonce_neuf() -> [u8; 16] {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let a = nanos ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let b = nanos.rotate_left(17) ^ u64::from(std::process::id());
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&a.to_le_bytes());
    out[8..].copy_from_slice(&b.to_le_bytes());
    out
}

/// Nombre d'outils publiés par une réponse `tools/list`.
fn nombre_outils(r: &Resp) -> usize {
    r.json()["result"]["tools"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0)
}

/// Résultat attendu d'une cellule de la matrice.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Attendu {
    /// 200 et exactement `n` outils publiés (`tools/list`).
    Liste(usize),
    /// 200 avec un résultat : l'outil a réellement tourné.
    Ok,
    /// 401 de corps constant + `WWW-Authenticate` (identité absente, fausse,
    /// périmée, rejouée ou révoquée).
    Refus401,
    /// 403 `-32023` : droit manquant, le handler n'a pas tourné.
    RefusDroit,
}

/// Une ligne de la matrice : identité + attendus des quatre surfaces `/mcp`,
/// dans l'ordre tools/list, outil lecture, outil pilotage, outil execution.
/// Les deux cellules `/api/*` sont vérifiées à part (elles ne dépendent que de
/// la session locale, jamais de la clé MCP).
struct Cas {
    nom: &'static str,
    attendus: [Attendu; 4],
}

/// Exécute une ligne et rend le nombre de cellules CONFORMES (sur 6).
/// `requete(host, corps)` doit envoyer le `POST /mcp` du cas.
fn execute_cas(
    e: &Engine,
    host: &str,
    cas: &Cas,
    requete: &dyn Fn(&str, &Value) -> Resp,
    session_disponible: bool,
) -> usize {
    let mut conformes = 0usize;
    let tl = requete(host, &corps_tools_list(10));
    let cellule_tl = match cas.attendus[0] {
        Attendu::Liste(n) => tl.status == 200 && nombre_outils(&tl) == n,
        Attendu::Refus401 => est_refus_401(&tl),
        Attendu::Ok => tl.status == 200 && tl.json().get("result").is_some(),
        Attendu::RefusDroit => est_refus_droit(&tl),
    };
    assert!(
        cellule_tl,
        "{} / {host} — tools/list : {}",
        cas.nom,
        tl.text().chars().take(300).collect::<String>()
    );
    conformes += 1;

    for (index, (outil, args)) in [
        ("list_servers", json!({})),
        ("refresh", json!({})),
        ("add_server", json!({ "transport": "http", "port": 30_977 })),
    ]
    .into_iter()
    .enumerate()
    {
        // Lot M3 : un outil `execution` n'est plus exécuté sans confirmation.
        // Pour une identité qui en a le droit, le harnais donne l'accord local
        // exigé par `docs/auth.md` §5 et REJOUE la requête (nouvelle signature,
        // donc nouveau nonce) : la cellule vérifie le droit, pas la confirmation
        // (les scénarios de confirmation forment le lot M3 et la coupent).
        let mut r = requete(host, &corps_tool(outil, args.clone(), 11 + index as u64));
        if demande_confirmation(&r) {
            e.approuver_en_attente();
            r = requete(host, &corps_tool(outil, args, 11 + index as u64));
        }
        let ok = match cas.attendus[index + 1] {
            Attendu::Refus401 => est_refus_401(&r),
            Attendu::RefusDroit => est_refus_droit(&r),
            Attendu::Ok => r.status == 200 && r.json().get("result").is_some(),
            Attendu::Liste(_) => false,
        };
        assert!(
            ok,
            "{} / {host} — {outil} : attendu {:?}, obtenu {} ({})",
            cas.nom,
            cas.attendus[index + 1],
            r.status,
            r.text().chars().take(300).collect::<String>()
        );
        conformes += 1;
    }

    // /api/* : seule la SESSION locale y donne accès — jamais la clé MCP.
    let etat = e.send(host, e.port, "GET", "/api/state", &[], None, SignMode::None);
    let attendu_api = if session_disponible { 200 } else { 401 };
    assert_eq!(
        etat.status,
        attendu_api,
        "{} / {host} — /api/state (session {})",
        cas.nom,
        if session_disponible {
            "présente"
        } else {
            "absente"
        }
    );
    conformes += 1;
    let action = e.send(
        host,
        e.port,
        "POST",
        "/api/action",
        &[("Content-Type", "application/json")],
        Some(br#"{"action":"client_list"}"#),
        SignMode::None,
    );
    assert_eq!(
        action.status,
        if session_disponible { 200 } else { 401 },
        "{} / {host} — /api/action (session {})",
        cas.nom,
        if session_disponible {
            "présente"
        } else {
            "absente"
        }
    );
    conformes += 1;
    conformes
}

/// Signature volontairement fausse : on signe juste, puis on altère le dernier
/// caractère de la valeur `Signature` (64 octets Ed25519 ⇒ refus).
fn signature_alteree(signer: &Signer, host: &str, e: &Engine, corps: &Value) -> Resp {
    let authority = format!("{}:{}", host, e.port);
    let body = corps.to_string();
    let base = [
        ("Content-Type", "application/json"),
        ("Mcp-Method", corps["method"].as_str().unwrap_or("")),
    ];
    let a_signer = RequestToSign::new("POST", "/mcp", &authority, &base, body.as_bytes());
    let signes = signer.headers(&a_signer);
    let mut all: Vec<(String, String)> = base
        .iter()
        .map(|(n, v)| ((*n).to_string(), (*v).to_string()))
        .collect();
    for (n, v) in signes.iter() {
        let valeur = if n.eq_ignore_ascii_case("signature") {
            let mut v = v.to_string();
            let dernier = if v.ends_with(':') { '"' } else { 'A' };
            v.pop();
            v.push(dernier);
            v
        } else {
            v.to_string()
        };
        all.push((n.to_string(), valeur));
    }
    let refs: Vec<(&str, &str)> = all.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
    e.send(
        host,
        e.port,
        "POST",
        "/mcp",
        &refs,
        Some(body.as_bytes()),
        SignMode::None,
    )
}

/// Matrice complète : {sans identité, signature fausse, rejeu, horloge
/// décalée, clé révoquée, client lecture, pilotage, execution} × {loopback,
/// LAN} × {tools/list, outil lecture, outil pilotage, outil execution,
/// `/api/state`, `/api/action`}.
///
/// Attendus (les deux transports se comportent pareil sur /mcp : la signature
/// ne dépend pas du chemin réseau) :
/// - sans identité, signature fausse, horloge décalée, clé révoquée → 401 sur
///   les quatre surfaces /mcp ; outils lecture/pilotage/execution compris.
/// - rejeu → le PREMIER envoi d'un corps signé passe (200 + 10 outils pour
///   `tools/list`), le SECOND avec les mêmes octets rend 401 (nonce consommé).
/// - lecture → 10 outils, outil lecture OK, pilotage et execution 403.
/// - pilotage → 10 outils, outil lecture 403 (droits non hiérarchiques),
///   pilotage OK, execution 403.
/// - execution → 10 outils, lecture 403, pilotage 403, execution OK.
/// - `/api/*` : 200 en loopback (session locale du harnais), 401 en LAN —
///   dans TOUS les cas, la clé MCP n'ouvre rien.
#[test]
fn matrice_identite_transport_surface() {
    let _garde = serial_guard();
    let mut e = spawn(EngineCfg {
        ui_port: free_port(),
        ui_bind: "0.0.0.0",
        api_access: None,
    })
    .expect("spawn moteur LAN");
    e.wait_ready(Duration::from_secs(30)).expect("moteur prêt");
    let lan = e.lan_ip();
    if lan.is_none() {
        eprintln!("SKIP moitié LAN de la matrice : aucune IP LAN non-loopback");
    }

    // Identités de la matrice (appairées avec la session du harnais).
    let c_lecture = client_appaire(&e, &["lecture"]);
    let c_pilotage = client_appaire(&e, &["pilotage"]);
    let c_execution = client_appaire(&e, &["execution"]);
    let c_revoque = client_appaire(&e, &["lecture"]);
    let keyid_revoque = {
        // le keyid du client révoqué est le dernier appairé
        let clients = e.action(&json!({ "action": "client_list" }))["clients"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        clients
            .last()
            .and_then(|c| c["keyid"].as_str())
            .unwrap_or("")
            .to_string()
    };
    let revoque = e.action(&json!({
        "action": "client_revoke", "keyid": keyid_revoque,
    }));
    assert_eq!(revoque["ok"], json!(true), "révocation : {revoque}");
    let c_inconnu = {
        let cle = SigningKey::generate();
        Signer::new(cle).with_keyid(nouveau_keyid())
    };

    let cas_sans_identite = Cas {
        nom: "sans identité",
        attendus: [Attendu::Refus401; 4],
    };
    let cas_signature_fausse = Cas {
        nom: "signature fausse",
        attendus: [Attendu::Refus401; 4],
    };
    let cas_horloge = Cas {
        nom: "horloge décalée",
        attendus: [Attendu::Refus401; 4],
    };
    let cas_revoque = Cas {
        nom: "clé révoquée",
        attendus: [Attendu::Refus401; 4],
    };
    let cas_lecture = Cas {
        nom: "client lecture",
        attendus: [
            Attendu::Liste(10),
            Attendu::Ok,
            Attendu::RefusDroit,
            Attendu::RefusDroit,
        ],
    };
    let cas_pilotage = Cas {
        nom: "client pilotage",
        attendus: [
            Attendu::Liste(10),
            Attendu::RefusDroit,
            Attendu::Ok,
            Attendu::RefusDroit,
        ],
    };
    let cas_execution = Cas {
        nom: "client execution",
        attendus: [
            Attendu::Liste(10),
            Attendu::RefusDroit,
            Attendu::RefusDroit,
            Attendu::Ok,
        ],
    };

    let horloge_decalee = now().saturating_sub(3600);
    let mut conformes = 0usize;
    let mut total = 0usize;
    let mut hosts: Vec<String> = vec!["127.0.0.1".to_string()];
    if let Some(ip) = &lan {
        hosts.push(ip.clone());
    }
    for host in &hosts {
        let session = host == "127.0.0.1";
        let h = host.as_str();
        conformes += execute_cas(
            &e,
            h,
            &cas_sans_identite,
            &|h, c| post_mcp_sans_signature(&e, h, c),
            session,
        );
        total += 6;
        conformes += execute_cas(
            &e,
            h,
            &cas_signature_fausse,
            &|h, c| signature_alteree(&c_execution, h, &e, c),
            session,
        );
        total += 6;
        conformes += execute_cas(
            &e,
            h,
            &cas_horloge,
            &|h, c| post_mcp_signe(&e, &c_lecture, h, c, Some(horloge_decalee), None),
            session,
        );
        total += 6;
        conformes += execute_cas(
            &e,
            h,
            &cas_revoque,
            &|h, c| post_mcp_signe(&e, &c_revoque, h, c, None, None),
            session,
        );
        total += 6;
        conformes += execute_cas(
            &e,
            h,
            &cas_lecture,
            &|h, c| post_mcp_signe(&e, &c_lecture, h, c, None, None),
            session,
        );
        total += 6;
        conformes += execute_cas(
            &e,
            h,
            &cas_pilotage,
            &|h, c| post_mcp_signe(&e, &c_pilotage, h, c, None, None),
            session,
        );
        total += 6;
        conformes += execute_cas(
            &e,
            h,
            &cas_execution,
            &|h, c| post_mcp_signe(&e, &c_execution, h, c, None, None),
            session,
        );
        total += 6;
        // une clé JAMAIS appairée : même refus qu'une signature fausse
        conformes += execute_cas(
            &e,
            h,
            &cas_sans_identite,
            &|h, c| post_mcp_signe(&e, &c_inconnu, h, c, None, None),
            session,
        );
        total += 6;
    }

    // REJEU : le premier envoi passe, le second (octets identiques) est refusé.
    for (i, host) in hosts.iter().enumerate() {
        let h = host.as_str();
        let corps = corps_tools_list(40);
        // nonce distinct par transport : le cache est par `keyid`, un nonce déjà
        // vu sur loopback serait refusé sur le LAN pour une autre raison.
        let nonce = [0x5au8.wrapping_add(i as u8); 16];
        let premier = post_mcp_signe(&e, &c_execution, h, &corps, None, Some(nonce));
        let second = post_mcp_signe(&e, &c_execution, h, &corps, None, Some(nonce));
        assert_eq!(
            premier.status,
            200,
            "rejeu / {h} — premier envoi : {}",
            premier.text()
        );
        assert_eq!(
            nombre_outils(&premier),
            10,
            "un client `execution` ne voit que ses 10 outils"
        );
        assert!(
            est_refus_401(&second),
            "rejeu / {h} — second envoi (même nonce) doit être refusé : {} {}",
            second.status,
            second.text()
        );
        conformes += 6;
        total += 6;
    }

    assert_eq!(
        conformes, total,
        "toutes les cellules doivent être conformes"
    );
    eprintln!("matrice : {conformes}/{total} cellules conformes");
}

/// La session locale n'est JAMAIS valable sur `/mcp` (docs/auth.md §6) : le
/// cookie du harnais présenté à `/mcp` sans signature rend 401.
#[test]
fn session_locale_jamais_valable_sur_mcp() {
    let _garde = serial_guard();
    let e = engine_ready();
    let cookie = e
        .session_cookie()
        .expect("le harnais doit avoir une session")
        .to_string();
    let corps = corps_tools_list(1);
    let body = corps.to_string();
    let entetes = [
        ("Content-Type", "application/json"),
        ("Mcp-Method", "tools/list"),
    ];
    let mut avec_cookie: Vec<(&str, &str)> = entetes.to_vec();
    let valeur = format!("mcc_session={cookie}");
    avec_cookie.push(("Cookie", &valeur));
    let r = e.send(
        "127.0.0.1",
        e.port,
        "POST",
        "/mcp",
        &avec_cookie,
        Some(body.as_bytes()),
        SignMode::None,
    );
    assert!(
        est_refus_401(&r),
        "la session locale ne doit rien ouvrir sur /mcp : {} {}",
        r.status,
        r.text()
    );
    // …alors que la même session sur /api/state passe
    assert_eq!(e.get("/api/state").status, 200);
}

/// Le `X-MCC-Access` hérité (DA3 = a) ne donne que la LECTURE sur `/mcp`, et
/// chaque usage laisse un avertissement au journal. `?access=` n'existe plus.
#[test]
fn legacy_access_en_lecture_seule_et_journalise() {
    let _garde = serial_guard();
    let code = "e2e-m2-legacy";
    let cfg = EngineCfg {
        ui_port: free_port(),
        ui_bind: "0.0.0.0",
        api_access: Some(code.to_string()),
    };
    let mut e = spawn(cfg).expect("spawn moteur LAN");
    e.wait_ready(Duration::from_secs(30)).expect("moteur prêt");
    let host = "127.0.0.1";

    // lecture : tools/list passe avec 10 outils (identité `lecture` implicite)
    let r = e.send(
        host,
        e.port,
        "POST",
        "/mcp",
        &[
            ("Content-Type", "application/json"),
            ("Mcp-Method", "tools/list"),
            ("X-MCC-Access", code),
        ],
        Some(corps_tools_list(1).to_string().as_bytes()),
        SignMode::None,
    );
    assert_eq!(r.status, 200, "legacy : {}", r.text());
    assert_eq!(
        nombre_outils(&r),
        10,
        "un code hérité ne voit que les outils de lecture"
    );
    // écriture : refus de droit, handler non appelé
    let avant = e.get("/api/state").json()["stats"]["total"].clone();
    let appel = corps_tool(
        "add_server",
        json!({ "transport": "http", "port": 30_966 }),
        2,
    );
    let r = e.send(
        host,
        e.port,
        "POST",
        "/mcp",
        &[
            ("Content-Type", "application/json"),
            ("Mcp-Method", "tools/call"),
            ("Mcp-Name", "add_server"),
            ("X-MCC-Access", code),
        ],
        Some(appel.to_string().as_bytes()),
        SignMode::None,
    );
    assert!(
        est_refus_droit(&r),
        "legacy sur un outil execution : {} {}",
        r.status,
        r.text()
    );
    assert_eq!(
        e.get("/api/state").json()["stats"]["total"],
        avant,
        "le handler n'a pas tourné"
    );
    // journal : chaque usage du code hérité est signalé
    let journal = std::fs::read_to_string(e.tmp_dir().join("mcc.log")).unwrap_or_default();
    assert!(
        journal.contains("[auth] legacy_access") && journal.contains("LECTURE SEULE"),
        "avertissement legacy_access attendu au journal"
    );
    // `/api/*` : le code hérité ne donne QUE la lecture, et `?access=` est mort
    assert_eq!(
        e.send(
            host,
            e.port,
            "GET",
            "/api/state",
            &[("X-MCC-Access", code), ("Cookie", "-")],
            None,
            SignMode::None,
        )
        .status,
        200,
        "X-MCC-Access : lecture autorisée"
    );
    assert_eq!(
        e.send(
            host,
            e.port,
            "GET",
            &format!("/api/state?access={code}"),
            &[("Cookie", "-")],
            None,
            SignMode::None,
        )
        .status,
        401,
        "?access= ne donne plus aucun accès (D5 clos)"
    );
    assert_eq!(
        e.send(
            host,
            e.port,
            "POST",
            "/api/action",
            &[
                ("Content-Type", "application/json"),
                ("X-MCC-Request", "1"),
                ("X-MCC-Access", code),
                ("Cookie", "-"),
            ],
            Some(br#"{"action":"client_list"}"#),
            SignMode::None,
        )
        .status,
        403,
        "code hérité : /api/action refusé (lecture seule)"
    );
}

/// Le jeton d'amorçage est à usage unique et la route est locale : un second
/// échange rend 401, et un échange venu du LAN est refusé (403).
#[test]
fn jeton_amorcage_usage_unique_et_local() {
    let _garde = serial_guard();
    let mut e = spawn(EngineCfg {
        ui_port: free_port(),
        ui_bind: "0.0.0.0",
        api_access: None,
    })
    .expect("spawn");
    e.wait_ready(Duration::from_secs(30)).expect("moteur prêt");
    // le harnais a déjà consommé le jeton au démarrage : un rejeu local échoue
    let deja = e.session_cookie().expect("session").to_string();
    assert_eq!(deja.len(), 43);
    // un jeton inventé est refusé
    let r = e.send(
        "127.0.0.1",
        e.port,
        "POST",
        "/api/session",
        &[("Content-Type", "application/json")],
        Some(br#"{"token":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}"#),
        SignMode::None,
    );
    assert_eq!(r.status, 401, "jeton inventé : {}", r.text());
    // depuis le LAN, même un jeton valide est refusé (route locale)
    if let Some(ip) = e.lan_ip() {
        let r = e.send(
            &ip,
            e.port,
            "POST",
            "/api/session",
            &[("Content-Type", "application/json"), ("X-MCC-Request", "1")],
            Some(br#"{"token":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}"#),
            SignMode::None,
        );
        assert_eq!(
            r.status,
            403,
            "échange depuis le LAN doit être refusé : {}",
            r.text()
        );
    }
}

/// Un moteur loopback prêt, identité et session comprises.
fn engine_ready() -> Engine {
    let mut e = spawn(EngineCfg::default()).expect("spawn du moteur");
    e.wait_ready(Duration::from_secs(30)).expect("moteur prêt");
    e
}

// =====================================================================
// Lot S1 — session locale : cookie ET secret (docs/auth.md §6)
// =====================================================================

/// GET /api/state avec EXACTEMENT les éléments de session demandés (cas
/// négatifs de S-1 : ni le harnais ni `app.js` n'ajoutent quoi que ce soit).
fn statut_avec_session(e: &Engine, cookie: Option<&str>, secret: Option<&str>) -> u16 {
    let mut req = format!("GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n", e.port);
    if let Some(c) = cookie {
        req.push_str(&format!("Cookie: mcc_session={c}\r\n"));
    }
    if let Some(s) = secret {
        req.push_str(&format!("X-MCC-Session: {s}\r\n"));
    }
    req.push_str("Connection: close\r\n\r\n");
    e.raw(req.as_bytes(), 0).status
}

/// S-1 : le cookie seul n'ouvre plus `/api/*` ; il faut cookie ET secret, et le
/// secret est comparé (mauvais secret = 401).
#[test]
fn session_exige_cookie_et_secret() {
    let _garde = serial_guard();
    let e = engine_ready();
    let cookie = e.session_cookie().expect("session").to_string();
    let secret = e.session_secret().expect("secret").to_string();
    assert_eq!(
        statut_avec_session(&e, Some(&cookie), None),
        401,
        "cookie seul : refusé (S-1)"
    );
    assert_eq!(
        statut_avec_session(&e, Some(&cookie), Some(&"B".repeat(43))),
        401,
        "cookie + mauvais secret : refusé"
    );
    assert_eq!(
        statut_avec_session(&e, None, Some(&secret)),
        401,
        "secret seul : refusé"
    );
    assert_eq!(
        statut_avec_session(&e, Some(&cookie), Some(&secret)),
        200,
        "cookie + secret : accepté"
    );
}

/// S-1 pt 2 : une 2e fenêtre obtient SA session (canal fichier, jeton à usage
/// unique) sans invalider la 1re.
#[test]
fn deuxieme_fenetre_sans_invalider_la_premiere() {
    let _garde = serial_guard();
    let e = engine_ready();
    let c1 = e.session_cookie().expect("session 1").to_string();
    let s1 = e.session_secret().expect("secret 1").to_string();
    let (c2, s2) = e.open_extra_session().expect("2e session");
    assert_ne!(c1, c2, "deux sessions distinctes");
    assert_ne!(s1, s2, "deux secrets distincts");
    assert_eq!(
        statut_avec_session(&e, Some(&c1), Some(&s1)),
        200,
        "la 1re fenêtre reste valide"
    );
    assert_eq!(
        statut_avec_session(&e, Some(&c2), Some(&s2)),
        200,
        "la 2e fenêtre est valide"
    );
    // le canal fichier ne laisse aucun jeton derrière lui
    assert!(
        !e.tmp_dir().join("mcc-session.req").exists(),
        "demande supprimée dès service"
    );
    assert!(
        !e.tmp_dir().join("mcc-session.grant").exists(),
        "réponse supprimée dès lecture"
    );
}

/// S-1 pt 2 : `POST /api/session/close-others` révoque les autres sessions et
/// garde la courante. Route locale, hors du dispatcher d'actions.
#[test]
fn fermer_les_autres_sessions_garde_la_courante() {
    let _garde = serial_guard();
    let e = engine_ready();
    let c1 = e.session_cookie().expect("session 1").to_string();
    let s1 = e.session_secret().expect("secret 1").to_string();
    let (c2, s2) = e.open_extra_session().expect("2e session");
    let (c3, s3) = e.open_extra_session().expect("3e session");
    // le harnais présente sa session courante (1re) sur la route
    let r = e.post_json("/api/session/close-others", &json!({}));
    assert_eq!(r.status, 200, "close-others : {}", r.text());
    assert_eq!(r.json()["closed"], json!(2), "{}", r.text());
    assert_eq!(
        statut_avec_session(&e, Some(&c1), Some(&s1)),
        200,
        "la session courante survit"
    );
    for (c, s, nom) in [(&c2, &s2, "2e"), (&c3, &s3, "3e")] {
        assert_eq!(
            statut_avec_session(&e, Some(c), Some(s)),
            401,
            "la {nom} session doit être révoquée"
        );
    }
}

/// S-1 pt 3 : un SECOND lancement (mode headless) sur le même `config.json`
/// n'imprime plus une sortie sans session : il obtient un jeton d'amorçage par
/// le canal fichier du dossier de config, et ce jeton ouvre une session.
/// C'est le scénario exact de l'audit (« sortie 0, reused:true, session_url
/// vide »).
#[test]
fn second_lancement_headless_obtient_un_jeton_utilisable() {
    let _garde = serial_guard();
    let e = engine_ready();
    let dir = e.tmp_dir().clone();
    let log = dir.join("second-stdout.log");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mcp-control"));
    cmd.env("MCC_CONFIG", dir.join("config.json"))
        .env("MCC_KEYRING_SERVICE", "")
        .env_remove("MCC_API_ACCESS");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let sortie = std::fs::File::create(&log).expect("log du 2e lancement");
    let mut enfant = cmd
        .stdout(Stdio::from(sortie))
        .stderr(Stdio::null())
        .spawn()
        .expect("2e lancement");
    // Le 2e process rend la main après avoir détecté le moteur et demandé le
    // jeton : on l'attend borné, puis on le tue s'il s'est trompé de chemin.
    let limite = Instant::now() + Duration::from_secs(30);
    let mut fini = false;
    while Instant::now() < limite {
        if enfant
            .try_wait()
            .expect("attente du 2e lancement")
            .is_some()
        {
            fini = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if !fini {
        let _ = enfant.kill();
        let _ = enfant.wait();
        panic!("le 2e lancement ne rend pas la main (moteur doublé ?)");
    }
    // Processus déjà terminé : `wait` récolte le statut (zombie évité).
    let _ = enfant.wait();
    let texte = std::fs::read_to_string(&log).unwrap_or_default();
    let url = texte
        .lines()
        .find(|l| l.starts_with("[session] http://"))
        .unwrap_or_else(|| panic!("aucune ligne [session] utilisable dans : {texte}"))
        .to_string();
    let jeton = url.rsplit("#t=").next().unwrap_or("").trim().to_string();
    assert_eq!(jeton.len(), 43, "URL de session inattendue : {url}");
    // Le jeton du canal fichier est réellement utilisable — une seule fois.
    let corps = format!("{{\"token\":\"{jeton}\"}}");
    let r = e.send(
        "127.0.0.1",
        e.port,
        "POST",
        "/api/session",
        &[("Content-Type", "application/json")],
        Some(corps.as_bytes()),
        SignMode::None,
    );
    assert_eq!(r.status, 200, "jeton du 2e lancement refusé : {}", r.text());
    assert!(
        r.json()["secret"].as_str().is_some_and(|s| s.len() == 43),
        "le 2e lancement doit aussi recevoir un secret de session"
    );
}

/// S-1 pt 2 : plafond de 8 sessions simultanées — la 9e est refusée, les 8
/// premières restent vivantes (échec fermé, pas d'éviction silencieuse).
#[test]
fn plafond_de_huit_sessions_en_e2e() {
    let _garde = serial_guard();
    let e = engine_ready();
    let c1 = e.session_cookie().expect("session 1").to_string();
    let s1 = e.session_secret().expect("secret 1").to_string();
    for n in 2..=8 {
        e.open_extra_session()
            .unwrap_or_else(|err| panic!("session {n} : {err}"));
    }
    assert!(
        e.open_extra_session().is_err(),
        "la 9e session doit être refusée"
    );
    assert_eq!(
        statut_avec_session(&e, Some(&c1), Some(&s1)),
        200,
        "les 8 sessions existantes restent valides"
    );
}

// =====================================================================
// Lot M3 — confirmation humaine avant exécution (docs/auth.md §5)
// =====================================================================

/// Durée du permis de confirmation des moteurs lancés PENDANT le test
/// (`MCC_CONFIRM_SECS`, §5) : dix minutes ne se jouent pas dans une suite de
/// tests. Le `Drop` remet l'environnement en état, même si le test panique.
struct GardeConfirmSecs;

impl GardeConfirmSecs {
    /// Aucune durée imposée : les moteurs du test prennent les 10 min de §5.
    fn defaut() -> Self {
        std::env::remove_var("MCC_E2E_CONFIRM_SECS");
        GardeConfirmSecs
    }

    /// Permis de `secs` secondes (préparation pour éprouver l'expiration).
    fn court(secs: u64) -> Self {
        std::env::set_var("MCC_E2E_CONFIRM_SECS", secs.to_string());
        GardeConfirmSecs
    }
}

impl Drop for GardeConfirmSecs {
    fn drop(&mut self) {
        std::env::remove_var("MCC_E2E_CONFIRM_SECS");
    }
}

/// Programme lancé par ces scénarios : il écrit un marqueur (preuve HORS du
/// moteur que le process a réellement démarré) puis répond juste assez au
/// protocole pour que la sonde stdio le déclare UP.
const MARQUEUR_JS: &str = r#"
const fs = require('fs');
fs.writeFileSync(process.argv[1], 'lance');
let tampon = '';
process.stdin.on('data', d => {
  tampon += d;
  let i;
  while ((i = tampon.indexOf('\n')) >= 0) {
    const ligne = tampon.slice(0, i);
    tampon = tampon.slice(i + 1);
    let m;
    try { m = JSON.parse(ligne); } catch (e) { continue; }
    if (m.id === undefined || m.id === null) continue;
    const r = m.method === 'initialize'
      ? { protocolVersion: '2025-06-18', capabilities: { tools: {} },
          serverInfo: { name: 'marqueur', version: '0.1' } }
      : m.method === 'tools/list' ? { tools: [] } : {};
    process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: m.id, result: r }) + '\n');
  }
});
setInterval(() => {}, 1000);
"#;

/// Le poller ne doit pas lancer le programme à la place de la confirmation :
/// même méthode que `scenarios_actions` (réglage à 1 h, puis on laisse passer
/// le cycle déjà armé — après quoi plus aucune sonde ne touche ces serveurs).
fn poller_endormi(e: &Engine) {
    let rep = e.action(&json!({ "action": "set_settings", "refresh_secs": 3600 }));
    assert_eq!(rep["ok"], json!(true), "set_settings refusé : {rep}");
    std::thread::sleep(Duration::from_secs(8));
}

/// Serveur stdio ajouté par l'API LOCALE (session `admin`, hors confirmation :
/// c'est le point de départ du scénario, pas son sujet).
fn ajoute_marqueur(e: &Engine, nom: &str) -> (String, std::path::PathBuf) {
    let marqueur = e.tmp_dir().join(format!("{nom}.txt"));
    let _ = std::fs::remove_file(&marqueur);
    let rep = e.action(&json!({
        "action": "add", "transport": "stdio", "program": "node",
        "args": ["-e", MARQUEUR_JS, marqueur.display().to_string()],
        "label": nom,
    }));
    assert_eq!(rep["ok"], json!(true), "add stdio refusé : {rep}");
    (
        rep["id"].as_str().expect("id du serveur").to_string(),
        marqueur,
    )
}

/// La demande en attente la plus récente pour `outil`, telle que l'écran
/// « Accès » la montre (session locale).
fn demande_en_attente(e: &Engine, outil: &str) -> Value {
    let l = e.action(&json!({ "action": "access_list" }));
    assert_eq!(l["ok"], json!(true), "access_list refusé : {l}");
    l["pending"]
        .as_array()
        .and_then(|a| a.iter().rev().find(|p| p["tool"] == json!(outil)).cloned())
        .unwrap_or_else(|| panic!("aucune demande en attente pour {outil} : {l}"))
}

/// Le marqueur apparaît-il dans le délai imparti ?
fn marqueur_ecrit(marqueur: &std::path::Path, delai: Duration) -> bool {
    crate::harness::wait_until(delai, || if marqueur.exists() { Some(true) } else { None })
}

/// Critère de fin n° 1 et n° 2 : un appel `execution` sans accord ne lance RIEN
/// (le process est absent du disque, pas seulement de l'état du moteur), reçoit
/// `403` + `-32023` + `approval_pending` + `retryable` + `retryAfterMs`, apparaît
/// sur l'écran « Accès » avec l'effet exact — puis part après approbation
/// locale. Le permis « une fois » est consommé par l'exécution.
#[test]
fn confirmation_rien_ne_part_sans_accord_puis_part() {
    let _garde = serial_guard();
    let _secs = GardeConfirmSecs::defaut();
    let mut e = engine_ready();
    e.set_auto_approve(false);
    poller_endormi(&e);
    let (id, marqueur) = ajoute_marqueur(&e, "m3-t1");
    let client = client_appaire(&e, &["execution"]);
    let appel = |n: u64| {
        post_mcp_signe(
            &e,
            &client,
            "127.0.0.1",
            &corps_tool("start_server", json!({ "id": id }), n),
            None,
            None,
        )
    };

    // 1) aucun accord : refus « en attente », et AUCUN process
    let r = appel(1);
    assert!(
        demande_confirmation(&r),
        "attendu 403/-32023/approval_pending : {} {}",
        r.status,
        r.text()
    );
    std::thread::sleep(Duration::from_millis(800));
    assert!(
        !marqueur.exists(),
        "un process a été lancé avant tout accord : {}",
        marqueur.display()
    );

    // 2) la demande porte l'effet EXACT (programme, arguments, dossier) et
    //    l'identité de l'appelant
    let p = demande_en_attente(&e, "start_server");
    assert_eq!(p["effect"]["program"], json!("node"), "{p}");
    assert_eq!(p["effect"]["args"][0], json!("-e"), "{p}");
    assert_eq!(
        p["effect"]["args"][2],
        json!(marqueur.display().to_string()),
        "{p}"
    );
    assert_eq!(p["address"], json!("127.0.0.1"), "{p}");
    assert!(
        p["fingerprint"]
            .as_str()
            .unwrap_or("")
            .starts_with("ed25519:"),
        "empreinte courte attendue : {p}"
    );

    // 3) approbation par la SEULE session locale, puis réessai : ça part
    let ok = e.action(&json!({
        "action": "confirm_approve", "request_id": p["id"], "mode": "once",
    }));
    assert_eq!(ok["ok"], json!(true), "approbation refusée : {ok}");
    let r = appel(2);
    assert_eq!(r.status, 200, "après accord : {} {}", r.status, r.text());
    assert!(
        r.json().get("result").is_some(),
        "pas de résultat : {}",
        r.text()
    );
    assert!(
        marqueur_ecrit(&marqueur, Duration::from_secs(20)),
        "le programme n'a pas été lancé après l'accord"
    );

    // 4) le permis « une fois » est consommé : le MÊME appel redemande
    let r = appel(3);
    assert!(
        demande_confirmation(&r),
        "permis « une fois » non consommé : {} {}",
        r.status,
        r.text()
    );
    let l = e.action(&json!({ "action": "access_list" }));
    assert!(
        l["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["verdict"] == "accordé (une fois)"),
        "le journal d'accès doit porter l'accord : {l}"
    );
    e.action(&json!({ "action": "kill", "id": id }));
}

/// Critère de fin n° 3 : le permis de 10 minutes (§5) vaut pour ce client, cet
/// outil et ces arguments ; à son échéance, le même appel REDEMANDE — durée
/// réglée à 2 s pour le test, par `MCC_CONFIRM_SECS`.
#[test]
fn permis_expire_et_redemande() {
    let _garde = serial_guard();
    let _secs = GardeConfirmSecs::court(2);
    let mut e = engine_ready();
    e.set_auto_approve(false);
    poller_endormi(&e);
    let (id, marqueur) = ajoute_marqueur(&e, "m3-t2");
    let client = client_appaire(&e, &["execution"]);
    let appel = |n: u64| {
        post_mcp_signe(
            &e,
            &client,
            "127.0.0.1",
            &corps_tool("start_server", json!({ "id": id }), n),
            None,
            None,
        )
    };

    assert!(demande_confirmation(&appel(1)));
    let p = demande_en_attente(&e, "start_server");
    // la durée du permis est bien celle de la configuration de test
    assert_eq!(
        e.action(&json!({ "action": "access_list" }))["window_secs"],
        json!(2),
        "MCC_CONFIRM_SECS n'a pas été pris en compte"
    );
    let ok = e.action(&json!({
        "action": "confirm_approve", "request_id": p["id"], "mode": "window",
    }));
    assert_eq!(ok["ok"], json!(true), "approbation refusée : {ok}");

    // permis valide : l'appel passe
    let r = appel(2);
    assert_eq!(r.status, 200, "permis valide : {} {}", r.status, r.text());
    assert!(marqueur_ecrit(&marqueur, Duration::from_secs(20)));

    // 2 s plus tard : échu, donc redemandé (rien ne s'exécute sans nouvel accord)
    std::thread::sleep(Duration::from_secs(3));
    let r = appel(3);
    assert!(
        demande_confirmation(&r),
        "permis expiré = redemande : {} {}",
        r.status,
        r.text()
    );
    let l = e.action(&json!({ "action": "access_list" }));
    assert!(
        l["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["verdict"] == "permis expiré"),
        "l'expiration doit laisser une ligne au journal d'accès : {l}"
    );
    assert_eq!(
        demande_en_attente(&e, "start_server")["tool"],
        json!("start_server")
    );
    e.action(&json!({ "action": "kill", "id": id }));
}

/// Critère de fin n° 4 : une demande refusée ne lance rien, et le client reçoit
/// `approval_denied` (jamais d'exécution).
#[test]
fn refus_local_ne_lance_rien() {
    let _garde = serial_guard();
    let _secs = GardeConfirmSecs::defaut();
    let mut e = engine_ready();
    e.set_auto_approve(false);
    poller_endormi(&e);
    let (id, marqueur) = ajoute_marqueur(&e, "m3-t3");
    let client = client_appaire(&e, &["execution"]);
    let appel = |n: u64| {
        post_mcp_signe(
            &e,
            &client,
            "127.0.0.1",
            &corps_tool("start_server", json!({ "id": id }), n),
            None,
            None,
        )
    };

    assert!(demande_confirmation(&appel(1)));
    let p = demande_en_attente(&e, "start_server");
    let ok = e.action(&json!({ "action": "confirm_deny", "request_id": p["id"] }));
    assert_eq!(ok["ok"], json!(true), "refus : {ok}");

    let r = appel(2);
    assert_eq!(r.status, 403, "refus local : {} {}", r.status, r.text());
    assert_eq!(
        r.json()["error"]["data"]["reason"].as_str(),
        Some("approval_denied"),
        "{}",
        r.text()
    );
    std::thread::sleep(Duration::from_millis(800));
    assert!(
        !marqueur.exists(),
        "un refus a quand même lancé le programme"
    );
    // le refus est journalisé, localement et dans mcc.log
    let l = e.action(&json!({ "action": "access_list" }));
    assert!(
        l["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["verdict"] == "refusée (local)"),
        "{l}"
    );
    let journal = std::fs::read_to_string(e.tmp_dir().join("mcc.log")).unwrap_or_default();
    assert!(
        journal.contains("[auth] confirmation refusée par la session locale"),
        "refus absent de mcc.log"
    );
}

/// Critère de fin n° 5, et la règle d'isolation de §5 : un autre client ne peut
/// ni approuver ni seulement atteindre une demande — l'approbation n'existe que
/// par la session locale, et `/mcp` n'expose aucun outil de décision.
#[test]
fn approbation_par_un_autre_client_refusee() {
    let _garde = serial_guard();
    let _secs = GardeConfirmSecs::defaut();
    let mut e = engine_ready();
    e.set_auto_approve(false);
    poller_endormi(&e);
    let (id, marqueur) = ajoute_marqueur(&e, "m3-t4");
    let a = client_appaire(&e, &["execution"]);
    let b = client_appaire(&e, &["execution"]);
    let appel = |s: &Signer, n: u64| {
        post_mcp_signe(
            &e,
            s,
            "127.0.0.1",
            &corps_tool("start_server", json!({ "id": id }), n),
            None,
            None,
        )
    };

    assert!(demande_confirmation(&appel(&a, 1)));
    let demande = demande_en_attente(&e, "start_server")["id"].clone();
    let id_demande = demande.as_str().expect("id de demande").to_string();

    // B (identité `execution` valide) tente d'approuver par la route locale :
    // sans la session de la fenêtre, c'est 401 — aucune clé MCP n'y ouvre quoi
    // que ce soit.
    let corps = json!({
        "action": "confirm_approve", "request_id": id_demande, "mode": "window"
    })
    .to_string();
    let r = e.send(
        "127.0.0.1",
        e.port,
        "POST",
        "/api/action",
        &[
            ("Content-Type", "application/json"),
            ("X-MCC-Request", "1"),
            ("Cookie", "-"),
        ],
        Some(corps.as_bytes()),
        SignMode::None,
    );
    assert_eq!(
        r.status,
        401,
        "un client ne peut pas approuver : {} {}",
        r.status,
        r.text()
    );

    // et /mcp n'expose AUCUN outil de décision (ni de lecture de l'écran)
    let tl = post_mcp_signe(&e, &b, "127.0.0.1", &corps_tools_list(2), None, None);
    assert_eq!(
        nombre_outils(&tl),
        10,
        "tools/list du client : {}",
        tl.text()
    );
    let texte = tl.text();
    for interdit in [
        "confirm_approve",
        "confirm_deny",
        "grant_revoke",
        "access_list",
    ] {
        assert!(!texte.contains(interdit), "{interdit} exposé sur /mcp");
    }
    let r = post_mcp_signe(
        &e,
        &b,
        "127.0.0.1",
        &corps_tool(
            "confirm_approve",
            json!({ "request_id": id_demande, "mode": "window" }),
            3,
        ),
        None,
        None,
    );
    // un outil de décision n'existe pas sur /mcp : erreur JSON-RPC -32602
    // (les erreurs MCP voyagent en HTTP 200, seul le corps fait foi)
    let v = r.json();
    assert_eq!(v["error"]["code"], json!(-32602), "{}", r.text());
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("Unknown tool"),
        "{}",
        r.text()
    );
    assert!(v.get("result").is_none(), "{}", r.text());

    // la demande de A attend toujours, et rien n'a été lancé
    assert_eq!(
        demande_en_attente(&e, "start_server")["id"],
        demande,
        "la demande de A ne doit pas avoir bougé"
    );
    std::thread::sleep(Duration::from_millis(800));
    assert!(
        !marqueur.exists(),
        "une tentative de B a lancé le programme"
    );

    // Point 5 du lot : un refus de DROIT ne peut venir que du hook
    // `on_auth_decision` (la politique de confirmation ne produit jamais
    // `right_missing`) — sa ligne dans l'écran prouve le branchement C4.
    let r = post_mcp_signe(
        &e,
        &b,
        "127.0.0.1",
        &corps_tool("list_servers", json!({}), 5),
        None,
        None,
    );
    assert!(
        est_refus_droit(&r),
        "lecture attendue en refus : {}",
        r.text()
    );
    let l = e.action(&json!({ "action": "access_list" }));
    assert!(
        l["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["reason"] == "right_missing" && d["tool"] == "list_servers"),
        "le refus de droit doit passer par on_auth_decision : {l}"
    );

    // la session locale, elle, décide — et tout part
    let ok = e.action(&json!({
        "action": "confirm_approve", "request_id": id_demande, "mode": "window",
    }));
    assert_eq!(ok["ok"], json!(true), "approbation locale : {ok}");
    let r = appel(&a, 4);
    assert_eq!(
        r.status,
        200,
        "après accord local : {} {}",
        r.status,
        r.text()
    );
    assert!(marqueur_ecrit(&marqueur, Duration::from_secs(20)));
    e.action(&json!({ "action": "kill", "id": id }));
}

/// Point 3 et 4 du lot : le bandeau ne transporte QUE des compteurs (aucun texte
/// venu d'un client par `/api/state`), l'écran « Accès » n'est lisible que par la
/// session locale, et un libellé hostile reste une donnée — jamais du HTML.
#[test]
fn bandeau_sans_texte_de_client_et_ecran_local_seulement() {
    let _garde = serial_guard();
    let _secs = GardeConfirmSecs::defaut();
    let mut e = engine_ready();
    e.set_auto_approve(false);
    poller_endormi(&e);
    let (id, _marqueur) = ajoute_marqueur(&e, "m3-t5");
    let hostile = "<img src=x onerror=alert(1)>";
    let client = client_appaire_label(&e, &["execution"], hostile);
    let r = post_mcp_signe(
        &e,
        &client,
        "127.0.0.1",
        &corps_tool("start_server", json!({ "id": id }), 1),
        None,
        None,
    );
    assert!(demande_confirmation(&r));

    // /api/state : compteurs seulement — le libellé hostile n'y figure pas (il
    // ne vit que dans `access_list`, réservé à la session locale, et dans le
    // toast système tronqué)
    let brut = e.get("/api/state").text();
    assert!(
        !brut.contains("onerror"),
        "le libellé du client voyage dans /api/state"
    );
    let v = e.get("/api/state").json();
    let access = v["access"].as_object().expect("access dans /api/state");
    assert_eq!(
        access.len(),
        4,
        "compteurs + revision seulement : {access:?}"
    );
    assert!(
        access.contains_key("revision"),
        "revision attendue dans access : {access:?}"
    );
    for (nom, valeur) in access {
        assert!(valeur.is_u64(), "{nom} doit être un compteur, pas un texte");
    }
    assert_eq!(
        v["access"]["pending"],
        json!(1),
        "bandeau : {}",
        v["access"]
    );

    // l'écran « Accès » est local : hors session, 401 (aucune fuite au LAN)
    let r = e.send(
        "127.0.0.1",
        e.port,
        "POST",
        "/api/action",
        &[
            ("Content-Type", "application/json"),
            ("X-MCC-Request", "1"),
            ("Cookie", "-"),
        ],
        Some(br#"{"action":"access_list"}"#),
        SignMode::None,
    );
    assert_eq!(r.status, 401, "access_list hors session : {}", r.text());

    // avec la session : la demande est là, libellé BRUT (c'est la page qui
    // l'échappe — test `libelle_hostile_jamais_dans_le_html` côté ui.rs)
    let l = e.action(&json!({ "action": "access_list" }));
    assert_eq!(l["pending"][0]["label"], json!(hostile), "{l}");
    assert_eq!(l["pending"][0]["tool"], json!("start_server"));
    assert!(l["clients"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["label"] == json!(hostile)));
    e.action(&json!({ "action": "kill", "id": id }));
}

// =====================================================================
// Lot M5 — pont stdio, journal d'audit chaîné, anti-force brute,
// uniformité des refus
// =====================================================================

fn spawn_hidden_cmd(bin: &str) -> Command {
    let mut cmd = Command::new(bin);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

/// Appaire un poste client via le vrai CLI `mcp-control auth pair` dans un
/// répertoire isolé (`MCC_CONFIG` + `MCC_KEYRING_SERVICE=""`) et rend son `keyid`.
fn appairer_cli(e: &Engine, client_cfg: &std::path::Path, rights: &[&str]) -> String {
    let enfant = spawn_hidden_cmd(env!("CARGO_BIN_EXE_mcp-control"))
        .args(["auth", "pair", e.base_url()])
        .env("MCC_CONFIG", client_cfg)
        .env("MCC_KEYRING_SERVICE", "")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("lancement auth pair");

    let limite = Instant::now() + Duration::from_secs(20);
    let pair = loop {
        let v = e.action(&json!({ "action": "pair_list" }));
        if let Some(p) = v["pending"].as_array().and_then(|a| a.first()) {
            break p.clone();
        }
        assert!(Instant::now() < limite, "demande d'appairage absente : {v}");
        std::thread::sleep(Duration::from_millis(150));
    };
    let keyid = pair["keyid"].as_str().unwrap_or("").to_string();
    let pair_id = pair["pair_id"].as_str().unwrap_or("").to_string();
    let ok = e.action(&json!({
        "action": "pair_approve",
        "pair_id": pair_id,
        "rights": rights,
    }));
    assert_eq!(ok["ok"], json!(true), "pair_approve refusé : {ok}");
    let sortie = enfant.wait_with_output().expect("attente auth pair");
    assert!(
        sortie.status.success(),
        "auth pair doit réussir : stderr={}",
        String::from_utf8_lossy(&sortie.stderr)
    );
    keyid
}

/// (Lot M5 point 1) Pont stdio `mcp-control bridge` :
/// - négatif : non appairé → stdout vide, message clair sur stderr (`auth pair`)
/// - positif : `initialize`, `notifications/initialized` (muet sur stdout),
///   `tools/list` (10 outils `lecture`), `tools/call` `list_servers` (200),
///   `tools/call` `add_server` (403 `right_missing`) ; stdout = JSON-RPC seul
/// - positif sans `--url` : lit `ui_port` dans `config.json`
/// - négatif : client révoqué → erreur JSON-RPC `-32023` explicite sur stdout
#[test]
fn pont_stdio_pilote_le_gateway_sans_cle_et_stdout_jsonrpc_seul() {
    use std::io::Write as _;
    let _garde = serial_guard();
    let e = engine_ready();
    let client_dir =
        std::env::temp_dir().join(format!("mcc-e2e-bridge-{}-{}", std::process::id(), e.port));
    let _ = std::fs::remove_dir_all(&client_dir);
    std::fs::create_dir_all(&client_dir).unwrap();
    let client_cfg = client_dir.join("config.json");
    // Config client minimale UTF-8 sans BOM (sert aussi au lancement sans --url).
    std::fs::write(
        &client_cfg,
        json!({ "settings": { "ui_port": e.port } }).to_string(),
    )
    .unwrap();

    // 1. Négatif : non appairé → sortie non nulle, stdout VIDE, diagnostic clair sur stderr.
    let non_appaire = spawn_hidden_cmd(env!("CARGO_BIN_EXE_mcp-control"))
        .args(["bridge", "--url", e.base_url()])
        .env("MCC_CONFIG", &client_cfg)
        .env("MCC_KEYRING_SERVICE", "")
        .stdin(Stdio::null())
        .output()
        .expect("lancement bridge non appairé");
    assert!(
        !non_appaire.status.success(),
        "un pont non appairé doit échouer"
    );
    assert!(
        non_appaire.stdout.is_empty(),
        "stdout doit être vide si non appairé : {:?}",
        String::from_utf8_lossy(&non_appaire.stdout)
    );
    let err_txt = String::from_utf8_lossy(&non_appaire.stderr);
    assert!(
        err_txt.contains("mcp-control auth pair"),
        "message actionnable attendu sur stderr : {err_txt}"
    );

    // 2. Appairage `lecture` avec le vrai CLI, puis échange complet via `bridge --url`.
    let keyid = appairer_cli(&e, &client_cfg, &["lecture"]);
    let mut pont = spawn_hidden_cmd(env!("CARGO_BIN_EXE_mcp-control"))
        .args(["bridge", "--url", e.base_url()])
        .env("MCC_CONFIG", &client_cfg)
        .env("MCC_KEYRING_SERVICE", "")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("lancement bridge");
    {
        let mut entree = pont.stdin.take().expect("stdin du pont");
        let messages = [
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": { "name": "e2e-stdio", "version": "0.1" }
                }
            }),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
            json!({
                "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": { "name": "list_servers", "arguments": {} }
            }),
            json!({
                "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                "params": { "name": "add_server", "arguments": { "transport": "http", "port": 30_955 } }
            }),
        ];
        for m in messages {
            writeln!(entree, "{}", m).unwrap();
        }
        let _ = entree.flush();
    }
    let out = pont.wait_with_output().expect("sortie bridge");
    assert!(
        out.status.success(),
        "bridge doit sortir en 0 : stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout_txt = String::from_utf8_lossy(&out.stdout);
    let stderr_txt = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr_txt.contains("bridge : stdio"),
        "diagnostics attendus sur stderr : {stderr_txt}"
    );
    let lignes: Vec<&str> = stdout_txt
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    assert_eq!(
        lignes.len(),
        4,
        "4 réponses attendues (notification muette) sur stdout :\n{stdout_txt}"
    );
    let parsed: Vec<Value> = lignes
        .iter()
        .map(|l| {
            let v: Value = serde_json::from_str(l)
                .unwrap_or_else(|err| panic!("ligne stdout non JSON-RPC ({err}) : {l}"));
            assert_eq!(v["jsonrpc"], json!("2.0"), "pas JSON-RPC 2.0 : {l}");
            v
        })
        .collect();
    assert_eq!(parsed[0]["id"], json!(1));
    assert!(
        parsed[0].get("result").is_some(),
        "initialize : {}",
        parsed[0]
    );
    assert_eq!(parsed[1]["id"], json!(2));
    assert_eq!(
        parsed[1]["result"]["tools"].as_array().map(Vec::len),
        Some(10),
        "tools/list lecture = 10 outils : {}",
        parsed[1]
    );
    assert_eq!(parsed[2]["id"], json!(3));
    assert!(
        parsed[2].get("result").is_some(),
        "list_servers : {}",
        parsed[2]
    );
    assert_eq!(parsed[3]["id"], json!(4));
    assert_eq!(parsed[3]["error"]["code"], json!(-32023));
    assert_eq!(parsed[3]["error"]["data"]["reason"], json!("right_missing"));

    // 3. Positif sans `--url` : le pont lit `settings.ui_port` dans config.json.
    let mut pont_auto = spawn_hidden_cmd(env!("CARGO_BIN_EXE_mcp-control"))
        .arg("bridge")
        .env("MCC_CONFIG", &client_cfg)
        .env("MCC_KEYRING_SERVICE", "")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("lancement bridge sans --url");
    {
        let mut entree = pont_auto.stdin.take().unwrap();
        writeln!(
            entree,
            "{}",
            json!({"jsonrpc":"2.0","id":9,"method":"tools/list"})
        )
        .unwrap();
    }
    let out_auto = pont_auto.wait_with_output().unwrap();
    assert!(out_auto.status.success());
    let rep_auto: Value =
        serde_json::from_str(String::from_utf8_lossy(&out_auto.stdout).trim()).unwrap();
    assert_eq!(rep_auto["id"], json!(9));
    assert_eq!(
        rep_auto["result"]["tools"].as_array().map(Vec::len),
        Some(10)
    );

    // 4. Négatif : révocation immédiate → le pont rend -32023 `unauthenticated`.
    let rev = e.action(&json!({ "action": "client_revoke", "keyid": keyid }));
    assert_eq!(rev["ok"], json!(true), "{rev}");
    let mut pont_rev = spawn_hidden_cmd(env!("CARGO_BIN_EXE_mcp-control"))
        .args(["bridge", "--url", e.base_url()])
        .env("MCC_CONFIG", &client_cfg)
        .env("MCC_KEYRING_SERVICE", "")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("lancement bridge révoqué");
    {
        let mut entree = pont_rev.stdin.take().unwrap();
        writeln!(
            entree,
            "{}",
            json!({"jsonrpc":"2.0","id":77,"method":"tools/list"})
        )
        .unwrap();
    }
    let out_rev = pont_rev.wait_with_output().unwrap();
    let v_rev: Value =
        serde_json::from_str(String::from_utf8_lossy(&out_rev.stdout).trim()).unwrap();
    assert_eq!(v_rev["id"], json!(77));
    assert_eq!(v_rev["error"]["code"], json!(-32023));
    assert_eq!(v_rev["error"]["data"]["reason"], json!("unauthenticated"));
    assert!(
        v_rev["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("mcp-control auth pair"),
        "{v_rev}"
    );
    let _ = std::fs::remove_dir_all(&client_dir);
}

/// (Lot M5 point 2) Journal d'audit chaîné `auth-audit.log` + `auth verify-log` :
/// - distinct de `mcc.log`, repli `auth/audit.key` journalisé et ACL restreinte,
///   aucun argument d'outil ni secret dans le journal
/// - `mcp-control auth verify-log` vert sur journal intact, rouge sur les 3
///   falsifications (ligne modifiée, ligne supprimée, ligne insérée)
#[test]
fn journal_audit_chaine_et_verify_log_trois_falsifications() {
    let _garde = serial_guard();
    let mut e = engine_ready();
    e.set_auto_approve(false);

    let c_lec = client_appaire(&e, &["lecture"]);
    let c_exec = client_appaire(&e, &["execution"]);
    let c_rev = client_appaire(&e, &["lecture"]);
    let keyid_rev = clients(&e)
        .last()
        .and_then(|c| c["keyid"].as_str())
        .unwrap()
        .to_string();
    assert_eq!(
        e.action(&json!({ "action": "client_revoke", "keyid": keyid_rev }))["ok"],
        json!(true)
    );
    let c_inconnu = Signer::new(SigningKey::generate()).with_keyid(nouveau_keyid());

    // Décisions variées : allowed, right_missing, approval_pending, unauthenticated (×4)
    let r_ok = post_mcp_signe(
        &e,
        &c_lec,
        "127.0.0.1",
        &corps_tool("list_servers", json!({}), 1),
        None,
        None,
    );
    assert_eq!(r_ok.status, 200);
    let r_right = post_mcp_signe(
        &e,
        &c_lec,
        "127.0.0.1",
        &corps_tool("refresh", json!({}), 2),
        None,
        None,
    );
    assert!(est_refus_droit(&r_right));
    let r_pend = post_mcp_signe(
        &e,
        &c_exec,
        "127.0.0.1",
        &corps_tool(
            "add_server",
            json!({ "transport": "http", "port": 30_988, "label": "SECRET_ARG_M5" }),
            3,
        ),
        None,
        None,
    );
    assert!(demande_confirmation(&r_pend));
    assert_eq!(
        post_mcp_sans_signature(&e, "127.0.0.1", &corps_tools_list(4)).status,
        401
    );
    assert_eq!(
        signature_alteree(&c_lec, "127.0.0.1", &e, &corps_tools_list(5)).status,
        401
    );
    assert_eq!(
        post_mcp_signe(
            &e,
            &c_inconnu,
            "127.0.0.1",
            &corps_tools_list(6),
            None,
            None
        )
        .status,
        401
    );
    assert_eq!(
        post_mcp_signe(&e, &c_rev, "127.0.0.1", &corps_tools_list(7), None, None).status,
        401
    );

    let audit_path = e.tmp_dir().join("auth-audit.log");
    let mcc_log_path = e.tmp_dir().join("mcc.log");
    let audit_key_path = e.tmp_dir().join("auth").join("audit.key");
    assert!(audit_path.exists(), "auth-audit.log attendu");
    assert!(mcc_log_path.exists(), "mcc.log attendu");
    assert!(audit_key_path.exists(), "auth/audit.key repli attendu");

    let mcc_log = std::fs::read_to_string(&mcc_log_path).unwrap();
    assert!(
        mcc_log.contains("[audit] clé HMAC du journal"),
        "le repli fichier de la clé audit doit être journalisé : {mcc_log}"
    );
    let audit_txt = std::fs::read_to_string(&audit_path).unwrap();
    for attendu in [
        "verdict=allowed",
        "verdict=right_missing",
        "verdict=approval_pending",
        "verdict=unauthenticated",
        "reason=signature_invalid",
        "reason=key_unknown",
        "reason=key_revoked",
    ] {
        assert!(
            audit_txt.contains(attendu),
            "{attendu} absent :\n{audit_txt}"
        );
    }
    for interdit in ["30988", "SECRET_ARG_M5", "PRIVATE KEY", "nonce=", "sig1=:"] {
        assert!(
            !audit_txt.contains(interdit),
            "fuite « {interdit} » interdite dans auth-audit.log :\n{audit_txt}"
        );
    }

    // `verify-log` via MCC_CONFIG et via `--path` : tous deux verts.
    let cfg_path = e.tmp_dir().join("config.json");
    let v_ok = spawn_hidden_cmd(env!("CARGO_BIN_EXE_mcp-control"))
        .args(["auth", "verify-log"])
        .env("MCC_CONFIG", &cfg_path)
        .env("MCC_KEYRING_SERVICE", "")
        .output()
        .expect("verify-log");
    assert!(
        v_ok.status.success(),
        "verify-log doit réussir : {}",
        String::from_utf8_lossy(&v_ok.stderr)
    );
    assert!(String::from_utf8_lossy(&v_ok.stdout).contains("INTACTE"));

    let v_path = spawn_hidden_cmd(env!("CARGO_BIN_EXE_mcp-control"))
        .args(["auth", "verify-log", "--path"])
        .arg(&audit_path)
        .env("MCC_KEYRING_SERVICE", "")
        .output()
        .expect("verify-log --path");
    assert!(v_path.status.success());

    // Les 3 falsifications dans un dossier isolé (copie du journal + clé).
    let run_tamper = |nom: &str, lignes: Vec<String>| -> std::process::Output {
        let d = e.tmp_dir().join(format!("tamper-{nom}"));
        std::fs::create_dir_all(d.join("auth")).unwrap();
        std::fs::copy(&audit_key_path, d.join("auth").join("audit.key")).unwrap();
        let p = d.join("auth-audit.log");
        std::fs::write(&p, format!("{}\n", lignes.join("\n"))).unwrap();
        spawn_hidden_cmd(env!("CARGO_BIN_EXE_mcp-control"))
            .args(["auth", "verify-log", "--path"])
            .arg(&p)
            .env("MCC_KEYRING_SERVICE", "")
            .output()
            .expect("verify-log falsifié")
    };
    let orig: Vec<String> = audit_txt.lines().map(String::from).collect();
    assert!(
        orig.len() >= 5,
        "au moins 5 lignes attendues : {}",
        orig.len()
    );

    // 1. Ligne modifiée
    let mut mod_l = orig.clone();
    mod_l[1] = mod_l[1].replace("verdict=right_missing", "verdict=allowed");
    let out1 = run_tamper("mod", mod_l);
    assert!(!out1.status.success(), "modification non détectée");
    let s1 = String::from_utf8_lossy(&out1.stderr);
    assert!(s1.contains("MODIFIÉE") && s1.contains("FALSIFIÉ"), "{s1}");

    // 2. Ligne supprimée (milieu + première)
    let mut del_l = orig.clone();
    del_l.remove(2);
    let out2 = run_tamper("del", del_l);
    assert!(!out2.status.success(), "suppression non détectée");
    let s2 = String::from_utf8_lossy(&out2.stderr);
    assert!(s2.contains("SUPPRIMÉE") && s2.contains("FALSIFIÉ"), "{s2}");

    let mut del_first = orig.clone();
    del_first.remove(0);
    let out2b = run_tamper("del1", del_first);
    assert!(!out2b.status.success(), "suppression ligne 1 non détectée");

    // 3. Ligne insérée
    let mut ins_l = orig.clone();
    let dup = ins_l[1].clone();
    ins_l.insert(2, dup);
    let out3 = run_tamper("ins", ins_l);
    assert!(!out3.status.success(), "insertion non détectée");
    let s3 = String::from_utf8_lossy(&out3.stderr);
    assert!(s3.contains("INSÉRÉE") && s3.contains("FALSIFIÉ"), "{s3}");
}

fn percentile_us(sorted: &[u128], num: usize, den: usize) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = (sorted.len() - 1) * num / den;
    sorted[idx]
}

/// (Lot M5 point 3) Anti-force brute :
/// - compteurs par IP et par `keyid`, 405/415 pré-auth ne remet jamais à zéro
/// - délai croissant à partir du 8e échec, blocage `429` + `Retry-After` au 40e
/// - loopback jamais bloqué par les échecs LAN sur le même `keyid`
/// - tempête soutenue pendant 60 s et mesure de la fluidité de la fenêtre locale
#[test]
fn anti_force_brute_lan_et_tempete_60s_fenetre_locale_fluide() {
    let _garde = serial_guard();
    let mut e = spawn(EngineCfg {
        ui_port: free_port(),
        ui_bind: "0.0.0.0",
        api_access: None,
    })
    .expect("spawn moteur LAN");
    e.wait_ready(Duration::from_secs(30)).expect("moteur prêt");
    let Some(lan) = e.lan_ip() else {
        eprintln!("SKIP anti_force_brute LAN : aucune IP LAN non-loopback");
        return;
    };

    let c_victim = client_appaire(&e, &["lecture"]);
    let corps_tl = corps_tools_list(1);
    let body_bytes = corps_tl.to_string().into_bytes();

    // 1. 10 échecs depuis le LAN sur `c_victim` : les 2 derniers subissent le délai (>= 20 ms).
    for i in 0..10 {
        let t0 = Instant::now();
        let r = signature_alteree(&c_victim, &lan, &e, &corps_tl);
        let dt = t0.elapsed();
        assert_eq!(r.status, 401, "échec {i} : {}", r.text());
        if i >= 8 {
            assert!(
                dt >= Duration::from_millis(20),
                "délai attendu après le 8e échec (échec {i}, mesuré {dt:?})"
            );
        }
    }

    // 2. Un 405 (`GET /mcp`) et un 415 (`text/plain`) ne remettent PAS le compteur à zéro.
    assert_eq!(
        e.send(&lan, e.port, "GET", "/mcp", &[], None, SignMode::None)
            .status,
        405
    );
    assert_eq!(
        e.send(
            &lan,
            e.port,
            "POST",
            "/mcp",
            &[("Content-Type", "text/plain")],
            Some(b"nope"),
            SignMode::None
        )
        .status,
        415
    );

    // 3. 30 échecs de plus (total = 40) → blocage 429 avec Retry-After au 41e.
    for i in 10..40 {
        let r = signature_alteree(&c_victim, &lan, &e, &corps_tl);
        assert_eq!(r.status, 401, "échec {i} : {}", r.text());
    }
    let bloque = signature_alteree(&c_victim, &lan, &e, &corps_tl);
    assert_eq!(
        bloque.status,
        429,
        "blocage 429 attendu : {}",
        bloque.text()
    );
    assert!(
        bloque
            .header("retry-after")
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|s| s >= 1),
        "Retry-After attendu : {:?}",
        bloque.headers
    );

    // 4. La MÊME clé depuis `127.0.0.1` passe toujours en 200 (loopback exempté).
    let local_ok = post_mcp_signe(&e, &c_victim, "127.0.0.1", &corps_tl, None, None);
    assert_eq!(
        local_ok.status,
        200,
        "loopback bloqué par le LAN : {}",
        local_ok.text()
    );
    // Une autre clé depuis la MÊME adresse LAN bloquée est aussi refusée en 429.
    let c_autre = client_appaire(&e, &["lecture"]);
    let autre_bloque = post_mcp_signe(&e, &c_autre, &lan, &corps_tl, None, None);
    assert_eq!(
        autre_bloque.status,
        429,
        "adresse LAN bloquée doit bloquer toute clé : {}",
        autre_bloque.text()
    );

    // 5. Mesure soutenue pendant 60 s sur un moteur LAN neuf + sonde continue de la fenêtre locale.
    drop(e);
    let mut e2 = spawn(EngineCfg {
        ui_port: free_port(),
        ui_bind: "0.0.0.0",
        api_access: None,
    })
    .expect("spawn moteur LAN 60s");
    e2.wait_ready(Duration::from_secs(30)).expect("moteur prêt");
    let c_atk = SigningKey::generate();
    let atk_signer = Signer::new(c_atk).with_keyid(nouveau_keyid());
    let lan_addr: std::net::SocketAddr = format!("{}:{}", lan, e2.port).parse().unwrap();
    let local_addr: std::net::SocketAddr = format!("127.0.0.1:{}", e2.port).parse().unwrap();

    // Pré-construit une requête signée altérée vers le LAN.
    let authority_lan = format!("{}:{}", lan, e2.port);
    let base_headers = [
        ("Content-Type", "application/json"),
        ("Mcp-Method", "tools/list"),
    ];
    let a_signer = RequestToSign::new("POST", "/mcp", &authority_lan, &base_headers, &body_bytes);
    let signes = atk_signer.headers(&a_signer);
    let mut raw_atk = format!(
        "POST /mcp HTTP/1.1\r\nHost: {authority_lan}\r\nContent-Type: application/json\r\nMcp-Method: tools/list\r\n"
    );
    for (n, v) in signes.iter() {
        let mut val = v.to_string();
        if n.eq_ignore_ascii_case("signature") {
            let dernier = if val.ends_with(':') { '"' } else { 'A' };
            val.pop();
            val.push(dernier);
        }
        raw_atk.push_str(&format!("{n}: {val}\r\n"));
    }
    raw_atk.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body_bytes.len()
    ));
    let mut raw_bytes = raw_atk.into_bytes();
    raw_bytes.extend_from_slice(&body_bytes);

    let cookie = e2.session_cookie().unwrap().to_string();
    let secret = e2.session_secret().unwrap().to_string();
    let raw_local = format!(
        "GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nCookie: mcc_session={cookie}\r\nX-MCC-Session: {secret}\r\nConnection: close\r\n\r\n",
        e2.port
    )
    .into_bytes();

    let fin = Instant::now() + Duration::from_secs(60);
    let atk_401 = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let atk_429 = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let atk_other = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let lat_429 = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u128>::new()));

    let mut handles = Vec::new();
    let lan_host_str = lan.clone();
    let lan_port_u16 = e2.port;
    for _ in 0..2 {
        let req = raw_bytes.clone();
        let lan_h = lan_host_str.clone();
        let c401 = std::sync::Arc::clone(&atk_401);
        let c429 = std::sync::Arc::clone(&atk_429);
        let coth = std::sync::Arc::clone(&atk_other);
        let l429 = std::sync::Arc::clone(&lat_429);
        handles.push(std::thread::spawn(move || {
            use std::io::Write as _;
            while Instant::now() < fin {
                let t0 = Instant::now();
                let mut status = 0u16;
                if cfg!(feature = "tls") {
                    if let Ok((data, _)) = mcp_control::auth::net::https_exchange_raw(
                        &lan_h,
                        lan_port_u16,
                        &req,
                        Duration::from_secs(5),
                        None,
                        4096,
                    ) {
                        let head = String::from_utf8_lossy(&data);
                        status = head
                            .lines()
                            .next()
                            .and_then(|l| l.split_whitespace().nth(1))
                            .and_then(|p| p.parse::<u16>().ok())
                            .unwrap_or(0);
                    }
                } else if let Ok(mut s) =
                    std::net::TcpStream::connect_timeout(&lan_addr, Duration::from_secs(2))
                {
                    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
                    let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
                    if s.write_all(&req).is_ok() {
                        let mut buf = [0u8; 512];
                        if let Ok(n) = s.read(&mut buf) {
                            let head = String::from_utf8_lossy(&buf[..n]);
                            status = head
                                .lines()
                                .next()
                                .and_then(|l| l.split_whitespace().nth(1))
                                .and_then(|p| p.parse::<u16>().ok())
                                .unwrap_or(0);
                        }
                    }
                }
                let us = t0.elapsed().as_micros();
                match status {
                    401 => {
                        c401.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    429 => {
                        c429.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if let Ok(mut g) = l429.lock() {
                            g.push(us);
                        }
                        // Cadence soutenue (~120 req/s sur 2 fils) sans saturer les ports TIME_WAIT OS.
                        std::thread::sleep(Duration::from_millis(15));
                    }
                    _ => {
                        coth.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        std::thread::sleep(Duration::from_millis(15));
                    }
                }
            }
        }));
    }

    // Sonde continue de la fenêtre locale (/api/state) pendant les 60 s.
    let mut local_ok_n = 0u64;
    let mut local_ko_n = 0u64;
    let mut local_us = Vec::<u128>::new();
    while Instant::now() < fin {
        let t0 = Instant::now();
        let mut status = 0u16;
        if let Ok(mut s) = std::net::TcpStream::connect_timeout(&local_addr, Duration::from_secs(2))
        {
            use std::io::Write as _;
            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
            if s.write_all(&raw_local).is_ok() {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                while let Ok(n) = s.read(&mut tmp) {
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                }
                let head = String::from_utf8_lossy(&buf);
                status = head
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|p| p.parse::<u16>().ok())
                    .unwrap_or(0);
            }
        }
        let us = t0.elapsed().as_micros();
        if status == 200 {
            local_ok_n += 1;
            local_us.push(us);
        } else {
            local_ko_n += 1;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    for h in handles {
        let _ = h.join();
    }

    let n401 = atk_401.load(std::sync::atomic::Ordering::Relaxed);
    let n429 = atk_429.load(std::sync::atomic::Ordering::Relaxed);
    let noth = atk_other.load(std::sync::atomic::Ordering::Relaxed);
    let mut l429 = lat_429.lock().unwrap().clone();
    l429.sort_unstable();
    local_us.sort_unstable();
    let local_p50 = percentile_us(&local_us, 50, 100);
    let local_p95 = percentile_us(&local_us, 95, 100);
    let local_max = local_us.last().copied().unwrap_or(0);
    let p50_429 = percentile_us(&l429, 50, 100);
    let p95_429 = percentile_us(&l429, 95, 100);
    eprintln!(
        "[M5 pt3 60s] LAN: 401={n401}, 429={n429} (p50={} us, p95={} us), autres={noth} | Fenêtre locale: 200={local_ok_n}, ko={local_ko_n}, p50={local_p50} us, p95={local_p95} us, max={local_max} us",
        p50_429, p95_429
    );

    assert!((40..=120).contains(&n401), "401 hors plage : {n401}");
    assert!(n429 >= 1000, "trop peu de 429 en 60 s : {n429}");
    assert_eq!(noth, 0, "statuts inattendus sur le LAN : {noth}");
    assert_eq!(
        local_ko_n, 0,
        "la fenêtre locale a échoué {local_ko_n} fois"
    );
    assert!(
        local_ok_n >= 400,
        "trop peu de sondes locales : {local_ok_n}"
    );
    assert!(
        local_p95 < 200_000,
        "fenêtre locale non fluide : p95={local_p95} us"
    );
}

/// (Lot M5 point 4) Uniformité des refus sur 1 000 requêtes de chaque sorte :
/// `key_unknown`, `signature_invalid`, `key_revoked`.
/// Même statut (401), même en-tête `WWW-Authenticate`, même corps octet pour
/// octet, et temps de réponse comparable.
#[test]
fn uniformite_refus_corps_statut_et_temps_1000_requetes() {
    let _garde = serial_guard();
    let e = engine_ready();

    let c_inconnu = Signer::new(SigningKey::generate()).with_keyid(nouveau_keyid());
    let c_actif = client_appaire(&e, &["lecture"]);
    let keyid_actif = clients(&e)
        .last()
        .and_then(|c| c["keyid"].as_str())
        .unwrap()
        .to_string();
    // Mauvaise clé privée signant sous le `keyid` de `c_actif` : passe tous
    // les contrôles amont (en-têtes, digest, horloge, nonce, lookup) et paie
    // la vraie vérification Ed25519 à l'étape 8 (`signature_invalid`).
    let c_fausse_sig = Signer::new(SigningKey::generate()).with_keyid(keyid_actif);
    let _ = c_actif;
    let c_revoque = client_appaire(&e, &["lecture"]);
    let keyid_rev = clients(&e)
        .last()
        .and_then(|c| c["keyid"].as_str())
        .unwrap()
        .to_string();
    assert_eq!(
        e.action(&json!({ "action": "client_revoke", "keyid": keyid_rev }))["ok"],
        json!(true)
    );

    let authority = format!("127.0.0.1:{}", e.port);
    let body = corps_tools_list(1).to_string();
    let body_bytes = body.as_bytes();
    let base = [
        ("Content-Type", "application/json"),
        ("Mcp-Method", "tools/list"),
    ];

    let created_base = now() + 30;
    let build_raw = |signer: &Signer, idx: usize| -> Vec<u8> {
        let req = RequestToSign::new("POST", "/mcp", &authority, &base, body_bytes);
        let mut nonce = [0u8; 16];
        nonce[..8].copy_from_slice(&(idx as u64).to_le_bytes());
        nonce[8..].copy_from_slice(&0xA5A5_5A5A_u64.to_le_bytes());
        let signes = signer.headers_at(&req, created_base, &nonce);
        let mut s = format!(
            "POST /mcp HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/json\r\nMcp-Method: tools/list\r\n"
        );
        for (n, v) in signes.iter() {
            s.push_str(&format!("{n}: {v}\r\n"));
        }
        s.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            body_bytes.len()
        ));
        let mut bytes = s.into_bytes();
        bytes.extend_from_slice(body_bytes);
        bytes
    };

    const WARMUP: usize = 30;
    const N: usize = 1000;
    let mut reqs_unknown = Vec::with_capacity(WARMUP + N);
    let mut reqs_invalid = Vec::with_capacity(WARMUP + N);
    let mut reqs_revoked = Vec::with_capacity(WARMUP + N);
    for i in 0..(WARMUP + N) {
        reqs_unknown.push(build_raw(&c_inconnu, i));
        reqs_invalid.push(build_raw(&c_fausse_sig, i));
        reqs_revoked.push(build_raw(&c_revoque, i));
    }

    let addr: std::net::SocketAddr = authority.parse().unwrap();
    let send_measure = |raw: &[u8]| -> (Resp, u128) {
        use std::io::Write as _;
        let t0 = Instant::now();
        let mut s = std::net::TcpStream::connect(addr).expect("connect");
        s.set_read_timeout(Some(Duration::from_secs(5))).ok();
        s.write_all(raw).expect("write");
        let mut data = Vec::new();
        let _ = s.read_to_end(&mut data);
        let us = t0.elapsed().as_micros();
        // Parse minimal identique à harness::Resp
        let raw_txt = String::from_utf8_lossy(&data);
        let (head, body_part) = raw_txt.split_once("\r\n\r\n").unwrap_or((&raw_txt, ""));
        let mut lines = head.split("\r\n");
        let status = lines
            .next()
            .unwrap_or("")
            .split_whitespace()
            .nth(1)
            .and_then(|x| x.parse::<u16>().ok())
            .unwrap_or(0);
        let mut headers = Vec::new();
        for l in lines {
            if let Some((n, v)) = l.split_once(':') {
                headers.push((n.trim().to_ascii_lowercase(), v.trim().to_string()));
            }
        }
        (
            Resp {
                status,
                headers,
                body: body_part.as_bytes().to_vec(),
            },
            us,
        )
    };

    // Chauffe
    for i in 0..WARMUP {
        assert_eq!(send_measure(&reqs_unknown[i]).0.status, 401);
        assert_eq!(send_measure(&reqs_invalid[i]).0.status, 401);
        assert_eq!(send_measure(&reqs_revoked[i]).0.status, 401);
    }

    let mut t_unknown = Vec::with_capacity(N);
    let mut t_invalid = Vec::with_capacity(N);
    let mut t_revoked = Vec::with_capacity(N);
    let mut ref_body: Option<Vec<u8>> = None;

    for i in WARMUP..(WARMUP + N) {
        for (req, bucket, nom) in [
            (&reqs_unknown[i], &mut t_unknown, "key_unknown"),
            (&reqs_invalid[i], &mut t_invalid, "signature_invalid"),
            (&reqs_revoked[i], &mut t_revoked, "key_revoked"),
        ] {
            let (resp, us) = send_measure(req);
            assert_eq!(resp.status, 401, "{nom} statut != 401");
            assert_eq!(
                resp.header("www-authenticate").as_deref(),
                Some("Signature realm=\"litemcp\", tag=\"litemcp-auth-v1\""),
                "{nom} WWW-Authenticate inattendu"
            );
            match &ref_body {
                None => ref_body = Some(resp.body),
                Some(b) => assert_eq!(&resp.body, b, "{nom} corps différent des autres refus 401"),
            }
            bucket.push(us);
        }
    }

    let stats = |v: &mut Vec<u128>| -> (u128, u128, f64, u128, f64) {
        v.sort_unstable();
        let min = v[0];
        let p50 = percentile_us(v, 50, 100);
        let p95 = percentile_us(v, 95, 100);
        let mean = v.iter().map(|&x| x as f64).sum::<f64>() / (v.len() as f64);
        let var = v
            .iter()
            .map(|&x| {
                let d = x as f64 - mean;
                d * d
            })
            .sum::<f64>()
            / (v.len() as f64);
        (min, p50, mean, p95, var.sqrt())
    };

    let s_unk = stats(&mut t_unknown);
    let s_inv = stats(&mut t_invalid);
    let s_rev = stats(&mut t_revoked);
    eprintln!(
        "[M5 pt4 3x1000] key_unknown: min={} p50={} mean={:.1} p95={} sd={:.1} us",
        s_unk.0, s_unk.1, s_unk.2, s_unk.3, s_unk.4
    );
    eprintln!(
        "[M5 pt4 3x1000] sig_invalid: min={} p50={} mean={:.1} p95={} sd={:.1} us",
        s_inv.0, s_inv.1, s_inv.2, s_inv.3, s_inv.4
    );
    eprintln!(
        "[M5 pt4 3x1000] key_revoked: min={} p50={} mean={:.1} p95={} sd={:.1} us",
        s_rev.0, s_rev.1, s_rev.2, s_rev.3, s_rev.4
    );

    let p50_min = s_unk.1.min(s_inv.1).min(s_rev.1).max(1);
    let p50_max = s_unk.1.max(s_inv.1).max(s_rev.1);
    assert!(
        p50_max * 100 <= p50_min * 150,
        "écart temporel médian excessif entre motifs 401 : unk={} inv={} rev={}",
        s_unk.1,
        s_inv.1,
        s_rev.1
    );
    // Preuve que chaque lot a bien exercé son motif exact (pas de clock_skew ni nonce_replay).
    let audit_txt = std::fs::read_to_string(e.tmp_dir().join("auth-audit.log")).unwrap();
    for motif in [
        "reason=key_unknown",
        "reason=signature_invalid",
        "reason=key_revoked",
    ] {
        let count = audit_txt.lines().filter(|l| l.contains(motif)).count();
        assert!(
            count >= N,
            "motif {motif} attendu >= {N} fois dans auth-audit.log, vu {count}"
        );
    }
}

// =====================================================================
// Lot M4 — TLS 1.3 réseau, pairs migrés, sondes et proxy signés
// =====================================================================

/// (Lot M4 points 1 & 2, avec feature `tls`) :
/// - Écoute hors loopback en TLS 1.3 (`rustls` + `rcgen`, `ALPN http/1.1`),
///   certificat auto-signé persisté et identique après redémarrage ;
/// - Capture TCP brute via relais : **0 octet HTTP en clair** (ni en-tête, ni
///   signature, ni corps JSON-RPC) sur le lien réseau ;
/// - HTTP en clair hors loopback refusé (0 octet HTTP servi) ;
/// - ClientHello TLS 1.2 refusé (alerte TLS `0x15`, 0 octet HTTP) ;
/// - Certificat substitué refusé par `PinnedServer` avant tout échange HTTP ;
/// - `tls_fp` (`sha256:<64 hex>`) annoncée dans `/auth/pair` et `/api/state`.
#[cfg(feature = "tls")]
#[test]
fn m4_tls13_lan_capture_zero_clair_refus_tls12_et_cert_substitue() {
    use std::io::Write as _;
    let _garde = serial_guard();
    let mut e = spawn(EngineCfg {
        ui_port: free_port(),
        ui_bind: "0.0.0.0",
        api_access: None,
    })
    .expect("spawn moteur TLS LAN");
    e.wait_ready(Duration::from_secs(30)).expect("moteur prêt");

    let st = e.get("/api/state").json();
    assert_eq!(st["settings"]["tls"]["compiled"], json!(true));
    assert_eq!(st["settings"]["tls"]["active"], json!(true));
    assert_eq!(st["settings"]["tls"]["bind_refused"], json!(false));
    let tls_fp = e.tls_fp();
    assert!(
        mcp_control::auth::net::valid_tls_fp(&tls_fp),
        "tls_fp sha256:<64 hex> attendue, reçu : {tls_fp}"
    );
    let acc = e.action(&json!({ "action": "access_list" }));
    assert_eq!(acc["tls"]["compiled"], json!(true));
    assert_eq!(acc["tls"]["active"], json!(true));
    assert_eq!(acc["tls"]["fingerprint"], json!(tls_fp));
    assert_eq!(
        acc["ui_url"],
        json!(format!("https://127.0.0.1:{}", e.port))
    );
    assert!(
        e.tmp_dir().join("tls").join("server.crt").exists(),
        "tls/server.crt doit être persisté"
    );
    assert!(
        e.tmp_dir().join("tls").join("server.key").exists(),
        "tls/server.key doit être persisté (MCC_KEYRING_SERVICE vide)"
    );

    // Point 2 : `/auth/pair` intègre `tls_fp` dans sa réponse et dans le code de comparaison.
    let (_, v_pair) = demander(&e, "mcc-m4-0000000000000001");
    assert_eq!(
        v_pair["tls_fp"].as_str(),
        Some(tls_fp.as_str()),
        "/auth/pair doit annoncer tls_fp"
    );

    let Some(lan) = e.lan_ip() else {
        eprintln!("SKIP partie LAN de m4_tls13_lan : aucune IP LAN non-loopback");
        return;
    };

    // 1) Relais TCP passif enregistrant TOUS les octets sur le câble entre le
    //    client TLS et `<lan>:<e.port>`.
    let relay_listener = std::net::TcpListener::bind((lan.as_str(), 0)).expect("bind relais TCP");
    let relay_port = relay_listener.local_addr().unwrap().port();
    let target_addr: std::net::SocketAddr = format!("{}:{}", lan, e.port).parse().unwrap();
    let cap_c2s = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let cap_s2c = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let c2s_cl = std::sync::Arc::clone(&cap_c2s);
    let s2c_cl = std::sync::Arc::clone(&cap_s2c);
    let relay_th = std::thread::spawn(move || {
        if let Ok((mut client_sock, _)) = relay_listener.accept() {
            let mut srv_sock =
                std::net::TcpStream::connect_timeout(&target_addr, Duration::from_secs(5))
                    .expect("connect relais -> moteur");
            let _ = client_sock.set_read_timeout(Some(Duration::from_secs(5)));
            let _ = srv_sock.set_read_timeout(Some(Duration::from_secs(5)));
            let mut client_r = client_sock.try_clone().unwrap();
            let mut srv_w = srv_sock.try_clone().unwrap();
            let t_up = std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = client_r.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    c2s_cl.lock().unwrap().extend_from_slice(&buf[..n]);
                    if srv_w.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            });
            let mut buf = [0u8; 4096];
            while let Ok(n) = srv_sock.read(&mut buf) {
                if n == 0 {
                    break;
                }
                s2c_cl.lock().unwrap().extend_from_slice(&buf[..n]);
                if client_sock.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
            let _ = t_up.join();
        }
    });

    // Appel signé `POST /mcp` (`tools/list`) à travers le relais TCP avec
    // vérification `PinnedServer` de `tls_fp`.
    let c_lan = client_appaire(&e, &["lecture"]);
    let authority = format!("{}:{}", lan, e.port);
    let body = corps_tools_list(77).to_string();
    let base_hdrs = [
        ("Content-Type", "application/json"),
        ("Mcp-Method", "tools/list"),
    ];
    let a_signer = RequestToSign::new("POST", "/mcp", &authority, &base_hdrs, body.as_bytes());
    let sig_hdrs = c_lan.headers(&a_signer);
    let mut raw_req = format!(
        "POST /mcp HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/json\r\nMcp-Method: tools/list\r\n"
    );
    for (k, v) in sig_hdrs.iter() {
        raw_req.push_str(&format!("{k}: {v}\r\n"));
    }
    raw_req.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    ));

    let (dec_resp, obs_fp) = mcp_control::auth::net::https_exchange_raw(
        &lan,
        relay_port,
        raw_req.as_bytes(),
        Duration::from_secs(10),
        Some(&tls_fp),
        2 * 1024 * 1024,
    )
    .expect("échange TLS 1.3 via relais");
    let _ = relay_th.join();

    assert_eq!(obs_fp, tls_fp, "empreinte observée == empreinte épinglée");
    let parsed = Resp::parse(&dec_resp);
    assert_eq!(parsed.status, 200, "réponse déchiffrée : {}", parsed.text());
    assert_eq!(nombre_outils(&parsed), 10);

    // Preuve capture : 0 en-tête et 0 corps en clair sur le câble TCP !
    let c2s = cap_c2s.lock().unwrap().clone();
    let s2c = cap_s2c.lock().unwrap().clone();
    assert!(
        c2s.len() > 200 && s2c.len() > 200,
        "octets TLS capturés : c2s={}, s2c={}",
        c2s.len(),
        s2c.len()
    );
    assert_eq!(c2s[0], 0x16, "premier octet c2s = 0x16 (TLS Handshake)");
    assert_eq!(s2c[0], 0x16, "premier octet s2c = 0x16 (TLS Handshake)");
    for interdit in [
        &b"HTTP/1.1"[..],
        &b"POST /mcp"[..],
        &b"Host:"[..],
        &b"Signature"[..],
        &b"Signature-Input"[..],
        &b"Content-Digest"[..],
        &b"jsonrpc"[..],
        &b"tools/list"[..],
        &b"list_servers"[..],
    ] {
        assert!(
            mcp_control::find_sub(&c2s, interdit).is_none(),
            "motif en clair {:?} trouvé dans c2s !",
            String::from_utf8_lossy(interdit)
        );
        assert!(
            mcp_control::find_sub(&s2c, interdit).is_none(),
            "motif en clair {:?} trouvé dans s2c !",
            String::from_utf8_lossy(interdit)
        );
    }
    eprintln!(
        "[M4 capture LAN] c2s={} octets, s2c={} octets, 0 motif HTTP/JSON-RPC/Signature en clair",
        c2s.len(),
        s2c.len()
    );

    // 2) HTTP en clair sur `<lan>:<e.port>` → refusé (0 réponse HTTP).
    {
        let mut plain = std::net::TcpStream::connect_timeout(&target_addr, Duration::from_secs(3))
            .expect("connect TCP clair");
        let _ = plain.set_read_timeout(Some(Duration::from_secs(3)));
        let _ = plain.write_all(b"GET /api/hello HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
        let mut rep = Vec::new();
        let _ = plain.read_to_end(&mut rep);
        assert!(
            !String::from_utf8_lossy(&rep).contains("HTTP/1.1"),
            "HTTP en clair hors loopback ne doit jamais recevoir de réponse HTTP : {rep:?}"
        );
    }

    // 3) ClientHello TLS 1.2 sur `<lan>:<e.port>` → refusé (alerte TLS 0x15, 0 HTTP).
    {
        let mut tcp12 = std::net::TcpStream::connect_timeout(&target_addr, Duration::from_secs(3))
            .expect("connect TCP TLS 1.2");
        let _ = tcp12.set_read_timeout(Some(Duration::from_secs(3)));
        let hello_tls12: [u8; 52] = [
            0x16, 0x03, 0x01, 0x00, 0x2f, // Record Handshake (47 octets)
            0x01, 0x00, 0x00, 0x2b, // ClientHello (43 octets)
            0x03, 0x03, // Version TLS 1.2 (0x0303)
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c,
            0x1d, 0x1e, 0x1f, 0x20, 0x00, // Session ID len = 0
            0x00, 0x02, 0xc0, 0x2f, // Cipher Suite TLS 1.2
            0x01, 0x00, // Compression null
            0x00, 0x00, // Extensions len = 0
        ];
        tcp12.write_all(&hello_tls12).unwrap();
        let mut rep12 = Vec::new();
        let _ = tcp12.read_to_end(&mut rep12);
        assert!(
            !String::from_utf8_lossy(&rep12).contains("HTTP/1.1"),
            "TLS 1.2 ne doit jamais négocier HTTP : {rep12:?}"
        );
        if !rep12.is_empty() {
            assert_eq!(rep12[0], 0x15, "alerte TLS 0x15 attendue sur TLS 1.2");
        }
    }

    // 4) Certificat substitué (mauvaise empreinte attendue par PinnedServer) → refusé.
    let faux_fp = format!("sha256:{}", "f".repeat(64));
    let err_sub = mcp_control::auth::net::https_exchange_raw(
        &lan,
        e.port,
        raw_req.as_bytes(),
        Duration::from_secs(5),
        Some(&faux_fp),
        4096,
    )
    .expect_err("un certificat différent de PinnedServer doit être refusé");
    assert!(
        err_sub.contains("PinnedServer") || err_sub.contains("refus TLS"),
        "attendu refus PinnedServer, reçu : {err_sub}"
    );

    // 5) Redémarrage : le certificat TLS persiste (même `tls_fp`).
    e.restart().expect("redémarrage moteur TLS");
    assert_eq!(
        e.tls_fp(),
        tls_fp,
        "l'empreinte du certificat TLS doit survivre au redémarrage"
    );
}

/// (Lot M4 point 1, sans feature `tls` — DA5 = a) :
/// Sans la feature `tls`, un moteur configuré avec `ui_bind: "0.0.0.0"` refuse
/// d'écouter hors loopback, reste en `127.0.0.1`, et publie un message clair
/// dans le journal (`mcc.log`) et dans `/api/state` (`settings.tls`).
#[cfg(not(feature = "tls"))]
#[test]
fn m4_sans_feature_tls_refuse_ecoute_hors_loopback_et_reste_en_local() {
    let _garde = serial_guard();
    let mut e = spawn(EngineCfg {
        ui_port: free_port(),
        ui_bind: "0.0.0.0",
        api_access: None,
    })
    .expect("spawn moteur sans feature tls");
    e.wait_ready(Duration::from_secs(30)).expect("moteur prêt");

    let st = e.get("/api/state").json();
    assert_eq!(st["settings"]["tls"]["compiled"], json!(false));
    assert_eq!(st["settings"]["tls"]["active"], json!(false));
    assert_eq!(st["settings"]["tls"]["bind_refused"], json!(true));
    let notice = st["settings"]["tls"]["notice"].as_str().unwrap_or("");
    assert!(
        notice.contains("feature") && notice.contains("127.0.0.1"),
        "notice TLS claire attendue dans /api/state, reçu : {notice}"
    );
    let acc = e.action(&json!({ "action": "access_list" }));
    assert_eq!(acc["tls"]["compiled"], json!(false));
    assert_eq!(acc["tls"]["active"], json!(false));
    assert_eq!(acc["tls"]["bind_refused"], json!(true));
    assert_eq!(acc["ui_url"], json!(format!("http://127.0.0.1:{}", e.port)));

    let journal = std::fs::read_to_string(e.tmp_dir().join("mcc.log")).unwrap_or_default();
    assert!(
        journal.contains("[tls]")
            && journal.contains("hors loopback refusée")
            && journal.contains("127.0.0.1"),
        "message clair attendu dans mcc.log : {journal}"
    );

    if let Some(host_lan) = e.host_lan_ip() {
        let addr: std::net::SocketAddr = format!("{}:{}", host_lan, e.port).parse().unwrap();
        assert!(
            std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(800)).is_err(),
            "le moteur sans feature `tls` ne doit PAS écouter sur l'IP LAN {addr}"
        );
    }
}

/// (Lot M4 point 3) Pairs migrés : un pair MCP Control appairé avec le droit
/// `lecture` interroge `POST /mcp` (`tools/call` `list_servers`) signé par sa
/// clé Ed25519 (et vérifiant `PinnedServer` en TLS 1.3 hors loopback), sans
/// aucun secret partagé `api_access` ni `X-MCC-Proof`.
#[test]
fn m4_pairs_migres_identite_signee_lecture_sans_x_mcc_proof() {
    let _garde = serial_guard();
    let mut e_b = spawn(EngineCfg {
        ui_port: free_port(),
        ui_bind: "127.0.0.1",
        api_access: None,
    })
    .expect("spawn pair B");
    e_b.wait_ready(Duration::from_secs(30))
        .expect("pair B prêt");

    // Ajoute un serveur sur B pour qu'il soit visible dans la découverte de pairs.
    let add_b = e_b.action(&json!({
        "action": "add",
        "transport": "http",
        "port": 31_991,
        "path": "/mcp",
        "label": "Serveur-Du-Pair-B"
    }));
    assert_eq!(add_b["ok"], json!(true), "{add_b}");

    let mut e_a = spawn(EngineCfg {
        ui_port: free_port(),
        ui_bind: "127.0.0.1",
        api_access: None,
    })
    .expect("spawn moteur A");
    e_a.wait_ready(Duration::from_secs(30))
        .expect("moteur A prêt");

    // Appairage de A auprès de B comme client `lecture` (stocké dans `A/auth/clients/`).
    let cle_pair = SigningKey::generate();
    let keyid_pair = "mcc-peer-0000000000000001";
    let pub_hex = hex_bytes(&cle_pair.verifying_key().to_bytes());
    let r_pair = e_b.post_json(
        "/auth/pair",
        &json!({
            "keyid": keyid_pair,
            "label": "pair-moteur-A",
            "public_key": pub_hex,
            "created": now(),
        }),
    );
    assert_eq!(r_pair.status, 200, "{}", r_pair.text());
    let pair_id = r_pair.json()["pair_id"].as_str().unwrap().to_string();
    let server_pub_b = r_pair.json()["server_pub"].as_str().unwrap().to_string();
    let tls_fp_b = r_pair.json()["tls_fp"].as_str().unwrap_or("").to_string();
    let app = e_b.action(&json!({
        "action": "pair_approve",
        "pair_id": pair_id,
        "rights": ["lecture"],
    }));
    assert_eq!(app["ok"], json!(true), "{app}");

    let clients_dir_a = e_a.tmp_dir().join("auth").join("clients");
    std::fs::create_dir_all(&clients_dir_a).unwrap();
    mcp_control::auth::secret::write_secret_file(
        &clients_dir_a.join(format!("{keyid_pair}.key")),
        &cle_pair.export(),
    )
    .unwrap();
    let pin_doc = json!({
        "keyid": keyid_pair,
        "label": "pair-moteur-A",
        "url": format!("http://127.0.0.1:{}", e_b.port),
        "host": "127.0.0.1",
        "port": e_b.port,
        "pair_id": pair_id,
        "server_pub": server_pub_b,
        "tls_fp": tls_fp_b,
        "rights": ["lecture"],
        "store": "file",
        "paired_at": mcp_control::types::iso_utc(now()),
    });
    mcp_control::auth::secret::write_secret_file(
        &clients_dir_a.join(format!("{keyid_pair}.pin.json")),
        &serde_json::to_string_pretty(&pin_doc).unwrap(),
    )
    .unwrap();

    // Active la découverte réseau sur A visant le port de B (sans aucun `api_access`).
    let set_a = e_a.action(&json!({
        "action": "set_settings",
        "network_enabled": true,
        "network_ports": [e_b.port],
    }));
    assert_eq!(set_a["ok"], json!(true), "{set_a}");

    let disc = e_a.action(&json!({ "action": "discover_peers" }));
    assert_eq!(disc["ok"], json!(true), "{disc}");

    let st_a = e_a.get("/api/state").json();
    let peers = st_a["peers"].as_array().expect("peers array");
    let found_b = peers
        .iter()
        .find(|p| p["ui_port"] == json!(e_b.port))
        .expect("le pair B doit être découvert par identité signée");
    assert_eq!(found_b["ip"], json!("127.0.0.1"));
    assert!(
        found_b["stats"]["total"].as_u64().unwrap_or(0) >= 1,
        "le catalogue de B doit être remonté : {found_b}"
    );

    // Preuve côté B : le journal d'audit a tracé l'appel signé `list_servers` par `keyid_pair`.
    let audit_b = std::fs::read_to_string(e_b.tmp_dir().join("auth-audit.log")).unwrap_or_default();
    assert!(
        audit_b.contains(&format!("keyid={keyid_pair}"))
            && audit_b.contains("tool=list_servers")
            && audit_b.contains("verdict=allowed"),
        "l'appel de pair signé doit figurer dans auth-audit.log de B : {audit_b}"
    );
}

/// (Lot M4 points 4 & 5) Serveur LiteMCP verrouillé :
/// 1. Avant appairage : la sonde reçoit `401` + `WWW-Authenticate` de `litemcp`
///    → le serveur est vu « joignable mais verrouillé » (`locked: true`,
///    `state: "PORT-ONLY"`, `framework: "litemcp"`, `tools_count: 0`), et
///    `call_server_tool` refuse avec un motif explicite ;
/// 2. Action locale `server_pair` (droit `admin`, refusée hors session locale) :
///    génère et stocke la clé cliente Ed25519 + épinglage `<keyid>.pin.json` ;
/// 3. Après inscription de la clé publique sur le serveur LiteMCP : la sonde
///    signe ses requêtes (`initialize` + `tools/list`), le serveur passe `UP`
///    avec son nombre exact d'outils (2) et `locked: true`, et le proxy
///    `call_server_tool` signe l'appel d'outil de bout en bout.
#[test]
fn m4_serveur_litemcp_verrouille_sonde_server_pair_et_proxy_call_server_tool() {
    use litemcp::auth::sign::{ClientKey, KeyStore, NonceCache, SignatureVerifier};
    use litemcp::auth::VerifiedRequest;
    use litemcp::server::{McpHeaders, ProtocolProfile};
    use litemcp::{zod, Gateway as LiteGateway, ServerBuilder};
    use std::io::Write as _;

    struct DynamicKeyStore {
        keys: std::sync::Mutex<Vec<(String, [u8; 32])>>,
    }
    impl KeyStore for DynamicKeyStore {
        fn lookup(&self, keyid: &str) -> Option<ClientKey> {
            let g = self.keys.lock().unwrap();
            g.iter().find(|(k, _)| k == keyid).map(|(k, pub_bytes)| {
                ClientKey::new(k.clone(), *pub_bytes)
                    .with_label("mcp-control")
                    .with_rights(["lecture", "pilotage", "execution"])
            })
        }
    }

    let _garde = serial_guard();
    let store = std::sync::Arc::new(DynamicKeyStore {
        keys: std::sync::Mutex::new(Vec::new()),
    });
    let verifier = std::sync::Arc::new(SignatureVerifier::new(
        std::sync::Arc::clone(&store) as std::sync::Arc<dyn KeyStore>,
        NonceCache::default(),
    ));
    let core = LiteGateway::new(
        ServerBuilder::new("litemcp-locked-demo", "1.0.0")
            .protocol_profile(ProtocolProfile::Legacy)
            .tool_typed(
                "echo_locked",
                "Renvoie le message reçu (serveur LiteMCP verrouillé).",
                zod::object().field("msg", zod::string()).strict(),
                |args| {
                    let m = args["msg"].as_str().unwrap_or("");
                    Ok(json!({ "echoed": m }))
                },
            )
            .require_right("echo_locked", "execution")
            .tool_typed(
                "ping_locked",
                "Sonde de test signée.",
                zod::object().strict(),
                |_args| Ok(json!({ "pong": true })),
            )
            .require_right("ping_locked", "lecture")
            .verifier(verifier),
    )
    .snapshot();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind serveur LiteMCP");
    let srv_port = listener.local_addr().unwrap().port();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_cl = std::sync::Arc::clone(&stop);
    let core_cl = std::sync::Arc::clone(&core);

    let srv_th = std::thread::spawn(move || {
        while !stop_cl.load(std::sync::atomic::Ordering::Relaxed) {
            let Ok((mut s, _)) = listener.accept() else {
                break;
            };
            if stop_cl.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            let _ = s.set_read_timeout(Some(Duration::from_millis(400)));
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            let mut sep_pos = None;
            loop {
                if let Some(p) = mcp_control::find_sub(&buf, b"\r\n\r\n") {
                    sep_pos = Some(p);
                    let head = String::from_utf8_lossy(&buf[..p]).to_ascii_lowercase();
                    let cl = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if buf.len() >= p + 4 + cl {
                        break;
                    }
                }
                match s.read(&mut tmp) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                }
            }
            let Some(p) = sep_pos else { continue };
            let head_str = String::from_utf8_lossy(&buf[..p]).to_string();
            let body = &buf[p + 4..];
            let mut lines = head_str.split("\r\n");
            let req_line = lines.next().unwrap_or("");
            let mut parts = req_line.split_whitespace();
            let method = parts.next().unwrap_or("");
            let path = parts.next().unwrap_or("/");
            if method == "GET" && path == "/health" {
                let _ = s.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                );
                continue;
            }
            let mut hdrs: Vec<(String, String)> = Vec::new();
            let mut authority = String::new();
            let mut mcp_method = None;
            let mut mcp_name = None;
            for l in lines {
                if let Some((k, v)) = l.split_once(':') {
                    let kl = k.trim().to_ascii_lowercase();
                    let vt = v.trim().to_string();
                    if kl == "host" {
                        authority = vt.clone();
                    } else if kl == "mcp-method" {
                        mcp_method = Some(vt.clone());
                    } else if kl == "mcp-name" {
                        mcp_name = Some(vt.clone());
                    }
                    hdrs.push((kl, vt));
                }
            }
            if mcp_method.is_none() {
                let _ = s.write_all(
                    b"HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: 27\r\nConnection: close\r\n\r\n{\"error\":\"missing headers\"}",
                );
                continue;
            }
            let _ = McpHeaders {
                protocol_version: None,
                method: mcp_method,
                name: mcp_name,
                params: Vec::new(),
            };
            let refs: Vec<(&str, &str)> =
                hdrs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            let vreq = VerifiedRequest::new(method, path, &authority, &refs, body);
            let vresp = core_cl.dispatch_verified(&vreq);
            let status_txt = match vresp.status {
                200 => "200 OK",
                202 => "202 Accepted",
                401 => "401 Unauthorized",
                403 => "403 Forbidden",
                _ => "400 Bad Request",
            };
            let mut http_resp = format!(
                "HTTP/1.1 {status_txt}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
                vresp.body.len()
            );
            for (hk, hv) in &vresp.headers {
                http_resp.push_str(&format!("{hk}: {hv}\r\n"));
            }
            http_resp.push_str("Connection: close\r\n\r\n");
            let _ = s.write_all(http_resp.as_bytes());
            let _ = s.write_all(&vresp.body);
        }
    });

    let e = engine_ready();
    let add = e.action(&json!({
        "action": "add",
        "transport": "http",
        "port": srv_port,
        "path": "/mcp",
        "label": "LiteMCP Verrouillé"
    }));
    assert_eq!(add["ok"], json!(true), "{add}");
    let srv_id = add["id"].as_str().unwrap().to_string();

    // 1) Avant appairage : le serveur répond 401 + WWW-Authenticate LiteMCP →
    //    vu « joignable mais verrouillé » (`locked: true`, `framework: "litemcp"`,
    //    `state: "PORT-ONLY"`, `tools_count: 0`).
    let _ = e.action(&json!({ "action": "refresh", "id": srv_id }));
    let st1 = e.get("/api/state").json();
    let s1 = st1["servers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == json!(srv_id))
        .expect("serveur présent");
    assert_eq!(s1["locked"], json!(true), "locked:true attendu : {s1}");
    assert_eq!(s1["state"], json!("PORT-ONLY"), "PORT-ONLY attendu : {s1}");
    assert_eq!(
        s1["framework"],
        json!("litemcp"),
        "framework litemcp attendu : {s1}"
    );
    assert_eq!(s1["tools_count"], json!(0));
    assert!(
        s1["last_error"]
            .as_str()
            .unwrap_or("")
            .contains("verrouillé (401 Signature)"),
        "motif verrouillé attendu : {s1}"
    );

    // `call_server_tool` avant appairage → motif clair indiquant qu'il est verrouillé.
    let call_avant = e.post_json(
        "/mcp",
        &corps_tool(
            "call_server_tool",
            json!({
                "server": srv_id,
                "tool": "echo_locked",
                "arguments": { "msg": "test" }
            }),
            50,
        ),
    );
    assert_eq!(call_avant.status, 200);
    assert!(
        call_avant.text().contains("verrouillé") && call_avant.text().contains("server_pair"),
        "motif explicite attendu sur call_server_tool non appairé : {}",
        call_avant.text()
    );

    // 2) `server_pair` sans session locale → refusé en 401 (action `admin` locale).
    let sans_sess = e.send(
        "127.0.0.1",
        e.port,
        "POST",
        "/api/action",
        &[("Content-Type", "application/json"), ("Cookie", "-")],
        Some(
            json!({ "action": "server_pair", "id": srv_id })
                .to_string()
                .as_bytes(),
        ),
        SignMode::None,
    );
    assert_eq!(
        sans_sess.status,
        401,
        "server_pair sans session : {}",
        sans_sess.text()
    );

    // Appel local légitime de `server_pair`.
    let paired = e.action(&json!({
        "action": "server_pair",
        "id": srv_id,
        "label": "mcc-console-m4"
    }));
    assert_eq!(paired["ok"], json!(true), "server_pair : {paired}");
    let keyid = paired["keyid"].as_str().unwrap().to_string();
    let pub_hex = paired["public_key"].as_str().unwrap().to_string();
    let pub_bytes = mcp_control::auth::registry::decode_public_key(&pub_hex)
        .expect("clé publique Ed25519 valide");
    store.keys.lock().unwrap().push((keyid.clone(), pub_bytes));

    // 3) Après appairage : `refresh` signe `initialize` + `tools/list` → le
    //    serveur passe `UP` avec ses 2 outils et conserve `locked: true` (badge cadenas).
    let _ = e.action(&json!({ "action": "refresh", "id": srv_id }));
    let st2 = e.get("/api/state").json();
    let s2 = st2["servers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == json!(srv_id))
        .expect("serveur présent après server_pair");
    assert_eq!(s2["state"], json!("UP"), "serveur doit passer UP : {s2}");
    assert_eq!(s2["locked"], json!(true), "badge cadenas conservé : {s2}");
    assert_eq!(s2["framework"], json!("litemcp"));
    assert_eq!(s2["tools_count"], json!(2), "2 outils attendus : {s2}");

    // 4) Proxy `call_server_tool` signé de bout en bout vers `echo_locked`.
    let call_apres = e.post_json(
        "/mcp",
        &corps_tool(
            "call_server_tool",
            json!({
                "server": srv_id,
                "tool": "echo_locked",
                "arguments": { "msg": "bonjour-litemcp-signe" }
            }),
            51,
        ),
    );
    assert_eq!(call_apres.status, 200, "{}", call_apres.text());
    assert!(
        call_apres.text().contains("bonjour-litemcp-signe"),
        "réponse de l'outil signé attendue : {}",
        call_apres.text()
    );
    let journal = std::fs::read_to_string(e.tmp_dir().join("mcc.log")).unwrap_or_default();
    assert!(
        journal.contains("call_server_tool") && journal.contains("(client "),
        "le proxy doit journaliser le client appelant (C1) : {journal}"
    );

    drop(e);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = std::net::TcpStream::connect(("127.0.0.1", srv_port));
    let _ = srv_th.join();
}

// =====================================================================
// Lot UI-api (Stage A) — surface API UI/Auth/Sécurité sans fuite
// =====================================================================

#[test]
fn ui_api_stage_a_contrats_securite_et_surface_complete() {
    let _garde = serial_guard();
    let _secs = GardeConfirmSecs::defaut();
    let mut e = engine_ready();
    e.set_auto_approve(false);
    poller_endormi(&e);

    // 1) Réglages discovery_ports_*, annonces LiteMCP non suivies, webhook & env secrets
    let rep = e.action(&json!({
        "action": "set_settings",
        "discovery_ports_only": [8787, 3000],
        "discovery_ports_excluded": [3131],
    }));
    assert_eq!(rep["ok"], json!(true), "{rep}");

    let rep_wh = e.action(&json!({
        "action": "set_notify",
        "webhook_url": "http://192.0.2.99/hook",
        "headers": "Authorization: Bearer e2e-webhook-secret-999\nX-Hook-Token: e2e-hook-888",
    }));
    assert_eq!(rep_wh["ok"], json!(true), "{rep_wh}");

    let rep_srv = e.action(&json!({
        "action": "add",
        "transport": "stdio",
        "label": "srv-e2e-sec",
        "program": "node",
        "args": ["nope.js"],
        "env": { "API_SECRET": "e2e-env-secret-777" },
    }));
    assert_eq!(rep_srv["ok"], json!(true), "{rep_srv}");

    let reg_dir = e.tmp_dir().join("litemcp-registry");
    std::fs::write(
        reg_dir.join("libre.json"),
        r#"{"name":"e2e-libre","port":28787,"version":"0.1.0"}"#,
    )
    .unwrap();

    let st0_resp = e.get("/api/state");
    assert_eq!(st0_resp.status, 200);
    let st0 = st0_resp.json();
    assert_eq!(st0["settings"]["discovery_ports_only"], json!([8787, 3000]));
    assert_eq!(st0["settings"]["discovery_ports_excluded"], json!([3131]));
    assert_eq!(st0["announcements_count"], json!(1));
    assert_eq!(st0["sessions_active"], json!(1));
    assert!(
        !st0["servers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["label"] == "e2e-libre"),
        "l'annonce ne doit jamais être adoptée : {st0}"
    );
    let rev0 = st0["access"]["revision"].as_u64().expect("access.revision");
    assert_eq!(
        e.get("/api/state").json()["access"]["revision"].as_u64(),
        Some(rev0),
        "revision stable au repos"
    );

    // 2) Sessions : 1 -> 2 via canal fichier -> 1 après close-others
    let (c2, s2) = e.open_extra_session().expect("seconde session");
    let st_s2 = e.get("/api/state").json();
    assert_eq!(st_s2["sessions_active"], json!(2));
    let acc_s2 = e.action(&json!({ "action": "access_list" }));
    assert_eq!(acc_s2["sessions"], json!({ "active": 2, "max": 8 }));

    let close = e.post_json("/api/session/close-others", &json!({}));
    assert_eq!(close.status, 200, "{}", close.text());
    assert_eq!(e.get("/api/state").json()["sessions_active"], json!(1));

    // 3) access_list : server_identity, sessions, tls, ui_url, clients[].stale_180d
    let acc = e.action(&json!({ "action": "access_list" }));
    assert_eq!(acc["ok"], json!(true), "{acc}");
    assert_eq!(acc["server_identity"]["keyid"], json!("mcc-server"));
    assert!(acc["server_identity"]["fingerprint"]
        .as_str()
        .unwrap_or("")
        .starts_with("ed25519:"));
    assert!(acc["server_identity"]["fingerprint_court"]
        .as_str()
        .unwrap_or("")
        .starts_with("ed25519:"));
    assert_eq!(acc["server_identity"]["storage"], json!("file_fallback"));
    assert_eq!(acc["sessions"], json!({ "active": 1, "max": 8 }));
    assert_eq!(acc["tls"]["compiled"], json!(cfg!(feature = "tls")));
    assert_eq!(acc["tls"]["active"], json!(false));
    assert_eq!(acc["tls"]["bind_refused"], json!(false));
    assert_eq!(acc["ui_url"], json!(format!("http://127.0.0.1:{}", e.port)));
    let clients_arr = acc["clients"].as_array().unwrap();
    assert!(!clients_arr.is_empty());
    for c in clients_arr {
        assert_eq!(c["stale_180d"], json!(false));
    }

    // 4) access.revision bouge sur chacun des 5 événements Accès
    let (srv_id, _marqueur) = ajoute_marqueur(&e, "uiapi-rev");
    let client_exec = client_appaire(&e, &["execution"]);
    let rev_client = e.get("/api/state").json()["access"]["revision"]
        .as_u64()
        .unwrap();
    assert_ne!(rev_client, rev0, "nouveau client → revision change");

    // 4a. Confirmation en attente
    let r_pend = post_mcp_signe(
        &e,
        &client_exec,
        "127.0.0.1",
        &corps_tool("start_server", json!({ "id": srv_id }), 1),
        None,
        None,
    );
    assert!(demande_confirmation(&r_pend));
    let rev_pend = e.get("/api/state").json()["access"]["revision"]
        .as_u64()
        .unwrap();
    assert_ne!(rev_pend, rev_client, "demande en attente → revision change");

    // 4b. Permis actif + révocation de permis
    let req_id = demande_en_attente(&e, "start_server")["id"]
        .as_str()
        .unwrap()
        .to_string();
    let app = e.action(&json!({
        "action": "confirm_approve",
        "request_id": req_id,
        "mode": "window",
    }));
    assert_eq!(app["ok"], json!(true));
    let rev_grant = e.get("/api/state").json()["access"]["revision"]
        .as_u64()
        .unwrap();
    assert_ne!(rev_grant, rev_pend, "permis actif → revision change");

    let revk = e.action(&json!({ "action": "grant_revoke", "request_id": req_id }));
    assert_eq!(revk["ok"], json!(true));
    let rev_ungrant = e.get("/api/state").json()["access"]["revision"]
        .as_u64()
        .unwrap();
    assert_ne!(rev_ungrant, rev_grant, "permis révoqué → revision change");

    // 4c. Décision récente (droit manquant)
    let client_lec = client_appaire(&e, &["lecture"]);
    let r_deny = post_mcp_signe(
        &e,
        &client_lec,
        "127.0.0.1",
        &corps_tool("refresh", json!({}), 2),
        None,
        None,
    );
    assert!(est_refus_droit(&r_deny));
    let rev_dec = e.get("/api/state").json()["access"]["revision"]
        .as_u64()
        .unwrap();
    assert_ne!(rev_dec, rev_ungrant, "décision récente → revision change");

    // 4d. Demande d'appairage
    let (ps, pv) = demander(&e, "mcc-uiapi-0000000000000009");
    assert_eq!(ps, 200, "{pv}");
    let rev_pair = e.get("/api/state").json()["access"]["revision"]
        .as_u64()
        .unwrap();
    assert_ne!(rev_pair, rev_dec, "demande d'appairage → revision change");

    // 5) Sécurité négative : zéro secret dans state_json et access_list
    let st_raw = e.get("/api/state").text();
    let acc_raw = e.action(&json!({ "action": "access_list" })).to_string();
    let server_key =
        std::fs::read_to_string(e.tmp_dir().join("auth").join("server.key")).unwrap_or_default();
    let client_key =
        std::fs::read_to_string(e.tmp_dir().join("e2e-client.key")).unwrap_or_default();
    let sess_cookie = e.session_cookie().unwrap_or("");
    let sess_secret = e.session_secret().unwrap_or("");
    for (nom, txt) in [("state_json", &st_raw), ("access_list", &acc_raw)] {
        for secret in [
            "PRIVATE KEY",
            "e2e-webhook-secret-999",
            "e2e-hook-888",
            "e2e-env-secret-777",
            "mcc_session",
            "X-MCC-Session",
            "x-mcc-session",
            sess_cookie,
            sess_secret,
            c2.as_str(),
            s2.as_str(),
        ] {
            if !secret.is_empty() {
                assert!(
                    !txt.contains(secret),
                    "{nom} ne doit jamais exposer « {secret} »"
                );
            }
        }
        for line in server_key
            .lines()
            .chain(client_key.lines())
            .filter(|l| l.len() > 24 && !l.starts_with("---"))
        {
            assert!(
                !txt.contains(line),
                "{nom} ne doit jamais exposer une ligne de clé privée"
            );
        }
    }

    // 6) Actions réservées inatteignables depuis /mcp (ni direct ni par injection),
    //    même pour un client doté de tous les droits MCP (lecture/pilotage/execution).
    let client_all = client_appaire(&e, &["lecture", "pilotage", "execution"]);
    let tl = post_mcp_signe(
        &e,
        &client_all,
        "127.0.0.1",
        &corps_tools_list(10),
        None,
        None,
    );
    let tl_txt = tl.text();
    for interdite in [
        "access_list",
        "server_pair",
        "client_rotate",
        "litemcp_announcements",
        "stop_server_force",
    ] {
        assert!(!tl_txt.contains(interdite), "{interdite} listée sur /mcp");
        let direct = post_mcp_signe(
            &e,
            &client_all,
            "127.0.0.1",
            &corps_tool(interdite, json!({}), 20),
            None,
            None,
        );
        assert_eq!(
            direct.json()["error"]["code"],
            json!(-32602),
            "appel direct /mcp {interdite} : {}",
            direct.text()
        );
        let inj = post_mcp_signe(
            &e,
            &client_all,
            "127.0.0.1",
            &corps_tool(
                "scan_ports",
                json!({ "from": 1, "to": 2, "action": interdite }),
                21,
            ),
            None,
            None,
        );
        assert_eq!(
            inj.json()["error"]["code"],
            json!(-32602),
            "injection /mcp {interdite} : {}",
            inj.text()
        );
    }

    e.action(&json!({ "action": "kill", "id": srv_id }));
}

/// Construit une requête HTTP/1.1 brute `POST /mcp` signée pour `authority`
/// (`host:port`) avec un `(created, nonce)` donné — réutilisable octet pour
/// octet pour tester les rejeux inter-ports (R-1) et post-redémarrage (R-2).
fn raw_mcp_signed(
    signer: &Signer,
    authority: &str,
    corps: &Value,
    created: u64,
    nonce: [u8; 16],
) -> Vec<u8> {
    let body = corps.to_string();
    let methode = corps["method"].as_str().unwrap_or("");
    let base: Vec<(String, String)> = vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("Mcp-Method".to_string(), methode.to_string()),
    ];
    let refs0: Vec<(&str, &str)> = base.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
    let a_signer = RequestToSign::new("POST", "/mcp", authority, &refs0, body.as_bytes());
    let signes = signer.headers_at(&a_signer, created, &nonce);
    let mut raw = format!(
        "POST /mcp HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/json\r\nMcp-Method: {methode}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (n, v) in signes.iter() {
        raw.push_str(&format!("{n}: {v}\r\n"));
    }
    raw.push_str("\r\n");
    raw.push_str(&body);
    raw.into_bytes()
}

/// R-1 (audit R) : une requête `/mcp` signée pour le port A (`Host: 127.0.0.1:A`)
/// doit être refusée (`403`) si rejouée sur une 2e instance écoutant sur le port B,
/// même quand le même client Ed25519 est enregistré sur les deux instances.
#[test]
fn r1_rejeu_inter_ports_inter_instances_refuse() {
    let _garde = serial_guard();
    let e_a = engine();
    let e_b = engine();
    assert_ne!(e_a.port, e_b.port, "deux ports distincts requis");

    let cle = SigningKey::generate();
    let keyid = nouveau_keyid();
    let pub_hex = hex_bytes(&cle.verifying_key().to_bytes());
    let demande = json!({
        "keyid": keyid,
        "label": "client-r1",
        "public_key": pub_hex,
        "created": now(),
    });
    for inst in [&e_a, &e_b] {
        let r = inst.post_json("/auth/pair", &demande);
        assert_eq!(r.status, 200, "appairage refusé : {}", r.text());
        let pair_id = r.json()["pair_id"].as_str().expect("pair_id").to_string();
        let ok = inst.action(&json!({
            "action": "pair_approve",
            "pair_id": pair_id,
            "rights": ["lecture"],
        }));
        assert_eq!(ok["ok"], json!(true), "approbation refusée : {ok}");
    }
    let signer = Signer::new(cle).with_keyid(keyid);
    let raw = raw_mcp_signed(
        &signer,
        &format!("127.0.0.1:{}", e_a.port),
        &corps_tools_list(1),
        now(),
        nonce_neuf(),
    );

    let rep_a = e_a.raw(&raw, 0);
    assert_eq!(
        rep_a.status,
        200,
        "instance A (port cible) → 200 : {}",
        rep_a.text()
    );

    let rep_b = e_b.raw(&raw, 0);
    assert!(
        rep_b.status == 403 || rep_b.status == 401,
        "rejeu inter-ports sur instance B doit être refusé (403/401), vu {} : {}",
        rep_b.status,
        rep_b.text()
    );
}

/// R-2 (audit R) : une requête `/mcp` signée avant un redémarrage du moteur
/// (donc `created < boot_ts`) doit être refusée en `401` après redémarrage,
/// même dans la fenêtre ±60 s.
#[test]
fn r2_rejeu_nonce_apres_redemarrage_refuse_401() {
    let _garde = serial_guard();
    let mut e = engine();
    let signer = client_appaire(&e, &["lecture"]);
    let ts = now();
    let raw = raw_mcp_signed(
        &signer,
        &format!("127.0.0.1:{}", e.port),
        &corps_tools_list(1),
        ts,
        nonce_neuf(),
    );

    let avant = e.raw(&raw, 0);
    assert_eq!(
        avant.status,
        200,
        "avant redémarrage → 200 : {}",
        avant.text()
    );

    // Garantir boot_ts > ts (horloge en secondes).
    std::thread::sleep(Duration::from_millis(1200));
    e.restart().expect("redémarrage sur la même config");

    let rejeu = e.raw(&raw, 0);
    assert_eq!(
        rejeu.status,
        401,
        "rejeu après redémarrage doit rendre 401, vu {} : {}",
        rejeu.status,
        rejeu.text()
    );
    assert_eq!(rejeu.json()["error"]["message"], json!("unauthorized"));

    // Contrôle positif : une requête neuve signée après redémarrage passe toujours.
    let frais = post_mcp_signe(&e, &signer, "127.0.0.1", &corps_tools_list(2), None, None);
    assert_eq!(
        frais.status,
        200,
        "requête fraîche post-boot → 200 : {}",
        frais.text()
    );
}

/// R-5 (audit R) : `mcp-control auth list` et `mcp-control auth revoke <keyid>`
/// obtiennent une session locale S1 via le canal fichier et réussissent en CLI
/// (sans jamais afficher de secret ni de cookie), et la révocation prend effet
/// dès la requête signée suivante (`401`).
#[test]
fn r5_cli_auth_list_et_auth_revoke_via_session_locale() {
    let _garde = serial_guard();
    let e = engine();

    let cle = SigningKey::generate();
    let keyid = nouveau_keyid();
    let pub_hex = hex_bytes(&cle.verifying_key().to_bytes());
    let r = e.post_json(
        "/auth/pair",
        &json!({
            "keyid": keyid,
            "label": "client-cli-r5",
            "public_key": pub_hex,
            "created": now(),
        }),
    );
    assert_eq!(r.status, 200, "appairage refusé : {}", r.text());
    let pair_id = r.json()["pair_id"].as_str().expect("pair_id").to_string();
    let ok = e.action(&json!({
        "action": "pair_approve",
        "pair_id": pair_id,
        "rights": ["lecture"],
    }));
    assert_eq!(ok["ok"], json!(true), "approbation refusée : {ok}");

    let signer = Signer::new(cle).with_keyid(keyid.clone());
    let avant = post_mcp_signe(&e, &signer, "127.0.0.1", &corps_tools_list(1), None, None);
    assert_eq!(
        avant.status,
        200,
        "avant révocation → 200 : {}",
        avant.text()
    );

    let cfg_path = e.tmp_dir().join("config.json");

    // 1. `mcp-control auth list` → exit 0, affiche le client, aucune fuite de session
    let out_list = Command::new(env!("CARGO_BIN_EXE_mcp-control"))
        .args(["auth", "list"])
        .env("MCC_CONFIG", &cfg_path)
        .env("MCC_KEYRING_SERVICE", "")
        .output()
        .expect("exécution auth list");
    let list_stdout = String::from_utf8_lossy(&out_list.stdout);
    let list_stderr = String::from_utf8_lossy(&out_list.stderr);
    assert!(
        out_list.status.success(),
        "auth list doit réussir (exit 0), stderr={list_stderr}, stdout={list_stdout}"
    );
    assert!(
        list_stdout.contains(&keyid),
        "auth list doit afficher {keyid} : {list_stdout}"
    );
    for interdit in ["mcc_session", "X-MCC-Session", "PRIVATE KEY"] {
        assert!(
            !list_stdout.contains(interdit),
            "fuite {interdit} sur stdout"
        );
        assert!(
            !list_stderr.contains(interdit),
            "fuite {interdit} sur stderr"
        );
    }

    // 2. `mcp-control auth revoke <keyid>` → exit 0, aucune fuite de session
    let out_rev = Command::new(env!("CARGO_BIN_EXE_mcp-control"))
        .args(["auth", "revoke", &keyid])
        .env("MCC_CONFIG", &cfg_path)
        .env("MCC_KEYRING_SERVICE", "")
        .output()
        .expect("exécution auth revoke");
    let rev_stdout = String::from_utf8_lossy(&out_rev.stdout);
    let rev_stderr = String::from_utf8_lossy(&out_rev.stderr);
    assert!(
        out_rev.status.success(),
        "auth revoke doit réussir (exit 0), stderr={rev_stderr}, stdout={rev_stdout}"
    );
    assert!(
        rev_stdout.contains(&keyid),
        "auth revoke doit confirmer {keyid} : {rev_stdout}"
    );
    for interdit in ["mcc_session", "X-MCC-Session", "PRIVATE KEY"] {
        assert!(
            !rev_stdout.contains(interdit),
            "fuite {interdit} sur stdout"
        );
        assert!(
            !rev_stderr.contains(interdit),
            "fuite {interdit} sur stderr"
        );
    }

    // 3. La requête signée suivante de ce client est immédiatement refusée en 401
    let apres = post_mcp_signe(&e, &signer, "127.0.0.1", &corps_tools_list(2), None, None);
    assert_eq!(
        apres.status,
        401,
        "client révoqué via CLI doit recevoir 401, vu {} : {}",
        apres.status,
        apres.text()
    );
}
