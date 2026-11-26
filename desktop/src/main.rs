#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Tauri 2 : fenêtre native OS. Le contenu est servi par le moteur embarqué
// (run_headless démarré avant le builder) : la fenêtre navigue vers
// http://127.0.0.1:<port>. Aucun navigateur, aucune console : un vrai
// process applicatif Windows.
//
// Cycle de vie : fermer la fenêtre la MASQUE (l'app reste en zone de
// notification, icône tray) ; quitter se fait depuis le menu du tray.
// L'inscription au démarrage de l'OS est gérée par le core (bouton
// « Auto-boot ON/OFF » de la barre de titre, argument `--hidden` : au boot
// l'app vit dans le tray et gère les serveurs sans ouvrir de fenêtre).

use std::sync::{Arc, OnceLock};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};

struct DesktopSessionCtx {
    reused: bool,
    config_path: String,
}

static DESKTOP_CTX: OnceLock<Arc<DesktopSessionCtx>> = OnceLock::new();

/// MC-C01 : réaffiche la fenêtre principale et, si la session WebView a expiré
/// (retour du tray après > 12h/24h, `--hidden` prolongé ou F5), injecte un
/// jeton d'amorçage frais dans `window.__mccRecoverSession` sans recharger la
/// page si la session est toujours active.
fn show_window_and_recover_session(w: &tauri::WebviewWindow) {
    let _ = w.show();
    let _ = w.unminimize();
    let _ = w.set_focus();
    let Some(ctx) = DESKTOP_CTX.get() else {
        return;
    };
    let tok_opt = if ctx.reused {
        mcp_control::auth::session::request_bootstrap(
            &ctx.config_path,
            std::time::Duration::from_secs(3),
        )
        .ok()
    } else {
        mcp_control::auth::session::mint_bootstrap()
    };
    if let Some(tok) = tok_opt {
        if tok
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            let _ = w.eval(format!(
                "window.__mccRecoverSession && window.__mccRecoverSession('{tok}');"
            ));
        }
    }
}

#[cfg(windows)]
fn attach_parent_console() {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(n_std_handle: u32) -> isize;
        fn AttachConsole(dw_process_id: u32) -> i32;
    }
    const STD_OUTPUT_HANDLE: u32 = (-11i32) as u32;
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        if h == 0 || h == -1 {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

fn main() {
    // Support natif des sous-commandes CLI `bridge` et `auth` directement dans
    // le binaire Desktop (NSIS n'installe que ce binaire : l'utilisateur peut
    // lancer `mcp-control-desktop auth pair ...` ou `bridge` sans CLI séparé).
    let cli_args: Vec<String> = std::env::args().collect();
    if cli_args.get(1).map(String::as_str) == Some("bridge") {
        std::process::exit(mcp_control::bridge::run(&cli_args[2..]));
    }
    if cli_args.get(1).map(String::as_str) == Some("auth") {
        #[cfg(windows)]
        attach_parent_console();
        std::process::exit(mcp_control::auth::cli::run(&cli_args[2..]));
    }

    // Auto-mise à jour : attend la sortie réelle de l'ancienne instance si
    // relancé juste après remplacement binaire / NSIS, et nettoie `.old`.
    mcp_control::manage::wait_if_restarting_from_update();
    mcp_control::manage::cleanup_old_binary();

    // lancé au boot de l'OS (entrée autostart) → pas de fenêtre, tray seulement
    let hidden = cli_args.iter().any(|a| a == "--hidden");

    tauri::Builder::default()
        // Instance unique AVANT run_headless() : un deuxième lancement réaffiche
        // la fenêtre existante et sort immédiatement sans démarrer ni sonder un
        // second moteur ni consommer de jeton d'amorçage.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|a| a == "--hidden") {
                return;
            }
            if let Some(w) = app.get_webview_window("main") {
                show_window_and_recover_session(&w);
            }
        }))
        .setup(move |app| {
            let outcome = mcp_control::run_headless();
            let base = outcome.url.clone();
            let mut reused_verified = false;
            let ui_url = if !outcome.session_url.is_empty() {
                outcome.session_url.clone()
            } else if outcome.reused {
                match mcp_control::auth::session::request_bootstrap_with_port(
                    &outcome.config_path,
                    std::time::Duration::from_secs(5),
                ) {
                    Ok((jeton, Some(port))) => {
                        reused_verified = true;
                        format!("http://127.0.0.1:{port}/#t={jeton}")
                    }
                    Ok((jeton, None)) => {
                        reused_verified = true;
                        format!("{}/#t={}", base.trim_end_matches('/'), jeton)
                    }
                    Err(e) => {
                        eprintln!("[session] {e}");
                        base.clone()
                    }
                }
            } else {
                base.clone()
            };
            let _ = DESKTOP_CTX.set(Arc::new(DesktopSessionCtx {
                reused: outcome.reused,
                config_path: outcome.config_path.clone(),
            }));
            let ui_target: tauri::Url = ui_url.parse().expect("URL UI invalide");
            let ui_origin = ui_target.origin();
            let is_loopback = matches!(
                ui_target.host_str(),
                Some("127.0.0.1" | "localhost" | "[::1]")
            );
            // AU-C02 : en mode `reused`, n'accorder les capacités IPC Tauri
            // que si le moteur distant a prouvé appartenir au même utilisateur
            // via le canal fichier `.grant`.
            let origin_trusted = !outcome.reused || reused_verified;
            let ui_cap_origin =
                (origin_trusted && matches!(ui_target.scheme(), "http" | "https") && is_loopback)
                    .then(|| ui_origin.ascii_serialization());

            // S-5 : capacité distante réduite à l'UNIQUE origine réellement
            // servie par ce moteur (loopback:<port>) — aucune autre origine
            // locale ou distante ne reçoit les contrôles de fenêtre.
            if let Some(ref cap_origin) = ui_cap_origin {
                app.add_capability(
                    tauri::ipc::CapabilityBuilder::new("main-remote-ui-origin")
                        .local(false)
                        .remote(cap_origin.clone())
                        .remote(format!("{cap_origin}/*"))
                        .window("main")
                        .webview("main")
                        .permission("core:window:allow-start-dragging")
                        .permission("core:window:allow-minimize")
                        .permission("core:window:allow-toggle-maximize")
                        .permission("core:window:allow-internal-toggle-maximize")
                        .permission("core:window:allow-close"),
                )?;
            }
            let win = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::External(ui_target),
            )
            // Confinement de navigation : seul le tuple (http, 127.0.0.1, port
            // du moteur) est navigable — aucune redirection sortante ni popup
            // ne peut embarquer la webview ; les liens externes passent par le
            // moteur (navigateur système).
            .on_navigation(move |url| url.origin() == ui_origin)
            .on_new_window(|_url, _features| tauri::webview::NewWindowResponse::Deny)
            .title("MCP Control")
            .inner_size(1320.0, 880.0)
            .min_inner_size(940.0, 620.0)
            // Barre de titre native supprimée : le header sombre de l'UI sert de
            // barre de fenêtre (drag + réduire/agrandir/fermer via la capability
            // main-remote-ui). Évite la barre blanche OS au-dessus du header.
            .decorations(false)
            .visible(!hidden)
            // IMPÉRATIF pour le drag & drop interne : le handler de drop de
            // fichiers de WebView2 (activé par défaut) avale les événements
            // HTML5 dragstart/over/drop — on le désactive pour que les lignes
            // de l'explorateur soient déplaçables dans la fenêtre native.
            .disable_drag_drop_handler()
            // icône explicite sur la fenêtre : barre des tâches et Alt+Tab
            // prennent l'icône personnalisée (hub MCP), pas un glyphe générique
            .icon(
                app.default_window_icon()
                    .expect("icône embarquée manquante")
                    .clone(),
            )?
            .build()?;
            if !hidden {
                let _ = win.set_focus();
            }

            // icône tray : afficher le tableau de bord, quitter
            let show = MenuItem::with_id(
                app,
                "show",
                "Afficher le tableau de bord",
                true,
                None::<&str>,
            )?;
            let quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            TrayIconBuilder::with_id("mcc-tray")
                .icon(
                    app.default_window_icon()
                        .expect("icône embarquée manquante")
                        .clone(),
                )
                .tooltip("MCP Control — gestionnaire de serveurs MCP")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, ev| match ev.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            show_window_and_recover_session(&w);
                        }
                    }
                    "quit" => {
                        mcp_control::shutdown_engine();
                        app.exit(0);
                    }
                    _ => {}
                })
                // clic gauche sur l'icône : afficher / masquer la fenêtre
                .on_tray_icon_event(|tray, ev| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = ev
                    {
                        if let Some(w) = tray.app_handle().get_webview_window("main") {
                            if w.is_visible().unwrap_or(false) {
                                let _ = w.hide();
                            } else {
                                show_window_and_recover_session(&w);
                            }
                        }
                    }
                })
                .build(app)?;
            Ok(())
        })
        // fermer la fenêtre = masquer dans le tray ; vraie sortie via le tray
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("erreur au lancement de l'application desktop");
}
