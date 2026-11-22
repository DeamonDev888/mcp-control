//! Tests transversaux du lot M1 : persistance du registre, permissions de
//! `config.json`, `KeyStore` et actions locales.
//!
//! Ils vivent ici (et non dans `manage/tests.rs`) parce qu'ils portent sur le
//! module `auth` tout en utilisant le vrai `save_config`/`load_config` du
//! moteur : c'est le seul moyen de prouver le round-trip ET l'absence de clé
//! privée dans le fichier.

use crate::manage::{handle_action, load_config, save_config, ACTIONS};
use crate::types::{lockv, AppState, Settings};
use std::sync::Arc;

/// Clé PUBLIQUE du premier vecteur RFC 8032 §7.1 — rien de secret.
const CLE_PUB: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

fn test_state() -> Arc<AppState> {
    Arc::new(AppState {
        servers: std::sync::Mutex::new(vec![]),
        log: std::sync::Mutex::new(std::collections::VecDeque::new()),
        settings: std::sync::Mutex::new(Settings::default()),
        children: std::sync::Mutex::new(std::collections::HashMap::new()),
        peers: std::sync::Mutex::new(std::collections::HashMap::new()),
        allowed_origins: std::sync::Mutex::new(Vec::new()),
        allowed_hosts: std::sync::Mutex::new(vec!["127.0.0.1".to_string()]),
        spawn_fails: std::sync::Mutex::new(std::collections::HashMap::new()),
        runtime: std::sync::Mutex::new(crate::runtime::RuntimeLog::new()),
        spawning: std::sync::Mutex::new(std::collections::HashSet::new()),
        update_latest: std::sync::Mutex::new(None),
        lan_ok: std::sync::Mutex::new(None),
        config_path: String::new(),
    })
}

fn tmp_dir(nom: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mcc-auth-config-{}-{}-{}",
        std::process::id(),
        crate::types::now_ts(),
        nom
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn state_avec_config(dir: &std::path::Path) -> Arc<AppState> {
    let mut state = test_state();
    {
        let mut s = lockv(&state.settings);
        s.auth.server = Some(crate::auth::ServerIdentity {
            keyid: crate::auth::SERVER_KEYID.to_string(),
            public_key: CLE_PUB.to_string(),
            created_at: "2026-11-15 12:00:00".to_string(),
        });
        s.auth
            .insert(crate::auth::ClientRecord::new(
                "mcc-a-0000000000000001".into(),
                "portable".into(),
                CLE_PUB.into(),
                vec!["lecture".into(), "pilotage".into()],
                "192.0.2.20".into(),
            ))
            .unwrap();
    }
    Arc::get_mut(&mut state).unwrap().config_path =
        dir.join("config.json").to_string_lossy().to_string();
    state
}

/// (point 2) Le registre voyage dans `config.json` — clé PUBLIQUE seulement —
/// et revient à l'identique au chargement du moteur.
#[test]
fn registre_auth_persiste_dans_config_json_sans_aucune_cle_privee() {
    let dir = tmp_dir("roundtrip");
    let state = state_avec_config(&dir);
    save_config(&state);
    let texte = std::fs::read_to_string(dir.join("config.json")).unwrap();
    assert!(texte.contains("\"auth\""), "section auth absente : {texte}");
    assert!(texte.contains(CLE_PUB), "clé publique absente : {texte}");
    assert!(
        texte.contains("mcc-a-0000000000000001"),
        "client absent : {texte}"
    );
    for interdit in ["PRIVATE KEY", "BEGIN PRIVATE", "ED25519 PRIVATE"] {
        assert!(
            !texte.contains(interdit),
            "config.json ne doit JAMAIS contenir « {interdit} » : {texte}"
        );
    }
    let (_, settings) = load_config(&dir.join("config.json").to_string_lossy());
    let server = settings
        .auth
        .server
        .as_ref()
        .expect("identité relue")
        .clone();
    assert_eq!(server.public_key, CLE_PUB);
    assert_eq!(server.keyid, crate::auth::SERVER_KEYID);
    let client = settings
        .auth
        .find("mcc-a-0000000000000001")
        .expect("client relu");
    assert_eq!(
        client.rights,
        vec!["lecture".to_string(), "pilotage".to_string()]
    );
    assert_eq!(client.paired_from, "192.0.2.20");
    assert_eq!(client.status, "active");
    let _ = std::fs::remove_dir_all(&dir);
}

/// (lot 3, audit 05 #10 — FINI au lot M1) `config.json` n'est lisible que par
/// l'utilisateur : 0600 sous Unix, ACL sans héritage sous Windows.
#[test]
fn config_json_a_permissions_reduites_apres_ecriture() {
    let dir = tmp_dir("acl");
    let state = state_avec_config(&dir);
    save_config(&state);
    let chemin = dir.join("config.json");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&chemin).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "0600 attendu, vu {mode:o}");
    }
    #[cfg(windows)]
    {
        let out = std::process::Command::new("icacls")
            .arg(&chemin)
            .output()
            .expect("icacls");
        let texte = String::from_utf8_lossy(&out.stdout).to_string();
        let user = std::env::var("USERNAME").unwrap_or_default();
        assert!(
            texte.contains(&user),
            "l'utilisateur courant doit rester autorisé : {texte}"
        );
        assert!(
            !texte.contains("(I)"),
            "l'héritage doit être coupé (audit 05 #10) : {texte}"
        );
        assert!(
            !texte.contains("BUILTIN\\Users") && !texte.contains("Everyone"),
            "aucun groupe large ne doit rester : {texte}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// (points 3/4) `KeyStore::lookup` relit le registre à chaque appel : une
/// révocation prend effet à la requête suivante, sans redémarrage.
#[test]
fn keystore_reflete_la_revocation_immediatement() {
    use ::litemcp::auth::sign::KeyStore;
    let dir = tmp_dir("revocation");
    let state = state_avec_config(&dir);
    let store = crate::auth::RegistryStore {
        state: Arc::clone(&state),
    };
    let avant = store.lookup("mcc-a-0000000000000001").expect("clé connue");
    assert_eq!(
        avant.rights,
        vec!["lecture".to_string(), "pilotage".to_string()]
    );
    assert!(
        store.lookup("mcc-a-00000000deadbeef").is_none(),
        "keyid inconnu = aucune clé"
    );
    let j = handle_action(
        &state,
        r#"{"action":"client_revoke","keyid":"mcc-a-0000000000000001"}"#,
    );
    assert_eq!(j["ok"], serde_json::json!(true), "{j}");
    let apres = store
        .lookup("mcc-a-0000000000000001")
        .expect("clé encore listée");
    assert_eq!(
        apres.status,
        ::litemcp::auth::sign::KeyStatus::Revoked,
        "la révocation doit être visible à l'appel suivant"
    );
    let texte = std::fs::read_to_string(dir.join("config.json")).unwrap();
    assert!(texte.contains("\"revoked\""), "{texte}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// (point 4) Les 7 actions d'authentification répondent sur le dispatcher LOCAL
/// et refusent proprement ce qui n'existe pas.
#[test]
fn actions_auth_locales_repondent_et_refusent_le_reste() {
    let state = test_state();
    for action in crate::auth::ADMIN_ACTIONS {
        assert!(
            ACTIONS.contains(&action) || action == "server_pair",
            "{action} doit figurer dans la surface ACTIONS (ou route locale server_pair)"
        );
    }
    let j_sp =
        crate::auth::net::handle_server_pair(&state, r#"{"action":"server_pair","id":"inconnu"}"#);
    assert_eq!(j_sp["ok"], serde_json::json!(false), "{j_sp}");
    let j = handle_action(&state, r#"{"action":"client_list"}"#);
    assert_eq!(j["ok"], serde_json::json!(true), "{j}");
    assert_eq!(j["clients"].as_array().unwrap().len(), 0);
    let j = handle_action(&state, r#"{"action":"pair_list"}"#);
    assert_eq!(j["ok"], serde_json::json!(true), "{j}");
    assert_eq!(j["pending"].as_array().unwrap().len(), 0);
    for corps in [
        r#"{"action":"pair_approve","pair_id":"00000000000000000000000000000000","rights":["lecture"]}"#,
        r#"{"action":"pair_reject","pair_id":"00000000000000000000000000000000"}"#,
        r#"{"action":"client_revoke","keyid":"mcc-a-0000000000000001"}"#,
        r#"{"action":"client_set_scopes","keyid":"mcc-a-0000000000000001","rights":["lecture"]}"#,
    ] {
        let j = handle_action(&state, corps);
        assert_eq!(j["ok"], serde_json::json!(false), "{corps} → {j}");
        assert!(
            j["error"].as_str().is_some_and(|e| !e.is_empty()),
            "un motif est attendu : {j}"
        );
    }
}
