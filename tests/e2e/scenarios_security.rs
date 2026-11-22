//! Scénarios e2e « sécurité » : fuzz HTTP brut sur le VRAI moteur (sockets TCP
//! brutes via Engine::raw), et isolation d'un label hostile ajouté via l'API.
//!
//! Contrats vérifiés contre src/lib.rs au 2026-11-13 (lot 5) :
//! - MAX_BODY = 2 MiB → 413 ; MAX_CONNS = 64 → 503 EXPLICITE à saturation ;
//! - Host absent → 400 (le contrôle anti-rebinding n'est plus contournable) ;
//! - échéance totale par requête (5 s) + plafond de connexions par adresse +
//!   part réservée au loopback (une tempête LAN ne coupe jamais la fenêtre
//!   locale) ;
//! - Transfer-Encoding présent → 400 ; Content-Length dupliqué → 400 ;
//! - OPTIONS → 403 ; Host/Origin étrangers → 403 ; /api hors loopback → 401 ;
//! - JSON malformé / action inconnue → 200 avec {"ok":false,...} ;
//! - bloc d'en-têtes > 1 MiB → connexion fermée sans réponse (le moteur survit).
//! - (lot B) réponse set_notify masquée (headers_set) + motif précis du refus
//!   d'URL webhook (TLS pour https, « instance » pour la boucle sur soi).
//!
//! À exécuter SEUL (jamais en parallèle des autres scénarios) :
//! `cargo test --test e2e scenarios_security -- --test-threads=1`

use crate::harness::{free_port, spawn, spawn_loopback, wait_until, Engine, EngineCfg, Fixture};
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Taille max d'un corps acceptée par le moteur (src/lib.rs MAX_BODY).
const MAX_BODY: usize = 2 * 1024 * 1024;

/// Fuzz léger mais réel : JSON malformé, action inconnue, request smuggling,
/// corps surdimensionné, Host absent, bloc d'en-têtes géant, tempête de
/// connexions. Chaque étape vérifie que le moteur répond CE qui est attendu
/// aujourd'hui (et surtout qu'il SURVIT).
#[test]
fn fuzz_lite() {
    let _garde = crate::harness::serial_guard();
    let e = spawn_loopback();

    // ---- 1. corps JSON malformé → 200 + {"ok":false,"error":"body invalide: …"}
    let bad_json = e.request(
        "127.0.0.1",
        e.port,
        "POST",
        "/api/action",
        &[],
        Some(b"{not json"),
    );
    assert_eq!(
        bad_json.status, 200,
        "un JSON malformé ne doit pas casser la réponse HTTP"
    );
    let v = bad_json.json();
    assert_eq!(v["ok"], json!(false), "ok:false attendu");
    assert!(
        v["error"]
            .as_str()
            .unwrap_or_default()
            .starts_with("body invalide"),
        "erreur « body invalide » attendue, obtenu : {}",
        v["error"]
    );

    // ---- 2. action inconnue → 200 + {"ok":false,"error":"action inconnue: …"}
    let unknown = e.post_json("/api/action", &json!({"action": "danse-des-canards"}));
    assert_eq!(
        unknown.status, 200,
        "action inconnue : HTTP 200 (erreur métier)"
    );
    let v = unknown.json();
    assert_eq!(v["ok"], json!(false), "ok:false attendu");
    assert!(
        v["error"]
            .as_str()
            .unwrap_or_default()
            .contains("action inconnue"),
        "erreur « action inconnue » attendue, obtenu : {}",
        v["error"]
    );

    // ---- 3. Transfer-Encoding: chunked → 400 (anti request-smuggling)
    let te = e.raw(
        b"POST /api/action HTTP/1.1\r\nHost: 127.0.0.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
        0,
    );
    assert_eq!(te.status, 400, "Transfer-Encoding: chunked → 400");

    // ---- 4. Content-Length dupliqué → 400 (cadrage ambigu)
    let dup = e.raw(
        b"POST /api/action HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 2\r\ncontent-length: 2\r\n\r\n{}",
        0,
    );
    assert_eq!(dup.status, 400, "Content-Length dupliqué → 400");

    // ---- 5. corps > MAX_BODY (2 MiB) → 413 Payload Too Large
    let big = vec![b'x'; MAX_BODY + 16];
    let mut flood = format!(
        "POST /api/action HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        big.len()
    )
    .into_bytes();
    flood.extend_from_slice(&big);
    let too_big = e.raw(&flood, 0);
    assert_eq!(too_big.status, 413, "corps > 2 MiB → 413");
    assert_eq!(
        too_big.header("content-type").unwrap_or_default(),
        "application/json"
    );
    for (nom, attendu) in [
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("cross-origin-opener-policy", "same-origin"),
        ("cross-origin-resource-policy", "same-origin"),
    ] {
        assert_eq!(
            too_big.header(nom).unwrap_or_default(),
            attendu,
            "en-tête {nom} absent sur 413"
        );
    }
    assert!(
        too_big.text().contains("body trop grand"),
        "message « body trop grand » attendu, obtenu : {}",
        too_big.text().chars().take(200).collect::<String>()
    );

    // ---- 6. requête SANS en-tête Host → 400 (lot 5, audit 05 #8 et
    // gateway 02 #9) : le contrôle anti DNS-rebinding ne peut plus être sauté
    // en omettant simplement Host. La fenêtre desktop (http://127.0.0.1:<port>),
    // le harnais e2e et les sondes de pairs (probe.rs http_host_header)
    // envoient tous Host — aucun client légitime n'est affecté.
    let no_host = e.raw(b"GET /api/hello HTTP/1.1\r\n\r\n", 0);
    assert_eq!(
        no_host.status, 400,
        "sans Host : 400 attendu (anti-rebinding incontournable, lot 5)"
    );

    // ---- 7. bloc d'en-têtes surdimensionné (> cap 1 MiB de la lecture d'en-tête)
    // → le moteur ferme SANS répondre (head_end None → return). On accepte un
    // échec d'écriture côté client (RST pendant le flood) : l'essentiel est que
    // le moteur SURVIVE et reponde ensuite.
    let mut huge = String::from("GET /api/hello HTTP/1.1\r\nHost: 127.0.0.1\r\n");
    for i in 0..15_000u32 {
        huge.push_str(&format!("X-Flood-{}: {}\r\n", i, "A".repeat(64)));
    }
    huge.push_str("\r\n");
    let _ = e.raw(huge.as_bytes(), 0); // aucune réponse attendue (status 0)
    let alive = e.get("/api/hello");
    assert_eq!(
        alive.status, 200,
        "le moteur doit survivre à un bloc d'en-têtes surdimensionné"
    );

    // ---- 8. tempête de connexions : au-delà de MAX_CONNS (64), refus
    // EXPLICITE en 503 (lot 5, audit 05 #1) : le client voit POURQUOI, plus de
    // fermeture muette. On tient 90 sockets muettes ouvertes (deadline moteur
    // : 5 s par requête muette, donc on entretient la tempête tant que la
    // saturation n'est pas observée), puis on vérifie que le moteur récupère
    // une fois les sockets relâchées.
    let mut held: Vec<TcpStream> = Vec::new();
    for _ in 0..90 {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", e.port)) {
            held.push(s);
        }
    }
    thread_sleep(Duration::from_millis(300));
    let probe = b"GET /api/hello HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
    let saturated = wait_until(Duration::from_secs(6), || {
        let r = e.raw(probe, 0);
        if r.status == 503 {
            Some(true) // refus EXPLICITE (plafond de connexions atteint, lot 5)
        } else {
            // pas encore saturé : on entretient la tempête (les sockets
            // muettes meurent à l'échéance de requête de 5 s)
            for _ in 0..4 {
                if let Ok(s) = TcpStream::connect(("127.0.0.1", e.port)) {
                    held.push(s);
                }
            }
            None
        }
    });
    assert!(
        saturated,
        "saturation MAX_CONNS (64) non observée avec 90+ connexions tenues (503 attendu)"
    );
    drop(held);
    // ConnGuard relâchées → le moteur répond de nouveau (il a SURVÉCU)
    let recovered = wait_until(Duration::from_secs(6), || {
        if e.get("/api/hello").status == 200 {
            Some(true)
        } else {
            None
        }
    });
    assert!(recovered, "le moteur doit récupérer après la tempête");
    // et une action métier passe toujours après l'incident
    let after = e.post_json("/api/action", &json!({"action": "scan"}));
    assert_eq!(after.status, 200, "POST /api/action après récupération");
    assert_eq!(
        after.json()["ok"],
        json!(true),
        "action scan ok après l'incident"
    );
}

/// Petit sleep nommé pour la lisibilité (l'attente RÉELLE est wait_until).
fn thread_sleep(d: Duration) {
    std::thread::sleep(d);
}

/// Un label d'UI hostile (XSS) ajouté via l'API ne doit JAMAIS se retrouver
/// dans le HTML servi sur / : le label vit dans le JSON de /api/state (donnée
/// brute pour le front), la page reste le shell statique avec son CSP.
#[test]
fn hostile_label_never_in_html() {
    let _garde = crate::harness::serial_guard();
    // faux serveur MCP complet : initialize valide → le moteur doit le voir UP
    let fx = Fixture::spawn_mcp();
    let e = spawn_loopback();

    let label = "<img src=x onerror=alert(1)>";
    let add = e.post_json(
        "/api/action",
        &json!({
            "action": "add",
            "transport": "http",
            "port": fx.port(),
            "path": "/mcp",
            "label": label
        }),
    );
    let av = add.json();
    assert_eq!(
        av["ok"],
        json!(true),
        "l'ajout doit réussir, obtenu : {}",
        add.text().chars().take(200).collect::<String>()
    );
    let id = av["id"].as_str().expect("id du serveur ajouté").to_string();
    assert!(
        id.starts_with("manual-http-"),
        "id attendu manual-http-*, obtenu : {}",
        id
    );

    // le moteur sonde ::1 puis 127.0.0.1, chemin /mcp (puis / et /api/mcp) :
    // la fixture répond à TOUT → UP au prochain cycle (refresh_secs = 5).
    // Deadline large (45 s) : sous la suite complète, 45 scénarios lancent
    // leurs moteurs en parallèle et le premier cycle de sonde peut dépasser
    // largement refresh_secs — ce test a flotté à 20 s en exécution chargée.
    let up = wait_until(Duration::from_secs(45), || {
        let v = e.get("/api/state").json();
        let found = v["servers"]
            .as_array()?
            .iter()
            .find(|s| s["id"] == json!(id))?;
        if found["state"] == json!("UP") {
            Some(true)
        } else {
            None
        }
    });
    assert!(
        up,
        "le serveur fictif doit être UP (initialize + tools/list répondu)"
    );

    // /api/state : le label EST présent, en clair, comme DONNÉE JSON
    let state = e.get("/api/state");
    assert_eq!(state.status, 200);
    assert!(
        state.text().contains("onerror=alert(1)"),
        "le label hostile doit être conservé tel quel dans /api/state"
    );

    // la page HTML : NI le label, NI onerror, NI un quelconque échappement
    let root = e.get("/");
    assert_eq!(root.status, 200);
    assert!(
        root.header("content-type")
            .unwrap_or_default()
            .starts_with("text/html"),
        "la page / est bien du HTML"
    );
    let html = root.text();
    assert!(
        !html.contains("onerror"),
        "aucun onerror ne doit apparaître dans la page / (XSS neutralisée)"
    );
    assert!(
        !html.contains("<img src=x"),
        "le label hostile ne doit pas fuiter dans la page /"
    );
    assert!(
        root.header("content-security-policy").is_some(),
        "le CSP doit être présent sur la page /"
    );
}

// ---------- lot 5 : sécurité HTTP (audit 05 #1, #2, #4, #5, #7) ----------

/// Lot S2 : en-têtes de durcissement sur les réponses HTML ET JSON, CSP
/// stricte sur la coquille, page servie sans aucun gestionnaire en ligne.
/// C'est le contrat côté serveur ; le contrat côté DOM (aucun script exécuté)
/// est vérifié par tests navigateur headless (CDP).
#[test]
fn entetes_de_durcissement_et_csp_stricte() {
    let _garde = crate::harness::serial_guard();
    let e = spawn_loopback();

    let racine = e.get("/");
    assert_eq!(racine.status, 200);
    assert_eq!(
        racine.header("content-type").unwrap_or_default(),
        "text/html; charset=utf-8",
        "Content-Type exact attendu sur la coquille"
    );
    for (nom, attendu) in [
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("cross-origin-opener-policy", "same-origin"),
        ("cross-origin-resource-policy", "same-origin"),
    ] {
        assert_eq!(
            racine.header(nom).unwrap_or_default(),
            attendu,
            "en-tête {nom} absent ou faux sur /"
        );
    }
    let csp = racine.header("content-security-policy").unwrap_or_default();
    for attendu in ["script-src 'self'", "style-src 'self'"] {
        assert!(csp.contains(attendu), "CSP sans {attendu} : {csp}");
    }
    for interdit in ["unsafe-inline", "unsafe-eval", "nonce-"] {
        assert!(
            !csp.contains(interdit),
            "CSP permissive ({interdit}) : {csp}"
        );
    }
    let html = racine.text();
    for interdit in ["onclick=", "onchange=", "oninput=", "onsubmit=", "style=\""] {
        assert!(
            !html.contains(interdit),
            "la page servie contient encore {interdit}"
        );
    }

    // même durcissement sur les assets et les réponses JSON (200 et 404), Content-Type exact
    for (path, ctype_attendu) in [
        ("/app.css", "text/css; charset=utf-8"),
        ("/app-boot.js", "text/javascript; charset=utf-8"),
        ("/app.js", "text/javascript; charset=utf-8"),
        ("/api/state", "application/json"),
        ("/api/hello", "application/json"),
        ("/api/store", "application/json"),
        ("/api/detail?id=inexistant", "application/json"),
        ("/route-inconnue", "application/json"),
    ] {
        let rep = e.get(path);
        assert_eq!(
            rep.header("content-type").unwrap_or_default(),
            ctype_attendu,
            "Content-Type inattendu sur {path}"
        );
        for (nom, attendu) in [
            ("x-content-type-options", "nosniff"),
            ("referrer-policy", "no-referrer"),
            ("cross-origin-opener-policy", "same-origin"),
            ("cross-origin-resource-policy", "same-origin"),
        ] {
            assert_eq!(
                rep.header(nom).unwrap_or_default(),
                attendu,
                "en-tête {nom} absent ou faux sur {path}"
            );
        }
    }

    // S-5 (Tauri) : withGlobalTauri: false, CSP stricte, aucune origine distante
    // statique (ni localhost ni plage 5090..5099), confinement de navigation.
    let tauri_conf = include_str!("../../desktop/tauri.conf.json");
    assert!(
        tauri_conf.contains("\"withGlobalTauri\": false"),
        "withGlobalTauri doit être false"
    );
    for attendu in ["script-src 'self'", "style-src 'self'"] {
        assert!(
            tauri_conf.contains(attendu),
            "tauri.conf.json sans {attendu}"
        );
    }
    for interdit in ["unsafe-inline", "unsafe-eval", "nonce-"] {
        assert!(
            !tauri_conf.contains(interdit),
            "tauri.conf.json permissif ({interdit})"
        );
    }
    let cap = include_str!("../../desktop/capabilities/main-remote-ui.json");
    assert!(
        !cap.contains("http://localhost:") && !cap.contains("http://127.0.0.1:509"),
        "la capacité statique ne doit plus lister localhost ni la plage 5090..5099"
    );
    let desktop_main = include_str!("../../desktop/src/main.rs");
    for attendu in ["CapabilityBuilder", ".on_navigation", ".on_new_window"] {
        assert!(
            desktop_main.contains(attendu),
            "desktop/src/main.rs sans {attendu}"
        );
    }
}

/// Récepteur HTTP local minimal qui enregistre les corps reçus : cible des
/// webhooks de test (127.0.0.1, port éphémère — un webhook vers un service
/// local NON self est un usage normal, cf. audit 05 #4).
struct RecepteurWebhook {
    port: u16,
    stop: Arc<AtomicBool>,
    corps: Arc<Mutex<Vec<String>>>,
    th: Option<std::thread::JoinHandle<()>>,
}

impl RecepteurWebhook {
    fn demarrer() -> RecepteurWebhook {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind récepteur");
        let port = listener.local_addr().expect("local_addr récepteur").port();
        let stop = Arc::new(AtomicBool::new(false));
        let corps: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let stop2 = stop.clone();
        let corps2 = corps.clone();
        let th = std::thread::Builder::new()
            .name("mcc-recepteur-webhook".into())
            .spawn(move || {
                let _ = listener.set_nonblocking(true);
                loop {
                    if stop2.load(Ordering::SeqCst) {
                        break;
                    }
                    match listener.accept() {
                        Ok((mut s, _)) => {
                            let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
                            let mut buf = Vec::new();
                            let mut tmp = [0u8; 4096];
                            let head_end = loop {
                                match s.read(&mut tmp) {
                                    Ok(0) => break buf.len(),
                                    Ok(n) => {
                                        buf.extend_from_slice(&tmp[..n]);
                                        if let Some(p) = find_sub(&buf, b"\r\n\r\n") {
                                            break p;
                                        }
                                        if buf.len() > 65536 {
                                            break buf.len();
                                        }
                                    }
                                    Err(_) => break buf.len(),
                                }
                            };
                            let head =
                                String::from_utf8_lossy(&buf[..head_end]).to_ascii_lowercase();
                            let mut cl = 0usize;
                            for line in head.lines().skip(1) {
                                if let Some(v) = line.strip_prefix("content-length:") {
                                    cl = v.trim().parse().unwrap_or(0);
                                    break;
                                }
                            }
                            // connexion fermée sans \r\n\r\n complet (sonde de scan
                            // parallèle, client parti tôt) : corps vide — le
                            // récepteur ne doit PAS paniquer
                            let mut body = if buf.len() >= head_end + 4 {
                                buf[head_end + 4..].to_vec()
                            } else {
                                Vec::new()
                            };
                            while body.len() < cl {
                                match s.read(&mut tmp) {
                                    Ok(0) => break,
                                    Ok(n) => body.extend_from_slice(&tmp[..n]),
                                    Err(_) => break,
                                }
                            }
                            body.truncate(cl);
                            corps2
                                .lock()
                                .unwrap()
                                .push(String::from_utf8_lossy(&body).to_string());
                            let resp =
                                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
                            let _ = s.write_all(resp);
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(25)),
                    }
                }
            })
            .expect("spawn thread récepteur");
        RecepteurWebhook {
            port,
            stop,
            corps,
            th: Some(th),
        }
    }

    fn corps_recus(&self) -> Vec<String> {
        self.corps.lock().unwrap().clone()
    }
}

impl Drop for RecepteurWebhook {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.th.take() {
            let _ = h.join();
        }
    }
}

fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Contenu du mcc.log du moteur (journal des envois webhook inclus).
fn mcc_log_of(e: &Engine) -> String {
    std::fs::read_to_string(e.tmp_dir().join("mcc.log")).unwrap_or_default()
}

/// Lot 5, audit 05 #1 : une tempête de sockets muettes depuis une adresse NON
/// loopback (l'IP LAN locale de la machine — tout reste sur cette machine,
/// aucun trafic sortant) ne doit jamais couper la fenêtre locale : le plafond
/// par adresse limite l'assaillant, la part réservée au loopback garantit
/// /api/state et l'UI en 200 depuis 127.0.0.1, et le moteur survit.
#[test]
fn tempete_muettes_lan_ne_coupe_pas_la_fenetre_locale() {
    let _garde = crate::harness::serial_guard();
    let cfg = EngineCfg {
        ui_port: free_port(),
        ui_bind: "0.0.0.0",
        api_access: None,
    };
    let mut e = spawn(cfg).expect("spawn moteur LAN");
    e.wait_ready(Duration::from_secs(15))
        .expect("moteur LAN prêt");
    let Some(ip) = e.lan_ip() else {
        eprintln!("SKIP tempete_muettes_lan : aucune IP LAN non-loopback détectée");
        return;
    };
    eprintln!("diagnostic : IP LAN retenue = {ip}");

    // 64 sockets muettes depuis l'adresse LAN (aucun octet envoyé) : au-delà
    // du plafond PAR ADRESSE (16), le refus est explicite (503, écrit à l'accept).
    let mutees: Vec<TcpStream> = (0..64)
        .filter_map(|_| TcpStream::connect((ip.as_str(), e.port)).ok())
        .collect();
    assert!(
        mutees.len() >= 17,
        "au moins 17 connexions LAN doivent être établies côté TCP (le refus moteur se fait après accept)"
    );
    thread_sleep(Duration::from_millis(600)); // laisser l'accept loop écrire les 503

    // une NOUVELLE requête depuis la même adresse LAN → 503 explicite, message
    // « votre adresse » (plafond par adresse) — plus de silence muet.
    let lan_probe = e.request(&ip, e.port, "GET", "/api/state", &[], None);
    assert_eq!(
        lan_probe.status,
        503,
        "LAN au-delà du plafond par adresse → 503 (lot 5), obtenu : {}",
        lan_probe.text().chars().take(200).collect::<String>()
    );
    assert!(
        lan_probe.text().contains("votre adresse"),
        "message « votre adresse » attendu, obtenu : {}",
        lan_probe.text().chars().take(200).collect::<String>()
    );

    // LA FENÊTRE LOCALE RESTE JOIGNABLE : /api/state ET l'UI répondent 200
    // depuis le loopback (part réservée + loopback exempté des plafonds LAN).
    let st = e.get("/api/state");
    assert_eq!(
        st.status, 200,
        "/api/state loopback doit répondre 200 malgré la tempête LAN"
    );
    let ui = e.get("/");
    assert_eq!(
        ui.status, 200,
        "l'UI loopback doit répondre 200 malgré la tempête LAN"
    );

    // le moteur SURVIT : une fois les sockets relâchées, tout revient.
    drop(mutees);
    let recovered = wait_until(Duration::from_secs(8), || {
        if e.get("/api/hello").status == 200 {
            Some(true)
        } else {
            None
        }
    });
    assert!(recovered, "le moteur doit récupérer après la tempête LAN");
}

/// Lot 5, audit 05 #6 : /api/hello ne peut plus être noyé par une adresse —
/// au-delà de 30 requêtes / 60 s depuis la même adresse LAN, réponse 429
/// explicite ; le loopback reste exempté (fenêtre locale, harnais e2e).
#[test]
fn hello_rate_limite_par_adresse_hors_loopback() {
    let _garde = crate::harness::serial_guard();
    let cfg = EngineCfg {
        ui_port: free_port(),
        ui_bind: "0.0.0.0",
        api_access: None,
    };
    let mut e = spawn(cfg).expect("spawn moteur LAN");
    e.wait_ready(Duration::from_secs(15))
        .expect("moteur LAN prêt");
    let Some(ip) = e.lan_ip() else {
        eprintln!("SKIP hello_rate_limite : aucune IP LAN non-loopback détectée");
        return;
    };

    // /api/hello est pré-authentifié : sans code, depuis le LAN, les requêtes
    // passent le gating (nonce délivré) — mais sont comptées par adresse.
    for i in 0..30u32 {
        let r = e.request(&ip, e.port, "GET", "/api/hello", &[], None);
        assert_eq!(
            r.status,
            200,
            "hello #{} depuis le LAN doit passer, obtenu : {}",
            i + 1,
            r.text().chars().take(200).collect::<String>()
        );
        assert!(
            r.text().contains("nonce"),
            "nonce attendu dans le hello #{}",
            i + 1
        );
    }
    let r31 = e.request(&ip, e.port, "GET", "/api/hello", &[], None);
    assert_eq!(
        r31.status, 429,
        "la 31e requête /api/hello de la fenêtre → 429 (lot 5)"
    );
    assert!(
        r31.text().contains("taux"),
        "message de taux attendu, obtenu : {}",
        r31.text().chars().take(200).collect::<String>()
    );
    // le loopback n'est PAS limité : la fenêtre locale reste opérationnelle
    let local = e.get("/api/hello");
    assert_eq!(
        local.status, 200,
        "le loopback est exempté du limiteur de débit /api/hello"
    );
    assert!(local.text().contains("nonce"));
    // la limite persiste pour l'adresse LAN (fenêtre glissante de 60 s)
    let again = e.request(&ip, e.port, "GET", "/api/hello", &[], None);
    assert_eq!(
        again.status, 429,
        "l'adresse reste limitée pendant la fenêtre"
    );
}

/// Lot 5, audit 05 #4 : https:// refusé à l'enregistrement (aucune pile TLS —
/// l'envoi partait en HTTP brut sur le port 443), URL auto-référente refusée
/// (boucle sur cette instance), cible journalisée dans mcc.log à chaque envoi ;
/// bouton « Tester » (test_notify) : les deux canaux répondent ok en réel
/// (toast PowerShell réel — cf. audit 05 #7, le canal était mort). HTTP 404
/// impossible : le récepteur répond 200 à tout POST.
#[test]
fn webhook_https_et_auto_instance_refuses_journalise_et_test_notify() {
    let _garde = crate::harness::serial_guard();
    let e = spawn_loopback();
    let recu = RecepteurWebhook::demarrer();
    let url_receveur = format!("http://127.0.0.1:{}/hook", recu.port);

    // 1. https:// → refus à l'enregistrement avec le motif PRÉCIS (lot B
    // point 3 : le handler set_notify renvoie désormais
    // notify::webhook_url_reject_reason — plus le vieux message générique qui
    // citait encore https:// comme une forme valide).
    let r = e.post_json(
        "/api/action",
        &json!({"action": "set_notify", "webhook_url": "https://example.com/hook"}),
    );
    assert_eq!(r.json()["ok"], json!(false), "https:// doit être refusé");
    let erreur = r.json()["error"].as_str().unwrap_or_default().to_string();
    assert!(
        erreur.contains("TLS"),
        "motif TLS précis attendu, obtenu : {}",
        r.text().chars().take(200).collect::<String>()
    );
    assert!(
        !erreur.contains("ou https"),
        "l'ancien message générique ne doit plus sortir"
    );

    // 2. URL qui vise CETTE instance → refus (boucle sur soi)
    let r2 = e.post_json(
        "/api/action",
        &json!({
            "action": "set_notify",
            "webhook_url": format!("http://127.0.0.1:{}/api/state", e.port)
        }),
    );
    assert_eq!(
        r2.json()["ok"],
        json!(false),
        "auto-instance doit être refusée"
    );
    assert!(
        r2.json()["error"]
            .as_str()
            .unwrap_or_default()
            .contains("instance"),
        "motif précis « cette instance elle-même » attendu, obtenu : {}",
        r2.text().chars().take(200).collect::<String>()
    );

    // 3. enregistrement valide → ok ; « Tester » : desktop ok (PowerShell réel,
    // exit 0) ET webhook ok vers le récepteur local.
    let r3 = e.post_json(
        "/api/action",
        &json!({
            "action": "set_notify",
            "desktop": true,
            "webhook_url": url_receveur,
            "states": ["DOWN"],
            "min_interval_secs": 0
        }),
    );
    assert_eq!(
        r3.json()["ok"],
        json!(true),
        "enregistrement valide accepté : {}",
        r3.text().chars().take(200).collect::<String>()
    );

    let t = e.post_json("/api/action", &json!({"action": "test_notify"}));
    let tj = t.json();
    assert_eq!(
        tj["ok"],
        json!(true),
        "test_notify : {}",
        t.text().chars().take(300).collect::<String>()
    );
    // canal desktop : réel sous Windows ; refus propre hors Windows (pas de
    // pile de notification) — le webhook, lui, doit réussir partout
    #[cfg(windows)]
    assert_eq!(
        tj["desktop"]["ok"],
        json!(true),
        "toast desktop réel doit réussir (un seul -Command) : {}",
        t.text().chars().take(300).collect::<String>()
    );
    #[cfg(not(windows))]
    assert_eq!(
        tj["desktop"]["error"],
        json!("notifications desktop : non supporté hors Windows"),
        "desktop hors Windows : refus propre attendu : {}",
        t.text().chars().take(300).collect::<String>()
    );
    assert_eq!(
        tj["webhook"]["ok"],
        json!(true),
        "webhook vers le récepteur local doit réussir : {}",
        t.text().chars().take(300).collect::<String>()
    );

    // 4. le récepteur a BIEN reçu le POST de test (serveur « test », état TEST)
    let recu_ok = wait_until(Duration::from_secs(20), || {
        if recu
            .corps_recus()
            .iter()
            .any(|b| b.contains("\"server\":\"test\""))
        {
            Some(true)
        } else {
            None
        }
    });
    assert!(
        recu_ok,
        "le webhook de test doit atteindre le récepteur local"
    );

    // 5. chaque envoi est journalisé dans mcc.log (cible + résultat)
    let journal = mcc_log_of(&e);
    assert!(
        journal.contains(&format!("[webhook] envoi vers {url_receveur}")),
        "la cible du webhook doit être journalisée dans mcc.log"
    );
}

/// Lot 5, audit 05 #5 : /api/state n'expose plus les en-têtes du webhook (un
/// Authorization y vivait en clair, lisible sans code en loopback) — un
/// booléen `headers_set` suffit ; et un set_notify SANS clé `headers` ne doit
/// PAS effacer les en-têtes déjà configurés (contrat du volet Notifications :
/// champ vide = inchangé).
#[test]
fn etat_api_masque_les_entetes_et_vide_ne_efface_pas() {
    let _garde = crate::harness::serial_guard();
    let e = spawn_loopback();

    let set = e.post_json(
        "/api/action",
        &json!({
            "action": "set_notify",
            "webhook_url": "http://127.0.0.1:29999/hook",
            "headers": "Authorization: Bearer jeton-e2e-secret"
        }),
    );
    assert_eq!(
        set.json()["ok"],
        json!(true),
        "set_notify avec en-têtes : {}",
        set.text().chars().take(200).collect::<String>()
    );

    let st = e.get("/api/state");
    assert_eq!(st.status, 200);
    let corps = st.text();
    assert!(
        corps.contains("\"headers_set\":true"),
        "headers_set:true attendu dans /api/state"
    );
    assert!(
        !corps.contains("jeton-e2e-secret"),
        "aucune valeur d'en-tête ne doit sortir de /api/state"
    );
    assert!(
        !corps.contains("Authorization"),
        "aucun en-tête en clair dans /api/state"
    );

    // enregistrement SANS clé headers (le nouveau contrat UI : vide = inchangé)
    // → les en-têtes déjà configurés sont conservés (headers_set reste true)
    let re = e.post_json(
        "/api/action",
        &json!({"action": "set_notify", "desktop": false}),
    );
    assert_eq!(re.json()["ok"], json!(true));
    let st2 = e.get("/api/state");
    assert!(
        st2.text().contains("\"headers_set\":true"),
        "les en-têtes doivent survivre à un set_notify sans clé headers"
    );
    assert!(!st2.text().contains("jeton-e2e-secret"));
}

/// Lot 5, audit 05 #7 : le chemin RÉEL de notification (transition UP → DOWN
/// du poller) avec un label de serveur contenant ' et " — le toast part par
/// variables d'environnement (aucune concaténation, donc rien à injecter) et
/// le webhook transporte le label comme donnée JSON ; chaque envoi est
/// journalisé, et le canal desktop ne logge plus d'échec.
#[test]
fn notification_label_hostile_passe_par_les_deux_canaux() {
    let _garde = crate::harness::serial_guard();
    let e = spawn_loopback();
    let recu = RecepteurWebhook::demarrer();

    // canal desktop + webhook local, états DOWN, anti-spam désactivé
    let cfg = e.post_json(
        "/api/action",
        &json!({
            "action": "set_notify",
            "desktop": true,
            "webhook_url": format!("http://127.0.0.1:{}/hook", recu.port),
            "states": ["DOWN"],
            "min_interval_secs": 0
        }),
    );
    assert_eq!(cfg.json()["ok"], json!(true));

    let label = "l'insolent \"aux guillemets\" 'tous' $(calc)";
    let fx = Fixture::spawn_mcp();
    let add = e.post_json(
        "/api/action",
        &json!({
            "action": "add",
            "transport": "http",
            "port": fx.port(),
            "path": "/mcp",
            "label": label
        }),
    );
    assert_eq!(
        add.json()["ok"],
        json!(true),
        "ajout du serveur au label hostile : {}",
        add.text().chars().take(200).collect::<String>()
    );
    let id = add.json()["id"]
        .as_str()
        .expect("id du serveur ajouté")
        .to_string();

    // UP d'abord (le moteur sonde /mcp puis / …)
    let up = wait_until(Duration::from_secs(45), || {
        let v = e.get("/api/state").json();
        let s = v["servers"]
            .as_array()?
            .iter()
            .find(|s| s["id"] == json!(id))?;
        if s["state"] == json!("UP") {
            Some(true)
        } else {
            None
        }
    });
    assert!(up, "le serveur fictif doit être UP avant la coupure");

    // coupure → transition UP→DOWN → notification sur les deux canaux
    drop(fx);
    let down = wait_until(Duration::from_secs(45), || {
        let v = e.get("/api/state").json();
        let s = v["servers"]
            .as_array()?
            .iter()
            .find(|s| s["id"] == json!(id))?;
        if s["state"] == json!("DOWN") {
            Some(true)
        } else {
            None
        }
    });
    assert!(
        down,
        "le serveur doit passer DOWN après la coupure de la fixture"
    );

    // le webhook transporte le label HOSTILE comme donnée JSON (inerte) — le
    // toast desktop (réel, ~9 s dans PowerShell) part AVANT le webhook
    let recu_ok = wait_until(Duration::from_secs(30), || {
        if recu
            .corps_recus()
            .iter()
            .any(|b| b.contains("insolent") && b.contains("calc"))
        {
            Some(true)
        } else {
            None
        }
    });
    assert!(
        recu_ok,
        "le webhook DOWN doit atteindre le récepteur avec le label hostile"
    );

    // le toast desktop n'a PAS échoué (aucun warn [notif] desktop dans mcc.log)
    // et la cible + le label sont journalisés
    let journal = mcc_log_of(&e);
    let echecs_desktop = journal
        .lines()
        .filter(|l| l.contains("[notif] desktop :") && !l.contains("non supporté hors Windows"))
        .count();
    assert_eq!(
        echecs_desktop,
        0,
        "le toast desktop ne doit plus échouer : {}",
        journal
            .lines()
            .filter(|l| l.contains("[notif]"))
            .collect::<Vec<_>>()
            .join(" | ")
    );
    assert!(
        journal.contains(&format!(
            "[webhook] envoi vers http://127.0.0.1:{}/hook",
            recu.port
        )),
        "cible du webhook journalisée"
    );
    assert!(
        journal.contains("insolent"),
        "le label hostile figure dans la ligne de journal (serveur « … »)"
    );
}

/// Lot B, points 2-3 : la RÉPONSE de set_notify masque les en-têtes du
/// webhook (même surface que /api/state — un Authorization ne doit sortir ni
/// ici ni là), et le refus d'URL à l'enregistrement porte le motif PRÉCIS de
/// notify::webhook_url_reject_reason (TLS pour https://, « http:// attendu »
/// pour un autre schéma — plus le vieux « http:// ou https:// attendu »).
#[test]
fn set_notify_reponse_masquee_et_motif_precis() {
    let _garde = crate::harness::serial_guard();
    let e = spawn_loopback();

    // 1. enregistrement avec en-têtes → la réponse ne transporte JAMAIS la valeur
    let r = e.post_json(
        "/api/action",
        &json!({
            "action": "set_notify",
            "desktop": true,
            "webhook_url": "http://hooks.exemple.test/hook",
            "headers": "X-Jeton: valeur-e2e-5392"
        }),
    );
    assert_eq!(r.status, 200);
    let j = r.json();
    assert_eq!(
        j["ok"],
        json!(true),
        "enregistrement accepté : {}",
        r.text().chars().take(200).collect::<String>()
    );
    assert_eq!(
        j["notify"]["headers_set"],
        json!(true),
        "booléen headers_set attendu dans la réponse set_notify"
    );
    assert!(
        j["notify"].get("headers").is_none(),
        "les en-têtes ne doivent PAS sortir en clair dans la réponse set_notify"
    );
    assert!(
        !r.text().contains("valeur-e2e-5392"),
        "valeur d'en-tête qui fuite dans la réponse set_notify : {}",
        r.text().chars().take(300).collect::<String>()
    );

    // 2. URL https:// → motif précis TLS (plus le vieux message générique)
    let r2 = e.post_json(
        "/api/action",
        &json!({"action": "set_notify", "webhook_url": "https://hooks.exemple.test/hook"}),
    );
    let j2 = r2.json();
    assert_eq!(j2["ok"], json!(false));
    let err2 = j2["error"].as_str().unwrap_or_default();
    assert!(err2.contains("TLS"), "motif TLS attendu, obtenu : {err2}");
    assert!(
        !err2.contains("ou https"),
        "l'ancien message générique ne doit plus sortir"
    );

    // 3. autre schéma → motif précis aussi (http:// attendu)
    let r3 = e.post_json(
        "/api/action",
        &json!({"action": "set_notify", "webhook_url": "ftp://hooks.exemple.test/hook"}),
    );
    let j3 = r3.json();
    assert_eq!(j3["ok"], json!(false));
    let err3 = j3["error"].as_str().unwrap_or_default();
    assert!(err3.contains("http://"), "motif attendu, obtenu : {err3}");
    assert!(
        !err3.contains("ou https"),
        "l'ancien message générique ne doit plus sortir"
    );
}

/// R-3 (audit R) : en-têtes critiques dupliqués (`Host`, `Origin`, `Cookie`,
/// `X-MCC-Session`) et `Content-Length` non numérique → `400 Bad Request`.
#[test]
fn r3_entetes_dupliques_et_content_length_invalide_rejetes_400() {
    let _garde = crate::harness::serial_guard();
    let e = spawn_loopback();
    let port = e.port;

    let cases: [(&str, String); 6] = [
        (
            "Host dupliqué",
            format!(
                "GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nHost: evil.example:{port}\r\nConnection: close\r\n\r\n"
            ),
        ),
        (
            "Origin dupliqué",
            format!(
                "GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nOrigin: https://evil.example\r\nConnection: close\r\n\r\n"
            ),
        ),
        (
            "X-MCC-Session dupliqué",
            format!(
                "GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-MCC-Session: a\r\nX-MCC-Session: b\r\nConnection: close\r\n\r\n"
            ),
        ),
        (
            "Cookie mcc_session dupliqué (2 en-têtes)",
            format!(
                "GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: mcc_session=a\r\nCookie: mcc_session=b\r\nConnection: close\r\n\r\n"
            ),
        ),
        (
            "Cookie mcc_session dupliqué (1 en-tête)",
            format!(
                "GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: mcc_session=a; mcc_session=b\r\nConnection: close\r\n\r\n"
            ),
        ),
        (
            "Content-Length non numérique",
            format!(
                "POST /api/action HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: abc\r\nConnection: close\r\n\r\n{{}}"
            ),
        ),
    ];

    for (label, raw_req) in &cases {
        let resp = e.raw(raw_req.as_bytes(), 0);
        assert_eq!(
            resp.status, 400,
            "{label} doit rendre 400, vu {}",
            resp.status
        );
    }
}
