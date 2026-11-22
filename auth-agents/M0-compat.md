# M0 — Compatibilité `litemcp` 0.12.0

État : **FAIT** — le dépôt compile contre 0.12.0 et le gateway est épinglé au profil `Legacy` (2025-11-25), conformément à
la décision de l'orchestrateur (`TODO-AUTH.md` l. 114-118). `cargo fmt --check`, `cargo clippy --all-targets --
-D warnings` et **214/214** tests lib passent ; l'E2E donne **58/59**, le seul échec étant une assertion de test qui fige
encore `protocolVersion == "2026-07-28"` (§5) — non corrigeable sans toucher aux tests, ce que ce lot interdit.

Base : branche `auth`, commit `bd650dd`, Windows x86-64, rustc 1.99.1, 2026-11-16.

## 1. Points

| # | point | état | fichiers | tests | preuve |
|---|---|---|---|---|---|
| P1.1 | Dépendance par chemin, feature `auth-sign`, `default-features` intact | FAIT | `Cargo.toml` (l. 13-19) | — | `cargo tree -p mcp-control --depth 1` → `litemcp v0.12.0 (C:\Users\Deamon\Desktop\litemcp)` ; `cargo tree -p litemcp -e features` → `ed25519-dalek rand_core/zeroize` + `litemcp-macros default` |
| P1.2 | `Cargo.lock` régénéré, pas d'épingle manuelle | FAIT | `Cargo.lock` | — | +17 paquets (`litemcp 0.12.0`, `ed25519-dalek 3.0.0`, `curve25519-dalek 5.0.0`, `sha2 0.11.1`, `zeroize 1.9.1`…) |
| P2.1 | Littéral `McpHeaders` complété, comportement inchangé | FAIT | `src/gateway.rs` (l. 1325-1330) | — | 2 champs ajoutés : `protocol_version: None`, `params: Vec::new()` ; le gateway ne lisait déjà pas `mcp-protocol-version`, ces champs restent donc vides |
| P2.2 | Profil protocolaire du gateway épinglé à `Legacy` (2025-11-25) | FAIT | `src/gateway.rs` (l. 53, 215-219) | — | `.protocol_profile(ProtocolProfile::Legacy)` sur le `ServerBuilder` de `build_builder` — point **unique** de construction (appelé par `install` l. 99 et `refresh` l. 120) : aucun réglage ailleurs dans `src/` (recherche `protocol_profile\|ProtocolProfile\|.legacy(\|LEGACY_PROTOCOL_VERSION` → 2 occurrences, l'import et l'appel), et les snapshots reconstruits par `Gateway::edit` conservent le profil |
| P2.3 | Aucune modification hors périmètre | FAIT | — | — | `git status --short` → `M Cargo.lock`, `M Cargo.toml`, `M src/gateway.rs`, plus les non-suivis `TODO-AUTH*.md`, `auth-agents/`, `docs/auth.md` (intacts) ; journaux `.tmp-*`/`*.log` ignorés |
| P3.1 | `cargo fmt --check` | FAIT | — | exit 0 | 0 fichier reformaté, aucune sortie |
| P3.2 | `cargo clippy --all-targets -- -D warnings` | FAIT | — | exit 0 | 0 avertissement (2,06 s, incrémental) |
| P3.3 | `cargo test --lib` | **FAIT** | `src/gateway.rs` (module `tests`) | **214 / 214** | 0 échec, 0 ignoré, 51,16 s |
| P3.4 | `cargo test --test e2e --no-fail-fast` | **PARTIEL** | `tests/e2e/scenarios_gateway.rs` (l. 109-125) | **58 / 59** | 1 échec : assertion de version, motif et suite en §5 ; 618,89 s, exit 101 |
| P4.1 | Entrée Run préservée pendant l'E2E | FAIT | — | — | §4 : sauvegardée avant, absence constatée après l'E2E, restaurée, puis **2** `reg query` conformes |
| P5.1 | Rapport au format du projet | FAIT | `auth-agents/M0-compat.md` | — | ce fichier |

## 2. Sorties brutes (2026-11-16, Windows x86-64, rustc 1.99.1, branche `auth` sur `bd650dd`)

```
cargo fmt --check                         → exit 0, 0 fichier, aucune sortie
cargo clippy --all-targets -- -D warnings → exit 0, 0 warning (2,06 s)
cargo test --lib                          → 214 passed; 0 failed; 0 ignored (214 tests) en 51,16 s
cargo test --test e2e --no-fail-fast      → 58 passed; 1 failed; 0 ignored (59 tests) en 618,89 s, exit 101
```

## 3. Décision de profil : `Legacy` (2025-11-25)

`litemcp` 0.12 applique par défaut `ProtocolProfile::Strict2026_07_28` (`litemcp/src/server.rs:904-916`) : `_meta`
obligatoire dans `params`, `initialize` et `ping` supprimés (`-32601`), `MCP-Protocol-Version` exigé dès qu'un en-tête
est fourni, `id: null` rejeté. Écrite pour l'enveloppe 0.9, la surface du gateway de MCP Control n'envoyait rien de
tout cela : 11 tests lib et 9 scénarios E2E basculaient d'un coup.

Le profil retenu est **`Legacy`** — `initialize` et `ping` servis, `initialize` négocie la révision `2025-11-25`,
`_meta` optionnel, `id: null` toléré, en-têtes SEP-2243 optionnels. Trois raisons :

1. **Compatibilité client.** Claude Desktop, Cursor et Claude Code font `initialize` et n'envoient pas `_meta` : le
   profil strict les refuserait tous les trois. C'est une régression visible pour l'utilisateur, que M0 (lot de
   compatibilité de dépendance) n'a pas mandat d'introduire.
2. **Un seul point de réglage.** `build_builder` est le constructeur unique du `ServerBuilder` du gateway ; `install`
   (l. 99) et `refresh` (l. 120) passent par lui, et `Gateway::edit` clone le builder stocké. Le profil est donc posé
   une fois et survit à toutes les reconstructions de snapshot ; aucun `protocol_profile`/`legacy()` n'existait
   ailleurs dans `src/` avant ce lot.
3. **Le contrôle d'en-tête qui bloquait disparaît.** Dans la lib, l'exigence est gardée par
   `is_modern = strict_profile && headers.is_some()` (`litemcp/src/server.rs:1782`) : en `Legacy`, l'absence de
   `MCP-Protocol-Version` n'est plus une faute — c'est le point qui faisait rendre `-32020` à
   `respond_verification_en_tetes_sep_2243`. Le gateway continue de transmettre `McpHeaders` dès qu'un `Mcp-Method`
   ou `Mcp-Name` est présent (l. 1325-1330), donc un en-tête de version **fourni mais incohérent** reste refusé.
   Vérifié isolément : `cargo test --lib -- gateway::tests::respond_verification_en_tetes_sep_2243 --exact` → 1 passed.

**Ce qu'il faudra trancher au lot M2** (`docs/auth.md` à mettre à jour) :

- **Rester en `Legacy`** : simple, mais le gateway ne peut alors pas exiger l'enveloppe signée 2026-07-28 que le
  chantier « authentification forte » vise à terme.
- **Basculer en strict** : suppose que tous les clients visés envoient `_meta` + `MCP-Protocol-Version` ; casse
  aujourd'hui Claude Desktop, Cursor et Claude Code, et exige de réécrire les fixtures lib et E2E.
- **Négocier les deux** sur le même `POST /mcp` (servir `initialize`, puis router selon `MCP-Protocol-Version`/`_meta`
  par requête) : le plus cohérent avec un gateway authentifié de bout en bout, mais c'est un travail de lib ou de
  couche de transport, à planifier comme lot dédié.
- Dans tous les cas, l'assertion E2E du §5 et la doc de module de `src/gateway.rs` (§7) suivent la décision.

## 4. Entrée Run (P4.1)

`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, valeur `MCP Control`, attendue :
`"C:\Users\Deamon\AppData\Local\MCP Control\mcp-control-desktop.exe" --hidden`.

| Moment | Commande | Résultat |
|---|---|---|
| avant l'E2E | `reg query "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v "MCP Control"` | `MCP Control    REG_SZ    "C:\Users\Deamon\AppData\Local\MCP Control\mcp-control-desktop.exe" --hidden` (copie de sauvegarde dans `%TEMP%\mcc-run-backup.txt`) |
| après l'E2E | idem | **exit 1**, « le système n'a pas trouvé la clé ou la valeur de Registre spécifiée » — valeur supprimée par `scenarios_actions`, régression connue |
| restauration | `New-ItemProperty -Path 'HKCU:\…\Run' -Name 'MCP Control' -Value <sauvegarde> -PropertyType String -Force` | valeur reposée à l'identique depuis la sauvegarde |
| preuve 1 | `reg query … /v "MCP Control"` | `REG_SZ    "C:\Users\Deamon\AppData\Local\MCP Control\mcp-control-desktop.exe" --hidden`, exit 0 |
| preuve 2 | `reg query … /v "MCP Control"` | identique, exit 0 |

## 5. Écart restant : 1 scénario E2E

`scenarios_gateway::gateway_initialize_tools_list_et_discover` échoue sur une **assertion de version**, pas sur un
défaut du gateway :

```
thread 'scenarios_gateway::gateway_initialize_tools_list_et_discover' (31500) panicked at tests\e2e\scenarios_gateway.rs:122:5:
assertion `left == right` failed: le gateway annonce la spec LiteMCP : {"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"mcp-control","version":"0.7.21"}},"capabilities":{"prompts":{},"resources":{},"tools":{}},"protocolVersion":"2025-11-25","resultType":"complete","serverInfo":{"name":"mcp-control","version":"0.7.21"}}
  left: String("2025-11-25")
 right: "2026-07-28"
```

Le scénario (l. 109-125) envoie un `initialize` en `protocolVersion: "2024-11-05"` et exige que la réponse annonce
`2026-07-28` — l'attente du profil strict. En `Legacy`, la lib négocie `2025-11-25`, ce qui est exactement le
comportement décidé. Le test n'a **pas** été affaibli ni modifié (interdiction de toucher aux tests) ; l'assertion
panique avant les vérifications suivantes du scénario (`tools/list`, `server/discover`), qui n'ont donc pas été
exécutées dans ce scénario. La même surface passe au niveau lib (`respond_dispatch_tools_list_avec_les_meta_litemcp`,
`snapshot_contient_toute_la_surface_supervision`, `tool_count_compte_le_catalogue_publie` — inclus dans les 214/214).

Correction suggérée, **à décider par le parent** : attendre `2025-11-25` (ou la constante de la lib) dans cette
assertion — une ligne de test, hors du périmètre autorisé ici.

## 6. Ce que ce changement casse hors développement

- **CI GitHub** : `path = "../litemcp"` n'existe pas dans un clone ; toute compilation CI échoue avant même les tests.
  À traiter au **lot D**, quand 0.12.0 sera publiée : revenir à `litemcp = "0.12"`.
- `cargo package` / publication crates.io impossibles (dépendance de chemin) ; toute machine sans
  `C:\Users\Deamon\Desktop\litemcp` ne compile plus.
- Le graphe gagne 17 paquets (`ed25519-dalek`, `curve25519-dalek`, `sha2`, `zeroize`…) : empreinte binaire et temps de
  build augmentent, même si seul un sous-ensemble d'`auth-sign` est utilisé pour l'instant.

## 7. Découvert hors périmètre, non corrigé

- La doc de module de `src/gateway.rs` (l. 31-37) annonce encore « spec 2026-07-28 stateless » et le « module
  `gateway` de LiteMCP 0.9 » : périmée sur les deux points (0.12.0, profil `Legacy`). Non touchée ici — la
  modification autorisée portait sur la construction du `ServerBuilder` ; à rafraîchir quand M2 tranchera le profil.
- Aucun autre écart : les 8 autres scénarios E2E et les 11 tests lib en échec au tour précédent passent désormais ;
  seul `gateway_initialize_tools_list_et_discover` reste rouge, pour le motif du §5.
