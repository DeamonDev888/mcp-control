//! Synchronisation légère des logos SVG officiels de **Swarm-control**
//! déclenchée uniquement lors d'un changement de version MCP détecté
//! (`serverInfo.version`) ou sur demande manuelle (`swarm_sync_logos`).
//!
//! - **0 trafic réseau au repos** : tant que la version MCP annoncée par
//!   `swarm-control` correspond à celle en cache (`version.txt`), aucune
//!   requête GitHub n'est émise.
//! - **Ultra-léger** : `curl` système (zéro nouvelle dépendance), en-têtes
//!   `If-None-Match` / `ETag` (`304 Not Modified` = 0 octet sur les logos
//!   inchangés), cache mémoire `RwLock` (0 accès disque sur les requêtes UI).
//! - **Sécurité stricte** : liste blanche des 12 slugs, HTTPS TLS ≥ 1.2,
//!   plafond 600 Kio par fichier, validation SVG anti-XSS (rejet `<script`,
//!   `javascript:`, `on*=`, `foreignObject`), écriture atomique `.tmp` → rename.

use crate::manage;
use crate::probe;
use crate::types::AppState;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

/// Version embarquée à la compilation (utilisée comme base avant la 1re sync).
pub const EMBEDDED_SWARM_VERSION: &str = "0.1.2";

/// Taille maximale acceptée pour un SVG téléchargé (600 Kio).
const MAX_SVG_BYTES: usize = 600 * 1024;

/// Table blanche des 12 logos officiels (slug → chemin relatif dans
/// `DeamonDev888/swarm-control@main`).
pub const LOGO_SLUGS: [(&str, &str); 12] = [
    ("logo", "assets/pixel/hornet-pixel-logo.svg"),
    ("coordinator", "assets/classes/hornet-pixel-coordinator.svg"),
    ("coder", "assets/classes/hornet-pixel-coder.svg"),
    ("explorer", "assets/classes/hornet-pixel-explorer.svg"),
    ("verifier", "assets/classes/hornet-pixel-verifier.svg"),
    ("monitor", "assets/classes/hornet-pixel-monitor.svg"),
    ("browser", "assets/classes/hornet-pixel-browser.svg"),
    ("discord", "assets/classes/hornet-pixel-discord.svg"),
    ("google", "assets/classes/hornet-pixel-google.svg"),
    ("whatsapp", "assets/classes/hornet-pixel-whatsapp.svg"),
    ("x", "assets/classes/hornet-pixel-x.svg"),
    ("custom", "assets/classes/hornet-pixel-custom.svg"),
];

const RAW_BASE: &str = "https://raw.githubusercontent.com/DeamonDev888/swarm-control/main";

static SYNC_RUNNING: AtomicBool = AtomicBool::new(false);
static LOGO_REV: AtomicU64 = AtomicU64::new(1);

fn mem_cache() -> &'static RwLock<HashMap<String, Vec<u8>>> {
    static CACHE: OnceLock<RwLock<HashMap<String, Vec<u8>>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn known_version_slot() -> &'static Mutex<Option<String>> {
    static SLOT: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Révision monotone des logos (exposée dans `/api/state` sous `swarm_logos_rev`
/// pour rafraîchir automatiquement les `<img src="/swarm-logo/...svg?v=...">`).
pub fn revision() -> u64 {
    LOGO_REV.load(Ordering::Relaxed)
}

/// Dossier cache sur disque : `<config_dir>/cache/swarm-logos`.
pub fn cache_dir(config_path: &str) -> PathBuf {
    let base = Path::new(config_path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    base.join("cache").join("swarm-logos")
}

/// Valide un SVG téléchargé avant mise en cache (taille, balise racine `<svg`,
/// fermeture `</svg>`, absence de vecteurs XSS).
pub fn validate_svg(bytes: &[u8]) -> Result<(), &'static str> {
    if bytes.is_empty() || bytes.len() > MAX_SVG_BYTES {
        return Err("taille SVG hors bornes");
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "SVG non UTF-8")?;
    let trimmed = text.trim();
    let body = if let Some(rest) = trimmed.strip_prefix("<?xml") {
        let end = rest.find("?>").ok_or("prologue XML invalide")?;
        rest[end + 2..].trim()
    } else {
        trimmed
    };
    if !body.starts_with("<svg") || !body.ends_with("</svg>") {
        return Err("balise racine <svg>...</svg> absente");
    }
    let low = body.to_ascii_lowercase();
    for forbidden in [
        "<script",
        "javascript:",
        "vbscript:",
        "<foreignobject",
        "onload=",
        "onerror=",
        "onclick=",
        "onmouseover=",
    ] {
        if low.contains(forbidden) {
            return Err("contenu SVG interdit");
        }
    }
    Ok(())
}

/// Charge les octets d'un logo officiel (priorité : cache mémoire → cache
/// disque validé → SVG embarqué à la compilation).
pub fn load_svg_bytes(config_path: &str, slug: &str) -> Option<Vec<u8>> {
    // Vérifie la liste blanche avant tout accès.
    let embedded = crate::ui::swarm_logo_svg(slug)?;
    if let Ok(guard) = mem_cache().read() {
        if let Some(bytes) = guard.get(slug) {
            return Some(bytes.clone());
        }
    }
    let disk_path = cache_dir(config_path).join(format!("{slug}.svg"));
    if let Ok(bytes) = std::fs::read(&disk_path) {
        if validate_svg(&bytes).is_ok() {
            if let Ok(mut guard) = mem_cache().write() {
                guard.insert(slug.to_string(), bytes.clone());
            }
            return Some(bytes);
        }
    }
    Some(embedded.as_bytes().to_vec())
}

fn read_cached_version(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("version.txt"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| EMBEDDED_SWARM_VERSION.to_string())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Appelée en O(1) dès qu'un serveur Swarm UP annonce sa version MCP.
/// Si la version diffère de celle déjà synchronisée, déclenche une sync
/// asynchrone en arrière-plan (désactivée en tests / `MCC_NO_LOCAL_DISCOVERY`).
pub fn notify_detected_version(state: &Arc<AppState>, detected_version: &str) {
    let ver = detected_version.trim();
    if ver.is_empty() || cfg!(test) {
        return;
    }
    if std::env::var("MCC_NO_LOCAL_DISCOVERY").as_deref() == Ok("1") {
        return;
    }
    let should_sync = {
        let mut slot = known_version_slot()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let current = slot
            .get_or_insert_with(|| read_cached_version(&cache_dir(&state.config_path)))
            .clone();
        if current == ver {
            false
        } else {
            *slot = Some(ver.to_string());
            true
        }
    };
    if !should_sync {
        return;
    }
    let st = Arc::clone(state);
    let ver_owned = ver.to_string();
    std::thread::spawn(move || {
        let rep = sync_now(&st, Some(&ver_owned));
        manage::add_log(
            &st,
            "info",
            &format!(
                "[swarm] nouvelle version v{} détectée — sync logos : {} mis à jour, {} inchangés, {} erreur(s)",
                ver_owned, rep.updated, rep.unchanged, rep.errors
            ),
        );
    });
}

#[derive(Debug, Default)]
pub struct SyncReport {
    pub updated: usize,
    pub unchanged: usize,
    pub errors: usize,
    pub busy: bool,
    pub rev: u64,
}

type FetchedSvg = (Vec<u8>, Option<String>);

/// Télécharge un seul SVG avec support `If-None-Match` (ETag) :
/// `Ok(None)` = `304 Not Modified`, `Ok(Some((bytes, etag)))` = nouveau SVG.
fn fetch_svg_with_etag(
    url: &str,
    prev_etag: Option<&str>,
    dir: &Path,
    slug: &str,
) -> Result<Option<FetchedSvg>, String> {
    let hdr_path = dir.join(format!("{slug}.hdr.tmp"));
    let body_path = dir.join(format!("{slug}.body.tmp"));
    let _ = std::fs::remove_file(&hdr_path);
    let _ = std::fs::remove_file(&body_path);

    let mut c = probe::system_command("curl");
    probe::probe_hidden_cmd(&mut c);
    c.args([
        "-sSL",
        "--proto",
        "=https",
        "--tlsv1.2",
        "-m",
        "12",
        "--max-filesize",
        &MAX_SVG_BYTES.to_string(),
        "-H",
        "User-Agent: mcp-control",
        "-D",
    ])
    .arg(&hdr_path)
    .arg("-o")
    .arg(&body_path);
    if let Some(et) = prev_etag.filter(|s| !s.is_empty()) {
        c.arg("-H").arg(format!("If-None-Match: {et}"));
    }
    c.arg("-w").arg("%{http_code}").arg(url);

    let out = c.output().map_err(|e| format!("curl indisponible : {e}"))?;
    let code_s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let code: u16 = code_s.parse().unwrap_or(0);

    let new_etag = std::fs::read_to_string(&hdr_path).ok().and_then(|h| {
        h.lines().rev().find_map(|line| {
            let low = line.to_ascii_lowercase();
            low.strip_prefix("etag:")
                .and_then(|_| line.split_once(':').map(|(_, v)| v.trim().to_string()))
        })
    });
    let _ = std::fs::remove_file(&hdr_path);

    if code == 304 {
        let _ = std::fs::remove_file(&body_path);
        return Ok(None);
    }
    if code != 200 {
        let _ = std::fs::remove_file(&body_path);
        return Err(format!("HTTP {code}"));
    }
    let bytes = std::fs::read(&body_path).map_err(|e| format!("lecture tmp : {e}"))?;
    let _ = std::fs::remove_file(&body_path);
    validate_svg(&bytes).map_err(|e| e.to_string())?;
    Ok(Some((bytes, new_etag)))
}

/// Synchronise immédiatement les 12 logos depuis GitHub (avec ETag pour ne
/// télécharger que les fichiers modifiés). Protégé contre les appels concurrents.
pub fn sync_now(state: &Arc<AppState>, target_version: Option<&str>) -> SyncReport {
    if SYNC_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return SyncReport {
            busy: true,
            rev: revision(),
            ..Default::default()
        };
    }
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            SYNC_RUNNING.store(false, Ordering::SeqCst);
        }
    }
    let _guard = Guard;

    let dir = cache_dir(&state.config_path);
    if std::fs::create_dir_all(&dir).is_err() {
        return SyncReport {
            errors: LOGO_SLUGS.len(),
            rev: revision(),
            ..Default::default()
        };
    }

    let mut rep = SyncReport::default();
    for (slug, rel) in LOGO_SLUGS {
        let url = format!("{RAW_BASE}/{rel}");
        let etag_path = dir.join(format!("{slug}.etag"));
        let prev_etag = std::fs::read_to_string(&etag_path)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        match fetch_svg_with_etag(&url, prev_etag.as_deref(), &dir, slug) {
            Ok(None) => rep.unchanged += 1,
            Ok(Some((bytes, etag))) => {
                let svg_path = dir.join(format!("{slug}.svg"));
                if write_atomic(&svg_path, &bytes).is_ok() {
                    if let Some(et) = etag {
                        let _ = write_atomic(&etag_path, et.as_bytes());
                    }
                    if let Ok(mut guard) = mem_cache().write() {
                        guard.insert(slug.to_string(), bytes);
                    }
                    rep.updated += 1;
                } else {
                    rep.errors += 1;
                }
            }
            Err(_) => rep.errors += 1,
        }
    }

    if rep.errors == 0 {
        if let Some(v) = target_version.filter(|s| !s.trim().is_empty()) {
            let _ = write_atomic(&dir.join("version.txt"), v.trim().as_bytes());
            if let Ok(mut slot) = known_version_slot().lock() {
                *slot = Some(v.trim().to_string());
            }
        }
    }
    if rep.updated > 0 {
        LOGO_REV.fetch_add(1, Ordering::SeqCst);
    }
    rep.rev = revision();
    rep
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_svg_accepte_valide_et_rejette_xss() {
        let ok = b"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'><rect width='10' height='10'/></svg>";
        assert!(validate_svg(ok).is_ok());

        let xss = b"<svg><script>alert(1)</script></svg>";
        assert!(validate_svg(xss).is_err());

        let on = b"<svg onload='alert(1)'></svg>";
        assert!(validate_svg(on).is_err());

        let bad = b"<html><body>pas un svg</body></html>";
        assert!(validate_svg(bad).is_err());
    }

    #[test]
    fn repli_embarque_et_liste_blanche_stricte() {
        assert!(load_svg_bytes("inexistant/config.json", "logo").is_some());
        assert!(load_svg_bytes("inexistant/config.json", "coordinator").is_some());
        assert!(load_svg_bytes("inexistant/config.json", "../secret").is_none());
    }
}
