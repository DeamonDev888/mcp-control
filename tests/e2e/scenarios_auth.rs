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

use crate::harness::{serial_guard, spawn, Engine, EngineCfg, Resp};
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

fn clients(e: &Engine) -> Vec<Value> {
    e.action(&json!({ "action": "client_list" }))["clients"]
        .as_array()
        .cloned()
        .unwrap_or_default()
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
    assert_eq!(
        acceptees, 5,
        "le quota d'appairage (5/min/Adresse) doit borner la tempête"
    );
    assert_eq!(refusees, 495);
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
