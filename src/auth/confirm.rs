//! Confirmation humaine avant exécution — politique de confirmation (contrat
//! C5) et état de l'écran « Accès », spécifié par `docs/auth.md` §5.
//!
//! Ce que ce module garantit :
//!
//! - **Tout** appel d'un outil `execution` crée une demande en attente (client,
//!   libellé, empreinte courte, adresse source, outil, effet exact) et rend
//!   `Pending` : la lib n'exécute rien et ne met rien en file, c'est le client
//!   qui réessaie (§5).
//! - L'approbation n'existe QUE par la session locale (`admin`) : la politique
//!   n'a aucun chemin d'approbation, et [`approve`] refuse tout autre acteur.
//!   Un client ne voit que ses propres demandes ([`list`] avec
//!   [`Actor::Client`]).
//! - Un permis vaut pour **ce client**, **cet outil** et **ces arguments**
//!   (empreinte SHA-256 des arguments réellement exécutés, contrat C5) pendant
//!   dix minutes (`MCC_CONFIRM_SECS` pour les tests). Il se révoque depuis le
//!   même écran et disparaît au redémarrage : tout vit en mémoire, rien n'est
//!   persisté.
//! - Demandes bornées (32) et expirées (2 min) ; les valeurs de variables
//!   d'environnement et les secrets sont remplacés par les **noms** dans
//!   l'effet affiché (`docs/auth.md` §5).
//!
//! Règles de la maison : aucune primitive cryptographique écrite ici (SHA-256
//! vient de [`crate::crypto`]) ; aucun secret dans un journal, une réponse ou
//! l'écran ; comparaisons de secrets en temps constant quand il y en a.

use litemcp::auth::{
    ApprovalPolicy, ApprovalRequest, ApprovalVerdict, AuthDecisionEvent, AuthVerdict,
};
use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::types::{lockv, now_ts, AppState};

/// Demandes réellement **en attente** au plus (les permis déjà accordés ne
/// comptent pas : ce ne sont plus des demandes). Au-delà, la plus ancienne est
/// évincée — l'écran et le journal le disent.
pub const MAX_PENDING: usize = 32;

/// Durée de vie d'une demande non décidée (§5).
pub const REQUEST_TTL_SECS: u64 = 120;

/// Durée d'un permis accordé (§5 : dix minutes).
pub const DEFAULT_WINDOW_SECS: u64 = 600;

/// Nombre de décisions conservées pour l'écran « Accès » (C4 : les 100
/// dernières).
pub const DECISIONS_KEPT: usize = 100;

/// Durée du permis, en secondes. `MCC_CONFIRM_SECS` n'existe que pour les
/// tests (une fenêtre de dix minutes ne se joue pas dans une CI) ; la valeur
/// est bornée à [1, 86 400] et vaut [`DEFAULT_WINDOW_SECS`] par défaut.
pub fn window_secs() -> u64 {
    std::env::var("MCC_CONFIRM_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| (1..=86_400).contains(v))
        .unwrap_or(DEFAULT_WINDOW_SECS)
}

// ---------------------------------------------------------------------------
// Adresse source de la requête en cours
// ---------------------------------------------------------------------------

thread_local! {
    /// Adresse du pair de la requête `/mcp` traitée sur CE fil. La politique
    /// C5 ne reçoit que `(client, outil, arguments, request_id)` : l'adresse,
    /// qui doit figurer sur l'écran (§5), vient du transport par ce canal.
    static REQUEST_PEER: std::cell::Cell<Option<IpAddr>> = const { std::cell::Cell::new(None) };
}

/// Posée par [`crate::gateway::serve`] au début du traitement d'une requête.
pub fn set_request_peer(peer: Option<IpAddr>) {
    REQUEST_PEER.with(|c| c.set(peer));
}

/// Adresse source affichée — `-` quand le transport ne la connaît pas (tests
/// unitaires, dispatch direct).
pub fn request_peer() -> String {
    REQUEST_PEER
        .with(|c| c.get())
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "-".to_string())
}

// ---------------------------------------------------------------------------
// Acteur : qui demande, qui décide
// ---------------------------------------------------------------------------

/// Qui agit. La session locale (`admin`) est la SEULE à pouvoir approuver,
/// refuser ou révoquer (§5) ; un client du gateway ne voit que ses demandes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Actor<'a> {
    /// Session locale de la fenêtre — droit `admin` (`docs/auth.md` §6).
    LocalAdmin,
    /// Client identifié sur `/mcp`, par son `keyid`.
    Client(&'a str),
}

impl Actor<'_> {
    fn keyid(&self) -> Option<&str> {
        match self {
            Actor::LocalAdmin => None,
            Actor::Client(k) => Some(k),
        }
    }
}

// ---------------------------------------------------------------------------
// Effet exact d'un appel (ce que l'écran montre)
// ---------------------------------------------------------------------------

/// Clés dont la VALEUR ne sort jamais : ni à l'écran, ni au journal, ni dans
/// la notification. Les noms, eux, sont affichés (§5).
const CLES_SECRETES: [&str; 10] = [
    "token",
    "secret",
    "password",
    "passwd",
    "apikey",
    "api_key",
    "authorization",
    "credential",
    "cookie",
    "private_key",
];

fn cle_sensible(nom: &str) -> bool {
    let n = nom.to_ascii_lowercase();
    CLES_SECRETES.iter().any(|c| n.contains(c))
}

/// Tronque un texte pour l'affichage (jamais un argument arbitrairement gros
/// venu du réseau).
fn court(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// Arguments d'un outil distant, prêts à afficher : les clés sensibles sont
/// masquées, les textes tronqués, le nombre d'entrées borné.
pub fn masked_arguments(v: &Value) -> Value {
    fn walk(v: &Value, depth: usize) -> Value {
        if depth > 3 {
            return json!("…");
        }
        match v {
            Value::Object(o) => {
                let mut out = Map::new();
                for (i, (k, val)) in o.iter().enumerate() {
                    if i >= 24 {
                        out.insert("…".into(), json!("(entrées suivantes omises)"));
                        break;
                    }
                    if cle_sensible(k) {
                        out.insert(k.clone(), json!("<masqué>"));
                    } else {
                        out.insert(k.clone(), walk(val, depth + 1));
                    }
                }
                Value::Object(out)
            }
            Value::Array(a) => {
                let mut out: Vec<Value> = a.iter().take(12).map(|x| walk(x, depth + 1)).collect();
                if a.len() > 12 {
                    out.push(json!("…"));
                }
                Value::Array(out)
            }
            Value::String(s) => json!(court(s, 160)),
            other => other.clone(),
        }
    }
    walk(v, 0)
}

/// Extrait les programmes d'une configuration MCP importée (bloc
/// `mcpServers` / `servers` / objet direct). Le TEXTE importé n'est jamais
/// recopié : ni à l'écran, ni au journal — il peut porter des jetons.
fn programmes_importes(texte: &str) -> Vec<Value> {
    let Ok(v) = serde_json::from_str::<Value>(texte) else {
        return vec![json!({"label": "(texte illisible)", "program": "?", "args": []})];
    };
    let bloc = v
        .get("mcpServers")
        .or_else(|| v.get("servers"))
        .or_else(|| v.get("mcp"))
        .unwrap_or(&v);
    let mut out = Vec::new();
    if let Some(o) = bloc.as_object() {
        for (nom, entree) in o.iter().take(10) {
            match entree.get("command").and_then(Value::as_str) {
                Some(cmd) => {
                    let args: Vec<String> = entree
                        .get("args")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(Value::as_str)
                                .take(8)
                                .map(|s| court(s, 80))
                                .collect()
                        })
                        .unwrap_or_default();
                    out.push(json!({
                        "label": court(nom, 64),
                        "program": court(cmd, 120),
                        "args": args,
                        // noms des env du bloc importé — jamais les valeurs
                        "env": entree.get("env").and_then(Value::as_object)
                            .map(|e| e.keys().take(12).cloned().collect::<Vec<_>>())
                            .unwrap_or_default(),
                    }));
                }
                None => {
                    let url = entree
                        .get("url")
                        .or_else(|| entree.get("serverUrl"))
                        .and_then(Value::as_str)
                        .unwrap_or("?");
                    out.push(json!({
                        "label": court(nom, 64),
                        "program": court(url, 120),
                        "args": [],
                    }));
                }
            }
        }
    }
    if out.is_empty() {
        out.push(json!({"label": "(aucun programme reconnu)", "program": "?", "args": []}));
    }
    out
}

/// Commande de lancement connue d'un serveur supervisé (par son id) — c'est
/// elle qui s'exécute quand `start_server` / `toggle_server` /
/// `set_auto_restart` sont approuvés.
fn commande_du_serveur(state: &Arc<AppState>, id: &str) -> Option<Value> {
    let servers = lockv(&state.servers);
    let s = servers.iter().find(|s| s.id == id)?;
    let cmd = crate::manage::start_cmd_for(s)?;
    Some(json!({
        "label": s.label,
        "program": cmd.program,
        "args": cmd.args,
        "workdir": cmd.workdir,
        // NOMS seulement : les valeurs d'env ne sortent jamais (§5).
        "env": crate::types::env_names(&cmd.env),
    }))
}

/// L'effet **exact** d'un appel, tel que l'utilisateur doit le lire avant
/// d'approuver (§5). Construit à partir des arguments que la politique reçoit,
/// c'est-à-dire exactement ceux qui seront exécutés (contrat C5).
pub fn effect_of(state: &Arc<AppState>, tool: &str, args: &Value) -> Value {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("");
    let arr = |k: &str| -> Vec<String> {
        args.get(k)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .take(24)
                    .map(|x| court(x, 160))
                    .collect()
            })
            .unwrap_or_default()
    };
    let lancement = |label: &str, program: &str, args: Vec<String>, workdir: &str| -> Value {
        let resume = format!(
            "{program} {}",
            if args.is_empty() {
                String::new()
            } else {
                args.join(" ")
            }
        );
        json!({
            "kind": "lancement",
            "label": label,
            "resume": court(&resume, 300),
            "program": court(program, 160),
            "args": args,
            "workdir": workdir,
            "env": [],
        })
    };
    match tool {
        // ---- lancements : programme + arguments + dossier de travail ----
        "start_server" | "toggle_server" | "set_auto_restart" => {
            let id = s("id");
            match commande_du_serveur(state, id) {
                Some(c) => json!({
                    "kind": "lancement",
                    "label": format!("serveur {}", id),
                    "resume": format!(
                        "{} {}",
                        c["program"].as_str().unwrap_or("?"),
                        c["args"].as_array().map(|a| a.iter().filter_map(Value::as_str)
                            .collect::<Vec<_>>().join(" ")).unwrap_or_default()
                    ),
                    "program": c["program"],
                    "args": c["args"],
                    "workdir": c["workdir"],
                    "env": c["env"],
                    "note": if tool == "start_server" { "démarrage du programme" }
                            else if tool == "toggle_server" { "interrupteur ON : le programme est relancé" }
                            else { "relance automatique au boot et après un crash" },
                }),
                None => lancement(
                    &format!("serveur {id}"),
                    "(aucune commande de lancement connue)",
                    vec![],
                    "",
                ),
            }
        }
        "add_server" => match s("transport") {
            "stdio" => lancement(
                "nouveau serveur stdio",
                s("program"),
                arr("args"),
                s("workdir"),
            ),
            _ => json!({
                "kind": "ajout",
                "label": court(s("label"), 64),
                "resume": format!("HTTP {}:{}{}", s("host"), s("port"), s("path")),
                "target": format!("{}:{}", s("host"), s("port")),
            }),
        },
        "set_start_command" => {
            if args.get("program").map(Value::is_null).unwrap_or(true) {
                json!({
                    "kind": "commande",
                    "label": format!("serveur {}", s("id")),
                    "resume": "retrait de la commande de lancement".to_string(),
                })
            } else {
                let mut v = lancement(
                    &format!("serveur {}", s("id")),
                    s("program"),
                    arr("args"),
                    s("workdir"),
                );
                v["note"] = json!("cette commande sera exécutée par start_server et à l'autoboot");
                v
            }
        }
        "install_store_server" => {
            let env: Vec<String> = args
                .get("env")
                .and_then(Value::as_object)
                .map(|o| o.keys().take(12).cloned().collect())
                .unwrap_or_default();
            json!({
                "kind": "lancement",
                "label": format!("boutique {}", s("id")),
                "resume": format!("installation depuis la boutique : {}", court(s("id"), 80)),
                "args": arr("args"),
                // NOMS des variables fournies, jamais leurs valeurs (§5)
                "env": env,
            })
        }
        "import_config" => {
            let progs = programmes_importes(s("text"));
            json!({
                "kind": "import",
                "resume": format!("import de {} serveur(s) depuis une configuration MCP", progs.len()),
                "servers": progs,
            })
        }
        "call_server_tool" => json!({
            "kind": "outil",
            "target": court(s("server"), 96),
            "tool": court(s("tool"), 96),
            "resume": format!("appel de l'outil {} sur le serveur {}", court(s("tool"), 96), court(s("server"), 96)),
            // arguments de l'outil distant : clés sensibles masquées
            "arguments": masked_arguments(args.get("arguments").unwrap_or(&Value::Null)),
            "note": "un outil distant peut exécuter n'importe quoi",
        }),
        "ssh_save" | "ssh_discover" => json!({
            "kind": "ssh",
            "target": format!("{}@{}", s("user"), s("ip")),
            "resume": format!("SSH {}@{}", s("user"), s("ip")),
            "note": if tool == "ssh_save" { "enregistre la cible que ssh_discover contactera" }
                    else { "exécute ssh vers la machine" },
        }),
        _ => json!({
            "kind": "autre",
            "resume": format!("appel de {tool}"),
        }),
    }
}

// ---------------------------------------------------------------------------
// Machine à états (pure, temps injecté)
// ---------------------------------------------------------------------------

/// Verdict de la politique, réduit à ce que la lib attend.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    Allow,
    Pending,
    Deny,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Etat {
    /// Demande créée, personne n'a tranché.
    Attente,
    /// Permis « une fois » : consommé par le prochain appel identique.
    UneFois,
    /// Permis « dix minutes » : tout appel identique passe jusqu'à l'échéance.
    Fenetre,
    /// Refusée localement : le client reçoit `approval_denied` jusqu'à
    /// l'expiration de la demande (pas de nouvelle demande à chaque essai).
    Refusee,
}

struct Entree {
    id: String,
    keyid: String,
    label: String,
    empreinte: String,
    adresse: String,
    outil: String,
    empreinte_args: String,
    effet: Value,
    request_id: String,
    cree: u64,
    echeance: u64,
    etat: Etat,
}

/// Identité de l'appelant, résolue AVANT de prendre le verrou de la machine à
/// états (l'ordre des verrous reste : réglages → serveurs → confirmations).
struct Qui {
    keyid: String,
    label: String,
    empreinte: String,
    adresse: String,
}

/// Ce que la politique a décidé, plus les lignes à porter au journal.
pub struct Decision {
    pub verdict: Verdict,
    pub lignes: Vec<String>,
    /// Identifiant de la demande CRÉÉE par cet appel (aucun si l'appel a
    /// trouvé une demande existante ou un permis) : sert à notifier une seule
    /// fois par demande.
    pub created: Option<String>,
}

#[derive(Default)]
struct Core {
    entrees: Vec<Entree>,
    decisions: VecDeque<Value>,
    /// Compteur monotone de révision (incrémenté sur création, décision,
    /// consommation, révocation ou expiration).
    rev_seq: u64,
    ttl: u64,
    fenetre: u64,
}

impl Core {
    fn new(ttl: u64, fenetre: u64) -> Self {
        Core {
            entrees: Vec::new(),
            decisions: VecDeque::new(),
            rev_seq: 0,
            ttl,
            fenetre,
        }
    }

    fn note(&mut self, ts: u64, keyid: &str, outil: &str, verdict: &str, motif: &str) {
        self.decisions.push_back(json!({
            "ts": ts, "keyid": keyid, "tool": outil,
            "verdict": verdict, "reason": motif,
        }));
        while self.decisions.len() > DECISIONS_KEPT {
            self.decisions.pop_front();
        }
        self.rev_seq = self.rev_seq.wrapping_add(1);
    }

    fn en_attente(&self) -> usize {
        self.entrees
            .iter()
            .filter(|e| e.etat == Etat::Attente)
            .count()
    }

    /// Retire les entrées arrivées à échéance et rend les lignes de journal
    /// correspondantes (demande expirée, permis expiré).
    fn balayer(&mut self, now: u64) -> Vec<String> {
        let mut lignes = Vec::new();
        let mut gardees: Vec<Entree> = Vec::with_capacity(self.entrees.len());
        for e in std::mem::take(&mut self.entrees) {
            if now < e.echeance {
                gardees.push(e);
                continue;
            }
            self.rev_seq = self.rev_seq.wrapping_add(1);
            match e.etat {
                Etat::Attente => {
                    lignes.push(format!(
                        "[auth] confirmation expirée sans décision : {} {} (demande {})",
                        e.keyid, e.outil, e.id
                    ));
                    self.note(now, &e.keyid, &e.outil, "expirée", "approval_expired");
                }
                Etat::UneFois | Etat::Fenetre => {
                    lignes.push(format!(
                        "[auth] permis de confirmation expiré : {} {} (redemande au prochain appel)",
                        e.keyid, e.outil
                    ));
                    self.note(now, &e.keyid, &e.outil, "permis expiré", "grant_expired");
                }
                // Refusée : l'entrée meurt à son échéance, sans nouvelle ligne
                // (le refus a déjà été journalisé au moment de la décision).
                Etat::Refusee => {}
            }
        }
        self.entrees = gardees;
        lignes
    }

    #[allow(clippy::too_many_arguments)] // miroir de `decider` : tout est utile
    fn creer(
        &mut self,
        now: u64,
        qui: &Qui,
        outil: &str,
        digest: &str,
        effet: &Value,
        request_id: &str,
    ) -> String {
        // Borne : la plus ancienne demande EN ATTENTE cède la place.
        if self.en_attente() >= MAX_PENDING {
            if let Some(pos) = self.entrees.iter().position(|e| e.etat == Etat::Attente) {
                let vieille = self.entrees.remove(pos);
                self.note(
                    now,
                    &vieille.keyid,
                    &vieille.outil,
                    "évincée",
                    "approval_evicted",
                );
            }
        }
        // Identifiant opaque de 32 hexadécimaux : client + outil + arguments +
        // horodatage fin + compteur — deux demandes ne peuvent pas le partager.
        static COMPTEUR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COMPTEUR.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let fin = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let id = crate::crypto::hex(&crate::crypto::sha256(
            format!("{now}:{fin}:{n}:{}:{outil}:{digest}", qui.keyid).as_bytes(),
        ));
        let id: String = id.chars().take(32).collect();
        self.entrees.push(Entree {
            id: id.clone(),
            keyid: qui.keyid.clone(),
            label: qui.label.clone(),
            empreinte: qui.empreinte.clone(),
            adresse: qui.adresse.clone(),
            outil: outil.to_string(),
            empreinte_args: digest.to_string(),
            effet: effet.clone(),
            request_id: request_id.to_string(),
            cree: now,
            echeance: now.saturating_add(self.ttl),
            etat: Etat::Attente,
        });
        self.rev_seq = self.rev_seq.wrapping_add(1);
        id
    }

    /// Décide du sort d'un appel : permis valide → `Allow` ; demande en attente
    /// ou créée → `Pending` ; refus local → `Deny`.
    fn decider(
        &mut self,
        now: u64,
        qui: &Qui,
        outil: &str,
        digest: &str,
        effet: &Value,
        request_id: &str,
    ) -> Decision {
        let mut lignes = self.balayer(now);
        let mut verdict = Verdict::Pending;
        let mut created = None;
        match self
            .entrees
            .iter()
            .position(|e| e.keyid == qui.keyid && e.outil == outil && e.empreinte_args == digest)
        {
            Some(pos) => match self.entrees[pos].etat {
                Etat::Attente => {
                    // demande déjà visible : on ne la duplique pas, on ne
                    // re-notifie pas (§5 : une seule notification par demande)
                }
                Etat::UneFois => {
                    self.entrees.remove(pos);
                    verdict = Verdict::Allow;
                    lignes.push(format!(
                        "[auth] confirmation utilisée (une fois) : {} {}",
                        qui.keyid, outil
                    ));
                    self.note(now, &qui.keyid, outil, "accordé (une fois)", "-");
                }
                Etat::Fenetre => {
                    verdict = Verdict::Allow;
                    self.note(now, &qui.keyid, outil, "accordé (permis)", "-");
                }
                Etat::Refusee => {
                    verdict = Verdict::Deny;
                    self.note(now, &qui.keyid, outil, "refusé (local)", "approval_denied");
                }
            },
            None => {
                let id = self.creer(now, qui, outil, digest, effet, request_id);
                lignes.push(format!(
                    "[auth] confirmation demandée : {} {} (demande {id}, expire dans {} s)",
                    qui.keyid, outil, self.ttl
                ));
                self.note(now, &qui.keyid, outil, "en attente", "approval_pending");
                created = Some(id);
            }
        }
        Decision {
            verdict,
            lignes,
            created,
        }
    }

    #[allow(dead_code)] // utilisé par les tests de la machine à états
    fn trouver(&self, id: &str) -> Option<&Entree> {
        self.entrees.iter().find(|e| e.id == id)
    }

    /// Demandes et permis visibles par `actor` : la session locale voit tout,
    /// un client ne voit QUE les siens (§5 : isolation entre clients).
    fn vue(&self, actor: Actor<'_>, now: u64) -> Value {
        let visible = |e: &Entree| match actor.keyid() {
            None => true,
            Some(k) => e.keyid == k,
        };
        let pending: Vec<Value> = self
            .entrees
            .iter()
            .filter(|e| e.etat == Etat::Attente && visible(e))
            .map(|e| {
                json!({
                    "id": e.id,
                    "keyid": e.keyid,
                    "label": e.label,
                    "fingerprint": e.empreinte,
                    "address": e.adresse,
                    "tool": e.outil,
                    "effect": e.effet,
                    "request_id": e.request_id,
                    "created_ts": e.cree,
                    "expires_in": e.echeance.saturating_sub(now),
                })
            })
            .collect();
        let grants: Vec<Value> = self
            .entrees
            .iter()
            .filter(|e| matches!(e.etat, Etat::UneFois | Etat::Fenetre) && visible(e))
            .map(|e| {
                json!({
                    "id": e.id,
                    "keyid": e.keyid,
                    "label": e.label,
                    "tool": e.outil,
                    "mode": if e.etat == Etat::UneFois { "once" } else { "window" },
                    "expires_in": e.echeance.saturating_sub(now),
                })
            })
            .collect();
        let decisions: Vec<Value> = self
            .decisions
            .iter()
            .rev()
            .filter(|d| match actor.keyid() {
                None => true,
                Some(k) => d["keyid"].as_str() == Some(k),
            })
            .take(DECISIONS_KEPT)
            .cloned()
            .collect();
        json!({
            "pending": pending, "grants": grants, "decisions": decisions,
            "pending_max": MAX_PENDING, "request_ttl_secs": self.ttl,
            "window_secs": self.fenetre,
        })
    }
}

// ---------------------------------------------------------------------------
// État process-global (perdu au redémarrage, §5)
// ---------------------------------------------------------------------------

fn store() -> &'static Mutex<Core> {
    static STORE: OnceLock<Mutex<Core>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(Core::new(REQUEST_TTL_SECS, window_secs())))
}

fn lock_store() -> MutexGuard<'static, Core> {
    store().lock().unwrap_or_else(|p| p.into_inner())
}

/// Journalise des lignes produites par la machine à états. Aucune de ces lignes
/// ne porte d'argument d'outil, de valeur d'environnement ni de secret.
fn journaliser(state: &Arc<AppState>, lignes: &[String]) {
    for l in lignes {
        crate::manage::add_log(state, "info", l);
    }
}

/// Résout l'identité affichable d'un client (libellé + empreinte courte) dans
/// le registre. Aucun accès réseau : le client est déjà authentifié.
fn identite(state: &Arc<AppState>, keyid: &str) -> (String, String) {
    let settings = lockv(&state.settings);
    match settings.auth.find(keyid) {
        Some(c) => (
            c.label.clone(),
            crate::auth::registry::short_fingerprint_of(&c.public_key).unwrap_or_default(),
        ),
        None => (String::new(), String::new()),
    }
}

/// Empreinte SHA-256 des arguments **réellement exécutés** (contrat C5) : elle
/// lie le permis à ces arguments, jamais à un résumé reformaté.
pub fn arguments_digest(args: &Value) -> String {
    crate::crypto::hex(&crate::crypto::sha256(args.to_string().as_bytes()))
}

/// Décision de la politique C5 pour un appel d'outil.
pub fn decide(
    state: &Arc<AppState>,
    client: &str,
    tool: &str,
    args: &Value,
    request_id: &Value,
) -> Decision {
    let adresse = request_peer();
    let (label, empreinte) = identite(state, client);
    let effet = effect_of(state, tool, args);
    let digest = arguments_digest(args);
    let rid = match request_id {
        Value::String(s) => court(s, 64),
        Value::Number(n) => n.to_string(),
        _ => "-".to_string(),
    };
    let qui = Qui {
        keyid: client.to_string(),
        label,
        empreinte,
        adresse,
    };
    let now = now_ts();
    let d = lock_store().decider(now, &qui, tool, &digest, &effet, &rid);
    journaliser(state, &d.lignes);
    // Notification SYSTÈME à la création d'une demande seulement : une seule
    // par demande (§5), hors du fil de la requête (le toast PowerShell prend
    // plusieurs secondes — la réponse `403` ne doit pas attendre).
    if d.created.is_some() {
        notifier(state, &qui, tool, &effet);
    }
    d
}

/// Bandeau + notification : un toast Windows par demande créée. Un échec du
/// toast n'empêche jamais la réponse — et rien ne ramène la fenêtre au premier
/// plan (elle ne revient que sur clic de l'utilisateur, `docs/auth.md` §5).
fn notifier(state: &Arc<AppState>, qui: &Qui, tool: &str, effet: &Value) {
    let desktop = lockv(&state.settings).notify.desktop;
    if !desktop {
        return;
    }
    let eff = effet["resume"].as_str().unwrap_or("(effet inconnu)");
    // Texte de notification : mis en forme par `notify` (pur, testé), jamais
    // concaténé dans le script PowerShell ni dans la page.
    let (titre, msg) = crate::notify::approval_pending_message(
        &court(&qui.label, 48),
        &qui.empreinte,
        &qui.adresse,
        tool,
        eff,
    );
    let etat = Arc::clone(state);
    std::thread::spawn(move || {
        if let Err(e) = crate::notify::desktop_toast(&titre, &msg) {
            crate::manage::add_log(&etat, "warn", &format!("[auth] notification : {e}"));
        }
    });
}

/// Politique branchée sur le `ServerBuilder` (`require_approval`, contrat C5).
pub struct Policy {
    pub state: Arc<AppState>,
}

impl ApprovalPolicy for Policy {
    fn decide(&self, request: &ApprovalRequest<'_>) -> ApprovalVerdict {
        let d = decide(
            &self.state,
            request.client,
            request.tool,
            request.arguments,
            request.request_id,
        );
        match d.verdict {
            Verdict::Allow => ApprovalVerdict::Approved,
            Verdict::Deny => ApprovalVerdict::Denied,
            // 2 s : une demande décidée se consomme au réessai suivant, sans
            // faire tourner le client en boucle serrée.
            Verdict::Pending => ApprovalVerdict::Pending {
                retry_after: Some(std::time::Duration::from_secs(2)),
            },
        }
    }
}

/// Hook C4 : chaque décision d'autorisation de la lib atterrit dans le journal
/// d'accès (les 100 dernières, écran « Accès ») et, pour les refus, dans
/// `mcc.log`. C'est le seul chemin par lequel un refus de droit ou
/// d'authentification devient visible localement.
pub fn note_decision(state: &Arc<AppState>, ev: &AuthDecisionEvent<'_>) {
    let (verdict, motif) = match ev.verdict {
        AuthVerdict::Allowed => ("accordé", "-"),
        AuthVerdict::RightMissing => ("droit insuffisant", "right_missing"),
        AuthVerdict::ApprovalDenied => ("refusé", ev.reason.unwrap_or("approval_denied")),
        AuthVerdict::ApprovalPending => ("en attente", "approval_pending"),
        AuthVerdict::Unauthenticated => ("non authentifié", ev.reason.unwrap_or("unauthenticated")),
        _ => ("décision", ev.reason.unwrap_or("-")),
    };
    // Journal d'audit CHAÎNÉ (lot M5, docs/auth.md §11) : la décision part au
    // transport, seul à connaître l'adresse source et le `keyid` revendiqué.
    // Vocabulaire machine, aligné sur `AuthVerdict`.
    let jeton = match ev.verdict {
        AuthVerdict::Allowed => "allowed",
        AuthVerdict::RightMissing => "right_missing",
        AuthVerdict::ApprovalDenied => "approval_denied",
        AuthVerdict::ApprovalPending => "approval_pending",
        AuthVerdict::Unauthenticated => "unauthenticated",
        _ => "decision",
    };
    crate::audit::note(jeton, motif, ev.client, ev.method, ev.name.unwrap_or("-"));
    let keyid = ev.client.unwrap_or("-");
    let outil = ev.name.unwrap_or("-");
    let now = now_ts();
    {
        let mut c = lock_store();
        c.note(now, keyid, outil, verdict, motif);
    }
    if !matches!(ev.verdict, AuthVerdict::Allowed) {
        crate::manage::add_log(
            state,
            "warn",
            &format!("[auth] {verdict} : {keyid} {outil} ({motif})"),
        );
    }
}

/// Vue de l'écran « Accès » (demandes, permis, décisions). `clients` et
/// `pairs` sont ajoutés par l'action locale `access_list`.
pub fn list(state: &Arc<AppState>, actor: Actor<'_>) -> Value {
    let now = now_ts();
    let (lignes, mut vue) = {
        let mut c = lock_store();
        let lignes = c.balayer(now);
        let vue = c.vue(actor, now);
        (lignes, vue)
    };
    journaliser(state, &lignes);
    if let Value::Object(o) = &mut vue {
        o.insert(
            "actor".into(),
            json!(match actor {
                Actor::LocalAdmin => "admin",
                Actor::Client(_) => "client",
            }),
        );
    }
    vue
}

fn push_bytes(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    buf.extend_from_slice(bytes);
}

impl Core {
    fn hash_into(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.rev_seq.to_be_bytes());
        buf.extend_from_slice(&(self.entrees.len() as u64).to_be_bytes());
        for e in &self.entrees {
            push_bytes(buf, e.id.as_bytes());
            buf.push(match e.etat {
                Etat::Attente => 0,
                Etat::UneFois => 1,
                Etat::Fenetre => 2,
                Etat::Refusee => 3,
            });
            buf.extend_from_slice(&e.cree.to_be_bytes());
            buf.extend_from_slice(&e.echeance.to_be_bytes());
        }
        buf.extend_from_slice(&(self.decisions.len() as u64).to_be_bytes());
        for d in &self.decisions {
            push_bytes(buf, d.to_string().as_bytes());
        }
    }
}

fn hash_auth_registry(auth: &crate::auth::AuthState, now: u64, buf: &mut Vec<u8>) {
    buf.extend_from_slice(&(auth.pairs.len() as u64).to_be_bytes());
    for p in &auth.pairs {
        push_bytes(buf, p.pair_id.as_bytes());
        push_bytes(buf, p.keyid.as_bytes());
        push_bytes(buf, p.status(now).as_bytes());
        buf.extend_from_slice(&p.created.to_be_bytes());
        buf.extend_from_slice(&p.decided_at.unwrap_or(0).to_be_bytes());
    }
    buf.extend_from_slice(&(auth.clients.len() as u64).to_be_bytes());
    for c in &auth.clients {
        push_bytes(buf, c.keyid.as_bytes());
        push_bytes(buf, c.label.as_bytes());
        push_bytes(buf, c.public_key.as_bytes());
        push_bytes(buf, c.status.as_bytes());
        push_bytes(buf, c.created_at.as_bytes());
        push_bytes(buf, c.last_used_at.as_deref().unwrap_or("").as_bytes());
        push_bytes(buf, c.revoked_at.as_deref().unwrap_or("").as_bytes());
        push_bytes(buf, c.rotated_to.as_deref().unwrap_or("").as_bytes());
        push_bytes(buf, c.expires_at.as_deref().unwrap_or("").as_bytes());
        buf.push(u8::from(c.stale_180d(now)));
        buf.extend_from_slice(&(c.rights.len() as u64).to_be_bytes());
        for r in &c.rights {
            push_bytes(buf, r.as_bytes());
        }
    }
}

/// Empreinte entière (53 bits, exacte en JS) de tout l'état Accès : change dès
/// qu'une confirmation en attente, un permis actif, une demande d'appairage, un
/// client enregistré ou une décision récente change.
#[cfg(test)]
fn compute_revision(core: &Core, auth: &crate::auth::AuthState, now: u64) -> u64 {
    let mut buf = Vec::with_capacity(512);
    core.hash_into(&mut buf);
    hash_auth_registry(auth, now, &mut buf);
    digest_to_revision(&buf)
}

fn digest_to_revision(buf: &[u8]) -> u64 {
    let digest = crate::crypto::sha256(buf);
    u64::from_be_bytes([
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    ]) & ((1u64 << 53) - 1)
}

/// Résumé pour le bandeau de l'interface (aucun texte venu d'un client).
pub fn summary(state: &Arc<AppState>) -> Value {
    let now = now_ts();
    let mut buf = Vec::with_capacity(512);
    let (pending, grants) = {
        let mut c = lock_store();
        let _ = c.balayer(now);
        c.hash_into(&mut buf);
        (
            c.entrees.iter().filter(|e| e.etat == Etat::Attente).count(),
            c.entrees
                .iter()
                .filter(|e| matches!(e.etat, Etat::UneFois | Etat::Fenetre))
                .count(),
        )
    };
    let pairs = {
        let mut settings = lockv(&state.settings);
        crate::auth::pair::prune(&mut settings.auth.pairs, now);
        let pending_pairs = settings
            .auth
            .pairs
            .iter()
            .filter(|p| p.is_pending(now))
            .count();
        hash_auth_registry(&settings.auth, now, &mut buf);
        pending_pairs
    };
    let revision = digest_to_revision(&buf);
    json!({"pending": pending, "grants": grants, "pairs": pairs, "revision": revision})
}

// ---------------------------------------------------------------------------
// Décisions locales (session `admin` uniquement)
// ---------------------------------------------------------------------------

fn exiger_admin(actor: Actor<'_>) -> Result<(), String> {
    match actor {
        Actor::LocalAdmin => Ok(()),
        // Un client ne peut ni approuver sa propre demande ni celle d'un
        // autre : il n'existe aucun chemin pour lui (§5).
        Actor::Client(_) => Err("approbation réservée à la session locale".to_string()),
    }
}

/// Approuve une demande : `"once"` (une seule exécution) ou `"window"`
/// (permis valable `window_secs()` pour ce client, cet outil et ces arguments).
pub fn approve(
    state: &Arc<AppState>,
    id: &str,
    mode: &str,
    actor: Actor<'_>,
) -> Result<Value, String> {
    exiger_admin(actor)?;
    if mode != "once" && mode != "window" {
        return Err("mode inconnu (once|window)".to_string());
    }
    let now = now_ts();
    let mut c = lock_store();
    let lignes = c.balayer(now);
    let Some(pos) = c
        .entrees
        .iter()
        .position(|e| e.id == id && e.etat == Etat::Attente)
    else {
        drop(c);
        journaliser(state, &lignes);
        return Err("demande inconnue, expirée ou déjà décidée".to_string());
    };
    c.entrees[pos].etat = if mode == "once" {
        Etat::UneFois
    } else {
        Etat::Fenetre
    };
    c.entrees[pos].echeance = now.saturating_add(c.fenetre);
    let (keyid, outil, effet) = {
        let e = &c.entrees[pos];
        (e.keyid.clone(), e.outil.clone(), e.effet.clone())
    };
    let duree = c.fenetre;
    c.note(
        now,
        &keyid,
        &outil,
        if mode == "once" {
            "approuvée (une fois)"
        } else {
            "approuvée (permis)"
        },
        "-",
    );
    drop(c);
    journaliser(state, &lignes);
    crate::manage::add_log(
        state,
        "info",
        &format!(
            "[auth] confirmation approuvée par la session locale : {keyid} {outil} — {}{}",
            if mode == "once" { "une fois" } else { "permis" },
            if mode == "once" {
                String::new()
            } else {
                format!(" {duree} s")
            }
        ),
    );
    Ok(json!({
        "ok": true, "id": id, "mode": mode, "tool": outil,
        "keyid": keyid, "effect": effet, "window_secs": duree,
    }))
}

/// Refuse une demande : le client reçoit `approval_denied` et rien ne s'exécute.
pub fn deny(state: &Arc<AppState>, id: &str, actor: Actor<'_>) -> Result<Value, String> {
    exiger_admin(actor)?;
    let now = now_ts();
    let mut c = lock_store();
    let lignes = c.balayer(now);
    let Some(pos) = c
        .entrees
        .iter()
        .position(|e| e.id == id && e.etat == Etat::Attente)
    else {
        drop(c);
        journaliser(state, &lignes);
        return Err("demande inconnue, expirée ou déjà décidée".to_string());
    };
    c.entrees[pos].etat = Etat::Refusee;
    let (keyid, outil) = {
        let e = &c.entrees[pos];
        (e.keyid.clone(), e.outil.clone())
    };
    c.note(now, &keyid, &outil, "refusée (local)", "approval_denied");
    drop(c);
    journaliser(state, &lignes);
    crate::manage::add_log(
        state,
        "info",
        &format!("[auth] confirmation refusée par la session locale : {keyid} {outil}"),
    );
    Ok(json!({"ok": true, "id": id, "tool": outil, "keyid": keyid}))
}

/// Révoque un permis en cours (même écran, §5 : « révocable »).
pub fn revoke(state: &Arc<AppState>, id: &str, actor: Actor<'_>) -> Result<Value, String> {
    exiger_admin(actor)?;
    let now = now_ts();
    let mut c = lock_store();
    let lignes = c.balayer(now);
    let Some(pos) = c
        .entrees
        .iter()
        .position(|e| e.id == id && matches!(e.etat, Etat::UneFois | Etat::Fenetre))
    else {
        drop(c);
        journaliser(state, &lignes);
        return Err("permis inconnu ou déjà consommé".to_string());
    };
    let (keyid, outil) = {
        let e = &c.entrees[pos];
        (e.keyid.clone(), e.outil.clone())
    };
    c.entrees.remove(pos);
    c.note(now, &keyid, &outil, "permis révoqué", "grant_revoked");
    drop(c);
    journaliser(state, &lignes);
    crate::manage::add_log(
        state,
        "info",
        &format!("[auth] permis de confirmation révoqué : {keyid} {outil}"),
    );
    Ok(json!({"ok": true, "id": id, "tool": outil, "keyid": keyid}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Settings;
    use std::collections::{HashMap, HashSet};

    /// État de moteur minimal (aucun serveur, réglages par défaut).
    fn etat() -> Arc<AppState> {
        Arc::new(AppState {
            servers: Mutex::new(Vec::new()),
            log: Mutex::new(VecDeque::new()),
            settings: Mutex::new(Settings::default()),
            children: Mutex::new(HashMap::new()),
            peers: Mutex::new(HashMap::new()),
            allowed_origins: Mutex::new(Vec::new()),
            allowed_hosts: Mutex::new(Vec::new()),
            spawn_fails: Mutex::new(HashMap::new()),
            runtime: Mutex::new(crate::runtime::RuntimeLog::new()),
            spawning: Mutex::new(HashSet::new()),
            update_latest: Mutex::new(None),
            lan_ok: Mutex::new(None),
            config_path: String::new(),
        })
    }

    const A: &str = "mcc-ai-0000000000000001";
    const B: &str = "mcc-ai-0000000000000002";

    fn qui(keyid: &str) -> Qui {
        Qui {
            keyid: keyid.into(),
            label: format!("client {keyid}"),
            empreinte: "ed25519:0000000000000000".into(),
            adresse: "127.0.0.1".into(),
        }
    }

    fn args_simple() -> Value {
        json!({ "id": "srv-1" })
    }

    fn effet() -> Value {
        json!({"kind": "lancement", "resume": "node srv.js"})
    }

    /// Les tests qui passent par l'état GLOBAL (celui du vrai moteur) se
    /// sérialisent : ils partagent le même magasin de demandes.
    fn garde() -> std::sync::MutexGuard<'static, ()> {
        static G: Mutex<()> = Mutex::new(());
        G.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Une demande neuve bloque, la même demande n'est pas dupliquée, un autre
    /// jeu d'arguments en crée une seconde.
    #[test]
    fn demande_creee_une_seule_fois_par_client_outil_et_arguments() {
        let mut c = Core::new(120, 600);
        let d = arguments_digest(&args_simple());
        let r1 = c.decider(1000, &qui(A), "start_server", &d, &effet(), "1");
        assert_eq!(r1.verdict, Verdict::Pending);
        assert_eq!(c.en_attente(), 1);
        let r2 = c.decider(1001, &qui(A), "start_server", &d, &effet(), "2");
        assert_eq!(r2.verdict, Verdict::Pending);
        assert_eq!(c.en_attente(), 1, "la demande ne doit pas être dupliquée");
        // autres arguments = autre demande (ce qui s'exécute change)
        let autre = arguments_digest(&json!({ "id": "srv-2" }));
        let r3 = c.decider(1002, &qui(A), "start_server", &autre, &effet(), "3");
        assert_eq!(r3.verdict, Verdict::Pending);
        assert_eq!(c.en_attente(), 2);
    }

    /// Permis « une fois » : consommé par le premier appel, le second redemande.
    #[test]
    fn permis_une_fois_consomme_au_premier_appel() {
        let mut c = Core::new(120, 600);
        let d = arguments_digest(&args_simple());
        c.decider(1000, &qui(A), "start_server", &d, &effet(), "1");
        let id = c.entrees[0].id.clone();
        c.entrees[0].etat = Etat::UneFois;
        c.entrees[0].echeance = 1600;
        assert_eq!(
            c.decider(1001, &qui(A), "start_server", &d, &effet(), "2")
                .verdict,
            Verdict::Allow
        );
        assert!(c.trouver(&id).is_none(), "le permis une fois est consommé");
        assert_eq!(
            c.decider(1002, &qui(A), "start_server", &d, &effet(), "3")
                .verdict,
            Verdict::Pending
        );
    }

    /// Permis « fenêtre » : valable jusqu'à l'échéance, puis REDEMANDÉ (le
    /// permis expiré est journalisé et retiré).
    #[test]
    fn permis_fenetre_expire_et_redemande() {
        let mut c = Core::new(120, 600);
        let d = arguments_digest(&args_simple());
        c.decider(1000, &qui(A), "start_server", &d, &effet(), "1");
        c.entrees[0].etat = Etat::Fenetre;
        c.entrees[0].echeance = 1005;
        assert_eq!(
            c.decider(1004, &qui(A), "start_server", &d, &effet(), "2")
                .verdict,
            Verdict::Allow
        );
        let apres = c.decider(1006, &qui(A), "start_server", &d, &effet(), "3");
        assert_eq!(apres.verdict, Verdict::Pending, "permis expiré = redemande");
        assert!(
            apres
                .lignes
                .iter()
                .any(|l| l.contains("permis de confirmation expiré")),
            "l'expiration doit laisser une ligne : {:?}",
            apres.lignes
        );
        assert_eq!(c.en_attente(), 1);
    }

    /// Demande non décidée : elle expire au bout de 2 min, le journal le dit,
    /// et un nouvel appel crée une demande neuve.
    #[test]
    fn demande_expiree_est_journalisee_puis_recreee() {
        let mut c = Core::new(120, 600);
        let d = arguments_digest(&args_simple());
        c.decider(1000, &qui(A), "start_server", &d, &effet(), "1");
        let r = c.decider(1121, &qui(A), "start_server", &d, &effet(), "2");
        assert_eq!(r.verdict, Verdict::Pending);
        assert!(
            r.lignes.iter().any(|l| l.contains("expirée sans décision")),
            "{:?}",
            r.lignes
        );
        assert_eq!(c.en_attente(), 1, "une seule demande vivante");
        // la trace reste dans l'anneau des décisions
        assert!(c.decisions.iter().any(|d| d["verdict"] == "expirée"));
    }

    /// Refus local : `Deny` (jamais d'exécution), et pas de nouvelle demande
    /// tant que l'entrée vit.
    #[test]
    fn refus_local_bloque_sans_redemander() {
        let mut c = Core::new(120, 600);
        let d = arguments_digest(&args_simple());
        c.decider(1000, &qui(A), "start_server", &d, &effet(), "1");
        c.entrees[0].etat = Etat::Refusee;
        assert_eq!(
            c.decider(1001, &qui(A), "start_server", &d, &effet(), "2")
                .verdict,
            Verdict::Deny
        );
        assert_eq!(c.en_attente(), 0);
    }

    /// 32 demandes au plus : la 33e évince la plus ancienne, qui reste tracée.
    #[test]
    fn demandes_bornees_a_32_la_plus_ancienne_evincee() {
        let mut c = Core::new(120, 600);
        for i in 0..MAX_PENDING {
            let d = arguments_digest(&json!({ "id": format!("srv-{i}") }));
            c.decider(1000, &qui(A), "start_server", &d, &effet(), "1");
        }
        assert_eq!(c.en_attente(), MAX_PENDING);
        let premier = c.entrees[0].id.clone();
        let d = arguments_digest(&json!({ "id": "srv-de-trop" }));
        // même instant : aucune demande n'a expiré, seule la borne agit
        c.decider(1001, &qui(A), "start_server", &d, &effet(), "2");
        assert_eq!(c.en_attente(), MAX_PENDING, "la borne tient");
        assert!(
            c.trouver(&premier).is_none(),
            "la plus ancienne est évincée"
        );
        assert!(c.decisions.iter().any(|d| d["verdict"] == "évincée"));
    }

    /// Isolation : un client ne voit NI les demandes NI les permis d'un autre.
    #[test]
    fn un_client_ne_voit_que_ses_demandes() {
        let mut c = Core::new(120, 600);
        let d = arguments_digest(&args_simple());
        c.decider(1000, &qui(A), "start_server", &d, &effet(), "1");
        let vue_b = c.vue(Actor::Client(B), 1001);
        assert!(vue_b["pending"].as_array().unwrap().is_empty());
        assert!(vue_b["decisions"].as_array().unwrap().is_empty());
        let vue_a = c.vue(Actor::Client(A), 1001);
        assert_eq!(vue_a["pending"].as_array().unwrap().len(), 1);
        // la session locale, elle, voit tout
        assert_eq!(
            c.vue(Actor::LocalAdmin, 1001)["pending"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    /// Un client ne peut pas approuver — ni sa demande, ni celle d'un autre.
    #[test]
    fn un_client_ne_peut_pas_approuver_sa_propre_demande() {
        let _g = garde();
        let st = etat();
        let d = arguments_digest(&args_simple());
        {
            let mut c = lock_store();
            c.entrees.clear();
            c.decider(now_ts(), &qui(A), "start_server", &d, &effet(), "1");
        }
        let id = {
            let c = lock_store();
            c.entrees[0].id.clone()
        };
        assert_eq!(
            approve(&st, &id, "window", Actor::Client(A)).unwrap_err(),
            "approbation réservée à la session locale"
        );
        assert_eq!(
            deny(&st, &id, Actor::Client(A)).unwrap_err(),
            "approbation réservée à la session locale"
        );
        // et l'approbation d'un AUTRE client est refusée de la même façon
        assert!(approve(&st, &id, "window", Actor::Client(B)).is_err());
        // la session locale, elle, approuve
        let ok = approve(&st, &id, "once", Actor::LocalAdmin).unwrap();
        assert_eq!(ok["ok"], json!(true));
        {
            let mut c = lock_store();
            c.entrees.clear();
        }
    }

    /// Le permis est lié aux ARGUMENTS : mêmes client et outil, arguments
    /// différents = demande distincte (rien ne s'exécute sans accord).
    #[test]
    fn permis_lie_aux_arguments_pas_seulement_au_couple_client_outil() {
        let mut c = Core::new(120, 600);
        let d1 = arguments_digest(&json!({ "id": "srv-1" }));
        let d2 = arguments_digest(&json!({ "id": "srv-2" }));
        c.decider(1000, &qui(A), "start_server", &d1, &effet(), "1");
        c.entrees[0].etat = Etat::Fenetre;
        c.entrees[0].echeance = 1600;
        assert_eq!(
            c.decider(1001, &qui(A), "start_server", &d1, &effet(), "2")
                .verdict,
            Verdict::Allow
        );
        assert_eq!(
            c.decider(1002, &qui(A), "start_server", &d2, &effet(), "3")
                .verdict,
            Verdict::Pending,
            "un autre argument ne doit pas hériter du permis"
        );
    }

    /// Révocation : le permis disparaît, l'appel suivant redemande.
    #[test]
    fn permis_revoque_redemande() {
        let _g = garde();
        let st = etat();
        let d = arguments_digest(&args_simple());
        let id = {
            let mut c = lock_store();
            c.entrees.clear();
            let r = c.decider(now_ts(), &qui(A), "start_server", &d, &effet(), "1");
            assert_eq!(r.verdict, Verdict::Pending);
            c.entrees[0].id.clone()
        };
        approve(&st, &id, "window", Actor::LocalAdmin).unwrap();
        revoke(&st, &id, Actor::LocalAdmin).unwrap();
        let r = decide(&st, A, "start_server", &args_simple(), &json!(1));
        assert_eq!(r.verdict, Verdict::Pending, "permis révoqué = redemande");
        {
            let mut c = lock_store();
            c.entrees.clear();
        }
    }

    /// L'effet d'un `install_store_server` porte les NOMS des variables
    /// d'environnement, jamais leurs valeurs.
    #[test]
    fn effet_ne_divulgue_jamais_une_valeur_d_environnement() {
        let st = etat();
        let effet = effect_of(
            &st,
            "install_store_server",
            &json!({
                "id": "github",
                "env": { "GITHUB_TOKEN": "ghp_valeur-secrete", "API_KEY": "sk-xyz" }
            }),
        );
        let texte = effet.to_string();
        assert!(
            texte.contains("GITHUB_TOKEN") && texte.contains("API_KEY"),
            "{texte}"
        );
        assert!(!texte.contains("ghp_valeur-secrete"), "{texte}");
        assert!(!texte.contains("sk-xyz"), "{texte}");
    }

    /// Les arguments d'un outil distant sont affichés, secrets masqués.
    #[test]
    fn arguments_de_proxy_masques() {
        let st = etat();
        let effet = effect_of(
            &st,
            "call_server_tool",
            &json!({
                "server": "srv-1", "tool": "deploy",
                "arguments": { "token": "abcdef", "path": "/tmp/x", "password": "p" }
            }),
        );
        let texte = effet.to_string();
        assert!(texte.contains("/tmp/x"), "{texte}");
        assert!(!texte.contains("abcdef"), "{texte}");
        assert!(!texte.contains("\"p\""), "{texte}");
        assert!(texte.contains("<masqué>"), "{texte}");
    }

    /// Un texte importé n'est jamais recopié : on n'en extrait que les
    /// programmes (le bloc peut porter des jetons dans ses `env`).
    #[test]
    fn import_ne_recopie_jamais_le_texte_brut() {
        let st = etat();
        let texte = r#"{"mcpServers":{"gh":{"command":"npx","args":["-y","gh-mcp"],
            "env":{"GITHUB_TOKEN":"ghp_secret_importe"}}}}"#;
        let effet = effect_of(&st, "import_config", &json!({ "text": texte }));
        let rendu = effet.to_string();
        assert!(rendu.contains("npx") && rendu.contains("gh"), "{rendu}");
        assert!(!rendu.contains("ghp_secret_importe"), "{rendu}");
        assert!(!rendu.contains("mcpServers"), "{rendu}");
    }

    /// L'anneau des décisions est borné à 100 (C4).
    #[test]
    fn decisions_bornees_a_100() {
        let mut c = Core::new(120, 600);
        for i in 0..150 {
            c.note(i, A, "list_servers", "accordé", "-");
        }
        assert_eq!(c.decisions.len(), DECISIONS_KEPT);
        assert_eq!(c.decisions.back().unwrap()["ts"], json!(149));
    }

    /// La durée du permis vient de `MCC_CONFIRM_SECS`, bornée à [1, 86 400].
    #[test]
    fn duree_du_permis_reglable_pour_les_tests() {
        // valeur par défaut quand la variable est absente ou aberrante
        std::env::remove_var("MCC_CONFIRM_SECS");
        assert_eq!(window_secs(), DEFAULT_WINDOW_SECS);
        std::env::set_var("MCC_CONFIRM_SECS", "2");
        assert_eq!(window_secs(), 2);
        std::env::set_var("MCC_CONFIRM_SECS", "0");
        assert_eq!(window_secs(), DEFAULT_WINDOW_SECS);
        std::env::set_var("MCC_CONFIRM_SECS", "beaucoup");
        assert_eq!(window_secs(), DEFAULT_WINDOW_SECS);
        std::env::remove_var("MCC_CONFIRM_SECS");
    }

    #[test]
    fn revision_change_sur_toutes_les_mutations_et_reste_stable_au_repos() {
        let mut c = Core::new(120, 600);
        let mut auth = crate::auth::AuthState::default();
        let r0 = compute_revision(&c, &auth, 1000);
        assert_eq!(
            r0,
            compute_revision(&c, &auth, 1005),
            "stable au repos même quand le temps avance"
        );

        // 1. Confirmation en attente créée
        let d = arguments_digest(&args_simple());
        c.decider(1005, &qui(A), "start_server", &d, &effet(), "1");
        let r1 = compute_revision(&c, &auth, 1005);
        assert_ne!(r1, r0);
        assert_eq!(
            r1,
            compute_revision(&c, &auth, 1010),
            "stable tant que la demande vit sans changement"
        );

        // 2. Passage en permis actif (UneFois -> consommé -> Fenetre)
        c.entrees[0].etat = Etat::Fenetre;
        c.entrees[0].echeance = 1605;
        let r2 = compute_revision(&c, &auth, 1010);
        assert_ne!(r2, r1);

        // 3. Décision récente enregistrée (même sans changer les permis)
        c.note(1011, A, "list_servers", "accordé", "-");
        let r3 = compute_revision(&c, &auth, 1011);
        assert_ne!(r3, r2);

        // 4. Expiration balayée
        let _ = c.balayer(1606);
        let r4 = compute_revision(&c, &auth, 1606);
        assert_ne!(r4, r3);

        // 5. Demande d'appairage ajoutée puis expirée
        auth.pairs.push(crate::auth::pair::PairRequest {
            pair_id: "00000000000000000000000000000001".into(),
            keyid: A.into(),
            label: "poste".into(),
            public_key: "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a".into(),
            code: "123 456".into(),
            created: 1606,
            paired_from: "127.0.0.1".into(),
            decision: None,
            decided_at: None,
        });
        let r5 = compute_revision(&c, &auth, 1606);
        assert_ne!(r5, r4);
        let r5_exp = compute_revision(&c, &auth, 1606 + 120);
        assert_ne!(r5_exp, r5, "expiration d'appairage détectée");

        // 6. Client enregistré puis modifié
        auth.insert(crate::auth::ClientRecord::new(
            A.into(),
            "poste".into(),
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a".into(),
            vec!["lecture".into()],
            "127.0.0.1".into(),
        ))
        .unwrap();
        let r6 = compute_revision(&c, &auth, 1730);
        assert_ne!(r6, r5_exp);
        auth.revoke(A).unwrap();
        let r7 = compute_revision(&c, &auth, 1730);
        assert_ne!(r7, r6);
    }
}
