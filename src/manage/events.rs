// Notifications d'incident (toast/webhook) + journal runtime par serveur.
// (split v0.7.20 : manage.rs reste la racine — API stable via réexport)
use super::*;

fn notify_cooldown_map(
) -> &'static std::sync::Mutex<std::collections::HashMap<(usize, String), u64>> {
    static MAP: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<(usize, String), u64>>,
    > = std::sync::OnceLock::new();
    MAP.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// NOTIFICATION d'incident : déclenchée sur une transition UP → DOWN/PORT-ONLY.
/// Deux canaux : toast desktop (Windows) et webhook HTTP. Le délai minimum
/// (`min_interval_secs`) évite le spam quand un serveur oscille.
pub(crate) fn notify_incident(app: &Arc<AppState>, label: &str, new_state: &str, detail: &str) {
    let cfg = lockv(&app.settings).notify.clone();
    if !cfg.desktop && cfg.webhook_url.trim().is_empty() {
        return;
    }
    if !cfg.wants(new_state) {
        return;
    }
    // anti-spam : pas deux notifications du même serveur dans l'intervalle [H-14]
    // (stocké hors `state.runtime` pour ne pas polluer config.json ni /api/runtime)
    let now = crate::types::now_ts();
    {
        let mut cd = lockv(notify_cooldown_map());
        let key = (Arc::as_ptr(app) as usize, label.to_string());
        if let Some(&prev) = cd.get(&key) {
            if now.saturating_sub(prev) < cfg.min_interval_secs {
                return;
            }
        }
        cd.insert(key, now);
    }
    if cfg.desktop {
        let title = format!("MCP — {label}");
        let msg = format!("{new_state} : {detail}");
        let app_bg = app.clone();
        // Exécution asynchrone : un toast lent (PowerShell) ne bloque jamais un worker de sondage [H-14].
        let _ = std::thread::Builder::new()
            .name("mcc-toast".into())
            .spawn(move || {
                if let Err(e) = crate::notify::desktop_toast(&title, &msg) {
                    add_log(&app_bg, "warn", &format!("[notif] desktop : {e}"));
                }
            });
    }
    if !cfg.webhook_url.trim().is_empty() {
        match crate::notify::send_webhook(&cfg, label, new_state, detail) {
            Ok(()) => add_log(
                app,
                "info",
                &format!("[notif] webhook envoyé : {label} {new_state}"),
            ),
            Err(e) => add_log(app, "warn", &format!("[notif] webhook : {e}")),
        }
    }
}
/// Enregistre un événement runtime (crash, redémarrage, succès…) et le
/// persiste — l'historique survit au redémarrage de l'application.
pub(crate) fn runtime_event(state: &AppState, id: &str, kind: &str, detail: &str) {
    let label = lockv(&state.servers)
        .iter()
        .find(|s| s.id == id)
        .map(|s| s.label.clone())
        .unwrap_or_else(|| id.to_string());
    {
        let mut rt = lockv(&state.runtime);
        let entry = rt.entry(id.to_string()).or_default();
        match kind {
            "crash" => entry.crashes += 1,
            "restart" | "recovered" => entry.restarts += 1,
            _ => {}
        }
        if kind == "recovered" || kind == "start-ok" {
            entry.fails = 0;
            entry.last_success_ts = crate::types::now_ts();
        }
        entry.push_event(crate::runtime::RuntimeEvent {
            ts: crate::types::now_ts(),
            id: id.to_string(),
            label: label.clone(),
            kind: kind.to_string(),
            detail: detail.to_string(),
        });
    }
    save_config(state);
}
/// Compte un échec de spawn auto pour l'id ; à partir de 3 échecs consécutifs,
/// désactive l'auto-restart (sinon boucle infinie d'essais + warnings sur une
/// machine cliente où les chemins des seeds n'existent pas).
pub(crate) fn note_spawn_failure(state: &AppState, id: &str) {
    // Ordre des verrous : `servers` lu et relâché AVANT `runtime` (jamais
    // `servers` sous `runtime` — prévient tout interblocage) [H-13].
    let label = lockv(&state.servers)
        .iter()
        .find(|s| s.id == id)
        .map(|s| s.label.clone())
        .unwrap_or_else(|| id.to_string());
    let (fails, backoff, disable) = {
        let mut rt = lockv(&state.runtime);
        let entry = rt.entry(id.to_string()).or_default();
        entry.fails += 1;
        entry.last_attempt_ts = crate::types::now_ts();
        entry.push_event(crate::runtime::RuntimeEvent {
            ts: crate::types::now_ts(),
            id: id.to_string(),
            label: label.clone(),
            kind: "start-fail".into(),
            detail: format!("{} échec(s) consécutif(s)", entry.fails),
        });
        (
            entry.fails,
            entry.backoff_secs(),
            crate::runtime::should_disable_autoboot(entry.fails),
        )
    };
    if disable {
        {
            let mut servers = lockv(&state.servers);
            if let Some(srv) = servers.iter_mut().find(|s| s.id == id) {
                srv.auto_restart = false;
            }
        }
        add_log(
            state,
            "warn",
            &format!(
                "[{}] auto-restart désactivé après {} échecs de démarrage consécutifs",
                id, fails
            ),
        );
    } else {
        add_log(
            state,
            "warn",
            &format!(
                "[{}] échec de démarrage #{} — nouvelle tentative dans {} s",
                id, fails, backoff
            ),
        );
    }
    // PERSISTANCE : le compteur d'échecs et le backoff doivent survivre au
    // redémarrage de l'app (sinon la boucle d'échecs repart de zéro).
    save_config(state);
}
