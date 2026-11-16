# M1 — Identités, registre des clients, appairage

État : **FAIT**, 6 points sur 6. Base : branche `auth`, `ee4a796`, Windows x86-64, 2026-11-16.
Aucune route existante ne change de comportement : l'authentification n'est pas encore exigée (lot M2).

## 1. Points

| # | point | état | fichiers | tests (positifs / négatifs) | preuve |
|---|---|---|---|---|---|
| 1 | Identité du serveur : Ed25519 au trousseau, repli fichier 0600 journalisé, jamais dans `config.json` | FAIT | `src/auth/secret.rs` (n.), `src/manage/config.rs`, `src/lib.rs` | + repli créé puis **même** clé au redémarrage ; + `auth.server` = clé publique ; − bloc privé illisible = erreur, jamais de régénération ; − aucune clé privée dans `config.json` | journal `[auth] identité du serveur créée (mcc-server) — clé privée : trousseau du système` (self-test, service jetable, entrée `cmdkey` supprimée) ; repli : `auth/server.key` + `icacls` sans `(I)` |
| 2 | Registre `auth.clients[]` (10 champs §3.2), `KeyStore::lookup`, écriture atomique, audit 05 #10 | FAIT | `src/auth/registry.rs` (n.), `src/types.rs`, `src/manage/config.rs` | + round-trip `save_config`/`load_config` (droits, `paired_from`) ; + `lookup` → `Active` puis `Revoked` à l'appel suivant ; − keyid hors grammaire, clé invalide (y=2), hex majuscule, `admin`, doublon : refusés | `config_json_a_permissions_reduites_apres_ecriture` → `icacls` : utilisateur seul, 0 héritage (audit 05 #10 **fini**) |
| 3 | `POST /auth/pair` ouvert mais borné : 5/min/adresse, 16 en attente, 2 min, 404 sur id inconnu | FAIT | `src/auth/pair.rs` (n.), `src/lib.rs` | + appairage **entre deux process** (CLI réel), `pending` → `approved` ; − non approuvée = registre vide ; − expirée (122 s réelles) non approuvable ; − 500 demandes : 5 admises / 495 `429`, file ≤ 16, `/api/state` et `/api/hello` debout ; − 404 id inconnu, 400 horloge/clé, 409 keyid pris | `scenarios_auth.rs` (6 scénarios), `pair.rs` (10 tests) |
| 4 | 7 actions locales `admin`, jamais au gateway, révocation effective | FAIT | `src/manage/actions.rs`, `src/gateway.rs`, `src/lib.rs`, `src/auth.rs` | + les 7 répondent en local ; − `injections_interdites` 6 → **13** entrées × 17 tools = **221** refus `-32602` ; − LAN → `403` avant dispatcher ; − action inconnue → `{ok:false}` | `injection_action_refusee_sur_tous_les_tools_daction`, `appairage_ouvert_mais_borne_et_actions_admin_interdites_au_lan` |
| 5 | CLI `auth pair\|list\|revoke\|fingerprint` | FAIT | `src/auth/cli.rs` (n.), `src/main.rs` | + code affiché = code de la fenêtre, sortie 0, clé privée côté client (0600) ; + `fingerprint` = empreintes seulement ; − `https://` refusé (M4), keyid invalide refusé | E2E `appairage_complet_entre_deux_process` (stdout CLI comparé à `pair_list`) |
| 6 | Aucune route existante ne change | FAIT | — | 243 tests lib (214 avant) et **65** E2E verts | §2 |

## 2. Sorties finales (2026-11-16)

```
cargo fmt --check                         → exit 0
cargo clippy --all-targets -- -D warnings → exit 0, 0 warning
cargo test --lib                          → 243 passed; 0 failed   (214 avant + 29)
cargo test --test e2e --no-fail-fast      → 65 passed; 0 failed; 0 ignored (767,34 s)
```

E2E : 3 passages complets ; les 1er et 2e ont rendu 64/65 sur deux scénarios **différents et étrangers à ce lot** (`tempete_muettes_lan…`, puis `gateway_tool_refresh…` : « tool execution timed out after 30000 ms »), chacun vert isolément (3,88 s / 70,90 s) et verts au 3e passage — même profil que l'historique (lot F : 1 échec isolé sur 6 passages).

**Taille ajoutée par `keyring`** (`cargo build --release`, binaire avant/après) : base `ee4a796` **3 094 528** o ; M1 `--no-default-features` **3 260 928** o (+166 400 = tout le module `auth`) ; M1 avec `keyring` **4 887 552** o → **+1 626 624 o (+1,55 Mio) pour `keyring` seul**.
Dépendances : **25 paquets** ajoutés au `Cargo.lock`, **3 compilés sous Windows** (`keyring 4.2.0`, `keyring-core 1.0.1`, `windows-native-keyring-store 1.1.0`) ; les 22 autres (`apple-native-keyring-store`, `zbus…`, `secret-service`, `security-framework`, `num*`, `aes`/`cbc`/`hkdf`…) sont verrouillés mais exclus par `cfg(target_os)`. `keyring 4.2.0` existe (MIT OR Apache-2.0, MSRV 1.88) et compile.

**Point non tranché n° 1 de `docs/auth.md`, vérifié par le code** : le gateway n'expose **aucun gabarit** — 4 ressources à **URI exacte** (`mcp-control://servers`, `://journal`, `://settings`, `://server/<id>`), aucune `resource_template`, `resources/templates/list` jamais servi, `require_resource_right` jamais appelé : l'écart de couverture des gabarits de la lib n'a **aucun effet** ici (option (a) tenue).
Mais « n'expose aucune ressource » (A0) était inexact.

## 3. Changements visibles (pour le CHANGELOG)

- `mcp-control auth pair <url>`, `auth list`, `auth revoke <keyid>`, `auth fingerprint`.
- `config.json` gagne `auth.server` (clé **publique**) et `auth.clients[]` ; permissions réduites sous Windows aussi.
- Deux routes non authentifiées : `POST /auth/pair` (5/min/adresse, 16 en attente, 2 min) et `GET /auth/pair/{id}`.

## 4. Écarts par rapport à `docs/auth.md`

1. `keyring` : feature **`v1`** (les 3 magasins natifs), pas `keychain` — sous-feature d'`apple-native…` en 4.2.0 ; `default-features = false`, `=4.2.0`, feature Cargo `keyring` active par défaut.
2. `MCC_KEYRING_SERVICE` (env) renomme le service du trousseau, **valeur vide = repli fichier forcé** : sans cela chaque moteur de test écrirait une identité dans le Credential Manager de l'utilisateur. Non prévu au §9.
3. `client_rotate` prend `{keyid, new_keyid, public_key}` : la spec ne dit pas d'où vient la nouvelle clé.
4. `POST /auth/pair` répond **200** (statut non fixé) ; une demande non décidée survit 120 s au-delà de son expiration, sinon le client lirait 404 au lieu de `expired`.
5. L'état d'authentification vit dans `Settings` : un champ ajouté à `AppState` imposait de toucher `src/gateway.rs` hors périmètre. Conséquence assumée : un seul verrou (`state.settings`).
6. Le quota d'appairage s'applique **aussi au loopback** : c'est ce qui borne la tempête exigée par le critère E2E.
7. `admin` n'est pas encore *vérifié* faute de session locale (M2) : seule la provenance locale l'est ; la révocation, elle, est déjà effective immédiatement (test).
8. Taille de `keyring` mesurée (§2) alors que `docs/auth.md` la donnait « non mesurée ».

## 5. Découvert non corrigé

- `icacls /inheritance:r` retire aussi SYSTEM et Administrateurs : une sauvegarde système ne lira plus `config.json`. À trancher au lot R (ajouter SYSTEM en lecture ?).
- `state-cache.json` reste sans ACL réduite (aucun secret dedans) — à réévaluer si M3 y écrit.
- `desktop/` n'est pas compilé par les commandes de vérification ; il n'appelle que `run_headless()`, signature inchangée — non vérifié par compilation.
