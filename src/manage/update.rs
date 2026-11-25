// Vérification de mise à jour (release GitHub) — au boot puis chaque heure.
// Assets de release : table blanche (jamais construits depuis la requête).
// (split v0.7.20 : manage.rs reste la racine — API stable via réexport)
use super::*;

pub fn version_newer(latest: &str, current: &str) -> bool {
    let parse = |v: &str| -> (Vec<u64>, Option<String>) {
        let trimmed = v.trim().trim_start_matches('v');
        let no_build = trimmed.split('+').next().unwrap_or(trimmed);
        let (core, pre) = match no_build.split_once('-') {
            Some((c, p)) => (c, Some(p.to_string())),
            None => (no_build, None),
        };
        let nums: Vec<u64> = core
            .split('.')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0))
            .collect();
        (nums, pre)
    };
    let ((a, a_pre), (b, b_pre)) = (parse(latest), parse(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    // SemVer §11 : à cœur égal, une version stable (sans pré-version) est plus
    // récente qu'une pré-version (`0.9.0 > 0.9.0-rc.1`).
    match (a_pre, b_pre) {
        (None, Some(_)) => true,
        (Some(_), None) => false,
        (Some(pa), Some(pb)) => pa > pb,
        (None, None) => false,
    }
}

/// Interroge la dernière release GitHub. `curl` d'abord (léger, présent sur
/// macOS/Linux/Windows 10+) avec repli PowerShell — l'ancien code n'utilisait
/// QUE PowerShell : la vérification était silencieusement inopérante partout
/// ailleurs (audit 2026-10-24). `Ok(Some(tag))` = tag lu ; `Ok(None)` = réponse
/// sans tag ; `Err` = réseau/API/outil indisponible — la distinction permet au
/// bouton « Vérifier » de ne plus prétendre « à jour » quand le check a raté.
pub(crate) fn github_latest_tag() -> Result<Option<String>, &'static str> {
    let mut c = crate::probe::system_command("curl");
    probe::probe_hidden_cmd(&mut c);
    if let Ok(out) = c
        .args([
            "-fsSL",
            "--proto",
            "=https",
            "--tlsv1.2",
            "-m",
            "10",
            "-H",
            "User-Agent: mcp-control",
            "https://api.github.com/repos/DeamonDev888/mcp-control/releases/latest",
        ])
        .output()
    {
        if out.status.success() {
            if let Ok(v) = serde_json::from_slice::<Value>(&out.stdout) {
                if let Some(tag) = v.get("tag_name").and_then(|x| x.as_str()) {
                    return Ok(Some(tag.to_string()));
                }
            }
        }
    }
    let mut cmd = crate::probe::system_command("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "(Invoke-RestMethod -TimeoutSec 10 -Headers @{ 'User-Agent' = 'mcp-control' }           'https://api.github.com/repos/DeamonDev888/mcp-control/releases/latest').tag_name",
    ]);
    probe::probe_hidden_cmd(&mut cmd);
    let Ok(out) = cmd.output() else {
        return Err("réseau injoignable (curl et PowerShell en échec)");
    };
    if !out.status.success() {
        return Err("réseau injoignable (curl et PowerShell en échec)");
    }
    let tag = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if tag.is_empty() || tag.len() > 30 {
        return Ok(None);
    }
    Ok(Some(tag))
}

/// Issue d'une vérification de release GitHub — distinguée pour que le bouton
/// « Vérifier » des réglages puisse dire « à jour (vérifié à l'instant) »,
/// « vX disponible » ou « vérification impossible (réseau ?) » au lieu du
/// silence d'avant, qui faisait croire à un clic mort.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCheck {
    /// Release plus récente lue et enregistrée dans update_latest.
    Available(String),
    /// API GitHub lue : rien de plus récent que la version installée.
    UpToDate,
    /// Réseau/API injoignable ou réponse illisible — aucun verdict.
    Unreachable,
}

/// Vérifie la dernière release GitHub : `update_latest` = tag SI plus récent
/// que l'installée ; vidé si l'installée redevient la dernière (sinon le badge
/// « mise à jour » resterait affiché avec une dispo périmée). Côté journal,
/// silence hors ligne — le poller horaire ne doit pas spammer : le verdict est
/// RENVOYÉ et l'action `check_update` le traduit pour l'UI.
pub fn check_update(state: &Arc<AppState>) -> UpdateCheck {
    // Le tag GitHub arrive sous la forme « v0.7.16 » : on range la version SANS
    // son préfixe. Tous les consommateurs (message du journal, latest_version de
    // /api/state, réponse check_update) affichent « v » + version — préfixer le
    // tag tel quel produisait « vv0.7.16 » dans le journal et le badge.
    let tag = match github_latest_tag() {
        Ok(Some(t)) => t,
        Ok(None) | Err(_) => return UpdateCheck::Unreachable,
    };
    let version = tag.strip_prefix('v').unwrap_or(&tag).to_string();
    let current = env!("CARGO_PKG_VERSION");
    if version_newer(&version, current) {
        // VERROUILLAGE (audit 2026-10-25) : ne PAS tenir update_latest pendant
        // add_log (qui prend le verrou log). state_json prend log PUIS
        // update_latest → un AB-BA était possible : check_update tenait
        // update_latest et attendait log pendant que state_json tenait log et
        // attendait update_latest. On libère update_latest AVANT de logger.
        let should_log = {
            let mut u = lockv(&state.update_latest);
            if u.as_deref() != Some(version.as_str()) {
                *u = Some(version.clone());
                true
            } else {
                false
            }
        };
        if should_log {
            add_log(
                state,
                "info",
                &format!(
                    "mise à jour disponible : v{} (installée : v{}) — github.com/DeamonDev888/mcp-control/releases",
                    version, current
                ),
            );
            // S-7 : le moteur n'exécute AUCUN installateur (il ouvre l'URL de
            // release dans l'OS) — il n'y a donc rien à vérifier avant
            // téléchargement. Ce qui manquait, c'est l'empreinte publiée : on
            // lit SHA256SUMS.txt de LA MÊME release et on la journalise, pour
            // que l'utilisateur puisse comparer à la main. Réseau borné et
            // facultatif : une somme absente ou un fichier illisible ne change
            // pas le verdict de mise à jour.
            if let Some(asset) = running_asset() {
                if let Ok(Sha256Lookup::Found(sum)) = expected_sha256(&tag, asset) {
                    add_log(
                        state,
                        "info",
                        &format!(
                            "empreinte SHA-256 de {asset} (v{version}) : {sum} — à comparer au fichier téléchargé (SHA256SUMS.txt de la release)"
                        ),
                    );
                }
            }
        }
        return UpdateCheck::Available(version);
    }
    // L'installée redevient la dernière : on vide le badge.
    {
        let mut u = lockv(&state.update_latest);
        if u.is_some() {
            *u = None;
        }
    }
    UpdateCheck::UpToDate
}

pub fn release_asset(kind: &str, os: &str) -> Option<&'static str> {
    match (kind, os) {
        ("desktop", "windows") => Some("mcp-control-desktop-windows-x86_64-setup.exe"),
        ("desktop", "macos") => Some("mcp-control-desktop-macos-universal"),
        ("desktop", "linux") => Some("mcp-control-desktop-linux-x86_64"),
        ("cli", "windows") => Some("mcp-control-cli-windows-x86_64.exe"),
        ("cli", "macos") => Some("mcp-control-cli-macos-universal"),
        ("cli", "linux") => Some("mcp-control-cli-linux-x86_64"),
        _ => None,
    }
}

/// Actif CLI publié pour un OS + architecture distants (`ssh_install_cli`).
/// Table blanche stricte : une architecture sans livrable CLI publié renvoie `None`.
pub fn cli_release_asset(os: &str, arch: &str) -> Option<&'static str> {
    let os_norm = os.trim().to_ascii_lowercase();
    let arch_norm = arch.trim().to_ascii_lowercase();
    match (os_norm.as_str(), arch_norm.as_str()) {
        ("linux", "x86_64" | "amd64") => Some("mcp-control-cli-linux-x86_64"),
        ("macos" | "darwin", "x86_64" | "arm64" | "aarch64") => {
            Some("mcp-control-cli-macos-universal")
        }
        ("windows", "x86_64" | "amd64") => Some("mcp-control-cli-windows-x86_64.exe"),
        _ => None,
    }
}

/// Résout `(tag, url, expected_sha256)` pour le binaire CLI le plus récent d'un OS/arch.
/// Refuse catégoriquement si le tag est hors gabarit ou si le SHA-256 est absent/malformé.
pub fn latest_cli_download(os: &str, arch: &str) -> Result<(String, String, String), String> {
    let asset = cli_release_asset(os, arch).ok_or_else(|| {
        format!("aucun binaire CLI publié pour {os}/{arch} (installation auto impossible)")
    })?;
    let tag = github_latest_tag()
        .map_err(|e| format!("release GitHub injoignable : {e}"))?
        .ok_or_else(|| "aucune release GitHub publiée trouvée".to_string())?;
    if !tag_url_safe(&tag) {
        return Err("tag de release GitHub hors gabarit".to_string());
    }
    let sha256 = match expected_sha256(&tag, asset) {
        Ok(Sha256Lookup::Found(s)) => s,
        Ok(Sha256Lookup::Absent) => {
            return Err(format!(
                "empreinte SHA-256 absente de SHA256SUMS.txt pour {asset} ({tag}) — refus d'installer"
            ));
        }
        Ok(Sha256Lookup::Malformed) => {
            return Err(format!(
                "empreinte SHA-256 malformée dans SHA256SUMS.txt pour {asset} ({tag}) — refus d'installer"
            ));
        }
        Err(e) => {
            return Err(format!(
                "SHA256SUMS.txt illisible pour {tag} ({e}) — refus d'installer sans vérification"
            ));
        }
    };
    let url =
        format!("https://github.com/DeamonDev888/mcp-control/releases/download/{tag}/{asset}");
    Ok((tag, url, sha256))
}

/// L'OS du binaire qui tourne, tel que nommé dans la table ci-dessus.
pub fn engine_os() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "windows"
    }
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "linux"
    }
    #[cfg(not(any(windows, target_os = "macos", all(unix, not(target_os = "macos")))))]
    {
        "inconnu"
    }
}

// ---------- Empreinte SHA-256 publiée (SHA256SUMS.txt) ----------
//
// S-7 (vérifié par l'audit exécuté) : le moteur ne télécharge aucun
// installateur — il ouvre l'URL de release dans l'OS. Il n'y a donc aucune
// somme à vérifier avant un téléchargement qui n'a pas lieu ici. Ce que le
// moteur peut faire, c'est rapporter l'empreinte PUBLIÉE pour que
// l'utilisateur la compare lui-même au fichier qu'il vient de récupérer. Le
// fichier de sommes est produit par .github/workflows/release.yml
// (`sha256sum * | sort -k2`), et signé (cosign keyless).

/// Nom de l'actif de sommes publié à côté des binaires.
pub const SHA256SUMS_ASSET: &str = "SHA256SUMS.txt";

/// Réponse à « quelle est l'empreinte attendue de cet actif ? ».
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sha256Lookup {
    /// Somme trouvée : 64 hexadécimaux, minuscules.
    Found(String),
    /// L'actif n'est pas listé dans le fichier.
    Absent,
    /// L'actif est listé mais la ligne est inutilisable (somme tronquée ou non
    /// hexadécimale, ou deux lignes contradictoires pour le même actif).
    Malformed,
}

/// Gabarit accepté pour un tag de release avant de le mettre dans une URL :
/// rien qui puisse ajouter un segment de chemin, une requête ou un fragment.
fn tag_url_safe(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 64
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// URL de SHA256SUMS.txt pour une release donnée ; `None` = tag hors gabarit.
pub fn sha256sums_url(tag: &str) -> Option<String> {
    if !tag_url_safe(tag) {
        return None;
    }
    Some(format!(
        "https://github.com/DeamonDev888/mcp-control/releases/download/{tag}/{SHA256SUMS_ASSET}"
    ))
}

/// Télécharge un petit fichier texte : `curl` d'abord (présent sur les trois
/// OS livrés), repli PowerShell (Windows sans curl). Plafonné : un
/// SHA256SUMS.txt fait quelques centaines d'octets.
fn fetch_text(url: &str) -> Result<String, &'static str> {
    const MAX: usize = 64 * 1024;
    let mut c = crate::probe::system_command("curl");
    probe::probe_hidden_cmd(&mut c);
    if let Ok(out) = c
        .args([
            "-fsSL",
            "--proto",
            "=https",
            "--tlsv1.2",
            "-m",
            "6",
            "-H",
            "User-Agent: mcp-control",
            url,
        ])
        .output()
    {
        if out.status.success() && out.stdout.len() <= MAX {
            return Ok(String::from_utf8_lossy(&out.stdout).to_string());
        }
    }
    // `url` est construit uniquement par `sha256sums_url` (tag validé) : aucune
    // donnée extérieure n'entre dans la commande.
    let mut cmd = crate::probe::system_command("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &format!(
            "(Invoke-WebRequest -UseBasicParsing -TimeoutSec 6 -Headers @{{ 'User-Agent' = 'mcp-control' }} '{url}').Content"
        ),
    ]);
    probe::probe_hidden_cmd(&mut cmd);
    let Ok(out) = cmd.output() else {
        return Err("réseau injoignable (curl et PowerShell en échec)");
    };
    if !out.status.success() || out.stdout.len() > MAX {
        return Err("réseau injoignable (curl et PowerShell en échec)");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Contenu de SHA256SUMS.txt de la release `tag`.
pub fn fetch_sha256sums(tag: &str) -> Result<String, &'static str> {
    let url = sha256sums_url(tag).ok_or("tag de release hors gabarit")?;
    fetch_text(&url)
}

fn hex64(s: &str) -> Option<String> {
    if s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(s.to_ascii_lowercase())
    } else {
        None
    }
}

/// Lit une ligne `<somme>  <nom>` (format `sha256sum`, binaire `*nom` toléré).
pub fn sha256_for_asset(sums: &str, asset: &str) -> Sha256Lookup {
    let mut found: Option<String> = None;
    for line in sums.lines() {
        let line = line.trim();
        if line.is_empty() || line.len() > 512 {
            continue;
        }
        let Some((raw, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let name = rest.trim_start().trim_start_matches('*').trim();
        if name != asset {
            continue;
        }
        let Some(digest) = hex64(raw) else {
            return Sha256Lookup::Malformed;
        };
        match &found {
            Some(prev) if prev != &digest => return Sha256Lookup::Malformed,
            Some(_) => {}
            None => found = Some(digest),
        }
    }
    match found {
        Some(d) => Sha256Lookup::Found(d),
        None => Sha256Lookup::Absent,
    }
}

/// Empreinte attendue de `asset` pour la release `tag` : lit SHA256SUMS.txt de
/// cette release. `Err` = fichier illisible ou absent de la release ;
/// `Absent`/`Malformed` disent que le fichier a été lu mais ne permet pas de
/// conclure.
pub fn expected_sha256(tag: &str, asset: &str) -> Result<Sha256Lookup, &'static str> {
    let sums = fetch_sha256sums(tag)?;
    Ok(sha256_for_asset(&sums, asset))
}

/// L'actif de release correspondant au binaire en cours d'exécution — même
/// détection que `download_update` (nom d'exécutable contenant `desktop`).
pub fn running_asset() -> Option<&'static str> {
    let kind = match std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_lowercase()))
    {
        Some(exe) if exe.contains("desktop") => "desktop",
        _ => "cli",
    };
    release_asset(kind, engine_os())
}

// ---------- Auto-mise à jour vérifiée par SHA-256 (téléchargement + relance) ----------

/// Plafond de taille d'un binaire / installateur téléchargé (150 Mio).
const UPDATE_MAX_BYTES: u64 = 150 * 1024 * 1024;

/// Variable d'environnement posée par l'ancienne instance lors d'un
/// auto-remplacement binaire autonome pour laisser le port se libérer avant
/// le `TcpListener::bind` de la nouvelle instance.
pub const UPDATE_WAIT_ENV: &str = "MCC_UPDATE_WAIT_PID";

/// Attend au démarrage la sortie réelle de l'ancienne instance (jusqu'à 10 s)
/// si le processus vient d'être relancé par une auto-mise à jour, afin que le
/// port UI et les fichiers soient totalement libérés.
pub fn wait_if_restarting_from_update() {
    if let Ok(raw) = std::env::var(UPDATE_WAIT_ENV) {
        std::env::remove_var(UPDATE_WAIT_ENV);
        if let Ok(pid) = raw.trim().parse::<u32>() {
            if pid != 0 && pid != std::process::id() {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while std::time::Instant::now() < deadline && probe::process_alive(pid) {
                    std::thread::sleep(std::time::Duration::from_millis(150));
                }
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(1200));
    }
}

/// Supprime silencieusement `<exe>.old` laissé par une auto-mise à jour
/// précédente (binaire autonome Windows/macOS/Linux).
pub fn cleanup_old_binary() {
    if let Ok(exe) = std::env::current_exe() {
        let mut old = exe.into_os_string();
        old.push(".old");
        let old_path = std::path::PathBuf::from(old);
        if old_path.exists() {
            let _ = std::fs::remove_file(&old_path);
        }
    }
}

/// Calcule le SHA-256 d'un fichier (borné à `UPDATE_MAX_BYTES`) et le compare
/// en temps constant à `expected_hex`. En cas d'écart, supprime le fichier
/// immédiatement (échec fermé : aucun binaire non vérifié ne reste sur disque).
pub fn verify_file_sha256(path: &std::path::Path, expected_hex: &str) -> Result<String, String> {
    let Some(expected) = hex64(expected_hex) else {
        let _ = std::fs::remove_file(path);
        return Err("empreinte SHA-256 attendue invalide".into());
    };
    let meta = std::fs::metadata(path).map_err(|e| {
        let _ = std::fs::remove_file(path);
        format!("fichier téléchargé illisible : {e}")
    })?;
    if meta.len() == 0 || meta.len() > UPDATE_MAX_BYTES {
        let _ = std::fs::remove_file(path);
        return Err(format!(
            "taille du fichier téléchargé hors bornes ({} octets)",
            meta.len()
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| {
        let _ = std::fs::remove_file(path);
        format!("lecture du fichier téléchargé impossible : {e}")
    })?;
    let actual = crate::crypto::hex(&crate::crypto::sha256(&bytes));
    if !crate::crypto::ct_eq(actual.as_bytes(), expected.as_bytes()) {
        let _ = std::fs::remove_file(path);
        return Err(format!(
            "empreinte SHA-256 invalide (attendu {expected}, obtenu {actual}) — fichier supprimé"
        ));
    }
    Ok(actual)
}

/// URL de téléchargement HTTPS d'un actif de release pour un `tag` validé.
pub fn release_download_url(tag: &str, asset: &str) -> Option<String> {
    if !tag_url_safe(tag) {
        return None;
    }
    // Vérifie que `asset` appartient strictement à la table blanche.
    let valid_asset = [
        "mcp-control-desktop-windows-x86_64-setup.exe",
        "mcp-control-desktop-macos-universal",
        "mcp-control-desktop-linux-x86_64",
        "mcp-control-cli-windows-x86_64.exe",
        "mcp-control-cli-macos-universal",
        "mcp-control-cli-linux-x86_64",
    ]
    .contains(&asset);
    if !valid_asset {
        return None;
    }
    Some(format!(
        "https://github.com/DeamonDev888/mcp-control/releases/download/{tag}/{asset}"
    ))
}

/// Extrait le dernier `Content-Length` valide (hors réponse intermédiaire
/// de redirection `301`/`302` et borné à `1..=UPDATE_MAX_BYTES`) depuis
/// les en-têtes bruts renvoyés par `curl -I`.
fn parse_last_content_length(headers: &str) -> Option<u64> {
    let mut current: Option<u64> = None;
    for line in headers.lines() {
        let trimmed = line.trim();
        if trimmed.len() >= 5 && trimmed[..5].eq_ignore_ascii_case("http/") {
            current = None;
            continue;
        }
        if let Some((name, val)) = trimmed.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                current = val
                    .trim()
                    .parse::<u64>()
                    .ok()
                    .filter(|&n| (1..=UPDATE_MAX_BYTES).contains(&n));
            }
        }
    }
    current
}

/// Interroge en HTTPS la taille attendue de l'actif via un `HEAD` préalable
/// (`None` si indisponible ou réponse chunked ; le téléchargement continue).
fn fetch_remote_content_length(url: &str) -> Option<u64> {
    let mut c = crate::probe::system_command("curl");
    probe::probe_hidden_cmd(&mut c);
    let out = c
        .args([
            "-fsSLI",
            "--proto",
            "=https",
            "--tlsv1.2",
            "-m",
            "10",
            "-H",
            "User-Agent: mcp-control",
            url,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_last_content_length(&String::from_utf8_lossy(&out.stdout))
}

/// Télécharge l'actif de release en HTTPS uniquement (TLS ≥ 1.2) via `curl`
/// système dans `dest`, en remontant périodiquement les octets écrits.
fn download_release_binary(
    tag: &str,
    asset: &str,
    dest: &std::path::Path,
    on_progress: impl Fn(u64, Option<u64>),
) -> Result<(), String> {
    use std::io::Read;

    let url = release_download_url(tag, asset)
        .ok_or_else(|| "tag ou actif de release hors table blanche".to_string())?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("impossible de créer le dossier de mise à jour : {e}"))?;
    }
    let _ = std::fs::remove_file(dest);
    let total = fetch_remote_content_length(&url);
    on_progress(0, total);

    let mut c = crate::probe::system_command("curl");
    probe::probe_hidden_cmd(&mut c);
    let mut child = c
        .args([
            "-fsSL",
            "--proto",
            "=https",
            "--tlsv1.2",
            "--max-time",
            "180",
            "--max-filesize",
            &UPDATE_MAX_BYTES.to_string(),
            "-H",
            "User-Agent: mcp-control",
            "-o",
        ])
        .arg(dest)
        .arg(&url)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("téléchargement impossible (curl) : {e}"))?;

    let err_reader = child.stderr.take().map(|pipe| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.take(4096).read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).trim().to_string()
        })
    });

    let mut last_bytes: u64 = 0;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {
                let bytes = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
                if bytes != last_bytes {
                    last_bytes = bytes;
                    on_progress(bytes, total);
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                if let Some(h) = err_reader {
                    let _ = h.join();
                }
                let _ = std::fs::remove_file(dest);
                return Err(format!("téléchargement impossible (curl) : {e}"));
            }
        }
    };
    let stderr = err_reader.and_then(|h| h.join().ok()).unwrap_or_default();
    if !status.success() {
        let _ = std::fs::remove_file(dest);
        return Err(if stderr.is_empty() {
            format!("téléchargement échoué ({status})")
        } else {
            format!("téléchargement échoué : {stderr}")
        });
    }
    let final_bytes = std::fs::metadata(dest)
        .map(|m| m.len())
        .unwrap_or(last_bytes);
    on_progress(final_bytes, total.or(Some(final_bytes)));
    Ok(())
}

/// Remplace atomiquement `target_exe` par `downloaded` en sauvegardant
/// l'ancien binaire en `<target_exe>.old` (restaure en cas d'échec).
pub fn replace_binary_file(
    downloaded: &std::path::Path,
    target_exe: &std::path::Path,
) -> Result<(), String> {
    let mut old_os = target_exe.as_os_str().to_os_string();
    old_os.push(".old");
    let old_path = std::path::PathBuf::from(old_os);
    if old_path.exists() {
        let _ = std::fs::remove_file(&old_path);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(downloaded, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("chmod +x impossible : {e}"))?;
    }
    // Étape 1 : déplacer l'exécutable courant vers `.old` (autorisé même en
    // cours d'exécution sous Windows, macOS et Linux).
    std::fs::rename(target_exe, &old_path)
        .map_err(|e| format!("impossible de déplacer l'exécutable actuel : {e}"))?;
    // Étape 2 : déplacer ou copier le nouveau binaire vérifié à sa place.
    let placed = std::fs::rename(downloaded, target_exe).or_else(|_| {
        std::fs::copy(downloaded, target_exe).map(|_| {
            let _ = std::fs::remove_file(downloaded);
        })
    });
    if let Err(e) = placed {
        let _ = std::fs::rename(&old_path, target_exe);
        return Err(format!("remplacement du binaire impossible : {e}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(target_exe, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

/// Lance l'installateur silencieux NSIS (Windows Desktop) ou remplace le
/// binaire autonome puis relance automatiquement la nouvelle version.
fn launch_verified_update(downloaded: &std::path::Path, asset: &str) -> Result<(), String> {
    let is_nsis = cfg!(windows) && asset.ends_with("-setup.exe");
    if is_nsis {
        let mut cmd = std::process::Command::new(downloaded);
        // Mode silencieux NSIS : aucun clic de wizard requis ; le hook
        // `NSIS_HOOK_POSTINSTALL` relance automatiquement l'application qui
        // hérite de `MCC_UPDATE_WAIT_PID` pour attendre notre sortie réelle.
        cmd.arg("/S")
            .env(UPDATE_WAIT_ENV, std::process::id().to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0000_0200);
        }
        cmd.spawn()
            .map_err(|e| format!("lancement de l'installateur impossible : {e}"))?;
    } else {
        let current_exe = std::env::current_exe()
            .map_err(|e| format!("chemin de l'exécutable courant introuvable : {e}"))?;
        replace_binary_file(downloaded, &current_exe)?;
        let args: Vec<String> = std::env::args().skip(1).collect();
        let mut cmd = std::process::Command::new(&current_exe);
        cmd.args(&args)
            .env(UPDATE_WAIT_ENV, std::process::id().to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000 | 0x0000_0200);
        }
        cmd.spawn()
            .map_err(|e| format!("relance de la nouvelle version impossible : {e}"))?;
    }
    // Laisse 1500 ms à l'interface (polling 350 ms) pour observer
    // `phase: "restarting"`, puis arrête proprement le moteur avant de sortir.
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        crate::shutdown_engine();
        std::process::exit(0);
    });
    Ok(())
}

static UPDATE_IN_PROGRESS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

struct UpdateGuard;
impl Drop for UpdateGuard {
    fn drop(&mut self) {
        UPDATE_IN_PROGRESS.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

#[derive(Clone)]
struct UpdateStatus {
    active: bool,
    phase: String,
    version: Option<String>,
    asset: Option<String>,
    bytes_downloaded: u64,
    bytes_total: Option<u64>,
    message: String,
    error: Option<String>,
    started_at: u64,
    updated_at: u64,
}

impl Default for UpdateStatus {
    fn default() -> Self {
        Self {
            active: false,
            phase: "idle".to_string(),
            version: None,
            asset: None,
            bytes_downloaded: 0,
            bytes_total: None,
            message: String::new(),
            error: None,
            started_at: 0,
            updated_at: 0,
        }
    }
}

static UPDATE_STATUS: std::sync::OnceLock<std::sync::Mutex<UpdateStatus>> =
    std::sync::OnceLock::new();

fn update_status_cell() -> &'static std::sync::Mutex<UpdateStatus> {
    UPDATE_STATUS.get_or_init(|| std::sync::Mutex::new(UpdateStatus::default()))
}

fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn set_update_status(f: impl FnOnce(&mut UpdateStatus)) {
    let mut s = lockv(update_status_cell());
    f(&mut s);
    s.updated_at = now_epoch_secs();
}

fn update_percent(downloaded: u64, total: Option<u64>) -> Option<u8> {
    let tot = total.filter(|&t| t > 0)?;
    Some(((downloaded as u128 * 100) / tot as u128).min(100) as u8)
}

pub(crate) fn current_update_status() -> Value {
    let s = lockv(update_status_cell()).clone();
    let percent = update_percent(s.bytes_downloaded, s.bytes_total);
    json!({
        "active": s.active,
        "phase": s.phase,
        "version": s.version,
        "asset": s.asset,
        "bytes_downloaded": s.bytes_downloaded,
        "bytes_total": s.bytes_total,
        "percent": percent,
        "message": s.message,
        "error": s.error,
        "started_at": s.started_at,
        "updated_at": s.updated_at,
    })
}

pub(crate) fn update_status_hash<H: std::hash::Hasher>(h: &mut H) {
    use std::hash::Hash;
    let s = lockv(update_status_cell());
    s.active.hash(h);
    s.phase.hash(h);
    s.version.hash(h);
    s.asset.hash(h);
    s.bytes_downloaded.hash(h);
    s.bytes_total.hash(h);
    s.message.hash(h);
    s.error.hash(h);
    s.started_at.hash(h);
    s.updated_at.hash(h);
}

/// Dossier privé de staging de mise à jour (`<config_dir>/update-staging/`,
/// hors `/tmp` partagé).
fn private_update_dir(config_path: &str) -> std::path::PathBuf {
    let base = if !config_path.is_empty() {
        std::path::Path::new(config_path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(std::path::Path::to_path_buf)
    } else {
        None
    };
    base.or_else(default_user_config_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("update-staging")
}

/// Pipeline asynchrone d'auto-mise à jour avec suivi d'avancement temps réel
/// (`UPDATE_STATUS` / `/api/state`).
fn run_auto_update_job(state: &Arc<AppState>, guard: UpdateGuard) {
    let fail = |msg: String| {
        add_log(state, "warn", &format!("[update] {msg}"));
        set_update_status(|s| {
            s.active = false;
            s.phase = "error".to_string();
            s.error = Some(msg.clone());
            s.message = msg;
        });
    };

    let tag = match github_latest_tag() {
        Ok(Some(t)) => t,
        Ok(None) | Err(_) => {
            fail("impossible de joindre GitHub pour vérifier la dernière release".to_string());
            return;
        }
    };
    let version = tag.strip_prefix('v').unwrap_or(&tag).to_string();
    let current = env!("CARGO_PKG_VERSION");
    set_update_status(|s| {
        s.version = Some(version.clone());
    });
    if !version_newer(&version, current) {
        fail(format!(
            "déjà à jour (v{current}, dernière release : v{version})"
        ));
        return;
    }
    let Some(asset) = running_asset() else {
        fail(format!("plateforme non prise en charge ({})", engine_os()));
        return;
    };
    set_update_status(|s| {
        s.asset = Some(asset.to_string());
        s.phase = "verifying_manifest".to_string();
        s.message = format!("Vérification de SHA256SUMS.txt pour v{version}…");
    });
    let expected = match expected_sha256(&tag, asset) {
        Ok(Sha256Lookup::Found(sum)) => sum,
        Ok(Sha256Lookup::Absent) => {
            fail(format!(
                "empreinte SHA-256 absente de SHA256SUMS.txt pour {asset} — mise à jour refusée par sécurité"
            ));
            return;
        }
        Ok(Sha256Lookup::Malformed) => {
            fail(
                "SHA256SUMS.txt invalide ou ambigu — mise à jour refusée par sécurité".to_string(),
            );
            return;
        }
        Err(e) => {
            fail(format!(
                "impossible de lire SHA256SUMS.txt ({e}) — mise à jour refusée par sécurité"
            ));
            return;
        }
    };

    let upd_dir = private_update_dir(&state.config_path);
    let _ = std::fs::create_dir_all(&upd_dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&upd_dir, std::fs::Permissions::from_mode(0o700));
    }
    let dest = upd_dir.join(format!("{}-{asset}", std::process::id()));
    add_log(
        state,
        "info",
        &format!("[update] téléchargement de {asset} (v{version})…"),
    );
    set_update_status(|s| {
        s.phase = "downloading".to_string();
        s.bytes_downloaded = 0;
        s.bytes_total = None;
        s.message = format!("Téléchargement de {asset} (v{version})…");
    });
    if let Err(e) = download_release_binary(&tag, asset, &dest, |dl, tot| {
        set_update_status(|s| {
            s.bytes_downloaded = dl;
            s.bytes_total = tot;
        });
    }) {
        let _ = std::fs::remove_file(&dest);
        fail(e);
        return;
    }

    set_update_status(|s| {
        s.phase = "verifying_sha256".to_string();
        s.message = format!("Vérification SHA-256 de {asset}…");
    });
    let verified_sha = match verify_file_sha256(&dest, &expected) {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_file(&dest);
            fail(e);
            return;
        }
    };

    set_update_status(|s| {
        s.phase = "installing".to_string();
        s.message = format!("Installation de v{version}…");
    });
    add_log(
        state,
        "info",
        &format!(
            "[update] {asset} (v{version}) vérifié (SHA-256 {verified_sha}) — installation et redémarrage automatique"
        ),
    );
    if let Err(e) = launch_verified_update(&dest, asset) {
        let _ = std::fs::remove_file(&dest);
        fail(e);
        return;
    }
    set_update_status(|s| {
        s.active = true;
        s.phase = "restarting".to_string();
        s.error = None;
        s.message = format!("v{version} vérifiée (SHA-256 OK) — redémarrage en cours…");
    });
    // Conserve le verrou pendant le court délai de sortie avant arrêt du processus.
    std::mem::forget(guard);
}

/// Orchestration non bloquante de l'action locale `install_update` :
/// démarre la mise à jour vérifiée en tâche de fond et renvoie aussitôt
/// l'état initial (`update_status`), ensuite diffusé en direct via `/api/state`.
pub fn perform_auto_update(state: &Arc<AppState>) -> Value {
    if UPDATE_IN_PROGRESS
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .is_err()
    {
        return json!({
            "ok": true,
            "started": false,
            "already_running": true,
            "update_status": current_update_status(),
        });
    }
    let guard = UpdateGuard;
    let initial_version = lockv(&state.update_latest).clone();
    let initial_asset = running_asset().map(str::to_string);
    let now = now_epoch_secs();
    set_update_status(|s| {
        s.active = true;
        s.phase = "checking".to_string();
        s.version = initial_version;
        s.asset = initial_asset;
        s.bytes_downloaded = 0;
        s.bytes_total = None;
        s.message = "Vérification de la dernière release GitHub…".to_string();
        s.error = None;
        s.started_at = now;
    });
    let st = Arc::clone(state);
    match std::thread::Builder::new()
        .name("mcc-auto-update".to_string())
        .spawn(move || run_auto_update_job(&st, guard))
    {
        Ok(_) => json!({
            "ok": true,
            "started": true,
            "update_status": current_update_status(),
        }),
        Err(e) => {
            let msg = format!("impossible de lancer le thread de mise à jour : {e}");
            set_update_status(|s| {
                s.active = false;
                s.phase = "error".to_string();
                s.error = Some(msg.clone());
                s.message = msg.clone();
            });
            json!({
                "ok": false,
                "error": msg,
                "update_status": current_update_status(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        cli_release_asset, current_update_status, parse_last_content_length, update_percent,
        UPDATE_MAX_BYTES,
    };

    #[test]
    fn cli_release_asset_table_blanche() {
        assert_eq!(
            cli_release_asset("linux", "x86_64"),
            Some("mcp-control-cli-linux-x86_64")
        );
        assert_eq!(
            cli_release_asset("Linux", "amd64"),
            Some("mcp-control-cli-linux-x86_64")
        );
        assert_eq!(
            cli_release_asset("macos", "arm64"),
            Some("mcp-control-cli-macos-universal")
        );
        assert_eq!(
            cli_release_asset("Darwin", "aarch64"),
            Some("mcp-control-cli-macos-universal")
        );
        assert_eq!(
            cli_release_asset("windows", "x86_64"),
            Some("mcp-control-cli-windows-x86_64.exe")
        );
        assert_eq!(cli_release_asset("linux", "aarch64"), None);
        assert_eq!(cli_release_asset("freebsd", "x86_64"), None);
    }

    #[test]
    fn parse_last_content_length_suit_redirections_et_bornes() {
        assert_eq!(
            parse_last_content_length("HTTP/1.1 200 OK\r\nContent-Length: 123456\r\n\r\n"),
            Some(123_456)
        );
        assert_eq!(
            parse_last_content_length(
                "HTTP/1.1 302 Found\r\nLocation: https://objects.example/bin\r\nContent-Length: 0\r\n\r\nHTTP/2 200\r\ncontent-length: 42991616\r\n\r\n"
            ),
            Some(42_991_616)
        );
        assert_eq!(
            parse_last_content_length(
                "HTTP/1.1 302 Found\r\nContent-Length: 512\r\n\r\nHTTP/2 200\r\nTransfer-Encoding: chunked\r\n\r\n"
            ),
            None
        );
        assert_eq!(
            parse_last_content_length("HTTP/2 200\r\nContent-Length: 0\r\n\r\n"),
            None
        );
        let too_big = format!(
            "HTTP/2 200\r\nContent-Length: {}\r\n\r\n",
            UPDATE_MAX_BYTES + 1
        );
        assert_eq!(parse_last_content_length(&too_big), None);
    }

    #[test]
    fn current_update_status_expose_champs_attendus() {
        let st = current_update_status();
        for key in [
            "active",
            "phase",
            "version",
            "asset",
            "bytes_downloaded",
            "bytes_total",
            "percent",
            "message",
            "error",
            "started_at",
            "updated_at",
        ] {
            assert!(
                st.get(key).is_some(),
                "champ `{key}` manquant dans update_status : {st}"
            );
        }
        assert_eq!(update_percent(0, None), None);
        assert_eq!(update_percent(50, Some(0)), None);
        assert_eq!(update_percent(50, Some(200)), Some(25));
        assert_eq!(update_percent(300, Some(200)), Some(100));
    }
}
