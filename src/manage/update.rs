// Vérification de mise à jour (release GitHub) — au boot puis chaque heure.
// Assets de release : table blanche (jamais construits depuis la requête).
// (split v0.7.20 : manage.rs reste la racine — API stable via réexport)
use super::*;

pub fn version_newer(latest: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.trim()
            .trim_start_matches('v')
            .split('.')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (a, b) = (parse(latest), parse(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

/// Interroge la dernière release GitHub. `curl` d'abord (léger, présent sur
/// macOS/Linux/Windows 10+) avec repli PowerShell — l'ancien code n'utilisait
/// QUE PowerShell : la vérification était silencieusement inopérante partout
/// ailleurs (audit 2026-10-24). `Ok(Some(tag))` = tag lu ; `Ok(None)` = réponse
/// sans tag ; `Err` = réseau/API/outil indisponible — la distinction permet au
/// bouton « Vérifier » de ne plus prétendre « à jour » quand le check a raté.
fn github_latest_tag() -> Result<Option<String>, &'static str> {
    let mut c = crate::probe::system_command("curl");
    probe::probe_hidden_cmd(&mut c);
    if let Ok(out) = c
        .args([
            "-fsSL",
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
        .args(["-fsSL", "-m", "6", "-H", "User-Agent: mcp-control", url])
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
