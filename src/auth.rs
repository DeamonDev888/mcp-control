//! Authentification forte — **identités, registre des clients, appairage**.
//!
//! Lot M1 de `TODO-AUTH.md`, spécifié par `docs/auth.md` §3 et §9. Ce module ne
//! change le comportement d'AUCUNE route existante : l'authentification n'est
//! pas encore exigée (c'est le lot M2 qui fermera le sas). Il apporte :
//!
//! - l'identité Ed25519 du serveur (`mcc-server`), clé privée au trousseau du
//!   système et **jamais** dans `config.json` (§9) ;
//! - le registre des clients (`auth.clients[]`, 10 champs du §3.2) et son
//!   implémentation de [`litemcp::auth::sign::KeyStore`] ;
//! - le protocole d'appairage en 5 messages (§3.4), non authentifié mais borné
//!   par adresse, 16 demandes en attente, expiration 2 minutes ;
//! - les actions locales `admin` (jamais exposées au gateway) et la ligne de
//!   commande `mcp-control auth ...`.
//!
//! Règles de la maison, appliquées ici : aucun secret dans un journal, une
//! réponse HTTP ou `config.json` ; comparaisons de secrets en temps constant
//! ([`crate::crypto::ct_eq`]) ; aléa = celui du système (`lib::random_hex`) ;
//! aucune primitive cryptographique écrite à la main (Ed25519 vient de la lib).

pub mod cli;
pub mod confirm;
pub mod pair;
pub mod registry;
pub mod secret;
pub mod session;

#[cfg(test)]
mod tests;

pub use registry::{
    is_grantable, valid_keyid, AuthState, ClientRecord, RegistryStore, ServerIdentity, ALL_RIGHTS,
    RIGHT_ADMIN, RIGHT_EXECUTION, RIGHT_LECTURE, RIGHT_PILOTAGE, SERVER_KEYID,
};

/// Actions locales réservées au droit `admin` (docs/auth.md §4) : elles
/// n'existent QUE sur l'API locale du moteur — jamais sur `/mcp`, jamais
/// depuis le LAN. Leur présence ici est la liste de référence du dispatcher, du
/// test d'exclusion du gateway et du contrôle d'origine de `route_peer`.
///
/// Lot M3 : `access_list` (état de l'écran « Accès »), `confirm_approve`,
/// `confirm_deny` et `grant_revoke` (décisions de confirmation, §5) rejoignent
/// les sept actions d'appairage et de registre du lot M1.
pub const ADMIN_ACTIONS: [&str; 11] = [
    "pair_list",
    "pair_approve",
    "pair_reject",
    "client_list",
    "client_set_scopes",
    "client_revoke",
    "client_rotate",
    "access_list",
    "confirm_approve",
    "confirm_deny",
    "grant_revoke",
];

/// Cette action est-elle réservée à la session locale (`admin`) ?
pub fn action_reservee_admin(nom: &str) -> bool {
    ADMIN_ACTIONS.contains(&nom)
}
