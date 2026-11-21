//! Registre des clients — `config.json`, section `auth` (docs/auth.md §3.2).
//!
//! Le fichier ne contient **que** des clés PUBLIQUES et des métadonnées : aucun
//! test ne doit y trouver une clé privée, une graine ni un secret (§9). Le
//! registre est relu à chaque requête par [`RegistryStore::lookup`], ce qui rend
//! une révocation effective à la requête suivante (exigence du lot M1).

use serde_json::{json, Map, Value};
use std::sync::Arc;

use crate::types::{lockv, AppState};

/// Les 4 droits du chantier. `admin` n'est **jamais** accordé à un client du
/// gateway : il n'existe que sur la session locale (§4, §6).
pub const RIGHT_LECTURE: &str = "lecture";
pub const RIGHT_PILOTAGE: &str = "pilotage";
pub const RIGHT_EXECUTION: &str = "execution";
pub const RIGHT_ADMIN: &str = "admin";

/// Droits qu'une approbation locale peut accorder (jamais `admin`).
pub const ALL_RIGHTS: [&str; 3] = [RIGHT_LECTURE, RIGHT_PILOTAGE, RIGHT_EXECUTION];

/// `keyid` de l'identité du serveur (§3.1 : identité fixe, créée au premier
/// lancement).
pub const SERVER_KEYID: &str = "mcc-server";

/// Seuil d'inactivité signalé dans l'écran « Accès » (docs/auth.md §3.3).
pub const STALE_180D_SECS: u64 = 180 * 86_400;

/// Analyse un horodatage UTC `"YYYY-MM-DD HH:MM:SS"` ou `"YYYY-MM-DDTHH:MM:SSZ"`
/// en secondes Unix. Inverse exact de [`crate::types::iso_utc`].
fn parse_iso_utc(s: &str) -> Option<u64> {
    let s = s.trim().strip_suffix('Z').unwrap_or(s.trim());
    let (date, time) = s.split_once(' ').or_else(|| s.split_once('T'))?;
    let mut dp = date.split('-');
    let y: i64 = dp.next()?.parse().ok()?;
    let m: i64 = dp.next()?.parse().ok()?;
    let d: i64 = dp.next()?.parse().ok()?;
    if dp.next().is_some()
        || !(1970..=9999).contains(&y)
        || !(1..=12).contains(&m)
        || !(1..=31).contains(&d)
    {
        return None;
    }
    let mut tp = time.split(':');
    let hh: u64 = tp.next()?.parse().ok()?;
    let mm: u64 = tp.next()?.parse().ok()?;
    let ss: u64 = tp.next()?.split('.').next()?.parse().ok()?;
    if tp.next().is_some() || hh > 23 || mm > 59 || ss > 59 {
        return None;
    }
    let y_adj = y - if m <= 2 { 1 } else { 0 };
    let era = y_adj.div_euclid(400);
    let yoe = y_adj.rem_euclid(400);
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    if days < 0 {
        return None;
    }
    let ts = (days as u64) * 86_400 + hh * 3600 + mm * 60 + ss;
    let canonical = format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}");
    if crate::types::iso_utc(ts) != canonical {
        return None;
    }
    Some(ts)
}

/// Un droit est accordable à un client s'il fait partie des trois ci-dessus.
pub fn is_grantable(right: &str) -> bool {
    ALL_RIGHTS.contains(&right)
}

/// `^mcc-[a-z0-9]{1,8}-[0-9a-f]{16}$` — MCP Control resserre la grammaire de la
/// lib (§3.1). Un `keyid` hors de cette forme n'entre jamais dans le registre.
pub fn valid_keyid(keyid: &str) -> bool {
    let Some(rest) = keyid.strip_prefix("mcc-") else {
        return false;
    };
    let Some((prefix, suffix)) = rest.rsplit_once('-') else {
        return false;
    };
    (1..=8).contains(&prefix.len())
        && prefix
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && suffix.len() == 16
        && suffix
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Décode 64 hexadécimaux minuscules en 32 octets. `None` si la forme est
/// mauvaise — l'appelant refuse alors la demande (échec fermé).
pub fn decode_public_key(hex: &str) -> Option<[u8; 32]> {
    // Hexadécimal MINUSCULE seulement : c'est la forme que la spec impose
    // (§3.1/§3.2) et celle que les deux côtés du protocole concatènent.
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        let pair = hex.get(i * 2..i * 2 + 2)?;
        *slot = u8::from_str_radix(pair, 16).ok()?;
    }
    // Un point Ed25519 invalide n'est pas une clé : la lib refuse, on refuse.
    litemcp::auth::sign::VerifyingKey::from_bytes(&out).ok()?;
    Some(out)
}

/// `ed25519:` + SHA-256(clé publique), forme complète (§3.1).
pub fn fingerprint_of(public_key_hex: &str) -> Option<String> {
    let bytes = decode_public_key(public_key_hex)?;
    let key = litemcp::auth::sign::VerifyingKey::from_bytes(&bytes).ok()?;
    Some(key.fingerprint().full())
}

/// Forme courte d'affichage (`ed25519:` + 16 hexadécimaux, §3.1).
pub fn short_fingerprint_of(public_key_hex: &str) -> Option<String> {
    let bytes = decode_public_key(public_key_hex)?;
    let key = litemcp::auth::sign::VerifyingKey::from_bytes(&bytes).ok()?;
    Some(key.fingerprint().short())
}

/// Pourquoi une clé ne peut pas entrer au registre — motif LOCAL seulement,
/// jamais renvoyé tel quel au réseau (§0 : refus génériques).
#[derive(Debug, PartialEq, Eq)]
pub enum RegistryError {
    KeyidInvalide,
    CleInvalide,
    Doublon,
    DroitRefuse,
    Inconnu,
    DejaRevoque,
}

impl RegistryError {
    pub fn motif(&self) -> &'static str {
        match self {
            RegistryError::KeyidInvalide => "keyid hors grammaire mcc-",
            RegistryError::CleInvalide => "clé publique invalide",
            RegistryError::Doublon => "keyid déjà présent au registre",
            RegistryError::DroitRefuse => "droit inconnu ou réservé (admin)",
            RegistryError::Inconnu => "keyid inconnu",
            RegistryError::DejaRevoque => "clé déjà révoquée",
        }
    }
}

/// Une ligne du registre (§3.2) : `created_at`/`last_used_at` comptent pour un
/// champ du tableau de la spec, d'où les 10 lignes et 11 valeurs ci-dessous.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientRecord {
    pub keyid: String,
    pub label: String,
    pub public_key: String,
    pub rights: Vec<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub status: String,
    pub revoked_at: Option<String>,
    pub rotated_to: Option<String>,
    pub paired_from: String,
    pub expires_at: Option<String>,
}

impl ClientRecord {
    pub fn new(
        keyid: String,
        label: String,
        public_key: String,
        rights: Vec<String>,
        paired_from: String,
    ) -> Self {
        ClientRecord {
            keyid,
            label,
            public_key,
            rights,
            created_at: crate::types::iso_utc(crate::types::now_ts()),
            last_used_at: None,
            status: "active".to_string(),
            revoked_at: None,
            rotated_to: None,
            paired_from,
            expires_at: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.status == "active"
    }

    /// Vrai si la clé est active et inutilisée depuis plus de 180 jours
    /// (`docs/auth.md` §3.3 : signalement seul, jamais de révocation d'office).
    pub fn stale_180d(&self, now: u64) -> bool {
        if !self.is_active() {
            return false;
        }
        let ref_str = self
            .last_used_at
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&self.created_at);
        let Some(ref_ts) = parse_iso_utc(ref_str) else {
            return false;
        };
        now.saturating_sub(ref_ts) > STALE_180D_SECS
    }

    /// Vue publique : ce que `client_list` et l'écran « Accès » affichent. La
    /// clé publique y est, l'empreinte aussi ; aucun secret, par construction.
    pub fn to_json(&self) -> Value {
        let mut o = Map::new();
        o.insert("keyid".into(), json!(self.keyid));
        o.insert("label".into(), json!(self.label));
        o.insert("public_key".into(), json!(self.public_key));
        o.insert("rights".into(), json!(self.rights));
        o.insert("created_at".into(), json!(self.created_at));
        if let Some(v) = &self.last_used_at {
            o.insert("last_used_at".into(), json!(v));
        }
        o.insert("status".into(), json!(self.status));
        if let Some(v) = &self.revoked_at {
            o.insert("revoked_at".into(), json!(v));
        }
        if let Some(v) = &self.rotated_to {
            o.insert("rotated_to".into(), json!(v));
        }
        o.insert("paired_from".into(), json!(self.paired_from));
        if let Some(v) = &self.expires_at {
            o.insert("expires_at".into(), json!(v));
        }
        Value::Object(o)
    }

    pub fn from_json(v: &Value) -> Option<Self> {
        let keyid = v.get("keyid")?.as_str()?.to_string();
        if !valid_keyid(&keyid) {
            return None;
        }
        let public_key = v.get("public_key")?.as_str()?.to_string();
        decode_public_key(&public_key)?;
        let mut rights: Vec<String> = Vec::new();
        for r in v
            .get("rights")
            .and_then(|x| x.as_array())
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            let Some(r) = r.as_str() else { continue };
            if is_grantable(r) && !rights.iter().any(|e| e == r) {
                rights.push(r.to_string());
            }
        }
        Some(ClientRecord {
            keyid,
            label: v
                .get("label")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .chars()
                .take(64)
                .collect(),
            public_key,
            rights,
            created_at: v
                .get("created_at")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            last_used_at: v
                .get("last_used_at")
                .and_then(|x| x.as_str())
                .map(String::from),
            status: if v.get("status").and_then(|x| x.as_str()) == Some("revoked") {
                "revoked".to_string()
            } else {
                "active".to_string()
            },
            revoked_at: v
                .get("revoked_at")
                .and_then(|x| x.as_str())
                .map(String::from),
            rotated_to: v
                .get("rotated_to")
                .and_then(|x| x.as_str())
                .map(String::from),
            paired_from: v
                .get("paired_from")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            expires_at: v
                .get("expires_at")
                .and_then(|x| x.as_str())
                .map(String::from),
        })
    }
}

/// Identité PUBLIQUE du serveur (`auth.server`, §8.1). La clé privée vit au
/// trousseau (`auth::secret`), jamais ici.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerIdentity {
    pub keyid: String,
    pub public_key: String,
    pub created_at: String,
}

impl ServerIdentity {
    pub fn to_json(&self) -> Value {
        json!({
            "keyid": self.keyid,
            "public_key": self.public_key,
            "created_at": self.created_at,
        })
    }

    pub fn from_json(v: &Value) -> Option<Self> {
        let public_key = v.get("public_key")?.as_str()?.to_string();
        decode_public_key(&public_key)?;
        Some(ServerIdentity {
            keyid: v
                .get("keyid")
                .and_then(|x| x.as_str())
                .unwrap_or(SERVER_KEYID)
                .to_string(),
            public_key,
            created_at: v
                .get("created_at")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
        })
    }
}

/// Tout l'état d'authentification persisté (et les demandes d'appairage en
/// cours, éphémères). Rangé dans `Settings` plutôt qu'en champ d'`AppState`
/// pour rester dans les fichiers autorisés du lot ; il est écrit par
/// `save_config` à la clé `auth` de `config.json`.
#[derive(Clone, Debug, Default)]
pub struct AuthState {
    pub server: Option<ServerIdentity>,
    pub clients: Vec<ClientRecord>,
    pub pairs: Vec<crate::auth::pair::PairRequest>,
    /// Empreinte `sha256:<64 hex>` du certificat TLS 1.3 actif (vide quand
    /// l'écoute est en loopback, `docs/auth.md` §3.4 et §7).
    pub tls_fp: String,
    /// Vrai si `ui_bind` hors loopback a été refusé au démarrage (binaire sans
    /// feature `tls` ou certificat invalide, DA5 = a).
    pub tls_bind_refused: bool,
    /// Message d'information/refus TLS exposé à l'interface (`/api/state`).
    pub tls_notice: Option<String>,
}

impl AuthState {
    pub fn from_json(v: &Value) -> Self {
        let mut st = AuthState::default();
        if let Some(s) = v.get("server") {
            st.server = ServerIdentity::from_json(s);
        }
        if let Some(arr) = v.get("clients").and_then(|x| x.as_array()) {
            for c in arr {
                if let Some(rec) = ClientRecord::from_json(c) {
                    // un keyid en double dans le fichier (édition manuelle) :
                    // la première ligne gagne, jamais d'écrasement silencieux
                    if !st.clients.iter().any(|e| e.keyid == rec.keyid) {
                        st.clients.push(rec);
                    }
                }
            }
        }
        st
    }

    pub fn to_json(&self) -> Value {
        let clients: Vec<Value> = self.clients.iter().map(ClientRecord::to_json).collect();
        let mut o = Map::new();
        if let Some(s) = &self.server {
            o.insert("server".into(), s.to_json());
        }
        o.insert("clients".into(), json!(clients));
        Value::Object(o)
    }

    pub fn find(&self, keyid: &str) -> Option<&ClientRecord> {
        self.clients.iter().find(|c| c.keyid == keyid)
    }

    pub fn find_mut(&mut self, keyid: &str) -> Option<&mut ClientRecord> {
        self.clients.iter_mut().find(|c| c.keyid == keyid)
    }

    pub fn server_public_key(&self) -> Option<[u8; 32]> {
        decode_public_key(&self.server.as_ref()?.public_key)
    }

    /// Ajoute une ligne validée. L'appelant tient le verrou des réglages.
    pub fn insert(&mut self, rec: ClientRecord) -> Result<(), RegistryError> {
        if !valid_keyid(&rec.keyid) {
            return Err(RegistryError::KeyidInvalide);
        }
        if decode_public_key(&rec.public_key).is_none() {
            return Err(RegistryError::CleInvalide);
        }
        if rec.rights.iter().any(|r| !is_grantable(r)) {
            return Err(RegistryError::DroitRefuse);
        }
        if self.find(&rec.keyid).is_some() {
            return Err(RegistryError::Doublon);
        }
        self.clients.push(rec);
        Ok(())
    }

    /// Change les droits d'un client. Un droit inconnu (ou `admin`) fait refuser
    /// l'opération ENTIÈRE : jamais d'application partielle.
    pub fn set_rights(&mut self, keyid: &str, rights: Vec<String>) -> Result<(), RegistryError> {
        if rights.iter().any(|r| !is_grantable(r)) {
            return Err(RegistryError::DroitRefuse);
        }
        let rec = self.find_mut(keyid).ok_or(RegistryError::Inconnu)?;
        rec.rights = rights;
        Ok(())
    }

    pub fn revoke(&mut self, keyid: &str) -> Result<(), RegistryError> {
        let rec = self.find_mut(keyid).ok_or(RegistryError::Inconnu)?;
        if !rec.is_active() {
            return Err(RegistryError::DejaRevoque);
        }
        rec.status = "revoked".to_string();
        rec.revoked_at = Some(crate::types::iso_utc(crate::types::now_ts()));
        Ok(())
    }

    /// Rotation : nouveau `keyid` (+ sa clé publique), mêmes droits ; l'ancien
    /// passe `revoked` avec `rotated_to` (§3.3). Aucune clé privée ne transite :
    /// le client fournit sa nouvelle clé PUBLIQUE.
    pub fn rotate(
        &mut self,
        keyid: &str,
        new_keyid: String,
        public_key: String,
    ) -> Result<ClientRecord, RegistryError> {
        let ancien = self.find(keyid).ok_or(RegistryError::Inconnu)?.clone();
        if !ancien.is_active() {
            return Err(RegistryError::DejaRevoque);
        }
        if !valid_keyid(&new_keyid) {
            return Err(RegistryError::KeyidInvalide);
        }
        if self.find(&new_keyid).is_some() {
            return Err(RegistryError::Doublon);
        }
        if decode_public_key(&public_key).is_none() {
            return Err(RegistryError::CleInvalide);
        }
        let nouveau = ClientRecord::new(
            new_keyid,
            ancien.label.clone(),
            public_key,
            ancien.rights.clone(),
            ancien.paired_from.clone(),
        );
        self.insert(nouveau.clone())?;
        if let Some(old) = self.find_mut(keyid) {
            old.status = "revoked".to_string();
            old.revoked_at = Some(crate::types::iso_utc(crate::types::now_ts()));
            old.rotated_to = Some(nouveau.keyid.clone());
        }
        Ok(nouveau)
    }
}

/// Signature du vecteur 1 de la RFC 8032 §7.1 (message **vide**) — donnée
/// PUBLIQUE, employée uniquement pour payer le même prix qu'une vraie
/// vérification sur les chemins d'échec (voir [`verification_fantome`]).
const SIGNATURE_FANTOME: [u8; 64] = [
    0xe5, 0x56, 0x43, 0x00, 0xc3, 0x60, 0xac, 0x72, 0x90, 0x86, 0xe2, 0xcc, 0x80, 0x6e, 0x82, 0x8a,
    0x84, 0x87, 0x7f, 0x1e, 0xb8, 0xe5, 0xd9, 0x74, 0xd8, 0x73, 0xe0, 0x65, 0x22, 0x49, 0x01, 0x55,
    0x5f, 0xb8, 0x82, 0x15, 0x90, 0xa3, 0x3b, 0xac, 0xc6, 0x1e, 0x39, 0x70, 0x1c, 0xf9, 0xb4, 0x6b,
    0xd2, 0x5b, 0xf5, 0xf0, 0x59, 0x5b, 0xbe, 0x24, 0x65, 0x51, 0x41, 0x43, 0x8e, 0x7a, 0x10, 0x0b,
];

/// Clé publique du vecteur 1 de la RFC 8032 §7.1 — publique.
const CLE_FANTOME: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

/// Paie une **vraie** vérification Ed25519 sur un chemin d'échec.
///
/// `signature_invalid` fait une vérification, `key_unknown` et `key_revoked`
/// s'arrêteraient avant : sans ce travail, le temps de réponse dirait à
/// l'attaquant *pourquoi* sa requête a été refusée (oracle temporel). Le
/// vecteur RFC 8032 est public et son résultat n'est jamais lu — seul le coût
/// compte. Le tout reste très en dessous d'une requête valide et n'ouvre rien.
fn verification_fantome_octets(bytes: &[u8; 32]) {
    let Ok(cle) = litemcp::auth::sign::VerifyingKey::from_bytes(bytes) else {
        return;
    };
    let _ = cle.verify_message(b"", &SIGNATURE_FANTOME);
}

fn verification_fantome() {
    let Some(bytes) = decode_public_key(CLE_FANTOME) else {
        return;
    };
    verification_fantome_octets(&bytes);
}

/// [`litemcp::auth::sign::KeyStore`] branché sur `config.json` : la lib relit la
/// clé à chaque requête, donc une révocation prend effet immédiatement (§8.2 C6).
pub struct RegistryStore {
    pub state: Arc<AppState>,
}

impl litemcp::auth::sign::KeyStore for RegistryStore {
    fn lookup(&self, keyid: &str) -> Option<litemcp::auth::sign::ClientKey> {
        // Le verrou des réglages n'est PAS tenu pendant la vérification fantôme :
        // elle coûte un Ed25519, et la fenêtre locale lit les mêmes réglages.
        let rec = {
            let settings = lockv(&self.state.settings);
            settings.auth.find(keyid).cloned()
        };
        let Some(rec) = rec else {
            verification_fantome();
            return None;
        };
        let Some(public_key) = decode_public_key(&rec.public_key) else {
            verification_fantome();
            return None;
        };
        let actif = rec.is_active();
        if !actif {
            // `decode_public_key` a déjà coûté une décompression ; il reste
            // exactement une seconde décompression + une vérification Ed25519,
            // comme `SignatureVerifier` sur `signature_invalid`.
            verification_fantome_octets(&public_key);
        }
        let mut key = litemcp::auth::sign::ClientKey::new(rec.keyid.clone(), public_key);
        if !rec.label.is_empty() {
            key = key.with_label(rec.label.clone());
        }
        key = key.with_rights(rec.rights.clone());
        if !actif {
            key = key.revoked();
        }
        Some(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clé publique du premier vecteur de test RFC 8032 §7.1 — publique, donc
    /// utilisable dans un test committé (aucune clé privée ici).
    const CLE: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

    #[test]
    fn grammaire_keyid_resserree() {
        assert!(valid_keyid("mcc-ai-3f9a1c07b2d84e51"));
        assert!(valid_keyid("mcc-a-0000000000000000"));
        assert!(valid_keyid("mcc-12345678-abcdef0123456789"));
        for mauvais in [
            "mcc-ai-3F9A1C07B2D84E51",        // majuscules
            "mcc--3f9a1c07b2d84e51",          // préfixe vide
            "mcc-123456789-3f9a1c07b2d84e51", // préfixe trop long
            "mcc-ai-3f9a1c07b2d84e5",         // 15 hex
            "mcc-ai-3f9a1c07b2d84e511",       // 17 hex
            "MCC-ai-3f9a1c07b2d84e51",        // préfixe global
            "mcc_ai_3f9a1c07b2d84e51",        // séparateurs
            "ai-3f9a1c07b2d84e51",
            "",
        ] {
            assert!(!valid_keyid(mauvais), "keyid accepté à tort : {mauvais}");
        }
    }

    #[test]
    fn cle_publique_hex_validee() {
        assert!(decode_public_key(CLE).is_some());
        assert!(
            decode_public_key(&CLE.to_uppercase()).is_none(),
            "hex majuscule refusé (spec)"
        );
        assert!(decode_public_key("zz").is_none());
        assert!(
            decode_public_key(&format!("02{}", "00".repeat(31))).is_none(),
            "y=2 n'est pas sur la courbe edwards25519 : point invalide refusé"
        );
    }

    #[test]
    fn empreinte_forme_spec() {
        let fp = fingerprint_of(CLE).expect("empreinte");
        assert!(fp.starts_with("ed25519:"), "{fp}");
        assert_eq!(fp.len(), 8 + 64);
        assert!(fp[8..]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        assert_eq!(short_fingerprint_of(CLE).unwrap().len(), 8 + 16);
    }

    fn rec(keyid: &str, droits: &[&str]) -> ClientRecord {
        ClientRecord::new(
            keyid.into(),
            "poste de test".into(),
            CLE.into(),
            droits.iter().map(|s| s.to_string()).collect(),
            "127.0.0.1".into(),
        )
    }

    #[test]
    fn registre_insere_refuse_et_revoque() {
        let mut st = AuthState::default();
        st.insert(rec("mcc-a-0000000000000001", &["lecture"]))
            .unwrap();
        // doublon
        assert_eq!(
            st.insert(rec("mcc-a-0000000000000001", &["lecture"])),
            Err(RegistryError::Doublon)
        );
        // droit réservé
        assert_eq!(
            st.insert(rec("mcc-a-0000000000000002", &["admin"])),
            Err(RegistryError::DroitRefuse)
        );
        // keyid hors grammaire
        assert_eq!(
            st.insert(rec("mcc-A-0000000000000003", &["lecture"])),
            Err(RegistryError::KeyidInvalide)
        );
        // révocation + idempotence refusée
        st.revoke("mcc-a-0000000000000001").unwrap();
        assert!(!st.find("mcc-a-0000000000000001").unwrap().is_active());
        assert!(st
            .find("mcc-a-0000000000000001")
            .unwrap()
            .revoked_at
            .is_some());
        assert_eq!(
            st.revoke("mcc-a-0000000000000001"),
            Err(RegistryError::DejaRevoque)
        );
        assert_eq!(
            st.revoke("mcc-a-0000000000000009"),
            Err(RegistryError::Inconnu)
        );
    }

    #[test]
    fn droits_ensemble_vide_cle_inerte_et_set_scopes_atomique() {
        let mut st = AuthState::default();
        st.insert(rec("mcc-a-0000000000000001", &[])).unwrap();
        assert!(st.find("mcc-a-0000000000000001").unwrap().rights.is_empty());
        st.set_rights("mcc-a-0000000000000001", vec!["lecture".into()])
            .unwrap();
        // un droit invalide dans la liste → refus TOTAL, les droits n'ont pas bougé
        assert_eq!(
            st.set_rights(
                "mcc-a-0000000000000001",
                vec!["lecture".into(), "admin".into()]
            ),
            Err(RegistryError::DroitRefuse)
        );
        assert_eq!(
            st.find("mcc-a-0000000000000001").unwrap().rights,
            vec!["lecture".to_string()]
        );
    }

    #[test]
    fn rotation_transfere_les_droits_et_trace_l_ancien() {
        let mut st = AuthState::default();
        st.insert(rec("mcc-a-0000000000000001", &["lecture", "pilotage"]))
            .unwrap();
        let nouveau = st
            .rotate(
                "mcc-a-0000000000000001",
                "mcc-a-0000000000000002".into(),
                CLE.into(),
            )
            .unwrap();
        assert_eq!(
            nouveau.rights,
            vec!["lecture".to_string(), "pilotage".to_string()]
        );
        let ancien = st.find("mcc-a-0000000000000001").unwrap();
        assert_eq!(ancien.status, "revoked");
        assert_eq!(ancien.rotated_to.as_deref(), Some("mcc-a-0000000000000002"));
        // rotation d'une clé déjà révoquée → refus
        assert_eq!(
            st.rotate(
                "mcc-a-0000000000000001",
                "mcc-a-0000000000000003".into(),
                CLE.into()
            ),
            Err(RegistryError::DejaRevoque)
        );
    }

    #[test]
    fn json_aller_retour_sans_cle_privee() {
        let mut st = AuthState {
            server: Some(ServerIdentity {
                keyid: SERVER_KEYID.into(),
                public_key: CLE.into(),
                created_at: "2026-11-15 12:00:00".into(),
            }),
            ..AuthState::default()
        };
        st.insert(rec("mcc-a-0000000000000001", &["lecture"]))
            .unwrap();
        let texte = st.to_json().to_string();
        assert!(!texte.contains("PRIVATE"), "{texte}");
        assert!(!texte.contains("seed"), "{texte}");
        let relu = AuthState::from_json(&serde_json::from_str(&texte).unwrap());
        assert_eq!(relu.server, st.server);
        assert_eq!(relu.clients, st.clients);
        // une ligne invalide est ignorée, jamais promue
        let corrompu: Value = serde_json::from_str(
            r#"{"clients":[{"keyid":"n-importe-quoi","public_key":"00","rights":["admin"]}]}"#,
        )
        .unwrap();
        assert!(AuthState::from_json(&corrompu).clients.is_empty());
    }

    #[test]
    fn stale_180d_seuils_et_roundtrip_utc() {
        for ts in [0, 1, 86_399, 951_782_400, 1_700_000_000, 4_102_444_800] {
            let formatted = crate::types::iso_utc(ts);
            assert_eq!(parse_iso_utc(&formatted), Some(ts), "{formatted}");
        }
        assert_eq!(parse_iso_utc("2023-11-14T22:13:20Z"), Some(1_700_000_000));
        assert_eq!(parse_iso_utc("2026-02-29 12:00:00"), None);
        assert_eq!(parse_iso_utc(""), None);

        let now = 1_800_000_000u64;
        let mut c = rec("mcc-a-0000000000000001", &["lecture"]);
        c.created_at = crate::types::iso_utc(now - STALE_180D_SECS);
        c.last_used_at = None;
        assert!(!c.stale_180d(now), "exactement 180j = pas encore obsolète");

        c.created_at = crate::types::iso_utc(now - STALE_180D_SECS - 1);
        assert!(c.stale_180d(now), "> 180j sans usage = obsolète");

        // Un usage récent annule l'ancienneté de created_at
        c.last_used_at = Some(crate::types::iso_utc(now - 3600));
        assert!(!c.stale_180d(now));

        // Usage > 180j = obsolète
        c.last_used_at = Some(crate::types::iso_utc(now - STALE_180D_SECS - 10));
        assert!(c.stale_180d(now));

        // Client déjà révoqué : jamais signalé comme simple clé inactive
        c.status = "revoked".to_string();
        assert!(!c.stale_180d(now));
    }
}
