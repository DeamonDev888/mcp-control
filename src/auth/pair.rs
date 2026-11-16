//! Appairage — protocole en 5 messages de `docs/auth.md` §3.4.
//!
//! `POST /auth/pair` est **non authentifié** (c'est son objet) mais borné :
//! 5 demandes par adresse et par minute, 16 demandes en attente au plus (la plus
//! ancienne évincée), expiration à 2 minutes, `pair_id` inconnu → 404. Rien
//! n'est accordé tant qu'une action locale `pair_approve` (droit `admin`) n'a pas
//! été faite : la demande seule ne crée AUCUNE ligne au registre.
//!
//! Le code de comparaison à 6 chiffres n'est pas un secret : il sert uniquement
//! à ce qu'un humain détecte une substitution de clé (deux relais donnent deux
//! codes différents puisque les clés et `tls_fp` diffèrent).

use serde_json::{json, Value};
use std::net::IpAddr;
use std::sync::Arc;

use crate::auth::registry::{is_grantable, valid_keyid, ClientRecord, RegistryError, SERVER_KEYID};
use crate::types::{iso_utc, lockv, AppState};

/// Durée de vie d'une demande (§3.3).
pub const PAIR_TTL_SECS: u64 = 120;
/// Demandes en attente au plus, la plus ancienne évincée (§3.3).
pub const PAIR_MAX_PENDING: usize = 16;
/// Une demande décidée reste lisible ce délai : un client qui approuve à la
/// dernière seconde doit encore lire `approved`, pas `expired`.
pub const PAIR_DECIDED_TTL_SECS: u64 = 120;
/// 5 demandes par adresse et par minute (§3.3).
pub const PAIR_RATE_MAX: usize = 5;
pub const PAIR_RATE_WINDOW_SECS: u64 = 60;
/// Adresses suivies au plus par le limiteur (borne mémoire de l'état global).
pub const PAIR_RATE_KEYS_MAX: usize = 1024;
/// Nom du protocole, renvoyé au client (il refuse un `api` inconnu).
pub const PAIR_API: &str = "mcc-auth-v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairDecision {
    Approved(Vec<String>),
    Rejected,
}

/// Une demande d'appairage. Aucun secret : `public_key` est publique, `code`
/// est un simple contrôle visuel humain.
#[derive(Clone, Debug)]
pub struct PairRequest {
    pub pair_id: String,
    pub keyid: String,
    pub label: String,
    pub public_key: String,
    pub code: String,
    pub created: u64,
    pub paired_from: String,
    pub decision: Option<PairDecision>,
    pub decided_at: Option<u64>,
}

impl PairRequest {
    pub fn status(&self, now: u64) -> &'static str {
        match &self.decision {
            Some(PairDecision::Approved(_)) => "approved",
            Some(PairDecision::Rejected) => "rejected",
            None if now >= self.created.saturating_add(PAIR_TTL_SECS) => "expired",
            None => "pending",
        }
    }

    pub fn is_pending(&self, now: u64) -> bool {
        self.status(now) == "pending"
    }

    pub fn rights(&self) -> Vec<String> {
        match &self.decision {
            Some(PairDecision::Approved(r)) => r.clone(),
            _ => Vec::new(),
        }
    }
}

/// `code = SHA-256("mcc-pair-v1" ‖ pair_id ‖ client_pub ‖ server_pub ‖ tls_fp)[0..4]`
/// lu en entier 32 bits big-endian, modulo 1 000 000, affiché en six chiffres
/// groupés (§3.4 message 3). Les deux côtés concatènent les mêmes chaînes
/// ASCII : `pair_id` (32 hex), les deux clés en hexadécimal minuscule, et
/// `tls_fp` (`` vide tant que l'écoute est en loopback).
pub fn comparison_code(pair_id: &str, client_pub: &str, server_pub: &str, tls_fp: &str) -> String {
    let mut msg =
        Vec::with_capacity(64 + pair_id.len() + client_pub.len() + server_pub.len() + tls_fp.len());
    msg.extend_from_slice(b"mcc-pair-v1");
    msg.extend_from_slice(pair_id.as_bytes());
    msg.extend_from_slice(client_pub.as_bytes());
    msg.extend_from_slice(server_pub.as_bytes());
    msg.extend_from_slice(tls_fp.as_bytes());
    let digest = crate::crypto::sha256(&msg);
    let n = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 1_000_000;
    format!("{:03} {:03}", n / 1000, n % 1000)
}

fn pair_rate_slot() -> &'static std::sync::Mutex<std::collections::HashMap<IpAddr, Vec<u64>>> {
    static SLOT: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<IpAddr, Vec<u64>>>,
    > = std::sync::OnceLock::new();
    SLOT.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Même limiteur que `/api/hello` (fenêtre glissante de 60 s), avec le quota de
/// l'appairage. `false` = au-delà de `PAIR_RATE_MAX` demandes dans la fenêtre.
/// Appliqué à TOUTES les adresses : une tempête locale doit être bornée aussi
/// (critère E2E), et 5 demandes/minute suffisent à un humain qui s'appaire.
pub fn pair_rate_ok(ip: IpAddr, now: u64) -> bool {
    let mut slot = pair_rate_slot().lock().unwrap_or_else(|p| p.into_inner());
    let prior = slot.get(&ip).map(Vec::as_slice).unwrap_or(&[]);
    let (decision, next) = crate::rate_decide(now, prior, PAIR_RATE_MAX, PAIR_RATE_WINDOW_SECS);
    slot.insert(ip, next);
    // Purge par ANCIENNETÉ et non par l'horloge de l'appelant : un appel daté
    // plus tard (autre instance, test) ne doit pas effacer l'historique d'une
    // autre adresse. La table reste bornée par la purge du stamp le plus vieux.
    if slot.len() > PAIR_RATE_KEYS_MAX {
        if let Some(plus_vieille) = slot
            .iter()
            .min_by_key(|(_, stamps)| stamps.last().copied().unwrap_or(0))
            .map(|(k, _)| *k)
        {
            slot.remove(&plus_vieille);
        }
    }
    decision.is_ok()
}

/// Élague les demandes mortes et borne la file. Pure (testable avec une horloge
/// injectée).
pub fn prune(pairs: &mut Vec<PairRequest>, now: u64) {
    pairs.retain(|p| match p.decision {
        // Une demande NON décidée survit à son expiration le temps que le client
        // lise `expired` : la supprimer à la seconde près rendrait 404 à un
        // client qui interroge juste après la limite (et non « expiré »).
        None => {
            now < p
                .created
                .saturating_add(PAIR_TTL_SECS + PAIR_DECIDED_TTL_SECS)
        }
        Some(_) => {
            now < p
                .decided_at
                .unwrap_or(p.created)
                .saturating_add(PAIR_DECIDED_TTL_SECS)
        }
    });
    // plafond : 16 en attente ; la plus ancienne évincée (§3.3)
    while pairs.iter().filter(|p| p.decision.is_none()).count() > PAIR_MAX_PENDING {
        let Some(idx) = pairs
            .iter()
            .enumerate()
            .filter(|(_, p)| p.decision.is_none())
            .min_by_key(|(_, p)| p.created)
            .map(|(i, _)| i)
        else {
            break;
        };
        pairs.remove(idx);
    }
}

fn json_error(status: u16, motif: &str) -> (u16, &'static str, Vec<u8>) {
    (
        status,
        "application/json",
        json!({ "ok": false, "error": motif })
            .to_string()
            .into_bytes(),
    )
}

/// Corps de `POST /auth/pair` (message 2). Validation dans l'ordre du §2.1 de la
/// lib : forme d'abord (gratuit), registre ensuite.
pub fn handle_post(
    state: &Arc<AppState>,
    body: &[u8],
    from: Option<IpAddr>,
    now: u64,
) -> (u16, &'static str, Vec<u8>) {
    let v: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return json_error(400, "demande invalide"),
    };
    let keyid = v.get("keyid").and_then(|x| x.as_str()).unwrap_or("");
    if !valid_keyid(keyid) {
        return json_error(400, "demande invalide");
    }
    let public_key = v.get("public_key").and_then(|x| x.as_str()).unwrap_or("");
    if crate::auth::registry::decode_public_key(public_key).is_none() {
        return json_error(400, "demande invalide");
    }
    let created = match v.get("created").and_then(|x| x.as_u64()) {
        Some(c) => c,
        None => return json_error(400, "demande invalide"),
    };
    if now.abs_diff(created) > 60 {
        return json_error(400, "demande invalide");
    }
    // Libellé : ≤ 64 caractères, caractères de contrôle retirés. Il sera
    // affiché ÉCHAPPÉ par l'UI (§3.2) — jamais interprété.
    let label: String = v
        .get("label")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect();
    let paired_from = from
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "-".to_string());
    if let Some(ip) = from {
        if !pair_rate_ok(ip, now) {
            return json_error(
                429,
                "trop de demandes d'appairage pour cette adresse — réessayez dans une minute",
            );
        }
    }

    let mut settings = lockv(&state.settings);
    let Some(server) = settings.auth.server.clone() else {
        return json_error(503, "identité du serveur indisponible");
    };
    if settings.auth.find(keyid).is_some() {
        return json_error(409, "keyid déjà enregistré");
    }
    let pairs = &mut settings.auth.pairs;
    prune(pairs, now);
    // Même keyid déjà en attente : on rend LA MÊME demande (un client qui
    // réessaie ne crée pas de doublon). Clé différente = conflit.
    if let Some(existing) = pairs.iter().find(|p| p.keyid == keyid && p.is_pending(now)) {
        if existing.public_key != public_key {
            return json_error(409, "keyid déjà en attente avec une autre clé");
        }
        let resp = json!({
            "pair_id": existing.pair_id,
            "expires_in": PAIR_TTL_SECS,
            "api": PAIR_API,
            "server_keyid": server.keyid,
            "server_pub": server.public_key,
            "tls_fp": "",
        });
        return (200, "application/json", resp.to_string().into_bytes());
    }
    let pair_id = crate::random_hex(2);
    let code = comparison_code(&pair_id, public_key, &server.public_key, "");
    pairs.push(PairRequest {
        pair_id: pair_id.clone(),
        keyid: keyid.to_string(),
        label,
        public_key: public_key.to_string(),
        code,
        created: now,
        paired_from,
        decision: None,
        decided_at: None,
    });
    prune(pairs, now);
    let resp = json!({
        "pair_id": pair_id,
        "expires_in": PAIR_TTL_SECS,
        "api": PAIR_API,
        "server_keyid": server.keyid,
        "server_pub": server.public_key,
        "tls_fp": "",
    });
    (200, "application/json", resp.to_string().into_bytes())
}

/// `GET /auth/pair/{pair_id}` (message 5). `pair_id` inconnu → 404 : les
/// demandes ne sont pas énumérables sans connaître ces 16 octets.
pub fn handle_get(state: &Arc<AppState>, pair_id: &str, now: u64) -> (u16, &'static str, Vec<u8>) {
    if pair_id.len() != 32
        || !pair_id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return json_error(404, "demande inconnue");
    }
    let reponse = {
        let mut settings = lockv(&state.settings);
        prune(&mut settings.auth.pairs, now);
        let Some(pair) = settings.auth.pairs.iter().find(|p| p.pair_id == pair_id) else {
            return json_error(404, "demande inconnue");
        };
        let Some(server) = settings.auth.server.as_ref() else {
            return json_error(503, "identité du serveur indisponible");
        };
        (pair.status(now), pair.rights(), server.public_key.clone())
    };
    let resp = json!({
        "status": reponse.0,
        "rights": reponse.1,
        "server_pub": reponse.2,
        "tls_fp": "",
    });
    (200, "application/json", resp.to_string().into_bytes())
}

/// Action locale `pair_list` : ce que la fenêtre affiche pour comparer le code.
pub fn list(state: &Arc<AppState>, now: u64) -> Value {
    let mut settings = lockv(&state.settings);
    prune(&mut settings.auth.pairs, now);
    let server_pub = settings
        .auth
        .server
        .as_ref()
        .map(|s| s.public_key.clone())
        .unwrap_or_default();
    let pending: Vec<Value> = settings
        .auth
        .pairs
        .iter()
        .filter(|p| p.is_pending(now))
        .map(|p| {
            json!({
                "pair_id": p.pair_id,
                "keyid": p.keyid,
                "label": p.label,
                "public_key": p.public_key,
                "fingerprint": crate::auth::registry::fingerprint_of(&p.public_key).unwrap_or_default(),
                "fingerprint_court": crate::auth::registry::short_fingerprint_of(&p.public_key).unwrap_or_default(),
                "code": p.code,
                "paired_from": p.paired_from,
                "age_secs": now.saturating_sub(p.created),
                "expires_in": p.created.saturating_add(PAIR_TTL_SECS).saturating_sub(now),
            })
        })
        .collect();
    json!({
        "ok": true,
        "server_keyid": SERVER_KEYID,
        "server_pub": server_pub,
        "pending": pending,
        "pending_max": PAIR_MAX_PENDING,
        "ttl_secs": PAIR_TTL_SECS,
    })
}

/// Action locale `pair_approve` : SEUL chemin qui inscrit un client au registre.
/// Les droits sont validés (jamais `admin`) ; une demande expirée ou déjà
/// décidée est refusée.
pub fn approve(
    state: &Arc<AppState>,
    pair_id: &str,
    rights: Vec<String>,
    now: u64,
) -> Result<Value, String> {
    if rights.iter().any(|r| !is_grantable(r)) {
        return Err(RegistryError::DroitRefuse.motif().to_string());
    }
    let mut settings = lockv(&state.settings);
    prune(&mut settings.auth.pairs, now);
    // 1) lire la demande (emprunt court), 2) inscrire au registre, 3) marquer
    // la décision : trois étapes disjointes, aucun emprunt croisé.
    let demande = {
        let Some(pair) = settings.auth.pairs.iter().find(|p| p.pair_id == pair_id) else {
            return Err("demande d'appairage inconnue".to_string());
        };
        if pair.decision.is_some() {
            return Err("demande déjà décidée".to_string());
        }
        if !pair.is_pending(now) {
            return Err("demande expirée".to_string());
        }
        (
            pair.keyid.clone(),
            pair.label.clone(),
            pair.public_key.clone(),
            pair.paired_from.clone(),
        )
    };
    let rec = ClientRecord::new(
        demande.0.clone(),
        demande.1,
        demande.2,
        rights.clone(),
        demande.3,
    );
    let fingerprint = crate::auth::registry::fingerprint_of(&rec.public_key).unwrap_or_default();
    settings
        .auth
        .insert(rec)
        .map_err(|e| e.motif().to_string())?;
    if let Some(pair) = settings
        .auth
        .pairs
        .iter_mut()
        .find(|p| p.pair_id == pair_id)
    {
        pair.decision = Some(PairDecision::Approved(rights.clone()));
        pair.decided_at = Some(now);
    }
    Ok(json!({
        "ok": true,
        "pair_id": pair_id,
        "keyid": demande.0,
        "rights": rights,
        "fingerprint": fingerprint,
        "decided_at": iso_utc(now),
    }))
}

pub fn reject(state: &Arc<AppState>, pair_id: &str, now: u64) -> Result<Value, String> {
    let mut settings = lockv(&state.settings);
    prune(&mut settings.auth.pairs, now);
    let Some(pair) = settings
        .auth
        .pairs
        .iter_mut()
        .find(|p| p.pair_id == pair_id)
    else {
        return Err("demande d'appairage inconnue".to_string());
    };
    if pair.decision.is_some() {
        return Err("demande déjà décidée".to_string());
    }
    let deja_expiree = !pair.is_pending(now);
    pair.decision = Some(PairDecision::Rejected);
    pair.decided_at = Some(now);
    if deja_expiree {
        // refus d'une demande morte : rien à décider, mais l'état est posé pour
        // que le client lise `rejected` plutôt que d'attendre indéfiniment
        return Ok(json!({ "ok": true, "pair_id": pair_id, "expiree": true }));
    }
    Ok(json!({ "ok": true, "pair_id": pair_id }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::registry::RIGHT_LECTURE;

    fn state() -> Arc<AppState> {
        let mut st = crate::types::Settings::default();
        st.auth.server = Some(crate::auth::registry::ServerIdentity {
            keyid: SERVER_KEYID.into(),
            public_key: "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a".into(),
            created_at: "2026-11-15 12:00:00".into(),
        });
        Arc::new(AppState {
            servers: std::sync::Mutex::new(vec![]),
            log: std::sync::Mutex::new(std::collections::VecDeque::new()),
            settings: std::sync::Mutex::new(st),
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

    // Clés PUBLIQUES seulement (vecteurs RFC 8032 §7.1) : rien de secret ici.
    const CLIENT_PUB: &str = "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c";
    const CLIENT_PUB2: &str = "fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025";

    fn corps(now: u64, keyid: &str, pubkey: &str) -> Vec<u8> {
        json!({ "keyid": keyid, "label": "portable", "public_key": pubkey, "created": now })
            .to_string()
            .into_bytes()
    }

    fn pair_id_of(body: &[u8]) -> String {
        let v: Value = serde_json::from_slice(body).unwrap();
        v["pair_id"].as_str().unwrap_or("").to_string()
    }

    #[test]
    fn code_de_comparaison_stable_et_sensible_aux_cles() {
        let a = comparison_code("00112233445566778899aabbccddeeff", CLIENT_PUB, "aa", "");
        assert_eq!(a.len(), 7, "six chiffres groupés : {a}");
        assert_eq!(a.chars().nth(3), Some(' '));
        assert!(a.replace(' ', "").chars().all(|c| c.is_ascii_digit()));
        assert_eq!(
            a,
            comparison_code("00112233445566778899aabbccddeeff", CLIENT_PUB, "aa", ""),
            "mêmes entrées ⇒ même code (les deux côtés doivent tomber d'accord)"
        );
        assert_ne!(
            a,
            comparison_code("00112233445566778899aabbccddeeff", CLIENT_PUB2, "aa", "")
        );
        assert_ne!(
            a,
            comparison_code("00112233445566778899aabbccddeeff", CLIENT_PUB, "ab", "")
        );
        assert_ne!(
            a,
            comparison_code(
                "00112233445566778899aabbccddeeff",
                CLIENT_PUB,
                "aa",
                "sha256:00"
            )
        );
    }

    #[test]
    fn post_valide_la_forme_et_refuse_avant_le_registre() {
        let st = state();
        let now = 1_800_000_000;
        let ip = Some("192.0.2.10".parse().unwrap());
        for mauvais in [
            json!({ "keyid": "pas-un-keyid", "public_key": CLIENT_PUB, "created": now }),
            json!({ "keyid": "mcc-a-0000000000000001", "public_key": "zz", "created": now }),
            json!({ "keyid": "mcc-a-0000000000000001", "public_key": CLIENT_PUB, "created": now - 61 }),
            json!({ "keyid": "mcc-a-0000000000000001", "public_key": CLIENT_PUB }),
        ] {
            let (s, _, _) = handle_post(&st, mauvais.to_string().as_bytes(), ip, now);
            assert_eq!(s, 400, "{mauvais}");
        }
        // horloge à +60 s : encore dans la fenêtre
        let (s, _, _) = handle_post(
            &st,
            &corps(now - 60, "mcc-a-0000000000000001", CLIENT_PUB),
            ip,
            now,
        );
        assert_eq!(s, 200);
        assert_eq!(lockv(&st.settings).auth.pairs.len(), 1);
        // la demande SEULE n'a créé aucune ligne au registre
        assert!(lockv(&st.settings).auth.clients.is_empty());
    }

    #[test]
    fn post_sans_identite_serveur_rend_503() {
        let st = state();
        lockv(&st.settings).auth.server = None;
        let now = 1_800_000_000;
        let (s, _, _) = handle_post(
            &st,
            &corps(now, "mcc-a-0000000000000001", CLIENT_PUB),
            Some("192.0.2.10".parse().unwrap()),
            now,
        );
        assert_eq!(s, 503);
    }

    #[test]
    fn post_keyid_deja_au_registre_refuse_409_et_idempotence_en_attente() {
        let st = state();
        let now = 1_800_000_000;
        let ip = Some("192.0.2.11".parse().unwrap());
        let (s, _, body) = handle_post(
            &st,
            &corps(now, "mcc-a-0000000000000001", CLIENT_PUB),
            ip,
            now,
        );
        assert_eq!(s, 200);
        let premier = pair_id_of(&body);
        // même keyid, même clé → même demande (pas de doublon)
        let (s, _, body) = handle_post(
            &st,
            &corps(now, "mcc-a-0000000000000001", CLIENT_PUB),
            ip,
            now,
        );
        assert_eq!(s, 200);
        assert_eq!(pair_id_of(&body), premier);
        assert_eq!(lockv(&st.settings).auth.pairs.len(), 1);
        // même keyid, AUTRE clé → conflit
        let (s, _, _) = handle_post(
            &st,
            &corps(now, "mcc-a-0000000000000001", CLIENT_PUB2),
            ip,
            now,
        );
        assert_eq!(s, 409);
        // approuvé puis nouvelle demande → 409 (déjà au registre)
        approve(&st, &premier, vec![RIGHT_LECTURE.into()], now).unwrap();
        let (s, _, _) = handle_post(
            &st,
            &corps(now, "mcc-a-0000000000000001", CLIENT_PUB),
            ip,
            now,
        );
        assert_eq!(s, 409);
    }

    #[test]
    fn rate_limite_5_par_minute_et_par_adresse() {
        let st = state();
        let now = 1_800_000_000;
        let ip = Some("192.0.2.12".parse().unwrap());
        let mut acceptees = 0;
        let mut refusees = 0;
        for i in 0..12 {
            let keyid = format!("mcc-a-{:016x}", i);
            let (s, _, _) = handle_post(&st, &corps(now, &keyid, CLIENT_PUB), ip, now);
            match s {
                200 => acceptees += 1,
                429 => refusees += 1,
                autre => panic!("statut inattendu {autre}"),
            }
        }
        assert_eq!(acceptees, PAIR_RATE_MAX);
        assert_eq!(refusees, 12 - PAIR_RATE_MAX);
        // une autre adresse a son propre quota
        let (s, _, _) = handle_post(
            &st,
            &corps(now, "mcc-a-00000000000000ff", CLIENT_PUB),
            Some("192.0.2.13".parse().unwrap()),
            now,
        );
        assert_eq!(s, 200);
        // la fenêtre glisse : une minute plus tard, le quota est reconstitué
        assert!(pair_rate_ok("192.0.2.12".parse().unwrap(), now + 61));
    }

    #[test]
    fn seize_demandes_en_attente_au_plus_la_plus_ancienne_evincee() {
        let mut pairs: Vec<PairRequest> = Vec::new();
        let now = 1_800_000_000;
        for i in 0..20u64 {
            pairs.push(PairRequest {
                pair_id: format!("{i:032x}"),
                keyid: format!("mcc-a-{i:016x}"),
                label: String::new(),
                public_key: CLIENT_PUB.to_string(),
                code: "000 000".to_string(),
                created: now,
                paired_from: "192.0.2.14".to_string(),
                decision: None,
                decided_at: None,
            });
            prune(&mut pairs, now);
        }
        assert_eq!(
            pairs.iter().filter(|p| p.decision.is_none()).count(),
            PAIR_MAX_PENDING
        );
        // les 4 plus anciennes sont parties
        assert!(pairs.iter().all(|p| p.pair_id != format!("{:032x}", 0u64)));
        assert!(pairs.iter().any(|p| p.pair_id == format!("{:032x}", 19u64)));
    }

    #[test]
    fn get_404_sur_pair_id_inconnu_ou_mal_forme() {
        let st = state();
        let now = 1_800_000_000;
        for mauvais in ["", "abc", &"z".repeat(32), &"0".repeat(33)] {
            let (s, _, _) = handle_get(&st, mauvais, now);
            assert_eq!(s, 404, "{mauvais}");
        }
        let (s, _, _) = handle_get(&st, &"0".repeat(32), now);
        assert_eq!(s, 404);
    }

    #[test]
    fn cycle_complet_pending_expire_approved_rejected() {
        let st = state();
        let now = 1_800_000_000;
        let ip = Some("192.0.2.15".parse().unwrap());
        let (status_post, _, body) = handle_post(
            &st,
            &corps(now, "mcc-a-0000000000000001", CLIENT_PUB),
            ip,
            now,
        );
        assert_eq!(
            status_post,
            200,
            "POST refusé : {}",
            String::from_utf8_lossy(&body)
        );
        let pair_id = pair_id_of(&body);
        assert_eq!(
            lockv(&st.settings).auth.pairs.len(),
            1,
            "pair_id={pair_id} body={}",
            String::from_utf8_lossy(&body)
        );
        let statut = |t: u64| {
            let (s, _, b) = handle_get(&st, &pair_id, t);
            assert_eq!(s, 200);
            let v: Value = serde_json::from_slice(&b).unwrap();
            v["status"].as_str().unwrap_or("").to_string()
        };
        assert_eq!(statut(now), "pending");
        assert_eq!(statut(now + 119), "pending");
        assert_eq!(statut(now + 120), "expired");
        // approuver une demande expirée échoue
        let err = approve(&st, &pair_id, vec![RIGHT_LECTURE.into()], now + 121).unwrap_err();
        assert!(err.contains("expirée"), "{err}");
        assert!(lockv(&st.settings).auth.clients.is_empty());

        // nouvelle demande, refusée proprement
        let (s, _, body) = handle_post(
            &st,
            &corps(now + 200, "mcc-a-0000000000000002", CLIENT_PUB),
            ip,
            now + 200,
        );
        assert_eq!(s, 200);
        let pair_id2 = pair_id_of(&body);
        reject(&st, &pair_id2, now + 200).unwrap();
        let (_, _, b) = handle_get(&st, &pair_id2, now + 200);
        let v: Value = serde_json::from_slice(&b).unwrap();
        assert_eq!(v["status"], "rejected");
        // une demande refusée ne peut plus être approuvée
        assert!(approve(&st, &pair_id2, vec![RIGHT_LECTURE.into()], now + 200).is_err());
        assert!(lockv(&st.settings).auth.clients.is_empty());
    }

    #[test]
    fn approve_refuse_admin_et_inscrit_les_droits_choisis() {
        let st = state();
        let now = 1_800_000_000;
        let ip = Some("192.0.2.16".parse().unwrap());
        let (_, _, body) = handle_post(
            &st,
            &corps(now, "mcc-a-0000000000000001", CLIENT_PUB),
            ip,
            now,
        );
        let pair_id = pair_id_of(&body);
        let err = approve(&st, &pair_id, vec!["admin".into()], now).unwrap_err();
        assert!(err.contains("admin"), "{err}");
        assert!(
            lockv(&st.settings).auth.clients.is_empty(),
            "refus atomique"
        );
        let ok = approve(
            &st,
            &pair_id,
            vec![RIGHT_LECTURE.into(), "pilotage".into()],
            now,
        )
        .unwrap();
        assert_eq!(ok["keyid"], "mcc-a-0000000000000001");
        let settings = lockv(&st.settings);
        let rec = settings
            .auth
            .find("mcc-a-0000000000000001")
            .expect("client inscrit");
        assert_eq!(
            rec.rights,
            vec!["lecture".to_string(), "pilotage".to_string()]
        );
        assert_eq!(rec.status, "active");
        assert_eq!(rec.paired_from, "192.0.2.16");
        assert_eq!(rec.label, "portable");
        // double approbation refusée
        drop(settings);
        assert!(approve(&st, &pair_id, vec![RIGHT_LECTURE.into()], now).is_err());
    }

    #[test]
    fn list_ne_montre_que_les_demandes_en_attente_avec_le_code() {
        let st = state();
        let now = 1_800_000_000;
        let ip = Some("192.0.2.17".parse().unwrap());
        handle_post(
            &st,
            &corps(now, "mcc-a-0000000000000001", CLIENT_PUB),
            ip,
            now,
        );
        handle_post(
            &st,
            &corps(now, "mcc-a-0000000000000002", CLIENT_PUB),
            ip,
            now,
        );
        let v = list(&st, now);
        assert_eq!(v["ok"], true);
        assert_eq!(v["pending"].as_array().unwrap().len(), 2);
        assert_eq!(v["server_keyid"], SERVER_KEYID);
        let code = v["pending"][0]["code"].as_str().unwrap();
        assert!(code.contains(' '), "{code}");
        // une fois approuvée, la demande sort de la liste des en attente
        let pair_id = v["pending"][0]["pair_id"].as_str().unwrap().to_string();
        approve(&st, &pair_id, vec![RIGHT_LECTURE.into()], now).unwrap();
        assert_eq!(list(&st, now)["pending"].as_array().unwrap().len(), 1);
    }
}
