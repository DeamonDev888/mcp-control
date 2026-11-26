//! mcp-control — CLI headless.
//!
//! Zéro interface : moteur + serveur HTTP + journal. L'UI native (fenêtre OS)
//! est le binaire `desktop` (Tauri 2). Lancé avec `--hidden` (autostart), la
//! fenêtre console est masquée — la partie HTTP ne doit rien montrer à l'écran.

use mcp_control::manage;

fn main() {
    // Sous-commandes `auth` (lot M1) : elles n'ouvrent AUCUN moteur — elles
    // parlent au moteur local (ou lisent config.json) et sortent.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("auth") {
        std::process::exit(mcp_control::auth::cli::run(&args[2..]));
    }
    // Pont stdio (lot M5, DA6 = a) : parle MCP sur stdin/stdout et signe pour
    // le client local. N'ouvre AUCUN moteur — un client MCP standard le lance.
    if args.get(1).map(String::as_str) == Some("bridge") {
        std::process::exit(mcp_control::bridge::run(&args[2..]));
    }
    // Auto-mise à jour : si relancé par l'ancienne instance, laisse le port
    // se libérer, puis nettoie tout binaire `.old` résiduel.
    manage::wait_if_restarting_from_update();
    manage::cleanup_old_binary();
    // MCC_CONFIG override ; sinon résolution runtime (dev → manifest, release → exe dir)
    let _config_path =
        manage::resolve_config_path(env!("CARGO_MANIFEST_DIR"), std::env::var("MCC_CONFIG").ok());
    #[cfg(all(windows, not(test)))]
    hide_console_if_hidden();
    let outcome = mcp_control::run_headless();
    if outcome.reused {
        // INSTANCE UNIQUE : un moteur tourne déjà (desktop ou headless) — il
        // supervise déjà les serveurs ; on ne double rien, on sort proprement.
        println!(
            "mcp-control : un moteur tourne déjà sur {} — rien à lancer, sortie.",
            outcome.url
        );
        // En mode `--hidden` (autostart sans console visible), personne ne lira
        // le lien de session : inutile de consommer un jeton d'amorçage.
        if !args.iter().any(|a| a == "--hidden") {
            match mcp_control::auth::session::request_bootstrap_with_port(
                &outcome.config_path,
                std::time::Duration::from_secs(5),
            ) {
                Ok((jeton, Some(port))) => {
                    println!("[session] http://127.0.0.1:{port}/#t={jeton}");
                }
                Ok((jeton, None)) => println!(
                    "[session] {}/#t={}",
                    outcome.url.trim_end_matches('/'),
                    jeton
                ),
                Err(e) => eprintln!("[session] {e}"),
            }
        }
        return;
    }
    println!("mcp-control v{} (headless)", env!("CARGO_PKG_VERSION"));
    println!("API : {}", outcome.url);
    println!("  GET  /api/state            état complet + journal");
    println!("  GET  /api/detail?id=<id>   détail d'un serveur");
    println!("  GET  /api/store            catalogue boutique");
    println!("  POST /api/action           {{\"action\": ...}}");
    println!("Headless : aucune interface ouverte. Lance `desktop` pour la fenêtre native.");
    println!("Journal disque : mcc.log (à côté du config.json).");
    mcp_control::park_forever();
}

/// `--hidden` (autostart / lancement silencieux) : masque la fenêtre console du
/// moteur — la partie HTTP tourne en arrière-plan sans rien montrer. Sans
/// effet quand le binaire est lancé à la main sans flag (console de dev).
#[cfg(all(windows, not(test)))]
fn hide_console_if_hidden() {
    if !std::env::args().any(|a| a == "--hidden") {
        return;
    }
    // GetConsoleWindow (kernel32) + ShowWindow(SW_HIDE) (user32), sans crate externe
    #[link(name = "kernel32")]
    extern "system" {
        fn GetConsoleWindow() -> isize;
    }
    #[link(name = "user32")]
    extern "system" {
        fn ShowWindow(hwnd: isize, ncmdshow: i32) -> i32;
    }
    const SW_HIDE: i32 = 0;
    unsafe {
        let hwnd = GetConsoleWindow();
        if hwnd != 0 {
            ShowWindow(hwnd, SW_HIDE);
        }
    }
}
