# M3 — Confirmation humaine avant exécution, et écran « Accès »

État : **FAIT**, 5 points sur 5. Base : branche `auth`, `e6bfb19` + reprise du travail non commité de
l'agent précédent (relu, corrigé, complété). Windows x86-64, 2026-11-16.

## 1. Points

| # | point | état | fichiers | tests (+ / −) | preuve |
|---|---|---|---|---|---|
| 1 | tout appel `execution` crée une demande (client, libellé, empreinte courte, adresse, outil, effet exact — programme, arguments, dossier ; env = **noms**) ; le client reçoit `403` `-32023` `approval_pending` + `retryable` + `retryAfterMs` et réessaie | FAIT | `src/auth/confirm.rs` (n.), `src/gateway.rs` (`with_approval`, `serve(peer)`), `src/lib.rs`, `src/manage/json.rs` | + E2E `confirmation_rien_ne_part_sans_accord_puis_part` ; + `demande_creee_une_seule_fois_par_client_outil_et_arguments` ; − `permis_lie_aux_arguments_…`, `effet_ne_divulgue_jamais_une_valeur_d_environnement`, `arguments_de_proxy_masques`, `import_ne_recopie_jamais_le_texte_brut` | `403` → `{"code":-32023,"data":{"reason":"approval_pending","retryAfterMs":2000,"retryable":true}}` ; demande lue par l'écran : `effect.program="node"`, `effect.args[2]=<marqueur>`, `address="127.0.0.1"`, `fingerprint="ed25519:…"` |
| 2 | approbation par la session locale **seule**, 10 min pour ce client **et** cet outil, révocable, perdue au redémarrage ; 32 demandes bornées, expiration 2 min ; ni auto-approbation ni visibilité croisée | FAIT | `confirm.rs`, `src/manage/actions.rs` (`access_list`, `confirm_approve/deny`, `grant_revoke`), `src/auth.rs` (`ADMIN_ACTIONS` 7→11) | + E2E `permis_expire_et_redemande`, `approbation_par_un_autre_client_refusee` ; − `un_client_ne_peut_pas_approuver_sa_propre_demande`, `un_client_ne_voit_que_ses_demandes`, `demandes_bornees_a_32_…`, `demande_expiree_…`, `permis_revoque_redemande` | état en mémoire (`OnceLock<Mutex<Core>>`), aucun fichier → un redémarrage redemande tout ; `access_list`/`confirm_*` hors session → **401** ; `/mcp` n'expose aucun outil de décision (`-32602 Unknown tool`) |
| 3 | écran « Accès » : clients (droits, révocation), appairage (code de comparaison), confirmations (une fois / 10 min / refus), 100 dernières décisions ; tout texte client par `esc()` + test « libellé hostile » | FAIT | `src/ui.rs` (n. : bouton barre, dialogue, 3 tests), `src/ui/app.js` (+295), `src/ui/app.css` (+68) | + `libelle_hostile_jamais_dans_le_html`, `ecran_acces_echappe_tout_champ_venu_d_un_client`, `ecran_acces_branche_sur_les_actions_locales` ; + E2E `bandeau_sans_texte_de_client_et_ecran_local_seulement` | `esc()` retire aussi C0/DEL ; inventaire statique des 30+ interpolations du bloc : toutes par `esc()`/`js()`, aucun `${p.label}` nu |
| 4 | bandeau + notification système, une par demande ; la fenêtre cachée ne revient au premier plan que sur clic | FAIT | `confirm.rs` (`notifier`), `src/notify.rs` (`approval_pending_message`), `app.js` (`accessBadge`) | + `message_de_demande_de_confirmation_sans_secret` ; + E2E `bandeau_sans_texte…` (3 compteurs, aucun texte client) | toast émis **à la création** seulement (`Decision.created`), hors du fil de la requête ; `desktop/` ne lève la fenêtre que sur clic tray / 2ᵉ lancement / boot — rien côté confirmation |
| 5 | approbations, refus et expirations par `on_auth_decision`, une ligne au journal d'accès | FAIT | `src/gateway.rs` (`on_auth_decision` → `confirm::note_decision`), `confirm.rs` (`note`, `balayer`) | + E2E `refus_local_ne_lance_rien` (`mcc.log` + anneau), `permis_expire_…` (« permis expiré »), `approbation_…` (refus de droit `right_missing` → ligne : seul le hook peut l'écrire) | anneau de 100, `reason` = `approval_pending` / `approval_denied` / `right_missing` / `grant_expired` |

## 2. Sorties finales (2026-11-16)

```
cargo fmt --check                         → exit 0, aucun diff
cargo clippy --all-targets -- -D warnings → exit 0, 0 warning
cargo test --lib                          → 273 passed; 0 failed (254 avant, +19)
cargo test --test e2e --no-fail-fast      → 74 passed; 0 failed; 0 ignored (902,40 s) [69 avant, +5]
cargo check --all-targets                 → exit 0
cargo check -p mcp-control-desktop        → exit 0
```
Entrée Run `MCP Control` : supprimée par `scenarios_actions` (régression connue), restaurée à l'identique depuis `%TEMP%\mcc-m3-run-avant.txt`, **deux** `reg query` conformes (`…\mcp-control-desktop.exe" --hidden`).
`cargo test --lib` a rendu **272/273 une fois** sur `auth::session::…cookie_valide_seulement_pour_la_session_vivante` (créneau process-global partagé, lot M2 : race préexistante hors périmètre, `src/auth/session.rs`) ; vert aux deux passages suivants.

## 3. Les 5 tests de confirmation (critère de fin), `tests/e2e/scenarios_auth.rs`

| test | ce qu'il prouve | résultat |
|---|---|---|
| `confirmation_rien_ne_part_sans_accord_puis_part` | sans accord : `403`+`-32023`+`approval_pending`+`retryable`+`retryAfterMs`, **marqueur absent du disque** (rien n'est lancé) ; effet exact affiché ; après « une fois » : 200 + marqueur écrit ; l'appel suivant redemande | ok |
| `permis_expire_et_redemande` | `MCC_CONFIRM_SECS=2` : permis « fenêtre » → 200 + lancé ; 3 s plus tard même appel → **redemandé**, ligne « permis expiré » | ok |
| `refus_local_ne_lance_rien` | refus local → `approval_denied`, 403, marqueur absent, refus dans `mcc.log` | ok |
| `approbation_par_un_autre_client_refusee` | B (`execution`) → `POST /api/action` **401**, `/mcp` : 10 outils, aucun outil de décision, `-32602` ; demande de A intacte, rien lancé ; refus de droit de B → ligne `right_missing` | ok |
| `bandeau_sans_texte_de_client_et_ecran_local_seulement` | `/api/state.access` = 3 compteurs (libellé hostile `<img onerror>` absent), `access_list` hors session 401, libellé brut côté session (échappé par la page) | ok |

## 4. Passe navigateur — recette (non exécutée ici : pas de navigateur)

```
cargo build
New-Item -ItemType Directory -Force "$env:TEMP\mcc-m3"
# config.json UTF-8 SANS BOM : ui_port 5503, autostart false, removed_seeds = les 8 ids, servers []
$env:MCC_CONFIG = "$env:TEMP\mcc-m3\config.json"; $env:MCC_KEYRING_SERVICE = ""
.\target\debug\mcp-control.exe
# stdout : [session] http://127.0.0.1:5503/#t=<43 car.>  → ouvrir cette URL (jamais dans mcc.log)
```
Clics : barre d'outils → **Accès** (bouclier, pastille rouge = demandes) → onglets **Confirmations / Clients /
Appairage / Journal** ; Clients : basculer `lecture`/`pilotage`/`execution`, **Révoquer** ; Appairage :
`.\target\debug\mcp-control.exe auth pair http://127.0.0.1:5503` dans un 2ᵉ terminal → le même code à 6 caractères,
cocher `execution`, **Accepter** ; Journal : 100 dernières décisions.
Limite : la carte de confirmation exige un client MCP qui **signe** un appel `execution` — aucun outil du dépôt ne
le fait à la main avant le pont (M5). Générateur reproductible : `cargo test --test e2e
confirmation_rien_ne_part_sans_accord_puis_part` (mêmes cartes, sur son propre moteur).

## 5. Changements visibles (pour le CHANGELOG)

- Les 10 outils `execution` exigent une **confirmation locale** : `403`/`approval_pending` + `retryAfterMs`, rien ne part avant l'accord.
- Écran **Accès** dans la barre d'outils : clients et droits, appairage, confirmations (une fois / 10 min / refus), 100 dernières décisions ; bandeau + toast système, **une** notification par demande.
- `/api/state` gagne `access` : `{pending, grants, pairs}` (compteurs seulement).

## 6. Écarts

1. `MCC_CONFIRM_SECS` (borné 1–86 400 s) n'existe que pour les tests ; sans lui, dix minutes (§5).
2. Le permis est lié aux **arguments** (SHA-256 des arguments réellement exécutés) en plus du couple client/outil : §5 dit « ce client et cet outil », C5 exige que l'approuvé soit l'exécuté → un argument différent redemande. Plus strict, jamais plus permissif.
3. Harnais E2E : approbation automatique (`Engine::set_auto_approve`, active par défaut, coupée par les 5 scénarios M3) + `MCC_E2E_CONFIRM_SECS` relayé ; `execute_cas` approuve et rejoue la requête pour la cellule `execution` de la matrice.

## 7. Découvert en chemin, NON corrigé

- `access_list` ne rendait pas `ok: true` → l'écran se serait arrêté sur « lecture impossible ». **Corrigé** (`src/manage/actions.rs`).
- `cargo fmt --check` était rouge sur tout le travail hérité (`confirm.rs`, `ui.rs`, `notify.rs`, `actions.rs`) : reformaté, aucun changement de comportement.
- `src/auth/session.rs` : race préexistante entre deux tests unitaires (créneau global) — 1 échec sur 3 passages complets.
- Le crate voisin `litemcp` a été refactoré (`src/server/`, `src/http/`) **pendant** ce lot : deux compilations ont échoué côté lib puis sont revenues vertes ; les nombres ci-dessus datent d'un arbre vert, `Cargo.lock` inchangé côté MCP Control.
- Rien ne permet à un humain de fabriquer une demande de confirmation avant le pont M5 (cf. §4) : une commande `mcp-control auth call <url> <outil> <json>` serait le complément naturel (hors périmètre).
