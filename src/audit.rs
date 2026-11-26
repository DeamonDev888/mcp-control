//! Journal d'audit **chaîné par empreinte** (lot M5, `docs/auth.md` §11).
//!
//! Un fichier distinct de `mcc.log`, une ligne par décision d'accès, écrite
//! avant que la réponse ne parte. Chaque ligne porte l'empreinte de la
//! précédente : modifier, supprimer ou insérer une ligne rompt la chaîne, et
//! `mcp-control auth verify-log` recalcule tout pour le dire.
//!
//! Format (champs séparés par une espace, valeurs sans espace) :
//!
//! ```text
//! seq=000042 ts=2026-11-15T12:00:00Z keyid=mcc-ai-3f9a1c07b2d84e51 \
//!   fp=ed25519:3f9a1c07b2d84e51 ip=192.0.2.7 method=tools/call \
//!   tool=start_server verdict=approval_pending reason=- prev=<64 hex> mac=<64 hex>
//! ```
//!
//! `mac = HMAC-SHA256(k_audit, ligne_sans_mac)` ; la ligne sans `mac` se
//! termine par `prev=<64 hex> ` et `prev` vaut le `mac` de la ligne
//! précédente (32 zéros pour la première ligne d'une chaîne neuve).
//!
//! **Ce qui n'y figure jamais** (§11) : arguments d'outil, valeurs
//! d'environnement, nonce, signature, clé (publique ou privée), résultat
//! d'outil, contenu de réponse. Seuls le `keyid` (public), son empreinte
//! courte, l'adresse source, la méthode, le **nom** de l'outil, le verdict et
//! le motif précis entrent ici — et le motif précis ne sort que par ce
//! fichier, jamais par le réseau.
//!
//! **Rotation** sans casser la chaîne : au-delà de 8 Mio le fichier courant
//! devient `auth-audit.log.1` (`.1`→`.2`… jusqu'à `.5`), et le nouveau
//! fichier commence par une ligne `verdict=segment reason=rotation` dont le
//! `prev` est le `mac` de la dernière ligne de l'ancien segment. La chaîne se
//! vérifie donc d'un fichier à l'autre.
//!
//! **Clé** : celle du trousseau (compte `audit-hmac`, §9) ou son repli fichier
//! `auth/audit.key` — voir [`crate::auth::secret`].

use crate::types::{lockv, AppState};
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// Nom du fichier, à côté du `config.json` (§11) — distinct de `mcc.log`.
pub const AUDIT_FILE: &str = "auth-audit.log";
/// Seuil de rotation (§11 : 8 Mio, cinq fichiers).
pub const ROTATE_BYTES: u64 = 8 * 1024 * 1024;
/// Nombre de segments conservés (`.1` à `.5`).
pub const ROTATE_KEEP: usize = 5;
/// `prev` de la première ligne d'une chaîne neuve : 32 octets nuls en hex.
pub const ZERO_MAC: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Une décision d'accès, telle qu'elle est écrite sur disque.
pub struct Entry<'a> {
    pub ts: u64,
    pub keyid: &'a str,
    pub fp: &'a str,
    pub ip: &'a str,
    pub method: &'a str,
    pub tool: &'a str,
    pub verdict: &'a str,
    pub reason: &'a str,
}

/// Valeur de champ sûre : ASCII imprimable sans espace, bornée. Un texte venu
/// d'un client (libellé, nom d'outil) ne peut donc ni casser le format ni
/// injecter une ligne.
fn champ(valeur: &str, max: usize) -> String {
    let mut out = String::with_capacity(valeur.len().min(max));
    for c in valeur.chars() {
        if out.len() >= max {
            break;
        }
        let ok = c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-' | '@' | '+');
        out.push(if ok { c } else { '_' });
    }
    if out.is_empty() {
        out.push('-');
    }
    out
}

/// Horodatage ISO-8601 UTC (`2026-11-15T12:00:00Z`), forme du §11.
fn iso(ts: u64) -> String {
    format!("{}Z", crate::types::iso_utc(ts).replace(' ', "T"))
}

/// Préfixe de ligne **sans `mac`** — c'est exactement ce que le HMAC couvre,
/// `prev` compris.
fn prefixe(seq: u64, e: &Entry<'_>, prev: &str) -> String {
    format!(
        "seq={seq:06} ts={} keyid={} fp={} ip={} method={} tool={} verdict={} reason={} prev={prev} ",
        iso(e.ts),
        champ(e.keyid, 128),
        champ(e.fp, 24),
        champ(e.ip, 45),
        champ(e.method, 32),
        champ(e.tool, 64),
        champ(e.verdict, 24),
        champ(e.reason, 34),
    )
}

fn mac_de(cle: &[u8], prefixe: &str) -> String {
    crate::crypto::hex(&crate::crypto::hmac_sha256(cle, prefixe.as_bytes()))
}

/// Ligne complète : préfixe + `mac=<64 hex>`.
fn ligne_complete(seq: u64, e: &Entry<'_>, prev: &str, cle: &[u8]) -> (String, String) {
    let p = prefixe(seq, e, prev);
    let mac = mac_de(cle, &p);
    (format!("{p}mac={mac}"), mac)
}

/// Chemin du journal pour un `config.json` donné (même dossier, §11).
pub fn path_for(config_path: &str) -> PathBuf {
    if config_path.is_empty() {
        return PathBuf::from(AUDIT_FILE);
    }
    Path::new(config_path).with_file_name(AUDIT_FILE)
}

fn segment_path(base: &Path, n: usize) -> PathBuf {
    let mut s = base.as_os_str().to_os_string();
    s.push(format!(".{n}"));
    PathBuf::from(s)
}

/// Indique si un journal d'audit ou l'un de ses artefacts (segments `.1`..`.5`,
/// ancre `.head`) existe sur disque `[H-04][H-05]`.
pub fn has_audit_artifacts(base: &Path) -> bool {
    base.exists()
        || head_path(base).exists()
        || (1..=ROTATE_KEEP).any(|i| segment_path(base, i).exists())
}

/// Fichier d'ancre créé lors d'une rotation qui évince le segment `.5` (§11) :
/// conserve le `seq` et le `mac` de la dernière ligne évincée, signés par HMAC,
/// pour permettre de reprendre la chaîne tout en empêchant la suppression
/// furtive des premières lignes avant rotation.
pub fn head_path(base: &Path) -> PathBuf {
    base.with_extension("head")
}

fn write_head(path: &Path, seq: u64, mac: &str, cle: &[u8]) -> Result<(), String> {
    let prefix = format!("head seq={seq} mac={mac} ");
    let hmac = mac_de(cle, &prefix);
    let content = format!("{prefix}hmac={hmac}\n");
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("dossier {} impossible : {e}", parent.display()))?;
        }
    }
    let mut tmp_os = path.as_os_str().to_os_string();
    tmp_os.push(format!(".{}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp_os);
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp)
        .map_err(|e| format!("création {} impossible : {e}", tmp.display()))?;
    if let Err(e) = crate::manage::restrict_permissions(&tmp.to_string_lossy()) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("permissions {} impossibles : {e}", tmp.display()));
    }
    let write_res = (|| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.sync_all()?;
        Ok(())
    })();
    if let Err(e) = write_res {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("écriture {} impossible : {e}", tmp.display()));
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("renommage {} impossible : {e}", path.display())
    })?;
    let _ = crate::manage::restrict_permissions(&path.to_string_lossy());
    Ok(())
}

fn read_head(path: &Path, cle: &[u8]) -> Result<(u64, String), Falsification> {
    let nom = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    let texte = std::fs::read_to_string(path).map_err(|_| Falsification::Illisible {
        fichier: nom.clone(),
        ligne: 1,
    })?;
    let ligne = texte.trim();
    let Some(rest) = ligne.strip_prefix("head seq=") else {
        return Err(Falsification::Modifiee {
            fichier: nom,
            ligne: 1,
        });
    };
    let Some((seq_s, rest)) = rest.split_once(" mac=") else {
        return Err(Falsification::Modifiee {
            fichier: nom,
            ligne: 1,
        });
    };
    let Ok(seq) = seq_s.parse::<u64>() else {
        return Err(Falsification::Modifiee {
            fichier: nom,
            ligne: 1,
        });
    };
    let Some((mac, hmac)) = rest.split_once(" hmac=") else {
        return Err(Falsification::Modifiee {
            fichier: nom,
            ligne: 1,
        });
    };
    if !is_lower_hex64(mac) || !is_lower_hex64(hmac) {
        return Err(Falsification::Modifiee {
            fichier: nom,
            ligne: 1,
        });
    }
    let attendu = mac_de(cle, &format!("head seq={seq} mac={mac} "));
    if !crate::crypto::ct_eq(attendu.as_bytes(), hmac.as_bytes()) {
        return Err(Falsification::Modifiee {
            fichier: nom,
            ligne: 1,
        });
    }
    Ok((seq, mac.to_string()))
}

/// Dernière ligne exploitable d'un fichier : `(seq, mac)`. `None` = fichier
/// absent ou vide. Un fichier non vide dont la dernière ligne est illisible ou
/// dont le MAC ne se recalcule pas (si `cle` est fournie) est une **erreur** :
/// on n'écrit pas à la suite d'une chaîne inconnue ou falsifiée (échec fermé).
fn derniere_ligne(path: &Path, cle: Option<&[u8]>) -> Result<Option<(u64, String)>, String> {
    let texte = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("lecture {} impossible : {e}", path.display())),
    };
    let Some(l) = texte.lines().rev().find(|l| !l.trim().is_empty()) else {
        return Ok(None);
    };
    let lue = parse_ligne(l).ok_or_else(|| {
        format!(
            "dernière ligne illisible dans {} — chaîne inconnue, écriture refusée",
            path.display()
        )
    })?;
    if let Some(k) = cle {
        let i_mac = l.rfind(" mac=").unwrap_or(0);
        let recalcule = mac_de(k, &l[..i_mac + 1]);
        if !crate::crypto::ct_eq(recalcule.as_bytes(), lue.mac.as_bytes()) {
            return Err(format!(
                "dernière ligne falsifiée dans {} (MAC invalide) — écriture refusée",
                path.display()
            ));
        }
    }
    Ok(Some((lue.seq, lue.mac.to_string())))
}

/// Chaîne d'écriture d'un fichier de journal.
struct Journal {
    path: PathBuf,
    cle: Vec<u8>,
    seq: u64,
    last: String,
}

impl Journal {
    fn open(path: PathBuf, cle: Vec<u8>) -> Result<Self, String> {
        let (seq, last) = match derniere_ligne(&path, Some(&cle))? {
            Some(pair) => pair,
            None => {
                // Échec fermé [H-05] : si le journal courant a été supprimé ou vidé
                // alors que des segments archivés ou une ancre `.head` existent,
                // ne pas repartir silencieusement à seq=0.
                let has_older = head_path(&path).exists()
                    || (1..=ROTATE_KEEP).any(|i| segment_path(&path, i).exists());
                if has_older {
                    return Err(format!(
                        "journal courant {} absent ou vide alors que des segments archivés existent — chaîne rompue, écriture refusée",
                        path.display()
                    ));
                }
                (0, ZERO_MAC.to_string())
            }
        };
        Ok(Self {
            path,
            cle,
            seq,
            last,
        })
    }

    /// Rotation : le fichier courant passe en `.1`, les segments glissent, et
    /// le nouveau fichier s'ouvre sur une ligne `segment` qui prolonge la
    /// chaîne (son `prev` est le `mac` de la dernière ligne de l'ancien).
    ///
    /// MC-C04 : le nouveau segment est d'abord écrit, restreint (ACL) et
    /// synchronisé (`sync_all`) dans un fichier `.rotate.tmp` AVANT de toucher
    /// à `auth-audit.log` ou aux archives `.1..5` — un disque plein (`ENOSPC`)
    /// laisse le journal intact sans déclencher le verrouillage `[H-05]`.
    fn rotate(&mut self, max_bytes: u64, now: u64) -> Result<(), String> {
        let taille = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        if taille < max_bytes {
            return Ok(());
        }
        let entete = Entry {
            ts: now,
            keyid: "-",
            fp: "-",
            ip: "-",
            method: "-",
            tool: "-",
            verdict: "segment",
            reason: "rotation",
        };
        let seq = self.seq + 1;
        let (ligne, mac) = ligne_complete(seq, &entete, &self.last, &self.cle);
        let new_tmp = PathBuf::from(format!(
            "{}.{}.rotate.tmp",
            self.path.display(),
            std::process::id()
        ));
        let stage_res = (|| -> Result<(), String> {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&new_tmp)
                .map_err(|e| format!("ouverture {} impossible : {e}", new_tmp.display()))?;
            let _ = crate::manage::restrict_permissions(&new_tmp.to_string_lossy());
            writeln!(f, "{ligne}")
                .map_err(|e| format!("écriture {} impossible : {e}", new_tmp.display()))?;
            f.sync_all()
                .map_err(|e| format!("sync {} impossible : {e}", new_tmp.display()))?;
            Ok(())
        })();
        if let Err(e) = stage_res {
            let _ = std::fs::remove_file(&new_tmp);
            return Err(e);
        }
        let evince = segment_path(&self.path, ROTATE_KEEP);
        if evince.exists() {
            if let Some((seq_evince, mac_evince)) = derniere_ligne(&evince, Some(&self.cle))? {
                if let Err(e) =
                    write_head(&head_path(&self.path), seq_evince, &mac_evince, &self.cle)
                {
                    let _ = std::fs::remove_file(&new_tmp);
                    return Err(e);
                }
            }
        }
        for i in (1..ROTATE_KEEP).rev() {
            let from = segment_path(&self.path, i);
            if from.exists() {
                let to = segment_path(&self.path, i + 1);
                // `.5` est écrasé : cinq segments au plus (§11).
                let _ = std::fs::remove_file(&to);
                if let Err(e) = std::fs::rename(&from, &to) {
                    let _ = std::fs::remove_file(&new_tmp);
                    return Err(format!(
                        "rotation {} -> {} impossible : {e}",
                        from.display(),
                        to.display()
                    ));
                }
            }
        }
        let seg1 = segment_path(&self.path, 1);
        if let Err(e) = std::fs::rename(&self.path, &seg1) {
            let _ = std::fs::remove_file(&new_tmp);
            return Err(format!("rotation {} impossible : {e}", self.path.display()));
        }
        if let Err(e) = std::fs::rename(&new_tmp, &self.path) {
            let _ = std::fs::rename(&seg1, &self.path);
            let _ = std::fs::remove_file(&new_tmp);
            return Err(format!(
                "activation {} impossible : {e}",
                self.path.display()
            ));
        }
        self.seq = seq;
        self.last = mac;
        Ok(())
    }

    fn append(&mut self, e: &Entry<'_>, max_bytes: u64) -> Result<(), String> {
        self.rotate(max_bytes, e.ts)?;
        let seq = self.seq + 1;
        let (ligne, mac) = ligne_complete(seq, e, &self.last, &self.cle);
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("dossier {} impossible : {e}", parent.display()))?;
            }
        }
        let nouveau = !self.path.exists();
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("ouverture {} impossible : {e}", self.path.display()))?;
        if nouveau {
            let _ = crate::manage::restrict_permissions(&self.path.to_string_lossy());
        }
        f.write_all(ligne.as_bytes())
            .and_then(|()| f.write_all(b"\n"))
            .map_err(|e| format!("écriture {} impossible : {e}", self.path.display()))?;
        self.seq = seq;
        self.last = mac;
        Ok(())
    }
}

// ---------- état du process ----------

struct Global {
    path: PathBuf,
    journal: Option<Journal>,
    /// Dernier message d'erreur journalisé (anti-spam quand le disque refuse).
    derniere_erreur: Option<(u64, String)>,
}

static GLOBAL: OnceLock<Mutex<Option<Global>>> = OnceLock::new();

fn global() -> &'static Mutex<Option<Global>> {
    GLOBAL.get_or_init(|| Mutex::new(None))
}

/// Écrit une décision. Ne panique jamais ; une écriture impossible est
/// signalée au journal applicatif (au plus une fois par minute) mais ne bloque
/// jamais la requête en cours — la décision d'accès, elle, est déjà prise par
/// le vérificateur.
pub fn record(state: &Arc<AppState>, e: &Entry<'_>) {
    let path = path_for(&state.config_path);
    let dir = Path::new(&state.config_path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut erreur: Option<String> = None;
    let mut alerte_repli: Option<String> = None;
    {
        let mut g = global().lock().unwrap_or_else(|p| p.into_inner());
        let entry = g.get_or_insert_with(|| Global {
            path: path.clone(),
            journal: None,
            derniere_erreur: None,
        });
        if entry.path != path {
            entry.path = path.clone();
            entry.journal = None;
        }
        if entry.journal.is_none() {
            // Clé chargée UNE fois par processus et par fichier : le trousseau
            // n'est pas interrogé à chaque décision.
            match crate::auth::secret::load_or_create_audit_key(
                &dir,
                crate::auth::secret::service().as_deref(),
            ) {
                Ok(loaded) => {
                    if loaded.store == crate::auth::secret::SecretStore::File {
                        let verbe = if loaded.created { "créée" } else { "lue" };
                        alerte_repli = Some(format!(
                            "[audit] clé HMAC du journal {verbe} depuis un fichier à permissions restreintes (trousseau indisponible) : {} — protection moindre",
                            crate::auth::secret::audit_key_path(&dir).display()
                        ));
                    }
                    match Journal::open(path.clone(), loaded.key) {
                        Ok(j) => entry.journal = Some(j),
                        Err(err) => erreur = Some(err),
                    }
                }
                Err(err) => erreur = Some(err),
            }
        }
        if erreur.is_none() {
            if let Some(j) = entry.journal.as_mut() {
                if let Err(err) = j.append(e, ROTATE_BYTES) {
                    erreur = Some(err);
                }
            }
        }
        if erreur.is_some() {
            entry.journal = None;
        }
    }
    if let Some(msg) = alerte_repli {
        crate::manage::add_log(state, "warn", &msg);
    }
    if let Some(err) = erreur {
        signaler(state, err);
    }
}

fn signaler(state: &Arc<AppState>, message: String) {
    let now = crate::types::now_ts();
    let mut g = global().lock().unwrap_or_else(|p| p.into_inner());
    let entry = g.get_or_insert_with(|| Global {
        path: PathBuf::new(),
        journal: None,
        derniere_erreur: None,
    });
    if let Some((ts, _)) = &entry.derniere_erreur {
        if now.saturating_sub(*ts) < 60 {
            return;
        }
    }
    entry.derniere_erreur = Some((now, message.clone()));
    drop(g);
    crate::manage::add_log(
        state,
        "error",
        &format!("[audit] journal d'audit chaîné : {message}"),
    );
}

// ---------- décision du fil courant (hook C4 → transport) ----------

/// Dernière décision d'autorisation du fil, posée par le hook `C4`
/// (`auth::confirm::note_decision`) et lue par le transport, seul à connaître
/// l'adresse source et le `keyid` **revendiqué**.
#[derive(Clone, Debug)]
pub struct Decision {
    pub verdict: &'static str,
    pub reason: String,
    pub keyid: Option<String>,
    pub method: String,
    pub tool: String,
}

thread_local! {
    static COURANTE: std::cell::RefCell<Option<Decision>> =
        const { std::cell::RefCell::new(None) };
}

/// Remet la décision du fil à zéro avant un dispatch `/mcp`.
pub fn begin_mcp_request() {
    COURANTE.with(|c| *c.borrow_mut() = None);
}

/// Pose la décision du fil (appelée par le hook C4).
pub fn note(verdict: &'static str, reason: &str, keyid: Option<&str>, method: &str, tool: &str) {
    COURANTE.with(|c| {
        *c.borrow_mut() = Some(Decision {
            verdict,
            reason: reason.to_string(),
            keyid: keyid.map(String::from),
            method: method.to_string(),
            tool: tool.to_string(),
        });
    });
}

pub(crate) fn take() -> Option<Decision> {
    COURANTE.with(|c| c.borrow_mut().take())
}

/// `keyid` **revendiqué** par une requête, lu dans `Signature-Input` sans rien
/// vérifier : c'est une donnée publique, utile au journal et au compteur
/// d'échecs par clé. Jamais utilisé pour autoriser quoi que ce soit.
pub fn claimed_keyid(headers: &[(String, String)]) -> Option<String> {
    let mut vals = headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case("signature-input"))
        .map(|(_, v)| v.as_str());
    let v = vals.next()?;
    // Un en-tête dupliqué sera refusé en 401 : on n'impute pas un keyid ambigu.
    if vals.next().is_some() {
        return None;
    }
    // Les paramètres suivent la liste des composants `(...)` : on cherche
    // `keyid="..."` après la parenthèse fermante.
    let params = v.find(')').map(|pos| &v[pos + 1..]).unwrap_or(v);
    let i = params.rfind("keyid=\"")? + "keyid=\"".len();
    let fin = params[i..].find('"')?;
    let keyid = &params[i..i + fin];
    crate::auth::registry::valid_keyid(keyid).then(|| keyid.to_string())
}

/// Branchement du journal dans le transport : une ligne par décision d'accès
/// sur `/mcp`, accordée ou refusée. Les refus de **forme** (405, 415, 406,
/// 400) ne sont pas des décisions d'accès et n'écrivent rien.
pub fn note_http(
    state: &Arc<AppState>,
    headers: &[(String, String)],
    _method: &str,
    status: u16,
    peer: Option<IpAddr>,
    keyid_revendique: Option<&str>,
) {
    let decision = take();
    if status != 401 && decision.is_none() {
        return;
    }
    let verdict = decision
        .as_ref()
        .map(|d| d.verdict)
        .unwrap_or("unauthenticated");
    let reason = decision
        .as_ref()
        .map(|d| d.reason.as_str())
        .unwrap_or("unauthorized");
    let entete = |nom: &str| {
        headers
            .iter()
            .find(|(n, _)| n == nom)
            .map(|(_, v)| v.as_str())
    };
    let mcp_method = decision
        .as_ref()
        .map(|d| d.method.as_str())
        .filter(|m| !m.is_empty())
        .or_else(|| entete("mcp-method"))
        .unwrap_or("-");
    let tool = decision
        .as_ref()
        .map(|d| d.tool.as_str())
        .filter(|t| !t.is_empty() && *t != "-")
        .or_else(|| entete("mcp-name"))
        .unwrap_or("-");
    let keyid = decision
        .as_ref()
        .and_then(|d| d.keyid.as_deref())
        .or(keyid_revendique)
        .unwrap_or("-");
    // Empreinte : lue au registre, jamais recalculée depuis le réseau.
    let fp = if keyid == "-" {
        "-".to_string()
    } else {
        let settings = lockv(&state.settings);
        settings
            .auth
            .find(keyid)
            .and_then(|c| crate::auth::registry::short_fingerprint_of(&c.public_key))
            .unwrap_or_else(|| "-".to_string())
    };
    let ip = peer
        .map(|p| p.to_string())
        .unwrap_or_else(|| "-".to_string());
    let entry = Entry {
        ts: crate::types::now_ts(),
        keyid,
        fp: &fp,
        ip: &ip,
        method: mcp_method,
        tool,
        verdict,
        reason,
    };
    record(state, &entry);
}

// ---------- vérification ----------

/// Ce que la vérification a lu quand tout est cohérent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rapport {
    pub lignes: u64,
    pub segments: usize,
    pub premier_seq: u64,
    pub dernier_seq: u64,
    /// La chaîne commence au milieu d'une histoire déjà tournée (segments
    /// antérieurs supprimés) : l'intégrité est vérifiée de ce point à la fin.
    pub partiel: bool,
}

/// Nature d'une falsification détectée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Falsification {
    /// Une ligne a été réécrite : son `mac` ne se recalcule plus, ou son `prev`
    /// ne correspond plus.
    Modifiee { fichier: String, ligne: usize },
    /// Une ligne manque : la numérotation saute.
    Supprimee {
        fichier: String,
        ligne: usize,
        attendu: u64,
        trouve: u64,
    },
    /// Une ligne a été ajoutée : la numérotation recule ou se répète.
    Inseree {
        fichier: String,
        ligne: usize,
        attendu: u64,
        trouve: u64,
    },
    /// Une ligne est illisible (format inconnu).
    Illisible { fichier: String, ligne: usize },
}

impl Falsification {
    /// Message d'une ligne, pour `verify-log` et les tests.
    pub fn message(&self) -> String {
        match self {
            Falsification::Modifiee { fichier, ligne } => {
                format!("ligne MODIFIÉE dans {fichier}:{ligne} — le MAC ne se recalcule pas")
            }
            Falsification::Supprimee {
                fichier,
                ligne,
                attendu,
                trouve,
            } => format!(
                "ligne SUPPRIMÉE avant {fichier}:{ligne} — seq {attendu} attendu, {trouve} trouvé"
            ),
            Falsification::Inseree {
                fichier,
                ligne,
                attendu,
                trouve,
            } => format!(
                "ligne INSÉRÉE avant {fichier}:{ligne} — seq {attendu} attendu, {trouve} trouvé"
            ),
            Falsification::Illisible { fichier, ligne } => {
                format!("ligne ILLISIBLE dans {fichier}:{ligne} — format du journal inconnu")
            }
        }
    }
}

struct LigneLue<'a> {
    seq: u64,
    prev: &'a str,
    mac: &'a str,
}

fn is_lower_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn parse_ligne(l: &str) -> Option<LigneLue<'_>> {
    let seq = l
        .strip_prefix("seq=")?
        .split(' ')
        .next()?
        .parse::<u64>()
        .ok()?;
    let i_prev = l.find(" prev=")? + " prev=".len();
    let i_mac = l.rfind(" mac=")?;
    if i_prev + 64 != i_mac || i_mac + " mac=".len() + 64 != l.len() {
        return None;
    }
    let prev = &l[i_prev..i_prev + 64];
    let mac = &l[i_mac + " mac=".len()..];
    if !is_lower_hex64(prev) || !is_lower_hex64(mac) {
        return None;
    }
    Some(LigneLue { seq, prev, mac })
}

/// Fichiers d'un journal, du plus ancien au plus récent.
fn segments(path: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = (1..=ROTATE_KEEP)
        .rev()
        .map(|i| segment_path(path, i))
        .filter(|p| p.exists())
        .collect();
    if path.exists() {
        out.push(path.to_path_buf());
    }
    out
}

/// Recalcule toute la chaîne. `Ok` = intacte ; `Err` = première
/// falsification, avec fichier et ligne.
pub fn verify(path: &Path, cle: &[u8]) -> Result<Rapport, Falsification> {
    let fichiers = segments(path);
    let head = head_path(path);
    let (mut attendu_seq, mut attendu_prev, mut premier_seq, partiel) = if head.exists() {
        let (hseq, hmac) = read_head(&head, cle)?;
        (hseq, hmac, hseq + 1, true)
    } else {
        (0u64, ZERO_MAC.to_string(), 1u64, false)
    };
    if !path.exists() && (!fichiers.is_empty() || head.exists()) {
        return Err(Falsification::Supprimee {
            fichier: path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| AUDIT_FILE.to_string()),
            ligne: 1,
            attendu: attendu_seq + 1,
            trouve: 0,
        });
    }
    let mut lignes = 0u64;
    let mut dernier_seq = 0u64;
    let mut premier_vu = false;
    for fichier in &fichiers {
        let nom = fichier
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| fichier.display().to_string());
        let texte = match std::fs::read_to_string(fichier) {
            Ok(t) => t,
            Err(_) => {
                return Err(Falsification::Illisible {
                    fichier: nom,
                    ligne: 1,
                })
            }
        };
        let mut lignes_fichier = 0usize;
        for (n, l) in texte.lines().filter(|l| !l.trim().is_empty()).enumerate() {
            lignes_fichier += 1;
            let numero = n + 1;
            let lue = match parse_ligne(l) {
                Some(x) => x,
                None => {
                    return Err(Falsification::Illisible {
                        fichier: nom,
                        ligne: numero,
                    })
                }
            };
            // 1. Numérotation : exactement +1 par ligne.
            if lue.seq != attendu_seq + 1 {
                return Err(if lue.seq > attendu_seq + 1 {
                    Falsification::Supprimee {
                        fichier: nom,
                        ligne: numero,
                        attendu: attendu_seq + 1,
                        trouve: lue.seq,
                    }
                } else {
                    Falsification::Inseree {
                        fichier: nom,
                        ligne: numero,
                        attendu: attendu_seq + 1,
                        trouve: lue.seq,
                    }
                });
            }
            // 2. Le `prev` annoncé est celui de la ligne précédente (ou du fichier head).
            if lue.prev != attendu_prev {
                return Err(Falsification::Modifiee {
                    fichier: nom,
                    ligne: numero,
                });
            }
            // 3. Le MAC se recalcule sur le préfixe exact.
            let i_mac = l.rfind(" mac=").unwrap_or(0);
            let recalcule = mac_de(cle, &l[..i_mac + 1]);
            if !crate::crypto::ct_eq(recalcule.as_bytes(), lue.mac.as_bytes()) {
                return Err(Falsification::Modifiee {
                    fichier: nom,
                    ligne: numero,
                });
            }
            if !premier_vu {
                premier_seq = lue.seq;
                premier_vu = true;
            }
            attendu_seq = lue.seq;
            attendu_prev = lue.mac.to_string();
            dernier_seq = lue.seq;
            lignes += 1;
        }
        if fichiers.len() > 1 && lignes_fichier == 0 {
            return Err(Falsification::Illisible {
                fichier: nom,
                ligne: 1,
            });
        }
    }
    if lignes == 0 {
        if partiel {
            return Err(Falsification::Supprimee {
                fichier: path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| AUDIT_FILE.to_string()),
                ligne: 1,
                attendu: premier_seq,
                trouve: 0,
            });
        }
        premier_seq = 0;
    }
    Ok(Rapport {
        lignes,
        segments: fichiers.len(),
        premier_seq,
        dernier_seq,
        partiel,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static DIR_SEQ: AtomicU64 = AtomicU64::new(0);

    fn dir(nom: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "mcc-audit-{}-{}-{}-{}",
            std::process::id(),
            crate::types::now_ts(),
            DIR_SEQ.fetch_add(1, Ordering::Relaxed),
            nom
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const CLE: &[u8] = b"cle-de-test-journal-audit-32o";

    fn entree(ts: u64, verdict: &'static str, tool: &'static str) -> Entry<'static> {
        Entry {
            ts,
            keyid: "mcc-ai-3f9a1c07b2d84e51",
            fp: "ed25519:3f9a1c07b2d84e51",
            ip: "192.0.2.7",
            method: "tools/call",
            tool,
            verdict,
            reason: "-",
        }
    }

    /// Journal de référence : cinq lignes, clé fixe, même chemin que le vrai.
    fn journal(path: &Path, n: usize) {
        let mut j = Journal::open(path.to_path_buf(), CLE.to_vec()).expect("ouverture");
        for i in 0..n {
            let e = if i % 2 == 0 {
                entree(1_760_000_000 + i as u64, "allowed", "list_servers")
            } else {
                Entry {
                    ts: 1_760_000_000 + i as u64,
                    keyid: "mcc-ai-3f9a1c07b2d84e51",
                    fp: "ed25519:3f9a1c07b2d84e51",
                    ip: "192.0.2.7",
                    method: "tools/call",
                    tool: "start_server",
                    verdict: "approval_pending",
                    reason: "approval_pending",
                }
            };
            j.append(&e, ROTATE_BYTES).expect("append");
        }
    }

    fn lire(path: &Path) -> Vec<String> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(String::from)
            .collect()
    }

    fn ecrire(path: &Path, lignes: &[String]) {
        std::fs::write(path, format!("{}\n", lignes.join("\n"))).unwrap();
    }

    #[test]
    fn journal_intact_verifie_et_chaine_les_macs() {
        let d = dir("intact");
        let path = d.join(AUDIT_FILE);
        journal(&path, 5);
        let lignes = lire(&path);
        assert_eq!(lignes.len(), 5);
        assert!(lignes[0].contains(&format!("prev={ZERO_MAC}")));
        for i in 1..lignes.len() {
            let precedent = parse_ligne(&lignes[i - 1]).unwrap().mac.to_string();
            assert!(
                lignes[i].contains(&format!("prev={precedent}")),
                "ligne {i} : prev ne suit pas le mac précédent"
            );
        }
        let r = verify(&path, CLE).expect("journal intact");
        assert_eq!(r.lignes, 5);
        assert_eq!(r.premier_seq, 1);
        assert_eq!(r.dernier_seq, 5);
        assert!(!r.partiel);
        // aucun argument, aucun secret : le format ne porte que le NOM de l'outil
        assert!(!lignes[1].contains("arguments"));
        assert!(!lignes[1].contains("nonce"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Falsification 1/3 : une ligne réécrite (le refus devient un accord).
    #[test]
    fn ligne_modifiee_est_detectee() {
        let d = dir("modifiee");
        let path = d.join(AUDIT_FILE);
        journal(&path, 5);
        let mut lignes = lire(&path);
        lignes[2] = lignes[2].replace("tool=list_servers", "tool=start_server");
        ecrire(&path, &lignes);
        let err = verify(&path, CLE).expect_err("une ligne modifiée doit être détectée");
        assert!(
            matches!(err, Falsification::Modifiee { ligne: 3, .. }),
            "{err:?}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Falsification 2/3 : une ligne supprimée au milieu.
    #[test]
    fn ligne_supprimee_est_detectee() {
        let d = dir("supprimee");
        let path = d.join(AUDIT_FILE);
        journal(&path, 5);
        let mut lignes = lire(&path);
        lignes.remove(2);
        ecrire(&path, &lignes);
        let err = verify(&path, CLE).expect_err("une ligne supprimée doit être détectée");
        assert!(
            matches!(err, Falsification::Supprimee { ligne: 3, .. }),
            "{err:?}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Falsification 2b/3 : suppression de la toute première ligne détectée.
    #[test]
    fn premiere_ligne_supprimee_est_detectee() {
        let d = dir("suppr1");
        let path = d.join(AUDIT_FILE);
        journal(&path, 5);
        let mut lignes = lire(&path);
        lignes.remove(0);
        ecrire(&path, &lignes);
        let err = verify(&path, CLE).expect_err("la suppression de ligne 1 doit être détectée");
        assert!(
            matches!(
                err,
                Falsification::Supprimee {
                    ligne: 1,
                    attendu: 1,
                    trouve: 2,
                    ..
                }
            ),
            "{err:?}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Falsification 3/3 : une ligne insérée (numérotation qui se répète).
    #[test]
    fn ligne_inseree_est_detectee() {
        let d = dir("inseree");
        let path = d.join(AUDIT_FILE);
        journal(&path, 5);
        let mut lignes = lire(&path);
        let copie = lignes[1].clone();
        lignes.insert(2, copie);
        ecrire(&path, &lignes);
        let err = verify(&path, CLE).expect_err("une ligne insérée doit être détectée");
        assert!(
            matches!(
                err,
                Falsification::Inseree {
                    ligne: 3,
                    attendu: 3,
                    trouve: 2,
                    ..
                }
            ),
            "{err:?}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Rotation : le nouveau segment commence par un en-tête dont le `prev`
    /// est le `mac` de la dernière ligne de l'ancien — la chaîne continue.
    #[test]
    fn rotation_garde_la_chaine_sur_les_segments() {
        let d = dir("rotation");
        let path = d.join(AUDIT_FILE);
        let mut j = Journal::open(path.clone(), CLE.to_vec()).unwrap();
        // Seuil minuscule : chaque ligne (~250 octets) déclenche une rotation.
        for i in 0..4 {
            j.append(&entree(1_760_000_000 + i, "allowed", "list_servers"), 200)
                .unwrap();
        }
        assert!(segment_path(&path, 1).exists(), "segment .1 attendu");
        let r = verify(&path, CLE).expect("chaîne intacte après rotation");
        // 4 ajouts + 3 rotations (chacune ajoute une ligne `verdict=segment`).
        assert_eq!(r.lignes, 7);
        assert_eq!(r.dernier_seq, 7);
        assert!(!r.partiel);
        assert!(r.segments >= 2, "segments lus : {}", r.segments);
        // l'en-tête du segment courant prolonge l'ancien
        let courant = lire(&path);
        assert!(
            courant[0].contains("verdict=segment"),
            "en-tête de segment attendu : {}",
            courant[0]
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Rotation au-delà des 5 segments conservés : `.5` est évincé et son
    /// dernier `(seq, mac)` passe dans `auth-audit.head` — la chaîne reste
    /// vérifiable (`partiel = true`), mais supprimer un segment intermédiaire
    /// ou falsifier `.head` échoue.
    #[test]
    fn segments_anciens_supprimes_chaine_partielle_mais_verifiable() {
        let d = dir("partiel");
        let path = d.join(AUDIT_FILE);
        let mut j = Journal::open(path.clone(), CLE.to_vec()).unwrap();
        for i in 0..8 {
            j.append(&entree(1_760_000_000 + i, "allowed", "list_servers"), 200)
                .unwrap();
        }
        assert!(
            head_path(&path).exists(),
            "fichier .head attendu après >5 rotations"
        );
        let r = verify(&path, CLE).expect("reprise de chaîne via .head");
        assert!(r.partiel, "chaîne partielle attendue : {r:?}");
        assert_eq!(r.segments, ROTATE_KEEP + 1);
        assert_eq!(r.dernier_seq, 15);

        // Supprimer un segment intermédiaire (.1) DOIT casser la chaîne.
        let _ = std::fs::remove_file(segment_path(&path, 1));
        assert!(
            matches!(verify(&path, CLE), Err(Falsification::Supprimee { .. })),
            "la suppression d'un segment intermédiaire doit être détectée"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn head_falsifie_est_detecte() {
        let d = dir("head_tamper");
        let path = d.join(AUDIT_FILE);
        let mut j = Journal::open(path.clone(), CLE.to_vec()).unwrap();
        for i in 0..8 {
            j.append(&entree(1_760_000_000 + i, "allowed", "list_servers"), 200)
                .unwrap();
        }
        let hp = head_path(&path);
        let txt = std::fs::read_to_string(&hp).unwrap();
        std::fs::write(&hp, txt.replace("seq=3", "seq=4")).unwrap();
        assert!(matches!(
            verify(&path, CLE),
            Err(Falsification::Modifiee { .. })
        ));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn champs_du_journal_ne_peuvent_pas_injecter_une_ligne() {
        let e = Entry {
            ts: 1_760_000_000,
            keyid: "mcc-a-0000000000000001",
            fp: "ed25519:3f9a1c07b2d84e51",
            ip: "192.0.2.7",
            method: "tools/call",
            tool: "start_server\nseq=999999 verdict=allowed",
            verdict: "refusé",
            reason: "key_unknown",
        };
        let (ligne, _) = ligne_complete(1, &e, ZERO_MAC, CLE);
        assert_eq!(ligne.lines().count(), 1, "une seule ligne physique");
        assert!(!ligne.contains('\n'));
        assert!(ligne.contains("tool=start_server_seq_999999_verdict_allowed"));
        assert!(
            ligne.contains("verdict=refus_"),
            "non-ASCII remplacé : {ligne}"
        );
    }

    #[test]
    fn keyid_revendique_lu_sans_verification() {
        let h = vec![(
            "signature-input".to_string(),
            "sig1=(\"@method\" \"@path\");created=1;nonce=\"x\";keyid=\"mcc-ai-3f9a1c07b2d84e51\";alg=\"ed25519\";tag=\"litemcp-auth-v1\"".to_string(),
        )];
        assert_eq!(
            claimed_keyid(&h).as_deref(),
            Some("mcc-ai-3f9a1c07b2d84e51")
        );
        let mauvais = vec![(
            "signature-input".to_string(),
            "sig1=();keyid=\"../../etc/passwd\"".to_string(),
        )];
        assert_eq!(claimed_keyid(&mauvais), None);
        let doublon = vec![
            (
                "Signature-Input".to_string(),
                "sig1=(\"@method\");created=1;nonce=\"x\";keyid=\"mcc-ai-3f9a1c07b2d84e51\";alg=\"ed25519\";tag=\"litemcp-auth-v1\"".to_string(),
            ),
            (
                "signature-input".to_string(),
                "sig1=(\"@method\");created=1;nonce=\"y\";keyid=\"mcc-ai-0000000000000002\";alg=\"ed25519\";tag=\"litemcp-auth-v1\"".to_string(),
            ),
        ];
        assert_eq!(claimed_keyid(&doublon), None);
        assert_eq!(claimed_keyid(&[]), None);
    }

    #[test]
    fn write_head_est_atomique_sans_residu_tmp_et_lisible() {
        let d = dir("r8-head");
        let head = d.join("auth-audit.head");
        let tmp = head.with_extension("head.tmp");
        let mac = "a".repeat(64);
        write_head(&head, 42, &mac, CLE).expect("write_head");
        assert!(head.exists(), "auth-audit.head attendu");
        assert!(!tmp.exists(), "aucun résidu .head.tmp ne doit subsister");
        let (seq, read_mac) = read_head(&head, CLE).expect("read_head");
        assert_eq!(seq, 42);
        assert_eq!(read_mac, mac);
        // Second appel (écrasement atomique sur rotation suivante)
        let mac2 = "b".repeat(64);
        write_head(&head, 84, &mac2, CLE).expect("second write_head");
        assert!(!tmp.exists());
        let (seq2, read_mac2) = read_head(&head, CLE).expect("second read_head");
        assert_eq!(seq2, 84);
        assert_eq!(read_mac2, mac2);
        let _ = std::fs::remove_dir_all(&d);
    }
}
