# Rapport de correction — Mini-lot sécurité `R-fix`

- **Branche** : `lot/R-fix` (base `dd550cd`, worktree `.lot-wt/R-fix`)
- **Référence** : `auth-agents/R-audit.md` (5 P2 `R-1`..`R-5` + 3 P3 code `R-6`..`R-8` ; `R-9` réservé au lot `D`)
- **Périmètre** : `src/lib.rs`, `src/gateway.rs`, `src/auth/secret.rs`, `src/auth/cli.rs`, `src/auth/pair.rs`, `src/auth/session.rs`, `src/auth/net.rs` (ligne 1097 uniquement, autorisé #64), `src/audit.rs`, `tests/e2e/scenarios_security.rs`, `tests/e2e/scenarios_auth.rs`, `auth-agents/R-fix.md`.

---

## 1. Tableau des correctifs (`R-1`..`R-8`)

| # | Gravité | Fichiers modifiés | Correctif appliqué | Tests ajoutés & preuves |
| :--- | :---: | :--- | :--- | :--- |
| **R-1** | **P2** | `src/lib.rs`<br>`tests/e2e/scenarios_auth.rs` | Ajout de `host_port_of` et `check_host_port` dans le sas `handle_conn_stream` : tout port explicite dans `Host` différent de `ui_port` est rejeté en `403 Forbidden` (port mal formé → `400`). Sur `/mcp`, un `Host` sans port est refusé (`403`) hors ports par défaut `80`/`443`. | Unit : `tests::check_host_port_bloque_rejeu_inter_ports_et_mcp_sans_port`<br>E2E : `scenarios_auth::r1_rejeu_inter_ports_inter_instances_refuse` (`200` sur instance A, `403` sur instance B). |
| **R-2** | **P2** | `src/gateway.rs`<br>`tests/e2e/scenarios_auth.rs` | Enregistrement de `BOOT_TS` (`OnceLock<u64>`) dans `gateway::install`, extraction de `created` dans `Signature-Input` via `signature_created`, et refus en `401` uniforme (`{"error":{"code":-32600,"message":"unauthorized"},"id":null,"jsonrpc":"2.0"}` + `WWW-Authenticate`) dans `respond_auth` dès que `created < boot_ts`. | Unit : `gateway::tests::signature_created_extrait_horodatage_ou_refuse_doublon`, `gateway::tests::requete_signee_avant_boot_ts_refusee_en_401_uniforme`<br>E2E : `scenarios_auth::r2_rejeu_nonce_apres_redemarrage_refuse_401` (`200` avant redémarrage, `401` après redémarrage, `200` sur requête fraîche). |
| **R-3** | **P2** | `src/lib.rs`<br>`src/auth/session.rs`<br>`tests/e2e/scenarios_security.rs` | Rejet en `400 Bad Request` dans `check_access` et `handle_conn_stream` si `count(host) != 1`, `count(origin) > 1`, `count(x-mcc-session) > 1`, `cookie_count(mcc_session) > 1`, ou `Content-Length` dupliqué / non numérique / non parsable en `usize` (suppression du `.unwrap_or(0)`). Défense en profondeur dans `session::cookie_value` et `session::header_value`. | Unit : `tests::check_access_rejette_entetes_dupliques_et_content_length_invalide`, `auth::session::tests::doublons_cookie_et_en_tete_session_refuses`<br>E2E : `scenarios_security::r3_entetes_dupliques_et_content_length_invalide_rejetes_400` (6 cas → `400`). |
| **R-4** | **P2** | `src/auth/secret.rs` | `write_secret_file` aligné sur une écriture sans TOCTOU : création de `.key.tmp` vide → `restrict_permissions(&tmp)` (nettoyage sur erreur) → écriture du secret + `sync_all` (nettoyage sur erreur) → `rename` atomique (nettoyage sur erreur). | Unit : `auth::secret::tests::write_secret_file_ne_laisse_aucun_tmp_et_restreint_les_permissions` (0 `.key.tmp` résiduel, ACL sans `(I)`). |
| **R-5** | **P2** | `src/auth/cli.rs`<br>`src/auth/session.rs`<br>`tests/e2e/scenarios_auth.rs` | `cli::action()` obtient désormais une session locale S1 via le canal fichier (`session::request_bootstrap`), l'échange sur `POST /api/session`, puis envoie `Cookie: mcc_session=...` et `X-MCC-Session: ...` sur `POST /api/action` sans jamais afficher de secret/cookie. `session::write_private` rendu atomique (`.<pid>.tmp` → ACL → écriture + `sync_all` → `rename`) pour fermer une course d'accès concurrente entre `request_bootstrap` et `serve_session_requests`. | Unit : `auth::cli::tests::extraction_cookie_session_dans_set_cookie`<br>E2E : `scenarios_auth::r5_cli_auth_list_et_auth_revoke_via_session_locale` (`auth list` exit `0` + client affiché, `auth revoke` exit `0`, requête suivante `401`, 0 fuite stdout/stderr). |
| **R-6** | **P3** | `src/auth/pair.rs`<br>`src/lib.rs` | Limiteur par IP dédié sur `GET /auth/pair/{pair_id}` (`PAIR_GET_RATE_MAX = 60` req / 60 s / IP, fenêtre glissante) évalué dans `handle_get` avant l'acquisition de `state.settings` (`429` au-delà). | Unit : `auth::pair::tests::get_pair_rate_limite_60_par_minute_renvoie_429` (60 × `200`, 61e → `429`, autre IP `200`, reprise après 61 s). |
| **R-7** | **P3** | `src/lib.rs`<br>`src/auth/pair.rs`<br>`src/auth/cli.rs`<br>`src/auth/net.rs` | Suppression complète du repli PRNG non cryptographique (`RandomState + now_ts()`) dans `random_hex` : signature `Option<String>` alimentée exclusivement par `os_random`. Échec fermé (`503` sur `/auth/pair` et `/api/hello`, erreur explicite dans `cli::cmd_pair` et `net::handle_server_pair`). | Unit : `tests::random_hex_echoue_ferme_si_entropie_os_indisponible`. |
| **R-8** | **P3** | `src/auth/session.rs`<br>`src/audit.rs` | `session::journal` clone le chemin hors du verrou `LOG_PATH` avant d'ouvrir le fichier ; `session::exchange` relâche `SLOT` avant `journal()` et `fresh_token()`. `audit::write_head` rendu atomique (`.head.tmp` vide → `restrict_permissions` → écriture + `sync_all` → `rename`, nettoyage sur erreur). | Unit : `audit::tests::write_head_est_atomique_sans_residu_tmp_et_lisible`, `audit::tests::*` + E2E `journal_audit_chaine_et_verify_log_trois_falsifications` verts. |

---

## 2. Vérifications exécutées

| Commande | Résultat |
| :--- | :--- |
| `cargo fmt --manifest-path .lot-wt/R-fix/Cargo.toml --check` | **OK** (exit `0`) |
| `cargo clippy --manifest-path .lot-wt/R-fix/Cargo.toml --all-targets --all-features -- -D warnings` | **OK** (exit `0`, 0 warning) |
| `cargo check --manifest-path .lot-wt/R-fix/Cargo.toml --no-default-features` | **OK** (exit `0`) |
| `cargo check --manifest-path .lot-wt/R-fix/Cargo.toml -p mcp-control-desktop` | **OK** (exit `0`) |
| `cargo test --manifest-path .lot-wt/R-fix/Cargo.toml --lib` | **OK** (`340 passed; 0 failed`) |
| `cargo test --manifest-path .lot-wt/R-fix/Cargo.toml --features tls --lib` | **OK** (`341 passed; 0 failed`) |
| `cargo test --manifest-path .lot-wt/R-fix/Cargo.toml --features tls --test e2e scenarios_security -- --test-threads=1` | **OK** (`10 passed; 0 failed`) |
| `cargo test --manifest-path .lot-wt/R-fix/Cargo.toml --features tls --test e2e scenarios_auth -- --test-threads=1` | **OK** (30/30 + `r5_cli_auth_list_et_auth_revoke_via_session_locale` vert après correctif atomique `write_private`, 31/31) |
| `reg query "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v "MCP Control"` | **Intact** (`"C:\Users\Deamon\Desktop\mcp-command-center\.lot-wt\S2\target\debug\mcp-control.exe" --hidden`) |
