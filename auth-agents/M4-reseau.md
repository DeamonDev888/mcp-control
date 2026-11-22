# Lot M4 — TLS 1.3 réseau, pairs migrés, sondes et proxy signés

Worktree `.lot-wt/M4`, branche `lot/M4` (base `auth` `4b55235`). Aucun merge, aucun push ; `../litemcp` intact.

## 1. Points

| # | Point | État | Fichiers touchés | Tests ajoutés (+ / −) | Preuve |
|---|---|---|---|---|---|
| 1 | Écoute TLS 1.3 hors loopback (`tls`), refus sans feature (`127.0.0.1` + log/UI), HTTP en loopback | FAIT | `Cargo.toml`, `Cargo.lock`, `src/auth/net.rs`, `src/auth/registry.rs`, `src/lib.rs`, `src/manage/json.rs`, `src/ui/app.js`, `tests/e2e/harness.rs` | + `certificat_tls_persiste_recharge_et_refuse_tls12_ou_cert_substitue`, `m4_json_expose_locked_et_etat_tls` ; E2E `m4_tls13_lan_capture_zero_clair_refus_tls12_et_cert_substitue`, `m4_sans_feature_tls_refuse_ecoute_hors_loopback_et_reste_en_local` | Capture relais LAN : `c2s=901 o, s2c=5179 o`, 0 motif HTTP/JSON-RPC/Signature en clair ; HTTP clair & TLS 1.2 (`0x15`) refusés ; sans `tls`, `ui_bind=0.0.0.0` reste en `127.0.0.1` + notice UI/log |
| 2 | `tls_fp` dans `/auth/pair` et code de comparaison ; `auth pair` épingle et refuse tout changement | FAIT | `src/auth/pair.rs`, `src/auth/cli.rs`, `src/auth/net.rs` | + `tls_fp_integree_dans_post_get_list_et_code_de_comparaison`, `changement_identite_ou_certificat_refuse_sans_contournement`, `url_http_seulement_et_port_optionnel` (IPv6/https) | `tls_fp` vérifiée pendant la poignée TLS (`PinnedServer`) ; repli HTTP ou cert/clé changés refusés explicitement |
| 3 | Pairs migrés (client `lecture` signé sur `POST /mcp`), repli transitoire DA3 0.8.x | FAIT | `src/probe.rs`, `src/manage/peers.rs`, `src/lib.rs` | + E2E `m4_pairs_migres_identite_signee_lecture_sans_x_mcc_proof` | `discover_peers` sonde `POST /mcp` (`list_servers`) signé ; `auth-audit.log` du pair trace `keyid=mcc-peer-… verdict=allowed` ; plus de génération `api_access` |
| 4 | Sondes 401 LiteMCP (« joignable mais verrouillé », badge cadenas) et proxy `call_server_tool` signé | FAIT | `src/probe.rs`, `src/manage/scans.rs`, `src/manage/json.rs`, `src/gateway.rs`, `src/ui/app.{js,css}` | + `reconnaissance_defi_www_authenticate_litemcp`, `validation_empreinte_tls_sha256` ; E2E `m4_serveur_litemcp_verrouille_sonde_server_pair_et_proxy_call_server_tool` | Avant appairage : `locked:true`, `PORT-ONLY`, `framework:"litemcp"`, `call_server_tool` refuse avec motif `server_pair` ; après : `UP`, `tools_count=2`, appel `echo_locked` signé + log C1 |
| 5 | Action locale `server_pair` (`admin`, exclue du gateway) + stockage clé cliente | FAIT | `src/auth.rs`, `src/auth/net.rs`, `src/lib.rs`, `src/auth/tests.rs`, `src/gateway.rs` | + `actions_auth_locales_repondent_et_refusent_le_reste`, `injections_interdites` (`server_pair`) ; E2E `server_pair` 401 sans session puis 200 local | Clé privée dans trousseau (`client-<host>:<port>`) ou repli `auth/clients/<keyid>.key` 0600/ACL + `<keyid>.pin.json` ; inatteignable via `/mcp` |

## 2. Vérifications (exécutées après la dernière retouche `src/auth/net.rs`)

- `cargo fmt --check` : exit 0 ; `cargo clippy --all-targets --all-features -- -D warnings` : exit 0 ; `cargo clippy --all-targets -- -D warnings` : exit 0
- `cargo check --no-default-features` : exit 0 ; `cargo check -p mcp-control-desktop` : exit 0
- `cargo test --lib` : **323 passed, 0 failed** (51,72 s) ; `--features tls --lib` : **324 passed, 0 failed** (51,68 s) ; `--no-default-features --lib` : **323 passed, 0 failed** (51,73 s)
- `cargo test --features tls --test e2e scenarios_auth -- --test-threads=1` : **27 passed, 0 failed** (451,32 s)
- `cargo test --features tls --test e2e scenarios_security -- --test-threads=1` : **9 passed, 0 failed** (65,57 s)
- `cargo test --test e2e scenarios_auth -- --test-threads=1` : **27 passed, 0 failed** (346,22 s)
- `cargo test --test e2e scenarios_security -- --test-threads=1` : **9 passed, 0 failed** (65,12 s)
- `cargo deny check` et `cargo deny --all-features check` : exit 0 (`advisories ok, bans ok, licenses ok, sources ok`)
- `node --check src/ui/app.js` : exit 0 ; `rg "\son[a-z]+=" src/ui.rs src/ui/app.js` : seule fixture test `ui.rs:1195` ; 0 attribut HTML `style=` dans `app.js`
- `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` `MCP Control` identique avant et après les E2E : `"C:\Users\Deamon\Desktop\mcp-command-center\.lot-wt\S2\target\debug\mcp-control.exe" --hidden`

## 3. Changements visibles par l'utilisateur (CHANGELOG)

- Écoute réseau hors loopback chiffrée en **TLS 1.3 uniquement** (`--features tls`) avec certificat auto-signé persisté et épinglage à l'appairage (`tls_fp` dans le code de comparaison). Sans `tls`, le bind LAN est refusé et le moteur reste en `127.0.0.1` avec alerte dans le journal et les Réglages.
- Pairs migrés vers l'identité Ed25519 signée (`lecture` sur `/mcp`, repli transitoire DA3 0.8.x conservé) ; plus de génération automatique d'`api_access`.
- Détection des serveurs LiteMCP verrouillés (`401 Signature`, badge cadenas), action locale `server_pair` et sondes/proxy `call_server_tool` signés de bout en bout.

## 4. Écarts par rapport à `docs/auth.md`

- `/api/hello` et ses nonces restent présents en 0.8.x (suppression prévue en 0.9.0, §10) : encore requis par le repli transitoire DA3 de `probe_peer`, `control_ui_at_hosts` et les suites E2E (`wait_ready`, `scenarios_boot`, `scenarios_security`).
- `server_pair` est routée dans `src/lib.rs` (et listée dans `ADMIN_ACTIONS` + `injections_interdites`) plutôt que dans `src/manage/actions.rs` afin de respecter le périmètre de fichiers du lot M4.
- `https_exchange_raw` pré-construit la session TLS avant `connect_timeout` et lit un éventuel refus HTTP `503` pré-TLS (`admit_conn`) pour éviter un `RST`/10053 sous Windows lors d'une tempête TCP.

## 5. Découvert en chemin et NON corrigé

- Flake préexistant S1 hors périmètre (`src/auth/session.rs` `write_private`/`icacls` en course avec `serve_session_requests` 250 ms sur `mcc-session.req`) vu 1 fois sur `plafond_de_huit_sessions_en_e2e` sans `tls` ; relancé 27/27 vert. Timeout transitoire vu 1 fois sur `notification_label_hostile_passe_par_les_deux_canaux` sans `tls` ; relancé 9/9 vert.
- Suites E2E hors `scenarios_auth` / `scenarios_security` (`boot`, `gateway`, `actions`, `lifecycle`, `scan`) et `cargo check` / tests sous Linux non exécutés dans ce lot.
