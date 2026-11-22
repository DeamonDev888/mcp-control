// Démarrage automatique de l'app avec l'OS (registre Windows / autostart freedesktop).
// (split v0.7.20 : manage.rs reste la racine — API stable via réexport)
use super::*;

/// Clé/valeur du registre Run (Windows) ou fichiers autostart (Linux/macOS).
#[cfg_attr(not(windows), allow(dead_code))] // utilisé par le registre Run (Windows)
const AUTOSTART_RUN_NAME: &str = "MCP Control";

/// Clé Run utilisée : `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
/// par défaut. `MCC_AUTOSTART_KEY` la remplace — c'est ce qui permet à l'E2E
/// d'écrire dans une clé de test au lieu de l'entrée RÉELLE de la machine.
#[cfg_attr(not(windows), allow(dead_code))]
fn run_key() -> String {
    std::env::var("MCC_AUTOSTART_KEY")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run".to_string())
}

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
        let mut c = Command::new(probe::system_tool("reg"));
        probe::probe_hidden_cmd(&mut c);
        c.args(["query", &run_key(), "/v", AUTOSTART_RUN_NAME])
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
        let root = run_key();
        let mut c = Command::new(probe::system_tool("reg"));
        probe::probe_hidden_cmd(&mut c);
        let status = if enable {
            c.args([
                "add",
                &root,
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
            c.args(["delete", &root, "/v", AUTOSTART_RUN_NAME, "/f"])
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

/// Lit la valeur `name` d'une sous-clé HKCU par l'API Windows
/// (`RegOpenKeyExW` + `RegQueryValueExW`, advapi32) — plus de parsing du texte
/// de `reg query`, dont le format dépend de la langue et des espaces du
/// chemin. `key` doit être `HKCU\...` : toute autre racine est refusée.
#[cfg(windows)]
fn read_run_value(key: &str, name: &str) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(
            root: isize,
            sub: *const u16,
            options: u32,
            access: u32,
            out: *mut isize,
        ) -> i32;
        fn RegQueryValueExW(
            key: isize,
            name: *const u16,
            reserved: *mut u32,
            ty: *mut u32,
            data: *mut u8,
            len: *mut u32,
        ) -> i32;
        fn RegCloseKey(key: isize) -> i32;
    }
    const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as isize;
    const KEY_QUERY_VALUE: u32 = 0x0001;
    const REG_SZ: u32 = 1;
    const REG_EXPAND_SZ: u32 = 2;
    let sub = key
        .strip_prefix("HKCU\\")
        .or_else(|| key.strip_prefix("HKCU/"))?;
    let wide_sub: Vec<u16> = std::ffi::OsStr::new(sub)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let wide_name: Vec<u16> = std::ffi::OsStr::new(name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let mut hkey: isize = 0;
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            wide_sub.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut hkey,
        ) != 0
        {
            return None;
        }
        let mut ty: u32 = 0;
        let mut len: u32 = 0;
        let rc = RegQueryValueExW(
            hkey,
            wide_name.as_ptr(),
            std::ptr::null_mut(),
            &mut ty,
            std::ptr::null_mut(),
            &mut len,
        );
        if rc != 0 || (ty != REG_SZ && ty != REG_EXPAND_SZ) {
            RegCloseKey(hkey);
            return None;
        }
        let mut buf = vec![0u8; len.max(2) as usize];
        let rc = RegQueryValueExW(
            hkey,
            wide_name.as_ptr(),
            std::ptr::null_mut(),
            &mut ty,
            buf.as_mut_ptr(),
            &mut len,
        );
        RegCloseKey(hkey);
        if rc != 0 || (ty != REG_SZ && ty != REG_EXPAND_SZ) {
            return None;
        }
        // REG_SZ : UTF-16LE terminé par NUL (la longueur rendue est en OCTETS)
        let used = (len as usize).min(buf.len());
        let (paires, _) = buf[..used].as_chunks::<2>();
        let units: Vec<u16> = paires
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .take_while(|u| *u != 0)
            .collect();
        let s = String::from_utf16_lossy(&units);
        (!s.trim().is_empty()).then_some(s)
    }
}

/// Valeur ACTUELLE de l'entrée Run (Windows) — None si absente/illisible.
/// Lecture seule : le boot ne doit JAMAIS écrire le registre (lot B).
#[cfg(windows)]
fn recorded_run_value() -> Option<String> {
    read_run_value(&run_key(), AUTOSTART_RUN_NAME)
}

/// Chemin d'exécutable actuellement enregistré dans l'entrée de démarrage
/// (None si absente/illisible). Public : l'action `set_autostart` s'en sert
/// pour demander confirmation quand l'entrée appartient à une AUTRE
/// installation (audit S4, point 6).
pub fn autostart_recorded_path() -> Option<String> {
    recorded_entry_path()
}

/// L'entrée enregistrée appartient-elle à une AUTRE installation que
/// `current_exe` ? Dans ce cas `set_autostart` exige `confirm:true` avant de
/// la réécrire (audit S4, point 6) — une instance de test ou de développement
/// ne peut plus écraser le démarrage automatique de l'installation réelle.
pub fn autostart_needs_confirm(recorded: Option<&str>, current_exe: &str) -> bool {
    recorded
        .map(|r| !entry_points_to_own_exe(r, current_exe))
        .unwrap_or(false)
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

    #[test]
    fn confirmation_exigee_si_entree_d_une_autre_installation() {
        // audit S4, point 6 : set_autostart ne réécrit pas l'entrée d'une autre
        // installation sans `confirm:true`.
        assert!(
            autostart_needs_confirm(Some(REEL), DEV),
            "une entrée qui pointe sur l'installation réelle exige confirmation depuis un build de dev"
        );
        assert!(
            !autostart_needs_confirm(Some(REEL), REEL),
            "notre propre entrée : aucune confirmation"
        );
        assert!(
            !autostart_needs_confirm(None, DEV),
            "entrée absente : création directe, rien à écraser"
        );
    }

    /// Lecture par l'API Windows (`RegQueryValueExW`) sur une clé de TEST :
    /// aucune écriture dans l'entrée Run réelle de la machine. La clé est
    /// supprimée à la fin, même si les assertions échouent (garde de nettoyage).
    #[cfg(windows)]
    #[test]
    fn lecture_du_registre_par_api_sur_cle_de_test() {
        let cle = format!(r"HKCU\Software\mcp-control-selftest-{}", std::process::id());
        let valeur = format!("\"{REEL}\" --hidden");
        let ajout = probe::system_command("reg")
            .args([
                "add",
                &cle,
                "/v",
                "MCP Control",
                "/t",
                "REG_SZ",
                "/d",
                &valeur,
                "/f",
            ])
            .output()
            .expect("reg add");
        assert!(ajout.status.success(), "reg add a échoué : {ajout:?}");
        let lu = read_run_value(&cle, "MCP Control");
        // Négatif : un type REG_DWORD est ignoré (seuls REG_SZ / REG_EXPAND_SZ sont lus)
        let ajout_dword = probe::system_command("reg")
            .args([
                "add",
                &cle,
                "/v",
                "NonChaine",
                "/t",
                "REG_DWORD",
                "/d",
                "1",
                "/f",
            ])
            .output()
            .expect("reg add dword");
        let lu_dword = if ajout_dword.status.success() {
            read_run_value(&cle, "NonChaine")
        } else {
            None
        };
        // nettoyage AVANT les assertions : la clé de test ne doit pas survivre
        let _ = probe::system_command("reg")
            .args(["delete", &cle, "/f"])
            .output();
        assert!(lu_dword.is_none(), "REG_DWORD ne doit pas être lu");
        let lu = lu.expect("valeur lue par l'API");
        assert_eq!(lu, valeur, "la valeur REG_SZ doit être rendue telle quelle");
        assert_eq!(
            run_value_exe_path(&lu).as_deref(),
            Some(REEL),
            "chemin extrait de la valeur quotée"
        );
        assert!(
            read_run_value(&cle, "MCP Control").is_none(),
            "clé supprimée : plus aucune valeur"
        );
        assert!(read_run_value(r"HKCU\Software", "MCP Control").is_none());
        assert!(
            read_run_value(r"HKLM\Software", "MCP Control").is_none(),
            "seule la racine HKCU est acceptée"
        );
    }
}
