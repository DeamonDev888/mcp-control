# Lot M5 — pont stdio, journal d'audit chaîné, anti-force brute, uniformité des refus

Worktree `.lot-wt/M5`, branche `lot/M5` (base `auth` 3648eb6). Aucun merge, aucun push.

## 1. Points

| # | Point | État | Fichiers | Preuves (positif / négatif) |
|---|-------|------|----------|-----------------------------|
| 1 | Pont `mcp-control bridge [--url]` | FAIT | `src/bridge.rs`, `src/main.rs` | E2E `pont_stdio_pilote_le_gateway_sans_cle_et_stdout_jsonrpc_seul` ; tests unitaires (un seul `Host`, pin le plus récent) |
| 2 | Journal `auth-audit.log` chaîné + `auth verify-log` | FAIT | `src/audit.rs`, `src/auth/{cli,secret}.rs` | E2E `journal_audit_chaine_et_verify_log_trois_falsifications` (modifiée / supprimée / insérée) ; tests `audit::tests::*` (rotation, ligne 1 supprimée, `.head` falsifié) |
| 3 | Anti-force brute (adresse ET keyid) | FAIT | `src/auth/throttle.rs`, `src/lib.rs` | E2E `anti_force_brute_lan_et_tempete_60s_fenetre_locale_fluide` |
| 4 | Uniformité des refus | FAIT | `src/auth/registry.rs`, `src/lib.rs` | E2E `uniformite_refus_corps_statut_et_temps_1000_requetes` |

## 2. Vérifications (après la dernière modification de code)

- `cargo fmt --check` : exit 0
- `cargo clippy --all-targets --all-features -- -D warnings` : exit 0
- `cargo check --no-default-features` : exit 0
- `cargo check -p mcp-control-desktop` : exit 0
- `cargo test --lib` : 305 passed, 0 failed
- `cargo test --no-default-features --lib` : 305 passed, 0 failed
- `cargo test --test e2e scenarios_auth` : 24 passed, 0 failed (452,78 s)
- `HKCU\...\Run` `MCP Control` identique avant et après les E2E :
  `"C:\Users\Deamon\Desktop\mcp-command-center\.lot-wt\S2\target\debug\mcp-control.exe" --hidden`
- `contrat_dispatcher_toutes_actions` non lancé.

## 3. Mesures

Profil `dev` (non optimisé). Point 3, 60 s de requêtes invalides depuis l'adresse LAN :
- LAN : 41 × 401, puis 6219 × 429 (p50 1165 µs, p95 2952 µs), 0 autre statut.
- Fenêtre locale (loopback, sondée en continu) : 577 × 200, 0 échec, p50 2757 µs, p95 7629 µs, max 55770 µs.
- Cadence volontairement limitée (~15 ms, 2 threads) pour ne pas épuiser les ports éphémères Windows.

Point 4, 1000 requêtes par sorte (même statut et même corps vérifiés par le test) :

| Sorte | min µs | p50 µs | moyenne µs | p95 µs | écart-type µs |
|-------|--------|--------|------------|--------|---------------|
| clé inconnue | 8906 | 9560 | 10420,5 | 14099 | 2731,7 |
| signature fausse | 8937 | 9666 | 10458,8 | 14461 | 2227,9 |
| clé révoquée | 8936 | 9605 | 10441,6 | 14076 | 2971,2 |

## 4. Configs clients (après `mcp-control auth pair http://127.0.0.1:5090`)

Claude Desktop (`claude_desktop_config.json`), Claude Code (`.mcp.json`), Cursor (`.cursor/mcp.json`) :
```json
{ "mcpServers": { "mcp-control": { "command": "mcp-control", "args": ["bridge"] } } }
```
Avec URL explicite : `"args": ["bridge", "--url", "http://127.0.0.1:5090"]`. Ces blocs suivent le schéma
`mcpServers` standard ; je ne les ai pas testés dans les clients eux-mêmes.

## 5. Écarts et points ouverts

- Rotation : ancre `auth-audit.head` signée HMAC, écrite quand le segment `.5` est évincé (ajout à `docs/auth.md` §11).
- `signature_invalid` mesuré avec une vraie clé de signature différente (une signature mal formée est rejetée avant Ed25519).
- `src/lib.rs` : le compteur d'échecs n'est remis à zéro que sur 200/202/403 avec signature présente (les 400/405/406/415 ne l'effacent plus).
- Un flake isolé de `probe::tests::sse_get_detecte_un_flux_event_stream_et_rejette_l_encodage_sse_du_streamable` (`src/probe.rs`, hors lot) vu une fois ; relancé, 305/305.
- CHANGELOG : à rédiger par le coordinateur (hors périmètre M5).
