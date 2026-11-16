use crate::probe;
use crate::types::{lockv, AppState, LogEntry, Origin, SState, Server, StartCmd, Transport};
use serde_json::{json, Map, Value};
#[cfg_attr(not(windows), allow(unused_imports))] // Stdio : chemins cfg(windows)
use std::process::{Command, Stdio};
use std::sync::Arc;

// log : déplacé vers manage/log.rs (v0.7.20) — réexport pour API stable.
mod log;
pub(crate) use log::*;

// ---------- config ----------

// config : déplacé vers manage/config.rs (v0.7.20) — réexport pour API stable.
mod config;
pub(crate) use config::*;
// Appelée par le binaire racine (src/main.rs) : le réexport doit être pub
// (E0603 du split v0.7.20, audit 01 constat 1).
pub use config::resolve_config_path;

// store : déplacé vers manage/store.rs (v0.7.20) — réexport pour API stable.
mod store;
pub(crate) use store::*;

/// Porté de mcp-doc.ts : accepte {"mcpServers": {...}}, {"mcp_servers": {...}}
/// ou une map brute name → entry. Retourne (nom, entry).
// import : déplacé vers manage/import.rs (v0.7.20) — réexport pour API stable.
mod import;
pub(crate) use import::*;

/// Nombre de tools actifs (tools_disabled écartés).
pub fn tools_actifs(s: &Server) -> usize {
    s.tools
        .iter()
        .filter(|t| !s.tools_disabled.contains(&t.name))
        .count()
}

/// Supervision : les sondes tournent sur un pool persistant de 12 workers
/// (manage/litemcp.rs) — chaque serveur publie son état à la fin de SA sonde,
/// sans attendre les serveurs lents (SSH, process MCP muet), qui sont sautés
/// par le cycle suivant tant que leur sonde tourne. L'ancien découpage en
/// paquets de 8 joints par `thread::scope` imposait une barrière par paquet :
/// avec 30 serveurs dont 15 en SSH, un cycle séquentiel dépassait largement
/// le refresh de 15 s.
// litemcp : déplacé vers manage/litemcp.rs (v0.7.20) — réexport pour API stable.
mod litemcp;
pub(crate) use litemcp::*;

// autostart : déplacé vers manage/autostart.rs (v0.7.20) — réexport pour API stable.
mod autostart;
pub(crate) use autostart::*;

/// Filesystem : le dossier exposé doit être présent — il faut au moins les
/// options + le package + UN chemin (le nom du package contient un « / », la
/// longueur est donc le critère fiable), et le dernier arg doit ressembler à un chemin.
fn filesystem_dossier_manquant(args: &[String]) -> bool {
    args.len() < 3
        || args
            .last()
            .map(|a| !a.contains('/') && !a.contains('\\'))
            .unwrap_or(true)
}

// events : déplacé vers manage/events.rs (v0.7.20) — réexport pour API stable.
mod events;
pub(crate) use events::*;
// scans : déplacé vers manage/scans.rs (v0.7.20) — réexport pour API stable.
mod scans;
pub(crate) use scans::*;

// peers : déplacé vers manage/peers.rs (v0.7.20) — réexport pour API stable.
mod peers;
pub(crate) use peers::*;

// actions : déplacé vers manage/actions.rs (v0.7.20) — réexport pour API stable.
mod actions;
pub(crate) use actions::*;

// update : déplacé vers manage/update.rs (v0.7.20) — réexport pour API stable.
mod update;
pub(crate) use update::*;

// cache : déplacé vers manage/cache.rs (v0.7.20) — réexport pour API stable.
mod cache;
pub(crate) use cache::*;

// json : déplacé vers manage/json.rs (v0.7.20) — réexport pour API stable.
mod json;
pub(crate) use json::*;

#[cfg(test)]
mod tests;
