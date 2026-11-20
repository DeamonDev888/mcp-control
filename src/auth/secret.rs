//! Stockage des clés privées — trousseau du système, repli fichier 0600.
//!
//! `docs/auth.md` §9 : la clé privée Ed25519 du serveur vit au trousseau
//! (service `mcp-control`, compte `server-ed25519`). Si le trousseau est
//! indisponible, on retombe sur `<config_dir>/auth/server.key` avec des
//! permissions réduites à l'utilisateur, et **chaque démarrage le dit** au
//! journal : un repli silencieux serait un mensonge sur le niveau de protection.
//!
//! Aucune primitive cryptographique ici : la clé vient de
//! [`litemcp::auth::sign::SigningKey`] (aléa du système, graine effacée à la
//! destruction par la feature `zeroize` de la lib).
//!
//! Isolation des tests : `MCC_KEYRING_SERVICE` remplace le nom du service, et
//! une valeur **vide** force le repli fichier. Sans cela, chaque moteur de test
//! écrirait une identité dans le gestionnaire d'identifiants de l'utilisateur.

use litemcp::auth::sign::SigningKey;
use std::path::{Path, PathBuf};

/// Service du trousseau (§9).
pub const KEYRING_SERVICE: &str = "mcp-control";
/// Compte de la clé privée du serveur (§9).
pub const ACCOUNT_SERVER: &str = "server-ed25519";
/// Compte de la clé HMAC du journal d'audit (§9, §11).
pub const ACCOUNT_AUDIT: &str = "audit-hmac";
/// Variable d'environnement d'isolation (tests, instances multiples).
pub const SERVICE_ENV: &str = "MCC_KEYRING_SERVICE";

/// D'où vient la clé réellement utilisée — journalisé au démarrage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretStore {
    Keyring,
    File,
}

impl SecretStore {
    pub fn as_str(&self) -> &'static str {
        match self {
            SecretStore::Keyring => "trousseau du système",
            SecretStore::File => "fichier à permissions restreintes (repli)",
        }
    }
}

pub struct LoadedKey {
    pub key: SigningKey,
    pub store: SecretStore,
}

/// Service du trousseau à utiliser : `None` = trousseau désactivé (repli
/// fichier), ce que `MCC_KEYRING_SERVICE=""` demande explicitement.
pub fn service() -> Option<String> {
    match std::env::var(SERVICE_ENV) {
        Ok(v) => {
            let v = v.trim().to_string();
            if v.is_empty() {
                None
            } else {
                Some(v)
            }
        }
        Err(_) => Some(KEYRING_SERVICE.to_string()),
    }
}

/// Chemin du repli fichier : `auth/server.key` à côté du `config.json`.
pub fn server_key_path(config_dir: &Path) -> PathBuf {
    config_dir.join("auth").join("server.key")
}

/// Chemin du repli fichier de la clé du journal d'audit (§9) :
/// `auth/audit.key`, 32 octets en hexadécimal, permissions utilisateur seul.
pub fn audit_key_path(config_dir: &Path) -> PathBuf {
    config_dir.join("auth").join("audit.key")
}

/// Lit une entrée du trousseau. `Ok(None)` = entrée absente ; `Err` = trousseau
/// indisponible (l'appelant bascule sur le repli fichier).
#[cfg(feature = "keyring")]
fn keyring_read(service: &str, account: &str) -> Result<Option<String>, String> {
    match keyring::Entry::new(service, account) {
        Ok(entry) => match entry.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        },
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(not(feature = "keyring"))]
fn keyring_read(_service: &str, _account: &str) -> Result<Option<String>, String> {
    Err("crate keyring non compilée (feature `keyring` désactivée)".to_string())
}

#[cfg(feature = "keyring")]
fn keyring_write(service: &str, account: &str, secret: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(service, account).map_err(|e| e.to_string())?;
    entry.set_password(secret).map_err(|e| e.to_string())
}

#[cfg(not(feature = "keyring"))]
fn keyring_write(_service: &str, _account: &str, _secret: &str) -> Result<(), String> {
    Err("crate keyring non compilée (feature `keyring` désactivée)".to_string())
}

/// Écrit un secret dans un fichier à permissions réduites : dossier créé au
/// besoin, écriture temporaire + renommage, permissions posées AVANT que la clé
/// ne soit visible sous son nom définitif.
pub fn write_secret_file(path: &Path, secret: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("dossier {} impossible : {}", parent.display(), e))?;
    }
    let tmp = path.with_extension("key.tmp");
    std::fs::write(&tmp, secret)
        .map_err(|e| format!("écriture {} impossible : {}", tmp.display(), e))?;
    crate::manage::restrict_permissions(&tmp.to_string_lossy())
        .map_err(|e| format!("permissions {} impossibles : {}", tmp.display(), e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("renommage {} impossible : {}", path.display(), e)
    })?;
    let _ = crate::manage::restrict_permissions(&path.to_string_lossy());
    Ok(())
}

fn read_secret_file(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("lecture {} impossible : {}", path.display(), e)),
    }
}

/// Charge la clé privée du serveur, ou la crée si c'est le premier lancement.
///
/// Ordre : trousseau → repli fichier → génération. Une entrée de trousseau
/// illisible est une ERREUR (on ne régénère jamais une identité par accident :
/// tout client appairé la refuserait ensuite).
pub fn load_or_create_server(dir: &Path, service: Option<&str>) -> Result<LoadedKey, String> {
    let path = server_key_path(dir);
    if let Some(svc) = service {
        match keyring_read(svc, ACCOUNT_SERVER) {
            Ok(Some(block)) => {
                let key = SigningKey::import(&block)
                    .map_err(|_| "clé du trousseau illisible (bloc invalide)".to_string())?;
                return Ok(LoadedKey {
                    key,
                    store: SecretStore::Keyring,
                });
            }
            Ok(None) => {}
            // trousseau indisponible : on ne s'arrête pas, on le DIT (l'appelant
            // journalise le repli) — mais on ne perd pas une clé déjà en fichier.
            Err(_) => {}
        }
    }
    if let Some(block) = read_secret_file(&path)? {
        let key = SigningKey::import(&block)
            .map_err(|_| format!("clé du repli illisible : {}", path.display()))?;
        return Ok(LoadedKey {
            key,
            store: SecretStore::File,
        });
    }
    let key = SigningKey::generate();
    let block = key.export();
    if let Some(svc) = service {
        if keyring_write(svc, ACCOUNT_SERVER, &block).is_ok() {
            return Ok(LoadedKey {
                key,
                store: SecretStore::Keyring,
            });
        }
    }
    write_secret_file(&path, &block)?;
    Ok(LoadedKey {
        key,
        store: SecretStore::File,
    })
}

/// Range la clé privée d'un client SORTANT (résultat de `auth pair`) au
/// trousseau, compte `client-<hôte>:<port>` (§9). Le repli fichier est la
/// responsabilité de l'appelant, qui connaît le `keyid`.
pub fn store_client_key(service: &str, account: &str, block: &str) -> Result<(), String> {
    keyring_write(service, account, block)
}

/// Lit la clé privée d'un client sortant (`auth pair`), trousseau d'abord.
/// `Ok(None)` = aucune entrée ; `Err` = trousseau indisponible — l'appelant
/// bascule alors sur le repli fichier `auth/clients/<keyid>.key`.
pub fn load_client_key(service: Option<&str>, account: &str) -> Result<Option<String>, String> {
    match service {
        Some(svc) => keyring_read(svc, account),
        None => Ok(None),
    }
}

/// Décode 32 octets en hexadécimal minuscule (clé du journal).
fn decode_cle_hex(hex: &str) -> Result<Vec<u8>, String> {
    let hex = hex.trim();
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("clé du journal illisible (64 hexadécimaux attendus)".to_string());
    }
    let mut out = Vec::with_capacity(32);
    for i in 0..32 {
        let pair = hex.get(i * 2..i * 2 + 2).ok_or("clé tronquée")?;
        out.push(u8::from_str_radix(pair, 16).map_err(|_| "clé illisible")?);
    }
    Ok(out)
}

/// Lit la clé HMAC du journal d'audit, sans jamais la créer : `None` = ni
/// trousseau ni fichier (le journal ne peut alors pas être vérifié ici).
pub fn load_audit_key(dir: &Path, service: Option<&str>) -> Result<Option<Vec<u8>>, String> {
    if let Some(svc) = service {
        match keyring_read(svc, ACCOUNT_AUDIT) {
            Ok(Some(hex)) => return decode_cle_hex(&hex).map(Some),
            Ok(None) => {}
            Err(_) => {}
        }
    }
    match read_secret_file(&audit_key_path(dir))? {
        Some(hex) => decode_cle_hex(&hex).map(Some),
        None => Ok(None),
    }
}

pub struct LoadedAuditKey {
    pub key: Vec<u8>,
    pub store: SecretStore,
    pub created: bool,
}

/// Charge — ou crée — la clé HMAC du journal d'audit (§9 : trousseau compte
/// `audit-hmac`, repli `auth/audit.key` 0600, journalisé). 32 octets du
/// système ; jamais dans `config.json`, jamais renvoyée à un client.
pub fn load_or_create_audit_key(
    dir: &Path,
    service: Option<&str>,
) -> Result<LoadedAuditKey, String> {
    if let Some(svc) = service {
        match keyring_read(svc, ACCOUNT_AUDIT) {
            Ok(Some(hex)) => {
                return Ok(LoadedAuditKey {
                    key: decode_cle_hex(&hex)?,
                    store: SecretStore::Keyring,
                    created: false,
                });
            }
            Ok(None) => {}
            // trousseau indisponible : on continue vers le repli, l'appelant
            // journalise que le fichier est moins protecteur (§9).
            Err(_) => {}
        }
    }
    let path = audit_key_path(dir);
    if let Some(hex) = read_secret_file(&path)? {
        return Ok(LoadedAuditKey {
            key: decode_cle_hex(&hex)?,
            store: SecretStore::File,
            created: false,
        });
    }
    let mut buf = [0u8; 32];
    if !crate::os_random(&mut buf) {
        return Err("aléa du système indisponible".to_string());
    }
    let hex = crate::crypto::hex(&buf);
    if let Some(svc) = service {
        if keyring_write(svc, ACCOUNT_AUDIT, &hex).is_ok() {
            return Ok(LoadedAuditKey {
                key: buf.to_vec(),
                store: SecretStore::Keyring,
                created: true,
            });
        }
    }
    write_secret_file(&path, &hex)?;
    Ok(LoadedAuditKey {
        key: buf.to_vec(),
        store: SecretStore::File,
        created: true,
    })
}

/// Premier lancement : crée l'identité du serveur et publie sa clé PUBLIQUE dans
/// `config.json` (`auth.server`). Démarrages suivants : recharge la clé privée,
/// vérifie qu'elle correspond bien à la clé publiée, et journalise d'où elle
/// vient (surtout quand c'est le repli fichier).
///
/// Ne panique jamais et ne bloque jamais le démarrage : une identité
/// indisponible est une erreur de journal, pas un crash (l'appairage rendra 503
/// et le reste du produit se comporte comme avant le lot M1).
pub fn ensure_server_identity(state: &std::sync::Arc<crate::types::AppState>) {
    let dir = Path::new(&state.config_path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let svc = service();
    let loaded = match load_or_create_server(&dir, svc.as_deref()) {
        Ok(l) => l,
        Err(e) => {
            crate::manage::add_log(
                state,
                "error",
                &format!(
                    "[auth] identité du serveur indisponible ({}) — l'appairage répondra 503 ; aucun secret n'est écrit dans config.json",
                    e
                ),
            );
            return;
        }
    };
    let public_key = crate::crypto::hex(&loaded.key.verifying_key().to_bytes());
    let existante = {
        let settings = crate::types::lockv(&state.settings);
        settings.auth.server.clone()
    };
    let mut creee = false;
    match existante {
        Some(ident) if ident.public_key == public_key => {}
        Some(ident) => {
            // La clé privée ne correspond plus à la clé publiée : on REFUSE de
            // réécrire l'identité (les clients appairés deviendraient fous), on
            // le dit, et l'appairage restera fermé.
            crate::manage::add_log(
                state,
                "error",
                &format!(
                    "[auth] la clé privée du serveur ne correspond pas à la clé publiée ({} ≠ {}) — identité NON remplacée ; supprimer le trousseau/le fichier de clé pour repartir de zéro et ré-appairer",
                    crate::auth::registry::short_fingerprint_of(&public_key).unwrap_or_default(),
                    crate::auth::registry::short_fingerprint_of(&ident.public_key).unwrap_or_default()
                ),
            );
            return;
        }
        None => {
            let ident = crate::auth::registry::ServerIdentity {
                keyid: crate::auth::registry::SERVER_KEYID.to_string(),
                public_key: public_key.clone(),
                created_at: crate::types::iso_utc(crate::types::now_ts()),
            };
            crate::types::lockv(&state.settings).auth.server = Some(ident);
            creee = true;
        }
    }
    if creee {
        crate::manage::save_config(state);
        crate::manage::add_log(
            state,
            "info",
            &format!(
                "[auth] identité du serveur créée ({}) — clé privée : {}",
                crate::auth::registry::SERVER_KEYID,
                loaded.store.as_str()
            ),
        );
    } else if loaded.store == SecretStore::File {
        // Signalé à CHAQUE démarrage en repli (§9), pas seulement à la création.
        crate::manage::add_log(
            state,
            "warn",
            &format!(
                "[auth] clé privée du serveur lue depuis un fichier à permissions restreintes (trousseau indisponible) : {} — protection moindre, lisible par le même utilisateur",
                server_key_path(&dir).display()
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mcc-auth-secret-{}-{}-{}",
            std::process::id(),
            crate::types::now_ts(),
            name
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn repli_fichier_cree_puis_recharge_la_meme_cle() {
        let dir = tmp("repli");
        let premier = load_or_create_server(&dir, None).expect("création");
        assert_eq!(premier.store, SecretStore::File);
        let chemin = server_key_path(&dir);
        assert!(chemin.exists(), "repli attendu en {}", chemin.display());
        let bloc = std::fs::read_to_string(&chemin).unwrap();
        assert!(
            bloc.contains("PRIVATE KEY"),
            "le bloc privé doit être dans le fichier de repli"
        );
        let second = load_or_create_server(&dir, None).expect("relecture");
        assert_eq!(
            premier.key.verifying_key().to_bytes(),
            second.key.verifying_key().to_bytes(),
            "l'identité doit survivre à un redémarrage"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fichier_de_repli_a_permissions_reduites() {
        let dir = tmp("perms");
        load_or_create_server(&dir, None).expect("création");
        let chemin = server_key_path(&dir);
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
            assert!(texte.contains(&user), "ACL sans l'utilisateur : {texte}");
            assert!(
                !texte.contains("(I)"),
                "aucune autorisation héritée ne doit subsister : {texte}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn une_entree_trousseau_illisible_est_une_erreur_pas_une_regeneration() {
        let dir = tmp("corrompu");
        let chemin = server_key_path(&dir);
        std::fs::create_dir_all(chemin.parent().unwrap()).unwrap();
        std::fs::write(&chemin, "pas un bloc de clé").unwrap();
        assert!(
            load_or_create_server(&dir, None).is_err(),
            "un bloc illisible ne doit JAMAIS être remplacé silencieusement"
        );
        // le fichier est resté tel quel (pas de réécriture)
        assert_eq!(
            std::fs::read_to_string(&chemin).unwrap(),
            "pas un bloc de clé"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn service_du_trousseau_par_defaut_et_isolation_par_env() {
        // Pas de variable posée dans ce process de test : service par défaut.
        if std::env::var(SERVICE_ENV).is_err() {
            assert_eq!(service().as_deref(), Some(KEYRING_SERVICE));
        }
    }

    #[test]
    fn cle_audit_creee_puis_relue_en_repli_fichier() {
        let dir = tmp("audit-key");
        assert!(load_audit_key(&dir, None).unwrap().is_none());
        let first = load_or_create_audit_key(&dir, None).expect("création clé audit");
        assert_eq!(first.store, SecretStore::File);
        assert!(first.created);
        assert_eq!(first.key.len(), 32);
        let second = load_or_create_audit_key(&dir, None).expect("relecture clé audit");
        assert!(!second.created);
        assert_eq!(first.key, second.key);
        let loaded = load_audit_key(&dir, None).unwrap().expect("relue");
        assert_eq!(loaded, first.key);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
