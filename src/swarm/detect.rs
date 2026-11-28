//! Détection pure des instances **Swarm-control** (`swarm-control-N`) et des
//! agents MCP persistants/résidents (`swarm-<agent>`).
//!
//! Les serveurs Swarm-control sont construits sur LiteMCP (`2026-07-28`) : on
//! conserve `framework = Some("litemcp")` pour préserver l'appairage Ed25519
//! RFC 9421 et le badge LiteMCP, tout en enrichissant la sérialisation avec les
//! métadonnées Swarm (`swarm_kind`, `swarm_agent`, `swarm_instance`,
//! `swarm_bridge_port`, `swarm_workspace`).

use crate::types::{Server, Transport};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SwarmKind {
    Instance,
    Resident,
}

impl SwarmKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SwarmKind::Instance => "instance",
            SwarmKind::Resident => "resident",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SwarmMeta {
    pub kind: SwarmKind,
    pub agent: Option<String>,
    pub instance: Option<String>,
    pub bridge_port: Option<u16>,
    pub workspace: Option<String>,
}

/// Classifie un nom d'annonce `%TEMP%/litemcp-registry/<nom>.json` en instance
/// Swarm-control ou agent résident.
pub fn detect_announcement(name: &str) -> Option<SwarmKind> {
    let n = name.trim().to_ascii_lowercase();
    if n == "swarm-control" || n.starts_with("swarm-control-") {
        Some(SwarmKind::Instance)
    } else if let Some(rest) = n.strip_prefix("swarm-") {
        if !rest.is_empty() {
            Some(SwarmKind::Resident)
        } else {
            None
        }
    } else {
        None
    }
}

fn is_instance_name(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    n == "swarm-control" || n.starts_with("swarm-control-")
}

fn extract_resident_agent_from_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("swarm-") || is_instance_name(&lower) {
        return None;
    }
    let rest = &trimmed["swarm-".len()..];
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    }
}

fn cmd_parts(s: &Server) -> Option<(&str, &[String])> {
    if let Some(ref c) = s.start_cmd {
        return Some((c.program.as_str(), c.args.as_slice()));
    }
    if let Transport::Stdio { program, args, .. } = &s.transport {
        return Some((program.as_str(), args.as_slice()));
    }
    None
}

fn program_is_swarm(program: &str) -> bool {
    let stem = std::path::Path::new(program)
        .file_stem()
        .and_then(|x| x.to_str())
        .unwrap_or(program)
        .to_ascii_lowercase();
    stem == "swarm-control"
        || stem == "swarm_control"
        || stem.starts_with("swarm-control-")
        || stem.starts_with("swarm_control_")
}

fn parse_cli_flags(args: &[String]) -> (Option<String>, Option<u16>, Option<String>) {
    let mut agent: Option<String> = None;
    let mut bridge: Option<u16> = None;
    let mut workspace: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--agent" {
            if let Some(v) = args.get(i + 1) {
                if !v.trim().is_empty() {
                    agent = Some(v.trim().to_string());
                }
                i += 2;
                continue;
            }
        } else if let Some(v) = a.strip_prefix("--agent=") {
            if !v.trim().is_empty() {
                agent = Some(v.trim().to_string());
            }
        } else if a == "--bridge" {
            if let Some(v) = args.get(i + 1) {
                bridge = v.trim().parse::<u16>().ok().filter(|&p| p > 0);
                i += 2;
                continue;
            }
        } else if let Some(v) = a.strip_prefix("--bridge=") {
            bridge = v.trim().parse::<u16>().ok().filter(|&p| p > 0);
        } else if a == "--config" || a == "-c" {
            if let Some(v) = args.get(i + 1) {
                if !v.trim().is_empty() {
                    workspace = Some(v.trim().to_string());
                }
                i += 2;
                continue;
            }
        } else if let Some(v) = a.strip_prefix("--config=") {
            if !v.trim().is_empty() {
                workspace = Some(v.trim().to_string());
            }
        }
        i += 1;
    }
    (agent, bridge, workspace)
}

fn has_tool(s: &Server, name: &str) -> bool {
    s.tools.iter().any(|t| t.name == name)
}

/// Détecte si un serveur MCP Control correspond à une instance Swarm-control
/// ou à un agent résident Swarm-control.
pub fn detect_server(s: &Server) -> Option<SwarmMeta> {
    let (cli_agent, cli_bridge, cli_workspace, cli_is_swarm) = match cmd_parts(s) {
        Some((prog, args)) => {
            let (a, b, w) = parse_cli_flags(args);
            (a, b, w, program_is_swarm(prog))
        }
        None => (None, None, None, false),
    };

    // 1. Nom annoncé par le serveur MCP (initialize.serverInfo.name)
    if let Some(ref sn) = s.server_name {
        if is_instance_name(sn) {
            return Some(SwarmMeta {
                kind: SwarmKind::Instance,
                agent: None,
                instance: Some(sn.trim().to_string()),
                bridge_port: cli_bridge,
                workspace: cli_workspace,
            });
        }
        if let Some(agent) = extract_resident_agent_from_name(sn) {
            return Some(SwarmMeta {
                kind: SwarmKind::Resident,
                agent: Some(cli_agent.unwrap_or(agent)),
                instance: None,
                bridge_port: cli_bridge,
                workspace: cli_workspace,
            });
        }
    }

    // 2. Signature d'outils MCP
    let has_admin_tools = has_tool(s, "list_agents")
        && (has_tool(s, "agent_start")
            || has_tool(s, "create_agent")
            || has_tool(s, "agent_control"));
    if has_admin_tools {
        let inst = s
            .server_name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| s.label.clone());
        return Some(SwarmMeta {
            kind: SwarmKind::Instance,
            agent: None,
            instance: Some(inst),
            bridge_port: cli_bridge,
            workspace: cli_workspace,
        });
    }

    let has_any_admin_tool = has_tool(s, "list_agents")
        || has_tool(s, "create_agent")
        || has_tool(s, "delete_agent")
        || has_tool(s, "agent_start")
        || has_tool(s, "agent_stop")
        || has_tool(s, "agent_control");
    let has_resident_tools =
        has_tool(s, "ask") && has_tool(s, "status") && has_tool(s, "stop") && !has_any_admin_tool;
    if has_resident_tools
        && (s.framework.as_deref() == Some("litemcp")
            || cli_is_swarm
            || extract_resident_agent_from_name(&s.label).is_some())
    {
        let agent = cli_agent
            .or_else(|| extract_resident_agent_from_name(&s.label))
            .or_else(|| s.server_name.clone())
            .unwrap_or_else(|| s.label.clone());
        return Some(SwarmMeta {
            kind: SwarmKind::Resident,
            agent: Some(agent),
            instance: None,
            bridge_port: cli_bridge,
            workspace: cli_workspace,
        });
    }

    // 3. Commande de lancement (utile même si le serveur est DOWN / jamais sondé)
    if cli_is_swarm {
        if let Some(agent) = cli_agent {
            return Some(SwarmMeta {
                kind: SwarmKind::Resident,
                agent: Some(agent),
                instance: None,
                bridge_port: cli_bridge,
                workspace: cli_workspace,
            });
        }
        return Some(SwarmMeta {
            kind: SwarmKind::Instance,
            agent: None,
            instance: Some(s.label.clone()),
            bridge_port: cli_bridge,
            workspace: cli_workspace,
        });
    }

    // 4. Repli sur id / label (ex. serveur Boutique `store-swarm-control` avant le 1er probe)
    let id_l = s.id.to_ascii_lowercase();
    if id_l == "store-swarm-control"
        || id_l == "swarm-control"
        || id_l.starts_with("swarm-control-")
        || is_instance_name(&s.label)
    {
        return Some(SwarmMeta {
            kind: SwarmKind::Instance,
            agent: None,
            instance: Some(s.label.clone()),
            bridge_port: cli_bridge,
            workspace: cli_workspace,
        });
    }
    if let Some(agent) = extract_resident_agent_from_name(&s.label) {
        return Some(SwarmMeta {
            kind: SwarmKind::Resident,
            agent: Some(agent),
            instance: None,
            bridge_port: cli_bridge,
            workspace: cli_workspace,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Origin, SState, StartCmd, ToolInfo};

    fn dummy_server(id: &str, label: &str) -> Server {
        Server {
            id: id.into(),
            label: label.into(),
            transport: Transport::Http {
                host: None,
                port: 23080,
                path: "/mcp".into(),
            },
            origin: Origin::Manual,
            enabled: true,
            state: SState::Up,
            pid: None,
            bound_host: None,
            server_name: None,
            version: None,
            protocol: None,
            sse: false,
            sse_endpoint: false,
            ws: false,
            lang: None,
            framework: Some("litemcp".into()),
            in_container: false,
            container_name: None,
            via_ssh: false,
            tools: vec![],
            latency_ms: None,
            last_seen_ts: None,
            last_error: None,
            start_cmd: None,
            auto_restart: false,
            favorite: false,
            tools_disabled: vec![],
        }
    }

    #[test]
    fn detecte_annonces_instance_et_resident() {
        assert_eq!(
            detect_announcement("swarm-control-1"),
            Some(SwarmKind::Instance)
        );
        assert_eq!(
            detect_announcement("swarm-control"),
            Some(SwarmKind::Instance)
        );
        assert_eq!(
            detect_announcement("swarm-alice"),
            Some(SwarmKind::Resident)
        );
        assert_eq!(detect_announcement("swarm-"), None);
        assert_eq!(detect_announcement("litemcp-other"), None);
    }

    #[test]
    fn detecte_instance_par_server_name() {
        let mut s = dummy_server("srv-1", "Port 23080");
        s.server_name = Some("swarm-control-1".into());
        let m = detect_server(&s).expect("instance détectée");
        assert_eq!(m.kind, SwarmKind::Instance);
        assert_eq!(m.instance.as_deref(), Some("swarm-control-1"));
        assert_eq!(m.agent, None);
    }

    #[test]
    fn detecte_resident_par_server_name_et_bridge() {
        let mut s = dummy_server("srv-2", "Port 23081");
        s.server_name = Some("swarm-alice".into());
        s.start_cmd = Some(StartCmd {
            program: "swarm-control.exe".into(),
            args: vec![
                "--agent".into(),
                "alice".into(),
                "--bridge".into(),
                "23090".into(),
            ],
            workdir: None,
            env: vec![],
        });
        let m = detect_server(&s).expect("résident détecté");
        assert_eq!(m.kind, SwarmKind::Resident);
        assert_eq!(m.agent.as_deref(), Some("alice"));
        assert_eq!(m.bridge_port, Some(23090));
    }

    #[test]
    fn detecte_resident_par_outils_litemcp() {
        let mut s = dummy_server("srv-3", "Agent perso");
        s.tools = ["ask", "status", "stop"]
            .into_iter()
            .map(|n| ToolInfo {
                name: n.into(),
                desc: String::new(),
                input_schema: None,
            })
            .collect();
        let m = detect_server(&s).expect("résident par outils");
        assert_eq!(m.kind, SwarmKind::Resident);
    }

    #[test]
    fn detecte_resident_par_sept_outils_litemcp() {
        let mut s = dummy_server("srv-4", "Sniperbot");
        s.server_name = Some("sniperbot_analyst".into());
        s.tools = [
            "ask", "status", "stop", "history", "logs", "config", "session",
        ]
        .into_iter()
        .map(|n| ToolInfo {
            name: n.into(),
            desc: String::new(),
            input_schema: None,
        })
        .collect();
        let m = detect_server(&s).expect("résident 7 outils détecté");
        assert_eq!(m.kind, SwarmKind::Resident);
        assert_eq!(m.agent.as_deref(), Some("sniperbot_analyst"));
    }

    #[test]
    fn n_identifie_pas_admin_comme_resident_meme_avec_ask_status_stop() {
        let mut s = dummy_server("srv-5", "Serveur mixte");
        s.tools = ["ask", "status", "stop", "list_agents"]
            .into_iter()
            .map(|n| ToolInfo {
                name: n.into(),
                desc: String::new(),
                input_schema: None,
            })
            .collect();
        assert_eq!(detect_server(&s), None);
    }

    #[test]
    fn detecte_store_swarm_control_meme_down() {
        let mut s = dummy_server("store-swarm-control", "Swarm Control");
        s.state = SState::Down;
        s.framework = None;
        let m = detect_server(&s).expect("store-swarm-control détecté");
        assert_eq!(m.kind, SwarmKind::Instance);
    }

    #[test]
    fn ignore_serveur_non_swarm() {
        let mut s = dummy_server("github", "GitHub MCP");
        s.server_name = Some("github-mcp".into());
        assert_eq!(detect_server(&s), None);
    }
}
