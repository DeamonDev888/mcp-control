//! Support natif de **Swarm-control** (instances admin `swarm-control-N` et
//! agents MCP persistants/résidents `swarm-<agent>`, construits sur LiteMCP).
//!
//! Architecture :
//! - `detect` : détection pure d'une instance ou d'un agent résident (métadonnées
//!   exposées dans `/api/state` et `/api/detail` sans altérer `framework = "litemcp"`).
//! - `rpc` : appels MCP vers un serveur Swarm (HTTP/SSH/stdio) + dépliage JSON
//!   LiteMCP + client loopback minimal pour le `--bridge` HTTP optionnel.
//! - `jobs` : suivi borné des appels longs asynchrones (`ask`, `run_agents_parallel`).
//! - `actions` : dispatcher des actions locales `swarm_*` (réservées `ADMIN_ACTIONS`).

pub mod actions;
pub mod detect;
pub mod jobs;
pub mod logos;
pub mod rpc;

pub use actions::{
    get_swarm_class, handle_swarm_action, resolve_server_swarm_class, snapshot_swarm_classes,
};
pub use detect::{detect_announcement, detect_server, SwarmKind, SwarmMeta};
