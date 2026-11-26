## [0.8.10] — 2026-11-25

### Sécurité & authentification (Audit complet v0.8.9 — Lots 1 & 2)
- **Confirmation humaine imposée sur le pont hybride `POST /mcp/s/<id>` (`src/gateway.rs`)** : `serve_hybrid_stdio` passe désormais par `auth::confirm::decide` avant `tools/call`, `resources/read` et `prompts/get` (`403` JSON-RPC `-32023` `approval_pending` / `approval_denied`) ; en-têtes SEP-2243 `Mcp-Method` et `Mcp-Name` rendus obligatoires en mode signé.
- **Anti-usurpation d'aperçu (`Confirmation Spoofing`) & anti-TOCTOU (`src/auth/confirm.rs`)** : `add_server` avec `program` affiche toujours `kind: "lancement"` même en `transport: "http"`, `import_config` partage exactement `parse_servers_doc` (incluant `cwd` et marqueur d'omission d'arguments), et le `digest` des permis `window` 10 min lie désormais `(args, effet)` avec l'endpoint résolu.
- **Sessions locales scellées & anti-squat (`src/auth/session.rs`, `src/auth/cli.rs`, `src/lib.rs`, `src/main.rs`, `desktop/src/main.rs`)** : port loopback réellement écouté scellé dans `mcc-session-<id>.grant`, capacités IPC Tauri conditionnées à la preuve `.grant` en mode `reused`, réserve dédiée `MAX_EPHEMERAL_SESSIONS = 4` pour que `mcp-control auth revoke` fonctionne même avec 8 fenêtres UI ouvertes, et contrôle du plafond avant consommation du jeton bootstrap.
- **Anti-force brute (`src/auth/throttle.rs`), TLS & trousseau (`src/auth/net.rs`, `src/auth/registry.rs`, `src/auth/secret.rs`, `src/audit.rs`, `src/crypto.rs`, `src/auth.rs`)** : éviction corrigée (`bloque_jusqua > now`), conservation de la fenêtre d'échecs IP sur succès d'une clé secondaire, normalisation IPv6 `/64`, DA5 strict sur appels signés sortants hors loopback (`tls_fp` obligatoire), validation `valid_keyid` dans `server_unpair`, nettoyage des anciens `.pin.json` au ré-appairage, `"public_key"` écrit par `auth pair`, validité `server.crt` bornée à 365 j, détection de troncature HTTPS `Content-Length`, `last_used_at` mis à jour uniquement après signature Ed25519 valide avec persistance horaire, `verify-log` interrogeant le trousseau OS, `Host` IPv6 crocheté en CLI, `ct_eq` durci (`black_box`) + `zeroize_slice`, `admit_conn` fail-closed + `to_canonical().is_loopback()`, `ADMIN_ACTIONS` complété (31 actions), suppression du double spawn `icacls.exe`, et rotation atomique de `auth-audit.log` via `.rotate.tmp` (anti-verrouillage `[H-05]` sur `ENOSPC`).

### Moteur, Desktop, Bridge & Performances
- **Desktop Tauri & auto-update NSIS (`desktop/src/main.rs`, `src/manage/update.rs`, `desktop/nsis/installer-hooks.nsh`, `src/ui/app.js`)** : récupération transparente de session au retour du tray / `single_instance` via `window.__mccRecoverSession`, priorité loopback (`127.0.0.1`, `::1`) dans `probe_hosts`, `local_aux_listener` activé pour `ui_bind == "::"` sous Windows, et exécution de `shutdown_engine()` avant le lancement de `setup.exe /S` (+ délai silencieux `NSIS_HOOK_PREINSTALL`).
- **Bridge, Probe, Notify & Release (`src/bridge.rs`, `src/probe.rs`, `src/notify.rs`, `src/manage/scans.rs`, `src/manage/json.rs`, `src/gateway.rs`, `Cargo.toml`)** : lecture `stdin` bornée à 32 Mio et rechargement auto de clé sur `401` dans `bridge`, matching exact de port `u16` sur colonne locale (`ss`/`lsof`/`netstat`), blocage des chemins UNC à slashs mixtes (`\/server\share`), idempotence IPv6 `Host` + port `> 0`, `child.wait()` anti-zombie dans `probe_stdio`, `ProbeHint` activé pour les serveurs `UP` à 0 outil, `gateway::fingerprint` complet sur noms/descriptions/désactivés, `now_compact()` monotone, et `lto = "thin"` + `strip = "symbols"` en release.

### Interface UI & Boutique
- **UI (`src/ui/app.js`, `src/manage/actions.rs`)** : correction de `hasAuthEnv(s)` sur tableau JS (badge `AUTH` et retrait env rétablis), actions batch atomiques `set_all_tools` et `refresh_all` (1 requête HTTP au lieu de rafales N+1), et suivi asynchrone propre sur `sshBootstrap` et `doScanNet`.
- **Boutique (`src/manage/store.rs`, `src/manage/actions.rs`, `src/ui/app.js`, `src/ui/app.css`)** : correction de l'installation PostgreSQL (fin de la duplication d'arguments), rejet strict de tout flag `-*` dans `extra_args`, `workdir` dédié `<config_dir>/data/<id>` (fix `sqlite` et `at-memex` en autostart), logos `gh-*` corrigés, recherche Boutique enrichie + cache `OnceLock` sur `store_json()`, masquage des champs `env` sur les cartes dès qu'il y en a plus de 2 (demandés à l'installation en modale scrollable), et cartes Boutique compactées (`minmax(205px, 1fr)`, 2 lignes de description) pour afficher plus de MCP par écran.

## [0.8.9] — 2026-11-24

### Sécurité & authentification
- **Pont hybride `/mcp/s/<id>` durci (`src/gateway.rs`, `src/auth/confirm.rs`, `src/audit.rs`)** : vérification SEP-2243 (`Mcp-Method`/`Mcp-Name`), liste blanche stricte des méthodes JSON-RPC, réponses `202` sur les notifications sans `id`, et journalisation complète HMAC-SHA256 (`auth-audit.log`) + `confirm::note_hybrid_decision`.
- **Échec fermé du trousseau & intégrité du journal d'audit (`src/auth/secret.rs`, `src/audit.rs`, `src/auth/cli.rs`)** : refus de régénérer une identité serveur ou une clé HMAC d'audit quand le trousseau OS est verrouillé alors qu'une clé publique ou des segments d'audit existent déjà ; vérification HMAC de la dernière ligne dans `Journal::open`, détection des segments supprimés dans `audit::verify` et `verify-log`, `restrict_permissions` à la création/rotation, et correction de `premier_secret_du_fichier` (`auth fingerprint`).
- **Frontière locale & `open_path` (`src/lib.rs`, `src/auth.rs`, `src/manage/actions.rs`)** : exigence `peer_is_loopback` sur les sessions locales et sur tout `POST /api/action`, élargissement d'`ADMIN_ACTIONS` aux actions locales sensibles (27 actions), blocage des chemins UNC Windows dans `open_path` et `ssh_path_safe`, `open -R` sous macOS et `--` sous `xdg-open`.
- **Confirmations, registre & cadrage HTTP (`src/auth/confirm.rs`, `src/auth/registry.rs`, `src/auth/throttle.rs`, `src/auth/pair.rs`, `src/lib.rs`, `src/manage/config.rs`, `src/manage/log.rs`)** : alignement de `programmes_importes` sur le parseur `import`, aperçus complets `effect_of`, purge immédiate des permis (`confirm::purge_client`) à la révocation/rotation/changement de droits, application de `expires_at` et mise à jour bornée de `last_used_at`, bornage des tables `throttle`/`hello_rate_slot`/appairages par IP, cadrage HTTP strict RFC 9112 (`413` immédiat, corps tronqué `400`, `find_sub` O(N), `write_timeout`), écriture atomique restreinte avant `rename`, et assainissement ANSI + verrou fichier sur `mcc.log`.

### Desktop Tauri, CLI, Bridge TLS 1.3 & Auto-Update
- **Distribution Desktop/CLI unifiée (`desktop/src/main.rs`, `src/manage/config.rs`, `src/manage/json.rs`, `src/ui/app.js`)** : support natif des sous-commandes `auth` et `bridge` dans `mcp-control-desktop`, résolution partagée de `config.json` (`CARGO_MANIFEST_DIR` en debug uniquement + repli `%LOCALAPPDATA%`/`~/Library/Application Support`/`~/.config`), et génération de la commande exacte (`bridge_command`) dans *Conf agent*.
- **Bridge TLS 1.3 épinglé (`src/bridge.rs`)** : prise en charge `https://` avec vérification `PinnedServer`, normalisation loopback (`localhost`/`127.0.0.1`/`::1`), et refus de repli HTTP en clair lorsqu'une empreinte TLS est épinglée.
- **Sessions CLI, cycle de vie des processus & IPv6 (`src/auth/session.rs`, `src/auth/cli.rs`, `src/lib.rs`, `desktop/src/main.rs`, `src/probe.rs`)** : `POST /api/session/close` + sessions éphémères CLI fermées à la sortie, canal fichier `mcc-session-<id>.req` sans course, garde `--hidden` et déplacement de `run_headless()` après `single_instance`, arrêt propre des enfants à la fermeture (`shutdown_engine`), Job Object Windows (`KILL_ON_JOB_CLOSE`), terminaison Win32 `CreateToolhelp32Snapshot`, `process_group(0)` Unix, et adressage IPv6 (`[::1]`) / écouteur auxiliaire loopback.
- **Mise à jour automatique, progression en direct & autostart (`src/manage/update.rs`, `src/manage/json.rs`, `src/manage/autostart.rs`, `src/ui.rs`, `src/ui/app.css`, `src/ui/app.js`)** : dossier de staging privé `update-staging/`, verrou `AtomicBool`, `--proto =https --tlsv1.2`, attente PID NSIS réelle (`MCC_UPDATE_WAIT_PID`), comparaison SemVer pré-versions, suivi temps réel de la mise à jour (`install_update` asynchrone + `update_status` dans `/api/state`, octets téléchargés / taille totale `Content-Length`, bandeau `#updateBanner` + barre de progression dans *Réglages* et reconnexion/rechargement au redémarrage), suppression idempotente de l'autostart sous Windows et échappement XML/Desktop.

### Sondes, SSH, Supervision, MCP Stateful & Boutique
- **Durcissement & portabilité SSH (`src/probe.rs`, `src/manage/peers.rs`)** : routage de `ssh_exec_target` via `ssh_exec_stdin` borné + `ServerAliveInterval`, interdiction du `-` initial et séparateur `--` avant `user@host`, fichier temporaire aléatoire `mktemp`, détection `netstat -an -p tcp` sous macOS, et réécriture de `0.0.0.0`/`::` dans `import_peer`.
- **Supervision & cycle de vie (`src/manage/scans.rs`, `src/manage/events.rs`, `src/manage/actions.rs`, `src/manage/litemcp.rs`)** : re-vérification existence/`enabled` avant publication de `poll_one_inner`, arrêt des lanceurs HTTP expirés, persistance `auto_restart = false` sur `kill`/`stop_server_force`, synchronisation `Transport::Stdio` dans `set_start_cmd`, `reset_spawn_retry`, ordre des verrous `servers`→`runtime`, cooldown & toasts desktop non bloquants, persistance `save_cache` en fin de worker, suivi `confirmed_up` anti-faux crash au boot et notifications sur crash `stdio`.
- **MCP Stateful, réseau & Boutique (`src/gateway.rs`, `src/probe.rs`, `src/manage/import.rs`, `src/manage/scans.rs`, `src/store_github.rs`)** : `ProxyTarget::Http` routé via `call_http_tool` (`Mcp-Session-Id` + `notifications/initialized`), clé `_meta` corrigée, pagination `nextCursor`, `inputSchema` exposé dans `list_server_tools`, résolution DNS dans `socket_addr` et `SELF_LISTEN` en `RwLock`, plafond `256 KiB` sur `http_request_trusted`, garde SSRF sur `set_host`, plafond `SCAN_MAX_PORTS` sur `scan_network`, restauration de `tools_disabled` dans `config::from_json`, correction des parseurs `import.rs` / `chunked` / SSE / priorité `.exe`, nettoyage de 5 entrées non-MCP dans `store_github.rs` (`89 → 84`), flags dangereux bloqués + interpolation `${VAR}` en Boutique et prise en charge `uv` à la désinstallation.
- **Boutique : 11 nouveaux serveurs (`src/manage/store.rs`, `src/logos.rs`)** : Jira & Confluence (`mcp-atlassian`), Obsidian, Desktop Commander, Windows MCP (`uvx --python 3.14 windows-mcp serve`), YouTube Transcript, Tavily Search, Semgrep (`semgrep mcp` via uvx), Grafana (`mcp-grafana`), ClickHouse (`mcp-clickhouse`), Qdrant et Airtable — commandes vérifiées sur npm/PyPI et les README officiels, logos dédiés, variables d'environnement déclarées par article (`84 → 95`).

### Interface UI, CI/CD & Tests E2E
- **UI & réactivité (`src/lib.rs`, `src/ui.rs`, `src/ui/app.css`, `src/ui/app.js`, `src/ui/app-boot.js`, `ui-dist/index.html`, `ui-dist/boot.js`)** : réglages de luminosité (60–140 %) et de palette de couleur des écritures dans *Réglages → Général* (10 nuances calibrées Sombre/Clair, mémorisation locale sans flash ni style en ligne CSP), réveil du poller sur `set_settings`, garde `_detailReqSeq` dans `load()`, confirmation `needs_confirm` dans `toggleAutostart()`, préservation d'un `ui_bind` personnalisé et des serveurs filtrés lors du tri `Alt+↑/↓`, extraction du script inline de `ui-dist/index.html` vers `ui-dist/boot.js`, et libellé `auth-audit.log (HMAC-SHA256)`.
- **CI/CD & E2E (`.github/workflows/release.yml`, `deny.toml`, `tests/e2e/harness.rs`)** : épinglage `@tauri-apps/cli@2.5.0`, garde NSIS Windows obligatoire, SBOM `--all-features` et glob `dist/mcp-control*`, `[graph]` multi-targets dans `deny.toml`, et nettoyage RAII des dossiers temporaires E2E.

## [0.8.8] — 2026-11-23

### Audit & optimisation complète de l'interface (WCAG 2.2 AA, Dark & Light)
- **Bouton Accueil fonctionnel (`#tbHome`, juste à droite de `#tbExplToggle`)** : retour complet à l'état Dashboard initial (fermeture des modales/menus ouverts, réinitialisation sélection/filtres/pilules/scroll, `data-tauri-drag-region="false"` + `pointer-events:none` sur les éléments internes sous Tauri v2, protégé par `bouton_accueil_barre_titre_ramene_au_dashboard_initial`).
- **Formatage JSON & inspecteur d'outils** : introduction de `escPre` et réécriture de `hlJson` en tokenizer à un seul passage dans `src/ui/app.js` pour afficher schémas `inputSchema`, résultats `call_tool`, blocs `Conf agent` et zones d'arguments `.tool-args-ta` sur plusieurs lignes avec coloration syntaxique propre (zéro `<span>` imbriqué), boutons `Formater`, `Copier le résultat`, `#toolSearchClear` et correctif `toggleTool()` dans `#toolsRowsBox`.
- **Thèmes unifiés Dark/Light & contrastes WCAG 2.2 AA (`0` échec)** : consolidation de `src/ui/app.css` via jetons `:root` / `:root[data-theme="light"]`, suppression des `18 !important` et conflits de spécificité (`.primary`, `.btn-ghost`, `.favbtn.on`, `.autobtn.on`, `.exp-pill.sel`, `.set-nav-btn`, `#btnUpdInstall:disabled`), contraste relevé (`--border` Dark `3.79:1`, `.sw.on` Light `8.34:1`) et filtres logos Boutique adaptatifs par thème (`.logo-on-dark` / `.logo-on-light`).
- **Ergonomie, responsive & accessibilité clavier (`src/ui.rs`, `src/ui/app.css`, `src/ui/app.js`)** : lien d'évitement `.skip-link`, pile `_focusStack` et gestion `Échap`/backdrop sur les 7 `<dialog>`, `#toast` en Top Layer (`popover="manual"`), ancrage droit `#discoverMenu` à `940 px`, grille KPI `4/2/1`, `.d-head-endpoint` copiable, Journal chronologique avec filtres (`Tous`/`Alertes`/`Erreurs`) et hauteur étendue, compteurs Boutique, et bouton de mise à jour rétabli dans la barre de titre (`0e1873d`).

### Sécurité & mode hybride garanti pour toute la Boutique (89/89 serveurs)
- **Mise à jour de la Boutique (`draw.io` ajouté, `MetaTrader MCP` & `Filesystem` retirés, suppression complète)** : ajout de **draw.io (`@drawio/mcp`)** (catégorie *Design*, `npx -y @drawio/mcp`) avec son logo SVG dédié dans `src/logos.rs` et validation du pont hybride `/mcp/s/store-drawio` ; retrait de **MetaTrader MCP (`gh-metatrader`)** et **Filesystem (`filesystem`)** ; nouveau bouton **Supprimer** avec confirmation (`askConfirm`) sur les cartes déjà installées (`uninstall_store`), arrêtant le processus et supprimant configuration, cache d'état et fichiers/caches locaux (`npx`/`uvx`).
- **Blocage de la configuration `stdio` directe quand le mode hybride est actif** : `hermesYaml()`, `mcpJson()`, `claudeCmd()` et l'onglet *Conf agent* (`confAgent()`) ne publient plus jamais de commande `stdio` directe (`npx`/`uvx`/binaire) et affichent le bandeau de verrouillage `.conf-hybrid-lock` imposant le pont signé `mcp-control bridge --url …/mcp/s/<id>` (champ API `direct_stdio_blocked` dans `server_json` + test de régression).
- **Pont stdio & passerelle hybride `/mcp/s/<id>` (`src/bridge.rs`, `src/gateway.rs`, `src/lib.rs`, `src/probe.rs`)** : `parse_bridge_url` et `construire_requete` supportent désormais `--url http://…/mcp/s/<id>` (signature Ed25519 RFC 9421 + ligne POST sur le chemin exact), routage strict `/mcp/s/<id>` avec rejet HTTP `400` sur chemin mal formé, support de `ProxyTarget::Stdio` dans `call_server_tool`, vérification process vivant & état `DISABLED` dans `serve_hybrid_stdio`, pagination `tools/list` (`nextCursor`) dans `probe_stdio` et délais `300 s` pour `uvx --from git+…` (`src/manage/scans.rs`), couverts par tests de régression sur les **89/89** serveurs de la Boutique.

### Fiabilité `stdio` & premier lancement `npx`
- **Démarrage `stdio` plus fiable (`3f2bd3f`)** : remontée des véritables erreurs de démarrage de processus, délai du handshake adapté (jusqu'à 5 min) pour les premières installations `npx`, et correction Clippy docstring dans `src/manage/scans.rs`.

### Conteneurs Docker & guide d’appairage
- **Indicateur conteneur Docker / Podman (`src/probe.rs`, `src/manage/*`, `src/ui/app.js`, `src/ui/app.css`)** : détection automatique des serveurs MCP exécutés en conteneur (`docker`/`podman`/`nerdctl` & wrappers, `docker ps`, proxys de conteneur, cgroups Linux, sondes SSH, cache & import pair) et affichage du logo baleine Docker dans la barre de l’explorateur et le panneau détail.
- **Guide d’appairage `Accès` (`src/ui/app.js`)** : libellé mis à jour en `Hermes / Grokbot / Claude Code / Cursor`.

## [0.8.7] — 2026-11-22

### Thème sombre par défaut & boutons obsidienne-graphite
- **Mode sombre toujours par défaut** : suppression du basculement automatique sur `prefers-color-scheme: light` dans `src/ui/app-boot.js` ; MCP Control démarre désormais en mode sombre par défaut (le mode clair reste accessible via *⚙ Réglages*).
- **Boutons primaires & contrôles actifs obsidienne-graphite** : remplacement des fonds blancs trop vifs en mode sombre (`.primary`, `.sb-upd`, `.scat.sel`, `#storeHideInst.sel`, cases cochées et interrupteurs) par une teinte graphite-obsidienne satinée (`#323744 → #262a34`, texte `#f0f2f6`, bordure fine lumineuse).

## [0.8.6] — 2026-11-22

### Correctifs
- **Déplacement de fenêtre rétabli (cause racine)** : l'intercepteur `window.fetch` de `src/ui/app.js` recopiait les en-têtes via `Object.assign`, ce qui vidait les `Headers` de l'IPC Tauri (`Tauri-Invoke-Key` absent → `500`, plus de drag / réduire / agrandir / fermer). Les requêtes hors `/api/*` passent désormais telles quelles, et `/api/*` utilise `new Headers()` + `.set()`.

## [0.8.5] — 2026-11-22

### Refonte visuelle Studio Graphite & Platinum
- **Suppression complète des bleus/indigos vifs** : remplacement intégral par une palette exécutive **Studio Graphite / Platinum** (thèmes sombre et clair, contraste WCAG 2.2 AA), réservant la couleur exclusivement aux états sémantiques (émeraude feutré `UP`, ambre `WARN`, corail `DOWN`).
- **Boutons primaires, barre de titre & contrôles** : CTA primaires monochromes haute lisibilité avec relief d'arête subtil et micro-retour tactile, barre de titre graphite brossé, onglets et filtres harmonisés.
- **Explorateur, Dashboard, Détail & Inspecteur d'outils** : filet de sélection net de `2px` sans halo, séparateur `#grip` de précision `1px`, barre de révélation discrète sur les outils, coloration syntaxique JSON éditoriale adoucie et badges `HYBRIDE · HTTP + AUTH` / `AUTH ENV` raffinés.
- **Modales, Boutique, Centre d'accès, Menu contextuel & Toasts** : harmonisation complète de `<dialog>`, des cartes Boutique, du code d'appairage `.acc-code`, de `#ctxmenu` et de `#toast` en stricte conformité CSP (`'self'`, zéro style en ligne).
- **Filet de sécurité drag fenêtre (Windows / Tauri)** : écouteur `mousedown` en phase bulle sur `window` dans `src/ui/app.js` prenant le relais uniquement si `drag.js` natif n'a pas déjà intercepté le clic.

## [0.8.4] — 2026-11-22

### Pont hybride Streamable HTTP + Auth LiteMCP Ed25519 pour serveurs `stdio`
- **Couche Streamable HTTP native (`POST /mcp/s/<id>`)** : chaque serveur `stdio` supervisé par MCP Control (notamment les serveurs installés depuis la Boutique comme Mermaid, Filesystem, GitHub…) est automatiquement exposé en HTTP Streamable (JSON + SSE) sans ouvrir de port supplémentaire, protégé par la même signature **Ed25519 RFC 9421 + RFC 9530** que `POST /mcp` (`lecture` pour `initialize`/`ping`/`tools/list`, `execution` pour `tools/call`, filtrage strict des outils désactivés).
- **Canal JSON-RPC persistant & test d'outils en direct** : `StdioHandle` maintient le pipe `stdin/stdout` vivant (`StdioRpc`), débloquant l'exécution en direct dans l'inspecteur d'outils pour les serveurs `stdio` UP.
- **Délai `npx`/`uvx`/`bunx` porté à 45 s** : évite le timeout à 15 s lors du premier téléchargement de paquet (ex. `npx -y mcp-mermaid`).

### Logos Boutique dans l'interface
- **Logos conservés partout dans l'UI** : préchargement de `/api/store-logos` au démarrage et affichage du logo SVG de marque dans la liste des serveurs et l'en-tête de détail pour tous les serveurs issus de la Boutique (`store-*`), accompagné du badge `HTTP+AUTH` et du bloc **Conf agent** hybride.

### Correctif Windows (déplacement de la fenêtre)
- **Drag natif Tauri 2.12 sans course** : passage de `#titlebar` à `data-tauri-drag-region="deep"`, suppression du listener `mousedown` dupliqué qui appelait `start_dragging` deux fois en concurrence avec le `drag.js` natif de Tauri, et élargissement de `CapabilityBuilder` à `http`/`https` sur tout loopback (`127.0.0.1`, `localhost`, `[::1]`).

## [0.8.3] — 2026-11-22

### Authentification MCP Control sur les sous-serveurs (clic droit)
- **Menu contextuel Auth Ed25519 (RFC 9421)** : clic droit sur un serveur HTTP pour l'appairer en 1 clic (`server_pair` + copie automatique de `LITEMCP_TRUSTED_KEYS=…`), voir son statut actif (`Auth MCP Control active ✓`), copier la clé publique appairée ou l'identité Ed25519 du serveur, ré-appairer ou retirer l'appairage (`server_unpair`).
- **Injection d'environnement pour serveurs gérés / `stdio`** : nouvelle action locale `server_auth_env` permettant d'injecter ou retirer `MCC_AUTH_KEYID`, `MCC_AUTH_PUBKEY` et `LITEMCP_TRUSTED_KEYS` dans la commande de lancement (badge `AUTH` visible sur le serveur).
- **Test direct d'outils sur serveurs appairés** : les serveurs verrouillés mais appairés (`paired`) autorisent désormais l'exécution d'outils dans l'inspecteur en direct.

### Interface & ergonomie
- **Zone Serveurs & barre d'actions refondues** : explorateur élargi (`320px`), cartes `.srow` aérées sur 2 lignes (état/nom/compteur/switch en haut, métadonnées en pilules + actions rapides séparées en bas) et en-tête de détail structuré (endpoint en pilule, ruban `.d-meta-strip`, barre `.d-actions` divisée en groupes logiques avec séparateurs verticaux).
- **Palette neutre graphite + indigo** (inspiration Antigravity / Claude, sombre & clair, contraste WCAG 2.2 AA).
- **Scroll Boutique & modales corrigé** : défilement fluide dans `#storeDlg` et tous les `<dialog>` via layout flex-colonne + `.dlg-body` défilant.
- **Inspecteur de schéma & testeur d'outils en direct** : cliquer sur un outil déplie son schéma JSON (`inputSchema`), le tableau de ses paramètres, un modèle JSON pré-rempli, la copie du payload `tools/call` et l'exécution directe sur les serveurs HTTP UP (`call_tool`).

### Correctif Windows (fenêtre desktop)
- **Déplacement de la fenêtre Tauri v2 rétabli** : portée `CapabilityBuilder::remote` étendue (`/*`), `data-tauri-drag-region` posé sur les enfants non interactifs de la barre de titre et gestionnaire `mousedown` / double-clic sur `#titlebar`.

## [0.8.2] — 2026-11-22

### Correctif Windows
- **Zéro flash de fenêtre console (`icacls`/`reg`/`curl`)** : application de `CREATE_NO_WINDOW` dans `probe::system_command`, supprimant les 5 fenêtres console qui s'ouvraient brièvement lors d'un arrêt serveur ou *Tout couper*.

## [0.8.1] — 2026-11-22

### Boutique, navigation & mise à jour automatique
- **28 nouveaux MCP (`@tom_doerr`)** ajoutés à la Boutique avec leurs logos SVG dédiés.
- **Menu « Découverte ▾ »** regroupant *Scanner les ports*, *Scanner le réseau* et *Machines* ; modales *Réglages* et *Accès* agrandies en quasi plein écran.
- **Auto-mise à jour vérifiée SHA-256** : téléchargement de l'installateur NSIS, vérification de son empreinte contre `SHA256SUMS.txt`, exécution silencieuse (`/S`) et relance automatique de MCP Control.

## [0.8.0] — 2026-11-21

> **Guide de migration `0.7.x → 0.8.0`** : voir [`docs/migration-0.8.md`](docs/migration-0.8.md) et la spécification [`docs/auth.md`](docs/auth.md).

### Authentification & passerelle signée (`M1`–`M5`, `litemcp = "0.12.0"`)
- **Signatures Ed25519 par requête sur `POST /mcp`** (RFC 9421 + RFC 9530 `Content-Digest` SHA-256, nonce unique, fenêtre ±60 s, cache borné à 4 096 nonces/clé) : même en loopback, un process local sans identité reçoit `401`. Dépendance `litemcp = "0.12.0"` publiée sur crates.io.
- **Trousseau système (`keyring = "=4.2.0"`)** : les clés privées (`server-ed25519`, `server-tls`, `audit-hmac`, clients sortants) vivent dans le trousseau OS (Windows Credential Manager, macOS Keychain, Linux Secret Service) avec repli fichier `0600`/ACL journalisé. `config.json` ne stocke plus aucune clé privée.
- **Moindre privilège (3 droits + `admin` local)** : les 30 outils du gateway sont répartis en `lecture` (10), `pilotage` (10) et `execution` (10), avec filtrage strict de `tools/list`. Les actions d'administration (`pair_approve`, `client_revoke`, `client_rotate`, `server_pair`, réglages) restent exclusivement sur la fenêtre locale.
- **Confirmation humaine (`M3`)** : chacun des 10 outils `execution` (y compris `call_server_tool`) crée une demande visible dans **Accès → Confirmations** avec le programme et les arguments exacts liés par SHA-256 (*Une fois*, *Permis 10 min* révocable ou *Refuser*). File bornée à 32 demandes, expiration à 2 min.
- **TLS 1.3 obligatoire hors loopback (`M4`, `--features tls`)** : certificat auto-signé persisté (`rustls 0.23.46`, sans TLS 1.2) et épinglage `tls_fp` vérifié dans le code de comparaison à 6 chiffres lors de l'appairage (`mcp-control auth pair`). Sans la feature `tls`, l'écoute hors loopback est refusée et reste en `127.0.0.1` avec alerte dans le journal et l'interface.
- **Pairs migrés & serveurs LiteMCP verrouillés (`M4`)** : les pairs sont sondés via `POST /mcp` signé en `lecture` (repli transitoire `X-MCC-Access` lecture seule / `X-MCC-Proof` conservé en `0.8.x`, `?access=` supprimé). Les serveurs LiteMCP répondant `401 Signature` affichent un badge cadenas et s'appairent en 1 clic (`server_pair`) pour des sondes et un proxy `call_server_tool` signés.
- **Pont stdio, journal d'audit chaîné & anti-force brute (`M5`)** :
  - Nouveau pont stdio `mcp-control bridge [--url <url>]` permettant de brancher Claude Desktop, Claude Code et Cursor sans manipuler de clé.
  - Journal `auth-audit.log` chaîné par HMAC-SHA256 (rotation 8 Mio × 5 + ancre `auth-audit.head`), vérifiable par `mcp-control auth verify-log` (détecte toute ligne modifiée, supprimée ou insérée).
  - Anti-force brute par IP et par `keyid` (délai croissant puis `429`), réponses `401` uniformes en corps et en temps (~9,6 ms p50 mesurés sur 1 000 requêtes par catégorie), sans jamais bloquer le loopback depuis le LAN.

### Sécurité, durcissement & chaîne de livraison (`S1`–`S4`, audit `R` + `R-fix`)
- **Session locale durcie (`S1`)** : échange unique d'un jeton d'amorçage OS (`#t=` purgé aussitôt) contre le cookie `mcc_session` (`HttpOnly; SameSite=Strict`) **et** le secret mémoire `X-MCC-Session` (comparés en temps constant `ct_eq`, `X-MCC-Request` supprimé). Jusqu'à 8 sessions simultanées (12 h d'inactivité / 24 h absolues), route `POST /api/session/close-others`, canal fichier ACL utilisateur pour les seconds lancements et la CLI, échec fermé si l'entropie OS manque.
- **CSP stricte & Tauri confiné (`S2`)** : `default-src 'none'; script-src 'self'; style-src 'self'` (0 gestionnaire `on*=`, 0 attribut `style=`, 0 `.style.*` en JS), en-têtes `nosniff`, `no-referrer`, `COOP` et `CORP` sur toutes les réponses ; `withGlobalTauri: false`, capacité Tauri restreinte au seul port local servi et navigation externe bloquée.
- **Chaîne de livraison (`S3`)** : `deny.toml` strict (`wildcards = "deny"`, `rustix` mis à jour `1.1.5 → 1.1.6` pour régler `RUSTSEC-2026-0486`), `cargo-deny` + `cargo-audit` en CI, `SHA256SUMS.txt` signé (cosign keyless), attestations de provenance GitHub, SBOM CycloneDX (`*.cdx.json`) et affichage de l'empreinte SHA-256 attendue dans *Mises à jour*.
- **Système, process & fichiers (`S4`)** : `stop_server` ne tue plus un processus tiers occupant le port (renvoie nom/PID + exige `stop_server_force`), annonces LiteMCP listées en lecture seule (`litemcp_announcements`, sans adoption automatique), réglages `discovery_ports_only` / `discovery_ports_excluded` et `MCC_NO_LOCAL_DISCOVERY=1`, écritures atomiques avec ACL explicites (`config.json`, `state-cache.json`, `mcc.log`, `auth-audit.log`), résolution des outils système par chemin absolu (`system_tool`), `set_autostart` avec confirmation si entrée étrangère, et installation neuve sans serveur codé en dur.
- **Correctifs d'audit adverse (`R` + `R-fix`)** : vérification du port `Host` sur `/mcp` (interdit le rejeu inter-ports/inter-instances `R-1`), `BOOT_TS` rejetant en `401` uniforme toute requête signée avant le redémarrage courant (`R-2`), rejet `400` sur en-têtes critiques dupliqués (`Host`, `Origin`, `Cookie`, `X-MCC-Session`, `Content-Length`) ou `Content-Length` mal formé (`R-3`), écritures sans fenêtre TOCTOU pour les secrets et `auth-audit.head` (`R-4`, `R-8`), CLI `auth list` / `auth revoke` authentifiées par session locale S1 (`R-5`), limiteur 60 req/min/IP sur `GET /auth/pair/{id}` (`R-6`) et suppression complète du repli PRNG dans `random_hex` (`R-7`).

### Interface ultra-moderne & accessibilité (`UI-api`, `UI-modern` / `U1`, `U2`, `P2`)
- **Design system WCAG 2.2 AA (sombre & clair)** : 4 niveaux de surfaces hiérarchisés, contrastes mesurés via Headless Chrome CDP **≥ 6,16:1** en sombre et **≥ 5,33:1** en clair (bordures ≥ 3,83:1), typographie en `rem`, cibles ≥ 24×24 px, détection automatique `prefers-color-scheme`, respect de `prefers-reduced-motion`, anneau `:focus-visible`, landmarks HTML5, rôles ARIA (`tablist`, `switch`, `aria-live`), navigation clavier complète (`↑`/`↓`, `Alt+↑`/`Alt+↓`, `Ctrl+B`) et remplacement de tous les `alert()`/`confirm()`/`prompt()` natifs par `<dialog id="confirmDlg">`.
- **Centre Auth & Sécurité (5 onglets)** : *Confirmations* (compte à rebours local 1 s), *Appairage* (code à 6 chiffres + guide interactif `auth pair` / `bridge`), *Clients* (filtre actif/révoqué, badge `Inactif > 180 j`, rotation de clé Ed25519, révocation), *Identité & Sessions* (`mcc-server`, état trousseau/fichier, statut et empreinte TLS 1.3, sessions locales actives + *Fermer les autres sessions*, commande `auth verify-log`) et *Journal d'audit* (100 dernières décisions filtrables).
- **Ergonomie opérateur & DOM zéro-scintillement (`P2`)** : nouveau Dashboard actionnable (4 tuiles KPI + 4 cartes d'action rapide), pilules de filtre d'état (`Tous`, `UP`, `Panne`, `Verrouillés`, `Pause`), recherche et dépliage des outils par serveur (*Tout activer* / *Tout couper*), masquage des variables `env` dans la Boutique, et mémoïsation DOM garantissant **0 mutation DOM** sur les cycles `/api/state` au repos.

### Performances (`P0` → `P1`, mesurées sur binaire release `--features tls`)
- **CPU et process enfants au repos** : suppression des réécritures inutiles de `state-cache.json` (**12 → 0 écr./min** à 0 srv, **-83 %** à 40 srv) et des appels système `icacls`/`ipconfig`/`ping` redondants (**-77 à -85 %** de process enfants/min, `ipconfig`/`ping` **0/min**, CPU moyen **-49 à -60 %**).
- **Sondage MCP allégé** : mise en cache 5 min de `tools/list` tant que le serveur reste UP avec le même nom et la même version (`tools/list`/min **454,6 → 0,0** à 40 serveurs, cycle médian **1 616 ms → 393 ms, -76 %**, médiane de détection de panne inchangée).
- **`ETag` / `304 Not Modified`** sur `/api/state`, `/api/detail` et les assets statiques : rafraîchissement UI `load()` à 40 serveurs passé de **102,4 → 6,5 ms (-94 %)** sur le Dashboard et **138,1 → 4,3 ms (-97 %)** sur le Détail, **0 octet** retransféré au repos, heap CDP **-24 %** (0,99 Mo, 2 829 nœuds figés).
- **Démarrage et passerelle** : démarrage TCP moyen à 40 serveurs **667,4 → 148,2 ms (-78 %)** ; `POST /mcp list_servers` c=1 p50 **7,60 → 3,62 ms (-52 %)**, p99 **28,72 → 7,69 ms (-73 %)**, débit **103 → 215 req/s (+108 %)**.

### Correctifs et fiabilisation (inclus depuis 0.7.21)

### Supervision — un serveur muet ne retarde plus les autres
- **La période de supervision redevient celle du réglage** (« Intervalle de sondage », 15 s par défaut, 5 s minimum) : le cycle n'attend plus la fin des sondes, un serveur encore en sonde est sauté au cycle suivant, et une sonde est coupée dès la première forme envoyée sans aucun octet reçu. Mesuré sur une instance de test (1 serveur sain + 1 muet + 1 lent, réglage à 5 s) : **période du serveur sain 30-31 s avant, 5 s réguliers après** ; un serveur qui répond en ~6 s reste classé UP.
- **Un port qui accepte la connexion sans jamais répondre est classé en ~10 s au lieu de ~30** (9,9 / 10,4 / 9,8 s sur trois mesures, contre 30,2 s avant). L'erreur publiée reste celle de l'hôte qui a accepté la connexion, pas celle du repli refusé.
- **`config.json` édité à la main avec `scan_from > scan_to`** : la plage est remise dans l'ordre au chargement, avec une ligne « plage de scan inversée (…) — remise dans l'ordre au chargement » dans `mcc.log`, au lieu d'un scan qui ne rendait aucun port.

### Gateway MCP — `refresh` fait ce qu'il annonce, `set_start_command` sait retirer
- **Le tool `refresh` re-sonde tous les serveurs puis relance la recherche des nouveaux** (l'équivalent exact du bouton ⟳). L'ancien handler envoyait une action sans `id` et répondait « id manquant » à chaque appel ; en direct : `tools/call refresh` → `{"ok":true}`.
- **`set_start_command` accepte `program: null` (ou une chaîne vide) pour RETIRER la commande de lancement** : réponse `{"ok":true,"removed":true}`, `start_cmd: null` dans l'état, retrait persisté au rechargement. L'autoboot du serveur est désactivé au passage (il n'y aurait plus rien à lancer) et le process déjà lancé n'est pas tué — c'est le bouton ■ qui arrête.
- **Contrat modifié** : `program: "   "` (espaces seuls) levait une erreur, il retire désormais la commande, comme `null` et la chaîne vide.
- **`stop_server` / `remove_server` sur un identifiant inconnu répondent `{"ok":false,"error":"serveur inconnu"}`** au lieu d'un `ok` sans motif (Stop ne disait rien, Supprimer affichait un « échec » générique).

### Boutique — variables d'environnement à l'installation
- **L'installation d'un serveur de la boutique accepte un champ `env`** (`{"GITHUB_TOKEN": "…"}`) pour les items à jeton : les variables atteignent le transport et la commande de lancement, survivent au rechargement de la configuration, et **les valeurs ne ressortent jamais** — ni `/api/state`, ni `/api/detail`, ni la réponse, ni le journal (noms seuls).
- Disponible par l'interface (Boutique avec champs `env` masqués et bouton œil, `UI-modern`), par l'API locale (`install_store`) et par le tool `install_store_server` du gateway.

### Interface — un seul chemin pour les actions, panne moteur vue plus tôt
- **Tous les envois d'action de la page passent par un helper unique** (`postAction`, 30 sites d'appel, un seul `fetch('/api/action')`) qui lit `ok` et affiche `error` en toast : un refus du moteur ne peut plus passer en silence.
- **Moteur mort : réessai 2 s après le premier échec** (`POLL_RETRY_MS`), bandeau « moteur injoignable » au 2ᵉ échec consécutif au lieu d'attendre deux cycles complets, réessais toutes les 2 s pendant la panne, retour au rythme réglé dès la première réponse.
- **Une action qui ne part pas (erreur réseau) arme le bandeau immédiatement**, sans toast parasite ni exception ; au retour du moteur le bandeau tombe au réessai suivant.

### Démarrage automatique — une copie de l'app ne touche plus l'entrée d'une autre
- **Au démarrage, l'app n'écrit ou ne supprime l'entrée « MCP Control » que si elle pointe déjà sur son propre exécutable.** Une entrée qui pointe vers une autre installation — instance de test, build de développement, autre dossier — n'est jamais touchée ; seul le geste explicite sur l'interrupteur Auto-boot écrit l'entrée de l'exécutable courant. Même règle sous Linux (fichier XDG) et macOS (LaunchAgent).

### Sécurité — réponses masquées, motif de refus précis, réseau qui change
- **`set_notify` renvoie la configuration masquée** (même forme que `/api/state`, booléen `headers_set`) : les en-têtes du webhook ne ressortent plus en clair, pas même vers l'appelant qui vient de les enregistrer.
- **Le refus d'une URL de webhook donne le motif précis** (`https://` non pris en charge : aucune pile TLS embarquée) au lieu du message générique qui laissait croire que https était accepté.
- **Un `Host` inconnu déclenche un recalcul borné des adresses de la machine** (un recalcul au plus toutes les 5 s) : une adresse apparue après le démarrage (VPN, carte virtuelle, changement de Wi-Fi) n'est plus refusée jusqu'au redémarrage ; un `Host` qui n'est l'adresse d'aucune interface reste refusé en 403.

### Tests
- `cargo fmt --check` et `cargo clippy --all-targets -- -D warnings` : 0 avertissement. `cargo test --lib` : **214 verts**. `cargo test --test e2e --no-fail-fast` : **59 verts**, deux passages consécutifs.
- Harnais E2E : plus de débordement u16 du port de repli (« attempt to add with overflow »), et fixtures `probe_http_*` qui lisent la requête jusqu'au corps complet — les deux causes d'instabilité corrigées dans cette passe.

## [0.7.21] — fiabilisation : compilation, passerelle, supervision, persistance, UI, sécurité HTTP + doc alignée

### Refactorisation : `manage.rs` éclaté en 14 modules (zéro changement de comportement)
- **Le monolithe est éclaté** en modules thématiques purs (moves, API inchangée via réexports `pub(crate) use`) : `store` (boutique), `import` (parsing mcpServers), `config` (config.json), `json` (state/detail/runtime), `scans` (ports/netstat/réseau), `peers` + SSH, `update` (GitHub), `cache`, `autostart`, `litemcp`, `log`, `events` (notifications d'incident), `actions` (dispatcher, 1 409 lignes), `tests` (2 881 lignes) — `manage.rs` : 7 543 → 89 lignes (nombres mesurés après les correctifs des lots 0-5, ci-dessous).
- **160 tests unitaires verts à chaque étape du split** — mais le binaire `mcp-control` ne compilait plus depuis le 4ᵉ commit du split (E0603 : `resolve_config_path` invisible du crate binaire) : `cargo test` complet, clippy et la CI n'exécutaient donc **aucun** test à HEAD ; corrigé au lot 0 (ci-dessous). Captures README refaites pour la v0.7.19 dans le même intervalle (`445e005`).
- **Pourquoi** : les diffs d'UI et de moteur deviennent lisibles, les modules se lisent seuls, et le prochain raffinement (découper `actions.rs` par famille avec helpers `with_server_mut`/`commit`) se fera sans toucher au monolithe.

### Lot 0 — le binaire compile de nouveau, CI renforcée
- `resolve_config_path` redevient publique : **le binaire `mcp-control` compile de nouveau** ; clippy `-D warnings` remis à zéro (glob réexport inutile retiré, doc recollée à ses fonctions) ; la **CI vérifie le build release** du crate CLI. Aucun changement fonctionnel.

### Lot 1 — passerelle MCP durcie
- **Tout argument inconnu d'un tool est refusé** (`-32602`, schémas stricts) et l'action est fixée par le tool : l'injection d'`action` (`set_settings`, `set_notify`, `set_autostart`, `reset_state`, `reset_runtime`, `open_path`) ou d'une clé du dispatcher dans un tool d'action est impossible.
- **`Accept: */*` est accepté et servi en JSON** — l'exemple curl du README (défaut de curl et de la plupart des libs) fonctionne sans en-tête Accept ; `application/xml` reste 406, `text/event-stream` seul reste SSE.
- `set_start_command` accepte `program: null` au niveau du schéma (plus de `-32602`) ; le retrait effectif de la commande reste à implémenter côté dispatcher (voir TODO).

### Lot 2 — supervision : cycle et relances
- **Le cycle n'est plus retardé par un serveur lent, muet ou en cours de démarrage** (pool de sondes persistant) — période ≈ refresh + sonde la plus lente (48 s → 30 s mesurés avec 1 UP + 1 muet + 1 crashloop).
- **Relances bornées** : un démarrage qui échoue (process sorti sans MCP, échéance de 30 s dépassée) compte un échec — backoff croissant 5 → 10 → 20 → 40 s puis abandon (« auto-restart désactivé après 5 échecs » au journal) ; « start-ok » n'est journalisé qu'à l'UP réellement observé. Pendant la fenêtre de démarrage, le motif « démarrage en cours… » remplace le DOWN/PORT-ONLY prématuré.
- Motif d'erreur d'un port qui accepte sans répondre : **délai dépassé de l'hôte joignable** au lieu du « connexion refusée » du repli. Réponse d'outil au-delà de 2 Mio : « réponse trop grande (plafond de lecture de 2 Mio dépassé) ».

### Lot 3 — persistance, import, cycle de vie
- **Import mcpServers : les variables `env` des serveurs stdio sont conservées** et passées au process lancé (clé d'API fonctionnelle) ; `/api` et le journal n'exposent que les NOMS de variables.
- **Import d'un bloc JSON : les entrées écartées sont listées** dans la réponse `skipped:[{name,reason}]` avec motif explicite (fin du silence `ok:true`).
- **config.json** : écritures sérialisées (50 add simultanés → 50 persistés) ; fichier illisible (corrompu/vide/BOM) renommé `config.json.corrompu-<horodatage>` + trace dans mcc.log au lieu d'un écrasement silencieux ; droits 0600 à la création sous Unix.
- **Remove d'un serveur lancé par l'app : le port est libéré** même si le lanceur détachait son enfant (fin des process orphelins) — jamais pour un serveur non lancé par l'app.
- **Scan de ports : plafond 512** (refus clair au-delà), sondage parallèle (~60× plus rapide), réponse `{"ok":true,"found":N}`.

### Lot 4 — UI
- **Bandeau « moteur injoignable »** + compteurs grisés dès 2 échecs de sondage consécutifs (« moteur injoignable, nouvel essai… » au premier échec si rien n'a jamais chargé) ; retour automatique dès la première réponse ; un 401 (code d'accès demandé) ne compte pas comme une panne.
- **Dialog « Ajouter un serveur » : zone « Coller un bloc mcpServers ou une URL »** (JSON, URL http, commande npx/uvx/claude mcp add) — nombre ajouté affiché, entrées écartées listées avec leur motif.
- **URLs IPv6 entourées de crochets** dans « Conf agent », « Copier l'URL » et le texte d'info (`http://[::1]:4104/mcp`) ; **127.0.0.1 / ::1 / localhost explicites traités comme locaux** (plus de groupe « Machines distantes », formulaire de commande de lancement disponible).
- **Refus du moteur affichés en toast** pour favori, autoboot et scan de ports ; **scan** : bouton occupé pendant le balayage + toast « N serveur(s) MCP trouvé(s) ».
- **Barre de titre responsive** : icônes seules sous 1 120 px, pastilles sous 700 px, wordmark sous 480 px ; contrôles de fenêtre jamais compressibles (visibles à la largeur minimale du desktop, 940 px). Bouton « copier » de « Conf agent » : repli presse-papiers comme « Copier l'URL ».

### Lot 5 — sécurité HTTP
- **Anti-déni de service** : échéance totale de 5 s par requête, plafond de connexions par adresse IP et part réservée au loopback — une tempête de sockets muettes du LAN ne coupe plus la fenêtre locale ; saturation refusée en **503 explicite** (plus de fermeture muette).
- **/api/hello** : nonces évincés du plus ancien (plus de vidage global) et débit limité à 30 req./60 s par adresse hors loopback (429). **Requête sans en-tête `Host` → 400** (l'anti DNS-rebinding n'est plus contournable).
- **/api/state n'expose plus les en-têtes du webhook** (booléen `headers_set`) ; dans Réglages → Notifications, champ vide = en-têtes inchangés.
- **Webhook** : URL `https://` refusée (aucune pile TLS — l'envoi partait en HTTP brut sur 443), URL visant cette instance refusée, chaque envoi journalisé dans mcc.log (cible + résultat).
- **Notification desktop remise en service** : un seul `-Command`, titre/message passés par variables d'environnement (plus d'injection possible via un label de serveur).

### Lot 6 — doc alignée sur le code
- README / `docs/frameworks.md` : **38 frameworks** (et non 37), libellés réels des boutons, limites WebSocket et webhook complétées, surface gateway exacte et avertissement « code d'accès = droit d'exécution » ; un test verrouille le nombre annoncé dans la doc contre la table du code (`src/frameworks.rs`).

## [0.7.20] — Le wizard Windows épingle MCP Control à la barre des tâches

### Raccourci barre des tâches créé par l'installateur
- Nouveau point d'ancrage NSIS (`installerHooks` de Tauri v2) : à la fin de l'installation, le wizard **épine le raccourci du menu Démarrer à la barre des tâches** (verbe shell `taskbarpin`, Win10/11) — l'épingle apparaît immédiatement, sans redémarrage d'Explorer.
- La **désinstallation désépingle d'abord** (verbe `taskbarunpin` avant la suppression du raccourci — le verbe a besoin que le .lnk source existe) : plus d'icône morte dans la barre des tâches après désinstallation.
- `ExecShell` n'est jamais fatale : un verbe refusé par une stratégie d'entreprise n'interrompt pas l'installation.
- macOS et Linux n'ont pas de wizard ni d'API d'épinglage universelle : leurs binaires restent livrés bruts.
- Validé en local : `tauri build --bundles nsis` compile les hooks (makensis OK), installateur 2,81 Mo.

## [0.7.19] — UI en assets servis : fin du monolithe, CSP serrée

### Split de `ui.rs` : `/app.css` + `/app.js` + `/app-boot.js`
- **3 073 lignes** de HTML/CSS/JS inline dans une raw string Rust deviennent une **coquille HTML de ~240 lignes** + trois vrais fichiers (`src/ui/app.css` 652 lignes, `src/ui/app.js` 1 807, `src/ui/app-boot.js` 2) : diffs d'UI lisibles, plus la contrainte du guillemet-double-suivi-de-dièse interdit dans la raw string, édition directe des fichiers.
- **Service** : GET `/app.css`, `/app-boot.js`, `/app.js` (types `text/css` et `text/javascript`, charset explicite), pré-authentification comme la coquille — ils ne contiennent aucun secret (le code d'accès ne quitte jamais le serveur) et la page de login en a besoin.
- **CSP** : `script-src self unsafe-inline` et `style-src self unsafe-inline` — les fichiers externes sont même origine ; `unsafe-inline` reste requis par les attributs `on*` des gabarits (la délégation d'événements est le chantier suivant) et les styles inline générés.
- **Cache** : coquille `no-cache` (chaque navigation repart du lien ?v=VERSION à jour) ; le 304/ETag reste le chantier perf dédié.
- `app-boot.js` (thème anti-flash) reste chargé **bloquant dans <head>** — 2 lignes, zéro flash de thème ; le script principal passe en `defer`.
- **Contrats de tests étendus** : tout-handler-câblé, actions-connues-du-moteur et valeurs reset scannent désormais coquille + `app.js` ; nouveaux tests — la coquille référence les trois assets versionnés, `route()` sert chaque asset avec le bon type, la version substituée et **jamais le placeholder résiduel**.

## [0.7.18] — Favoris ⭐ épinglés, vrais logos de langages, mise à jour directe et explorateur refondu

### Favoris (⭐) : épingle tes serveurs en haut de la liste
- Nouvelle étoile sur chaque ligne de l'explorateur (et bouton « ★ Favori ON/OFF » dans le panneau de détail) : un serveur épinglé passe **en haut de sa section** — serveurs locaux ou par machine distante — quel que soit le mode de tri (manuel, A→Z, état…). Tri stable : les favoris gardent leur ordre relatif entre eux, et l'ordre posé au glisser-déposer n'est pas altéré (l'épingle s'applique à l'affichage).
- **Persisté** dans config.json (nouvelle action `set_favorite`, enregistrée dans la surface `ACTIONS` partagée) : l'épingle survit au redémarrage du moteur. La config n'écrit que l'activation (défaut `false`) — le fichier reste lisible. Non exposé au gateway (préférence locale d'affichage).
- Icône `star` ajoutée au set SVG monochrome de l'app ; étoile **dorée remplie** quand épinglée, outline grise sinon.

### Vrais logos de langages (SVG officiels embarqués)
- Les monogrammes colorés (JS, PY, GO…) laissent place aux **marques officielles** en SVG inline, même technique que le logo Rust (engrenage « R ») :
  - **Node.js/TS** : le carré jaune officiel ECMAScript avec « JS » noir ;
  - **Python** : les deux serpents bleu/jaune entrelacés — la forme bleue est répétée pivotée de 180°, la construction du vrai logo ;
  - **Go** : tuile cyan officielle ; **.NET** : carré violet officiel ;
  - **Java** : la tasse bleue à vapeur orange ; **Ruby** : le gem facetté rouge ; **PHP** : l'ellipse violette « php ».
- Zéro requête réseau (SVG inline au binaire), tooltips inchangés (« Langage inféré : … »), badge framework toujours coloré par langage.
- Dans l'explorateur tous les logos descendent à **14 px** (hauteur de ligne préservée) ; 18 px dans l'en-tête de détail.

### Alerte réseau : une pastille dans la barre du Journal, plus de bandeau plein écran
- L'ancien bandeau rouge pleine largeur (« Réseau local injoignable — les serveurs distants ne peuvent pas être sondés — VPN actif ? ») **poussait toute l'interface vers le bas** à chaque incident VPN. Il devient une **pastille rouge compacte dans la barre du Journal**, côté droit juste avant « auto-détection » — visible même le journal replié, alignée avec version et mise à jour.
- Le message complet (VPN actif ? Proton : « autoriser les connexions LAN ») vit dans le **tooltip** de la pastille et s'affiche **au clic** en toast erreur 6 s — l'explication reste complète sans encombrer la barre en permanence.
- La marge d'alignement droit suit la pastille quand elle est visible et revient au badge auto-détection quand elle disparaît (réseau rétabli) — le groupe reste toujours aligné à droite.
- `toast(msg, kind, ms)` accepte une durée explicite pour les messages longs.

### Boutique : 100 % des cartes ont leur logo
- Inventaire complet : **21 items du catalogue** n'avaient aucune icône (Neon, Logfire, iOS Simulator, N-able, OrcaReplay, Trace, Alpaca Trading, LiveTennisAPI, Memex, Google Workspace, HumanPen, Local MCP, BrowserAct, HikerAPI, Lamatok, CodeGraph, World Monitor, Codebase Memory, Context Mode, Serena, Deepgraph Next.js).
- **Marques officielles récupérées** quand Simple Icons les couvre (pomme Apple pour iOS Simulator, marque Next.js pour Deepgraph — CDN Simple Icons) ; pour les autres, **glyphes dessinés dans le style de la collection** (trait 2, couleur de marque) : éclair Neon vert, flamme Logfire, tuile « N » N-able, relecture OrcaReplay, chandeliers de trading, balle de tennis et ses coutures, grille aux quatre couleurs Google, montagnes HikerAPI, bulle Lamatok, graphe de nœuds CodeGraph, globe World Monitor, archive Codebase Memory, accolades Context Mode, étincelle Serena…
- `LOGOS_JSON` passe de 64 à **85 entrées** — la boutique affiche désormais une identité visuelle distincte sur chacune de ses 63 cartes (le glyphe « paquet » ne sert plus que de filet de sécurité pour de futurs items).

### Boutique refondue : navigation, icônes et état « Déjà installé »
- **En-tête collant** : recherche, tri et catégories restent visibles pendant le défilement de la grille ; **compteur de résultats** (« 63 serveurs » / « 12 affichés sur 63 »).
- **Tri** « Ordre du catalogue » / **A→Z** ; bascule **« Masquer les installés »** (pilule cohérente avec les catégories, état actif marqué).
- **Icône sur TOUTES les cartes** : les items sans marque reconnue reçoivent le glyphe « paquet » neutre — plus aucune carte sans icône, avant comme après le chargement des logos.
- **« ✓ Déjà installé »** : une carte dont le serveur (`store-<id>`) est déjà dans ta liste affiche un état vert non cliquable à la place d'Installer (source de vérité = `state.servers`, re-rendu après installation réussie) — plus de double installation possible d'un clic distrait.
- **Ligne de commande retirée des cartes** : la description suffit ; la commande reste visible dans le détail une fois installé. Au passage, plus aucun chemin personnel n'est affiché dans la boutique.
- Raccourci **`/`** et **Ctrl+K** : la recherche de la boutique a le focus quand le dialog est ouvert (sinon le filtre de l'explorateur, inchangé).

### Explorateur refondu : les noms de serveurs sont redevenus visibles
- **Bug d'affichage sérieux** : dans la sidebar, les badges (langage, transport, framework) écrasaient le nom du serveur (`flex:1` sans largeur plancher) — plusieurs serveurs n'affichaient **aucun nom** (largeur mesurée : 0 px), les autres un tronçon « r… », « to… ».
- **Nouvelle structure de ligne** : ligne 1 = point d'état + **nom seul** + actions (⚡ autoboot, ▶/■ start-stop, ON/OFF) ; ligne 2 = état textuel + transport + badges langage/transport/framework + compteur d'outils. Les badges descendent d'une ligne au lieu de se battre avec le nom ; à très faible largeur, la fin de la ligne 2 se coupe, **jamais le nom ni les actions**.
- Largeur par défaut de l'explorateur : 280 → **320 px** (l'ajustement par poignée est inchangé).

### Icônes SVG partout, fin des emojis
- Barre de titre, dialogues (Boutique, Machines, Accès, Réglages à volets), menus contextuels, boutons du détail, tuiles, journal réseau : un **jeu unique d'icônes SVG inline** (trait 1.5, `currentColor`, zéro couleur fixe) remplace les emojis — rendu identique quelle que soit la version d'Windows, alignement typographique propre, cohérence avec l'interface monochrome.
- Apostrophes réparées au passage dans les menus (« Copier l'URL », « Ouvrir dans l'explorateur »).

### Suppression : visible et confirmée
- Les boutons destructifs portent une **bordure rouge permanente** (plus seulement au survol) et « Supprimer » est **séparé** du groupe d'actions du détail.
- La suppression d'un serveur importé demandait **aucune confirmation** : un clic loupé suffisait. Désormais un `confirm()` explicite (« un serveur re-détecté par un scan réapparaîtra ») ; les seeds gardent leur confirmation dédiée « Supprimer définitivement ».

### Filtre de l'explorateur (recherche instantanée)
- Champ de filtre en tête d'explorateur : nom, id, framework, port, état. Raccourcis **`/`** et **Ctrl+K** depuis n'importe où, **Échap** efface, bouton × visible dès saisie. Les compteurs de groupe continuent de compter TOUS les serveurs ; un groupe filtré vide affiche « aucun serveur ne correspond au filtre ».

### Tuiles d'accueil cliquables
- « **serveurs UP** » filtre l'explorateur sur les UP (re-cliquer ou « serveurs suivis » réaffiche tout, tuile active marquée), « **tools actifs** » applique le tri par nombre d'outils, « **machines distantes** » ouvre le dialog SSH. L'accueil répond au clic au lieu d'être un constat passif.

### Descriptions d'outils : markdown inline rendu
- Les serveurs MCP renvoient souvent `**gras**` et du `code` : affichés bruts, les astérisques cassaient la lecture. Rendu inline minimal (**gras**, *italique*, `code`) — texte échappé AVANT transformation, aucune surface d'injection.

### Micro-finitions
- **Valeurs clé/valeur copiables d'un clic** (pid, commande, url…) avec affordance au survol ; « pas de valeur » affiche un tiret cadratin **—** partout (fini le pid « - »).
- **Toasts sémantiques** : succès (filet vert) et erreur (filet rouge, affichée 3,5 s au lieu de 1,2 s — un échec se lit).
- Halo LiteMCP **statique** (fin du pulse permanent qui attirait l'œil en continu) ; « désuet » → « **legacy** » (vocabulaire standard du domaine) ; tooltips explicites sur les badges de la barre de titre (UP, outils, refresh) ; message de vérification de mise à jour sans emoji.
- **Correctif « vv »** : le tag GitHub arrivait avec son « v » et chaque affichage en rajoutait un — le journal disait « mise à jour disponible : vv0.7.16 ». Le tag est normalisé à la source (`check_update`), tous les affichages (journal, badge, Réglages) sont cohérents.
- **Version et bouton ⬆ sur la ligne Journal** : la barre d'état séparée est retirée — « vX.Y.Z » et « ⬆ Mettre à jour » vivent sur la même ligne que « Journal » et « auto-détection », visibles même journal replié (le clic sur le bouton ne replie pas le journal — propagation coupée). Libellé épuré : juste **« Mettre à jour »** (icône download) — les versions (dispo + installée) s'affichent **en survol**.
- **Jaune JS assagi** (`#f7df1e` → `#dcc94f`) : le carré ECMAScript et les badges framework dérivés gardent leur identité sans flasher au neon.
- **Masquer un serveur de l'affichage** : « 👁 Masquer » au menu contextuel (clic droit) ou au panneau de détail retire la ligne de l'explorateur — comptes de groupe compris — sans toucher à la supervision. Une barre « N serveurs masqués — afficher » permet de les réafficher. **Session seulement** : rien n'est persisté (ni config.json ni localStorage), la réouverture de l'app les fait revenir — supervision continue en fond, l'affichage seul est filtré.

### Le bouton « ⬆ Mettre à jour » télécharge directement le BON fichier
- Avant : le bouton ouvrait la **page des releases** — il fallait encore trouver le bon fichier parmi les plateformes.
- Maintenant : **téléchargement direct** de l'asset de la dernière release correspondant à la machine. La détection est double :
  - **l'OS** vient du navigateur (la machine où atterrit le fichier — l'UI peut être servie par un moteur distant, cas VPS) ; le moteur valide contre une whitelist et retombe sur le sien si absent ;
  - **le type de binaire** suit le moteur qui sert l'UI : l'app desktop propose l'**installateur desktop** (setup), le moteur headless propose la **CLI** — détection par le nom de l'exécutable courant.
- **Sécurité** : l'URL est construite **uniquement** depuis une table blanche pure et testée (`release_asset`, 6 combos desktop×OS + CLI×OS) — jamais depuis la requête ; un `os` fourni hors whitelist est **refusé net** (jamais de repli silencieux vers une autre plateforme, aucun effet de bord avant le contrôle). Nouvelle action `download_update`, non exposée au gateway (lance un téléchargement).
- Repli : si le moteur ne peut pas lancer de navigateur (headless), l'URL rendue est ouverte par l'UI (`window.open` — vrai navigateur) ; si le moteur est injoignable, repli sur la dernière URL connue.

### Version et bouton ⬆ en bas à droite (barre d'état)
- Nouvelle **barre d'état** discrète en bas de fenêtre : la **version installée** (`v0.7.18`) et le **bouton bleu ciel « ⬆ Mettre à jour — vX dispo »** y vivent, à droite — la barre de titre est allégée (badges UP/outils/refresh restent en haut).
- Le bouton n'apparaît que quand une release plus récente existe (détection au boot puis chaque heure, inchangée).

### Réglages → Mises à jour
- « ⬆ Mettre à jour » fait le même téléchargement direct ; un bouton « 🔗 Releases » ouvre la page de toutes les versions (archives, anciennes releases).

### Tests
- **157 tests unitaires verts**, clippy 0 warning, fmt propre. Nouveaux contrats :
  - **favoris** : épingle → état + persistance config.json → relecture (survit au redémarrage) → retrait (la clé disparaît) → serveur inconnu refusé. Le test a attrapé au passage un bug d'indentation : le favori ne se serait jamais écrit (insertion dans le mauvais bloc `if`) ;
  - **mise à jour directe** : table blanche des 6 assets, refus des plateformes hors table **avant tout effet de bord** — le test du refus a attrapé un vrai bug pendant l'écriture : un `os` invalide retombait silencieusement sur la plateforme du moteur **et lançait le téléchargement**, corrigé en refus net ;
  - contrats « tout bouton câblé » et « actions ⊆ moteur » étendus aux nouveaux handlers (`set_favorite`, `tileFilter`/`tileSortTools`/`clearFilter`).
- Vérifications navigateur : étoiles dorées + épinglage en tête sur serveurs réels ; rendu visuel des 9 logos validé ; barre d'état (version à droite, bouton bleu ciel « vX dispo ») ; explorateur (noms visibles, filtre + raccourcis, tuile filtre UP, markdown, kv copiables, menus contextuels, Réglages sombre/clair, Boutique).

## [0.7.16] — Réglages professionnels à volets, Notifications câblées à l'UI, contrat « tout bouton »

### Dialog Réglages refondu (navigation par sections)
- **Volets façon app système** : une navigation latérale (⚙ Général · 🔍 Découverte · 🖥 Interface · 🔔 Notifications · ⬆ Mises à jour · ⚠ Réinitialiser) remplace la pile unique de groupes. Le dernier volet consulté est mémorisé (`mcc.setpane`), la section dangereuse est détachée sous un séparateur et prend la teinte rouge.
- **Dirty state** : toute saisie allume « ● Modifications non enregistrées » dans le pied ; Esc, × et Annuler avec des saisies pendantes demandent confirmation au lieu de les jeter en silence. Le focus est posé dans le dialog à l'ouverture — Esc fonctionne immédiatement.
- **Validation par champ** : chaque erreur s'affiche sous SA ligne (intervalle 5-3600, ports 1-65535, webhook http(s), au moins un état notifié) ; la plage de scan inversée est réordonnée avec une mention explicite. Fini la ligne unique qui disait tout ou rien.
- **Messages parlants** : « ✓ enregistré : intervalle de sondage, plage de scan » (labels humains via `SETTINGS_LABELS`) au lieu des clés internes ; rappel « l'écoute s'applique au prochain démarrage » quand `ui_bind` change.
- **Enregistrer ne rejoue plus tout** : `set_notify` n'est POSTé que si la section Notifications a réellement changé (instantané comparé à l'ouverture) — plus d'entrée « [notif] configuration enregistrée » dans le journal à chaque Enregistrer.

### Notifications : l'UI rattrape le moteur (promesse du 0.7.13 tenue)
- Le moteur avait `set_notify` (toast Windows + webhook), la config persistée et le déclenchement sur incident — mais **aucun réglage UI ne le pilotait**. Le nouveau volet Notifications expose tout : switch toast desktop, URL de webhook (+ méthode POST/PUT, en-têtes « Clé: valeur » par ligne), états notifiés (DOWN / PORT-ONLY — les seuls états qui déclenchent une transition), délai anti-spam.
- **Nouvelle action `test_notify`** (bouton « 🔔 Tester ») : enregistre les saisies pendantes puis émet une notification de test sur chaque canal ACTIVÉ et rend le résultat **par canal** (`desktop: ok/erreur`, `webhook: ok/erreur`) — jamais un silence. Volontairement **non exposée au gateway** (déclenche un toast et une requête vers l'URL configurée).
- `/api/state` expose désormais `settings.notify` pour peupler le volet — **jamais** passé au gateway `get_settings` : l'URL de webhook peut porter un `Authorization`.

### Réinitialiser : historique runtime
- Le moteur avait `reset_runtime` (effacer crashs, relances, backoff) sans aucun bouton. Le volet Réinitialiser l'expose avec confirmation, résultat affiché (« ✓ historique effacé (N serveur(s)) »).

### Polling UI armé sur les réglages
- `setInterval(load, 5000)` figé → `armPolling()` suit `settings.refresh_secs` (re-armé à chaque changement, plancher 5 s) : le badge « refresh Ns » et le rythme réel coïncident enfin ; moins de requêtes quand l'intervalle est élevé.

### Contrat « tout bouton est câblé » (tests)
- **`ui.rs` — module de tests** (nouveau) : parseur des attributs `on*` du HTML embarqué (y compris gabarits JS) → **tout handler référencé doit être défini** dans la page (un bouton mort = test rouge) ; **toute action API écrite dans l'UI doit figurer dans `manage::ACTIONS`** ; les 4 valeurs du select de réinitialisation sont confrontées au moteur.
- **`manage::ACTIONS`** (nouveau) : la surface du dispatcher devient une constante partagée (UI / gateway / tests), testée sans doublon et avec un échantillon dispatché « jamais action inconnue » (les actions à effet de bord réel restent couvertes par l'E2E).
- **Tests `set_notify` / `test_notify`** : URL invalide refusée avant tout, méthode normalisée, clamp anti-spam, persistance config.json, exposition `/api/state`, refus « aucun canal actif », échec rapporté par canal sans panic.
- **Tests gateway additionnels** : POST sans Accept → JSON par défaut, Content-Type absent → 415, outil inconnu → erreur propre sans panic, `resources/read` URI inconnue → erreur, `tool_count`, contrat `action_req`.
- **E2E** : deadline du scénario `hostile_label_never_in_html` élargie à 45 s — le test flottait sous la suite complète (45 moteurs en parallèle dépassaient 20 s), il passe isolément comme en suite.

### Packaging : un seul livrable Windows, moteurs renommés « CLI »
- **Windows = 1 fichier** : l'installateur wizard (`mcp-control-desktop-windows-x86_64-setup.exe`) est le seul livrable desktop Windows publié — le binaire portable desktop (déjà embarqué dans l'installateur) n'est plus un asset séparé. Fini les 3 fichiers Windows.
- **Les moteurs console s'appellent « CLI »** : `mcp-control-cli-windows-x86_64.exe`, `mcp-control-cli-linux-x86_64`, `mcp-control-cli-macos-universal` — le nom dit ce que c'est (une console, pas une app à fenêtre).

### Tests
- **154 tests unitaires + 45 tests E2E verts**, `cargo clippy --all-targets -- -D warnings` : 0 warning, `cargo fmt --check` : propre.
- Vérification UI réelle (moteur de test + navigateur) : volets, dirty badge, erreurs inline, swap de plage, sauvegarde (`refresh_secs`/`scan`/`notify` persistés, polling ré-armé), bouton Tester (résultats par canal), Esc/confirm, historique runtime effacé, volet mémorisé.

## [0.7.15] — Le bouton « Mettre à jour » qui marche vraiment

### Clic mort réparé (le vrai bug)
- **Cause** : dans la webview Tauri (app desktop), `window.open` est **neutralisé** — le badge « mise à jour » en haut et le bouton « ⬆ Mettre à jour » des réglages ne faisaient donc **rien du tout** au clic (silence total, aucune erreur).
- **Fix** : nouvelle action moteur `open_release_page` — URL **fixe** (`github.com/DeamonDev888/mcp-control/releases/latest`), **sans argument** (aucune surface d'injection), non exposée au gateway MCP. Le moteur lance le navigateur par défaut de la machine (`rundll32 FileProtocolHandler` sur Windows, `open` macOS, `xdg-open` Linux) ; l'UI garde un repli `window.open` quand elle tourne dans un vrai navigateur.

### Bouton « Mettre à jour » bleu ciel en haut (demande utilisateur)
- Le badge `stUpdate` devient un **vrai bouton bleu ciel** (`#8fd3ff`, texte foncé, gras, ombre douce) : il apparaît en haut de la fenêtre dès qu'une release plus récente que l'installée est détectée (au boot puis chaque heure), libellé « ⬆ Mettre à jour — vX dispo ». Clic → page de téléchargement.

### « Vérifier » ne ment plus
- `check_update` distinguait mal l'échec réseau d'un « à jour » : l'échec pouvait afficher « ✓ à jour » alors que la vérification avait **raté**. Nouveau verdict explicite (`UpdateCheck` : Available / UpToDate / Unreachable) et réponse d'action avec `checked:false` → l'UI affiche « ✗ vérification impossible (réseau ?) » au lieu d'une fausse certitude.
- `update_latest` est **vidé** quand l'installée redevient la dernière release (le badge ne reste plus collé avec une dispo périmée).

### Tests
- Pas d'e2e pour `open_release_page` : toute action lancerait un vrai navigateur — effet de bord visible interdit dans la suite (même règle qu'`open_path`/A15) ; le contrat est vérifié à la main (réponse bornée, URL fixe).
- Aucune régression : le poller horaire garde son silence hors ligne ; verrouillage AB-BA 2026-10-25 conservé ; outil gateway `check_update` inchangé.

## [0.7.14] — Packaging : binaire macOS universel, téléchargements simplifiés

- **macOS fusionné** : un seul livrable par type — `mcp-control-desktop-macos-universal` (app avec fenêtre) et `mcp-control-macos-universal` (moteur headless). Les deux architectures (Intel `x86_64` + Apple Silicon `aarch64`) sont compilées puis fusionnées avec `lipo` en un **binaire universel** : macOS choisit nativement le bon jeu d'instructions au lancement, aucun choix à faire — le même fichier tourne sur M1→M4 comme sur les Mac Intel (le binaire pèse ~2× un binaire mono-archi, c'est le prix de l'unicité).
- **Windows** : l'installateur wizard devient la seule voie proposée pour l'app avec fenêtre (le binaire portable reste disponible dans les assets de la release, il n'est simplement plus mis en avant — ni dans le README, ni dans les notes de release).
- **Workflow release** : job macOS unique par livrable (compilation des 2 cibles + `lipo -create`, vérification `lipo -info` dans les logs) ; `workflow_dispatch` permet de tester les builds sans publier. Notes de release resserrées à un fichier principal par OS.
- **README** : table de téléchargement à une ligne par OS — setup Windows, macOS universel, Linux x86_64.

### Tests
- Aucun changement de code Rust : `cargo test`, `clippy -D warnings`, `fmt --check` inchangés.

## [0.7.13] — Gateway MCP, notifications webhook, supervision runtime, suite E2E complète + ménage

### Gateway MCP (propulsé par LiteMCP)
- **`src/gateway.rs` (nouveau)** : la console expose **POST /mcp** sur son port UI (même socket durci que l'API) — un client MCP standard (ZCode, Claude Desktop, MCP Inspector…) obtient **toute la surface de l'UI en tools MCP** :
  - **lecture** : `list_servers` (états fins UP/PORT-ONLY/DOWN/DISABLED + filtre), `server_detail`, `list_server_tools`, `journal`, `runtime_history`, `list_peers`, `store_catalog` (recherche plein texte), `list_frameworks`, `get_settings` (jamais le code d'accès) ;
  - **actions** : chaque bouton de l'UI a son outil, tous adossés au **même dispatcher** que l'UI (`manage::handle_action` — une seule sémantique, une seule validation, un seul journal) : `refresh`, `scan_ports`, `scan_network`, `discover_peers`, `check_update`, `add_server`, `remove_server`, `toggle_server`, `start_server`, `stop_server`, `set_start_command`, `set_host`, `toggle_tool`, `set_auto_restart`, `reorder_servers`, `import_config`, `install_store_server`, `ssh_save`, `ssh_forget`, `ssh_discover` ;
  - **gateway** : `call_server_tool(server, tool, arguments)` proxyfie un `tools/call` vers n'importe quel serveur HTTP **détecté UP** (résultat upstream rendu tel quel ; outil coupé dans l'UI et serveurs via SSH/stdio/DOWN refusés avant tout appel réseau) ;
  - **ressources** : `mcp-control://servers` (aperçu), `mcp-control://server/<id>` (une par serveur détecté, snapshot reconstruit quand l'empreinte de détection change), `mcp-control://journal`, `mcp-control://settings`.
- **Serveur MCP 2026-07-28 complet** via la dépendance [litemcp](https://crates.io/crates/litemcp) 0.9 (serde uniquement) : `initialize`/`server/discover`, `resultType` + `_meta` sur chaque réponse, schémas d'entrée exécuteurs (zod — `-32602` avant tout handler), annotations read-only/destructive, hook `on_call` → le journal de l'UI raconte l'activité MCP.
- **Transport reflété sur notre socket** : `Content-Type: application/json` strict (415, doublon → 400), négociation `Accept` (SSE mono-événement `event: message` — injection de champs neutralisée ; 406 sans type acceptable), en-têtes `Mcp-Method`/`Mcp-Name` vérifiés contre le corps (SEP-2243 ; incohérents → -32020), notification → **202** sans corps, GET/DELETE → **405**. Auth héritée du sas existant : loopback jamais de code, hors loopback le **code d'accès API est exigé** (`check_access` couvre désormais `/mcp` comme `/api/*`).
- **Volontairement non exposés** (surface de confiance) : `set_settings` (code d'accès, bind, origines — le sas d'auth du gateway lui-même), `set_autostart`, `set_notify`, `reset_state`, `reset_runtime`, `open_path` — ces réglages restent des gestes UI locaux.
- Instantané MCP échangé quand la détection change (empreinte ids/états/outils comparée toutes les 5 s ; actions passées par le gateway → rafraîchissement immédiat).

### Divers
- **Ménage du dépôt** : suppression des launchers `start-mcp-control.bat` / `start-headless.bat` (la release installe ses propres raccourcis ; depuis un clone, lancer directement `target/release/mcp-control-desktop.exe` ou `mcp-control.exe`) ; passe hygiène vérifiée : **aucun secret ni URL personnelle** dans les fichiers suivis (config/cache/logs runtime déjà ignorés, aucune IP privée réelle — seulement des fixtures de test `10.0.0.x` et des domaines d'exemple).
- `reason_phrase` complétée (202/405/406/503) ; `Cache-Control: no-store` étendu aux réponses SSE.

### Tests
- **9 tests unitaires gateway** (transport 202/405/415/406/SSE/SEP-2243, surface complète des tools, validation de schéma avant handler, journalisation, empreinte, cible proxy, refus serveur inconnu/outil coupé) ;
- **7 scénarios E2E `scenarios_gateway.rs`** contre le VRAI binaire : initialize/tools/list/server/discover, supervision par MCP (list/detail/toggle vérifiés dans l'état du moteur), proxy vers une fixture MCP réelle (résultat upstream + refus DOWN), `toggle_tool` qui coupe le proxy, ressources par serveur détecté (snapshot reconstruit sans redémarrage), règles de transport complètes, matrice d'auth loopback/LAN ;
- **141 tests unitaires + 45 tests E2E verts**, `cargo clippy --all-targets -- -D warnings` : 0 warning, `cargo fmt --check` : propre.

### Notifications webhook (suite du lot)
- **`src/notify.rs`** : notification HTTP configurable (URL webhook, en-têtes personnalisés — voie dédiée tolérant `Authorization: Bearer …` avec échappement anti-CR/LF) lorsqu'un serveur crash ou redémarre. Configurable depuis l'UI via l'action `set_notify` ; état visible dans `/api/state`.

### Supervision runtime
- **`src/runtime.rs`** : l'auto-restart n'abandonne plus au bout de 3 échecs consécutifs quel que soit le délai — backoff exponentiel borné (5 s → 5 min), compteur **persisté** dans config.json, abandon seulement après 5 échecs dans la fenêtre de backoff. Historique borné (100 événements) par serveur : crashs, redémarrages, causes.

### Cœur — correctifs de l'audit 2026-10-26
- **Fusion des doublons restreinte** (`merge_duplicate_servers`) : seules les entrées **découvertes** avec un nom annoncé non vide peuvent fusionner (même nom + même machine, la plus récemment vue gagne) — jamais un serveur manuel, seed ou sans nom annoncé. L'ancienne version supprimait silencieusement des serveurs distincts (clé sans port, repli sur label) et persistait la suppression. Tests de régression ajoutés.
- **Découverte SSH non destructive** : une sonde validée sans `serverInfo`/`version` n'efface plus les valeurs connues ; le label fabriqué `ip:port` suit le nom annoncé ; les entrées désactivées ne sont jamais détournées ; `via_ssh` n'est plus forcé sur les serveurs manuels.

### Tests de bout en bout (nouveau)
- **`tests/e2e/`** : harnais Rust std-only (moteur réel lancé en isolement via `MCC_CONFIG`, fixtures MCP inline, verrou de sérialisation) + **38 scénarios** : auth loopback/LAN, Host/Origin forgés, **RCE sans code → 401 et aucun process lancé**, preuve de pair HMAC avec rejeu, fuzz filaire (413, Transfer-Encoding, Content-Length dupliqué), cycle de vie complet (add/UP/toggle tue le pid/relance/remove/persistance au redémarrage), scan (trouve une fixture, rejette `{"result":{}}`, régression fusion, prune), et **contrat de chaque bouton** (16 actions testées).

### Divers (suite du lot)
- Binaire/scripts renommés (`mcp-control`), `docs/frameworks.md`, bannières README, README mis à jour, workflow release ajusté.

### Tests (état au lot précédent)
- **121 tests unitaires + 38 tests E2E verts**, `cargo clippy --all-targets -- -D warnings` : 0 warning, `cargo fmt --check` : propre.

## [0.7.12] — Finitions « produit » : diagnostic réseau, pannes visibles, confort d'usage

### Diagnostic réseau (le cas VPN qui ressemblait à une panne)
- **Veille de la passerelle LAN** : toutes les 30 s, l'app pingue la passerelle par défaut et croise ce résultat avec l'état des serveurs distants. Si le LAN est coupé (VPN actif le plus souvent) alors que la machine est bien connectée, un **bandeau rouge** s'affiche en haut de la fenêtre : « réseau local injoignable — VPN actif ? (Proton VPN : autoriser les connexions LAN) ». Le journal enregistre l'apparition et le retour du réseau.
- Décision isolée dans une fonction pure `lan_decision()` et testée (4 cas : sain, passerelle muette, VPN trompeur, indéterminé).
- **Messages d'erreur actionnables** : une erreur SSH de type `connect to host …` n'est plus présentée comme un problème de clé mais comme « machine injoignable (LAN coupé / VPN actif, ou machine éteinte) ».

### Panics enfin visibles
- **`panic::set_hook`** : tout panic écrit désormais une ligne `[panic] …` dans `mcc.log` **et** sur stderr. Avant, en mode fenêtré (`windows_subsystem=windows`), un panic était totalement muet.
- **`catch_unwind` autour du cycle de sondage** : un panic dans un cycle ne tue plus le thread de supervision — il est journalisé et le cycle suivant repart.

### Confort d'usage
- **Boutons clarifiés** : `Désactiver` devient **« ⏸ Mettre en pause »** / **« ▶ Reprendre »** (le serveur reste listé) et `Retirer` devient **« 🗑 Supprimer »** — la confusion entre les deux avait conduit à mettre 14 serveurs hors supervision par erreur. Chaque bouton a un tooltip qui explique l'effet exact.
- **Bouton de déconnexion** (🔓 dans la barre de titre, visible seulement si un code est stocké) + **« Oublier le code enregistré »** dans le dialogue de déverrouillage : plus rien ne traîne dans le navigateur après une rotation de code ou sur un poste partagé.

### Performance
- **Sondage parallèle** : les serveurs sont sondés par paquets de 8 au lieu d'un par un. Avec ~30 serveurs dont 15 en SSH, un cycle dépassait le refresh de 15 s (états en retard d'un cycle) ; il tient maintenant dans l'intervalle. Les spawns concurrents restent protégés par la garde anti-doublon.

### Sécurité
- **Comparaisons à temps constant** (`crypto::ct_eq`) pour le code d'accès et la preuve de pair : la comparaison précédente sortait au premier octet différent.

### Documentation
- README : section **Installer** (assistant NSIS, sans droits admin), tableau des boutons mis à jour (⚡ autoboot par serveur, ⏸ pause, 🗑 suppression), 37 frameworks, FAQ « tous mes serveurs distants sont DOWN » (VPN) et « mes serveurs ont disparu » (pause).

### Tests
- **89 tests verts** (+ `lan_decision_couvre_les_cas_vpn_et_panne`), clippy `-D warnings` sous Windows **et** Linux, `cargo fmt --check`.

## [0.7.11] — Installateur wizard (NSIS) + bouton Autoboot par serveur

### Installateur Windows (wizard)
- **Vrai installateur `*-setup.exe`** généré par Tauri (NSIS), publié dans la release à côté du binaire portable. Parcours : **choix de la langue (Français/English) → bienvenue → dossier d'installation → dossier du menu Démarrer → installation → fin** (avec case « créer un raccourci bureau » et lancement de l'app).
- `installMode: currentUser` : installation dans le profil utilisateur, **aucune élévation UAC** ; désinstalleur et entrée « Ajout/Suppression de programmes » inclus.
- Métadonnées du paquet : description courte/longue, éditeur `DeamonDev888`, catégorie `DeveloperTool`, icône de l'installateur.
- CI : le job desktop Windows construit désormais le bundle NSIS (`@tauri-apps/cli build --bundles nsis`) et publie `<artifact>-setup.exe` en plus du binaire.

### Autoboot par serveur (⚡)
- **Nouveau bouton ⚡ par serveur** : dans la ligne de l'explorateur et dans la barre d'actions du détail. Il pilote `auto_restart` — le serveur est relancé **au démarrage du command center** et **après un crash**, indépendamment de l'autostart de l'app elle-même.
- Nouvelle action API `set_auto_restart` (`{action, id, enabled}`) : journalise le changement et, à la réactivation, **remet à zéro le compteur d'échecs de spawn** (un serveur qui avait été coupé après 3 échecs peut repartir).
- **Persistance réparée** : `auto_restart` n'était lu ni écrit dans `config.json` (toujours `true` au rechargement). Il est maintenant écrit quand il vaut `false` et relu au chargement — le bouton survit au redémarrage.
- État visuel : ⚡ orange quand actif, grisé sinon, tooltip explicite ; l'état est aussi rappelé en clair dans le panneau Démarrage du détail.

### Tests
- `auto_restart_par_serveur_action_et_persistance` : action → flag du serveur → écriture dans `config.json` → réactivation effaçant les échecs → serveur inconnu rejeté proprement.
- **88 tests verts**, clippy `-D warnings` (Windows et Linux), `cargo fmt --check`.

## [0.7.10] — Trois modes de transport (STDIO / HTTP / WS) + 37 frameworks MCP détectés par langage

### Frameworks MCP
- **37 frameworks reconnus, 5 maximum par langage** : Python (FastMCP, FastAPI-MCP, mcpo, mcp-proxy, MCP SDK), TypeScript/Node (FastMCP, xmcp, mcp-framework, MCP SDK, Fastify MCP), Rust (LiteMCP, rmcp, rust-mcp-sdk, tower-mcp, mcp-rs), Go (mcp-go, Go SDK, mcp-golang, gomcp, go-mcp), Java (Spring AI, MCP Java SDK, Quarkus MCP, LangChain4j, mcp-java), C#/.NET (MCP C# SDK, mcp-dotnet, mcpdotnet, csharp-mcp), Ruby (MCP Ruby, fast-mcp, ruby-mcp, mcp-rb), PHP (PHP MCP SDK, Laravel MCP, mcp-php, Symfony MCP).
- **Détection combinée** : signature comportementale d'abord (LiteMCP = spec 2026-07-28 non négociée + `/health` « ok » ; FastMCP = réponses SSE + `/sse` legacy co-servi, avec arbitrage Python/TypeScript par le langage du process), puis **nom annoncé** par le serveur (`serverInfo.name`) et **paquet lancé** pour les serveurs stdio.
- **Langage déduit du framework** quand il est identifié (rmcp → Rust, mcp-go → Go…), sinon en-têtes HTTP, process propriétaire du port ou commande stdio.
- **Matching par mot délimité** : `mcpo` ne matche plus `mcporter`, et `@modelcontextprotocol/server-*` (serveurs npm officiels) n'est plus pris pour le SDK C#. Tests de non-régression dédiés.
- Nouvelle route `GET /api/frameworks` (id, label, langage) : l'UI construit les badges depuis cette table, sans liste dupliquée en JavaScript. Badge générique `.tag.fw` pour les frameworks autres que LiteMCP/FastMCP.

### Durcissements (audit 2026-10-25)
- `sanitize_http_path` : un chemin de sonde sans `/` initial ou contenant un caractère de contrôle retombe sur le chemin par défaut (anti-CRLF dans la request-line) ; utilisé par `http_request`, `sse_get_host` et le probe.
- `http_host_header` : en-tête `Host` correct (`host:port`, crochets IPv6) sur toutes les requêtes de sonde.
- `read_bounded_line` : lecture stdio plafonnée (au plus `MAX_PROBE_READ` + 1 octets avant troncature) — plus d'allocation géante avant coupure.
- `RespHead.session_id` : le `Mcp-Session-Id` renvoyé par l'initialize est capturé, prêt à être rejoué sur les appels suivants.
- `ssh_error_hint` factorisé (un seul endroit pour les messages « auth refusée » / « machine injoignable »).

### UI
- **Plus de badge « HTTP+SSE » ni « SSE »** : le streamable JSON et les réponses encodées SSE (FastMCP/mcp-proxy) sont **tous les deux du HTTP** — le badge de transport affiche désormais simplement `HTTP`. Le badge de framework (`FastMCP`, `LiteMCP`) reste séparé et son tooltip ne parle plus de SSE.
- **Trois modes seulement : STDIO / HTTP / WS.** Le badge `HTTPS` (port 443) est retiré : ce n'était pas un mode de transport.
- **Badge WS** (orange) : affiché quand le serveur accepte une **upgrade WebSocket** (`HTTP/1.1 101 Switching Protocols`) sur le chemin MCP. Nouvelle sonde `ws_upgrade_host` (en-têtes `Upgrade: websocket`, `Sec-WebSocket-Key`), timeout court, détection seule — aucun frame WS n'est échangé.
- Détail serveur : la ligne framework de FastMCP n'affiche plus « + SSE legacy ».

### Technique
- `Server.ws` / `McpProbe.ws` propagés (probe HTTP loopback, probe par hôte, cache de démarrage, `/api/state`, `/api/detail`).
- Test de non-régression `ws_upgrade_detecte_101_et_rejette_le_reste` : 101 → WS, 200 → HTTP, port fermé → pas de panic.

## [0.7.9] — Journal replié par défaut + README ultra simple, captures refaites

### UI
- **Journal (console) replié par défaut** : la fenêtre est épurée à l'ouverture — un clic sur sa barre le déplie et le choix est retenu (localStorage, `mcc.logmin`).

### Docs
- **README encore plus noob-friendly** : vocabulaire courant, tableaux « Tu vois / Ça veut dire » et « Problème / Solution », parties techniques repliées en détails (API, sécurité, compilation). **Captures refaites sans le journal** (accueil, détail, serveur Rust avec le grand R, boutique ouverte).

## [0.7.8] — Audit complet 2026-10-25 : 3 panics mortels corrigés + XSS stockée fermée

### Corrigé
- **Le poller ne peut plus mourir en silence** : trois panics du parseur HTTP tuaient le thread de supervision des serveurs (aucun `catch_unwind` dans le projet, et en mode desktop aucune console → le panic était totalement invisible). La supervision s'arrêtait **définitivement** : plus de scan, plus d'auto-restart, état figé sans message d'erreur.
  - `data[sep + 4..]` paniquait sur une réponse HTTP de **moins de 4 octets** (serveur qui répond 1-3 octets puis ferme) — slicing hors bornes.
  - `dechunk()` paniquait sur une réponse **chunked tronquée** (`&data[i..]` avec `i > len`).
  - `dechunk()` paniquait sur une **taille de chunk hexadécimale absurde** (`ffffffffffffffff`) — overflow `usize` sur `start + size`.
  - Correctifs : bornage par `saturating_add` / `.min(data.len())`, boucle dechunk réécrite avec gardes de bornes. Tests de non-régression : 4 vecteurs de réponse courte + 8 vecteurs chunked hostiles. Fuzz de 16 réponses hostiles : **0 panic**.
- **XSS stockée fermée** : les statistiques d'un pair distant (`stats.up`, `stats.total`, `ui_port`) étaient interpolées sans échappement dans l'explorateur et le panneau Machines. Un hôte du LAN pouvait se faire passer pour un command-center (la sonde pair n'exige aucune preuve) et injecter du HTML/JS dans l'UI — donc exécuter du code dans la session de l'opérateur, lire le code d'accès stocké et appeler `/api/action` (ajout d'un serveur stdio = exécution de commande). Les 3 sites sont désormais échappés (`esc()`).
- **Lecture stdio plafonnée** : `BufRead::lines()` allouait une ligne entière en mémoire sans limite — un serveur MCP stdio écrivant 300 Mo sur une seule ligne saturait la RAM (mesuré : 314 572 800 octets en une lecture). Remplacé par une lecture bornée à 2 Mo, alignée sur le plafond HTTP existant.

### UI
- **Logo Rust agrandi** : 18 px (au lieu de 15) et glyphe « R » plus gras (font-size 12, weight 800), cerclage affiné — le R est nettement plus lisible dans l'explorateur et le détail.

### Docs
- **README refait, simple et à jour** : plus d'image en tête décorative — **quatre vraies captures d'écran de l'app** (accueil, détail avec tools et badges, serveur Rust avec le grand R + badge LiteMCP, boutique avec logos), section « En 30 secondes », tableau des boutons expliqués, FAQ (serveur invisible → ⟳, code d'accès, logs, auto-boot). Anciens assets décoratifs retirés (explorateur.png, boutique.png, banner.jpg).

### Documenté (non corrigé — voir AUDIT-2026-10-25.md)
- `panic::set_hook` journalisant sur disque : recommandé pour que le prochain panic ne soit jamais muet.
- `last_seen` absent de `/api/detail` : impossible de distinguer « UP stable » de « UP figé » dans l'UI.
- `Cache-Control: no-store` / `Referrer-Policy: no-referrer` absents.
- `poll_all` séquentiel : 32 serveurs × sonde réseau → un cycle dépasse déjà `refresh_secs` (15 s).
- `manage.rs` a franchi 4 500 lignes (4 596) ; constructeur `Server::new()` à factoriser.
- Code d'accès jamais effacé de `localStorage` ; `window.open` sans `noopener` ; capability Tauri trop large (10 ports) ; handlers inline à migrer vers `addEventListener` + CSP nonce ; rendu DOM à reconstruire intégralement à chaque cycle.

### Testé
- 75 tests verts, `clippy --all-targets -D warnings` propre, `cargo fmt --check` propre.
- Auth revérifiée en dynamique contre l'instance live (bind `0.0.0.0`) : 401 sans code, 403 sur `Origin`/`Host` hostiles, 200 avec le code, preuve de pair HMAC à usage unique (rejeu = 401, nonce inventé = 401), et `POST /api/action add stdio calc.exe` **sans code = 401** (RCE bloquée).

## [0.7.7] — Détection au max (multi-chemins + fingerprint FastMCP), mémoire au repos, E2E complet

### Ajouté
- **Détection multi-chemins** : chaque port découvert est sondé sur `/mcp`, puis `/`, puis `/api/mcp` — un serveur MCP monté à la racine ou sous `/api/mcp` est désormais trouvé (test unitaire : serveur racine-only découvert, non-MCP rejeté). Le chemin gagnant est conservé dans le serveur découvert.
- **Fingerprint FastMCP** : badge teal « FastMCP » sur les serveurs répondant à la double signature httpStream de mcp-proxy (réponses initialize encodées SSE **et** endpoint `/sse` legacy co-servi) — même mécanique que le badge LiteMCP, précédence au plus spécifique. **La spec MCP utilisée par chaque framework est affichée** (tooltip du badge + ligne framework du détail : « LiteMCP (Rust · stateless 2026-07-28) », « FastMCP (… · spec 2025-06-18) »). Tests : signature à 4 signaux, un-seul-signal rejeté, précédence.
- **Mémoire/CPU au repos** : le polling UI (5 s) est suspendu quand la fenêtre est masquée (tray) — reprise immédiate au retour (`visibilitychange`).

### Testé
- **E2E de la release installée** : chaque bouton de l'UI exercé dans un vrai navigateur (scan, scan réseau, machines, boutique, ajout, tri ⟳, onglets, replis) + toutes les actions API (toggle/start/kill/remove, reorder, import, install_store, autostart roundtrip) — zéro erreur console, état restauré.

## [0.7.6] — Logo Rust (« R » engrenagé) pour le chip de langage

### UI
- **Le chip de langage des serveurs Rust affiche désormais le vrai glyphe** — le « R » cerclé d'un engrenage (SVG vectoriel maison, teinte orange Rust du thème), même lecture que l'icône `.rs` des éditeurs — à la place du monogramme texte « RS ». Dans l'explorateur et l'en-tête du détail, inféré par en-têtes HTTP, process propriétaire du port, ou framework (LiteMCP = Rust par définition).

## [0.7.5] — Auto-scan au démarrage sur TOUS les ports en écoute + bouton ⟳ qui découvre

### Corrigé
- **Pourquoi un serveur sur 8787 restait invisible** : la découverte au boot ne couvrait que la plage `scan_from..scan_to` (3000-3200 par défaut) et le bouton ⟳ ne faisait que re-sonder les serveurs déjà connus — aucun des deux ne pouvait voir un port hors plage.

### Ajouté
- **Auto-scan au démarrage, tous ports confondus** : au boot, l'app lit les ports TCP en écoute de la machine directement depuis l'OS (`netstat -ano` / `ss -tln` + repli `netstat -tln`, parse existant, ports système écartés) et ne sonde que ces ports-là avec `/mcp` — plus aucune plage à deviner, un serveur MCP sur n'importe quel port est trouvé en quelques secondes. La plage `scan_from/scan_to` reste disponible via le bouton 🔍 et l'action `scan` (plage explicite), et pour le scan réseau.
- **Bouton ⟳ fonctionnel** : il re-sonde les serveurs connus ET lance la découverte (action `discover`) en parallèle — un seul clic actualise tout et fait apparaître les nouveaux serveurs.

## [0.7.4] — Plus jamais de code pour l'usage local

### Modifié
- **Le code d'accès n'est exigé que des clients du RÉSEAU** : les connexions venues de la machine elle-même (fenêtre desktop, navigateur local sur 127.0.0.1) n'ont **jamais** de code à saisir, même quand l'UI est bindée `0.0.0.0` pour la détection de pairs — plus de dialogue de déverrouillage à l'ouverture de l'app. Les contrôles Host/Origin (anti CSRF / DNS-rebinding) et la CSP restent appliqués à toutes les connexions, locales incluses ; un client du LAN doit toujours présenter le code (ou une preuve de pair HMAC).

## [0.7.3] — Menu de reclassement des serveurs (⇅)

### Ajouté
- **Menu « ⇅ » dans l'explorateur** pour reclasser les serveurs en un clic : **A→Z** (label, insensible à la casse, collation française), **état** (UP d'abord, puis PORT-ONLY/DOWN, désactivés en fin), **port** (stdio en fin), **latence** (du plus rapide au plus lent, non sondés en fin) et **tools** (du plus outillé au moins outillé). Réutilise l'action API `reorder` : l'ordre appliqué est **persisté** (settings.server_order) et gouverne l'affichage ET la priorité de sondage, comme le glisser-déposer.
- **« ⇅ manuel »** rend l'ordre posé au glisser-déposer AVANT le tri (mémorisé côté client au moment où un tri est appliqué) ; poser un drag à la main remet le menu sur « manuel » et réactualise la mémoire.

## [0.7.2] — Instance unique (reconnexion au moteur existant) + partie HTTP invisible

### Ajouté
- **Instance unique renforcée — « ça se reconnecte » au lieu d'ouvrir un autre** : au lancement, si un moteur command-center répond déjà sur le port UI voulu (détection par `GET /api/hello`, retry ×2), le nouveau process s'y **reconnecte** et n'en lance PAS un second — plus de moteurs jumeaux sur 5091/5092 qui se disputaient les spawns auto, réécrivaient le config.json et doublaient les scans. Le port saute au suivant UNIQUEMENT si l'occupant est un logiciel étranger (comportement historique conservé). Desktop relancé → `tauri_plugin_single_instance` réaffiche/focalise la fenêtre existante (y compris depuis le tray) ; headless relancé alors qu'un moteur vit (desktop OU headless) → message « un moteur tourne déjà sur … — rien à lancer » et sortie propre. Testé : deux instances successives sur le même port → une seule écoute.
- **Journal disque `mcc.log`** (à côté du config.json, rotation à 512 Ko → `mcc.log.old`) : la partie HTTP tourne parfois sans AUCUNE console (autostart `--hidden`, fenêtre masquée dans le tray) — sans fichier, aucun support ni historique possible (point noté depuis l'audit v0.6.3). Horodatage UTC lisible (`iso_utc`, algorithme civil_from_days, testé contre dates connues dont 29/02 et siècles).

### Modifié
- **Partie HTTP invisible** : le binaire headless lancé avec `--hidden` (cas autostart) **masque sa fenêtre console** (`GetConsoleWindow` + `ShowWindow(SW_HIDE)`, sans crate externe) — plus aucune fenêtre ne s'ouvre au boot de l'OS. Lancé à la main sans flag : la console de dev reste (journal live). Le desktop reste fenêtré sans console (`windows_subsystem = "windows"` en release).
- Le headless annonce le journal disque à son démarrage.

## [0.7.1] — Audit 2026-10-24 : auth réelle (login UI + preuve de pair HMAC), hygiène

### Sécurité
- **P0 — le code d'accès n'est plus servi par la page `/`** : l'UI embarquait le code d'accès API en clair (placeholder `__MCC_ACCESS__`) — quiconque pouvait joindre le port le lisait sur la page d'accueil et contournait toute l'auth `/api/*` (y compris `add` stdio = exécution de commande arbitraire). Le code est désormais demandé à l'utilisateur par un dialogue de déverrouillage, gardé en `localStorage` et présenté à chaque appel `/api/*` ; un 401 rouvre le dialogue (code tourné ou faux). En bind loopback, rien ne change (aucun code requis). Test de non-régression : `/` ne contient jamais le secret.
- **P1 — la découverte de pairs ne diffuse plus le code d'accès** : `discover_peers` envoyait NOTRE code en clair (`X-MCC-Access`) à toutes les cibles sondées du LAN — tout hôte à l'écoute sur 5090/5091 récupérait le secret sans le demander. Protocole en deux phases : `GET /api/hello` (préauthentifié, nonce à usage unique, aucune donnée) puis `POST /api/state` avec `X-MCC-Nonce` + `X-MCC-Proof` = HMAC-SHA256(code, nonce). Nouveau module `crypto.rs` (SHA-256/HMAC en std pur, zéro dépendance, validés contre les vecteurs NIST/RFC 4231). NB : les pairs doivent être ≥ 0.7.1.
- **Rotation du code d'accès + purge des artefacts qui le contenaient** : l'ancien code traînait en clair dans des captures e2e non gitignorées (`ui-e2e-*`, supprimées). `.gitignore` couvre désormais `state-cache.json`, `ui-e2e-*`, `pauvre-router-audit/`, `nul`, `patch_*.py` ; fixtures de test neutralisées (IP de documentation `192.0.2.10` au lieu du LAN réel).
- **CSP sur la page UI** (en-tête `Content-Security-Policy` servi par le core — le `csp` de tauri.conf.json ne s'applique pas aux URL externes chargées par le webview) : scripts/styles inline uniquement (page mono-fichier), `connect-src 'self'`, aucune frame — une XSS future ne peut ni charger de code externe ni exfiltrer. `X-Content-Type-Options: nosniff` sur toutes les réponses. Capability desktop resserrée aux ports 5090-5099 (plus de wildcard `:*` tous ports) et `csp` posé aussi dans tauri.conf (page de repli locale).
- **CI durcie** : actions GitHub épinglées par SHA de tag (checkout, toolchain, rust-cache, upload/download-artifact, gh-release), `contents: write` réservé au job release, clippy strict `--all-targets`, et nouveau workflow **CI sur chaque push/PR** (fmt + tests + clippy) — avant, les tests ne tournaient qu'au push de tag.

### Corrigé
- **Panic HTTP sur import tronqué** : `import_mcp` avec `claude mcp add` (3-4 tokens) indexait `toks[3]`/`toks[4]` → panic du thread, aucune réponse au client. Borne `≥ 5 tokens` + test de non-régression.
- **`kill_port` porté sur Linux/macOS** (no-op depuis v0.6.3 alors que le desktop est distribué sur 4 OS) : parse `ss -tlnp` par port exact (propre PID exclu, testé) + repli `lsof -sTCP:LISTEN`.
- **Vérification de mise à jour sur les trois OS** : elle ne passait QUE par PowerShell (silencieusement inopérante sur Linux/macOS) — `curl` d'abord, PowerShell en repli.
- **Caps mémoire sur les sondes sortantes** : lectures de réponses plafonnées (2 Mo JSON-RPC, 256 Ko SSE//health) — un serveur LAN rapide ne peut plus gonfler la RAM pendant la fenêtre de timeout.
- **Path de sonde sanitizé** côté client : aucun CR/LF ni espace dans la ligne de requête (injection d'en-tête impossible même avec un path configuré malveillant).
- **Divers** : warning clippy `unreachable_code` éliminé (corps autostart séparé du garde `cfg(test)`), test vide supprimé, fixture `examples/test-http-server.js` rebondie sur `[::1]` (elle écoutait sur toutes les interfaces, contrairement à son commentaire), `start-command-center.bat` vérifie le build avant de lancer, fichiers one-shot de dev supprimés de la racine.

## [0.7.0] — Support LiteMCP : fingerprint du framework + badge « glow HTTPS »

### Ajouté
- **Replis du détail à état mémorisé** : « Démarrage » et « Informations » retenaient leur état codé en dur — réimposé à chaque cycle de rafraîchissement (5 s) et à chaque réouverture d'onglet. L'état ouvert/fermé choisi est désormais persisté (localStorage, `mcc.fold.*`) et relu au rendu : le triangle reste où tu l'as laissé, y compris après fermeture de l'onglet.
- **Bouton « Auto-boot ON/OFF » dans la barre de titre** : choix de lancer (ou non) l'app au démarrage de l'OS. Préférence persistée (`settings.autostart`, défaut true) et appliquée par le core au boot et au clic — registre `HKCU…Run` sur Windows, autostart XDG sur Linux, LaunchAgent sur macOS (plus de plugin Tauri qui forçait l'activation à chaque lancement). Garde `cfg(test)` : les tests n'écrivent jamais dans l'OS. Action API `set_autostart {enabled}` ; `autostart` exposé dans `/api/state`.
- **Liens écosystème** : le README lie la lib [LiteMCP](https://github.com/DeamonDev888/litemcp) (fingerprint natif) ; le README de litemcp renvoie vers ce client.
- **README illustré** : nouvelle section « Fonctionnalités principales » avec les captures d'écran de l'explorateur (priorité par glisser-déposer, badges LiteMCP/·LAN·SSH, version) et de la boutique (64 MCP avec logos officiels) — détection sur les autres machines/VPS/Docker via SSH, multi-serveurs, drag & drop, auto-boot, version.
- **Drag & drop réparé dans la fenêtre native** : les lignes ne bougeaient qu'en navigateur — le handler de drop de fichiers de WebView2 (activé par défaut dans Tauri) avale les événements HTML5 dragstart/over/drop. `disable_drag_drop_handler()` sur la fenêtre (aucun usage du drop de fichiers dans l'app) — le glisser-déposer de l'explorateur fonctionne désormais à la souris dans la fenêtre Tauri.
- **Version affichée + vérification de mise à jour** : badge `v0.7.0` permanent dans la barre de titre (injecté au rendu). Au boot puis chaque heure, l'app interroge la dernière release GitHub via PowerShell système (aucune dépendance TLS ; échec réseau = silence) : si une version PLUS RÉCENTE existe, un badge ambre « ⬆ vX.Y.Z dispo » apparaît (clique = page releases) et le journal le note. Comparaison numérique par segments (`0.10.0 > 0.9.9`, préfixe `v` toléré, testé) ; `update_available`/`latest_version` exposés dans `/api/state`.
- **Glisser-déposer des serveurs (préférence et priorité)** : les lignes de l'explorateur sont réordonnables par drag & drop (marqueur d'insertion au-dessus/en dessous, hint « glisser = priorité »). L'ordre est persisté dans `settings.server_order` (config.json, action API `reorder` — ids inconnus filtrés, manquants réajoutés en fin) et gouverne **l'affichage ET l'ordre de sondage** (au boot le premier cycle est séquentiel : les serveurs en haut sont sondés en premier). La suppression purge l'entrée. Config de l'app installée séparée du dossier dev (`resolve_config_path` privilégie `exe_dir/config.json` existant — un exe installé ne réécrit plus la config du dossier dev).
- **Cache de démarrage (état dernier connu)** : chaque cycle de poll enregistre l'état courant dans `state-cache.json` (à côté du config.json, écriture atomique, skippée si inchangé) : nom, version, protocole, tools complets, langage, framework, via_ssh, latence, dernier vu. Au boot, AVANT la première sonde, le cache restaure les serveurs connus (par id, activés seulement) — l'UI est peuplée instantanément au lieu d'attendre 1-2 min de sondes séquentielles (stdio spawns inclus) ; les sondes confirment/corrigent en arrière-plan. Preuve mesurée : 3 s après le boot, 30 serveurs restaurés, 569 tools via SSH déjà affichés. Test roundtrip + protection des serveurs désactivés.
- **Icônes garanties sur les trois OS** : `icon.png` 512 + `icon.ico` multi-tailles 16→256 (Windows : embarquée à l'exe, barre des tâches, tray) + **`icon.icns` généré** (128/256/512, ajoute au bundle pour le Dock macOS) + fenêtre `.icon()` explicite (Linux : _NET_WM_ICON pour la barre des tâches, tray partout). `bundle.icon` référence les trois formats ; limitation macOS documentée : un binaire nu (non bundlé) montre l'icône générique du Dock — l'icône tray (menu bar) reste la nôtre.
- **Fingerprint du framework serveur** : nouveau champ de sonde `framework` — `"litemcp"` quand les DEUX signatures documentées de litemcp sont réunies : `initialize` répondu en `2026-07-28` **sans négocier** la version demandée par le client (le probe émet `2024-11-05`, litemcp stateless strict l'ignore) **et** sa sonde `GET /health` → `200` « ok ». Un seul signal ne suffit pas (specs à jour ailleurs, `/health` génériques) ; la sonde `/health` ne part que si l'initialize a répondu 2026-07-28 — aucune requête de plus pour les serveurs qui négocient normalement.
- **Badge UI « LiteMCP »** (style « glow HTTPS ») : halo violet pulsant (`glowPulse`) sur les serveurs fingerprintés, dans la liste et l'en-tête du détail ; ligne `framework` dans les informations du détail. `/api/state` et `/api/detail` exposent `framework`.
- **Langage rust par déduction** : un serveur litemcp est une lib Rust par définition — le chip de langage affiche `RS` quand ni les en-têtes ni le process écoutant le port n'ont permis d'inférer.
- **Seed `seed-litemcp`** : la démo officielle de litemcp (`examples/hello` — 3 outils : hello, timestamp, search) lancée via le binaire release (`cargo build --release --example hello` du dépôt litemcp), port 8080. Start/Stop opérationnels depuis l'UI ; binaire manquant → le mécanisme « 3 échecs → auto-restart off » s'applique.
- **Glow sur le badge HTTPS** : le badge `HTTPS` (port 443) gagne un halo violet statique — même famille visuelle que le badge framework.

### UI
- **Sonde MCP interne via SSH (le vrai fix)** : une fois la machine logée, TOUT se passe depuis la machine — plus aucune sonde externe. Un seul exec SSH boucle sur les ports avec `curl` interne sur `127.0.0.1` : passe les binds localhost ET les pare-feux (ufw). Les serveurs importés sont marqués `via_ssh` (persistés dans config.json, badge « ·SSH » dans l'UI, ligne « sonde : via SSH » au détail) et **pollés par SSH** à chaque cycle : initialize + tools/list dans la même connexion, avec gestion de l'en-tête `Mcp-Session-Id` (capture -D, extraction sanitisée, rejeu — nécessaire aux serveurs sur SDK officiel qui rejettent tools/list sans session). Pièges corrigés au passage : `${p}` jamais développé dans les quotes simples du curl distant, double-quotes interdites dans la commande distante (échappement Windows ssh), extraction du SID qui avalait les deux-points.
- **Persistance des serveurs via_ssh** : `save_config` garde les Discovered marqués via_ssh (leur re-découverte exige une connexion SSH réussie) — ils survivent au redémarrage et sont re-pollés au boot.
- **Machines distantes : découverte SSH des serveurs MCP** : « Détecter les pairs » devient **« Choisir une machine »** (bouton titlebar + écran d'accueil). Nouveau dialog Machines : IP + utilisateur SSH (enregistrables dans `settings.ssh_hosts`) → l'app se connecte avec le **client ssh système** (aucune dépendance, OpenSSH de Windows), récupère le **netstat distant**, filtre les ports système, sonde chaque port restant et **importe les serveurs MCP trouvés**. Auth par **clé obligatoire** (BatchMode) ; erreurs ssh restituées en clair dans le journal avec indication (clé manquante, machine injoignable). Actions `ssh_save` / `ssh_forget` / `ssh_discover` ; sanitize anti-injection des champs (testé).
- **Scan en un clic** : « Scanner les ports locaux » et « Scanner le réseau » ne demandent plus de plage — ils utilisent la plage configurée (`settings.scan_from/scan_to`), zéro `prompt()`.
- **Écran d'accueil refait** : « Centre de contrôle de tes serveurs MCP », tuile **machines distantes**, trois boutons d'action directs et une ligne d'aide sur la découverte SSH. Section latérale « Command centers (pairs) » renommée « Machines & pairs ».
- **Icône personnalisée de l'application** : l'ancien carré bleu uni est remplacé par un vrai logo (hub MCP lumineux connectant 6 satellites sur tuile sombre arrondie, généré procéduralement). `icon.png` 512 + `icon.ico` multi-tailles (16→256) ; icône posée explicitement sur la fenêtre (`.icon()` — barre des tâches et Alt+Tab) et sur l'icône tray ; embarquée comme ressource de l'exe (Explorer, Task Manager, releases GitHub).
- **Icônes de marque manquantes de la boutique** : +31 glyphes Simple Icons posés par id exact de carte (ElevenLabs, MySQL, Redis, Android, Chrome, Jupyter, Dynatrace, Elasticsearch, Grafana, Hugging Face, CircleCI, Postman, Pulumi, Railway, Terraform, Webflow, Trello, Bitbucket, LeetCode, arXiv, SearXNG, Box, Mermaid, Figma Dev Mode, GitHub Integration, AKS, Sentry local, Notion, MongoDB, Supabase, JFrog) — `logos.rs` passe à 64 logos ; les rares sans marque officielle gardent le glyphe « paquet » neutre.
- **Catalogue GitHub (+4 items populaires)** : les serveurs MCP installables les plus étoilés du dépôt GitHub (topic `mcp-server`) absents des autres catalogues — Chrome DevTools (54k★), Codebase Memory (47k★), Context Mode (32k★), Serena (30k★, `uvx --from serena-agent`). Nouveau module `store_github.rs` ; les reconaissances de logo par commande couvrent les quatre préfixes de catalogue (`at-`/`gl-`/`gh-`).
- **Catalogue Glama (+6 items populaires)** : les serveurs MCP les plus populaires du classement glama.ai (tri GitHub stars) absents de la boutique — GitLab, Sentry, Google Maps, PaddleOCR, CodeGraph, World Monitor — paquets vérifiés sur npm/PyPI, clés d'env documentées. Nouveau module `store_glama.rs` ; les logos GitLab, Sentry, Google Maps et PaddlePaddle rejoignent `logos.rs` (33 logos au total), les items des catalogues additionnels sans marque reconnue reçoivent un glyphe « paquet » neutre.
- **Compteur dans le titre de la boutique** : « Boutique — serveurs MCP (60 MCP) » — mis à jour automatiquement au chargement du catalogue.
- **Boutique élargie au catalogue aitmpl (+25 items)** : les serveurs MCP de aitmpl.com (dépôt `davila7/claude-code-templates`, MIT) rejoignent la boutique — 54 items au total, nouvelles catégories (Audio, Intégrations, Recherche). Dédupliqués contre le catalogue principal (même paquet npm ou même id → écartés) et limités aux installations `npx`/`uvx` (41 items à commande docker/binaire écartés). Chaque item porte les clés d'env requises dans sa description. Nouveau module `store_aitmpl.rs` (généré) ; `install_store` et `/api/store` couvrent les deux catalogues ; logos par marque reconnue dans la commande, sinon glyphe « paquet » neutre.
- **Bloc « Collez une config… » retiré de la boutique** : la zone d'import par collage (textarea + Importer) est supprimée de l'UI — l'action API `import_mcp` reste disponible pour les scripts.
- **Logos officiels dans la boutique** : chaque carte affiche le logo vectoriel officiel de sa marque (19 glyphes Simple Icons : Google Drive, Git, GitHub, Puppeteer, Brave, PostgreSQL, SQLite, Slack, Playwright, DuckDuckGo, Kubernetes, Supabase, MongoDB, Stripe, Notion, Blender, Figma, n8n, Todoist — teintés aux couleurs officielles, adaptées au fond sombre pour GitHub/Notion/SQLite) et des glyphes génériques cohérents pour les serveurs sans marque (Filesystem, Memory, Sequential Thinking, Everything, Time, Fetch, Context7, Task Master, Firecrawl, Exa). Embarqués au binaire (nouveau module `logos.rs`, +34 Ko) et servis par `GET /api/store-logos`.
- **« Retirer durablement » un seed** : nouveau bouton (vue détail d'un serveur seed, avec confirmation) qui supprime le seed ET consigne son id dans `settings.removed_seeds` (config.json) — il n'est plus réinjecté au boot, contrairement au simple « Retirer » qui ne survit pas au redémarrage. Réinitialisation possible en vidant `removed_seeds` dans config.json. L'action `POST /api/action {"action":"remove","id":…,"durable":true}` porte la fonction.
- **Vie dans la zone de notification (tray)** : fermer la fenêtre la **masque** — l'app continue de tourner et de superviser les serveurs, icône dans le tray (clic gauche = afficher/masquer, menu clic droit : « Afficher le tableau de bord » / « Quitter » ; seule vraie sortie). Deuxième lancement pendant que l'app tourne : la fenêtre existante est réaffichée (`tauri-plugin-single-instance`) au lieu d'un second moteur condamné par les ports UI.
- **Démarrage automatique avec Windows** : l'app s'inscrit dans `HKCU\...\Run` (`tauri-plugin-autostart`, entrée « MCP Command Center --hidden ») — au boot, elle vit dans le tray et lance/supervise les serveurs (`auto_restart` des seeds) sans ouvrir de fenêtre. À retirer depuis le Gestionnaire des tâches › Applications de démarrage si besoin.
- **Barre de fenêtre unique et sombre (desktop)** : décorations natives supprimées (`decorations: false`) — fini la barre de titre Windows blanche au-dessus du header. Le header de l'UI devient la barre de fenêtre : zone de drag (`data-tauri-drag-region`, double-clic = agrandir) et boutons réduire/agrandir/fermer à droite (masqués dans un navigateur classique, où `__TAURI__` n'existe pas). Nouvelle capability `main-remote-ui` (drag + contrôles de fenêtre pour l'origine http locale du moteur) et `withGlobalTauri: true`.
- **Vue détail allégée** : la grille « Informations » (12 lignes) passe sous un repli `<details>` fermé par défaut — la page ne montre plus que l'essentiel.
- **Nouveau repli « Démarrage »** (ouvert) : état du contrôle du process — `managed` (lancé et maintenu par ce command center, issu de la map children), état du process (**« au démarrage… »** quand le process est vivant mais que MCP n'a pas encore répondu, « en marche », « arrêté », « port occupé », « désactivé »), pid, auto-restart, commande de lancement complète. L'API expose désormais `managed` et `start_cmd` dans `/api/state` et `/api/detail`.
- **État « AU DÉMARRAGE »** : le pill et le point de l'en-tête passent en style `STARTING` (accent pulsant) pendant la fenêtre spawn → première réponse MCP.

### Vérifié
- Tests du fingerprint : pur (`detect_framework` exige les deux signaux), sonde `/health` (200 « ok » vs JSON sans « ok »), intégration — faux serveur litemcp (initialize 2026-07-28 + tools/list + /health) → `framework = "litemcp"` + `lang = "rust"` ; sans `/health` → pas de fingerprint.

## [0.6.6] — Sécurité : code d'accès API, anti CSRF/DNS-rebinding, kill_port exact

### Sécurité (audit)
- **Code d'accès API** : exigé sur `/api/*` dès que le bind n'est pas loopback (header `X-MCC-Access` ou `?access=`). Généré au boot si absent et persisté (`settings.api_access`) ; pour la détection de pairs, mettre la **même valeur** des deux côtés (la sonde de pairs l'embarque désormais). Source alternative : variable d'environnement `MCC_API_ACCESS`.
- **Bind par défaut `127.0.0.1`** (était `0.0.0.0`) : l'ouverture LAN est désormais un choix explicite de l'opérateur, protégé par le code d'accès.
- **Anti CSRF/DNS-rebinding** : `OPTIONS` → 403 (préflights morts), header `Host` comparé à la liste des hosts légitimes (loopback + IP locales + `settings.allowed_origins`), `Origin` hors liste → 403, `POST /api/action` exige `Content-Type: application/json` (bloque les POST « simples » text/plain exécutables sans préflight depuis un site web). `Access-Control-Allow-Origin: *` remplacé par un reflet **uniquement** des origines autorisées.
- **kill_port par colonne** : le match par sous-chaîne (`:3141` matchait `:31410`) tuait le mauvais PID via taskkill — parse exact du port, exclusion de notre propre PID (`pids_listening_on`, testé).
- **Caps HTTP** : corps de requête plafonné à 2 Mo (413 au-delà — l'ancien code suivait un Content-Length arbitraire), connexions simultanées plafonnées à 64.
- **Boutique Filesystem** : le dossier exposé est demandé explicitement à l'installation (UI prompt / `args`), le catalogue n'embarque plus de chemin personnel.

### Corrigé (robustesse)
- Seeds introuvables (machine cliente) : après 3 échecs de spawn consécutifs, l'auto-restart se désactive (fini la boucle d'essais/warnings toutes les 15 s).
- Course double-spawn : le poller et une action API ne peuvent plus démarrer le même serveur en même temps (garde par id).
- `save_config` : les erreurs d'écriture sont loguées (avant : ajout perdu silencieusement au reboot).
- `local_ips` sur Linux/macOS : `hostname -I` est réellement parsé (avant : 0 IP parsée, fallback UDP seul → sous-réseaux manqués).
- `Host:` exact envoyé aux serveurs (`host:port`, IPv6 crochété) au lieu de `localhost` en dur (vhosts sur hôte distant).
- Import `https://` refusé avec un message clair (pas de TLS dans ce client std — une URL https finissait en sonde plaintext et en DOWN permanent).

### UI
- Wrapper `fetch` global : le code d'accès est injecté dans la page servie par le moteur (même origine) et embarqué dans chaque requête `/api/`.
- Install Filesystem : prompt du dossier à exposer ; l'install sans dossier est refusée côté backend.

### Breaking
- `POST /api/action` sans `Content-Type: application/json` est refusé (415) — mettre à jour les scripts curl (les exemples du README l'avaient déjà).
- Sur un bind LAN sans `api_access`, un code est généré au premier boot : lire `config.json` pour le récupérer.

## [0.6.5] — Fini les faux badges SSE : les FastMCP httpStream sont étiquetés HTTP+SSE

### Corrigé (classification transport)
- Tous les serveurs FastMCP lancés en `transportType: httpStream` répondaient badge « SSE ». Cause : mcp-proxy encode les réponses JSON-RPC en événements SSE (`Content-Type: text/event-stream`, `enableJsonResponse=false` par défaut) tout en restant du **streamable HTTP** — et FastMCP co-sert aussi `/sse` (compat 2024-11-05). Le Content-Type de l'initialize ne qualifie PAS le transport.
- Nouveau champ de sonde `sse_endpoint` : GET /sse considéré legacy seulement si le flux contient un vrai `event:` (un POST /mcp répondu = streamable HTTP, point).
- Badge UI : « HTTP+SSE » quand le serveur répond en streamable ET co-sert /sse (signature FastMCP/mcp-proxy), « HTTP » streamable JSON, « SSE » réservé aux réponses event-stream sans endpoint streamable prouvé. Info visible au survol.
- `/api/state` expose `sse_endpoint` ; `sse` conserve sa signification (encodage SSE des réponses).

Preuve : Overmind 3099 / discord 3141 / postgres 5433 / X 3142 → HTTP+SSE ; fixtures test-sse (réponses data: sans event) et JSON chunked → HTTP ; flux `event: endpoint` → SSE. 35 tests verts, clippy -D warnings propre.

## [0.6.5] — UI honnête : « Retirer » masqué sur les seeds + refresh parallèle

### Corrigé (audit 2026-10-19, F6/F7)
- **« Retirer » masqué pour les serveurs seed** (origin: seed) : leur retrait ne survivait pas au reboot (`load_config` réinjecte les 7 seeds) — un bouton qui promettait ce qu'il ne tient pas. Une note dans la vue détail l'explique et renvoie à **Désactiver** (persisté, lui). Les serveurs manuels/boutique gardent leur « Retirer ».
- **`refreshAll()` parallélisé** : les sondes des serveurs activés partent en `Promise.all` avec un seul rechargement d'état à la fin — avant, N rafraîchissements séquentiels pouvaient prendre N × timeout.
- **`ui-dist/index.html` (frontendDist Tauri)** : la redirection figée vers le port 5090 est remplacée par une sonde 5090→5099 qui rejoint l'URL réelle quand le core a glissé de port — le fallback mort de l'audit redevient vivant.
- Versions alignées sur 0.6.5 (tauri.conf.json traînait à 0.6.4) + `cargo fmt` appliqué au préexistant `probe.rs` (rustfmt de toolchain récente).

### Vérifié
- 35/35 tests, `cargo fmt --check` OK, clippy 0 warning.
- E2E local (app lancée avec `MCC_CONFIG` de test, 13 serveurs désactivés — aucun launcher spawné) : HTML servi contient les 3 modifs, JS embarqué compile (vm), contrat `/api/detail` (`origin` seed×7 / manual×6), et rendu `renderDetail` exécuté contre les vraies réponses API — seed : sans « Retirer » + note ; manuel : avec « Retirer », sans note.

## [0.6.4] — Plus aucune fenêtre console qui s'ouvre

### Corrigé (bug bloquant)
- Cliquer Start/Stop (ou chaque poll) ouvrait des fenêtres console : les spawns utilitaires (netstat, tasklist, taskkill, arp, ipconfig, hostname) n'avaient pas CREATE_NO_WINDOW depuis une app GUI. Helper `hidden()` appliqué à TOUS les Command de l'app.
- Kill des serveurs stdio : `Child::kill()` ne tuait que le lanceur .cmd, pas node.exe détaché → processus orphelins et consoles résiduelles. `kill_tree()` (taskkill /F /T /PID) remplace les 6 sites de kill (toggle OFF, kill_stdio, remove, chemins d'erreur handshake).

### Vérifié
- 0 console visible (MainWindowHandle ≠ 0) avant/après 5 toggles + polls.
- Start → UP (pid réel), Stop → process tué côté OS, aucune fenêtre.

## [0.6.3] — Langage détecté par le process réel + chips visibles partout

### Corrigé
- Les serveurs HTTP locaux sans en-têtes n'avaient AUCUN langage → chip absent. Le probe interroge désormais le vrai process qui écoute le port (netstat -ano → PID → tasklist → image, cache 10 s) : node.exe → node, python.exe → python, java.exe → java, dotnet.exe → dotnet…
- Normalisation de l'image (`node.exe` → `node`) pour que la chip s'affiche.
- Chip « ? » explicite quand le langage est indétectable (au lieu de rien).
- La fenêtre native desktop embarquait un vieux core (v0.6.0) → rebuild aligné sur le core courant.

Preuve : Overmind/discord/postgres/X/NEWS détectés `node` via leur process node.exe ; py-header → python (header uvicorn) ; stdio → node (commande).

## [0.6.3] — Desktop multi-OS (Windows, Linux, macOS Intel + Apple Silicon)

### Distribution
- Le job CI `desktop` construit désormais la fenêtre native Tauri sur les 4 cibles : `windows-x86_64`, `linux-x86_64`, `macos-aarch64` (Apple Silicon), `macos-x86_64` (Intel).
- Linux : dépendances WebKitGTK installées dans le job (`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`).
- Assets publiés : `mcp-command-center-desktop-{windows-x86_64.exe, linux-x86_64, macos-aarch64, macos-x86_64}` en plus des 4 binaires CLI.

Note Linux : le binaire desktop utilise la webview système (WebKitGTK) — le paquet d'exécution `libwebkit2gtk-4.1-0` est requis (standard sur Ubuntu 24.04+).

## [0.6.2] — Langage des serveurs MCP + logos

### Frontend
- Chip de langage (monogramme coloré façon logo) à côté du badge de type : JS (Node/TS), PY, RS, GO, JV, C#, RB, PHP. Survol = libellé complet. Badge absent si le langage est inconnu (aucune invention).
- Transport conservé : STDIO / HTTP / SSE / HTTPS (+ ·LAN).

### Backend
- Détection du langage — la spec MCP n'expose PAS le langage (serverInfo = name/title/version uniquement) :
  - stdio : commande de lancement (node/npx/tsx → Node, python/uv/uvx → Python, cargo → Rust, go/java/dotnet/ruby/php), extension du script en dernier recours.
  - HTTP : en-têtes Server / X-Powered-By (uvicorn/gunicorn/werkzeug → Python, express → Node, hyper/axum → Rust, Kestrel → .NET, Jetty/Tomcat → Java, net/http → Go), sinon indice dans le nom.
- RespHead (content-type/server/x-powered-by) remplace le simple content-type ; `lang` exposé dans /api/state.
- Fixture `examples/test-py-header-server.js` (se présente comme uvicorn) pour tester l'inférence.

Preuves : py-header → python (header uvicorn) ; Filesystem/demo-stdio → node (commande) ; serveurs réels sans en-tête → lang inconnu.

## [0.6.1] — Badges du type de MCP (STDIO / HTTP / SSE / HTTPS)

### UI
- Chaque serveur affiche son type de transport avant le badge « désuet » : STDIO (cyan), HTTP (violet), SSE (vert, détecté au probe), HTTPS (port 443), suffixe ·LAN pour les serveurs distants.
- Même badge dans la vue détail à côté du pill d'état.

### Backend
- Détection SSE : Content-Type `text/event-stream` capturé sur la réponse initialize → champ `sse` (McpProbe/Server) exposé dans /api/state.
- Fixture `examples/test-sse-server.js` (serveur MCP SSE zéro dépendance) pour tester.

Preuve réelle : Overmind, discord, postgres, X, NEWS détectés SSE ; demo-http (JSON chunked) HTTP ; stdio → STDIO.

### Lancement des serveurs locaux
- Seeds : chaque serveur embarque désormais sa commande de lancement réelle (`node <workdir>/bin/launch.cjs` + workdir du launcher MCP MONITOR v7.3) → les boutons ▶/■ fonctionnent même sans `config.json` (exe distribué, premier démarrage). Corrige les boutons absents (`can_start: false`) après réécriture de la config.
- Start : re-probe borné (30 s max, pas de 1 s) après le spawn — les launchers font `npm run build` avant d'écouter.
- Stop : tue aussi le process qui écoute réellement le port (netstat → taskkill, local uniquement) car le launcher détache le serveur ; état DOWN immédiat (boucle de vérification bornée).
- Spawn détaché (`CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS`) : les serveurs survivent au shell et à un Ctrl+C dans la console de l'app.

Preuve réelle : dossier vierge sans config → `can_start: true` sur les 7 seeds ; Start Overmind → UP 16 tools ; Stop → DOWN immédiat, port 3099 fermé (3,7 s).

## [0.6.0] — App NATIVE (Tauri) + CLI headless

### Architecture
- Workspace cargo : `mcp-command-center` (core headless, lib + bin) + `mcp-command-center-desktop` (Tauri 2).
- Le core devient une LIBRAIRIE : tout le moteur (config, probes, polling, HTTP, boutique) est dans `src/lib.rs` ; `run_headless()` démarre tout et retourne l'URL, `park_forever()` bloque.
- `mcp-command-center.exe` = CLI **100 % headless** : plus AUCUNE ouverture de navigateur/fenêtre. Sortie console : URL + endpoints. (Le mode --app Edge/Chrome est supprimé.)
- `mcp-command-center-desktop.exe` = **vraie fenêtre native OS** : Tauri 2 + WebView2 système (déjà fourni par Windows 11), exe autonome 9 MB, `windows_subsystem = "windows"` (aucune console). Le moteur tourne dans le process, la fenêtre navigue vers l'URL locale.
- Lanceurs : `start-command-center.bat` → fenêtre native ; `start-headless.bat` → CLI.

### CI
- Jobs core scopés sur `-p mcp-command-center` (le desktop ne compile pas sur ubuntu sans webkit2gtk).
- Nouveau job `desktop` (windows-latest) : construit la fenêtre native et l'ajoute à la release (`mcp-command-center-desktop-windows-x86_64.exe`).
- Prérequis MSVC : toolchain `stable-x86_64-pc-windows-msvc` + Build Tools 2022 + WebView2 Runtime (tous présents ici).

## [0.5.1] — Audit complet + formatage

### Gates
- cargo fmt appliqué (tout le repo formaté, vérifié par fmt --check) + fmt --check ajouté au rituel de release.
- Corrigé : refresh_secs non clampé (une config avec 0 = busy-loop CPU) → clamp 5..3600.
- Nettoyé : variable batch morte dans scan_ports.

### Audit — feuille de route
Voir README (section Feuille de route) pour la liste complète construire/optimiser/sécuriser.

## [0.5.0] — Fenêtre desktop + Boutique MCP

### Desktop
- L'app s'ouvre maintenant dans une FENÊTRE dédiée (Edge/Chrome --app, style Electron sans la payload) au lieu d'un onglet navigateur. Fallback navigateur. MCC_NO_BROWSER=1 pour désactiver.

### Boutique MCP (portée de l'app Hermes desktop)
- Bouton « Boutique » : catalogue de 14 serveurs MCP officiels (Filesystem, Git, GitHub, Memory, Fetch, Puppeteer, Brave, PostgreSQL, SQLite, Slack…), catégories + recherche, install en un clic.
- Import « collez n'importe quoi » (porté de mcp-import.ts) : snippet mcp.json (mcpServers ou mcp_servers ou map brute), commande npx/uvx/docker/claude mcp add, ou simple URL → serveurs créés.
- Backend : /api/store, actions install_store + import_mcp, parsing tolerant (mcp-doc.ts), nettoyage de noms (@modelcontextprotocol/server-filesystem → filesystem).

### Corrigé (Windows)
- Rust Command::new("npx") échouait : CreateProcess ne résout que les .exe — résolution .cmd dans le PATH ajoutée (npx/uvx fonctionnent).
- Timeout de handshake stdio 6 s → 15 s (npx télécharge le package au premier lancement).

## [0.4.3] — L'app s'ouvre (navigateur auto au démarrage)

### Corrigé (UX bloquant)
- Double-cliquer l'exe ne montrait rien : serveur console silencieux, rien n'ouvrait la UI.
- Au démarrage, le navigateur par défaut s'ouvre automatiquement sur http://127.0.0.1:<port> (le port réel après fallback). Désactivable avec MCC_NO_BROWSER=1.
- L'URL ouverte pointe sur 127.0.0.1 même en bind 0.0.0.0.

## [0.4.2] — Persistance réparée sur la release distribuée

### Corrigé (bloquant)
- La release téléchargée ne persistait RIEN : config.json était écrit vers le chemin compile-time du runner CI (CARGO_MANIFEST_DIR), inexistant sur la machine cliente → tout serveur ajouté disparaissait au restart.
- Résolution au runtime : MCC_CONFIG (override) → CARGO_MANIFEST_DIR/config.json si présent (dev) → dossier de l'exe (release distribuée). Validé par simulation machine cliente : add → restart → serveur conservé, config.json écrit à côté de l'exe.

## [0.4.1] — ON/OFF = vrai interrupteur

### Backend
- toggle OFF tue le process lancé par le command center (stdio ou HTTP) et libère le pid — avant, désactiver un serveur UP laissait son process tourner.
- toggle ON démarre/sonde immédiatement (poll_one) au lieu d'attendre le cycle suivant.

### UI
- Switch ON/OFF sur chaque ligne de l'explorateur (en plus du bouton Activer/Désactiver du détail).

## [0.4.0] — Le command center lance aussi les MCP HTTP/HTTPS

### Backend
- start_cmd optionnel pour les serveurs HTTP : le command center peut les démarrer, les arrêter et les relancer (auto-restart), comme les stdio.
- Actions génériques `start` / `kill` (alias compat `start_stdio` / `kill_stdio`).
- Auto-start HTTP : un serveur HTTP local éteint avec commande de lancement est démarré au prochain cycle de poll, puis sondé (probe → spawn → re-probe). Jamais pour un serveur distant (gate host local).
- Persistance : la commande de lancement (program/args/workdir) est écrite et rechargée depuis config.json pour les HTTP.

### UI
- Form d'ajout HTTP : champs optionnels « commande de lancement ».
- Boutons Start/Stop sur chaque serveur avec commande (HTTP comme stdio) dans l'explorateur et le détail.

## [0.3.1] — Start/Stop stdio + badge « désuet »

### UI
- Bouton Start/Stop directement sur chaque ligne stdio de l'explorateur (▶/■) en plus de la vue détail.
- Badge « désuet » (legacy) sur les serveurs stdio uniquement (explorateur + détail) : pas de transport HTTP.

### Corrigé
- start_stdio ne faisait rien sur un serveur désactivé (poll_one sort sur !enabled) — il réactive désormais le serveur, reconstruit start_cmd et persiste.
- kill_stdio laissait un pid fantôme affiché — pid mis à None et état Down immédiatement.

## [0.3.0] — Refonte frontend « explorateur »

### UI
- Refonte complete : layout IDE (Antigravity) — explorateur a gauche (liste des serveurs MCP + pairs command-center), zone de detail a droite, journal style console en bas (repliable), titlebar avec badges stats et actions.
- Theme repris de l'app Hermes Desktop (skin nous, mode sombre) : chrome #0d0d0e, sidebar #0a0a0b, cards #161618, accent Nous #4a84fe, textes 94/74/54/36%, hover accent 4%.
- Vue detail : infos completes + liste de chaque tool (nom + description) avec toggle ON/OFF individuel, actions Start/Stop/Sonder/Activer/Retirer.
- Vue d'ensemble avec tuiles de stats quand rien n'est selectionne.
- Formulaire d'ajout en modal.

# Changelog

Tous les changements notables de ce projet sont documentés ici.
Format basé sur [Keep a Changelog](https://keepachangelog.com/fr/1.1.0/).

## [0.2.1] - 2026-10-16

### Fixed

- Sémantique d'états : un serveur éteint (port fermé) repasse à `DOWN` au lieu de `PORT-ONLY` — l'état `PORT-ONLY` est désormais réservé au port occupé sans MCP (zombie), comme documenté.

## [0.2.0] - 2026-10-16

### Added

- **Détection des autres command centers sur le réseau** (pairs) : cache ARP + loopback (multi-instances sur la même machine), sonde `/api/state` sur les ports UI configurables (`network_ports`), import en un clic des serveurs du pair avec réécriture `localhost` → IP du pair.
- **Détection des serveurs MCP du réseau local** : scan parallèle (254 hôtes × plage de ports, un thread par hôte) sur tous les sous-réseaux locaux (parse `ipconfig`/`hostname -I`), auto-ajout avec l'IP en host.
- **Transport HTTP distant** : champ `host` optionnel (None = localhost avec fallback `::1`/`127.0.0.1`, Some = hôte du réseau) — sonde, polling et détail fonctionnent sur un MCP d'une autre machine.
- **Toggle ON/OFF par tool** : bouton ON/OFF par tool dans le détail d'un serveur, `tools_disabled` persisté, compteurs (cartes, stats globales, journal) reflètent les tools actifs.
- UI : section « pairs » avec import, badge hôte distant `🌐 ip:port`, bouton Tools par carte, scan réseau et détection de pairs depuis la barre d'outils.
- Réglages réseau : `ui_bind` (défaut `0.0.0.0` pour être détectable), `network_enabled`, `network_ports`.
- 4 nouveaux tests (contrat `/api/state` du pair, rejet des non-command-centers, toggle tool, compteur tools actifs).

### Fixed

- Auto-détection éliminée (une instance ne se détecte plus elle-même via son IP LAN) : exclusion des IP locales + skip du port UI propre + écarts réseau/broadcast en cache ARP.
- `/api/state` accepte désormais POST (contrat de sonde des pairs).
- Scan réseau asynchrone (réponse API immédiate, détections dans le journal).
- Filtrage des interfaces : masques et passerelles d'`ipconfig` écartés (marqueur ligne « IPv4 »).

## [0.1.0] - 2026-10-16

### Added

- Détection des serveurs MCP **HTTP streamable** : handshake `initialize` + `tools/list`, sonde `::1` **et** `127.0.0.1` (les FastMCP locaux bindent en IPv6-only), parsing SSE et réponses chunked.
- Détection et pilotage des serveurs **stdio** : spawn + handshake JSON-RPC ligne-par-ligne, Start/Stop manuels, auto-restart au boot et au crash (flag `auto_restart`).
- **Découverte automatique** par scan de plage de ports (3000-3200 par défaut) : tout port répondant `initialize` avec un `serverInfo` est ajouté comme serveur découvert.
- États à 3 niveaux : `UP` / `PORT-ONLY` (port occupé par un non-MCP ou zombie) / `DOWN` + `DISABLED`.
- **UI web embarquée dans le binaire** : cartes par serveur (état, latence, host de bind, version, protocole, tools), détail cliquable, journal temps réel, scan/ajout pilotables depuis le navigateur.
- API HTTP scriptable : `GET /api/state`, `GET /api/detail?id=`, `POST /api/action` (`refresh`, `toggle`, `scan`, `add`, `remove`, `start_stdio`, `kill_stdio`).
- Persistance `config.json` avec 7 seeds pré-chargés (Overmind, discord, postgres, X, debats, NEWS_BTC, NEWS) et reconstruction `start_cmd` au rechargement.
- Fixtures de test MCP (stdio + HTTP) dans `examples/`, zéro dépendance Node.
- 18 tests unitaires/intégration (`cargo test`) ; client HTTP maison sans aucune dépendance réseau (std + serde_json uniquement).
