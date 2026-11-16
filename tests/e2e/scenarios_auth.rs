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
    assert_eq!(access.len(), 3, "compteurs seulement : {access:?}");
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
