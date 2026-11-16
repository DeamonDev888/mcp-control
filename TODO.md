# TODO — Plan d'améliorations MCP Control

**Date** : 2026-10-26 · Base : audit complet du 2026-10-26 (les 5 fichiers `AUDIT-*.md` ont été retirés du dépôt) + 3 analyses parallèles (tests E2E, performances, refactorisation).
**Mise à jour** : 2026-11-15 — lots A (moteur), B (autostart + sécurité), C (interface), F (finition), G (passe navigateur) et D (docs) livrés, non commités (rapports dans `restes-agents/`) ; section 5 réécrite en conséquence.
Tailles : **S** = ≤ ½ jour · **M** = 1-2 j · **L** = 3+ j.

---

## ✅ Lot 2026-10-30 (2) — Finitions UI « niveau produit » — LIVRÉ (v0.7.18)

Revue UI complète (moteur réel + mesures DOM) → lot livré dans `ui.rs` + `manage.rs` :

| # | Quoi | Statut |
|---|---|---|
| B1 | **Noms de serveurs écrasés à 0 px** (badges vs `flex:1`) : ligne 1 = état + nom + actions, ligne 2 = badges + compteurs ; explorateur 280 → 320 px | ✅ v0.7.18 |
| B2 | **Icônes SVG uniques** (trait 1.5, `currentColor`) partout — toolbar, dialogs, menus, détail, tuiles — fin des emojis | ✅ v0.7.18 |
| B3 | **Destructif assumé** : bordure rouge permanente, « Supprimer » séparé, `confirm()` sur la suppression d'un serveur importé (les seeds l'avaient déjà) | ✅ v0.7.18 |
| B4 | **Filtre d'explorateur** (nom/id/framework/port/état) + raccourcis `/`, Ctrl+K, Échap | ✅ v0.7.18 |
| B5 | **Tuiles d'accueil cliquables** : filtre UP, tri tools, dialog machines ; tuile active marquée | ✅ v0.7.18 |
| B6 | **Markdown inline des descriptions d'outils** (gras/italique/code, échappé avant transformation) | ✅ v0.7.18 |
| B7 | **Micro-finitions** : kv copiables d'un clic, tiret cadratin « — », toasts succès/erreur, halo LiteMCP statique, « désuet » → « legacy », tooltips badges, correctif « vv » (tag GitHub normalisé dans `check_update`) | ✅ v0.7.18 |
| B8 | **Boutique : navigation** — en-tête collant (recherche + tri + catégories visibles au défilement), tri Ordre catalogue / A→Z, compteur de résultats, bascule « Masquer les installés » | ✅ v0.7.18 |
| B9 | **Boutique : cartes** — icône sur TOUTES les cartes (glyphe neutre en repli), état « ✓ Déjà installé » vert non cliquable selon `state.servers` (re-rendu après install), ligne de commande retirée | ✅ v0.7.18 |
| B10 | **Raccourcis boutique** — `/` et Ctrl+K visent la recherche de la boutique quand son dialog est ouvert | ✅ v0.7.18 |
| B11 | **Boutique : 100 % des cartes avec logo** — inventaire des 21 items sans icône, marques officielles récupérées (Apple, Next.js via Simple Icons), 20 glyphes dessinés dans le style de la collection ; `LOGOS_JSON` 64 → 85 entrées | ✅ v0.7.18 |

Vérification navigateur sur moteur mocké (réponses réelles capturées, zéro contact avec l'instance en cours) : 157 tests verts (contrats UI étendus aux nouveaux handlers), vérification visuelle complète (boutique, logos, filtre, tri, bascule installés).

---

## ✅ Lot 2026-10-30 — Réglages professionnels + contrat « tout bouton » — LIVRÉ (v0.7.16)

Audit complet au 2026-10-30 (base saine : 141 tests unitaires + 44/45 E2E verts, clippy 0 warning) → lot livré :

| # | Quoi | Statut |
|---|---|---|
| A1 | **Dialog Réglages à volets** (nav latérale 6 sections, dirty state + confirmation Esc/×/Annuler, validation par champ inline, labels humains, volet mémorisé) | ✅ v0.7.16 |
| A2 | **Section Notifications** — le moteur avait `set_notify` (toast + webhook) sans aucune UI : volet complet + action `test_notify` (résultat par canal) + `settings.notify` dans `/api/state` (jamais au gateway) | ✅ v0.7.16 |
| A3 | **`reset_runtime` exposé** dans Réinitialiser (bouton « Effacer » + confirmation) | ✅ v0.7.16 |
| A4 | **Contrat « tout bouton »** : tests UI (tout handler `on*` référencé défini, toute action ⊆ `manage::ACTIONS`), const `ACTIONS` partagée, tests `set_notify`/`test_notify` | ✅ v0.7.16 |
| A5 | **Polling UI armé sur `refresh_secs`** (fini le 5 s figé) | ✅ v0.7.16 |
| A6 | **Gateway durci + testé** : 23 tests unitaires (transport, surface, proxy) — déjà bien écrit, revu sans défaut trouvé | ✅ v0.7.16 |
| A7 | **Test E2E flottant** (`hostile_label`) : deadline 20 s → 45 s (45 moteurs en parallèle) | ✅ v0.7.16 |

Vérification UI réelle faite (moteur de test + navigateur) : volets, badge « non enregistré », erreurs inline, swap de plage, sauvegarde persistée, polling ré-armé, Tester par canal, Esc/confirm, reset runtime, volet mémorisé.

### Reste à faire (priorisé, repris des sections ci-dessous)
1. ~~**Split `manage.rs`** (3.1)~~ — **LIVRÉ** (7 commits, v0.7.20) : `manage.rs` 7 543 → **84 lignes** (racine + bridges) et **14 modules** purs (store, import, config, json, scans, peers, ssh, update, cache, autostart, litemcp, log, events, actions, tests) — API inchangée, réexports `pub(crate) use`. **Reste (facultatif)** : découper `actions.rs` (1 340 lignes) par famille en sous-fonctions.
2. ~~**Split `ui.rs` en assets servis** (3.2)~~ — **SPLIT LIVRÉ** (v0.7.19) : `/app.css` + `/app-boot.js` + `/app.js` servis, CSP `script-src 'self' 'unsafe-inline'`. **Reste** : fermer totalement `unsafe-inline` = réécrire les attributs `on*` des gabarits en délégation d'événements (data-act + un listener) — chantier UI dédié.
3. **Schéma unique des réglages** (3.3) : `SETTINGS_SCHEMA` consommée JS+Rust — la validation par champ livrée en A1 est JS-only, le Rust garde ses clamps ; unifier pour supprimer la duplication ui.rs ↔ manage.rs.
4. **Optimisations mesurées** (2.1-2.6) : borner `ssh_exec`, pool de workers `poll_all`, plafond `probe_http`, 304/compteur `rev`, slowloris — inchangées, toujours valides.
5. ~~**Sécurité** (4)~~ — **LIVRÉ** (v0.7.18) : `Host` validé (403 si hors allowlist, testé), nonce flood → éviction (`HELLO_NONCE_MAX` + TTL), **CSPRNG `random_hex`** (BCryptGenRandom / /dev/urandom — RandomState était corrélable entre appels successifs), `${esc(i.kind)}` (ui.rs), CI épinglée par SHA.

---

## 0. P0 — AVANT TOUT COMMIT — ✅ TERMINÉ (2026-10-26)

| # | Quoi | Statut |
|---|---|---|
| 0.1 | Test runtime (assertions inversées) | ✅ corrigé (session parallèle) |
| 0.2 | **P1 merge_duplicate_servers** : périmètre restreint aux entrées Discovered avec nom annoncé non vide, jamais manuel/seed/sans-nom + 2 tests de régression (`merge_ne_touche_jamais_les_entrees_manuelles_ou_sans_nom`, `merge_fusionne_les_doublons_resolus_meme_nom_meme_machine`) | ✅ corrigé + tests verts |
| 0.3 | **P2 ssh_discover** : surcharge conditionnelle (version/nom jamais écrasés par None), label rafraîchi seulement si fabriqué `ip:port`, skip disabled, `via_ssh` non forcé sur origin Manual | ✅ corrigé |
| 0.4 | fmt + 3 warnings clippy de la session parallèle (probe.rs `last_err` ×2, runtime.rs `field_reassign_with_default`) corrigés — `cargo clippy --all-targets -- -D warnings` : **0 warning**, tests : **121/121** | ✅ vert |

---

## 1. Tests de bout en bout — ✅ LIVRÉ (2026-10-26, 38 scénarios)

**Réalisé** : `tests/e2e/` — `main.rs` + `harness.rs` (Engine/EngineCfg/Resp/Fixture MCP en Rust pur, MCC_CONFIG isolé, CREATE_NO_WINDOW, fallback port +1..+9, Drop RAII, **verrou global `E2E_LOCK` : la suite passe aussi en mode parallèle**) + 5 fichiers de scénarios. **38 scénarios** (boot 8, sécurité 2, lifecycle 7, scan 5, actions/boutons 16 — surface complète du dispatcher : contrat de toutes les actions, set_settings+clamp, reorder, set_host, set_start_cmd, set_auto_restart, toggle_tool, import_mcp, install_store (validation sans installation réelle), ssh/pairs en échec gracieux, reset_state ×2, check_update borné, open_path, start/kill).

Points clés appris pendant l'écriture (à savoir pour maintenir) :
- la config de test DOIT lister les 8 seeds dans `removed_seeds` — sinon le moteur de test auto-lance les vrais serveurs MCP de la machine ;
- `autostart:false` (pollution registre HKCU) et `network_enabled:false` (scan LAN) obligatoires ;
- le harnais écrit le config.json APRÈS spawn pour les tests de redémarrage (retry ×3, port réécrit) ;
- `scan`/`discover` sont synchrones et peuvent dépasser le timeout client — deadlines larges (75 s) côté scan ;
- fait d'audit gelé en test : une requête SANS en-tête Host est actuellement acceptée (P3, scénario fuzz_lite).

| Phase | Contenu | Statut |
|---|---|---|
| 1. Boot | hello/racine (secret absent, CSP, nosniff), matrice auth loopback, matrice auth LAN, Host/Origin forgés → 403 | ✅ 8/8 |
| 2. Sécurité filaire | RCE sans code → 401 et aucun process ; preuve pair HMAC valide/rejeu/faux nonce ; fuzz-lite (413, TE→400, CL dupliqué→400, en-têtes 2 Mo, 64 connexions, pas de Host) ; label hostile jamais servi en HTML | ✅ 2/2 |
| 3. Cycle de vie | fixture HTTP → UP nom/version/tools ; stdio node inline → UP + pid ; toggle OFF tue le pid (Windows) ; toggle ON ; persistance + redémarrage moteur ; remove nettoie config | ✅ 7/7 |
| 4. Scan/fusion | scan trouve la fixture ; scan rejette `{"result":{}}` ; manuels même nom/2 ports survivent au scan (régression fusion) ; port mort jamais pruné ; double scan borné | ✅ 5/5 |
| 5. Actions/boutons | contrat dispatcher (chaque action → 200 + `ok` sans panic), + 15 scénarios réels listés ci-dessus | ✅ 16/16 |

**Hors CI** (marqué `#[ignore]`, exécution locale) : tout le SSH, le scan LAN réel, le webview Tauri. Budget : suite E2E ≤ 3 min, job CI ≤ 6 min. Contrôles d'instabilité : polling avec deadline (jamais de `sleep`), ports éphémères par bind-`:0`, assert sur ids exacts jamais sur des comptes.

**Quick win avant même le harness** : l'anti-smuggling de `src/lib.rs` (contrôle « anti request-smuggling » des en-têtes de cadrage — TE + CL dupliqués rejetés) n'a **aucun test unitaire dédié** (il est couvert en E2E par `fuzz_lite`, `tests/e2e/scenarios_security.rs`) — 5 lignes à ajouter pour le verrouiller aussi au niveau unitaire.

---

## 2. Optimisations (mesuré : 42 serveurs, 742 tools, page 119 Ko, UI poll 5 s, refresh 15 s)

### Structurales (par ratio gain/effort)

| # | Quoi | Pourquoi (chiffré) | Gain | Effort |
|---|---|---|---|---|
| 2.1 | **Borner `ssh_exec`** : `spawn()` + boucle `try_wait` + `kill` à échéance (~20 s) + `ServerAliveInterval=5` | P1 audit : 1 session SSH stallée = supervision morte jusqu'au redémarrage | ∞ → 20 s pire cas | ~1-2 h |
| 2.2 | **Remplacer la barrière `chunks(8)` de `poll_all`** par un pool de 10-12 workers (mpsc) + backoff exponentiel sur DOWN + tools/list re-téléchargé seulement si le serveur a changé (sinon tous les 15 s pour 742 tools !) | Cycle = Σ max(par chunk) + refresh → refresh effectif jusqu'à 39 s ; ~100 requêtes réseau/cycle | cycle ~2-25 s → ~1-4 s ; −60 % de requêtes | 3-4 h |
| 2.3 | **Plafonner `probe_http`** : deadline totale ~8 s partagée entre les 6 tentatives (2 hôtes × init/strict/discover), timeout réduit après le 1er échec | Un serveur loopback qui accepte sans répondre = **39 s dans un poll_one** | 39 s → ~10 s | 1 h |
| 2.4 | **Scan réseau** : garde `AtomicBool` (pas de ré-entrée), pré-filtre ARP (150 ms sur hôtes silencieux), mode incrémental (ne re-sonder que les IPs mortes), stacks 64 Ko | /24 = 253 threads × 201 ports × 400 ms ≈ 80 s pire cas, empilables par clics | 80 s → ~35 s ; rescan stable ~2-5 s | 2-3 h |
| 2.5 | **Protocole UI** : compteur `rev` atomique + `/api/state?rev=N` → 304 si inchangé ; page servie **précompressée** (`index.html.gz` en `include_bytes!`, 119 → 34 Ko) ; interval UI calé sur `refresh_secs` | UI poll 3× plus vite que les données ne changent ; re-render innerHTML complet à chaque tick | trafic + churn DOM −90 % | 2-3 h |
| 2.6 | Slowloris : échéance absolue d'en-têtes (~10 s) + cap de connexions par IP (8) | P2 audit : 64 sockets à 1 octet/29 s verrouillent tout | dispo sous client hostile | 1 h |
| 2.7 | Boot : `discover_peers` et `scan_network` en parallèle (séquentiels aujourd'hui) | image LAN complète à 40-110 s aujourd'hui | −30-40 s | 30 min |

### Quick wins (≤ 30 min chacun)
- **`save_cache` écrit 108 Ko toutes les 15 s pour rien** : le test d'égalité compare `saved_at` et `last_seen` qui changent à chaque poll → l'égalité n'est jamais vraie. Exclure ces champs de la comparaison.
- **`"scan"` synchrone dans le thread HTTP** (jusqu'à ~2,7 h sur 1..65535) → même traitement `thread::spawn` que `"scan_network"`.
- Sortir `add_log` du verrou `servers` (branche `DedupDecision::Replace` du scan, `src/manage/scans.rs`).
- `setInterval(load, 5000)` UI → armé sur `settings.refresh_secs` (−66 % de fetches sans rien changer côté serveur).

### À ne PAS optimiser (échelle 30-40 serveurs)
`state_json` rebuild (< 1 ms), recherches linéaires par id (O(42) = ns), clonage `save_config`, thread-par-connexion, `add_log` (rotation 512 Ko existe), mémoire des 254 threads de scan (réservation virtuelle seulement). Vouloir un `HashMap<id>` ou du keep-alive HTTP = complexité pour rien.

---

## 3. Refactorisation

### 3.1 `manage.rs` (6 443 lignes) → `src/manage/` (moves purs, API stable par ré-exports)

Le monstre : `handle_action` = **1 001 lignes** (26 commandes stringly-typed — dispatcher `pub fn handle_action` de `src/manage/actions.rs`), `poll_one` 300 lignes, `ssh_discover` 171, `scan_network_range` 169. Duplication mesurée : 78 `lockv(&state.servers)`, 17 `iter_mut().find(id)` (6 avec le même « serveur inconnu »), 22 `save_config` dont 15 suivis d'un `add_log`, **9 copies du littéral `Server { …25 champs… }`**, 2 blocs identiques de 12 lignes de parsing de plage de ports.

**Découpage** (un module par commit, `cargo test` vert à chaque étape, zéro changement de call-site grâce aux `pub use`) :
`mod.rs` (ré-exports) · `log.rs` · `config.rs` · `catalog.rs` (seeds + STORE 263 lignes) · `import.rs` · `poll.rs` · `scan.rs` · `peers.rs` · `ssh.rs` · `autostart.rs` · `update.rs` · `state.rs` · `actions.rs` · `tests.rs`.

**Ordre recommandé** : ① tests de caractérisation d'abord (ci-dessous) → ② moves purs → ③ helpers → ④ dispatch typé.

**Helpers qui tuent le plus de duplication** (après les fixes P1/P2, leurs corps vont être réécrits) :

| Helper | Remplace | Sites |
|---|---|---|
| `with_server_mut(state, id, f)` (lock + find + « serveur inconnu » + save + log) | dance find/save/log | 17 |
| `commit(state, level, msg)` = save + log | paire persiste+journalise | 15 |
| `Server::new_manual(...)` (20 champs constants remplis) | littéral 25 champs | 9 |
| `port_range_of(req)` | 2 blocs identiques 12 lignes | 2 |

**Dispatch typé** (phase finale) : `enum Action` + `from_req(&Value)` + structs de payload (`AddPayload`…) + 26 fonctions handler testables sans HTTP ; `handle_action` tombe à ~80 lignes. Filet : test de fumée sur les 26 noms d'action (l'usine `test_state()` existe déjà dans `src/manage/tests.rs`).

### 3.2 `ui.rs` (2 140 lignes = 1 seule raw string) → assets servis + CSP

- CSS → `GET /app.css`, JS → `GET /app.js` (`include_str!`, cache-busting `?v=PKG_VERSION`) → **CSP `script-src 'self'`** (ferme l'audit P3-10 : plus d'`unsafe-inline` qui transformerait une XSS future en lanceur de process sans code d'accès) ; l'anti-flash thème reste inline via hash `'sha256-…'`. Supprime aussi le piège `"#` des raw strings (documenté 2× dans le code).
- JS minimal : helper `api(action, payload)` (remplace ~15 wrappers copiés-collés), sections IIFE (Api/Store/Render/Settings/Dnd/CtxMenu). La conversion massive des `onclick` inline est **incrémentale et optionnelle**.

### 3.3 Menu Réglages — refactor dédié

État actuel : 7 champs sur un seul `POST set_settings` (le « set-row » du commit est une classe CSS, pas une API). Le défaut n°1 : **la validation est dupliquée JS/Rust** — le parseur de ports existe deux fois verbatim (validation `collectSettings` de `src/ui/app.js` ↔ branche `set_settings` de `src/manage/actions.rs`), les bornes refresh (min/max HTML ↔ clamp Rust), la whitelist `ui_bind` (options HTML ↔ whitelist Rust), les valeurs `reset_state.what`.

**Le fix central** : une `SETTINGS_SCHEMA` unique (id, label, kind, min, max, options, help) **consommée des deux côtés** — le JS construit/peuple/collecte le formulaire en boucle générique, le Rust valide en boucle générique avec les mêmes règles ; un test affirme que les deux tables couvrent exactement les mêmes 7 ids (`refresh_secs`, `scan_from`, `scan_to`, `network_enabled`, `network_ports`, `ui_bind`, `ui_port` readonly).

**UX à récupérer presque gratuitement avec le schéma** :
- plus de coercions silencieuses : « valeur corrigée : X » quand un clamp/swap/filtre a modifié la saisie (aujourd'hui la valeur tapée reste affichée, la corrigée n'apparaît qu'à la réouverture) ;
- `changed` renvoie des labels humains au lieu des clés internes (« ui_bind » dans le toast) ;
- suivi de dirty state (Esc/× jette silencieusement aujourd'hui), erreur par champ au lieu de la seule ligne `settingsMsg` ;
- documenter l'asymétrie du thème (localStorage navigateur, jamais envoyé au serveur).

### 3.4 `lib.rs` — table de routes en data
La chaîne de `if` de `pub fn route` (`src/lib.rs`) → `const ROUTES: &[(&str, &str, Handler)]` (~30 lignes modifiées, les 16 tests existants restent verts). Étape indépendante, faisable avant tout le reste.

### 3.5 Tests de caractérisation À ÉCRIRE D'ABORD (verrouiller le comportement avant de déplacer le code)
1. `dedup_decide` table-driven — **red-first** avec le fix P0-2 (None-vs-None → Add)
2. Snapshot de la forme de `state_json` (contrat Rust↔JS, ne doit pas dériver pendant le split UI)
3. Matrice d'auth exhaustive en table sur les 8 routes
4. Roundtrip config complet (tous champs Settings + un serveur par transport)
5. Contrat `set_settings` en table (clamps, swap, coercions, « ports vides = aucun changement »)
6. Fumée dispatch : les 26 actions sans panic + messages d'erreur exacts
7. Sémantique `remove`/`reset_state` par valeur de `what`
8. Contrat assets/CSP après split (`/app.js` référencé, `script-src 'self'`, code d'accès jamais servi)

---

## 4. Durcissement sécurité (renvoi audit 2026-10-26 — les fichiers d'audit ont été retirés du dépôt)

En Attente après P0 : slowloris (2.6 ci-dessus) · `Host` requis (400 sinon) · nonce flood (évincer le plus ancien au lieu de `clear()`) · `random_hex` en CSPRNG (`getrandom`/`BCryptGenRandom`) · `${esc(i.kind)}` (gabarits de `src/ui/app.js`) · release.yml : CLI npm épinglé + SHA256SUMS des artefacts · `rust-cache` réépinglé sur un SHA vérifiable (l'actuel est introuvable en amont, résolu seulement par le cache interne d'Actions) · `Cache-Control: no-store` sur le HTML · supprimer la page de secours morte `ui-dist/index.html`.

---

## 5. Reste à faire après les lots de correction (état au 2026-11-15)

Lots 0-5 (2026-11-13) et lots A (moteur), B (autostart + sécurité), C (interface), F (finition) livrés (rapports dans `correctifs-agents/` puis `restes-agents/`). La liste 2026-11-13 (5.1 à 5.15) est close : points **livrés** retirés (5.1, 5.2, 5.4, 5.5, 5.6, 5.8, 5.9, 5.11, 5.12, 5.14, 5.15), points **tranchés** (5.10 : `?access=` conservé, D5 = a ; `env` conservé dans le tool `install_store_server`, D6 = a), point **non reproduit** retiré (5.7) et point **corrigé à la racine** (5.13, fixtures `probe_http_*`).

Ce qui reste ouvert (nouvelle numérotation) :

| # | Quoi | Source |
|---|---|---|
| R1 | **`kill` / `stop_server` tue le process qui écoute sur le port sans vérifier le pid enregistré** — peut tuer un tiers qui a pris le port (observé : une fixture node tuée à la place du serveur visé) | A §4, C §4 |
| R2 | **La découverte (⟳ / `discover`) sonde TOUS les ports en écoute de la machine** — en test, elle importe les serveurs réels de l'utilisateur et les annonces LiteMCP de `%TEMP%` dans la config de l'instance | A §4 |
| R3 | **Adoption automatique du registre LiteMCP** : `poll_all` importe les annonces du registre local à chaque cycle et écrit la configuration quand une annonce s'applique | A §4 |
| R4 | **Actions sans chemin de refus** (`reorder`, `scan_network`, `discover_peers`, `discover`, `set_autostart`) : toujours `ok:true`, aucun motif d'échec à afficher côté interface | C §4 |
| R5 | **Aucune vérification d'empreinte à la mise à jour** : le téléchargement direct ne compare ni SHA-256 ni signature (le README documente pourtant `SHA256SUMS.txt` à la main) | TODO-RESTES, lot D |
| R6 | **Flake de fond (produit)** : la découverte locale périodique sonde tous les ports en écoute et sature le pool de 12 sondes — une action `scan` synchrone peut attendre > 30 s (contourné dans le harnais par un délai de lecture à 60 s). À arbitrer : borner aux `network_ports` ou rendre la découverte non bloquante | F §4 |
| R7 | `set_autostart` reste sans garde d'appartenance : appelé depuis une instance de développement, il réécrit l'entrée de l'installation réelle (comportement voulu « geste explicite », à revoir si l'on veut une confirmation côté interface) | B §4 |
| R8 | `install_store_server` peut rendre `state:"DOWN"` quand `npx` est lent au premier spawn alors que le process démarre (cosmétique, se redresse au cycle suivant) | A §4 |
| R9 | **Le bouton Installer de la boutique n'a pas de champ pour les variables d'environnement** : `env` n'est disponible que par l'API locale (`install_store`) et le tool `install_store_server` | A §1 point 6, `src/ui/app.js` (`installStore`) |
| R10 | `recorded_run_value()` dépend du format texte de `reg query` (défaut sûr : entrée classée « ailleurs » donc jamais touchée, mais cassant si Microsoft change la sortie) | B §4 |
| R11 | Le scénario E2E `scenarios_actions` appelle `set_autostart{enabled:false}` : sur une machine où l'entrée « MCP Control » existe, il la supprime (suite non neutre pour le registre) | F §4, B §4 |
| R12 | `src/ui/app-boot.js` : fins de ligne mixtes préexistantes, couvertes par aucun test | C §4 |
| R13 | Tolérance `n <= 2` de `probe_http_port_muet_une_seule_forme_envoyee` conservée (la cause réelle des flakes est corrigée dans les fixtures) | F §4 |
| R14 | **Passe navigateur du lot C faite le 2026-11-15** (rapport `G-navigateur.md`) : toasts, bandeau et reprise conformes ; **non couverts** : le glisser-déposer (pas de drag dans le volet) et les largeurs 900 / 1 280 px (volet figé à 440 px) | G §1 |
| R15 | **Refaire les trois captures du README à la main** : `assets/readme-accueil.png`, `assets/readme-detail.png`, `assets/readme-rust.png` (la dernière est datée du 2026-10-26, cinq jours avant les autres) | TODO-RESTES, « À faire à la main » |
| R16 | **Le bandeau « moteur injoignable » arrive à ~20 s, pas 17 s** : chaque requête en échec coûte ~2 s, donc 15 s de cycle + 2 s + 2 s + 2 s. Mesuré 21,1 s après le dernier état reçu (19,6 s après la panne). À arbitrer : compter le 1er échec comme suffisant si la requête a duré > 1 s, ou réessayer sans délai après un échec de transport | G §1 point 2, G §2 (G1) |
| R17 | **`sshForget` sur un pair inconnu rend un toast « échec »** (`{ok:false}` sans champ `error`) : même famille que R4, alors que `kill` / `remove` ont reçu leur motif | G §2 (G5) |

<!-- Ligne du 2026-11-13 retirée le 2026-11-15 (périmée) :
| 5.5 | `install_store` n'accepte pas de `env` depuis la requête (complément naturel pour les items à token : GITHUB_TOKEN…) | correctifs 03 §4 | -->

---

## Ordre d'exécution recommandé

```
P0 (delta vert + fmt + commits)                       ~1 j
→ Caractérisation tests (3.5)                          ~1 j
→ Fixes audit P1 hang (2.1) + kill_tree Unix + verrou children  ~1 j
→ E2E phases 1-2 (sécurité filaire)                    ~2 j
→ UI asset split + CSP (3.2)                           ~1-2 j
→ Optimisations 2.2-2.5                                ~2 j
→ manage/ split (3.1) → route table (3.4) → helpers → dispatch typé  ~3-4 j
→ Réglages schéma (3.3) + UX                           ~1-2 j
→ E2E phases 3-4 (cycle de vie, scan)                  ~1,5 j
```

Chaque bloc compile, passe les tests, et se livre indépendamment.
