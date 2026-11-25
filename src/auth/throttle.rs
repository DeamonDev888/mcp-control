//! Anti-force brute des échecs d'authentification `/mcp` (lot M5 point 3,
//! `docs/auth.md` §1.4).
//!
//! Deux compteurs indépendants, comme la spécification l'exige : **par
//! adresse** et **par `keyid`**. Un attaquant qui change d'adresse reste
//! freiné par sa clé ; un attaquant qui change de clé reste freiné par son
//! adresse.
//!
//! - 8 échecs dans la fenêtre de 60 s → délai croissant (25 ms, 50 ms, 100 ms,
//!   200 ms, 400 ms, puis 500 ms au plus), appliqué **avant** la vérification.
//! - 40 échecs → blocage temporaire de 30 s, doublé à chaque récidive
//!   (60 s, 120 s, 240 s, 300 s au plus).
//! - Un succès remet le compteur à zéro.
//!
//! **Le loopback n'est jamais freiné ni bloqué** : une adresse locale ne peut
//! pas être bloquée par des échecs venus du LAN (critère du lot), et la
//! fenêtre locale doit rester fluide pendant une tempête. Le déni de service
//! local reste hors de portée, comme l'écrit `docs/auth.md` §1.
//!
//! Le compteur et la décision sont purs ([`Table`]) : les tests fixent
//! l'instant, aucun test ne dépend d'une horloge ni d'un état partagé.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Fenêtre glissante des échecs pris en compte.
pub const FENETRE_SECS: u64 = 60;
/// Nombre d'échecs à partir duquel un délai est appliqué.
pub const DELAI_A_PARTIR_DE: u32 = 8;
/// Nombre d'échecs à partir duquel l'adresse (ou la clé) est bloquée.
pub const BLOCAGE_A_PARTIR_DE: u32 = 40;
/// Durée du premier blocage ; doublée à chaque récidive.
pub const BLOCAGE_BASE_SECS: u64 = 30;
/// Plafond d'un blocage.
pub const BLOCAGE_MAX_SECS: u64 = 300;
/// Entrées suivies au plus par table (une clé inventée ne gonfle pas la mémoire).
const MAX_ENTREES: usize = 1024;

/// Ce que le transport doit faire de la requête, avant toute cryptographie.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Passer : aucune pénalité en cours.
    Allow,
    /// Attendre ce délai, puis vérifier.
    Delay(Duration),
    /// Refuser sans vérifier, avec `Retry-After` en secondes.
    Block(u64),
}

#[derive(Clone, Debug, Default)]
struct Compteur {
    /// Instants (secondes UNIX) des échecs encore dans la fenêtre.
    echecs: Vec<u64>,
    /// Nombre de blocages déjà infligés (escalade des durées).
    blocages: u32,
    /// Fin du blocage en cours (0 = aucun).
    bloque_jusqua: u64,
    /// Dernière activité — sert à l'éviction quand la table est pleine.
    dernier: u64,
}

impl Compteur {
    fn elaguer(&mut self, now: u64) {
        self.echecs
            .retain(|t| now.saturating_sub(*t) <= FENETRE_SECS);
    }

    fn echec(&mut self, now: u64) {
        self.elaguer(now);
        self.dernier = now;
        self.echecs.push(now);
        // Au-delà du seuil de blocage, aucun détail supplémentaire n'est utile :
        // la mémoire d'un compteur reste bornée même sous un déluge synthétique.
        let exces = self
            .echecs
            .len()
            .saturating_sub(BLOCAGE_A_PARTIR_DE as usize);
        if exces > 0 {
            self.echecs.drain(..exces);
        }
        if self.echecs.len() as u32 >= BLOCAGE_A_PARTIR_DE {
            self.blocages = self.blocages.saturating_add(1);
            self.bloque_jusqua = now.saturating_add(duree_blocage(self.blocages));
        }
    }

    fn succes(&mut self, now: u64) {
        self.echecs.clear();
        self.blocages = 0;
        self.bloque_jusqua = 0;
        self.dernier = now;
    }

    fn verdict(&mut self, now: u64) -> Verdict {
        self.elaguer(now);
        if now < self.bloque_jusqua {
            return Verdict::Block(self.bloque_jusqua - now);
        }
        let n = self.echecs.len() as u32;
        if n >= DELAI_A_PARTIR_DE {
            Verdict::Delay(Duration::from_millis(delai_ms(n)))
        } else {
            Verdict::Allow
        }
    }
}

/// Délai croissant : 25 ms au 8ᵉ échec, doublé à chaque fois, 500 ms au plus.
fn delai_ms(echecs: u32) -> u64 {
    let palier = echecs.saturating_sub(DELAI_A_PARTIR_DE).min(5);
    (25u64 << palier).min(500)
}

/// Durée du n-ième blocage : 30 s, 60 s, 120 s, 240 s, 300 s au plus.
fn duree_blocage(blocages: u32) -> u64 {
    let palier = blocages.saturating_sub(1).min(4);
    (BLOCAGE_BASE_SECS << palier).min(BLOCAGE_MAX_SECS)
}

/// Compteurs purs : aucune horloge, aucun état global — testable à l'instant
/// près.
#[derive(Default)]
pub struct Table {
    ips: HashMap<IpAddr, Compteur>,
    cles: HashMap<String, Compteur>,
}

impl Table {
    pub fn check(&mut self, peer: Option<IpAddr>, keyid: Option<&str>, now: u64) -> Verdict {
        if exempte(peer) {
            return Verdict::Allow;
        }
        let par_ip = peer
            .and_then(|ip| self.ips.get_mut(&ip))
            .map(|c| c.verdict(now))
            .unwrap_or(Verdict::Allow);
        let par_cle = keyid
            .and_then(|k| self.cles.get_mut(k))
            .map(|c| c.verdict(now))
            .unwrap_or(Verdict::Allow);
        // Le blocage l'emporte (le plus long), puis le délai le plus fort.
        match (par_ip, par_cle) {
            (Verdict::Block(a), Verdict::Block(b)) => Verdict::Block(a.max(b)),
            (Verdict::Block(a), _) | (_, Verdict::Block(a)) => Verdict::Block(a),
            (Verdict::Delay(a), Verdict::Delay(b)) => Verdict::Delay(a.max(b)),
            (Verdict::Delay(a), _) | (_, Verdict::Delay(a)) => Verdict::Delay(a),
            _ => Verdict::Allow,
        }
    }

    pub fn record(&mut self, peer: Option<IpAddr>, keyid: Option<&str>, echec: bool, now: u64) {
        if exempte(peer) {
            return;
        }
        if let Some(k) = keyid {
            let c = self.cles.entry(k.to_string()).or_default();
            if echec {
                c.echec(now);
            } else {
                c.succes(now);
            }
        }
        if let Some(ip) = peer {
            let c = self.ips.entry(ip).or_default();
            if echec {
                c.echec(now);
            } else {
                c.succes(now);
            }
        }
        borner(&mut self.cles);
        borner(&mut self.ips);
    }
}

/// Une adresse locale n'est jamais freinée : la fenêtre et le pont sont locaux,
/// et le LAN ne doit jamais pouvoir les bloquer. Une adresse inconnue (`None`)
/// reste soumise au compteur par `keyid` (échec fermé).
fn is_loopback_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
    }
}

fn exempte(peer: Option<IpAddr>) -> bool {
    peer.is_some_and(is_loopback_ip)
}

/// Éviction de la plus ancienne entrée quand la table dépasse sa borne : une
/// clé inventée ne fait pas grossir la mémoire indéfiniment.
fn borner<V: Clone + Eq + std::hash::Hash>(map: &mut HashMap<V, Compteur>) {
    while map.len() > MAX_ENTREES {
        // Ne jamais évincer une entrée activement bloquée avant une entrée non bloquée [M-04].
        let Some(plus_vieux) = map
            .iter()
            .min_by_key(|(_, c)| (c.bloque_jusqua > 0, c.dernier))
            .map(|(k, _)| k.clone())
        else {
            break;
        };
        map.remove(&plus_vieux);
    }
}

static TABLE: OnceLock<Mutex<Table>> = OnceLock::new();

fn table() -> &'static Mutex<Table> {
    TABLE.get_or_init(|| Mutex::new(Table::default()))
}

/// Décision pour une requête `/mcp` entrante.
pub fn check(peer: Option<IpAddr>, keyid: Option<&str>) -> Verdict {
    table()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .check(peer, keyid, crate::types::now_ts())
}

/// Enregistre l'issue d'une requête qui a atteint le vérificateur.
/// `echec` = refus d'authentification (401) ; tout le reste remet à zéro.
pub fn record(peer: Option<IpAddr>, keyid: Option<&str>, echec: bool) {
    table().lock().unwrap_or_else(|p| p.into_inner()).record(
        peer,
        keyid,
        echec,
        crate::types::now_ts(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().expect("adresse de test")
    }

    /// Documentation (RFC 5737) : jamais une adresse réelle.
    const LAN: &str = "192.0.2.7";
    const LAN2: &str = "192.0.2.8";

    #[test]
    fn echecs_lan_delai_croissant_puis_blocage() {
        let mut t = Table::default();
        let now = 1_000_000;
        for i in 0..7 {
            assert_eq!(
                t.check(Some(ip(LAN)), None, now),
                Verdict::Allow,
                "échec {i} : aucun délai attendu avant le 8ᵉ"
            );
            t.record(Some(ip(LAN)), None, true, now);
        }
        t.record(Some(ip(LAN)), None, true, now); // 8ᵉ
        assert_eq!(
            t.check(Some(ip(LAN)), None, now),
            Verdict::Delay(Duration::from_millis(25))
        );
        for _ in 8..40 {
            t.record(Some(ip(LAN)), None, true, now);
        }
        match t.check(Some(ip(LAN)), None, now) {
            Verdict::Block(s) => assert_eq!(s, BLOCAGE_BASE_SECS),
            autre => panic!("blocage attendu au 40ᵉ échec, vu {autre:?}"),
        }
        // Le blocage expire : le délai reste tant que les 40 échecs sont dans
        // la fenêtre de 60 s, puis tout s'oublie.
        assert!(!matches!(
            t.check(Some(ip(LAN)), None, now + BLOCAGE_BASE_SECS + 1),
            Verdict::Block(_)
        ));
        assert_eq!(
            t.check(Some(ip(LAN)), None, now + FENETRE_SECS + 1),
            Verdict::Allow
        );
    }

    #[test]
    fn adresse_locale_jamais_bloquee_par_le_lan() {
        let mut t = Table::default();
        let now = 1_000_000;
        // 60 échecs depuis le LAN, sur la clé du pont local.
        for _ in 0..60 {
            t.record(Some(ip(LAN)), Some("mcc-ai-3f9a1c07b2d84e51"), true, now);
        }
        assert!(matches!(
            t.check(Some(ip(LAN)), Some("mcc-ai-3f9a1c07b2d84e51"), now),
            Verdict::Block(_)
        ));
        // La même clé, depuis le loopback, n'est ni bloquée ni ralentie.
        assert_eq!(
            t.check(Some(ip("127.0.0.1")), Some("mcc-ai-3f9a1c07b2d84e51"), now),
            Verdict::Allow
        );
        // Et les échecs locaux n'enregistrent rien : le loopback reste exempté.
        for _ in 0..100 {
            t.record(
                Some(ip("127.0.0.1")),
                Some("mcc-ai-3f9a1c07b2d84e51"),
                true,
                now,
            );
        }
        assert_eq!(
            t.check(Some(ip("127.0.0.1")), Some("mcc-ai-3f9a1c07b2d84e51"), now),
            Verdict::Allow
        );
    }

    #[test]
    fn cle_saturee_bloque_meme_depuis_une_autre_adresse() {
        let mut t = Table::default();
        let now = 1_000_000;
        let cle = Some("mcc-ai-3f9a1c07b2d84e51");
        // 25 échecs depuis deux adresses : 50 au total, 25 par adresse — sous
        // le seuil d'adresse, au-dessus du seuil de clé.
        for _ in 0..25 {
            t.record(Some(ip(LAN)), cle, true, now);
            t.record(Some(ip(LAN2)), cle, true, now);
        }
        assert!(
            matches!(t.check(Some(ip(LAN2)), cle, now), Verdict::Block(_)),
            "la clé saturée doit bloquer même depuis une adresse peu active"
        );
        // Une AUTRE clé depuis cette adresse subit le délai de l'adresse
        // (50 échecs) mais pas le blocage de la clé saturée.
        match t.check(Some(ip(LAN2)), Some("mcc-ai-0000000000000009"), now) {
            Verdict::Delay(_) | Verdict::Block(_) => {}
            Verdict::Allow => panic!("l'adresse à 50 échecs doit être freinée"),
        }
    }

    #[test]
    fn un_succes_remet_les_compteurs_a_zero() {
        let mut t = Table::default();
        let now = 1_000_000;
        for _ in 0..30 {
            t.record(Some(ip(LAN)), Some("mcc-ai-3f9a1c07b2d84e51"), true, now);
        }
        t.record(Some(ip(LAN)), Some("mcc-ai-3f9a1c07b2d84e51"), false, now);
        assert_eq!(
            t.check(Some(ip(LAN)), Some("mcc-ai-3f9a1c07b2d84e51"), now),
            Verdict::Allow
        );
    }

    #[test]
    fn fenetre_glissante_oublie_les_vieux_echecs() {
        let mut t = Table::default();
        let now = 1_000_000;
        for _ in 0..30 {
            t.record(Some(ip(LAN)), None, true, now);
        }
        assert!(matches!(
            t.check(Some(ip(LAN)), None, now),
            Verdict::Delay(_) | Verdict::Block(_)
        ));
        // 61 s plus tard, les 30 échecs sont hors fenêtre.
        assert_eq!(
            t.check(Some(ip(LAN)), None, now + FENETRE_SECS + 1),
            Verdict::Allow
        );
    }

    #[test]
    fn la_table_est_bornee() {
        let mut t = Table::default();
        let now = 1_000_000;
        for i in 0..(MAX_ENTREES + 200) {
            let keyid = format!("mcc-ai-{:016x}", i as u64);
            t.record(Some(ip(LAN)), Some(&keyid), true, now);
        }
        assert!(t.cles.len() <= MAX_ENTREES);
        // l'adresse, elle, est toujours suivie
        assert!(matches!(
            t.check(Some(ip(LAN)), None, now),
            Verdict::Block(_)
        ));
    }

    #[test]
    fn adresse_inconnue_n_est_pas_exemptee_sur_keyid() {
        let mut t = Table::default();
        let now = 1_000_000;
        let cle = Some("mcc-ai-3f9a1c07b2d84e51");
        for _ in 0..BLOCAGE_A_PARTIR_DE {
            t.record(None, cle, true, now);
        }
        assert!(matches!(t.check(None, cle, now), Verdict::Block(_)));
    }
}
