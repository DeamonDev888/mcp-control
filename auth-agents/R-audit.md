# Rapport d'Audit de Sécurité Adverse — Lot `R`

- **Auditeur** : `r-auditor` (indépendant, lecture seule)
- **Périmètre** : branche `auth`, commit `500f4fd` (`merge UI-modern`), binaire `target/release/mcp-control.exe` compilé avec `--features tls`
- **Références** : `docs/auth.md`, `audit-v2/S1-session.md`, `audit-v2/S2-csp.md`, `audit-v2/S4-systeme.md`, `audit-v2/UI-moderne.md`, `Cargo.toml`, `deny.toml`
- **Isolation** : instances isolées sur les ports `5590` et `5591`, dossier hors dépôt `%TEMP%\mcc-audit-r\`, `MCC_KEYRING_SERVICE=""`, `MCC_AUTOSTART_KEY="MCP Control Test R"`, `MCC_NO_LOCAL_DISCOVERY="1"`. Zéro interaction avec le port `5090`, le `config.json` racine ou le trousseau Windows réel.
- **Preuves brutes** : `C:\Users\Deamon\AppData\Local\Temp\mcc-audit-r\results.json` (généré par `node C:\Users\Deamon\AppData\Local\Temp\mcc-audit-r-runner.mjs`).

---

## 1. Résumé exécutif

| Gravité | Nombre |
| :--- | :---: |
| **P1 (Critique / contournement direct)** | **0** |
| **P2 (Majeur / rupture d'invariant ou régression fonctionnelle sécurité)** | **5** |
| **P3 (Mineur / durcissement & hygiène)** | **4** |

### Verdict

**AVIS RÉSERVÉ — 0 P1, mais 5 P2 à corriger avant le lot `D` (publication / release).**

La surface critique tient solidement : aucun contournement d'authentification sur `/mcp` ni `/api/*`, aucune élévation de privilèges depuis `lecture`, confirmation humaine M3 étanche (arguments liés par SHA-256, usage unique consommé, révocation et expiration effectives), session locale S1 robuste (cookie `HttpOnly`/`SameSite=Strict` + secret mémoire `X-MCC-Session`), CSP stricte `'self'` sans script/style inline, TLS 1.3 strict sans repli HTTP hors loopback, journal d'audit chaîné intègre (`verify-log` OK) et zéro fuite de secret sur les API, journaux ou outils.

En revanche, **5 constats P2** justifient un mini-lot de correction ciblé avant `D` :
- **Lot `M2` (transport `/mcp` & sas HTTP)** : `R-1` (rejeu inter-instances / inter-ports accepté faute de vérification du port dans `Host`), `R-2` (rejeu de nonce accepté dans la fenêtre ±60 s après redémarrage du serveur), `R-3` (en-têtes `Host`/`Origin`/`Cookie`/`X-MCC-Session` dupliqués acceptés + `Content-Length` non numérique avalé en `0`).
- **Lot `M1` / `S4` (stockage des clés)** : `R-4` (fenêtre TOCTOU dans `write_secret_file` qui écrit la clé privée en clair avant `restrict_permissions` et laisse `.key.tmp` en cas d'échec).
- **Lot `M1` / `S1` (CLI d'administration)** : `R-5` (`mcp-control auth list` et `auth revoke` échouent systématiquement en HTTP `401` depuis la fermeture de `/api/*`, empêchant une révocation d'urgence en CLI).

---

## 2. Tableau des constats (P1–P3)

| # | Gravité | Où (`fichier:ligne`) | Constat | Preuve (commande + résultat réel) | Correctif proposé |
| :--- | :---: | :--- | :--- | :--- | :--- |
| **R-1** | **P2** | `src/lib.rs:605-612`<br>`src/gateway.rs:1537-1541, 1574` | **Rejeu inter-instances / inter-ports d'une requête signée `/mcp`.** `check_access` supprime le port via `host_only_of(h)` et ne vérifie pas que le port de `Host` correspond au `ui_port` écouté ; `gateway::respond_auth` transmet tel quel ce `Host` comme `@authority` attendu à `VerifiedRequest`. Une requête signée pour `127.0.0.1:5590` est acceptée par une 2e instance sur `127.0.0.1:5591` partageant le même client enregistré (rupture de `docs/auth.md` §2.2). | `node ...\mcc-audit-r-runner.mjs` (`family2_ed25519.cross_instance_replay_5590_to_5591`) :<br>Envoi d'une requête signée `Host: 127.0.0.1:5590` sur le port `5590` → `200 OK`, puis rejeu des **mêmes octets bruts** sur le port `5591` → **`200 OK`** (`{"jsonrpc":"2.0","id":1,"result":...}`). | Dans `check_access` (ou `gateway::respond_auth`), vérifier que le port porté par `Host` correspond exactement au `ui_port` de l'instance (ou rejeter en `403`/`401` tout `Host` dont le port diffère de `ui_port`). |
| **R-2** | **P2** | `src/gateway.rs:192-197, 1574` | **Rejeu d'un nonce accepté après redémarrage du serveur.** Le `NonceCache` de `SignatureVerifier` est uniquement en RAM : après redémarrage du moteur, toute requête signée capturée dans les 60 secondes précédentes redevient rejouable (y compris des outils destructifs `pilotage` comme `stop_server` / `remove_server` qui ne requièrent pas de confirmation). | `node ...\mcc-audit-r-runner.mjs` (`family8_dos_and_nonce.nonce_replay_after_restart`) :<br>Requête signée avant arrêt → `200 OK` ; redémarrage de `mcp-control.exe` sur `5590` ; rejeu des mêmes octets → **`200 OK`**. | Enregistrer `boot_ts = now_ts()` au démarrage du moteur et refuser en `401` toute requête `/mcp` signée dont le paramètre `created < boot_ts` (zéro E/S disque, ferme 100 % des rejeux pré-redémarrage). |
| **R-3** | **P2** | `src/lib.rs:602-629, 1405-1409`<br>`src/auth/session.rs:292-308` | **En-têtes critiques dupliqués acceptés (`Host`, `Origin`, `Cookie`, `X-MCC-Session`) et `Content-Length` mal formé ignoré.** `check_access` rejette `Content-Length` dupliqué (`400`) mais utilise `.find()` pour `Host` et `Origin` ; `session.rs` utilise `.find()` pour `Cookie` et `X-MCC-Session`. De plus, `value.parse().unwrap_or(0)` traite un `Content-Length: abc` comme `0` au lieu de `400`. Non conforme à RFC 9110 §7.2 (risque de désynchronisation en présence d'un intermédiaire). | `node ...\mcc-audit-r-runner.mjs` (`family1_routes_http`, `family6_session_ui`) :<br>• `GET /` avec `Host: 127.0.0.1:5590` + `Host: evil.example:5590` → **`200`**<br>• `GET /` avec `Origin: http://127.0.0.1:5590` + `Origin: https://evil.example` → **`200`**<br>• `GET /api/state` avec `Cookie` valide + `Cookie` invalide → **`200`**<br>• `GET /api/state` avec `X-MCC-Session` valide + invalide → **`200`** | Dans `check_access`, renvoyer `400 Bad Request` si `count(host) != 1`, `count(origin) > 1`, `count(x-mcc-session) > 1`, `count(cookie)` portant plusieurs `mcc_session=`, ou si `Content-Length` échoue à `parse::<usize>()`. |
| **R-4** | **P2** | `src/auth/secret.rs:110-126` | **Fenêtre TOCTOU et résidu `.key.tmp` non supprimé dans `write_secret_file`.** Contrairement à `session::write_private` (`src/auth/session.rs:399-414`), `write_secret_file` écrit la clé privée en clair (`server.key`, `audit.key`, `tls/server.key`, `clients/<keyid>.key`) dans `<path>.key.tmp` **avant** d'appeler `restrict_permissions`. Si `restrict_permissions` échoue (ligne 118), la fonction retourne `Err` sans supprimer `.key.tmp`, laissant le secret sur disque avec les ACL héritées. | Relecture statique `src/auth/secret.rs:115-123` :<br>`std::fs::write(&tmp, secret)?;` puis `restrict_permissions(&tmp)?;` sans `remove_file(&tmp)` sur erreur d'ACL. | Aligner `write_secret_file` sur `session::write_private` : créer le fichier `.tmp` vide → `restrict_permissions` (avec `remove_file` en cas d'erreur) → écrire le secret + `sync_all` → `rename`. |
| **R-5** | **P2** | `src/auth/cli.rs:264-278` | **Commandes CLI `auth list` et `auth revoke` cassées (`401 Unauthorized`).** `cli::action()` envoie `POST /api/action` sur `127.0.0.1:<port>` sans cookie `mcc_session` ni en-tête `X-MCC-Session`. Depuis le lot M2/S1, toute requête `/api/*` sans session locale échoue en `401`. La révocation d'urgence d'une clé en ligne de commande (`mcp-control auth revoke <keyid>`) est inopérante. | `mcp-control.exe auth list` avec `MCC_CONFIG=<temp>\inst1\config.json` :<br>exit `1`, stderr : `auth list : le moteur a répondu 401`. | Faire obtenir à `cli::action()` un jeton à usage unique via le canal fichier S1 (`auth::session::request_bootstrap`), l'échanger sur `POST /api/session`, puis envoyer `Cookie` + `X-MCC-Session` sur `POST /api/action`. |
| **R-6** | **P3** | `src/lib.rs:1742-1751`<br>`src/auth/pair.rs:273-305` | **`GET /auth/pair/{pair_id}` non limité en débit.** Contrairement à `POST /auth/pair` (limité à 5/min/IP) et à `docs/auth.md` §3.4 / §8.1, `GET /auth/pair/{id}` n'a aucun limiteur de débit et acquiert `state.settings` + exécute `prune()` à chaque requête. | `node ...\mcc-audit-r-runner.mjs` (`family5_pairing.get_pair_20_rapid`) :<br>20 `GET /auth/pair/{pair_id}` en rafale → `20/20` en **`200 OK`** (`any_429: false`). | Ajouter un limiteur par IP sur `GET /auth/pair/{id}` (ex. 30–60 req/min pour couvrir le polling toutes les 2 s) avant l'acquisition de `state.settings`. |
| **R-7** | **P3** | `src/lib.rs:243-259`<br>`src/auth/pair.rs:246` | **Repli PRNG non cryptographique résiduel dans `random_hex` (défaut S-3).** Si `os_random` échoue, `random_hex` se rabat encore sur `RandomState + now_ts() + compteur` pour `pair_id`, les nonces `/api/hello` et les `keyid` clients (`session.rs` est déjà corrigé en échec fermé). | Relecture statique `src/lib.rs:247-255` et `src/auth/pair.rs:246` (`let pair_id = crate::random_hex(2);`). | Rendre `random_hex` faillible (`Option<String>`) sans repli, et renvoyer `503` sur `/auth/pair` et `/api/hello` si l'entropie OS est indisponible. |
| **R-8** | **P3** | `src/audit.rs:138, 335-378`<br>`src/auth/session.rs:131-139, 261-288` | **Verrous `Mutex` tenus pendant des E/S disque/trousseau + écriture non atomique de `.head`.** `audit::record` tient `GLOBAL` pendant `load_or_create_audit_key` (trousseau + disque), `Journal::open`, `rotate` et `write_all`. `session::journal` tient `LOG_PATH` pendant l'écriture dans `mcc.log`, et est appelé sous verrou `SLOT` dans `exchange()`. `audit::write_head` utilise `std::fs::write` sans écriture atomique ni `restrict_permissions`. | Relecture statique `src/audit.rs:135-140, 335-378` et `src/auth/session.rs:130-140, 261-288`. | Cloner le chemin hors du verrou `LOG_PATH` ; relâcher `SLOT` avant `journal()` ; écrire `auth-audit.head` de manière atomique (`.tmp` + ACL + `rename`). |
| **R-9** | **P3** | `Cargo.lock`, `Cargo.toml:25`, `deny.toml` | **Avertissements de chaîne de dépendances (`cargo audit` / `cargo deny`).** `rustix 1.1.5` (`RUSTSEC-2026-0486`, unsound sur `xattr`, corrigé en `1.1.6`), `async-executor 1.14.0` (yanked, via `zbus`/`keyring` Linux), `chrono 0.4.45` (`RUSTSEC-2026-0543`, unmaintained, non listé dans `deny.toml`), `glib 0.18.5` (`RUSTSEC-2024-0429`, desktop Linux), et dépendance par chemin `litemcp = { path = "../litemcp" }` (wildcard). | `cargo deny check` → exit `0` (`advisories ok, bans ok, licenses ok, sources ok`, 1 warning wildcard `litemcp`).<br>`cargo audit` → exit `0`, **8 warnings** (`RUSTSEC-2026-0543`, `2024-0370`, `2026-0371`, `2026-0370`, `2026-0372`, `2024-0429`, `2026-0486`, `async-executor 1.14.0` yanked). | Au lot `D` : `cargo update -p rustix` (≥ `1.1.6`), publier `litemcp 0.12.0` sur crates.io et repasser `wildcards = "deny"` dans `deny.toml`. |

---

## 3. Vérifié et sain (11 familles d'attaques)

Toutes les preuves ci-dessous ont été exécutées contre `target/release/mcp-control.exe` (`500f4fd`) via `node C:\Users\Deamon\AppData\Local\Temp\mcc-audit-r-runner.mjs` (résultats complets dans `%TEMP%\mcc-audit-r\results.json`) et par revue statique exhaustive.

### Famille 1 — Contournement de routes & parsing HTTP
- **Routes sans identité en loopback** : `POST /mcp` → `401`, `GET /mcp` → `405`, `GET /api/state` → `401`, `GET /api/detail?id=x` → `401`, `POST /api/action` → `401`, `POST /api/session` (jeton faux) → `401`, `POST /api/session/close-others` → `401`, `GET /auth/pair` → `405`, `GET /auth/pair/<32a>` → `404`.
- **Simulation LAN / anti-rebinding / CSRF** : `Host` absent → `400` ; `Host: evil.example:5590` → `403` ; `Origin: https://evil.example` → `403` ; `X-Forwarded-For: 127.0.0.1` ignoré → `401`.
- **Chemins voisins** : `/mcp/` → `404`, `/MCP` → `404`, `//mcp` → `404`, `/%2fmcp` → `404`, `/%6dcp` → `404`, `/API/state` → `404`, `//api/state` → `404`, `/api/state/` → `401`, `/mcp?foo=1` → `401` (sans sig) / `200` (avec sig valide sur `@path: /mcp`, query ignorée).
- **Méthodes inattendues** : `OPTIONS` sur `/mcp` et `/api/action` → `403` ; `PUT`/`DELETE`/`HEAD` sur `/mcp` → `405`, sur `/api/action` → `401`.
- **Cadrage & casse** : `Content-Length` dupliqué → `400` ; `Transfer-Encoding: chunked` → `400` ; `Authorization` dupliqué → `401` ; casse mixte (`hOsT`, `cOoKiE`, `x-McC-sEsSiOn`) normalisée correctement (`200`). *(Voir `R-3` pour les doublons `Host`/`Origin`/`Cookie`/`X-MCC-Session`.)*

### Famille 2 — Signature Ed25519 (RFC 9421 + RFC 9530)
- **Requête signée valide (`lecture`)** : `POST /mcp` (`tools/list`) → `200 OK`, exactement **10 outils `lecture`** exposés (`check_update`, `get_settings`, `journal`, `list_frameworks`, `list_peers`, `list_server_tools`, `list_servers`, `runtime_history`, `server_detail`, `store_catalog`).
- **Rejeu immédiat et différé (+1,5 s)** : `401` de corps constant `{"error":{"code":-32600,"message":"unauthorized"},"id":null,"jsonrpc":"2.0"}` + `WWW-Authenticate: Signature realm="litemcp", tag="litemcp-auth-v1"`.
- **Altérations** : corps modifié → `401` ; en-tête couvert (`Mcp-Method`) modifié → `401` ; horloge décalée `-65 s` et `+65 s` → `401` ; signature d'une autre clé revendiquant le `keyid` victime → `401`.
- **En-tête non couvert** : `Accept: text/event-stream` + `X-Forwarded-For: 10.0.0.1` → `200` en `text/event-stream`, sans aucun changement d'identité ni d'autorisation.
- **Saturation du cache de nonces (4096/clé)** : envoi de 4101 requêtes signées distinctes en rafales de 16 → `4095` acceptées (`200`), refus dès la saturation (`reject_count: 6`, `first_reject_at: 4093`), puis rejeu de la 1re requête toujours refusé en **`401`** (échec fermé confirmé). *(Voir `R-1` pour le rejeu inter-ports et `R-2` pour le rejeu post-redémarrage.)*

### Famille 3 — Élévation de privilèges
- Client `lecture` appelant `stop_server`, `set_host`, `import_config`, `add_server`, `call_server_tool` → tous refusés en **`403`** JSON-RPC `-32023`, `data.reason = "right_missing"`.
- Client `lecture` appelant `pair_approve` via `/mcp` → `-32602` (`Unknown tool: pair_approve`), aucune action `admin` n'est exposée sur `/mcp`.
- Champs hors schéma (`list_servers` avec `{"state":"up","action":"start","id":"x"}`) → `-32602` (`unknown field (strict object)`).

### Famille 4 — Confirmation humaine (M3)
- Client `execution` appelant `add_server` → `403`, JSON-RPC `-32023`, `data: {"reason":"approval_pending","retryable":true,"retryAfterMs":2000}`.
- Tentative d'auto-approbation via `/mcp` (`confirm_approve`) → `-32602` (outil inexistant) ; via `/api/action` sans session → `401`.
- Approbation locale `once`, puis tentative d'exécution avec **arguments modifiés** (`port: 59998` au lieu de `59999`) → **`403 approval_pending`** (liaison SHA-256 des arguments validée).
- Exécution avec les arguments exacts → `200 OK`, puis réutilisation immédiate du permis `once` → **`403 approval_pending`** (usage unique consommé).
- Permis `window` révoqué (`grant_revoke`) → appel suivant `403 approval_pending`.
- Permis `window` expiré (`MCC_CONFIRM_SECS=3`, attente 3,5 s) → appel suivant `403 approval_pending`.
- Refus (`confirm_deny`) → appel suivant `403 approval_denied`.

### Famille 5 — Appairage (M1/M4)
- Code de comparaison 6 chiffres : `798 041` calculé côté client == `798 041` côté serveur (`pair_list`).
- Auto-approbation sans session → `401` ; approbation avec le droit `admin` → refusée (`{"ok":false,"error":"droit inconnu ou réservé (admin)"}`).
- Rate-limit `POST /auth/pair` : 5 requêtes acceptées (`200`), 6e et 7e refusées en **`429`**.
- Libellé hostile (`<script>alert(1)</script><img src=x onerror=alert(2)>\0\x1b[31m` + 4 Ko) → caractères de contrôle `\0` et `\x1b` retirés, tronqué à **64 caractères**, échappé à l'affichage via `esc()`. *(Voir `R-6` pour `GET /auth/pair/{id}`.)*

### Famille 6 — Session locale (S1) & nouvelle UI (S2 / UI-modern)
- `POST /api/session` : `200 OK`, `Set-Cookie: mcc_session=<43c>; HttpOnly; SameSite=Strict; Path=/` + `secret` en JSON ; rejeu du même jeton d'amorçage → `401`.
- Cookie seul (sans `X-MCC-Session`) → `401` ; `X-MCC-Session` seul (sans cookie) → `401` ; mauvais secret → `401` ; `Origin: https://evil.example` avec session valide → `403` ; session locale sur `POST /mcp` → `401`.
- `POST /api/session/close-others` (avec 2e session ouverte via le canal fichier S1) → `200 {"closed":1,"ok":true}`, 2e session révoquée (`401`), session courante toujours valide (`200`).
- En-têtes de sécurité sur `GET /` :
  - `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self' ipc: http://ipc.localhost; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`
  - `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Cross-Origin-Opener-Policy: same-origin`, `Cross-Origin-Resource-Policy: same-origin`.
- Nouvelle surface UI vérifiée en direct : `state_json` (`sessions_active: 1`, `announcements_count`, `settings.tls`), `access_list`, `litemcp_announcements`, `stop_server_force`, `server_pair` (0 clé privée dans la réponse), `client_rotate` (ancienne clé immédiatement `401`, nouvelle clé immédiatement `200`).

### Famille 7 — Fuites de secrets & intégrité du journal d'audit
- Recherche systématique de 8 marqueurs secrets (`PRIVATE KEY`, jeton d'amorçage, cookie `mcc_session`, secret `X-MCC-Session`, jeton webhook `SUPER_SECRET_WEBHOOK_TOKEN_777`, valeur `env` stdio `SUPER_SECRET_ENV_VALUE_999888`, nonce, signature Ed25519) sur 10 surfaces (`config.json`, `state-cache.json`, `mcc.log`, `auth-audit.log`, `/api/state`, `/api/detail`, `access_list`, `get_settings`, `journal`, `server_detail`) :
  - **0 fuite** dans `state-cache.json`, `mcc.log`, `auth-audit.log`, `/api/state`, `/api/detail`, `access_list`, `get_settings`, `journal`, `server_detail`.
  - Seules occurrences attendues et documentées (`docs/auth.md` §1.5) : `webhook_token` et `stdio_env_val` dans `config.json` lui-même (protégé par ACL Windows utilisateur + `SYSTEM` + `Administrateurs`).
- Vérification de la chaîne HMAC du journal d'audit : `mcp-control.exe auth verify-log --path <temp>\inst1\auth-audit.log` → exit `0`, `verify-log : chaîne INTACTE — 29 ligne(s), 1 segment(s), seq 1 → 29`.
- ACL Windows de `auth-audit.log` (`icacls`) : `BUILTIN\Administrateurs:(F)`, `NT AUTHORITY\SYSTEM:(F)`, `DESKTOP-VQD5PLM\Deamon:(F)` (aucune ACL héritée `(I)`).

### Famille 8 — Déni de service
- Tempête de 35 demandes `execution` distinctes (`ssh_save`) → file bornée à **32** demandes en attente (les plus anciennes évincées).
- Tempête de 40 signatures Ed25519 fausses en parallèle + requête `GET /api/state` simultanée → 40/40 refusées en `401` en **49 ms** au total ; la fenêtre locale (`/api/state`) répond `200 OK` en **33 ms**.

### Famille 9 — TLS 1.3 (M4)
- Instance redémarrée avec `ui_bind: "0.0.0.0"` sur port `5590` :
  - Handshake TLS 1.3 (`127.0.0.1:5590` et IP LAN `10.2.0.2:5590`) → **`TLSv1.3`**, suite `TLS_AES_256_GCM_SHA384`, empreinte SHA-256 du certificat identique à `access_list.tls.fingerprint` (`sha256:b9b1ea474ce29849121bca5ba1128a6b09c918842126b8cddb76f7d49238f4c3`), `GET /api/hello` → `200 OK`.
  - Handshake TLS 1.2 → **rejeté** (`SSL alert number 70` — `tlsv1 alert protocol version`).
  - HTTP en clair sur l'IP LAN (`10.2.0.2:5590`) → **rejeté** avant HTTP (`status: 0`, 7 octets d'alerte TLS, aucun repli silencieux vers HTTP).
  - ACL de `tls/server.key` (`icacls`) : `Administrateurs:(F)`, `SYSTEM:(F)`, `Deamon:(F)` uniquement.

### Famille 10 — Dépendances par feature
- `cargo tree --no-default-features --depth 1` : `litemcp v0.12.0` (path), `serde_json v1.0.154`.
- `cargo tree --depth 1` (défaut = `keyring`) : ajoute **`keyring v4.2.0`**.
- `cargo tree --features tls --depth 1` : ajoute **`rcgen v0.14.7`** et **`rustls v0.23.46`** (sans `tls12`, backend `ring`).
- `cargo deny check` : exit `0` (`advisories ok, bans ok, licenses ok, sources ok`). *(Voir `R-9` pour les warnings.)*

### Famille 11 — Relecture statique (`src/auth/**`, `src/audit.rs`, `src/lib.rs`)
- **0 `unwrap()` / `expect()`** hors `#[cfg(test)]` sur les entrées réseau dans `src/auth/**`, `src/audit.rs` et `src/lib.rs`.
- **Comparaisons de secrets en temps constant** : `crypto::ct_eq` vérifié (`src/crypto.rs:128-137`) et utilisé pour le cookie `mcc_session`, `X-MCC-Session`, le jeton d'amorçage, les preuves HMAC, les MAC du journal d'audit et l'épinglage `server_pub`/`PinnedServer`.
- **Arithmétique d'horloge** : `saturating_add`, `saturating_sub`, `abs_diff` et `Instant::saturating_duration_since` utilisés partout sur les chemins réseau (pas de débordement paniquant sur entrée réseau). *(Voir `R-4`, `R-7`, `R-8` pour les points relevés.)*

---

## 4. Non vérifiable (limites honnêtes)

- **Comportement du binaire sans la feature `tls`** : le binaire `target/release/mcp-control.exe` étant compilé avec `--features tls` (et aucune recompilation n'étant effectuée en lecture seule), le refus de bind `0.0.0.0` sans `tls` n'a été vérifié que statiquement (`src/lib.rs:831-842`) et via les tests unitaires existants, pas par exécution d'un binaire `--no-default-features`.
- **Trousseau Windows réel (DPAPI / Windows Credential Manager)** : volontairement non testé en direct (`MCC_KEYRING_SERVICE=""` obligatoire pour ne pas toucher le trousseau de l'utilisateur).
- **Webview Tauri réelle / navigateur CDP** : non relancé ici (déjà mesuré par le lot `UI-modern` dans `audit-v2/mesures/ui-modern/results.json`) ; l'audit R a vérifié les en-têtes CSP/COOP/CORP, le flux session/API et l'assainissement serveur.
- **Expiration réelle 12 h / 24 h des sessions locales** : vérifiée par lecture statique (`src/auth/session.rs:203-217`) et tests unitaires, non par attente d'horloge réelle.
- **Anti-force brute LAN (`auth::throttle`) en conditions réseau réelles** : le loopback étant exempté par conception (`throttle.rs:132`), le comportement de délai croissant (8 échecs) et de blocage (40 échecs) depuis une IP LAN distincte a été vérifié par revue statique (`src/auth/throttle.rs` + branchement `src/lib.rs:1587-1623`).

---

## 5. Preuve d'intégrité (`HKCU\...\Run` & `git status`)

### Clé registre `MCP Control` — AVANT et APRÈS audit (strictement identique)

```text
> reg query "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v "MCP Control"

HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run
    MCP Control    REG_SZ    "C:\Users\Deamon\Desktop\mcp-command-center\.lot-wt\S2\target\debug\mcp-control.exe" --hidden
```

### État du dépôt Git & processus résiduels

- `git status --short` avant audit : vide (`0` fichier suivi modifié).
- `git status --short` après audit : seul ce rapport non suivi `auth-agents/R-audit.md` (`0` fichier suivi modifié).
- Processus de test sur ports `5590` / `5591` : tous terminés via `taskkill /T /F /PID`, `netstat -ano | findstr 559` ne montre aucun socket `LISTENING` (uniquement `TIME_WAIT` résiduels de la rafale 4096).
