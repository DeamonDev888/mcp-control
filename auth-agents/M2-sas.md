# M2 — Le sas : signature exigée, droits par outil, session locale

État : **FAIT**, 6 points sur 6. Base : branche `auth`, `b12892d`, Windows x86-64, 2026-11-16.
Profil protocolaire : **`Legacy` (2025-11-25) inchangé** — décision M0 non rouverte (§5.3).

## 1. Points
| # | point | état | fichiers | tests (+ / −) | preuve |
|---|---|---|---|---|---|
| 1 | `/mcp` : `dispatch_verified` (C8), signature exigée en loopback aussi, contrôles existants avant la vérification | FAIT | `src/gateway.rs` (`respond_auth`), `src/lib.rs` | + `tools/list` signé 200 ; − sans signature / signature altérée / nonce rejoué / `created−3600` / clé révoquée → 401 constant | contrôles « sains » rejoués (§2) ; `WWW-Authenticate: Signature realm="litemcp"` |
| 2 | Droits des 30 outils (§4), échec si outil sans droit, `tools/list` filtré | FAIT | `src/gateway.rs` (`TOOL_RIGHTS`, `with_rights`), ressources en `lecture` | + 10/10/10 ; `lecture`→10, `pilotage`→10, `execution`→10, 3 droits→30 ; − `pilotage` + `add_server` → 403 `-32023`, **handler non appelé** (état et empreinte inchangés) ; − outil inconnu refusé même en `execution` | `tout_outil_enregistre_a_un_droit_declare` : surface publiée == table |
| 3 | Session §6 : jeton 32 o usage unique 5 min, cookie `mcc_session`, `#t=`, headless, `/api/*`, `app.js`, shell Tauri | FAIT | `src/auth/session.rs` (n.), `src/lib.rs` (`POST /api/session`, `Reply`), `src/ui/app.js`, `desktop/src/main.rs` | + cookie `HttpOnly; SameSite=Strict; Path=/` (sans `Max-Age` ni `Secure`), session → `/api/*` 200 ; − jeton rejoué/inventé 401, échange LAN 403, session sur `/mcp` 401 | `[session] http://…/#t=…` sur stdout seulement (0 jeton dans `mcc.log`) ; `cargo check -p mcp-control-desktop` vert |
| 4 | DA3 : `?access=` supprimé, `X-MCC-Access` lecture seule journalisée, preuve de pair → `legacy_proof` | FAIT | `src/lib.rs`, `src/gateway.rs` | + header hérité → `/mcp` 10 outils, GET `/api/state` 200 ; − écriture héritée 403 ; − `?access=` 401 ; `[auth] legacy_access` au journal | `legacy_access_en_lecture_seule_et_journalise`, `scenarios_boot::auth_matrix_lan` |
| 5 | Harnais E2E : appairage au démarrage, requêtes signées, scénarios existants verts | FAIT | `tests/e2e/harness.rs` | session = jeton imprimé (headless) ; client Ed25519 `mcc-e2e-…` appairé (3 droits), clé dans le tmp → survit au `restart` ; `Mcp-Method`/`Mcp-Name` couverts | 69/69 E2E, aucune assertion de sécurité affaiblie (§5.2) |
| 6 | Matrice `{identité} × {loopback, LAN} × {6 surfaces}` | FAIT | `tests/e2e/scenarios_auth.rs` | 96 cases demandées + 12 de rejeu = **108/108 conformes** | `matrice_identite_transport_surface` imprime `matrice : 108/108` |

## 2. Sorties finales (2026-11-16)

```
cargo fmt --check                         → exit 0, aucun diff
cargo clippy --all-targets -- -D warnings → exit 0, 0 warning
cargo test --lib                          → 254 passed; 0 failed; 0 ignored (51,4 s)   [243 avant, +11]
cargo test --test e2e --no-fail-fast      → 69 passed; 0 failed; 0 ignored (834,46 s)  [65 avant, +4]
cargo check -p mcp-control-desktop        → exit 0 (39,95 s)
```
Rejeu des contrôles « sains » (`correctifs-agents/05-securite.md` §2) sur une instance 5502 (`MCC_CONFIG` temporaire, `MCC_KEYRING_SERVICE` vide, bind 0.0.0.0) : loopback sans session 401 ; LAN sans code 401 ; `X-Forwarded-For`/`X-Real-IP` ignorés 401 ; code faux 401 ; `?access=` 401 ; `X-MCC-Access` valide 200 ; `/api/hello` LAN 200 puis 429 (31ᵉ) ; Host `evil.local` 403 ; Host absent 400 ; `OPTIONS` 403 ; `POST text/plain` `/api/action` 415 et `/mcp` 415 ; `Origin` étranger/`null` 403 ; ACAO absent sur origine étrangère (reflété sur la nôtre) ; CSP sur `/` ; corps 3 Mo 413 ; `Transfer-Encoding` 400 ; `Content-Length` dupliqué 400 ; `GET /mcp` 405 ; `/mcp` sans signature 401 + `WWW-Authenticate` ; code et jeton absents de `mcc.log` et `/api/state`.
Entrée Run `MCP Control` : supprimée par `scenarios_actions` (régression connue), restaurée depuis `%TEMP%\mcc-m2-run-avant.txt`, **deux** `reg query` conformes (`…\mcp-control-desktop.exe" --hidden`).

## 3. Matrice (point 6) — attendu = observé, 108/108

| identité | tools/list | outil lecture | outil pilotage | outil execution | `/api/state` | `/api/action` |
|---|---|---|---|---|---|---|
| sans identité | 401 | 401 | 401 | 401 | loopback 200 / LAN 401 | loopback 200 / LAN 401 |
| signature fausse | 401 | 401 | 401 | 401 | idem | idem |
| rejeu (même nonce) | 1ᵉʳ 200 (10) / 2ᵈ 401 | 1ᵉʳ 200 / 2ᵈ 401 | idem | idem | idem | idem |
| horloge décalée (−1 h) | 401 | 401 | 401 | 401 | idem | idem |
| clé révoquée | 401 | 401 | 401 | 401 | idem | idem |
| client `lecture` | 200 (10) | 200 | 403 | 403 | idem | idem |
| client `pilotage` | 200 (10) | 403 | 200 | 403 | idem | idem |
| client `execution` | 200 (10) | 403 | 403 | 200 | idem | idem |

`loopback` = `127.0.0.1`, `LAN` = IP locale (bind 0.0.0.0). `/mcp` se comporte pareil sur les deux transports (la signature ne dépend pas du chemin) ; `/api/*` ne répond qu'à la **session locale** — la clé MCP n'y ouvre rien, un code hérité n'y donne que la lecture.
Une 9ᵉ ligne (clé jamais appairée) et les 12 cases de rejeu portent le total à 108.

## 4. Changements visibles (pour le CHANGELOG)

- `/mcp` exige une **signature par requête** (RFC 9421) : plus aucun client sans identité, même en local.
- Les 30 outils sont bornés par les droits du client ; `tools/list` ne montre que les siens (`toggle_server` et `set_auto_restart` passent en `execution` : ils mènent à l'exécution).
- Session locale : jeton d'amorçage à usage unique (5 min), cookie `mcc_session`, `X-MCC-Request: 1` sur les écritures ; en headless l'URL `#t=` est imprimée au démarrage ; `?access=` **supprimé** et `X-MCC-Access` réduit à `lecture`, journalisé.

## 5. Écarts, et pourquoi

1. `POST /api/session` n'accepte que le **loopback** (403 sinon) : un jeton volé n'est pas échangeable du LAN.
2. Un `Mcp-Method` incohérent avec le corps rend **401** (le vérificateur couvre cet en-tête) et non `-32020` que garde le noyau ; refus dans les deux cas, assertion de `scenarios_gateway` élargie en conséquence.
3. Les ressources `mcp-control://…` déclarent `lecture` : sans déclaration la lib les rend invisibles à tout client authentifié (échec fermé) ; la table §4 ne couvre que les 30 outils.
4. Profil `Legacy` **conservé** : basculer en `Strict2026_07_28` casserait Claude Desktop/Cursor/Claude Code et les fixtures ; la négociation par requête reste un lot dédié (M0 §3).

## 6. Découvert en chemin, NON corrigé

- `effective_require_access` n'est plus consultée (le loopback n'exempte plus rien) : gardée exportée pour la portée historique du code partagé (§10), à supprimer au lot D.
- `app.js` : un 401 sur `/api/*` ouvre encore le dialogue du **code hérité** ; en session absente le message utile serait « rouvrez le lien du démarrage ». Cosmétique, hors périmètre « transport ».
- Fenêtre rouverte sur un moteur déjà lancé (`reused`) : aucun jeton pour ce process, `/api/*` refuse ; `desktop/` compile (`cargo check`) mais l'échange du fragment n'a pas été vu dans une vraie fenêtre Tauri (pas de session graphique ici).
- Deux fichiers **hors périmètre**, modification minimale et déclarée : `scenarios_boot.rs` (l'assertion `?access=` → 200 devient 401, la mission supprime `?access=`) et `scenarios_actions.rs` (`post_json_long` présente le cookie + `X-MCC-Request`, aucune assertion touchée). À valider par le parent.
