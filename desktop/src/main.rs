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

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};

fn main() {
    // Auto-mise à jour : attend la sortie de l'ancienne instance si relancé
    // juste après remplacement binaire, et nettoie un éventuel `.old`.
    mcp_control::manage::wait_if_restarting_from_update();
    mcp_control::manage::cleanup_old_binary();
    // Instance unique garantie à deux niveaux : (1) tauri_plugin_single_instance
    // ci-dessous réaffiche la fenêtre existante si le desktop est relancé ;
    // (2) run_headless se RECONNECTE à un moteur déjà en cours (headless ou
    // desktop) au lieu d'en lancer un second — un seul processus supervise.
    let outcome = mcp_control::run_headless();
    // SESSION LOCALE (docs/auth.md §6) : le shell navigue vers l'URL à usage
    // unique `http://127.0.0.1:<port>/#t=<jeton>` — le fragment n'est jamais
    // envoyé au serveur, `app.js` l'échange contre le cookie `mcc_session` puis
    // l'efface. Aucun jeton à demander à l'utilisateur.
    // Si le moteur a été RÉUTILISÉ (un moteur tournait déjà, desktop ou
    // headless), le shell demande un jeton par le canal fichier du dossier de
    // config (S-1 pt 3) : même utilisateur seulement, 2 min, usage unique.
    let base = outcome.url.clone();
    let ui_url = if !outcome.session_url.is_empty() {
        outcome.session_url.clone()
    } else if outcome.reused {
        match mcp_control::auth::session::request_bootstrap(
            &outcome.config_path,
            std::time::Duration::from_secs(5),
        ) {
            Ok(jeton) => format!("{}/#t={}", base.trim_end_matches('/'), jeton),
            // Aucun jeton journalisé : seule l'absence est signalée.
            Err(e) => {
                eprintln!("[session] {e}");
                base.clone()
            }
        }
    } else {
        base.clone()
    };
    // lancé au boot de Windows (entrée autostart) → pas de fenêtre, tray seulement
    let hidden = std::env::args().any(|a| a == "--hidden");
    // SEULE origine navigable : celle que le moteur vient de servir. Le
    // fragment #t=<jeton> ne compte pas dans l'origine. Toute autre navigation
    // est refusée : la fenêtre ne peut pas quitter l'UI (les liens externes
    // passent par le moteur, action open_release_page → navigateur système).
    let ui_target: tauri::Url = ui_url.parse().expect("URL UI invalide");
    let ui_origin = ui_target.origin();
    let is_loopback = matches!(
        ui_target.host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]")
    );
    let ui_cap_origin = (matches!(ui_target.scheme(), "http" | "https") && is_loopback)
        .then(|| ui_origin.ascii_serialization());

    tauri::Builder::default()
        // deuxième lancement pendant que l'app tourne : réafficher la fenêtre
        // existante au lieu d'un deuxième moteur qui mourrait sur les ports UI
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .setup(move |app| {
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
            let _ = win.set_focus();

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
                            let _ = w.show();
                            let _ = w.unminimize();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
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
                                let _ = w.show();
                                let _ = w.unminimize();
                                let _ = w.set_focus();
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
