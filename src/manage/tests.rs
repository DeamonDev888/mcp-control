use super::*;
use crate::types::{AppState, Settings};
use std::collections::VecDeque;
use std::sync::Mutex;

#[test]
fn store_catalogue_sain() {
    // ids et commandes uniques (install_store clé les serveurs sur "store-<id>"),
    // kinds et programmes connus
    let mut ids = std::collections::HashSet::new();
    let mut cmds: std::collections::HashMap<(&str, Vec<&str>), &str> =
        std::collections::HashMap::new();
    for it in all_store_items() {
        assert!(
            !it.id.is_empty() && !it.name.is_empty() && !it.desc.is_empty(),
            "{}: champs vides",
            it.id
        );
        assert!(ids.insert(it.id), "id store dupliqué : {}", it.id);
        let cmd_key = (it.program, it.args.to_vec());
        if let Some(prev_id) = cmds.insert(cmd_key, it.id) {
            panic!(
                "commande store dupliquée entre '{}' et '{}' : {} {:?}",
                prev_id, it.id, it.program, it.args
            );
        }
        assert!(
            it.kind == "npm" || it.kind == "python",
            "{}: kind inconnu {}",
            it.id,
            it.kind
        );
        assert!(
            it.program == "npx" || it.program == "uvx",
            "{}: programme inconnu {}",
            it.id,
            it.program
        );
        assert!(!it.args.is_empty(), "{}: args vides", it.id);
    }
    assert!(
        STORE.len() >= 29,
        "le catalogue principal doit garder les 14 originaux + les populaires"
    );
    assert!(
        !crate::store_aitmpl::STORE_AITMPL.is_empty(),
        "le catalogue aitmpl ne doit pas être vide"
    );
}

fn tmp_config(nom: &str, contenu: Option<&str>) -> String {
    let dir = std::env::temp_dir().join("mcpcc_tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(nom);
    if let Some(c) = contenu {
        std::fs::write(&path, c).unwrap();
    }
    path.to_string_lossy().to_string()
}

#[test]
fn installation_neuve_demarre_sans_serveur() {
    // Audit S4, point 8 : plus AUCUN serveur ni chemin propre à une machine.
    // Une installation neuve = liste vide (la découverte la remplit) ; les
    // réglages par défaut restent ceux du produit.
    let (servers, st) = load_config(&tmp_config("inexistant_aucune_chance.json", None));
    assert!(
        servers.is_empty(),
        "default_servers() doit être vide, obtenu : {:?}",
        servers.iter().map(|s| &s.id).collect::<Vec<_>>()
    );
    assert_eq!(st.refresh_secs, 15);
    assert_eq!(st.ui_port, 5090);
    assert!(!default_servers().iter().any(|s| s.start_cmd.is_some()));
}

#[test]
fn migration_les_entrees_seed_existantes_restent() {
    // Migration : un config.json écrit par une version antérieure garde ses
    // entrées `seed-*` telles quelles (relues comme des serveurs manuels).
    // Rien n'est réinjecté au démarrage, `removed_seeds` ne les supprime pas
    // puisqu'elles sont persistées.
    let cfg = r#"{"servers":[
        {"id":"seed-overmind","label":"Overmind","transport":"http","port":3099,"path":"/mcp","program":"node","args":["C:/ws/Workflow/bin/launch.cjs"],"workdir":"C:/ws/Workflow","enabled":true},
        {"id":"seed-news","label":"NEWS","transport":"http","port":3144,"path":"/mcp","enabled":false}
    ],"settings":{"removed_seeds":["seed-news"]}}"#;
    let (servers, st) = load_config(&tmp_config("cfg_migration_seeds.json", Some(cfg)));
    assert_eq!(servers.len(), 2, "les deux entrées persistées restent");
    let ov = servers.iter().find(|s| s.id == "seed-overmind").unwrap();
    assert_eq!(ov.origin, Origin::Manual, "relu comme un serveur manuel");
    assert_eq!(ov.label, "Overmind");
    assert_eq!(
        ov.start_cmd.as_ref().map(|c| c.program.as_str()),
        Some("node")
    );
    assert!(
        servers.iter().any(|s| s.id == "seed-news" && !s.enabled),
        "removed_seeds ne retire pas une entrée persistée"
    );
    assert_eq!(st.removed_seeds, vec!["seed-news".to_string()]);
}

#[test]
fn seed_retire_durablement_nest_pas_reinjecte() {
    // « Retirer durablement » : settings.removed_seeds ne réintroduit AUCUN
    // serveur au chargement. Depuis le lot S4 il n'y a plus de seeds par
    // défaut : le réglage reste lu (compatibilité) et une config qui ne
    // contient rien donne une liste vide.
    let cfg = "{\"settings\":{\"removed_seeds\":[\"seed-news\"]}}";
    let (servers, st) = load_config(&tmp_config("cfg_seed_retire.json", Some(cfg)));
    assert!(
        servers.is_empty(),
        "aucun seed ne doit être réinjecté : {:?}",
        servers.iter().map(|s| &s.id).collect::<Vec<_>>()
    );
    assert_eq!(st.removed_seeds, vec!["seed-news".to_string()]);
}

#[test]
fn remove_durable_consigne_le_seed_dans_la_config() {
    // Roundtrip complet : action API « remove durable » → config.json
    // → rechargement : le seed n'est plus là
    let path = tmp_config("cfg_remove_durable.json", None);
    let mut seed = srv_http("seed-news", None, "NEWS", None, 3144, Origin::Seed, None);
    seed.enabled = false;
    seed.state = SState::Disabled;
    let state = Arc::new(AppState {
        servers: Mutex::new(vec![seed]),
        log: Mutex::new(VecDeque::new()),
        settings: Mutex::new(Settings::default()),
        children: Mutex::new(std::collections::HashMap::new()),
        config_path: path.clone(),
        peers: Mutex::new(std::collections::HashMap::new()),
        allowed_origins: Mutex::new(Vec::new()),
        allowed_hosts: Mutex::new(Vec::new()),
        spawn_fails: Mutex::new(std::collections::HashMap::new()),
        runtime: Mutex::new(crate::runtime::RuntimeLog::new()),
        spawning: Mutex::new(std::collections::HashSet::new()),
        update_latest: Mutex::new(None),
        lan_ok: Mutex::new(None),
    });
    let j = handle_action(
        &state,
        "{\"action\":\"remove\",\"id\":\"seed-news\",\"durable\":true}",
    );
    assert_eq!(j["ok"], serde_json::json!(true));
    let (servers, st) = load_config(&path);
    assert!(
        servers.iter().all(|s| s.id != "seed-news"),
        "le seed ne doit pas revenir au rechargement"
    );
    assert_eq!(st.removed_seeds, vec!["seed-news".to_string()]);
}

#[test]
fn version_newer_compare_numeriquement() {
    assert!(version_newer("0.7.1", "0.7.0"));
    assert!(version_newer("v0.8.0", "0.7.9"), "préfixe v toléré");
    assert!(
        version_newer("0.10.0", "0.9.9"),
        "numérique, pas lexicographique"
    );
    assert!(version_newer("1.0", "0.9.9"), "longueurs différentes");
    assert!(
        !version_newer("0.6.4", "0.7.0"),
        "la release publiée est plus ancienne"
    );
    assert!(
        !version_newer("0.7.0", "0.7.0"),
        "égale = pas de mise à jour"
    );
    assert!(
        !version_newer("abc", "0.7.0"),
        "tag non numérique = pas plus récent"
    );
}

#[test]
fn autostart_est_persiste_et_expose() {
    let path = tmp_config(
        "cfg_autostart.json",
        Some("{\"settings\":{\"autostart\":false}}"),
    );
    let (_, st) = load_config(&path);
    assert!(!st.autostart, "autostart: false lu depuis la config");
    let state = test_state();
    {
        let mut s = lockv(&state.settings);
        s.autostart = false;
    }
    let d = state_json(&state);
    assert_eq!(d["autostart"], serde_json::json!(false));
    // l'action reflète la valeur demandée — apply_autostart est neutralisé
    // sous cfg(test) (jamais d'écriture OS depuis les tests). confirm:true
    // évite de dépendre d'une éventuelle entrée Run réelle sur la machine hôte.
    let j = handle_action(
        &state,
        "{\"action\":\"set_autostart\",\"enabled\":true,\"confirm\":true}",
    );
    assert_eq!(j["ok"], serde_json::json!(true));
    assert!(lockv(&state.settings).autostart);
    // NB : j["active"] reflète le registre/fichier RÉEL — non testé (effet OS)
}

#[test]
fn reorder_fixe_l_ordre_d_affichage_et_de_sondage() {
    let path = tmp_config("cfg_reorder.json", None);
    let mk = |ids: &[&str]| {
        Arc::new(AppState {
            servers: Mutex::new(
                ids.iter()
                    .map(|id| Server {
                        id: id.to_string(),
                        label: id.to_string(),
                        transport: Transport::Http {
                            host: None,
                            port: 1,
                            path: "/mcp".into(),
                        },
                        origin: Origin::Seed,
                        enabled: true,
                        auto_restart: false,
                        favorite: false,
                        state: SState::Down,
                        pid: None,
                        bound_host: None,
                        server_name: None,
                        version: None,
                        protocol: None,
                        sse: false,
                        sse_endpoint: false,
                        ws: false,
                        lang: None,
                        framework: None,
                        via_ssh: false,
                        tools: vec![],
                        latency_ms: None,
                        last_seen_ts: None,
                        last_error: None,
                        start_cmd: None,
                        tools_disabled: vec![],
                    })
                    .collect(),
            ),
            log: Mutex::new(VecDeque::new()),
            settings: Mutex::new(Settings::default()),
            children: Mutex::new(std::collections::HashMap::new()),
            config_path: path.clone(),
            peers: Mutex::new(std::collections::HashMap::new()),
            allowed_origins: Mutex::new(Vec::new()),
            allowed_hosts: Mutex::new(Vec::new()),
            spawn_fails: Mutex::new(std::collections::HashMap::new()),
            runtime: Mutex::new(crate::runtime::RuntimeLog::new()),
            spawning: Mutex::new(std::collections::HashSet::new()),
            update_latest: Mutex::new(None),
            lan_ok: Mutex::new(None),
        })
    };
    let state = mk(&["a", "b", "c"]);
    // 1. reorder avec ids inconnus filtrés + ordre appliqué
    let j = handle_action(
        &state,
        "{\"action\":\"reorder\",\"ids\":[\"c\",\"zzz\",\"b\"]}",
    );
    assert_eq!(j["ok"], serde_json::json!(true));
    assert_eq!(
        lockv(&state.settings).server_order,
        vec!["c".to_string(), "b".to_string(), "a".to_string()],
        "inconnu filtré, manquant (a) ajouté en fin"
    );
    // 2. l'affichage suit l'ordre
    let d = state_json(&state);
    let ids: Vec<String> = d["servers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, vec!["c", "b", "a"]);
    // 3. remove purge l'ordre
    handle_action(&state, "{\"action\":\"remove\",\"id\":\"c\"}");
    assert_eq!(
        lockv(&state.settings).server_order,
        vec!["b".to_string(), "a".to_string()]
    );
}

#[test]
fn server_order_est_persiste_et_recharge() {
    let path = tmp_config(
        "cfg_ordre.json",
        Some("{\"settings\":{\"server_order\":[\"b\",\"a\"]}}"),
    );
    let (_, st) = load_config(&path);
    assert_eq!(st.server_order, vec!["b".to_string(), "a".to_string()]);
}

#[test]
fn cache_roundtrip_restaure_le_dernier_etat() {
    let dir = std::env::temp_dir().join(format!("mcc-cache-rt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.json").to_string_lossy().to_string();
    let mk = |state_etat: SState, tools: bool| {
        Arc::new(AppState {
            servers: Mutex::new(vec![Server {
                id: "seed-overmind".into(),
                label: "Overmind".into(),
                transport: Transport::Http {
                    host: None,
                    port: 3099,
                    path: "/mcp".into(),
                },
                origin: Origin::Seed,
                enabled: true,
                auto_restart: true,
                favorite: false,
                state: state_etat,
                pid: None,
                bound_host: Some("127.0.0.1".into()),
                server_name: if tools {
                    Some("OverMind-MCP".into())
                } else {
                    None
                },
                version: if tools { Some("3.10.5".into()) } else { None },
                protocol: None,
                sse: false,
                sse_endpoint: false,
                ws: false,
                lang: Some("node".into()),
                framework: None,
                via_ssh: false,
                tools: if tools {
                    vec![
                        crate::types::ToolInfo {
                            name: "a".into(),
                            desc: "premier".into(),
                        },
                        crate::types::ToolInfo {
                            name: "b".into(),
                            desc: String::new(),
                        },
                    ]
                } else {
                    vec![]
                },
                latency_ms: if tools { Some(3) } else { None },
                last_seen_ts: if tools { Some(1_700_000_000) } else { None },
                last_error: None,
                start_cmd: None,
                tools_disabled: vec![],
            }]),
            log: Mutex::new(VecDeque::new()),
            settings: Mutex::new(Settings::default()),
            children: Mutex::new(std::collections::HashMap::new()),
            config_path: path.clone(),
            peers: Mutex::new(std::collections::HashMap::new()),
            allowed_origins: Mutex::new(Vec::new()),
            allowed_hosts: Mutex::new(Vec::new()),
            spawn_fails: Mutex::new(std::collections::HashMap::new()),
            runtime: Mutex::new(crate::runtime::RuntimeLog::new()),
            spawning: Mutex::new(std::collections::HashSet::new()),
            update_latest: Mutex::new(None),
            lan_ok: Mutex::new(None),
        })
    };
    // 1. état riche -> cache
    save_cache(&mk(SState::Up, true));
    // 2. nouvel état vide (comme au boot) -> load
    let fresh = mk(SState::Down, false);
    load_cache(&fresh);
    let s = lockv(&fresh.servers).first().unwrap().clone();
    assert_eq!(
        s.state,
        SState::Up,
        "l'état UP est restauré avant la première sonde"
    );
    assert_eq!(s.server_name.as_deref(), Some("OverMind-MCP"));
    assert_eq!(s.version.as_deref(), Some("3.10.5"));
    assert_eq!(s.tools.len(), 2);
    assert_eq!(s.tools[0].name, "a");
    assert_eq!(s.latency_ms, Some(3));
    assert_eq!(s.last_seen_ts, Some(1_700_000_000));
    // 3. serveur désactivé : le cache ne le touche pas
    let dis = mk(SState::Down, false);
    {
        let mut sv = lockv(&dis.servers);
        sv.first_mut().unwrap().enabled = false;
    }
    load_cache(&dis);
    let s2 = lockv(&dis.servers).first().unwrap().clone();
    assert_eq!(
        s2.state,
        SState::Down,
        "serveur désactivé non restauré en UP"
    );
}

#[test]
fn aucun_serveur_par_defaut_ne_porte_de_chemin_de_machine() {
    // Garde du point 8 : la liste par défaut est vide, donc aucun chemin
    // `C:/Users/...` ni serveur personnel ne peut être lancé sur une
    // installation neuve. Le test échoue si quelqu'un réintroduit un seed.
    let servers = default_servers();
    assert!(
        servers.is_empty(),
        "aucun serveur par défaut attendu, obtenu : {:?}",
        servers.iter().map(|s| &s.id).collect::<Vec<_>>()
    );
    let cfg = tmp_config("cfg_defauts_vides.json", None);
    let (charges, _) = load_config(&cfg);
    assert!(charges.is_empty());
}

#[test]
fn load_config_reactive_un_seed_et_ajoute_un_inconnu() {
    // Une entrée `seed-*` persistée est relue comme n'importe quel serveur
    // (plus de seed par défaut à « réactiver ») ; un inconnu est ajouté.
    let cfg = "{\"servers\":[{\"id\":\"seed-debats\",\"label\":\"debats\",\"transport\":\"http\",\"port\":3100,\"path\":\"/mcp\",\"enabled\":true},{\"id\":\"mon-mcp\",\"label\":\"Mon MCP\",\"transport\":\"http\",\"port\":4567,\"path\":\"/x\",\"enabled\":false}]}";
    let (servers, _) = load_config(&tmp_config("cfg_mixte.json", Some(cfg)));
    let debats = servers.iter().find(|s| s.id == "seed-debats").unwrap();
    assert!(debats.enabled);
    assert_eq!(debats.state, SState::Down);
    assert_eq!(
        debats.origin,
        Origin::Manual,
        "une entrée persistée ne redevient pas un seed"
    );
    let m = servers.iter().find(|s| s.id == "mon-mcp").unwrap();
    assert_eq!(m.origin, Origin::Manual);
    assert!(!m.enabled);
    match &m.transport {
        Transport::Http { host, port, path } => {
            assert!(host.is_none());
            assert_eq!(*port, 4567);
            assert_eq!(path, "/x");
        }
        _ => panic!("transport http attendu"),
    }
}

#[test]
fn load_config_reconstruit_start_cmd_stdio_persiste() {
    // LE bug v0.1 : un stdio recharge du disque sans start_cmd n'etait jamais demarre au boot
    let cfg = "{\"servers\":[{\"id\":\"man-s\",\"label\":\"S\",\"transport\":\"stdio\",\"program\":\"node\",\"args\":[\"a.js\",\"--p\",\"1\"],\"workdir\":\"C:/w\",\"enabled\":true}]}";
    let (servers, _) = load_config(&tmp_config("cfg_stdio.json", Some(cfg)));
    let s = servers.iter().find(|s| s.id == "man-s").unwrap();
    let cmd = s
        .start_cmd
        .as_ref()
        .expect("start_cmd doit etre reconstruit au load");
    assert_eq!(cmd.program, "node");
    assert_eq!(
        cmd.args,
        vec!["a.js".to_string(), "--p".to_string(), "1".to_string()]
    );
    assert_eq!(cmd.workdir.as_deref(), Some("C:/w"));
    assert!(s.auto_restart);
}

#[test]
fn save_puis_load_roundtrip_stdio() {
    let path = tmp_config("cfg_roundtrip.json", None);
    let state = AppState {
        servers: Mutex::new(Vec::new()),
        log: Mutex::new(VecDeque::new()),
        settings: Mutex::new(Settings::default()),
        children: Mutex::new(std::collections::HashMap::new()),
        peers: Mutex::new(std::collections::HashMap::new()),
        allowed_origins: Mutex::new(Vec::new()),
        allowed_hosts: Mutex::new(Vec::new()),
        spawn_fails: Mutex::new(std::collections::HashMap::new()),
        runtime: Mutex::new(crate::runtime::RuntimeLog::new()),
        spawning: Mutex::new(std::collections::HashSet::new()),
        update_latest: Mutex::new(None),
        lan_ok: Mutex::new(None),
        config_path: path.clone(),
    };
    {
        let mut servers = lockv(&state.servers);
        servers.push(Server {
            id: "rt1".into(),
            label: "RT".into(),
            transport: Transport::Stdio {
                program: "prog".into(),
                args: vec!["-x".into()],
                workdir: Some("W".into()),
                env: Vec::new(),
            },
            origin: Origin::Manual,
            enabled: true,
            auto_restart: true,
            favorite: false,
            state: SState::Up,
            pid: Some(42),
            bound_host: None,
            server_name: Some("n".into()),
            version: Some("1".into()),
            protocol: None,
            sse: false,
            sse_endpoint: false,
            ws: false,
            lang: None,
            framework: None,
            via_ssh: false,
            tools: vec![],
            latency_ms: Some(3),
            last_seen_ts: Some(1),
            last_error: None,
            start_cmd: Some(StartCmd {
                program: "prog".into(),
                args: vec!["-x".into()],
                workdir: Some("W".into()),
                env: Vec::new(),
            }),
            tools_disabled: vec![],
        });
    }
    save_config(&state);
    let (servers, _) = load_config(&path);
    let s = servers
        .iter()
        .find(|s| s.id == "rt1")
        .expect("doit etre persiste");
    match &s.transport {
        Transport::Stdio {
            program,
            args,
            workdir,
            ..
        } => {
            assert_eq!(program, "prog");
            assert_eq!(args, &vec!["-x".to_string()]);
            assert_eq!(workdir.as_deref(), Some("W"));
        }
        _ => panic!("stdio attendu apres round-trip"),
    }
    assert!(s.start_cmd.is_some(), "start_cmd survit au round-trip");
}

#[test]
fn settings_sont_parsees() {
    let cfg = "{\"servers\":[],\"settings\":{\"refresh_secs\":42,\"scan_from\":1000,\"scan_to\":1009,\"ui_port\":6000}}";
    let (_, st) = load_config(&tmp_config("cfg_settings.json", Some(cfg)));
    assert_eq!(st.refresh_secs, 42);
    assert_eq!((st.scan_from, st.scan_to), (1000, 1009));
    assert_eq!(st.ui_port, 6000);
}

#[test]
fn toggle_tool_desactive_et_reactive() {
    let state = AppState {
        servers: Mutex::new(vec![Server {
            id: "s".into(),
            label: "S".into(),
            transport: Transport::Http {
                host: None,
                port: 1,
                path: "/mcp".into(),
            },
            origin: Origin::Seed,
            enabled: true,
            auto_restart: true,
            favorite: false,
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
            framework: None,
            via_ssh: false,
            tools: vec![
                crate::types::ToolInfo {
                    name: "a".into(),
                    desc: String::new(),
                },
                crate::types::ToolInfo {
                    name: "b".into(),
                    desc: String::new(),
                },
            ],
            latency_ms: None,
            last_seen_ts: None,
            last_error: None,
            start_cmd: None,
            tools_disabled: vec![],
        }]),
        log: Mutex::new(VecDeque::new()),
        settings: Mutex::new(Settings::default()),
        children: Mutex::new(std::collections::HashMap::new()),
        peers: Mutex::new(std::collections::HashMap::new()),
        allowed_origins: Mutex::new(Vec::new()),
        allowed_hosts: Mutex::new(Vec::new()),
        spawn_fails: Mutex::new(std::collections::HashMap::new()),
        runtime: Mutex::new(crate::runtime::RuntimeLog::new()),
        spawning: Mutex::new(std::collections::HashSet::new()),
        update_latest: Mutex::new(None),
        lan_ok: Mutex::new(None),
        config_path: String::new(),
    };
    let arc = Arc::new(state);
    let body = serde_json::to_string(
        &serde_json::json!({"action": "toggle_tool", "id": "s", "tool": "a"}),
    )
    .unwrap();
    let j = handle_action(&arc, &body);
    assert_eq!(j["ok"], json!(true));
    assert_eq!(j["disabled"], json!(true));
    assert_eq!(j["tools_actifs"], json!(1));
    let j2 = handle_action(&arc, &body);
    assert_eq!(j2["disabled"], json!(false));
    assert_eq!(j2["tools_actifs"], json!(2));
}

#[test]
fn tools_actifs_exclut_les_desactives() {
    let state = AppState {
        servers: Mutex::new(vec![Server {
            id: "x".into(),
            label: "X".into(),
            transport: Transport::Http {
                host: None,
                port: 9,
                path: "/mcp".into(),
            },
            origin: Origin::Seed,
            enabled: true,
            auto_restart: true,
            favorite: false,
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
            framework: None,
            via_ssh: false,
            tools: vec![
                crate::types::ToolInfo {
                    name: "a".into(),
                    desc: String::new(),
                },
                crate::types::ToolInfo {
                    name: "b".into(),
                    desc: String::new(),
                },
                crate::types::ToolInfo {
                    name: "c".into(),
                    desc: String::new(),
                },
            ],
            latency_ms: None,
            last_seen_ts: None,
            last_error: None,
            start_cmd: None,
            tools_disabled: vec!["a".into(), "c".into()],
        }]),
        log: Mutex::new(VecDeque::new()),
        settings: Mutex::new(Settings::default()),
        children: Mutex::new(std::collections::HashMap::new()),
        peers: Mutex::new(std::collections::HashMap::new()),
        allowed_origins: Mutex::new(Vec::new()),
        allowed_hosts: Mutex::new(Vec::new()),
        spawn_fails: Mutex::new(std::collections::HashMap::new()),
        runtime: Mutex::new(crate::runtime::RuntimeLog::new()),
        spawning: Mutex::new(std::collections::HashSet::new()),
        update_latest: Mutex::new(None),
        lan_ok: Mutex::new(None),
        config_path: String::new(),
    };
    let servers = lockv(&state.servers);
    assert_eq!(tools_actifs(&servers[0]), 1);
}

#[test]
fn state_json_compte_bon_les_stats() {
    let state = AppState {
        servers: Mutex::new(vec![
            Server {
                id: "a".into(),
                label: "A".into(),
                transport: Transport::Http {
                    host: None,
                    port: 1,
                    path: "/mcp".into(),
                },
                origin: Origin::Seed,
                enabled: true,
                auto_restart: true,
                favorite: false,
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
                framework: None,
                via_ssh: false,
                tools: vec![crate::types::ToolInfo {
                    name: "t".into(),
                    desc: String::new(),
                }],
                latency_ms: None,
                last_seen_ts: None,
                last_error: None,
                start_cmd: None,
                tools_disabled: vec![],
            },
            Server {
                id: "b".into(),
                label: "B".into(),
                transport: Transport::Http {
                    host: None,
                    port: 2,
                    path: "/mcp".into(),
                },
                origin: Origin::Seed,
                enabled: false,
                auto_restart: true,
                favorite: false,
                state: SState::Disabled,
                pid: None,
                bound_host: None,
                server_name: None,
                version: None,
                protocol: None,
                sse: false,
                sse_endpoint: false,
                ws: false,
                lang: None,
                framework: None,
                via_ssh: false,
                tools: vec![],
                latency_ms: None,
                last_seen_ts: None,
                last_error: None,
                start_cmd: None,
                tools_disabled: vec![],
            },
            Server {
                id: "c".into(),
                label: "C".into(),
                transport: Transport::Http {
                    host: None,
                    port: 3,
                    path: "/mcp".into(),
                },
                origin: Origin::Seed,
                enabled: true,
                auto_restart: true,
                favorite: false,
                state: SState::PortOnly,
                pid: None,
                bound_host: None,
                server_name: None,
                version: None,
                protocol: None,
                sse: false,
                sse_endpoint: false,
                ws: false,
                lang: None,
                framework: None,
                via_ssh: false,
                // tools en cache d'un poll précédent alors que le serveur est PORT-ONLY :
                // ne doivent PAS gonfler stats.tools
                tools: vec![
                    crate::types::ToolInfo {
                        name: "t_stale_1".into(),
                        desc: String::new(),
                    },
                    crate::types::ToolInfo {
                        name: "t_stale_2".into(),
                        desc: String::new(),
                    },
                ],
                latency_ms: None,
                last_seen_ts: None,
                last_error: None,
                start_cmd: None,
                tools_disabled: vec![],
            },
        ]),
        log: Mutex::new(VecDeque::new()),
        settings: Mutex::new(Settings::default()),
        children: Mutex::new(std::collections::HashMap::new()),
        peers: Mutex::new(std::collections::HashMap::new()),
        allowed_origins: Mutex::new(Vec::new()),
        allowed_hosts: Mutex::new(Vec::new()),
        spawn_fails: Mutex::new(std::collections::HashMap::new()),
        runtime: Mutex::new(crate::runtime::RuntimeLog::new()),
        spawning: Mutex::new(std::collections::HashSet::new()),
        update_latest: Mutex::new(None),
        lan_ok: Mutex::new(None),
        config_path: String::new(),
    };
    let j = state_json(&Arc::new(state));
    assert_eq!(j["stats"]["total"], serde_json::json!(3));
    assert_eq!(j["stats"]["up"], serde_json::json!(1));
    assert_eq!(j["stats"]["tools"], serde_json::json!(1));
    assert_eq!(j["servers"][1]["state"], serde_json::json!("DISABLED"));
    assert_eq!(j["servers"][1]["origin"], serde_json::json!("seed"));
    assert_eq!(j["servers"][2]["state"], serde_json::json!("PORT-ONLY"));
}

#[test]
fn save_config_chemin_vide_ne_cree_pas_point_tmp() {
    let state = test_state();
    assert!(state.config_path.is_empty());
    let existait_avant = std::path::Path::new(".tmp").exists();
    assert!(save_config_checked(&state).is_ok());
    save_config(&state);
    if !existait_avant {
        assert!(
            !std::path::Path::new(".tmp").exists(),
            "save_config avec config_path vide ne doit jamais écrire '.tmp'"
        );
    }
}

fn test_state() -> Arc<AppState> {
    Arc::new(AppState {
        servers: Mutex::new(vec![]),
        log: Mutex::new(VecDeque::new()),
        settings: Mutex::new(Settings::default()),
        children: Mutex::new(std::collections::HashMap::new()),
        config_path: String::new(),
        peers: Mutex::new(std::collections::HashMap::new()),
        allowed_origins: Mutex::new(Vec::new()),
        allowed_hosts: Mutex::new(Vec::new()),
        spawn_fails: Mutex::new(std::collections::HashMap::new()),
        runtime: Mutex::new(crate::runtime::RuntimeLog::new()),
        spawning: Mutex::new(std::collections::HashSet::new()),
        update_latest: Mutex::new(None),
        lan_ok: Mutex::new(None),
    })
}

/// Fabrique un serveur HTTP pour les tests de fusion/dé-duplication.
fn srv_http(
    id: &str,
    name: Option<&str>,
    label: &str,
    host: Option<&str>,
    port: u16,
    origin: Origin,
    seen: Option<u64>,
) -> Server {
    Server {
        id: id.into(),
        label: label.into(),
        transport: Transport::Http {
            host: host.map(String::from),
            port,
            path: "/mcp".into(),
        },
        origin,
        enabled: true,
        state: SState::Up,
        pid: None,
        bound_host: None,
        server_name: name.map(String::from),
        version: None,
        protocol: None,
        sse: false,
        sse_endpoint: false,
        ws: false,
        lang: None,
        framework: None,
        via_ssh: false,
        tools: vec![],
        latency_ms: None,
        last_seen_ts: seen,
        last_error: None,
        start_cmd: None,
        auto_restart: false,
        favorite: false,
        tools_disabled: vec![],
    }
}

/// RÉGRESSION audit 2026-10-26 (P1) : la fusion ne doit toucher QUE les
/// entrées découvertes (origin Discovered) avec un nom annoncé non vide.
/// Deux serveurs manuels homonymes (ou sans label) sur des ports
/// différents doivent TOUJOURS survivre — l'ancienne version fusionnait
/// sur nom+label@hôte sans port et persistait la suppression.
#[test]
fn merge_ne_touche_jamais_les_entrees_manuelles_ou_sans_nom() {
    let state = test_state();
    {
        let mut servers = lockv(&state.servers);
        // deux manuels homonymes, ports différents
        servers.push(srv_http(
            "manual-a",
            Some("discord"),
            "discord",
            None,
            3000,
            Origin::Manual,
            Some(100),
        ));
        servers.push(srv_http(
            "manual-b",
            Some("discord"),
            "discord",
            None,
            3001,
            Origin::Manual,
            Some(200),
        ));
        // deux manuels SANS nom annoncé (le bug historique : label vide →
        // clé « @local » → suppression d'une entrée légitime)
        servers.push(srv_http(
            "manual-c",
            None,
            "",
            None,
            3002,
            Origin::Manual,
            Some(300),
        ));
        servers.push(srv_http(
            "manual-d",
            None,
            "",
            None,
            3003,
            Origin::Manual,
            Some(400),
        ));
        // un seed homonyme d'un manuel
        servers.push(srv_http(
            "seed-x",
            Some("discord"),
            "discord",
            None,
            3004,
            Origin::Seed,
            Some(500),
        ));
    }
    let removed = merge_duplicate_servers(&state);
    assert_eq!(
        removed, 0,
        "aucune entrée manuelle/seed ne doit être fusionnée"
    );
    assert_eq!(lockv(&state.servers).len(), 5);
}

#[test]
fn merge_fusionne_les_doublons_resolus_meme_nom_meme_machine() {
    let state = test_state();
    {
        let mut servers = lockv(&state.servers);
        // même serveur découvert deux fois (port changé) : la plus
        // récemment vue gagne, l'ancienne port est un fantôme
        servers.push(srv_http(
            "net-10.0.0.5.3141",
            Some("discord"),
            "10.0.0.5:3141",
            Some("10.0.0.5"),
            3141,
            Origin::Discovered,
            Some(100),
        ));
        servers.push(srv_http(
            "net-10.0.0.5.9999",
            Some("discord"),
            "10.0.0.5:9999",
            Some("10.0.0.5"),
            9999,
            Origin::Discovered,
            Some(200),
        ));
        // même nom mais MACHINE DIFFÉRENTE : deux serveurs distincts
        servers.push(srv_http(
            "net-10.0.0.6.3141",
            Some("discord"),
            "10.0.0.6:3141",
            Some("10.0.0.6"),
            3141,
            Origin::Discovered,
            Some(300),
        ));
        // découvert SANS nom annoncé : jamais fusionné (identité inconnue)
        servers.push(srv_http(
            "net-10.0.0.5.4000",
            None,
            "10.0.0.5:4000",
            Some("10.0.0.5"),
            4000,
            Origin::Discovered,
            Some(400),
        ));
    }
    let removed = merge_duplicate_servers(&state);
    assert_eq!(removed, 1, "seul le doublon même nom+même machine part");
    let servers = lockv(&state.servers);
    assert!(
        servers.iter().any(|s| s.id == "net-10.0.0.5.9999"),
        "la plus récente est gardée"
    );
    assert!(!servers.iter().any(|s| s.id == "net-10.0.0.5.3141"));
    assert!(
        servers.iter().any(|s| s.id == "net-10.0.0.6.3141"),
        "l'autre machine survit"
    );
    assert!(
        servers.iter().any(|s| s.id == "net-10.0.0.5.4000"),
        "sans nom annoncé : jamais fusionné"
    );
}

#[test]
fn lan_decision_couvre_les_cas_vpn_et_panne() {
    // passerelle qui répond, serveurs distants OK → réseau sain
    assert_eq!(lan_decision(Some(true), false), Some(true));
    // passerelle muette → coupé
    assert_eq!(lan_decision(Some(false), false), Some(false));
    // VPN : SA passerelle répond mais tous les distants échouent en réseau
    assert_eq!(lan_decision(Some(true), true), Some(false));
    // pas de passerelle détectée mais distants KO → coupé
    assert_eq!(lan_decision(None, true), Some(false));
    // pas de passerelle et distants OK → indéterminé (pas de LAN à surveiller)
    assert_eq!(lan_decision(None, false), None);
}

#[test]
fn auto_restart_par_serveur_action_et_persistance() {
    // Bouton ⚡ de l'UI : action « set_auto_restart » → flag du serveur,
    // persistance dans config.json, et remise à zéro des échecs de spawn.
    let path = tmp_config("cfg_autorestart.json", Some("{\"servers\":[]}"));
    // état partagé avec un vrai chemin de config pour tester l'écriture
    let mut base = match Arc::try_unwrap(test_state()) {
        Ok(s) => s,
        Err(_) => panic!("test_state doit être unique"),
    };
    base.config_path = path.clone();
    let state = Arc::new(base);
    {
        let mut st = lockv(&state.settings);
        st.autostart = false;
    }
    {
        let mut servers = lockv(&state.servers);
        servers.push(Server {
            id: "srv-a".into(),
            label: "Serveur A".into(),
            transport: Transport::Stdio {
                program: "node".into(),
                args: vec![],
                workdir: None,
                env: Vec::new(),
            },
            origin: Origin::Manual,
            enabled: true,
            state: SState::Down,
            pid: None,
            bound_host: None,
            server_name: None,
            version: None,
            protocol: None,
            sse: false,
            sse_endpoint: false,
            ws: false,
            lang: None,
            framework: None,
            via_ssh: false,
            tools: vec![],
            latency_ms: None,
            last_seen_ts: None,
            last_error: None,
            start_cmd: Some(StartCmd {
                program: "node".into(),
                args: vec![],
                workdir: None,
                env: Vec::new(),
            }),
            auto_restart: true,
            favorite: false,
            tools_disabled: vec![],
        });
    }
    lockv(&state.spawn_fails).insert("srv-a".to_string(), 3);

    // désactivation
    let j = handle_action(
        &state,
        "{\"action\":\"set_auto_restart\",\"id\":\"srv-a\",\"enabled\":false}",
    );
    assert_eq!(j["ok"], serde_json::json!(true));
    assert!(!lockv(&state.servers)[0].auto_restart);
    let cfg = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        cfg.contains("\"auto_restart\": false"),
        "le flag doit être persisté dans config.json : {}",
        cfg
    );

    // réactivation : les échecs de spawn repartent de zéro
    lockv(&state.spawn_fails).insert("srv-a".to_string(), 3);
    let j2 = handle_action(
        &state,
        "{\"action\":\"set_auto_restart\",\"id\":\"srv-a\",\"enabled\":true}",
    );
    assert_eq!(j2["ok"], serde_json::json!(true));
    assert!(lockv(&state.servers)[0].auto_restart);
    assert!(
        lockv(&state.spawn_fails).get("srv-a").is_none(),
        "réactiver l'autoboot doit effacer le compteur d'échecs"
    );

    // serveur inconnu → erreur propre, pas de panic
    let j3 = handle_action(
        &state,
        "{\"action\":\"set_auto_restart\",\"id\":\"inconnu\",\"enabled\":true}",
    );
    assert_eq!(j3["ok"], serde_json::json!(false));
}

#[test]
fn set_host_refuse_seed_et_rehost_manuel() {
    // Explorateur groupé par machine : action « set_host » (drop d'une ligne
    // sur l'en-tête d'un groupe). Un seed est refusé — la config par défaut
    // réimposerait son host au boot —, un serveur manuel est re-hosté et
    // persisté, un stdio est refusé (pas de host), erreurs propres sinon.
    let path = tmp_config("cfg_sethost.json", Some("{\"servers\":[]}"));
    let mut base = match Arc::try_unwrap(test_state()) {
        Ok(s) => s,
        Err(_) => panic!("test_state doit être unique"),
    };
    base.config_path = path.clone();
    let state = Arc::new(base);
    {
        let mut st = lockv(&state.settings);
        st.autostart = false;
    }
    {
        let mut servers = lockv(&state.servers);
        // seed HTTP local (comme un seed du launcher)
        servers.push(Server {
            id: "seed-1".into(),
            label: "Seed".into(),
            transport: Transport::Http {
                host: None,
                port: 3099,
                path: "/mcp".into(),
            },
            origin: Origin::Seed,
            enabled: true,
            state: SState::Down,
            pid: None,
            bound_host: None,
            server_name: None,
            version: None,
            protocol: None,
            sse: false,
            sse_endpoint: false,
            ws: false,
            lang: None,
            framework: None,
            via_ssh: false,
            tools: vec![],
            latency_ms: None,
            last_seen_ts: None,
            last_error: None,
            start_cmd: None,
            auto_restart: true,
            favorite: false,
            tools_disabled: vec![],
        });
        // serveur manuel HTTP importé d'une machine (host à déplacer)
        servers.push(Server {
            id: "man-1".into(),
            label: "Manuel".into(),
            transport: Transport::Http {
                host: Some("192.168.40.9".into()),
                port: 3141,
                path: "/mcp".into(),
            },
            origin: Origin::Manual,
            enabled: true,
            state: SState::Down,
            pid: None,
            bound_host: None,
            server_name: None,
            version: None,
            protocol: None,
            sse: false,
            sse_endpoint: false,
            ws: false,
            lang: None,
            framework: None,
            via_ssh: true,
            tools: vec![],
            latency_ms: None,
            last_seen_ts: None,
            last_error: None,
            start_cmd: None,
            auto_restart: true,
            favorite: false,
            tools_disabled: vec![],
        });
        // stdio : jamais concerné par un re-host
        servers.push(Server {
            id: "man-2".into(),
            label: "Stdio".into(),
            transport: Transport::Stdio {
                program: "node".into(),
                args: vec![],
                workdir: None,
                env: Vec::new(),
            },
            origin: Origin::Manual,
            enabled: true,
            state: SState::Down,
            pid: None,
            bound_host: None,
            server_name: None,
            version: None,
            protocol: None,
            sse: false,
            sse_endpoint: false,
            ws: false,
            lang: Some("node".into()),
            framework: None,
            via_ssh: false,
            tools: vec![],
            latency_ms: None,
            last_seen_ts: None,
            last_error: None,
            start_cmd: None,
            auto_restart: true,
            favorite: false,
            tools_disabled: vec![],
        });
    }

    // 1. re-host d'un seed → refus, host inchangé (reste local/None)
    let j = handle_action(
        &state,
        "{\"action\":\"set_host\",\"id\":\"seed-1\",\"host\":\"10.9.9.9\"}",
    );
    assert_eq!(j["ok"], serde_json::json!(false));
    {
        let servers = lockv(&state.servers);
        let Transport::Http { host, .. } = &servers[0].transport else {
            panic!("transport Http attendu");
        };
        assert_eq!(host.as_deref(), None, "le seed garde host:None (local)");
    }

    // 2. re-host d'un manuel → ok, ancien host rapporté, persisté
    let j2 = handle_action(
        &state,
        "{\"action\":\"set_host\",\"id\":\"man-1\",\"host\":\"10.9.9.9\"}",
    );
    assert_eq!(j2["ok"], serde_json::json!(true));
    assert_eq!(j2["old_host"], serde_json::json!("192.168.40.9"));
    {
        let servers = lockv(&state.servers);
        let Transport::Http { host, .. } = &servers[1].transport else {
            panic!("transport Http attendu");
        };
        assert_eq!(host.as_deref(), Some("10.9.9.9"));
    }
    let cfg = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        cfg.contains("10.9.9.9"),
        "le nouveau host doit être persisté dans config.json : {}",
        cfg
    );

    // 3. stdio → refus (pas de host possible)
    let j3 = handle_action(
        &state,
        "{\"action\":\"set_host\",\"id\":\"man-2\",\"host\":\"10.9.9.9\"}",
    );
    assert_eq!(j3["ok"], serde_json::json!(false));

    // 4. serveur inconnu → erreur propre
    let j4 = handle_action(
        &state,
        "{\"action\":\"set_host\",\"id\":\"inconnu\",\"host\":\"10.9.9.9\"}",
    );
    assert_eq!(j4["ok"], serde_json::json!(false));

    // 5. host vide/blanc → erreur propre
    let j5 = handle_action(
        &state,
        "{\"action\":\"set_host\",\"id\":\"man-1\",\"host\":\"  \"}",
    );
    assert_eq!(j5["ok"], serde_json::json!(false));
}

#[test]
fn set_start_cmd_refuse_seed_et_distant_persiste_le_local() {
    // Le bouton ▶/■ apparaît si start_cmd existe : action « set_start_cmd »
    // pour poser la commande d'un local découvert par scan. Seeds refusés
    // (la config par défaut réécraserait), distants refusés (le process
    // appartient à sa machine hôte), locaux OK + persisté.
    let path = tmp_config("cfg_setstartcmd.json", Some("{\"servers\":[]}"));
    let mut base = match Arc::try_unwrap(test_state()) {
        Ok(s) => s,
        Err(_) => panic!("test_state doit être unique"),
    };
    base.config_path = path.clone();
    let state = Arc::new(base);
    {
        let mut st = lockv(&state.settings);
        st.autostart = false;
    }
    {
        let mut servers = lockv(&state.servers);
        // local sans commande (découvert par scan de ports)
        servers.push(Server {
            id: "scan-1".into(),
            label: "port-4242".into(),
            transport: Transport::Http {
                host: None,
                port: 4242,
                path: "/mcp".into(),
            },
            origin: Origin::Discovered,
            enabled: true,
            state: SState::Down,
            pid: None,
            bound_host: None,
            server_name: None,
            version: None,
            protocol: None,
            sse: false,
            sse_endpoint: false,
            ws: false,
            lang: None,
            framework: None,
            via_ssh: false,
            tools: vec![],
            latency_ms: None,
            last_seen_ts: None,
            last_error: None,
            start_cmd: None,
            auto_restart: true,
            favorite: false,
            tools_disabled: vec![],
        });
        // distant importé (jamais startable ici)
        servers.push(Server {
            id: "dist-1".into(),
            label: "distant".into(),
            transport: Transport::Http {
                host: Some("10.1.2.3".into()),
                port: 8080,
                path: "/mcp".into(),
            },
            origin: Origin::Manual,
            enabled: true,
            state: SState::Down,
            pid: None,
            bound_host: None,
            server_name: None,
            version: None,
            protocol: None,
            sse: false,
            sse_endpoint: false,
            ws: false,
            lang: None,
            framework: None,
            via_ssh: true,
            tools: vec![],
            latency_ms: None,
            last_seen_ts: None,
            last_error: None,
            start_cmd: None,
            auto_restart: true,
            favorite: false,
            tools_disabled: vec![],
        });
        // seed local (commande figée par la config par défaut)
        servers.push(Server {
            id: "seed-9".into(),
            label: "seed".into(),
            transport: Transport::Http {
                host: None,
                port: 3099,
                path: "/mcp".into(),
            },
            origin: Origin::Seed,
            enabled: true,
            state: SState::Down,
            pid: None,
            bound_host: None,
            server_name: None,
            version: None,
            protocol: None,
            sse: false,
            sse_endpoint: false,
            ws: false,
            lang: None,
            framework: None,
            via_ssh: false,
            tools: vec![],
            latency_ms: None,
            last_seen_ts: None,
            last_error: None,
            start_cmd: None,
            auto_restart: true,
            favorite: false,
            tools_disabled: vec![],
        });
    }

    // 1. distant → refus
    let j = handle_action(
            &state,
            "{\"action\":\"set_start_cmd\",\"id\":\"dist-1\",\"program\":\"node\",\"args\":[\"srv.js\"]}",
        );
    assert_eq!(j["ok"], serde_json::json!(false));

    // 2. seed → refus
    let j2 = handle_action(
        &state,
        "{\"action\":\"set_start_cmd\",\"id\":\"seed-9\",\"program\":\"node\"}",
    );
    assert_eq!(j2["ok"], serde_json::json!(false));

    // 3. programme vide (espaces seuls) = RETRAIT (lot A : null/"" retire la
    //    commande ; scan-1 n'en a pas — no-op visible, autoboot désactivé)
    let j3 = handle_action(
        &state,
        "{\"action\":\"set_start_cmd\",\"id\":\"scan-1\",\"program\":\"  \"}",
    );
    assert_eq!(j3["ok"], serde_json::json!(true));
    assert_eq!(j3["removed"], serde_json::json!(true));
    {
        let servers = lockv(&state.servers);
        let scan1 = servers.iter().find(|s| s.id == "scan-1").unwrap();
        assert!(
            scan1.start_cmd.is_none(),
            "aucune commande n'existait : reste None"
        );
        assert!(
            !scan1.auto_restart,
            "autoboot désactivé par le retrait (plus rien à lancer)"
        );
    }

    // 4. local découvert → OK : start_cmd posé (args + workdir) et persisté
    let j4 = handle_action(
            &state,
            "{\"action\":\"set_start_cmd\",\"id\":\"scan-1\",\"program\":\"node\",\"args\":[\"server.js\",\"--port\",\"4242\"],\"workdir\":\"C:/tmp\"}",
        );
    assert_eq!(j4["ok"], serde_json::json!(true));
    {
        let servers = lockv(&state.servers);
        let cmd = servers[0]
            .start_cmd
            .as_ref()
            .expect("start_cmd doit exister");
        assert_eq!(cmd.program, "node");
        assert_eq!(
            servers[0].origin,
            Origin::Manual,
            "le découvert doit être promu manual pour survivre au reboot"
        );
        assert_eq!(
            cmd.args,
            vec![
                "server.js".to_string(),
                "--port".to_string(),
                "4242".to_string()
            ]
        );
        assert_eq!(cmd.workdir.as_deref(), Some("C:/tmp"));
    }
    let cfg = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        cfg.contains("server.js"),
        "la commande doit être persistée dans config.json : {}",
        cfg
    );
}

/// Verrou sérialisant les tests qui modifient `MCC_LITEMP_REGISTRY` en
/// parallèle dans le même process de test.
fn litemp_env_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// Helper : dossier d'annonces isolé pour un test (jamais %TEMP% réel).
fn dossier_annonces(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mcc-annonces-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("créer le dossier d'annonces");
    dir
}

#[test]
fn annonces_litemcp_listees_sans_adoption_ni_ecriture() {
    let _env_garde = litemp_env_guard();
    // Audit S4, point 2 : une annonce trouvée est LISTÉE (« détecté, non
    // suivi »), jamais adoptée : aucun serveur créé, aucun config.json écrit,
    // aucun champ exécuté (exe/args ne sortent même pas de la lecture).
    let dir = dossier_annonces("lecture");
    std::fs::write(
        dir.join("moltbook.json"),
        r#"{"name":"moltbook","port":8787,"version":"0.3.0","exe":"C:/ws/moltbook.exe","args":["C:/ws/moltbook.exe"],"cwd":"C:/ws"}"#,
    )
    .unwrap();
    // au-delà de 64 Kio : écarté, jamais lu
    std::fs::write(dir.join("enorme.json"), "x".repeat(64 * 1024 + 1)).unwrap();
    std::fs::write(dir.join("casse.json"), "pas du json").unwrap();
    // un dossier nommé *.json n'est pas une annonce
    std::fs::create_dir_all(dir.join("dossier.json")).unwrap();
    let path = tmp_config("cfg_annonces.json", None);
    let mut base = match Arc::try_unwrap(test_state()) {
        Ok(s) => s,
        Err(_) => panic!("test_state doit être unique"),
    };
    base.config_path = path.clone();
    let state = Arc::new(base);
    std::env::set_var("MCC_LITEMP_REGISTRY", &dir);

    let v = litemcp_announcements(&state);
    std::env::remove_var("MCC_LITEMP_REGISTRY");

    let arr = v.get("announcements").and_then(|a| a.as_array()).unwrap();
    let molt = arr
        .iter()
        .find(|a| a.get("name") == Some(&serde_json::json!("moltbook")))
        .expect("moltbook listé");
    assert_eq!(molt.get("port"), Some(&serde_json::json!(8787)));
    assert_eq!(molt.get("followed"), Some(&serde_json::json!(false)));
    assert_eq!(molt.get("detected"), Some(&serde_json::json!(true)));
    assert!(
        molt.get("exe").is_none() && molt.get("args").is_none(),
        "aucun champ exécutable ne doit sortir de la lecture : {molt}"
    );
    let skipped = v.get("skipped").and_then(|a| a.as_array()).unwrap();
    assert!(
        skipped
            .iter()
            .any(|s| s.to_string().contains("trop volumineux")),
        "le fichier > 64 Kio doit être écarté : {skipped:?}"
    );
    assert!(
        skipped
            .iter()
            .any(|s| s.to_string().contains("JSON invalide")),
        "le JSON invalide doit être écarté : {skipped:?}"
    );
    assert!(
        skipped
            .iter()
            .any(|s| s.to_string().contains("fichier ordinaire")),
        "un dossier .json doit être écarté : {skipped:?}"
    );
    // la garde de propriété est bien appliquée (positif + négatif)
    assert_eq!(
        probe::file_owned_by_current_user(&dir.join("moltbook.json")),
        Some(true),
        "l'annonce de test appartient au compte courant"
    );
    assert!(owner_check_result(Some(true)).is_ok());
    assert!(owner_check_result(None).is_ok());
    let refus =
        owner_check_result(Some(false)).expect_err("un autre propriétaire doit être refusé");
    assert!(refus.contains("autre compte"), "{refus}");
    #[cfg(windows)]
    {
        let sys_file = std::path::Path::new(r"C:\Windows\System32\notepad.exe");
        if sys_file.exists() {
            assert_ne!(
                probe::file_owned_by_current_user(sys_file),
                Some(true),
                "un binaire système (TrustedInstaller) ne doit pas passer le contrôle de propriété"
            );
        }
    }
    // aucune adoption : liste vide, aucun config.json créé
    assert!(lockv(&state.servers).is_empty(), "aucun serveur adopté");
    assert!(
        !std::path::Path::new(&path).exists(),
        "la lecture d'annonces ne doit RIEN écrire sur le disque"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // Nombre borné : 40 fichiers, 32 examinés au plus.
    let dir = dossier_annonces("borne");
    for i in 0..40 {
        std::fs::write(
            dir.join(format!("filler-{i}.json")),
            format!(r#"{{"name":"filler-{i}","port":{} }}"#, 20000 + i),
        )
        .unwrap();
    }
    std::env::set_var("MCC_LITEMP_REGISTRY", &dir);
    let v = litemcp_announcements(&state);
    std::env::remove_var("MCC_LITEMP_REGISTRY");
    let n = v
        .get("announcements")
        .and_then(|a| a.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    assert!(n <= 32, "32 fichiers examinés au plus, obtenu {n}");
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn reglages_de_decouverte_parses_et_appliques() {
    // Point 3 : validation des ports de découverte (1-65535, doublons, plafond)
    // et filtre « seulement / exclus ».
    let cfg = r#"{"settings":{"discovery_ports_only":[8787,8787,0,70000,"x",3000],"discovery_ports_excluded":[3131]}}"#;
    let (_, st) = load_config(&tmp_config("cfg_decouverte.json", Some(cfg)));
    assert_eq!(st.discovery_ports_only, vec![8787, 3000]);
    assert_eq!(st.discovery_ports_excluded, vec![3131]);
    assert!(discovery_port_allowed(8787, &st.discovery_ports_only, &[]));
    assert!(!discovery_port_allowed(8788, &st.discovery_ports_only, &[]));
    assert!(!discovery_port_allowed(
        3131,
        &[],
        &st.discovery_ports_excluded
    ));
    assert!(discovery_port_allowed(
        3132,
        &[],
        &st.discovery_ports_excluded
    ));
    assert!(
        !discovery_port_allowed(8787, &[8787], &[8787]),
        "excluded doit toujours primer sur only"
    );
    // plafond : 512 entrées max, aucun doublon
    let arr: Vec<serde_json::Value> = (1..=700).map(|p| serde_json::json!(p)).collect();
    let ports = parse_port_list(&arr, 512);
    assert_eq!(ports.len(), 512);
    assert_eq!(ports.first(), Some(&1));
    assert_eq!(ports.last(), Some(&512));
}

#[test]
fn temporaires_orphelins_purges_au_chargement() {
    // Point 4 : un kill pendant l'écriture laissait un `.tmp` complet (secrets
    // compris). Le boot les supprime avant toute écriture.
    let dir = std::env::temp_dir().join(format!("mcc-tmp-purge-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("config.json");
    std::fs::write(&cfg, "{\"servers\":[]}").unwrap();
    std::fs::write(dir.join("config.json.123.0.tmp"), "secrets").unwrap();
    std::fs::write(dir.join("state-cache.json.123.1.tmp"), "cache").unwrap();
    std::fs::write(dir.join("mcc.log"), "journal").unwrap();
    let (_, _) = load_config(&cfg.to_string_lossy());
    assert!(
        !dir.join("config.json.123.0.tmp").exists(),
        "tmp config purgé"
    );
    assert!(
        !dir.join("state-cache.json.123.1.tmp").exists(),
        "tmp cache purgé"
    );
    assert!(cfg.exists(), "la config elle-même reste intacte");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[allow(clippy::permissions_set_readonly_false)] // test : remise en écriture après l'assertion
fn echec_d_ecriture_remonte_et_ne_laisse_pas_de_tmp() {
    // Point 4 : plus de `ok:true` mensonger. Un config.json en lecture seule
    // fait échouer l'écriture : l'action rend ok:false avec la cause, aucun
    // `.tmp` ne subsiste.
    let dir = std::env::temp_dir().join(format!("mcc-ro-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("config.json");
    std::fs::write(&cfg, "{\"servers\":[]}").unwrap();
    #[cfg(windows)]
    {
        let mut perms = std::fs::metadata(&cfg).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&cfg, perms).unwrap();
    }
    #[cfg(unix)]
    {
        // Sur Unix, rename() vérifie les droits du DOSSIER parent (pas du fichier cible).
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    }

    let mut base = match Arc::try_unwrap(test_state()) {
        Ok(s) => s,
        Err(_) => panic!("test_state doit être unique"),
    };
    base.config_path = cfg.to_string_lossy().to_string();
    let state = Arc::new(base);
    let rep = handle_action(&state, "{\"action\":\"set_settings\",\"refresh_secs\":42}");
    let ok = rep.get("ok").and_then(|v| v.as_bool()).unwrap_or(true);
    let err = rep.get("error").and_then(|v| v.as_str()).unwrap_or("");
    // remise en écriture AVANT les assertions (nettoyage garanti)
    #[cfg(windows)]
    {
        let mut perms = std::fs::metadata(&cfg).unwrap().permissions();
        perms.set_readonly(false);
        std::fs::set_permissions(&cfg, perms).unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert!(
        !ok,
        "une écriture ratée ne doit pas répondre ok:true : {rep}"
    );
    assert!(
        err.contains("config non écrite"),
        "le motif doit être explicite : {rep}"
    );
    let tmp: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(tmp.is_empty(), "aucun .tmp ne doit rester : {tmp:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ecriture_atomique_disque_plein_laisse_config_intacte_et_aucun_tmp() {
    // Point 4 (disque plein simulé) : si l'écriture dans le `.tmp` échoue à
    // mi-chemin (ENOSPC), le temporaire partiel est supprimé et le fichier
    // cible reste intact et parsable.
    let dir = std::env::temp_dir().join(format!("mcc-enospc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("config.json");
    let intact = r#"{"servers":[{"id":"s1","label":"Intact","transport":"http","port":4123,"path":"/mcp"}]}"#;
    std::fs::write(&cfg, intact).unwrap();

    let res = write_atomic_with(&cfg.to_string_lossy(), "config", |tmp| {
        std::fs::write(tmp, b"{\"servers\":[tronque")?;
        Err(std::io::Error::other(
            "No space left on device (ENOSPC simulé)",
        ))
    });
    assert!(res.is_err(), "un échec ENOSPC doit remonter en Err");
    let contenu = std::fs::read_to_string(&cfg).unwrap();
    assert_eq!(
        contenu, intact,
        "config.json doit rester intact après ENOSPC"
    );
    let tmp: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(
        tmp.is_empty(),
        "aucun .tmp partiel ne doit rester : {tmp:?}"
    );

    // save_config_checked vers un dossier inexistant → Err, aucun .tmp créé
    let mut base = match Arc::try_unwrap(test_state()) {
        Ok(s) => s,
        Err(_) => panic!("test_state doit être unique"),
    };
    base.config_path = dir
        .join("absent")
        .join("config.json")
        .to_string_lossy()
        .to_string();
    let state = Arc::new(base);
    assert!(save_config_checked(&state).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// ACL Windows : l'utilisateur, SYSTEM et le groupe Administrateurs. Vérifié
/// par les SID (indépendants de la langue) via Get-Acl.
#[cfg(windows)]
#[test]
fn acl_config_cache_et_journal_utilisateur_system_administrateurs() {
    let dir = std::env::temp_dir().join(format!("mcc-acl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("config.json");
    std::fs::write(&cfg, "{\"servers\":[]}").unwrap();
    std::fs::write(dir.join("state-cache.json"), "{}").unwrap();
    std::fs::write(dir.join("mcc.log"), "").unwrap();
    secure_data_files(&cfg.to_string_lossy());
    let moi = probe::current_user_sid().expect("SID courant");
    for nom in ["config.json", "state-cache.json", "mcc.log"] {
        let p = dir.join(nom);
        let sddl_file = dir.join(format!("{nom}.sddl"));
        let _ = std::fs::remove_file(&sddl_file);
        // `icacls <fichier> /save <sddl>` écrit la chaîne SDDL (UTF-16LE,
        // indépendante de la langue et sans dépendre de PowerShell/PSModulePath).
        let out = probe::system_command("icacls")
            .arg(&p)
            .arg("/save")
            .arg(&sddl_file)
            .output()
            .expect("icacls /save");
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        assert!(
            out.status.success() && sddl_file.exists(),
            "{nom} : icacls /save échoué ({}) — {stderr}",
            out.status
        );
        let raw = std::fs::read(&sddl_file).expect("lire fichier sddl");
        let u16s: Vec<u16> = (0..raw.len() / 2)
            .map(|i| u16::from_le_bytes([raw[2 * i], raw[2 * i + 1]]))
            .collect();
        let sddl = String::from_utf16_lossy(&u16s);
        // SDDL est indépendant de la langue :
        // SY = S-1-5-18 (SYSTEM), BA = S-1-5-32-544 (Administrateurs),
        // LA = administrateur local intégré (RID -500, utilisé sur les runners Windows CI).
        assert!(
            sddl.contains(&moi) || (moi.ends_with("-500") && sddl.contains(";LA)")),
            "{nom} : compte courant ({moi}) absent — {sddl}"
        );
        assert!(
            sddl.contains(";SY)") || sddl.contains("S-1-5-18"),
            "{nom} : SYSTEM absent — {sddl}"
        );
        assert!(
            sddl.contains(";BA)") || sddl.contains("S-1-5-32-544"),
            "{nom} : Administrateurs absent — {sddl}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn set_settings_valide_borne_et_persiste() {
    // Dialog ⚙ : réglages principaux. Validation stricte : refresh clamp
    // 5-3600, scan borné 1-65535 avec from<=to, bind sur whitelist
    // (sinon loopback forcé), ports réseau 1-65535 uniquement, et
    // persistance dans config.json.
    let path = tmp_config("cfg_settings_dialog.json", Some("{\"servers\":[]}"));
    let mut base = match Arc::try_unwrap(test_state()) {
        Ok(s) => s,
        Err(_) => panic!("test_state doit être unique"),
    };
    base.config_path = path.clone();
    let state = Arc::new(base);
    {
        let mut st = lockv(&state.settings);
        st.autostart = false;
    }

    // 1. valeurs hors bornes → clamps + corrections
    let j = handle_action(
            &state,
            "{\"action\":\"set_settings\",\"refresh_secs\":99999,\"scan_from\":9000,\"scan_to\":100,\"ui_bind\":\"0.0.0.0\",\"network_enabled\":false,\"network_ports\":[5090,0,70000,8081,65536]}",
        );
    assert_eq!(j["ok"], serde_json::json!(true));
    assert_eq!(
        j["changed"],
        serde_json::json!(["refresh", "scan", "ui_bind", "network", "network_ports"])
    );
    {
        let st = lockv(&state.settings);
        assert_eq!(st.refresh_secs, 3600, "clamp haut");
        assert_eq!((st.scan_from, st.scan_to), (100, 9000), "from<=to imposé");
        assert_eq!(st.ui_bind, "0.0.0.0");
        assert!(!st.network_enabled);
        assert_eq!(st.network_ports, vec![5090, 8081], "0/70000/65536 filtrés");
    }
    let cfg = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(cfg.contains("\"refresh_secs\": 3600"), "persisté : {}", cfg);
    assert!(cfg.contains("\"network_enabled\": false"));

    // 2. bind hors whitelist → loopback forcé (jamais un bind arbitraire)
    let j2 = handle_action(
        &state,
        "{\"action\":\"set_settings\",\"ui_bind\":\"192.168.1.5\"}",
    );
    assert_eq!(j2["ok"], serde_json::json!(true));
    assert_eq!(lockv(&state.settings).ui_bind, "127.0.0.1");

    // 3. refresh trop bas → clamp bas (5 s minimum, pas de poll dément)
    let j3 = handle_action(&state, "{\"action\":\"set_settings\",\"refresh_secs\":1}");
    assert_eq!(j3["ok"], serde_json::json!(true));
    assert_eq!(lockv(&state.settings).refresh_secs, 5);

    // 4. aucun changement → changed vide, pas d'écriture inutile
    let j4 = handle_action(&state, "{\"action\":\"set_settings\",\"refresh_secs\":5}");
    assert_eq!(j4["changed"], serde_json::json!([] as [&str; 0]));
}

#[test]
fn set_notify_valide_et_persiste() {
    // Panneau Réglages → Notifications : toast desktop + webhook. URL
    // invalide refusée AVANT tout, méthode normalisée, min_interval
    // clampé, config persistée, et état exposé à l'UI (/api/state).
    let path = tmp_config("cfg_set_notify.json", Some("{\"servers\":[]}"));
    let mut base = match Arc::try_unwrap(test_state()) {
        Ok(s) => s,
        Err(_) => panic!("test_state doit être unique"),
    };
    base.config_path = path.clone();
    let state = Arc::new(base);

    // 1. URL invalide → refus net, rien de persisté
    let j = handle_action(
        &state,
        "{\"action\":\"set_notify\",\"webhook_url\":\"ftp://non\"}",
    );
    assert_eq!(j["ok"], serde_json::json!(false));
    assert!(j["error"].as_str().unwrap_or("").contains("invalide"));
    assert!(lockv(&state.settings).notify.webhook_url.is_empty());

    // 2. config complète → ok + état rendu + persisté dans config.json
    let j2 = handle_action(
            &state,
            "{\"action\":\"set_notify\",\"desktop\":true,\"webhook_url\":\"http://example.com/hook\",\"method\":\"put\",\"headers\":\"Authorization: Bearer x\",\"states\":[\"DOWN\"],\"min_interval_secs\":60}",
        );
    assert_eq!(j2["ok"], serde_json::json!(true));
    assert_eq!(j2["notify"]["desktop"], serde_json::json!(true));
    assert_eq!(
        j2["notify"]["webhook_url"],
        serde_json::json!("http://example.com/hook")
    );
    assert_eq!(
        j2["notify"]["method"],
        serde_json::json!("PUT"),
        "méthode normalisée"
    );
    assert_eq!(j2["notify"]["states"], serde_json::json!(["DOWN"]));
    {
        let st = lockv(&state.settings);
        assert!(st.notify.desktop);
        assert_eq!(st.notify.webhook_url, "http://example.com/hook");
        assert_eq!(st.notify.min_interval_secs, 60);
    }
    let cfg = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(cfg.contains("example.com/hook"), "persisté : {}", cfg);

    // 3. min_interval hors bornes → clamp 0..86400
    let j3 = handle_action(
        &state,
        "{\"action\":\"set_notify\",\"min_interval_secs\":999999}",
    );
    assert_eq!(j3["ok"], serde_json::json!(true));
    assert_eq!(lockv(&state.settings).notify.min_interval_secs, 86400);

    // 4. /api/state expose la config notifications (panneau UI) ; le
    //    journal raconte l'enregistrement
    let full = state_json(&state);
    assert_eq!(
        full["settings"]["notify"]["webhook_url"],
        serde_json::json!("http://example.com/hook")
    );
    let log = lockv(&state.log);
    assert!(
        log.iter()
            .any(|e| e.msg.contains("[notif] configuration enregistrée")),
        "le journal doit raconter l'enregistrement"
    );
}

#[test]
fn test_notify_refuse_sans_canal_et_rapporte_les_echecs() {
    // Bouton « Tester » : sans canal actif → refus explicite (l'UI affiche
    // pourquoi) ; webhook invalide → échec rapporté PAR CANAL, sans panic
    // et sans requête réseau (la validation URL précède tout envoi).
    let state = test_state();
    let j = handle_action(&state, "{\"action\":\"test_notify\"}");
    assert_eq!(j["ok"], serde_json::json!(false));
    assert!(j["error"]
        .as_str()
        .unwrap_or("")
        .contains("aucun canal actif"));
    {
        let mut st = lockv(&state.settings);
        st.notify.webhook_url = "notaurl".into();
    }
    let j2 = handle_action(&state, "{\"action\":\"test_notify\"}");
    assert_eq!(j2["ok"], serde_json::json!(true));
    let wh = j2["webhook"].as_object().expect("canal webhook rapporté");
    assert_eq!(wh["ok"], serde_json::json!(false));
    assert!(wh["error"].as_str().unwrap_or("").contains("invalide"));
    // canal desktop jamais activé → absent de la réponse (pas d'échec inventé)
    assert!(j2["desktop"].is_null());
}

#[test]
fn download_update_refuse_les_plateformes_hors_table() {
    // os fourni mais invalide → refus AVANT tout lancement de navigateur
    // (aucun repli silencieux sur la plateforme du moteur, aucun effet de
    // bord visible) ; l'UI affiche l'erreur telle quelle.
    let state = test_state();
    let j = handle_action(&state, "{\"action\":\"download_update\",\"os\":\"beos\"}");
    assert_eq!(j["ok"], serde_json::json!(false));
    assert!(j["error"].as_str().unwrap_or("").contains("os inconnu"));
}

#[test]
fn release_asset_table_blanche_couvre_toutes_les_plateformes() {
    // 6 combos : l'installateur desktop par OS + la CLI par OS. La table
    // est la SEULE source d'URL — hors table, rien n'est fabriqué.
    for (kind, os, attendu) in [
        (
            "desktop",
            "windows",
            "mcp-control-desktop-windows-x86_64-setup.exe",
        ),
        ("desktop", "macos", "mcp-control-desktop-macos-universal"),
        ("desktop", "linux", "mcp-control-desktop-linux-x86_64"),
        ("cli", "windows", "mcp-control-cli-windows-x86_64.exe"),
        ("cli", "macos", "mcp-control-cli-macos-universal"),
        ("cli", "linux", "mcp-control-cli-linux-x86_64"),
    ] {
        assert_eq!(release_asset(kind, os), Some(attendu), "{kind}/{os}");
    }
    assert_eq!(release_asset("cli", "beos"), None);
    assert_eq!(release_asset("autre", "windows"), None);
    assert_eq!(release_asset("", ""), None);
}

#[test]
fn sha256sums_somme_presente_est_lue_et_normalisee() {
    // Format exact de release.yml : `sha256sum * | sort -k2` → « somme  nom ».
    // La ligne binaire (`somme *nom`) et les majuscules restent acceptées.
    let a = "a".repeat(64);
    let sums = format!(
        "{a}  mcp-control-cli-linux-x86_64\n{} *mcp-control-desktop-windows-x86_64-setup.exe\n",
        "B".repeat(64)
    );
    assert_eq!(
        sha256_for_asset(&sums, "mcp-control-cli-linux-x86_64"),
        Sha256Lookup::Found(a)
    );
    assert_eq!(
        sha256_for_asset(&sums, "mcp-control-desktop-windows-x86_64-setup.exe"),
        Sha256Lookup::Found("b".repeat(64)),
        "ligne binaire `*nom` tolérée, majuscules normalisées"
    );
}

#[test]
fn sha256sums_somme_absente_ou_fichier_vide() {
    // Actif non listé = Absent, PAS une erreur : le fichier a été lu, la somme
    // manque — à distinguer d'un échec réseau (Err).
    let sums = format!("{}  mcp-control-cli-linux-x86_64\n", "c".repeat(64));
    assert_eq!(
        sha256_for_asset(&sums, "mcp-control-desktop-macos-universal"),
        Sha256Lookup::Absent
    );
    assert_eq!(
        sha256_for_asset("", "mcp-control-cli-linux-x86_64"),
        Sha256Lookup::Absent
    );
    assert_eq!(
        sha256_for_asset("\n\n  \n", "mcp-control-cli-linux-x86_64"),
        Sha256Lookup::Absent
    );
}

#[test]
fn sha256sums_somme_malformee_est_signalee() {
    let asset = "mcp-control-cli-windows-x86_64.exe";
    // somme tronquée (63 caractères)
    assert_eq!(
        sha256_for_asset(&format!("{}  {asset}\n", "d".repeat(63)), asset),
        Sha256Lookup::Malformed
    );
    // 64 caractères mais pas hexadécimaux
    assert_eq!(
        sha256_for_asset(&format!("{}z  {asset}\n", "d".repeat(63)), asset),
        Sha256Lookup::Malformed
    );
    // deux lignes contradictoires pour le même actif : on ne choisit pas
    let conflit = format!("{}  {asset}\n{}  {asset}\n", "d".repeat(64), "e".repeat(64));
    assert_eq!(sha256_for_asset(&conflit, asset), Sha256Lookup::Malformed);
    // ligne sans nom : ignorée, donc Absent et non Malformed
    assert_eq!(
        sha256_for_asset(&format!("{}\n", "d".repeat(64)), asset),
        Sha256Lookup::Absent
    );
}

#[test]
fn sha256sums_url_refuse_un_tag_hors_gabarit() {
    assert_eq!(
        sha256sums_url("v0.7.21").as_deref(),
        Some(
            "https://github.com/DeamonDev888/mcp-control/releases/download/v0.7.21/SHA256SUMS.txt"
        )
    );
    for mauvais in ["", "v1/../x", "v1?x=1", "v1#f", "v 1", &"9".repeat(65)] {
        assert_eq!(sha256sums_url(mauvais), None, "tag refusé : {mauvais:?}");
    }
    // Tag refusé ≠ réseau : l'échec est immédiat, sans lancer curl.
    assert!(expected_sha256("v1/../x", "mcp-control-cli-linux-x86_64").is_err());
}

#[test]
fn surface_actions_sans_doublon_et_dispatch_connu() {
    // La const partagée (UI / gateway / tests) ne doit jamais dériver du
    // dispatcher : pas de doublon, et un échantillon dispatché doit
    // répondre SANS « action inconnue ». Sont écartés de l'échantillon les
    // actions à effet de bord réel (scan, discover, ssh, open_path,
    // check_update, open_release_page) — couvertes par la suite E2E.
    let mut seen = std::collections::HashSet::new();
    for a in ACTIONS {
        assert!(seen.insert(*a), "action en double dans ACTIONS : {a}");
    }
    let state = test_state();
    for a in [
        "toggle",
        "refresh",
        "start",
        "kill",
        "kill_stdio",
        "start_stdio",
        "remove",
        "toggle_tool",
        "set_settings",
        "set_notify",
        "set_autostart",
        "reset_runtime",
        "reorder",
        "install_store",
    ] {
        let j = handle_action(&state, &format!("{{\"action\":\"{a}\"}}"));
        let err = j["error"].as_str().unwrap_or("");
        assert!(
            !err.contains("action inconnue"),
            "{a} listée dans ACTIONS mais rejetée par le dispatcher : {err}"
        );
    }
    // et l'inverse : une action hors surface est rejetée nommément
    let j = handle_action(&state, "{\"action\":\"cette_action_n_existe_pas\"}");
    assert!(j["error"]
        .as_str()
        .unwrap_or("")
        .contains("action inconnue"));
}

#[test]
fn set_favorite_epingle_persiste_et_se_relit() {
    // Bouton ⭐ de l'explorateur : épingler un serveur persiste le favori
    // dans config.json (seulement quand actif — défaut false), survit à la
    // relecture, est exposé à l'UI, et se retire proprement.
    let path = tmp_config("cfg_set_favorite.json", Some("{\"servers\":[]}"));
    let mut base = match Arc::try_unwrap(test_state()) {
        Ok(s) => s,
        Err(_) => panic!("test_state doit être unique"),
    };
    base.config_path = path.clone();
    let state = Arc::new(base);
    lockv(&state.servers).push(Server {
        id: "fav-srv".into(),
        label: "FAV".into(),
        transport: Transport::Http {
            host: None,
            port: 3_211,
            path: "/mcp".into(),
        },
        origin: Origin::Manual,
        enabled: true,
        auto_restart: true,
        favorite: false,
        state: SState::Down,
        pid: None,
        bound_host: None,
        server_name: None,
        version: None,
        protocol: None,
        sse: false,
        sse_endpoint: false,
        ws: false,
        lang: None,
        framework: None,
        via_ssh: false,
        tools: vec![],
        latency_ms: None,
        last_seen_ts: None,
        last_error: None,
        start_cmd: None,
        tools_disabled: vec![],
    });

    // 1. épingle → ok + état interne + persisté dans config.json
    let j = handle_action(
        &state,
        "{\"action\":\"set_favorite\",\"id\":\"fav-srv\",\"enabled\":true}",
    );
    assert_eq!(j["ok"], serde_json::json!(true));
    assert_eq!(j["favorite"], serde_json::json!(true));
    assert!(lockv(&state.servers)[0].favorite);
    let cfg = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(cfg.contains("\"favorite\": true"), "persisté : {}", cfg);

    // 2. /api/state l'expose (l'UI épingle d'après ce champ)
    let full = state_json(&state);
    assert_eq!(full["servers"][0]["favorite"], serde_json::json!(true));

    // 3. relecture : le favori survit au redémarrage du moteur
    let (servers, _) = load_config(&path);
    let s = servers.iter().find(|s| s.id == "fav-srv").unwrap();
    assert!(s.favorite, "le favori doit être relu du disque");

    // 4. retrait → la clé disparaît (défaut false, config lisible)
    let j2 = handle_action(
        &state,
        "{\"action\":\"set_favorite\",\"id\":\"fav-srv\",\"enabled\":false}",
    );
    assert_eq!(j2["ok"], serde_json::json!(true));
    let cfg2 = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(!cfg2.contains("favorite"), "défaut non écrit : {}", cfg2);

    // 5. serveur inconnu → erreur nette
    let j3 = handle_action(
        &state,
        "{\"action\":\"set_favorite\",\"id\":\"nope\",\"enabled\":true}",
    );
    assert_eq!(j3["ok"], serde_json::json!(false));
    assert_eq!(j3["error"], serde_json::json!("serveur inconnu"));
}

#[test]
fn reset_state_placements_detection_et_oublis() {
    // Bouton ⚙ → Réinitialiser : placement (ordre), detection (découverts
    // + imports natifs + pairs), oublis (seeds retirés réinjectés + machines
    // SSH) — et « what » inconnu refuse proprement.
    let path = tmp_config("cfg_reset.json", Some("{\"servers\":[]}"));
    let mut base = match Arc::try_unwrap(test_state()) {
        Ok(s) => s,
        Err(_) => panic!("test_state doit être unique"),
    };
    base.config_path = path.clone();
    let state = Arc::new(base);
    {
        let mut st = lockv(&state.settings);
        st.autostart = false;
        st.server_order = vec!["a".into(), "b".into()];
        st.ssh_hosts.push(crate::types::SshHost {
            ip: "192.168.40.9".into(),
            user: "demon".into(),
        });
        // un identifiant de seed retiré durablement (les seeds par défaut
        // n'existent plus : le réglage reste lu, rien n'est réinjecté)
        st.removed_seeds.push("seed-news".to_string());
    }
    {
        let mut servers = lockv(&state.servers);
        servers.push(Server {
            id: "disc-1".into(),
            label: "port-7000".into(),
            transport: Transport::Http {
                host: None,
                port: 7000,
                path: "/mcp".into(),
            },
            origin: Origin::Discovered,
            enabled: true,
            state: SState::Down,
            pid: None,
            bound_host: None,
            server_name: None,
            version: None,
            protocol: None,
            sse: false,
            sse_endpoint: false,
            ws: false,
            lang: None,
            framework: None,
            via_ssh: false,
            tools: vec![],
            latency_ms: None,
            last_seen_ts: None,
            last_error: None,
            start_cmd: None,
            auto_restart: true,
            favorite: false,
            tools_disabled: vec![],
        });
        servers.push(Server {
            id: "litemcp-test".into(),
            label: "test".into(),
            transport: Transport::Http {
                host: None,
                port: 8888,
                path: "/mcp".into(),
            },
            origin: Origin::Manual,
            enabled: true,
            state: SState::Down,
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
            via_ssh: false,
            tools: vec![],
            latency_ms: None,
            last_seen_ts: None,
            last_error: None,
            start_cmd: None,
            auto_restart: true,
            favorite: false,
            tools_disabled: vec![],
        });
        servers.push(Server {
            id: "man-keep".into(),
            label: "keep".into(),
            transport: Transport::Http {
                host: None,
                port: 9000,
                path: "/mcp".into(),
            },
            origin: Origin::Manual,
            enabled: true,
            state: SState::Down,
            pid: None,
            bound_host: None,
            server_name: None,
            version: None,
            protocol: None,
            sse: false,
            sse_endpoint: false,
            ws: false,
            lang: None,
            framework: None,
            via_ssh: false,
            tools: vec![],
            latency_ms: None,
            last_seen_ts: None,
            last_error: None,
            start_cmd: None,
            auto_restart: true,
            favorite: false,
            tools_disabled: vec![],
        });
    }
    lockv(&state.peers).insert(
        "10.1.1.1".into(),
        crate::types::Peer {
            ip: "10.1.1.1".into(),
            ui_port: 5090,
            last_seen: 1,
            servers: serde_json::json!([]),
        },
    );

    // 1. placement → server_order vidé, serveurs intacts
    let j = handle_action(
        &state,
        "{\"action\":\"reset_state\",\"what\":\"placement\"}",
    );
    assert_eq!(j["ok"], serde_json::json!(true));
    assert!(lockv(&state.settings).server_order.is_empty());
    assert_eq!(lockv(&state.servers).len(), 3);

    // 2. detection → découverts + litemcp retirés, manual conservé, pairs vidés
    let j2 = handle_action(
        &state,
        "{\"action\":\"reset_state\",\"what\":\"detection\"}",
    );
    assert_eq!(j2["ok"], serde_json::json!(true));
    assert_eq!(j2["removed"], serde_json::json!(2));
    {
        let servers = lockv(&state.servers);
        assert_eq!(servers.len(), 1);
        assert_eq!(
            servers[0].id, "man-keep",
            "le manual hors litemcp est conservé"
        );
    }
    assert!(lockv(&state.peers).is_empty());

    // 3. oublis → machines SSH vidées + removed_seeds vide + AUCUNE
    // réinjection (plus de serveur par défaut depuis le lot S4)
    let j3 = handle_action(&state, "{\"action\":\"reset_state\",\"what\":\"oublis\"}");
    assert_eq!(j3["ok"], serde_json::json!(true));
    assert_eq!(
        j3["reinjected"],
        serde_json::json!(0),
        "aucun seed ne doit être réinjecté"
    );
    {
        let st = lockv(&state.settings);
        assert!(st.ssh_hosts.is_empty());
        assert!(st.removed_seeds.is_empty());
    }
    assert_eq!(
        lockv(&state.servers).len(),
        1,
        "seul le serveur manuel conservé reste : rien n'est réinjecté"
    );

    // 4. what inconnu → refus propre, rien ne change
    let n = lockv(&state.servers).len();
    let j4 = handle_action(&state, "{\"action\":\"reset_state\",\"what\":\"nimporte\"}");
    assert_eq!(j4["ok"], serde_json::json!(false));
    assert_eq!(lockv(&state.servers).len(), n);
}

#[test]
fn discovery_candidates_exclut_connus_et_port_ui() {
    // ports en écoute : 5433/3000 déjà connus, 5090 = notre UI, 8787 = nouveau
    let ports = vec![3000, 5090, 5433, 8787];
    let known = vec![3000, 5433];
    assert_eq!(discovery_candidates(&ports, &known, 5090), vec![8787]);
    // notre propre port UI n'est jamais sonder, même inconnu
    assert!(discovery_candidates(&[5090], &[], 5090).is_empty());
    assert!(discovery_candidates(&[], &known, 5090).is_empty());
}

#[test]
fn cleanup_name_hermes() {
    assert_eq!(
        cleanup_name("@modelcontextprotocol/server-filesystem"),
        "filesystem"
    );
    assert_eq!(cleanup_name("mcp-server-git"), "git");
    assert_eq!(cleanup_name("mcp-server-time"), "time");
}

#[test]
fn parse_servers_doc_accepte_les_trois_formes() {
    let w = r#"{"mcpServers":{"fs":{"command":"npx","args":["-y","pkg"]}}}"#;
    let e = parse_servers_doc(w).unwrap();
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].0, "fs");
    let snake = r#"{"mcp_servers":{"fs":{"command":"npx"}}}"#;
    assert_eq!(parse_servers_doc(snake).unwrap().len(), 1);
    let brut = r#"{"fs":{"command":"npx"}}"#;
    assert_eq!(parse_servers_doc(brut).unwrap().len(), 1);
    assert!(parse_servers_doc("not json").is_err());
}

#[test]
fn import_mcp_texte_reconnait_tout() {
    let state = test_state();
    // URL (http : https est refusé explicitement, pas de TLS dans ce client)
    let j = import_mcp_text(&state, "http://mcp.linear.app/mcp");
    assert_eq!(j["ok"], json!(true));
    // commande claude
    let j2 = import_mcp_text(&state, "claude mcp add monserveur npx -y pkg");
    assert_eq!(j2["ok"], json!(true));
    // commande npx brute
    let j3 = import_mcp_text(
        &state,
        "npx -y @modelcontextprotocol/server-filesystem /tmp",
    );
    assert_eq!(j3["ok"], json!(true));
    let servers = lockv(&state.servers);
    assert!(servers.iter().any(|s| s.id == "store-linear"));
    assert!(servers.iter().any(|s| s.label.starts_with("monserveur")));
    assert!(servers
        .iter()
        .any(|s| s.id.starts_with("store-filesystem-")));
}

#[test]
fn import_mcp_texte_court_ne_panique_pas() {
    // Régression audit 2026-10-24 : « claude mcp add » (3-4 tokens) indexait
    // toks[3]/toks[4] → panic du thread HTTP, aucune réponse au client.
    let state = test_state();
    for texte in ["claude mcp add", "claude mcp add monserveur"] {
        let j = import_mcp_text(&state, texte);
        assert_eq!(
            j["ok"],
            json!(false),
            "'{texte}' doit être refusé proprement (pas de panic)"
        );
    }
    assert!(lockv(&state.servers).is_empty());
}

#[test]
fn import_https_est_refuse_avec_message_clair() {
    // Régression audit : une URL https finissait en sonde plaintext illisible (DOWN permanent)
    let state = test_state();
    let j = import_mcp_text(&state, "https://mcp.linear.app/mcp");
    assert_eq!(j["ok"], json!(false));
    assert!(j["error"].as_str().unwrap().contains("TLS"));
    assert!(lockv(&state.servers).is_empty());
}

#[test]
fn install_store_filesystem_exige_un_dossier_explicite() {
    // Régression audit : le catalogue embarquait C:/Users/<user> en dur (profil entier exposé)
    let state = test_state();
    // sans args → refus AVANT tout spawn (poll_one jamais atteint)
    let j = handle_action(&state, r#"{"action":"install_store","id":"filesystem"}"#);
    assert_eq!(j["ok"], json!(false));
    assert!(lockv(&state.servers).is_empty());
    // critère pur : le nom du package contient un « / » — seule la LONGUEUR protège
    let pkg = "-y".to_string();
    let nm = "@modelcontextprotocol/server-filesystem".to_string();
    assert!(filesystem_dossier_manquant(&[pkg.clone(), nm.clone()]));
    assert!(!filesystem_dossier_manquant(&[
        pkg,
        nm,
        "C:/data".to_string()
    ]));
    assert!(filesystem_dossier_manquant(&[
        "-y".to_string(),
        "@modelcontextprotocol/server-filesystem".to_string(),
        "relatif-sans-separateur".to_string()
    ]));
}

#[test]
fn settings_defaut_loopback_et_api_access_lu_de_la_config() {
    // Régression audit : le bind par défaut 0.0.0.0 rendait l'API contrôlable du LAN
    let (_, st) = load_config(&tmp_config("cfg_defaut_audit.json", None));
    assert_eq!(st.ui_bind, "127.0.0.1");
    assert!(st.api_access.is_none());
    assert!(st.allowed_origins.is_empty());
    let cfg = r#"{"servers":[],"settings":{"ui_bind":"0.0.0.0","api_access":"valeur-partagee-pair","allowed_origins":["http://mon-host.local:5090"]}}"#;
    let (_, st2) = load_config(&tmp_config("cfg_access.json", Some(cfg)));
    assert_eq!(st2.ui_bind, "0.0.0.0");
    assert_eq!(st2.api_access.as_deref(), Some("valeur-partagee-pair"));
    assert_eq!(
        st2.allowed_origins,
        vec!["http://mon-host.local:5090".to_string()]
    );
}

#[test]
fn name_from_url_hermes() {
    assert_eq!(name_from_url("mcp.linear.app"), "linear");
    assert_eq!(name_from_url("api.github.com"), "github");
    assert_eq!(name_from_url("example.com"), "example");
}

#[test]
fn detect_lang_stdio_par_commande() {
    let none: Vec<String> = vec![];
    assert_eq!(
        crate::probe::detect_lang_stdio("node", &none).as_deref(),
        Some("node")
    );
    assert_eq!(
        crate::probe::detect_lang_stdio("npx", &none).as_deref(),
        Some("node")
    );
    assert_eq!(
        crate::probe::detect_lang_stdio("uvx", &none).as_deref(),
        Some("python")
    );
    assert_eq!(
        crate::probe::detect_lang_stdio("C:/Python311/python.exe", &none).as_deref(),
        Some("python")
    );
    assert_eq!(
        crate::probe::detect_lang_stdio("cargo", &none).as_deref(),
        Some("rust")
    );
    assert_eq!(
        crate::probe::detect_lang_stdio("java", &none).as_deref(),
        Some("java")
    );
    assert_eq!(
        crate::probe::detect_lang_stdio("dotnet", &none).as_deref(),
        Some("dotnet")
    );
    assert_eq!(
        crate::probe::detect_lang_stdio("mybinary", &["server.py".to_string()]).as_deref(),
        Some("python")
    );
    assert_eq!(
        crate::probe::detect_lang_stdio("mybinary", &["index.js".to_string()]).as_deref(),
        Some("node")
    );
    assert_eq!(crate::probe::detect_lang_stdio("strange", &none), None);
}

#[test]
fn detect_lang_http_par_entetes() {
    let mk = |server: &str, powered: &str| crate::probe::RespHead {
        content_type: String::new(),
        server: server.to_string(),
        powered_by: powered.to_string(),
        session_id: String::new(),
    };
    assert_eq!(
        crate::probe::detect_lang_http(&mk("uvicorn", ""), None).as_deref(),
        Some("python")
    );
    assert_eq!(
        crate::probe::detect_lang_http(&mk("", "Express"), None).as_deref(),
        Some("node")
    );
    assert_eq!(
        crate::probe::detect_lang_http(&mk("hyper", ""), None).as_deref(),
        Some("rust")
    );
    assert_eq!(
        crate::probe::detect_lang_http(&mk("Kestrel", ""), None).as_deref(),
        Some("dotnet")
    );
    assert_eq!(
        crate::probe::detect_lang_http(&mk("Jetty(11)", ""), None).as_deref(),
        Some("java")
    );
    assert_eq!(crate::probe::detect_lang_http(&mk("", ""), None), None);
    assert_eq!(
        crate::probe::detect_lang_http(&mk("", ""), Some("my-rust-server")).as_deref(),
        Some("rust")
    );
}

#[test]
fn resolve_config_path_override_gagne() {
    let p = resolve_config_path("C:/nimporte", Some("D:/ma/config.json".to_string()));
    assert_eq!(p, "D:/ma/config.json");
}

#[test]
fn resolve_config_path_dev_existant_gagne() {
    let dir = std::env::temp_dir().join("mcpcc_resolve_dev");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.json"), "{}").unwrap();
    let p = resolve_config_path(&dir.to_string_lossy(), None);
    assert_eq!(p, dir.join("config.json").to_string_lossy().to_string());
}

#[test]
fn resolve_config_path_sans_config_pointe_vers_exe() {
    let dir = std::env::temp_dir().join("mcpcc_resolve_vide");
    std::fs::create_dir_all(&dir).unwrap();
    let _ = std::fs::remove_file(dir.join("config.json"));
    let p = resolve_config_path(&dir.to_string_lossy(), None);
    assert!(
        p.ends_with("config.json"),
        "doit finir par config.json, got {}",
        p
    );
    assert!(
        !p.starts_with(&dir.to_string_lossy().to_string()),
        "ne doit PAS pointer vers le manifest (machine cliente), got {}",
        p
    );
}

#[test]
fn http_avec_start_cmd_charge_et_persiste() {
    let cfg = r#"{"servers":[{"id":"h1","label":"H","transport":"http","port":3210,"path":"/mcp","program":"node","args":["srv.js","--p","3210"],"workdir":"C:/s","enabled":true}]}"#;
    let path = tmp_config("cfg_http_cmd.json", Some(cfg));
    let (servers, _) = load_config(&path);
    let s = servers.iter().find(|s| s.id == "h1").unwrap();
    let cmd = s
        .start_cmd
        .as_ref()
        .expect("start_cmd http doit être chargé du json");
    assert_eq!(cmd.program, "node");
    assert_eq!(
        cmd.args,
        vec!["srv.js".to_string(), "--p".to_string(), "3210".to_string()]
    );
    assert_eq!(cmd.workdir.as_deref(), Some("C:/s"));
    // round-trip save/reload
    let state = AppState {
        servers: Mutex::new(servers.clone()),
        log: Mutex::new(VecDeque::new()),
        settings: Mutex::new(Settings::default()),
        children: Mutex::new(std::collections::HashMap::new()),
        config_path: path.clone(),
        peers: Mutex::new(std::collections::HashMap::new()),
        allowed_origins: Mutex::new(Vec::new()),
        allowed_hosts: Mutex::new(Vec::new()),
        spawn_fails: Mutex::new(std::collections::HashMap::new()),
        runtime: Mutex::new(crate::runtime::RuntimeLog::new()),
        spawning: Mutex::new(std::collections::HashSet::new()),
        update_latest: Mutex::new(None),
        lan_ok: Mutex::new(None),
    };
    save_config(&state);
    let (servers2, _) = load_config(&path);
    let s2 = servers2.iter().find(|s| s.id == "h1").unwrap();
    let cmd2 = s2
        .start_cmd
        .as_ref()
        .expect("start_cmd http doit survivre au round-trip");
    assert_eq!(cmd2.program, "node");
    assert_eq!(
        cmd2.args,
        vec!["srv.js".to_string(), "--p".to_string(), "3210".to_string()]
    );
}

// ==================== Lot 3 — persistance, import, cycle de vie ====================

/// AppState de test (config_path passé tel quel : "" = aucune écriture fichier).
fn state_lot3(chemin: &str) -> Arc<AppState> {
    Arc::new(AppState {
        servers: Mutex::new(Vec::new()),
        log: Mutex::new(VecDeque::new()),
        settings: Mutex::new(Settings::default()),
        children: Mutex::new(std::collections::HashMap::new()),
        config_path: chemin.to_string(),
        peers: Mutex::new(std::collections::HashMap::new()),
        allowed_origins: Mutex::new(Vec::new()),
        allowed_hosts: Mutex::new(Vec::new()),
        spawn_fails: Mutex::new(std::collections::HashMap::new()),
        runtime: Mutex::new(crate::runtime::RuntimeLog::new()),
        spawning: Mutex::new(std::collections::HashSet::new()),
        update_latest: Mutex::new(None),
        lan_ok: Mutex::new(None),
    })
}

/// (lot 3, audit 03 constat 5) : l'env d'un bloc mcpServers est conservé à
/// l'import, porté par le transport ET la commande de lancement, survit au
/// round-trip config.json — et ses VALEURS ne sortent jamais par /api/state
/// ni /api/detail (noms seulement).
#[test]
fn import_env_stdio_persiste_et_ne_fuit_pas_par_lapi() {
    let entry: Value = serde_json::from_str(
        r#"{"command":"node","args":["srv.js"],"env":{"API_KEY":"valeur-secrete","AUTRE":"x2"}}"#,
    )
    .unwrap();
    let srv = entry_to_server("store-demo".into(), "demo", &entry).expect("entrée reconnue");
    match &srv.transport {
        Transport::Stdio { env, .. } => {
            assert_eq!(env.len(), 2, "env jeté à l'import");
            assert!(env.contains(&("API_KEY".to_string(), "valeur-secrete".to_string())));
        }
        _ => panic!("transport stdio attendu"),
    }
    let c = srv.start_cmd.as_ref().expect("start_cmd stdio");
    assert!(c
        .env
        .contains(&("API_KEY".to_string(), "valeur-secrete".to_string())));

    // round-trip config.json : l'env survit à save/load
    let path = tmp_config("cfg_env_import.json", None);
    let state = state_lot3(&path);
    lockv(&state.servers).push(srv);
    save_config(&state);
    let (servers, _) = load_config(&path);
    let s2 = servers
        .iter()
        .find(|s| s.id == "store-demo")
        .expect("persisté");
    match &s2.transport {
        Transport::Stdio { env, .. } => {
            assert_eq!(env.len(), 2, "env perdu au round-trip config");
            assert!(env
                .iter()
                .any(|(k, v)| k == "API_KEY" && v == "valeur-secrete"));
        }
        _ => panic!("transport stdio attendu"),
    }

    // API : noms SEULS — la valeur ne doit apparaître nulle part
    let etat = server_json(s2, false);
    let texte = etat.to_string();
    assert!(
        !texte.contains("valeur-secrete"),
        "VALEUR d'env exposée : {texte}"
    );
    assert!(texte.contains("API_KEY"), "nom d'env attendu : {texte}");
    let cmd_json = etat
        .get("start_cmd")
        .expect("start_cmd exposée")
        .to_string();
    assert!(
        !cmd_json.contains("valeur-secrete"),
        "VALEUR exposée : {cmd_json}"
    );
    assert!(
        cmd_json.contains("API_KEY"),
        "nom d'env attendu : {cmd_json}"
    );
    let detail = detail_json(&state, "store-demo").to_string();
    assert!(
        !detail.contains("valeur-secrete"),
        "VALEUR exposée par /api/detail"
    );
}

/// (lot 3, audit 03 constat 6) : une entrée {"url":"https://…"} d'un bloc JSON
/// était ignorée en silence avec ok:true — elle est désormais listée dans
/// `skipped` avec le même motif que le collage d'une URL https directe.
#[test]
fn import_json_url_https_listee_dans_skipped_avec_motif() {
    let state = state_lot3("");
    let raw = r#"{"mcpServers":{"aud-https":{"url":"https://exemple.org/mcp"},"aud-http":{"url":"http://127.0.0.1:3196/mcp"},"aud-bizarre":{"foo":1}}}"#;
    let rep = import_mcp_text(&state, raw);
    assert_eq!(rep.get("ok").and_then(|o| o.as_bool()), Some(true), "{rep}");
    assert_eq!(rep.get("added").and_then(|a| a.as_u64()), Some(1), "{rep}");
    let skipped = rep
        .get("skipped")
        .and_then(|s| s.as_array())
        .expect("liste skipped attendue");
    assert_eq!(skipped.len(), 2, "{rep}");
    let https = skipped
        .iter()
        .find(|s| s.get("name").and_then(|n| n.as_str()) == Some("aud-https"))
        .expect("entrée https absente de skipped");
    let motif = https.get("reason").and_then(|r| r.as_str()).unwrap_or("");
    assert!(
        motif.contains("TLS non supporté"),
        "motif attendu : {motif}"
    );
    let biz = skipped
        .iter()
        .find(|s| s.get("name").and_then(|n| n.as_str()) == Some("aud-bizarre"))
        .expect("entrée non reconnaissable absente de skipped");
    assert!(!biz
        .get("reason")
        .and_then(|r| r.as_str())
        .unwrap_or("")
        .is_empty());
    assert!(
        lockv(&state.servers)
            .iter()
            .any(|s| s.id == "store-aud-http"),
        "l'entrée http:// doit être importée"
    );
}

/// (lot 3, audit 03 constat 7) : 50 add simultanés → 50 présents dans le
/// fichier relu. Les écritures sont sérialisées (verrou global + tmp unique) :
/// le dernier écrivain voit toujours les 50 pushs (chaque thread pousse AVANT
/// de sauvegarder, et le verrou sérialise clone→écriture).
#[test]
fn save_config_concurrent_50_adds_tous_persistes() {
    let path = tmp_config("cfg_concurent_50.json", None);
    let state = state_lot3(&path);
    std::thread::scope(|scope| {
        for i in 0..50u32 {
            let state = &state;
            scope.spawn(move || {
                lockv(&state.servers).push(Server {
                    id: format!("conc-{i}"),
                    label: format!("Concurrent {i}"),
                    transport: Transport::Http {
                        host: None,
                        port: 30000 + i as u16,
                        path: "/mcp".into(),
                    },
                    origin: Origin::Manual,
                    enabled: true,
                    state: SState::Down,
                    pid: None,
                    bound_host: None,
                    server_name: None,
                    version: None,
                    protocol: None,
                    sse: false,
                    sse_endpoint: false,
                    ws: false,
                    lang: None,
                    framework: None,
                    via_ssh: false,
                    tools: vec![],
                    latency_ms: None,
                    last_seen_ts: None,
                    last_error: None,
                    start_cmd: None,
                    auto_restart: true,
                    favorite: false,
                    tools_disabled: vec![],
                });
                save_config(state);
            });
        }
    });
    let (servers, _) = load_config(&path);
    for i in 0..50u32 {
        assert!(
            servers.iter().any(|s| s.id == format!("conc-{i}")),
            "conc-{i} perdu : écriture concurrente non sérialisée"
        );
    }
}

/// (lot 3, audit 03 constat 8) : config.json ILLISIBLE (JSON invalide) =
/// renommage en config.json.corrompu-<horodatage> + repart des défauts (et
/// plus un repli silencieux qui écrasait la config à la première sauvegarde).
#[test]
fn config_corrompue_renommee_et_defauts() {
    let path = tmp_config("cfg_corrompue.json", Some("{servers: [ pas du json"));
    let (servers, _) = load_config(&path);
    assert!(
        servers.is_empty(),
        "défauts attendus après config corrompue (liste vide depuis le lot S4)"
    );
    assert!(
        !std::path::Path::new(&path).exists(),
        "l'original doit être renommé"
    );
    let prefixe = format!("{path}.corrompu-");
    let parent = std::path::Path::new(&path).parent().unwrap();
    assert!(
        std::fs::read_dir(parent)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.path().to_string_lossy().starts_with(&prefixe)),
        "un config.json.corrompu-<horodatage> doit exister"
    );
}

/// (lot 3, audit 03 constat 8) : fichier VIDE = même traitement que corrompu.
#[test]
fn config_vide_renommee_comme_corrompue() {
    let path = tmp_config("cfg_vide.json", Some(""));
    let (servers, _) = load_config(&path);
    assert!(
        servers.is_empty(),
        "défauts attendus après config vide (liste vide depuis le lot S4)"
    );
    assert!(!std::path::Path::new(&path).exists(), "doit être renommé");
}

/// CRITÈRE lot 3 : une config de la v0.7.20 (sans champ env) se recharge sans
/// perte — champ env absent → vide, tous les autres champs conservés, et le
/// round-trip save/load ne change rien.
#[test]
fn config_v0_7_20_se_recharge_sans_perte() {
    let cfg = r#"{
  "servers": [
    {"id":"old-http","label":"HTTP d'époque","transport":"http","host":"127.0.0.1","port":3101,"path":"/mcp","enabled":true},
    {"id":"old-stdio","label":"STDIO d'époque","transport":"stdio","program":"node","args":["srv.js"],"workdir":"C:/tmp","enabled":false},
    {"id":"old-lance","label":"Lancé","transport":"http","port":3102,"path":"/mcp","enabled":true,"program":"node","args":["launch.cjs"],"workdir":"C:/tmp"}
  ],
  "settings":{"refresh_secs":30,"ui_port":5090,"removed_seeds":["seed-news"],"api_access":"code-ancien"}
}"#;
    let path = tmp_config("cfg_v0720.json", Some(cfg));
    let (servers, st) = load_config(&path);
    let http = servers
        .iter()
        .find(|s| s.id == "old-http")
        .expect("http conservé");
    match &http.transport {
        Transport::Http {
            host,
            port,
            path: p,
        } => {
            assert_eq!(host.as_deref(), Some("127.0.0.1"));
            assert_eq!(*port, 3101);
            assert_eq!(p, "/mcp");
        }
        _ => panic!("transport http attendu"),
    }
    let stdio = servers
        .iter()
        .find(|s| s.id == "old-stdio")
        .expect("stdio conservé");
    match &stdio.transport {
        Transport::Stdio {
            program,
            args,
            workdir,
            env,
        } => {
            assert_eq!(program, "node");
            assert_eq!(args, &vec!["srv.js".to_string()]);
            assert_eq!(workdir.as_deref(), Some("C:/tmp"));
            assert!(env.is_empty(), "champ env absent → vide (compat v0.7.20)");
        }
        _ => panic!("transport stdio attendu"),
    }
    let lance = servers
        .iter()
        .find(|s| s.id == "old-lance")
        .expect("lancé conservé");
    let c = lance
        .start_cmd
        .as_ref()
        .expect("start_cmd http reconstruite");
    assert_eq!(c.program, "node");
    assert!(c.env.is_empty());
    assert_eq!(st.refresh_secs, 30);
    assert_eq!(st.ui_port, 5090);
    assert_eq!(st.removed_seeds, vec!["seed-news".to_string()]);
    assert_eq!(st.api_access.as_deref(), Some("code-ancien"));

    // round-trip : sauvegarde puis rechargement — rien ne se perd
    let state = state_lot3(&path);
    *lockv(&state.servers) = servers.clone();
    *lockv(&state.settings) = st.clone();
    save_config(&state);
    let (servers2, st2) = load_config(&path);
    assert!(servers2.iter().any(|s| s.id == "old-http"));
    let stdio2 = servers2.iter().find(|s| s.id == "old-stdio").unwrap();
    match &stdio2.transport {
        Transport::Stdio {
            program, args, env, ..
        } => {
            assert_eq!(program, "node");
            assert_eq!(args, &vec!["srv.js".to_string()]);
            assert!(env.is_empty());
        }
        _ => panic!("transport stdio attendu"),
    }
    assert_eq!(st2.refresh_secs, 30);
    assert_eq!(st2.removed_seeds, vec!["seed-news".to_string()]);
    assert_eq!(st2.api_access.as_deref(), Some("code-ancien"));
}

/// (lot 3, audit 03 constat 10) : plafonnement de plage — pur et testé.
#[test]
fn scan_plafonne_borne_la_plage_a_512() {
    assert_eq!(
        scan_plafonne(3000, 3200),
        (3000, 3200),
        "201 ports : inchangé"
    );
    assert_eq!(scan_plafonne(1, 3000), (1, 512), "tronqué en tête");
    assert_eq!(scan_plafonne(64000, 65535), (64000, 64511));
    assert_eq!(
        scan_plafonne(65100, 65535),
        (65100, 65535),
        "436 ports : sous le plafond, inchangé"
    );
}

/// (lot 3, audit 03 constat 10) : le handler refuse CLAIREMENT une plage
/// explicite au-delà du plafond (le scan synchrone figeait l'API des heures).
#[test]
fn scan_handler_refuse_une_plage_trop_large() {
    let state = state_lot3("");
    let rep = handle_action(
        &state,
        &json!({"action": "scan", "from": 1, "to": 3000}).to_string(),
    );
    assert_eq!(
        rep.get("ok").and_then(|o| o.as_bool()),
        Some(false),
        "{rep}"
    );
    let err = rep.get("error").and_then(|e| e.as_str()).unwrap_or("");
    assert!(err.contains("512"), "le plafond doit être cité : {err}");
}

/// node est-il disponible ? (les tests qui lancent un vrai serveur sautent sinon)
fn node_disponible_tests() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// (lot 3 constat 9, refondu lot S4 point 1) : un serveur lancé par l'app via
/// un lanceur qui DÉTACHE son enfant (ici node → node détaché) laisse un
/// process que l'app ne peut plus identifier (le handle du lanceur est mort).
/// `stop_server` REFUSE alors de le tuer (nom + pid rendus à l'appelant) ;
/// `stop_server_force`, geste explicite, l'arrête et libère le port.
#[test]
fn stop_refuse_puis_force_arrete_un_orphelin_detache() {
    if !node_disponible_tests() {
        eprintln!("SKIP stop_refuse_puis_force_arrete_un_orphelin_detache : node indisponible");
        return;
    }
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let dir = std::env::temp_dir().join("mcpcc_tests");
    std::fs::create_dir_all(&dir).unwrap();
    let js = dir.join("orphelin_lot3.js");
    std::fs::write(
        &js,
        format!(
            "require('http').createServer((q,s)=>{{s.writeHead(200);s.end('ok')}}).listen({port},'127.0.0.1');\n"
        ),
    )
    .unwrap();
    // ceinture-bretelles : jamais de node orphelin résiduel après le test
    struct PortNettoie(u16);
    impl Drop for PortNettoie {
        fn drop(&mut self) {
            probe::kill_port(self.0);
        }
    }
    let _garde = PortNettoie(port);

    let state = state_lot3("");
    let js_arg = js.display().to_string().replace('\\', "/");
    let script = format!(
        "const cp=require('child_process');const p=cp.spawn(process.execPath,['{}'],{{detached:true,stdio:'ignore'}});p.unref();",
        js_arg
    );
    let lanceur = probe::spawn_process("node", &["-e".to_string(), script], None, &[])
        .expect("spawn du lanceur");
    lockv(&state.children).insert("lc-detach".to_string(), lanceur);
    lockv(&state.servers).push(srv_http(
        "lc-detach",
        None,
        "Lanceur détaché",
        None,
        port,
        Origin::Manual,
        None,
    ));

    // le process détaché doit écouter avant l'arrêt (attente bornée)
    let mut ouvert = false;
    for _ in 0..50 {
        if probe::port_open("127.0.0.1", port, std::time::Duration::from_millis(500)) {
            ouvert = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    assert!(ouvert, "l'orphelin doit écouter avant l'arrêt");
    // laisser le lanceur sortir : le handle ne désigne plus le process du port
    std::thread::sleep(std::time::Duration::from_millis(800));

    // 1. stop_server refuse : le port est tenu par un process non identifié
    let rep = handle_action(
        &state,
        &json!({"action": "kill", "id": "lc-detach"}).to_string(),
    );
    assert_eq!(
        rep.get("ok").and_then(|o| o.as_bool()),
        Some(false),
        "stop_server doit refuser : {rep}"
    );
    assert_eq!(rep.get("needs_force"), Some(&json!(true)), "{rep}");
    let err = rep.get("error").and_then(|e| e.as_str()).unwrap_or("");
    assert!(
        err.contains(&format!("{port}")),
        "le refus doit nommer le port : {err}"
    );
    assert!(
        err.contains("pid"),
        "le refus doit nommer le process (nom + pid) : {err}"
    );
    assert!(
        probe::port_open("127.0.0.1", port, std::time::Duration::from_millis(500)),
        "le process tiers ne doit PAS être tué par stop_server"
    );

    // 2. « arrêter quand même » : geste explicite, le port est libéré
    let rep = handle_action(
        &state,
        &json!({"action": "stop_server_force", "id": "lc-detach"}).to_string(),
    );
    assert_eq!(rep.get("ok").and_then(|o| o.as_bool()), Some(true), "{rep}");
    assert_eq!(rep.get("forced"), Some(&json!(true)));
    let mut libere = false;
    for _ in 0..40 {
        if !probe::port_open("127.0.0.1", port, std::time::Duration::from_millis(500)) {
            libere = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    assert!(libere, "stop_server_force doit libérer le port");
}

/// Supprimer une entrée ne tue JAMAIS un process que l'app n'a pas lancé :
/// le port reste tenu, l'entrée disparaît, le journal explique.
#[test]
fn remove_ne_tue_pas_un_processus_tiers() {
    if !node_disponible_tests() {
        eprintln!("SKIP remove_ne_tue_pas_un_processus_tiers : node indisponible");
        return;
    }
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let mut enfant = std::process::Command::new("node")
        .args([
            "-e",
            &format!("require('http').createServer((q,s)=>s.end('ok')).listen({port},'127.0.0.1')"),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("node");
    for _ in 0..50 {
        if probe::port_open("127.0.0.1", port, std::time::Duration::from_millis(500)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let state = state_lot3("");
    lockv(&state.servers).push(srv_http(
        "tiers-remove",
        None,
        "Tiers",
        None,
        port,
        Origin::Manual,
        None,
    ));
    let rep = handle_action(
        &state,
        &json!({"action": "remove", "id": "tiers-remove"}).to_string(),
    );
    assert_eq!(rep.get("ok").and_then(|o| o.as_bool()), Some(true), "{rep}");
    assert!(
        lockv(&state.servers).iter().all(|s| s.id != "tiers-remove"),
        "l'entrée doit être supprimée"
    );
    assert!(
        probe::port_open("127.0.0.1", port, std::time::Duration::from_millis(500)),
        "le process tiers doit survivre à la suppression (non lancé par l'app)"
    );
    let _ = enfant.kill();
    let _ = enfant.wait();
}

/// Audit S4, point 1 (PID recyclé + arrêt positif) :
/// - Négatif : si un handle enregistré porte un PID vivant dont la date de
///   création diffère (PID recyclé par l'OS), `kill` et `toggle` OFF ne
///   doivent PAS le tuer.
/// - Positif : si l'app a lancé le process (même PID + même date), `kill` le tue.
#[cfg(windows)]
#[test]
fn kill_et_toggle_ne_tuent_pas_un_pid_recycle_a_date_differente() {
    if !node_disponible_tests() {
        eprintln!(
            "SKIP kill_et_toggle_ne_tuent_pas_un_pid_recycle_a_date_differente : node indisponible"
        );
        return;
    }
    let mut innocent = std::process::Command::new("node")
        .args(["-e", "setInterval(()=>{},60000)"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("node innocent");
    let pid_innocent = innocent.id();
    let vraie_date = probe::process_creation(pid_innocent).expect("date de création lisible");

    // 1. Négatif : handle avec fausse date de création (simule un PID recyclé)
    let state = state_lot3("");
    lockv(&state.servers).push(Server {
        id: "stdio-recycle".into(),
        label: "Recyclé".into(),
        transport: Transport::Stdio {
            program: "node".into(),
            args: vec![],
            workdir: None,
            env: vec![],
        },
        origin: Origin::Manual,
        enabled: true,
        state: SState::Up,
        pid: Some(pid_innocent),
        bound_host: None,
        server_name: None,
        version: None,
        protocol: None,
        sse: false,
        sse_endpoint: false,
        ws: false,
        lang: None,
        framework: None,
        via_ssh: false,
        tools: vec![],
        latency_ms: None,
        last_seen_ts: None,
        last_error: None,
        start_cmd: None,
        auto_restart: false,
        favorite: false,
        tools_disabled: vec![],
    });
    lockv(&state.children).insert(
        "stdio-recycle".into(),
        probe::StdioHandle {
            pid: pid_innocent,
            started: Some(vraie_date.saturating_sub(1)),
            exit: Arc::new(Mutex::new(None)),
            stdin: None,
        },
    );
    let _ = handle_action(&state, r#"{"action":"kill","id":"stdio-recycle"}"#);
    assert!(
        innocent.try_wait().ok().flatten().is_none(),
        "un PID recyclé (date différente) ne doit pas être tué par kill"
    );

    // Même garde sur toggle OFF
    lockv(&state.children).insert(
        "stdio-recycle".into(),
        probe::StdioHandle {
            pid: pid_innocent,
            started: Some(vraie_date.saturating_sub(1)),
            exit: Arc::new(Mutex::new(None)),
            stdin: None,
        },
    );
    let _ = handle_action(&state, r#"{"action":"toggle","id":"stdio-recycle"}"#);
    assert!(
        innocent.try_wait().ok().flatten().is_none(),
        "un PID recyclé (date différente) ne doit pas être tué par toggle OFF"
    );

    // 2. Positif : quand le handle vient d'un vrai spawn_process (date identique),
    //    `kill` tue effectivement le process.
    let lance = probe::spawn_process(
        "node",
        &["-e".into(), "setInterval(()=>{},60000)".into()],
        None,
        &[],
    )
    .expect("spawn_process");
    let pid_lance = lance.pid;
    lockv(&state.children).insert("stdio-recycle".into(), lance);
    let rep = handle_action(&state, r#"{"action":"kill","id":"stdio-recycle"}"#);
    assert_eq!(rep.get("ok").and_then(|o| o.as_bool()), Some(true), "{rep}");
    let mut mort = false;
    for _ in 0..30 {
        if probe::process_creation(pid_lance).is_none() {
            mort = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(
        mort,
        "le vrai process lancé par l'app doit être tué par kill"
    );

    let _ = innocent.kill();
    let _ = innocent.wait();
}

// ==================== Lot A — moteur ====================

/// (lot A, point 3) : `set_start_command` avec `program` null (ou chaîne vide)
/// RETIRE la commande de lancement et désactive l'autoboot — le schéma
/// gateway est nullable depuis le lot 1, le dispatcher répondait
/// « program manquant » au lieu de retirer. Le retrait survit au reload.
#[test]
fn set_start_cmd_program_null_retire_commande_et_autoboot() {
    let path = tmp_config("cfg_setstartcmd_null.json", Some("{\"servers\":[]}"));
    let mut base = match Arc::try_unwrap(test_state()) {
        Ok(s) => s,
        Err(_) => panic!("test_state doit être unique"),
    };
    base.config_path = path.clone();
    let state = Arc::new(base);
    {
        let mut st = lockv(&state.settings);
        st.autostart = false;
    }
    {
        let mut servers = lockv(&state.servers);
        servers.push(Server {
            id: "scan-null".into(),
            label: "port-4243".into(),
            transport: Transport::Http {
                host: None,
                port: 4243,
                path: "/mcp".into(),
            },
            origin: Origin::Discovered,
            enabled: true,
            state: SState::Down,
            pid: None,
            bound_host: None,
            server_name: None,
            version: None,
            protocol: None,
            sse: false,
            sse_endpoint: false,
            ws: false,
            lang: None,
            framework: None,
            via_ssh: false,
            tools: vec![],
            latency_ms: None,
            last_seen_ts: None,
            last_error: None,
            start_cmd: None,
            auto_restart: true,
            favorite: false,
            tools_disabled: vec![],
        });
    }

    // pose une commande (avec env, pour vérifier que le retrait ne rapporte
    // rien d'exposé) puis retire-la par program:null
    let j = handle_action(
        &state,
        "{\"action\":\"set_start_cmd\",\"id\":\"scan-null\",\"program\":\"node\",\"args\":[\"srv.js\"]}",
    );
    assert_eq!(j["ok"], serde_json::json!(true));
    let j2 = handle_action(
        &state,
        "{\"action\":\"set_start_cmd\",\"id\":\"scan-null\",\"program\":null}",
    );
    assert_eq!(
        j2["ok"],
        serde_json::json!(true),
        "program:null doit retirer la commande : {j2}"
    );
    assert_eq!(j2["removed"], serde_json::json!(true));
    {
        let servers = lockv(&state.servers);
        let srv = servers.iter().find(|s| s.id == "scan-null").unwrap();
        assert!(srv.start_cmd.is_none(), "la commande doit être retirée");
        assert!(
            !srv.auto_restart,
            "autoboot désactivé : le serveur n'a plus rien à lancer"
        );
        assert_eq!(
            srv.origin,
            Origin::Manual,
            "l'origine n'est pas rétrogradée"
        );
    }
    // la chaîne vide vaut null
    let j3 = handle_action(
        &state,
        "{\"action\":\"set_start_cmd\",\"id\":\"scan-null\",\"program\":\"\"}",
    );
    assert_eq!(j3["ok"], serde_json::json!(true));

    // le retrait survit au reload (config réécrite sans commande)
    let (servers, _) = load_config(&path);
    let srv = servers
        .iter()
        .find(|s| s.id == "scan-null")
        .expect("persisté");
    assert!(srv.start_cmd.is_none(), "retrait non persisté");
    assert!(!srv.auto_restart, "autoboot désactivé non persisté");
}

/// (lot A, point 6) : `install_store` accepte un champ `env` optionnel
/// (des items du catalogue ont besoin d'un jeton). Mêmes garanties que
/// l'import (lot 3) : les valeurs atteignent le process lancé (transport ET
/// start_cmd), survivent au round-trip config — et ne sortent JAMAIS par
/// l'API (noms seuls).
#[test]
fn install_store_env_persiste_et_ne_fuit_pas_par_lapi() {
    let path = tmp_config("cfg_store_env.json", Some("{\"servers\":[]}"));
    let state = state_lot3(&path);
    let rep = handle_action(
        &state,
        r#"{"action":"install_store","id":"github","env":{"GITHUB_TOKEN":"valeur-secrete-lotA"}}"#,
    );
    assert_eq!(rep.get("ok").and_then(|o| o.as_bool()), Some(true), "{rep}");
    assert_eq!(rep.get("added").and_then(|a| a.as_u64()), Some(1), "{rep}");

    let srv = lockv(&state.servers)
        .iter()
        .find(|s| s.id == "store-github")
        .expect("serveur boutique installé")
        .clone();
    let attendu = (
        "GITHUB_TOKEN".to_string(),
        "valeur-secrete-lotA".to_string(),
    );
    match &srv.transport {
        Transport::Stdio { env, .. } => {
            assert!(env.contains(&attendu), "env perdu côté transport");
        }
        _ => panic!("transport stdio attendu pour un item boutique"),
    }
    let cmd = srv.start_cmd.as_ref().expect("start_cmd boutique");
    assert!(cmd.env.contains(&attendu), "env perdu côté start_cmd");

    // round-trip config.json : l'env survit à save/load
    save_config(&state);
    let (servers, _) = load_config(&path);
    let s2 = servers
        .iter()
        .find(|s| s.id == "store-github")
        .expect("persisté");
    match &s2.transport {
        Transport::Stdio { env, .. } => {
            assert!(env.contains(&attendu), "env perdu au round-trip config");
        }
        _ => panic!("transport stdio attendu"),
    }

    // API : noms SEULS — la valeur ne sort par aucune vue
    let etat = server_json(s2, false).to_string();
    assert!(
        !etat.contains("valeur-secrete-lotA"),
        "VALEUR exposée : {etat}"
    );
    assert!(etat.contains("GITHUB_TOKEN"), "nom d'env attendu : {etat}");
    let detail = detail_json(&state, "store-github").to_string();
    assert!(
        !detail.contains("valeur-secrete-lotA"),
        "VALEUR exposée par /api/detail : {detail}"
    );
    // le résultat du dispatcher ne renvoie pas la valeur non plus
    assert!(
        !rep.to_string().contains("valeur-secrete-lotA"),
        "VALEUR exposée par la réponse d'install : {rep}"
    );
}

/// (lot A, point 5) : une plage de scan inversée dans un config.json édité à
/// la main (scan_from > scan_to) était silencieusement ignorée (0 port) —
/// elle est remise dans l'ordre au chargement, avec une ligne au journal.
#[test]
fn config_plage_scan_inversee_remise_dans_l_ordre_avec_journal() {
    let cfg = r#"{"settings":{"scan_from":3200,"scan_to":3000}}"#;
    let path = tmp_config("cfg_scan_inverse.json", Some(cfg));
    let (_, st) = load_config(&path);
    assert_eq!(
        (st.scan_from, st.scan_to),
        (3000, 3200),
        "la plage doit être remise dans l'ordre au chargement"
    );
    let mcc = std::fs::read_to_string(std::path::Path::new(&path).with_file_name("mcc.log"))
        .unwrap_or_default();
    assert!(
        mcc.contains("plage de scan inversée (3200 > 3000)"),
        "ligne de journal attendue : {mcc}"
    );
}

/// (lot F, point 5) : `kill` répondait {ok:true} sur un identifiant inconnu
/// (arrêt d'un serveur inexistant = no-op silencieux) et `remove` {ok:false}
/// SANS champ `error` — dans les deux cas l'interface ne pouvait afficher
/// qu'un « échec » générique. Même forme de refus que les autres actions.
#[test]
fn kill_et_remove_id_inconnu_refusent_avec_motif() {
    let state = test_state();
    for (action, corps) in [
        ("kill", r#"{"action":"kill","id":"inconnu-lotF"}"#),
        (
            "kill_stdio",
            r#"{"action":"kill_stdio","id":"inconnu-lotF"}"#,
        ),
        ("remove", r#"{"action":"remove","id":"inconnu-lotF"}"#),
    ] {
        let j = handle_action(&state, corps);
        assert_eq!(j["ok"], serde_json::json!(false), "{action} : {j}");
        assert_eq!(
            j["error"],
            serde_json::json!("serveur inconnu"),
            "{action} doit renvoyer un motif exploitable : {j}"
        );
    }
    // aucun serveur n'a été touché : l'état est inchangé
    assert!(
        lockv(&state.servers).iter().all(|s| s.id != "inconnu-lotF"),
        "aucun serveur ne doit être créé par un refus"
    );
}

#[cfg(windows)]
#[test]
fn system_tool_curl_utilise_chemin_absolu_et_ignore_le_dossier_courant() {
    let p = probe::system_tool("curl");
    assert!(p.is_absolute(), "curl doit être un chemin absolu : {p:?}");
    assert!(
        p.exists(),
        "curl introuvable à l'emplacement système : {p:?}"
    );
    // Négatif : un faux curl.exe déposé dans le dossier de travail n'est pas
    // pris par system_command (seul System32\curl.exe est exécuté).
    let dir = std::env::temp_dir().join(format!("mcc-systool-curl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let leurre = dir.join("curl.exe");
    std::fs::write(&leurre, b"ceci n'est pas un executable valide").unwrap();
    assert_ne!(probe::system_tool("curl"), leurre);
    let out = probe::system_command("curl")
        .arg("--version")
        .current_dir(&dir)
        .output()
        .expect("curl.exe système");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "le vrai curl.exe doit répondre même avec un leurre dans cwd"
    );
}

#[test]
fn m4_json_expose_locked_et_etat_tls() {
    let state = test_state();
    {
        let mut servers = lockv(&state.servers);
        let mut s_locked = srv_http(
            "srv-lock",
            Some("litemcp-srv"),
            "LiteMCP",
            Some("127.0.0.1"),
            39991,
            Origin::Manual,
            Some(100),
        );
        s_locked.state = SState::PortOnly;
        s_locked.last_error = Some(crate::auth::net::LOCKED_SERVER_MSG.to_string());
        servers.push(s_locked);

        let s_open = srv_http(
            "srv-open",
            Some("open-srv"),
            "Open",
            Some("127.0.0.1"),
            39992,
            Origin::Manual,
            Some(100),
        );
        servers.push(s_open);
    }
    {
        let mut st = lockv(&state.settings);
        st.auth.tls_bind_refused = true;
        st.auth.tls_notice = Some("écoute hors loopback refusée sans tls".to_string());
    }
    let j = state_json(&state);
    assert_eq!(j["servers"][0]["locked"], serde_json::json!(true));
    assert_eq!(j["servers"][1]["locked"], serde_json::json!(false));
    assert_eq!(
        j["settings"]["tls"]["bind_refused"],
        serde_json::json!(true)
    );
    assert_eq!(
        j["settings"]["tls"]["notice"],
        serde_json::json!("écoute hors loopback refusée sans tls")
    );
}

#[test]
fn ui_api_state_json_expose_decouverte_annonces_sessions_et_revision() {
    let _env_garde = litemp_env_guard();
    let dir = dossier_annonces("ui-api-state");
    // 2 annonces non suivies + 1 annonce déjà suivie par label/port + 1 JSON cassé
    std::fs::write(
        dir.join("libre-1.json"),
        r#"{"name":"libre-1","port":18781,"version":"0.1.0"}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("libre-2.json"),
        r#"{"name":"libre-2","port":18782}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("deja-suivi.json"),
        r#"{"name":"Suivi","port":18783}"#,
    )
    .unwrap();
    std::fs::write(dir.join("invalide.json"), "{pas json").unwrap();

    let cfg_dir = std::env::temp_dir().join(format!("mcc-uiapi-state-cfg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cfg_dir);
    std::fs::create_dir_all(&cfg_dir).unwrap();
    let cfg_path = cfg_dir.join("config.json");

    let mut base = match Arc::try_unwrap(test_state()) {
        Ok(s) => s,
        Err(_) => panic!("test_state doit être unique"),
    };
    base.config_path = cfg_path.to_string_lossy().to_string();
    let state = Arc::new(base);
    {
        let mut st = lockv(&state.settings);
        st.discovery_ports_only = vec![8787, 3000];
        st.discovery_ports_excluded = vec![3131, 5090];
        st.auth.server = Some(crate::auth::ServerIdentity {
            keyid: crate::auth::SERVER_KEYID.into(),
            public_key: "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a".into(),
            created_at: crate::types::iso_utc(crate::types::now_ts()),
        });
    }
    lockv(&state.servers).push(srv_http(
        "srv-suivi",
        None,
        "Suivi",
        None,
        18783,
        Origin::Manual,
        Some(100),
    ));

    std::env::set_var("MCC_LITEMP_REGISTRY", &dir);
    let j1 = state_json(&state);
    let j2 = state_json(&state);
    std::env::remove_var("MCC_LITEMP_REGISTRY");

    assert_eq!(
        j1["settings"]["discovery_ports_only"],
        serde_json::json!([8787, 3000])
    );
    assert_eq!(
        j1["settings"]["discovery_ports_excluded"],
        serde_json::json!([3131, 5090])
    );
    assert_eq!(
        j1["announcements_count"],
        serde_json::json!(2),
        "seules les 2 annonces non suivies doivent compter : {j1}"
    );
    // Lecture seule : aucune adoption, aucun config.json créé
    assert_eq!(lockv(&state.servers).len(), 1);
    assert!(!cfg_path.exists());
    assert!(j1["sessions_active"].is_u64());

    let rev0 = j1["access"]["revision"]
        .as_u64()
        .expect("access.revision u64 attendu");
    assert_eq!(
        j2["access"]["revision"].as_u64(),
        Some(rev0),
        "revision doit rester identique sans changement"
    );

    // Changement 1 : demande d'appairage ajoutée → revision change
    let now = crate::types::now_ts();
    let req_pair = serde_json::json!({
        "keyid": "mcc-ui-0000000000000001",
        "label": "client-ui",
        "public_key": "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
        "created": now,
    })
    .to_string();
    let (status, _, body) = crate::auth::pair::handle_post(&state, req_pair.as_bytes(), None, now);
    assert_eq!(status, 200);
    let pair_id = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["pair_id"]
        .as_str()
        .unwrap()
        .to_string();

    std::env::set_var("MCC_LITEMP_REGISTRY", &dir);
    let rev_pair = state_json(&state)["access"]["revision"].as_u64().unwrap();
    std::env::remove_var("MCC_LITEMP_REGISTRY");
    assert_ne!(rev_pair, rev0);

    // Changement 2 : approbation (client inscrit + paire décidée) → revision change
    let ok = handle_action(
        &state,
        &serde_json::json!({
            "action": "pair_approve",
            "pair_id": pair_id,
            "rights": ["lecture"],
        })
        .to_string(),
    );
    assert_eq!(ok["ok"], serde_json::json!(true));
    std::env::set_var("MCC_LITEMP_REGISTRY", &dir);
    let rev_approved = state_json(&state)["access"]["revision"].as_u64().unwrap();
    std::env::remove_var("MCC_LITEMP_REGISTRY");
    assert_ne!(rev_approved, rev_pair);

    // Changement 3 : modification des droits puis révocation → revision change
    let ok = handle_action(
        &state,
        r#"{"action":"client_set_scopes","keyid":"mcc-ui-0000000000000001","rights":["lecture","pilotage"]}"#,
    );
    assert_eq!(ok["ok"], serde_json::json!(true));
    std::env::set_var("MCC_LITEMP_REGISTRY", &dir);
    let rev_scopes = state_json(&state)["access"]["revision"].as_u64().unwrap();
    std::env::remove_var("MCC_LITEMP_REGISTRY");
    assert_ne!(rev_scopes, rev_approved);

    let ok = handle_action(
        &state,
        r#"{"action":"client_revoke","keyid":"mcc-ui-0000000000000001"}"#,
    );
    assert_eq!(ok["ok"], serde_json::json!(true));
    std::env::set_var("MCC_LITEMP_REGISTRY", &dir);
    let rev_revoked = state_json(&state)["access"]["revision"].as_u64().unwrap();
    std::env::remove_var("MCC_LITEMP_REGISTRY");
    assert_ne!(rev_revoked, rev_scopes);

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&cfg_dir);
}

#[test]
fn ui_api_access_list_expose_identite_sessions_tls_url_et_stale_180d() {
    let dir = std::env::temp_dir().join(format!("mcc-uiapi-access-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Crée une vraie clé serveur en repli fichier (service = None → zéro trousseau OS)
    let loaded = crate::auth::secret::load_or_create_server(&dir, None).expect("clé serveur");
    let srv_pub_hex = crate::crypto::hex(&loaded.key.verifying_key().to_bytes());
    let srv_block = loaded.key.export();

    let mut base = match Arc::try_unwrap(test_state()) {
        Ok(s) => s,
        Err(_) => panic!("test_state doit être unique"),
    };
    base.config_path = dir.join("config.json").to_string_lossy().to_string();
    let state = Arc::new(base);

    let now = crate::types::now_ts();
    let cle_pub = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
    let mut frais = crate::auth::ClientRecord::new(
        "mcc-a-0000000000000001".into(),
        "frais".into(),
        cle_pub.into(),
        vec!["lecture".into()],
        "127.0.0.1".into(),
    );
    frais.created_at = crate::types::iso_utc(now - 3600);

    let mut ancien = crate::auth::ClientRecord::new(
        "mcc-a-0000000000000002".into(),
        "ancien".into(),
        cle_pub.into(),
        vec!["lecture".into()],
        "127.0.0.1".into(),
    );
    ancien.created_at = crate::types::iso_utc(now - crate::auth::registry::STALE_180D_SECS - 120);

    let mut revoque_ancien = crate::auth::ClientRecord::new(
        "mcc-a-0000000000000003".into(),
        "revoque".into(),
        cle_pub.into(),
        vec!["lecture".into()],
        "127.0.0.1".into(),
    );
    revoque_ancien.created_at =
        crate::types::iso_utc(now - crate::auth::registry::STALE_180D_SECS - 120);
    revoque_ancien.status = "revoked".into();

    let ident = crate::auth::ServerIdentity {
        keyid: crate::auth::SERVER_KEYID.into(),
        public_key: srv_pub_hex.clone(),
        created_at: crate::types::iso_utc(now),
    };
    let tls_fp = format!("sha256:{}", "ab".repeat(32));
    {
        let mut st = lockv(&state.settings);
        st.ui_bind = "0.0.0.0".into();
        st.ui_port = 5544;
        st.auth.server = Some(ident.clone());
        st.auth.tls_fp = tls_fp.clone();
        st.auth.tls_bind_refused = false;
        st.auth.tls_notice = None;
        st.auth.insert(frais).unwrap();
        st.auth.insert(ancien).unwrap();
        st.auth.insert(revoque_ancien).unwrap();
    }

    let j = handle_action(&state, r#"{"action":"access_list"}"#);
    assert_eq!(j["ok"], serde_json::json!(true), "{j}");

    // server_identity sans aucune clé privée
    assert_eq!(
        j["server_identity"]["keyid"],
        serde_json::json!("mcc-server")
    );
    assert_eq!(
        j["server_identity"]["fingerprint"],
        serde_json::json!(crate::auth::registry::fingerprint_of(&srv_pub_hex).unwrap())
    );
    assert_eq!(
        j["server_identity"]["fingerprint_court"],
        serde_json::json!(crate::auth::registry::short_fingerprint_of(&srv_pub_hex).unwrap())
    );
    assert_eq!(
        j["server_identity"]["storage"],
        serde_json::json!("file_fallback")
    );
    assert_eq!(
        server_storage_kind(&ident, &base_path_or(&dir), Some(&srv_block)),
        "keyring"
    );
    // Une autre clé au trousseau ne trompe pas la détection file_fallback
    let autre_cle = ::litemcp::auth::sign::SigningKey::generate().export();
    assert_eq!(
        server_storage_kind(&ident, &base_path_or(&dir), Some(&autre_cle)),
        "file_fallback"
    );

    // sessions { active, max: 8 }
    assert!(j["sessions"]["active"].is_u64());
    assert_eq!(j["sessions"]["max"], serde_json::json!(8));

    // tls + ui_url (0.0.0.0 + TLS actif -> https://127.0.0.1:5544)
    assert_eq!(
        j["tls"]["compiled"],
        serde_json::json!(crate::auth::net::tls_compiled())
    );
    assert_eq!(j["tls"]["active"], serde_json::json!(true));
    assert_eq!(j["tls"]["fingerprint"], serde_json::json!(tls_fp));
    assert_eq!(j["tls"]["bind_refused"], serde_json::json!(false));
    assert!(j["tls"]["notice"].is_null());
    assert_eq!(j["ui_url"], serde_json::json!("https://127.0.0.1:5544"));

    // Variantes ui_url (loopback, IP LAN, IPv6, bind refusé)
    assert_eq!(
        current_ui_url("127.0.0.1", 5090, false, false),
        "http://127.0.0.1:5090"
    );
    assert_eq!(
        current_ui_url("0.0.0.0", 5090, false, true),
        "http://127.0.0.1:5090"
    );
    assert_eq!(
        current_ui_url("192.168.1.25", 5090, true, false),
        "https://192.168.1.25:5090"
    );
    assert_eq!(
        current_ui_url("192.168.1.25", 5090, false, true),
        "http://127.0.0.1:5090"
    );
    assert_eq!(
        current_ui_url("::1", 5090, false, false),
        "http://[::1]:5090"
    );

    // clients[] : stale_180d
    let clients = j["clients"].as_array().unwrap();
    let find_c = |id: &str| clients.iter().find(|c| c["keyid"] == id).unwrap();
    assert_eq!(
        find_c("mcc-a-0000000000000001")["stale_180d"],
        serde_json::json!(false)
    );
    assert_eq!(
        find_c("mcc-a-0000000000000002")["stale_180d"],
        serde_json::json!(true)
    );
    assert_eq!(
        find_c("mcc-a-0000000000000003")["stale_180d"],
        serde_json::json!(false)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

fn base_path_or(dir: &std::path::Path) -> String {
    dir.join("config.json").to_string_lossy().to_string()
}

#[test]
fn ui_api_check_update_sha256_etats() {
    let asset = Some("mcp-control-cli-windows-x86_64.exe");
    let sum = "a".repeat(64);
    let found = check_update_sha256_json(true, Some("0.8.0"), asset, |tag, a| {
        assert_eq!(tag, "v0.8.0");
        assert_eq!(a, "mcp-control-cli-windows-x86_64.exe");
        Ok(Sha256Lookup::Found(sum.clone()))
    })
    .unwrap();
    assert_eq!(
        found["asset"],
        serde_json::json!("mcp-control-cli-windows-x86_64.exe")
    );
    assert_eq!(found["sha256"], serde_json::json!(sum));
    assert!(found["note"].is_null());

    let absent = check_update_sha256_json(true, Some("v0.8.0"), asset, |tag, _| {
        assert_eq!(tag, "v0.8.0");
        Ok(Sha256Lookup::Absent)
    })
    .unwrap();
    assert!(absent["sha256"].is_null());
    assert!(absent["note"].as_str().unwrap().contains("SHA256SUMS.txt"));

    let malformed = check_update_sha256_json(true, Some("0.8.0"), asset, |_, _| {
        Ok(Sha256Lookup::Malformed)
    })
    .unwrap();
    assert!(malformed["sha256"].is_null());
    assert!(malformed["note"].as_str().is_some());

    let err =
        check_update_sha256_json(true, Some("0.8.0"), asset, |_, _| Err("réseau coupé")).unwrap();
    assert!(err["sha256"].is_null());
    assert_eq!(err["note"], serde_json::json!("réseau coupé"));

    assert!(check_update_sha256_json(true, None, asset, |_, _| panic!("pas appelé")).is_none());
    assert!(
        check_update_sha256_json(false, Some("0.8.0"), asset, |_, _| panic!("pas appelé"))
            .is_none()
    );
}

#[test]
fn ui_api_aucun_secret_ni_fuite_gateway() {
    let dir = std::env::temp_dir().join(format!("mcc-uiapi-sec-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("auth").join("clients")).unwrap();

    let loaded = crate::auth::secret::load_or_create_server(&dir, None).expect("clé serveur");
    let srv_pub = crate::crypto::hex(&loaded.key.verifying_key().to_bytes());
    let srv_priv_block = loaded.key.export();
    let client_priv_key = ::litemcp::auth::sign::SigningKey::generate().export();
    let client_key_path = dir
        .join("auth")
        .join("clients")
        .join("mcc-out-0000000000000001.key");
    std::fs::write(&client_key_path, &client_priv_key).unwrap();

    let mut base = match Arc::try_unwrap(test_state()) {
        Ok(s) => s,
        Err(_) => panic!("test_state doit être unique"),
    };
    base.config_path = dir.join("config.json").to_string_lossy().to_string();
    let state = Arc::new(base);
    {
        let mut st = lockv(&state.settings);
        st.api_access = Some("secret-legacy-api-access-999".into());
        st.notify.headers =
            "Authorization: Bearer secret-webhook-bearer-888\nX-Hook-Secret: hook-777".into();
        st.auth.server = Some(crate::auth::ServerIdentity {
            keyid: crate::auth::SERVER_KEYID.into(),
            public_key: srv_pub,
            created_at: crate::types::iso_utc(crate::types::now_ts()),
        });
        st.auth
            .insert(crate::auth::ClientRecord::new(
                "mcc-c-0000000000000001".into(),
                "client".into(),
                "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a".into(),
                vec!["lecture".into()],
                "127.0.0.1".into(),
            ))
            .unwrap();
    }
    lockv(&state.servers).push(Server {
        id: "stdio-sec".into(),
        label: "Stdio Secret".into(),
        transport: Transport::Stdio {
            program: "node".into(),
            args: vec!["srv.js".into()],
            workdir: None,
            env: vec![("SECRET_TOKEN".into(), "secret-env-value-555".into())],
        },
        origin: Origin::Manual,
        enabled: true,
        state: SState::Down,
        pid: None,
        bound_host: None,
        server_name: None,
        version: None,
        protocol: None,
        sse: false,
        sse_endpoint: false,
        ws: false,
        lang: None,
        framework: None,
        via_ssh: false,
        tools: vec![],
        latency_ms: None,
        last_seen_ts: None,
        last_error: None,
        start_cmd: Some(StartCmd {
            program: "node".into(),
            args: vec!["srv.js".into()],
            workdir: None,
            env: vec![("SECRET_TOKEN".into(), "secret-env-value-555".into())],
        }),
        auto_restart: false,
        favorite: false,
        tools_disabled: vec![],
    });

    let st_str = state_json(&state).to_string();
    let acc_str = handle_action(&state, r#"{"action":"access_list"}"#).to_string();

    for (nom, txt) in [("state_json", &st_str), ("access_list", &acc_str)] {
        for interdit in [
            "PRIVATE KEY",
            "secret-legacy-api-access-999",
            "secret-webhook-bearer-888",
            "hook-777",
            "secret-env-value-555",
            "mcc_session",
            "X-MCC-Session",
            "x-mcc-session",
        ] {
            assert!(
                !txt.contains(interdit),
                "{nom} ne doit JAMAIS contenir « {interdit} » : {txt}"
            );
        }
        for line in srv_priv_block
            .lines()
            .chain(client_priv_key.lines())
            .filter(|l| l.len() > 24 && !l.starts_with("---"))
        {
            assert!(
                !txt.contains(line),
                "{nom} a divulgué une ligne de clé privée"
            );
        }
    }

    // Aucune de ces actions admin/sensibles ne doit figurer au catalogue /mcp
    for interdite in [
        "access_list",
        "server_pair",
        "client_rotate",
        "litemcp_announcements",
        "stop_server_force",
    ] {
        assert!(
            !crate::gateway::TOOL_RIGHTS
                .iter()
                .any(|(name, _)| *name == interdite),
            "{interdite} ne doit jamais être exposée sur /mcp"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn p1_save_cache_ne_reecrit_pas_si_contenu_utile_inchange() {
    use std::collections::{HashMap, HashSet};
    let dir = std::env::temp_dir().join(format!(
        "mcc-p1-cache-{}-{}",
        std::process::id(),
        crate::types::now_ts()
    ));
    let _ = std::fs::create_dir_all(&dir);
    let cfg = dir.join("config.json");
    let cache = dir.join("state-cache.json");

    let state = Arc::new(AppState {
        servers: Mutex::new(vec![Server {
            id: "s1".into(),
            label: "S1".into(),
            transport: Transport::Http {
                host: Some("127.0.0.1".into()),
                port: 19991,
                path: "/mcp".into(),
            },
            origin: Origin::Manual,
            enabled: true,
            state: SState::Up,
            pid: None,
            bound_host: Some("127.0.0.1".into()),
            server_name: Some("demo".into()),
            version: Some("1.0.0".into()),
            protocol: Some("2024-11-05".into()),
            sse: false,
            sse_endpoint: false,
            ws: false,
            lang: Some("node".into()),
            framework: None,
            via_ssh: false,
            tools: vec![crate::types::ToolInfo {
                name: "ping".into(),
                desc: "ok".into(),
            }],
            latency_ms: Some(4),
            last_seen_ts: Some(1000),
            last_error: None,
            start_cmd: None,
            auto_restart: false,
            favorite: false,
            tools_disabled: vec![],
        }]),
        log: Mutex::new(VecDeque::new()),
        settings: Mutex::new(Settings::default()),
        children: Mutex::new(HashMap::new()),
        config_path: cfg.to_string_lossy().to_string(),
        peers: Mutex::new(HashMap::new()),
        allowed_origins: Mutex::new(vec![]),
        allowed_hosts: Mutex::new(vec![]),
        spawn_fails: Mutex::new(HashMap::new()),
        runtime: Mutex::new(crate::runtime::RuntimeLog::new()),
        spawning: Mutex::new(HashSet::new()),
        update_latest: Mutex::new(None),
        lan_ok: Mutex::new(None),
    });

    // 1re écriture : le fichier est créé.
    save_cache(&state);
    assert!(
        cache.exists(),
        "state-cache.json doit être écrit la 1re fois"
    );
    let first_bytes = std::fs::read(&cache).expect("lecture cache 1");

    // Seuls les champs volatils changent (latency_ms, last_seen_ts) :
    // on remplace le fichier par un marqueur pour prouver qu'aucune écriture
    // disque n'a lieu lors du second save_cache.
    std::fs::write(&cache, b"SENTINEL_UNTOUCHED").expect("écriture sentinelle");
    {
        let mut srvs = lockv(&state.servers);
        srvs[0].latency_ms = Some(99);
        srvs[0].last_seen_ts = Some(2000);
    }
    save_cache(&state);
    let after_unchanged = std::fs::read(&cache).expect("lecture sentinelle");
    assert_eq!(
        after_unchanged, b"SENTINEL_UNTOUCHED",
        "save_cache ne doit PAS réécrire sur disque quand seul latency_ms/last_seen_ts/saved_at change"
    );

    // Changement réel (version) : le cache DOIT être réécrit.
    {
        let mut srvs = lockv(&state.servers);
        srvs[0].version = Some("2.0.0".into());
    }
    save_cache(&state);
    let after_changed = std::fs::read(&cache).expect("lecture cache 2");
    assert_ne!(after_changed, b"SENTINEL_UNTOUCHED");
    assert_ne!(after_changed, first_bytes);
    assert!(String::from_utf8_lossy(&after_changed).contains("2.0.0"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn p1_tools_list_mis_en_cache_sur_serveur_up_et_invalide_sur_changement_version() {
    use std::collections::{HashMap, HashSet};
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind fixture");
    let port = listener.local_addr().unwrap().port();
    let init_count = Arc::new(AtomicUsize::new(0));
    let tools_count = Arc::new(AtomicUsize::new(0));
    let version = Arc::new(Mutex::new("1.0.0".to_string()));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let ic = Arc::clone(&init_count);
    let tc = Arc::clone(&tools_count);
    let ver = Arc::clone(&version);
    let stp = Arc::clone(&stop);
    std::thread::spawn(move || {
        let _ = listener.set_nonblocking(false);
        for stream in listener.incoming() {
            if stp.load(Ordering::Relaxed) {
                break;
            }
            let Ok(mut s) = stream else { break };
            let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(2)));
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            for _ in 0..8 {
                match s.read(&mut tmp) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                }
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let req = String::from_utf8_lossy(&buf);
            if !req.contains("\r\n\r\n") {
                continue;
            }
            let body = if req.contains("\"tools/list\"") {
                tc.fetch_add(1, Ordering::SeqCst);
                r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo","description":"ok"}]}}"#
                    .to_string()
            } else if req.contains("\"initialize\"") {
                ic.fetch_add(1, Ordering::SeqCst);
                let v = ver.lock().unwrap().clone();
                format!(
                    r#"{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":"2024-11-05","serverInfo":{{"name":"p1-demo","version":"{v}"}}}}}}"#
                )
            } else {
                r#"{"ok":false}"#.to_string()
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = s.write_all(resp.as_bytes());
        }
    });

    let state = Arc::new(AppState {
        servers: Mutex::new(vec![Server {
            id: "p1-s".into(),
            label: "P1-S".into(),
            transport: Transport::Http {
                host: Some("127.0.0.1".into()),
                port,
                path: "/mcp".into(),
            },
            origin: Origin::Manual,
            enabled: true,
            state: SState::Down,
            pid: None,
            bound_host: None,
            server_name: None,
            version: None,
            protocol: None,
            sse: false,
            sse_endpoint: false,
            ws: false,
            lang: None,
            framework: None,
            via_ssh: false,
            tools: vec![],
            latency_ms: None,
            last_seen_ts: None,
            last_error: None,
            start_cmd: None,
            auto_restart: false,
            favorite: false,
            tools_disabled: vec![],
        }]),
        log: Mutex::new(VecDeque::new()),
        settings: Mutex::new(Settings::default()),
        children: Mutex::new(HashMap::new()),
        config_path: String::new(),
        peers: Mutex::new(HashMap::new()),
        allowed_origins: Mutex::new(vec![]),
        allowed_hosts: Mutex::new(vec![]),
        spawn_fails: Mutex::new(HashMap::new()),
        runtime: Mutex::new(crate::runtime::RuntimeLog::new()),
        spawning: Mutex::new(HashSet::new()),
        update_latest: Mutex::new(None),
        lan_ok: Mutex::new(None),
    });

    // Cycle 1 (DOWN → UP) : initialize + tools/list exécutés.
    poll_one_bg(&state, "p1-s");
    assert_eq!(init_count.load(Ordering::SeqCst), 1);
    assert_eq!(tools_count.load(Ordering::SeqCst), 1);
    assert_eq!(lockv(&state.servers)[0].tools.len(), 1);

    // Cycle 2 (UP → UP, même version) : initialize seul, tools/list SAUTÉ !
    poll_one_bg(&state, "p1-s");
    assert_eq!(init_count.load(Ordering::SeqCst), 2);
    assert_eq!(
        tools_count.load(Ordering::SeqCst),
        1,
        "tools/list ne doit pas être redemandé sur un serveur UP inchangé"
    );
    assert_eq!(lockv(&state.servers)[0].tools.len(), 1);

    // Cycle 3 : changement de version → tools/list rechargé !
    *version.lock().unwrap() = "2.0.0".to_string();
    poll_one_bg(&state, "p1-s");
    assert_eq!(init_count.load(Ordering::SeqCst), 3);
    assert_eq!(
        tools_count.load(Ordering::SeqCst),
        2,
        "tools/list doit être rechargé quand la version change"
    );
    assert_eq!(lockv(&state.servers)[0].version.as_deref(), Some("2.0.0"));

    stop.store(true, Ordering::Relaxed);
    let _ = std::net::TcpStream::connect(("127.0.0.1", port));
}

#[test]
fn p1_scan_network_guard_anti_reentrance() {
    let g1 = scans::NetworkScanGuard::try_acquire();
    assert!(g1.is_some(), "premier verrou scan réseau doit passer");
    assert!(
        scans::NetworkScanGuard::try_acquire().is_none(),
        "second verrou concurrent doit être refusé"
    );
    drop(g1);
    assert!(
        scans::NetworkScanGuard::try_acquire().is_some(),
        "après libération, un nouveau scan réseau est autorisé"
    );
}

#[test]
fn p1_refresh_lan_state_saute_sans_distant_ni_network_enabled() {
    use std::collections::{HashMap, HashSet};
    let settings = Settings {
        network_enabled: false,
        ..Settings::default()
    };
    let state = Arc::new(AppState {
        servers: Mutex::new(vec![Server {
            id: "local-1".into(),
            label: "Local".into(),
            transport: Transport::Http {
                host: Some("127.0.0.1".into()),
                port: 19992,
                path: "/mcp".into(),
            },
            origin: Origin::Manual,
            enabled: true,
            state: SState::Up,
            pid: None,
            bound_host: Some("127.0.0.1".into()),
            server_name: None,
            version: None,
            protocol: None,
            sse: false,
            sse_endpoint: false,
            ws: false,
            lang: None,
            framework: None,
            via_ssh: false,
            tools: vec![],
            latency_ms: None,
            last_seen_ts: None,
            last_error: None,
            start_cmd: None,
            auto_restart: false,
            favorite: false,
            tools_disabled: vec![],
        }]),
        log: Mutex::new(VecDeque::new()),
        settings: Mutex::new(settings),
        children: Mutex::new(HashMap::new()),
        config_path: String::new(),
        peers: Mutex::new(HashMap::new()),
        allowed_origins: Mutex::new(vec![]),
        allowed_hosts: Mutex::new(vec![]),
        spawn_fails: Mutex::new(HashMap::new()),
        runtime: Mutex::new(crate::runtime::RuntimeLog::new()),
        spawning: Mutex::new(HashSet::new()),
        update_latest: Mutex::new(None),
        lan_ok: Mutex::new(Some(true)),
    });
    let t0 = std::time::Instant::now();
    refresh_lan_state(&state);
    assert!(
        t0.elapsed() < std::time::Duration::from_millis(200),
        "sans serveur distant ni network_enabled, aucun ipconfig/ping ne doit être lancé"
    );
    assert_eq!(*lockv(&state.lan_ok), None);
}

#[test]
fn p1_servers_summary_json_filtre_et_stats_coherents() {
    use std::collections::{HashMap, HashSet};
    let state = Arc::new(AppState {
        servers: Mutex::new(vec![
            Server {
                id: "up-1".into(),
                label: "Up1".into(),
                transport: Transport::Http {
                    host: None,
                    port: 19993,
                    path: "/mcp".into(),
                },
                origin: Origin::Manual,
                enabled: true,
                state: SState::Up,
                pid: None,
                bound_host: None,
                server_name: Some("u".into()),
                version: Some("1".into()),
                protocol: None,
                sse: false,
                sse_endpoint: false,
                ws: false,
                lang: None,
                framework: None,
                via_ssh: false,
                tools: vec![crate::types::ToolInfo {
                    name: "t1".into(),
                    desc: "d".into(),
                }],
                latency_ms: Some(2),
                last_seen_ts: None,
                last_error: None,
                start_cmd: None,
                auto_restart: false,
                favorite: false,
                tools_disabled: vec![],
            },
            Server {
                id: "down-1".into(),
                label: "Down1".into(),
                transport: Transport::Http {
                    host: None,
                    port: 19994,
                    path: "/mcp".into(),
                },
                origin: Origin::Manual,
                enabled: true,
                state: SState::Down,
                pid: None,
                bound_host: None,
                server_name: None,
                version: None,
                protocol: None,
                sse: false,
                sse_endpoint: false,
                ws: false,
                lang: None,
                framework: None,
                via_ssh: false,
                tools: vec![],
                latency_ms: None,
                last_seen_ts: None,
                last_error: None,
                start_cmd: None,
                auto_restart: false,
                favorite: false,
                tools_disabled: vec![],
            },
        ]),
        log: Mutex::new(VecDeque::new()),
        settings: Mutex::new(Settings::default()),
        children: Mutex::new(HashMap::new()),
        config_path: String::new(),
        peers: Mutex::new(HashMap::new()),
        allowed_origins: Mutex::new(vec![]),
        allowed_hosts: Mutex::new(vec![]),
        spawn_fails: Mutex::new(HashMap::new()),
        runtime: Mutex::new(crate::runtime::RuntimeLog::new()),
        spawning: Mutex::new(HashSet::new()),
        update_latest: Mutex::new(None),
        lan_ok: Mutex::new(None),
    });

    let all = servers_summary_json(&state, None);
    assert_eq!(all["stats"]["total"], 2);
    assert_eq!(all["stats"]["up"], 1);
    assert_eq!(all["stats"]["tools"], 1);
    assert_eq!(all["servers"].as_array().unwrap().len(), 2);

    let only_up = servers_summary_json(&state, Some("UP"));
    assert_eq!(only_up["stats"]["total"], 2);
    assert_eq!(only_up["servers"].as_array().unwrap().len(), 1);
    assert_eq!(only_up["servers"][0]["id"], "up-1");
}

#[test]
fn auto_update_verify_sha256_accepte_valide_et_supprime_fichier_altere() {
    let dir = std::env::temp_dir().join(format!("mcc-upd-sha-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("fake-bin.exe");
    std::fs::write(&f, b"mcp-control-release-binary-v0.8.1").unwrap();
    let expected = crate::crypto::hex(&crate::crypto::sha256(b"mcp-control-release-binary-v0.8.1"));

    // Empreinte exacte → OK et le fichier reste présent
    let got = verify_file_sha256(&f, &expected).expect("SHA-256 exact doit réussir");
    assert_eq!(got, expected);
    assert!(f.exists());

    // Empreinte falsifiée → refus + suppression immédiate du fichier
    let faux = "0".repeat(64);
    let err = verify_file_sha256(&f, &faux).expect_err("SHA-256 falsifié doit échouer");
    assert!(err.contains("SHA-256 invalide"), "{err}");
    assert!(
        !f.exists(),
        "un binaire dont le SHA-256 ne correspond pas doit être supprimé immédiatement"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn auto_update_replace_binary_file_echange_atomiquement_avec_old() {
    let dir = std::env::temp_dir().join(format!("mcc-upd-swap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let current = dir.join("mcp-control.exe");
    let downloaded = dir.join("downloaded.exe");
    std::fs::write(&current, b"old-v0.8.0").unwrap();
    std::fs::write(&downloaded, b"new-v0.8.1").unwrap();

    replace_binary_file(&downloaded, &current).expect("remplacement binaire");
    assert_eq!(std::fs::read(&current).unwrap(), b"new-v0.8.1");
    let old = dir.join("mcp-control.exe.old");
    assert_eq!(std::fs::read(&old).unwrap(), b"old-v0.8.0");
    assert!(!downloaded.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn auto_update_url_whitelist_et_action_locale_seulement() {
    assert_eq!(
        release_download_url("v0.8.1", "mcp-control-desktop-windows-x86_64-setup.exe").as_deref(),
        Some("https://github.com/DeamonDev888/mcp-control/releases/download/v0.8.1/mcp-control-desktop-windows-x86_64-setup.exe")
    );
    assert!(release_download_url("../evil", "mcp-control-cli-linux-x86_64").is_none());
    assert!(release_download_url("v0.8.1", "evil.exe").is_none());
    assert!(ACTIONS.contains(&"install_update"));
    assert!(crate::auth::action_reservee_admin("install_update"));
}
