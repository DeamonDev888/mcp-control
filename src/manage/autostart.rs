// Démarrage automatique de l'app avec l'OS (registre Windows / autostart freedesktop).
// (split v0.7.20 : manage.rs reste la racine — API stable via réexport)
use super::*;

/// Clé/valeur du registre Run (Windows) ou fichiers autostart (Linux/macOS).
#[cfg_attr(not(windows), allow(dead_code))] // utilisé par le registre Run (Windows)
const AUTOSTART_RUN_NAME: &str = "MCP Control";

/// Arguments de l'entrée de démarrage automatique (compilé hors tests seulement :
/// en build test le corps réel est neutralisé, la fonction deviendrait du code mort).
#[cfg(not(test))]
fn autostart_entry_args() -> String {
    let exe = std::env::current_exe()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    format!("\"{}\" --hidden", exe)
}

/// L'entrée de démarrage automatique existe-t-elle actuellement ?
pub fn autostart_active() -> bool {
    #[cfg(windows)]
    {
        let mut c = Command::new("reg");
        probe::probe_hidden_cmd(&mut c);
        c.args([
            "query",
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
            "/v",
            AUTOSTART_RUN_NAME,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        autostart_file_path().map(|p| p.exists()).unwrap_or(false)
    }
}

/// Applique la préférence : crée ou supprime l'entrée de démarrage automatique.
/// Windows : registre HKCU …\Run. Linux : ~/.config/autostart/*.desktop.
/// macOS : ~/Library/LaunchAgents/*.plist.
pub fn apply_autostart(state: &Arc<AppState>, enable: bool) {
    // les tests ne doivent JAMAIS toucher le registre/fichiers de l'OS réels
    #[cfg(test)]
    {
        let _ = (state, enable);
    }
    // corps réel séparé : le garde cfg(test) ne laisse rien d'inatteignable
    // (l'ancien `return` en build test déclenchait unreachable_code — clippy)
    #[cfg(not(test))]
    apply_autostart_os(state, enable);
}

/// Corps réel de l'alignement autostart (compilé hors tests seulement).
#[cfg(not(test))]
#[allow(unused_must_use)]
fn apply_autostart_os(state: &Arc<AppState>, enable: bool) {
    #[cfg_attr(not(windows), allow(unused_variables))] // lu dans le bloc cfg(windows)
    let args = autostart_entry_args();
    #[cfg(windows)]
    {
        let root = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run";
        let mut c = Command::new("reg");
        probe::probe_hidden_cmd(&mut c);
        let status = if enable {
            c.args([
                "add",
                root,
                "/v",
                AUTOSTART_RUN_NAME,
                "/t",
                "REG_SZ",
                "/d",
                &args,
                "/f",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        } else {
            c.args(["delete", root, "/v", AUTOSTART_RUN_NAME, "/f"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
        };
        if status.map(|s| s.success()).unwrap_or(false) {
            add_log(
                state,
                "info",
                &format!(
                    "[autostart] entrée de démarrage {}",
                    if enable { "créée" } else { "supprimée" }
                ),
            );
        } else {
            add_log(
                state,
                "warn",
                "[autostart] reg a échoué — l'entrée n'a pas changé",
            );
        }
    }
    #[cfg(not(windows))]
    {
        let Some(path) = autostart_file_path() else {
            return;
        };
        if enable {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir);
            }
            let exe = std::env::current_exe()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_default();
            let content = if path.extension().map(|e| e == "desktop").unwrap_or(false) {
                format!(
                    "[Desktop Entry]
Type=Application
Name=MCP Control
Exec=\"{}\" --hidden
Hidden=false
X-GNOME-Autostart-enabled=true
",
                    exe
                )
            } else {
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\"><dict>
<key>Label</key><string>com.deamondev.mcp-control</string>
<key>ProgramArguments</key><array>
<string>{}</string><string>--hidden</string>
</array>
<key>RunAtLoad</key><true/>
</dict></plist>
",
                    exe
                )
            };
            std::fs::write(&path, content);
            add_log(state, "info", "[autostart] entrée de démarrage créée");
        } else if path.exists() {
            std::fs::remove_file(&path);
            add_log(state, "info", "[autostart] entrée de démarrage supprimée");
        }
    }
}

/// Chemin du fichier autostart (Linux : XDG ; macOS : LaunchAgent). Windows :
/// registre — None (géré par reg).
#[cfg(not(windows))]
fn autostart_file_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    #[cfg(target_os = "macos")]
    {
        Some(
            std::path::PathBuf::from(home)
                .join("Library/LaunchAgents/com.deamondev.mcp-control.plist"),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        Some(std::path::PathBuf::from(home).join(".config/autostart/mcp-control-desktop.desktop"))
    }
}

/// Chemin d'exécutable inscrit dans la VALEUR de l'entrée de démarrage
/// (`"C:\...\app.exe" --hidden`, `Exec="..." --hidden`) — None si illisible.
/// Sans guillemets, le premier segment est pris (une valeur non quotée avec
/// espaces devient illisible : défaut sûr = « pointe ailleurs »).
fn run_value_exe_path(value: &str) -> Option<String> {
    let v = value.trim();
    let candidate = if let Some(rest) = v.strip_prefix('"') {
        let end = rest.find('"')?;
        &rest[..end]
    } else {
        v.split_whitespace().next()?
    };
    let candidate = candidate.trim();
    (!candidate.is_empty()).then(|| candidate.to_string())
}

/// Données brutes de la valeur Run dans la SORTIE de `reg query` (ligne
/// `    MCP Control    REG_SZ    "..." --hidden`) — None si absente.
#[cfg_attr(not(windows), allow(dead_code))] // lecteur du registre Run (Windows)
fn run_value_from_reg_query(output: &str) -> Option<String> {
    output
        .lines()
        .find(|l| l.contains("REG_SZ"))
        .and_then(|l| l.split("REG_SZ").nth(1))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Valeur ACTUELLE de l'entrée Run (Windows) — None si absente/illisible.
/// Lecture seule : le boot ne doit JAMAIS écrire le registre d'après le lot B.
#[cfg(windows)]
fn recorded_run_value() -> Option<String> {
    let mut c = Command::new("reg");
    probe::probe_hidden_cmd(&mut c);
    c.args([
        "query",
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
        "/v",
        AUTOSTART_RUN_NAME,
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .output()
    .ok()
    .filter(|o| o.status.success())
    .and_then(|o| run_value_from_reg_query(&String::from_utf8_lossy(&o.stdout)))
}

/// Chemin d'exécutable inscrit dans le CONTENU d'un fichier autostart
/// (Linux : ligne `Exec=` du .desktop ; macOS : premier `<string>` de
/// ProgramArguments du plist) — None si format inconnu. (Branche non-Windows,
/// testée depuis tous les OS.)
#[cfg_attr(windows, allow(dead_code))]
fn autostart_file_exe_path(content: &str) -> Option<String> {
    for line in content.lines() {
        if let Some(rest) = line.trim().strip_prefix("Exec=") {
            return run_value_exe_path(rest);
        }
    }
    if let Some(i) = content.find("<key>ProgramArguments</key>") {
        let rest = &content[i..];
        if let Some(start) = rest.find("<string>") {
            let after = &rest[start + "<string>".len()..];
            if let Some(end) = after.find("</string>") {
                let p = after[..end].trim();
                if !p.is_empty() {
                    return Some(p.to_string());
                }
            }
        }
    }
    None
}

/// Chemin d'exécutable actuellement enregistré dans l'entrée de démarrage
/// automatique — None si l'entrée est absente ou illisible. Windows : valeur
/// du registre Run ; Linux/macOS : contenu du fichier autostart.
fn recorded_entry_path() -> Option<String> {
    #[cfg(windows)]
    {
        recorded_run_value().and_then(|v| run_value_exe_path(&v))
    }
    #[cfg(not(windows))]
    {
        let path = autostart_file_path()?;
        let content = std::fs::read_to_string(path).ok()?;
        autostart_file_exe_path(&content)
    }
}

/// L'entrée enregistrée pointe-t-elle sur CET exécutable ? Comparaison
/// tolérante : guillemets retirés, séparateurs canonisés, casse ignorée.
/// Une fausse négativité (entrée classée « ailleurs ») reste le défaut SÛR
/// au démarrage : l'entrée est alors simplement laissée en place.
fn entry_points_to_own_exe(recorded: &str, current_exe: &str) -> bool {
    let norm = |p: &str| {
        p.trim()
            .trim_matches('"')
            .trim()
            .replace('\\', "/")
            .to_ascii_lowercase()
    };
    let (a, b) = (norm(recorded), norm(current_exe));
    !a.is_empty() && a == b
}

/// Décision d'alignement de l'entrée de démarrage AU BOOT (fonction pure —
/// aucun accès OS, testée sans registre). Règle du lot B : au démarrage,
/// l'app ne supprime ni ne réécrit l'entrée que si elle pointe déjà sur son
/// propre exécutable ; une entrée qui pointe ailleurs (autre installation,
/// build de dev, dossier différent) n'est JAMAIS touchée au démarrage — seul
/// le geste explicite de l'utilisateur (apply_autostart, interrupteur
/// Auto-boot) peut la reprendre. Entrée absente : créée seulement si le
/// réglage la demande (premier lancement, settings.autostart=true).
#[derive(Debug, PartialEq, Eq)]
enum AutostartBootAction {
    /// Ne rien faire.
    Aucun,
    /// Créer l'entrée (absente, réglage demandé).
    Activer,
    /// Supprimer l'entrée (pointe sur cet exécutable, réglage désactivé).
    Desactiver,
}

fn autostart_boot_decision(
    recorded: Option<&str>,
    current_exe: &str,
    want: bool,
) -> AutostartBootAction {
    match recorded {
        None => {
            if want {
                AutostartBootAction::Activer
            } else {
                AutostartBootAction::Aucun
            }
        }
        Some(path) => {
            if entry_points_to_own_exe(path, current_exe) {
                if want {
                    AutostartBootAction::Aucun // déjà correcte : rien à écrire
                } else {
                    AutostartBootAction::Desactiver
                }
            } else {
                // l'entrée appartient à une AUTRE installation : jamais touchée
                AutostartBootAction::Aucun
            }
        }
    }
}

/// Au boot : aligne l'entrée de démarrage sur la préférence persistée
/// UNIQUEMENT quand l'entrée appartient à CET exécutable (ou est absente).
/// Une entrée qui pointe sur une autre installation n'est jamais touchée :
/// une instance de test, de développement ou lancée depuis un autre dossier
/// ne casse plus le démarrage automatique de l'installation réelle.
pub fn sync_autostart_at_boot(state: &Arc<AppState>) {
    let want = lockv(&state.settings).autostart;
    let recorded = recorded_entry_path();
    let current = std::env::current_exe()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    match autostart_boot_decision(recorded.as_deref(), &current, want) {
        AutostartBootAction::Aucun => {}
        AutostartBootAction::Activer => apply_autostart(state, true),
        AutostartBootAction::Desactiver => apply_autostart(state, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Installation réelle (exemple) — c'est CE chemin qu'une instance de test
    /// trouvait dans l'entrée Run et qu'elle effaçait ou réécrivait.
    const REEL: &str = "C:\\Users\\Op\\AppData\\Local\\MCP Control\\mcp-control-desktop.exe";
    /// Build de développement (autre dossier, autre exécutable).
    const DEV: &str = "C:\\dev\\mcp-command-center\\target\\debug\\mcp-control.exe";

    #[test]
    fn decision_boot_entree_pointant_ailleurs_jamais_touchee() {
        // le bug rapporté : toute instance test/dev (autostart:false) EFFAÇAIT
        // l'entrée de l'installation réelle, et autostart:true la réécrivait
        for want in [true, false] {
            assert_eq!(
                autostart_boot_decision(Some(REEL), DEV, want),
                AutostartBootAction::Aucun,
                "want={want} : une entrée qui pointe ailleurs n'est jamais touchée"
            );
        }
    }

    #[test]
    fn decision_boot_entree_pointant_sur_soi_alignee() {
        assert_eq!(
            autostart_boot_decision(Some(REEL), REEL, true),
            AutostartBootAction::Aucun,
            "entrée déjà correcte : rien à écrire"
        );
        assert_eq!(
            autostart_boot_decision(Some(REEL), REEL, false),
            AutostartBootAction::Desactiver,
            "notre propre entrée : alignée sur le réglage"
        );
    }

    #[test]
    fn decision_boot_entree_absente_selon_le_reglage() {
        assert_eq!(
            autostart_boot_decision(None, REEL, true),
            AutostartBootAction::Activer
        );
        assert_eq!(
            autostart_boot_decision(None, REEL, false),
            AutostartBootAction::Aucun
        );
    }

    #[test]
    fn comparaison_tolere_guillemets_casse_et_separateurs() {
        // la fonction reçoit le chemin EXTRAIT (run_value_exe_path) — la forme
        // quotée avec --hidden est gérée en amont (cf. test extraction)
        let quotee = format!("\"{REEL}\"");
        assert!(
            entry_points_to_own_exe(&quotee, REEL),
            "forme quotée du registre"
        );
        assert!(
            entry_points_to_own_exe(REEL, &REEL.to_ascii_uppercase()),
            "casse Windows"
        );
        assert!(
            entry_points_to_own_exe(&REEL.replace('\\', "/"), REEL),
            "séparateurs canonisés"
        );
        // même dossier, AUTRE exécutable → pas notre entrée
        let autre = "C:\\Users\\Op\\AppData\\Local\\MCP Control\\autre.exe";
        assert!(!entry_points_to_own_exe(autre, REEL));
        assert!(!entry_points_to_own_exe(DEV, REEL));
        assert!(!entry_points_to_own_exe("", REEL));
    }

    #[test]
    fn chemin_exe_extrait_des_formes_ecrites() {
        let quotee = format!("\"{REEL}\" --hidden");
        assert_eq!(run_value_exe_path(&quotee).as_deref(), Some(REEL));
        // forme XDG (Linux) : Exec="…" --hidden
        let desktop = format!("[Desktop Entry]\nType=Application\nExec=\"{REEL}\" --hidden\n");
        assert_eq!(autostart_file_exe_path(&desktop).as_deref(), Some(REEL));
        // forme plist (macOS) : premier <string> de ProgramArguments
        let plist = format!(
            "<plist><dict><key>ProgramArguments</key><array>\n<string>{REEL}</string><string>--hidden</string>\n</array></dict></plist>"
        );
        assert_eq!(autostart_file_exe_path(&plist).as_deref(), Some(REEL));
        assert_eq!(autostart_file_exe_path("format inconnu"), None);
    }

    #[cfg(windows)]
    #[test]
    fn valeur_run_extraite_de_la_sortie_reg_query() {
        let sortie = concat!(
            "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\r\n",
            "    MCP Control    REG_SZ    \"C:\\Users\\Op\\AppData\\Local\\MCP Control\\mcp-control-desktop.exe\" --hidden\r\n\r\n",
        );
        let v = run_value_from_reg_query(sortie).expect("données REG_SZ extraites");
        assert!(v.starts_with('"'), "données brutes conservées : {v}");
        assert_eq!(
            run_value_exe_path(&v).as_deref(),
            Some(REEL),
            "chemin extrait de la valeur quotée"
        );
        assert!(run_value_from_reg_query("sortie sans REG_SZ").is_none());
    }
}
