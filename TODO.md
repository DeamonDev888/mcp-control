# TODO — Audit Complet MCP Control v0.8.9 (Corrections & Optimisations)

**Date** : 2026-11-25 · **Version auditée** : `v0.8.9` (`HEAD 111b3d0`)
**Périmètre couvert** : **1. MCP Control (Moteur, Supervision, Probe, SSH, Desktop, CI)** · **2. Interface UI** · **3. Gateway & Bridge** · **4. Authentification & Sécurité** · **5. Boutique**
**État de base vérifié** : `cargo clippy --workspace --all-targets -- -W clippy::all` (0 warning), `cargo check --no-default-features` (OK), `cargo check --all-features` (OK), `cargo fmt --check` (OK).

Légende :
- **Priorités** : **P0** = critique (faille de sécurité majeure, perte d'accès permanente ou fonctionnalité cassée à 100 %) · **P1** = important (bug fonctionnel franc, dérive sécurité ou goulot de performance majeur) · **P2** = moyen (robustesse, cas limites, dette technique, UX) · **P3** = finition / documentation.
- **Tailles** : **S** = ≤ ½ jour · **M** = 1–2 jours · **L** = 3+ jours.

---

## Synthèse exécutive — Les 8 urgences `P0` (✅ Lot 1 livré)

| ID | Domaine | Sévérité | Statut | Résumé | Fichier(s) |
|---|---|---|---|---|---|
| **GW-C01** | Gateway & Auth | **P0 · S** | ✅ | **Contournement total de la confirmation humaine (`ApprovalPolicy`) sur `POST /mcp/s/<id>`** : `serve_hybrid_stdio` appelle désormais `auth::confirm::decide` avant `tools/call`, `resources/read` et `prompts/get` (`403` `-32023` tant que non approuvé). | `src/gateway.rs` |
| **AU-C01** | Auth & Confirm | **P0 · S** | ✅ | **Usurpation de l'aperçu de confirmation humaine (`Confirmation Spoofing` / CWE-451)** : `add_server` HTTP+`program` affiche désormais `kind: "lancement"`, et `import_config` partage `parse_servers_doc` (incluant `cwd` + marqueur d'omission d'arguments). | `src/auth/confirm.rs` |
| **AU-C02** | Auth & Session | **P0 · S** | ✅ | **Vol du jeton `admin` d'amorçage sur repli de port (`+1..+9`) ou squat multi-utilisateurs** : `.grant` scelle le port loopback réel, utilisé par `auth::cli`, `main.rs` et `desktop/src/main.rs` (capacités Tauri conditionnées à la preuve `.grant`). | `src/auth/session.rs`, `src/auth/cli.rs`, `src/lib.rs`, `src/main.rs`, `desktop/src/main.rs` |
| **BT-C01** | Boutique & UI | **P0 · S** | ✅ | **Installation Boutique PostgreSQL cassée à 100 %** : `app.js` envoie `[url]` uniquement et `actions.rs` déduplique défensivement le préfixe `it.args`. | `src/ui/app.js`, `src/manage/actions.rs` |
| **MC-C01** | Desktop & Session | **P0 · S** | ✅ | **Blocage `401 session expirée` permanent au retour du tray Windows (`--hidden`) ou après `F5`** : `show_window_and_recover_session` injecte un jeton frais dans `window.__mccRecoverSession` sans recharger la page si la session est encore valide. | `desktop/src/main.rs`, `src/ui/app.js` |
| **MC-C02** | Desktop & Core | **P0 · S** | ✅ | **Instance existante (`reused`) inaccessible si `ui_bind` est une IP LAN** : `probe_hosts` teste désormais `127.0.0.1` et `::1` avant l'IP LAN. | `src/lib.rs` |
| **MC-C03** | Desktop Update | **P0 · S** | ✅ | **Course critique d'auto-mise à jour NSIS sous Windows** : `shutdown_engine()` s'exécute avant le spawn de `setup.exe /S` (juste avant `exit(0)`) + `NSIS_HOOK_PREINSTALL` silencieux. | `src/manage/update.rs`, `desktop/nsis/installer-hooks.nsh` |
| **MC-C04** | Audit & Disque | **P0 · S** | ✅ | **Verrouillage définitif de `auth-audit.log` sur `ENOSPC` pendant `Journal::rotate`** : staging `.rotate.tmp` + ACL + `sync_all` avant toute rotation, avec rollback si l'activation échoue. | `src/audit.rs` |

---

## 1. MCP Control — Moteur, Supervision, Probe, SSH, Desktop & CI/CD

### 1.1 Corrections (`MC-C*`)

- [x] **[MC-C01] [P0 · S] Régénérer un jeton `#t=` frais à la réouverture Desktop (tray / `single_instance` / reprise)**
  - **Constat** : `desktop/src/main.rs:57-91, 175-206` ouvre la WebView une seule fois avec `#t=<token>`, aussitôt effacé côté JS (`src/ui/app.js:252`). Après `IDLE_TTL` (12 h, `src/auth/session.rs:68`), `ABSOLUTE_TTL` (24 h, `line 75`), `F5` ou redémarrage moteur, `sessionLost = true` fige l'UI sur `401 — session expirée` sans moyen de récupération depuis une fenêtre Tauri sans barre d'URL.
  - **Correction** : conserver `Arc<SessionStore>` (ou un canal IPC / helper `mint_bootstrap_url`) dans Tauri ; lors d'un clic tray, `single_instance`, ou événement de session perdue, émettre un nouveau jeton via `SessionStore::issue_token()` (ou `request_session_token` en mode `reused`) et naviguer si la session courante n'est plus valide.

- [x] **[MC-C02] [P0 · S] Prioriser `127.0.0.1` et `::1` dans `probe_hosts` lors de la détection d'une instance existante**
  - **Constat** : `src/probe.rs:6263-6307` place `bind_host` (ex. `192.168.1.50`) *avant* `127.0.0.1`, ce qui fait retourner `https://192.168.1.50:5090` par `existing_ui_base_url` (`src/lib.rs:1123-1154`) alors que `local_aux_listener` écoute bien en HTTP sur `127.0.0.1:5090`. Conséquence : WebView2 bloque le certificat auto-signé, `/api/session` renvoie `403` (pair non-loopback) et `desktop/src/main.rs:78-118` coupe les capacités Tauri (`decorations: false` → fenêtre impossible à déplacer/fermer).
  - **Correction** : placer systématiquement `127.0.0.1` et `::1` en tête de `probe_hosts` pour que les clients locaux passent toujours par le listener auxiliaire loopback HTTP.

- [x] **[MC-C03] [P0 · S] Ordonner proprement l'arrêt moteur avant le spawn de l'installeur NSIS**
  - **Constat** : `src/manage/update.rs:695-740` exécute `Command::new(&staged).arg("/S").spawn()` puis attend `1500 ms` avant `shutdown_engine()` et `process::exit(0)`. En mode silencieux `/S`, NSIS tente immédiatement d'écraser le binaire en cours d'exécution.
  - **Correction** : envoyer la réponse HTTP `{ok:true}` (flush), exécuter `shutdown_engine()`, lancer un script/processus détaché ou spawner l'installeur juste avant `process::exit(0)`, et ajouter une boucle d'attente de libération du binaire dans `desktop/nsis/installer-hooks.nsh:11-18`.

- [x] **[MC-C04] [P0 · S] Rendre `Journal::rotate` atomique face à `ENOSPC` et poser les ACL avant écriture**
  - **Constat** : `src/audit.rs:307-353` renomme `auth-audit.log` en `.1` *avant* de créer le nouveau fichier (et `fs::write` ligne 347 écrit avant `restrict_permissions`). Si `fs::write` échoue (disque plein), `auth-audit.log` disparaît alors que `.1` existe, ce qui bloque définitivement tout redémarrage futur via la garde anti-suppression `[H-05]` (`lines 278-295`).
  - **Correction** : créer `auth-audit.log.new` avec `restrict_permissions` dès l'ouverture, écrire l'entrée de chaînage + `sync_all()`, *puis* décaler `.4 -> .5` .. `auth-audit.log -> .1` et renommer atomiquement `auth-audit.log.new -> auth-audit.log`.

- [x] **[MC-C05] [P1 · S] Activer `local_aux_listener` (`127.0.0.1`) quand `ui_bind == "::"` sous Windows**
  - **Constat** : `src/lib.rs:1259-1274` exclut `ui_bind != "::"` en supposant un socket dual-stack, alors que Windows active `IPV6_V6ONLY = 1` par défaut. Résultat : `127.0.0.1:<port>` n'est pas écouté alors même que `lib.rs:1289` annonce `http://127.0.0.1:<port>` dans les logs et que `auth::cli` cible `127.0.0.1`.
  - **Correction** : retirer `&& ui_bind != "::"` à la ligne 1260 (avec gestion gracieuse de `AddrInUse` sous Linux si `net.ipv6.bindv6only = 0`).

- [ ] **[MC-C06] [P1 · S] Ne jamais persister le port de repli temporaire (`+1..+9`) dans `config.json`**
  - **Constat** : `src/lib.rs:1185, 1203-1205` écrase `state.settings.ui_port` en mémoire lors d'un fallback temporaire (`5090` occupé → `5091`). Au premier `save_config` (`src/manage/config.rs:817`), `5091` est écrit définitivement dans `config.json`, cassant tous les clients `bridge` configurés sur `5090`.
  - **Correction** : stocker `bound_ui_port: AtomicU16` sur `AppState` pour l'exécution courante sans muter `settings.ui_port`.

- [ ] **[MC-C07] [P1 · M] Corriger les 3 défauts de `probe_stdio` (faux `UP`, fuite de zombie, Job Object tardif)**
  - **Constat** : dans `src/probe.rs` :
    1. Lignes `5388-5448` : si le processus enfant crash ou expire pendant `tools/list` après avoir répondu à `initialize` (`got_init = true`), `probe_stdio` retourne quand même `mcp_up: true, tools: []`.
    2. Lignes `5337-5340` : en cas d'erreur `writeln!` sur `notifications/initialized`, `kill_tree(pid)` est appelé sans `child.wait()`, laissant un processus zombie sous Unix.
    3. Lignes `334` et `5258` : sous Windows, `assign_child_to_job(pid)` n'est appelé qu'après le handshake au lieu d'être exécuté immédiatement après `cmd.spawn()`.
  - **Correction** : assigner au Job Object dès `cmd.spawn()` ; toujours appeler `let _ = child.wait();` dans les chemins d'erreur ; marquer `mcp_up: false` (ou relancer) si `child.try_wait()` indique que le processus est mort pendant `tools/list`.

- [x] **[MC-C08] [P1 · S] Remplacer le matching de port par sous-chaîne dans `parse_remote_port_listeners` et `pids_listening_on_ss`**
  - **Constat** : `src/probe.rs:3908-3941` fait `line.contains(":314")` (qui matche `:3141` ou un timestamp `12:31:40`) et `src/probe.rs:1164-1168` fait `ends_with(".80")` sur toute la ligne `netstat` (qui matche l'IP distante `192.168.1.80`). Un `kill` local ou SSH peut tuer le mauvais processus (recoupe aussi **R1**).
  - **Correction** : extraire exclusivement la colonne d'adresse locale (`ss`/`netstat`), découper sur le dernier `:` (ou `.` sur macOS `netstat`) et comparer le `u16` exact ; lors d'un `kill` serveur (`src/manage/actions.rs:678, 809`), vérifier aussi `s.pid` / l'identité du process quand connu.

- [ ] **[MC-C09] [P1 · M] Supporter Windows OpenSSH dans les sondes SSH et éliminer `/tmp/.mcc_call_$$` prévisible**
  - **Constat** : `src/probe.rs:3112-3176, 3196-3231, 3750-3808` envoie inconditionnellement de la syntaxe POSIX `sh` (`export PATH=...`, `2>/dev/null`, `/tmp/.mcc_call_$$`) même quand `peer.os == "windows"` (cmd/PowerShell par défaut sous Windows OpenSSH). De plus, `/tmp/.mcc_call_$$` dans `ssh_call_tool` (`line 3199`) est prévisible dans `/tmp` partagé (CWE-377).
  - **Correction** : utiliser `curl -s -i` sur stdout (zéro fichier temporaire) sur POSIX, et fournir la variante PowerShell (`Invoke-WebRequest` / `Get-NetTCPConnection`) quand la cible SSH est Windows.

- [x] **[MC-C10] [P1 · S] Corriger le double crochetage IPv6 `Host: [[::1]]:port` et valider le port `> 0` dans `notify::split_url`**
  - **Constat** : `src/notify.rs:370-383` conserve les crochets `[::1]` dans `host` et accepte le port `0`, tandis que `http_host_header` (`src/probe.rs:2315-2321`) rajoute `[...]` dès que `host.contains(':')`, émettant `Host: [[::1]]:8080`.
  - **Correction** : normaliser `host` sans crochets dans `split_url`, rejeter `port == 0`, et rendre `http_host_header` idempotent (`trim_start_matches('[').trim_end_matches(']')`).

- [ ] **[MC-C11] [P1 · M] Implémenter un vrai décodeur HTTP `Transfer-Encoding: chunked` et rafraîchir le timeout de lecture**
  - **Constat** : `src/probe.rs:4250-4274, 4506-4519` détecte la fin d'un corps `chunked` par simple recherche de sous-séquence `\r\n0\r\n` dans `buf` : un payload JSON/SSE contenant littéralement `\r\n0\r\n` au milieu d'un chunk est tronqué, et un corps vide `0\r\n\r\n` bloque jusqu'au timeout. De plus, `http_request_trusted` (`src/auth/net.rs:585`) ne met jamais à jour `set_read_timeout(remain)` dans sa boucle `read`.
  - **Correction** : parser les tailles hexadécimales de chunks selon RFC 9112 et rafraîchir `remain = deadline - Instant::now()` à chaque tour de boucle dans `http_request_trusted`.

- [x] **[MC-C12] [P1 · S] Bloquer les chemins UNC à slashs mixtes (`\/server\share`, `\\?\UNC\`) dans `ssh_path_safe` et `open_path`**
  - **Constat** : `src/probe.rs:2776-2788` et `src/manage/actions.rs:1257-1261` ne testent que `starts_with("\\\\")` et `starts_with("//")`. Sous Windows, l'API Win32 normalise `/` en `\` : `\/attacker\share` ou `\??\UNC\...` déclenche une authentification SMB/NTLM sortante (fuite de hash NetNTLMv2).
  - **Correction** : normaliser tous les `/` en `\` avant vérification, rejeter tout préfixe de ≥ 2 séparateurs ainsi que `\\?\` et `\\.\`.

- [ ] **[MC-C13] [P1 · S] Rendre les webhooks d'incidents asynchrones, indexer le cooldown par serveur et alléger `test_notify`**
  - **Constat** : `notify_incident` (`src/manage/events.rs:16-60`) exécute l'appel HTTP webhook de façon synchrone dans les workers `poll_one` (jusqu'à 5 s × 12 workers bloqués en cas de panne réseau), indexe l'anti-rebond dans `src/notify.rs:392-403` par `label` (deux serveurs partageant le même nom s'étouffent mutuellement), et `test_notify` (`src/manage/actions.rs:1504`) bloque la requête HTTP UI pendant 9 s à cause du `Start-Sleep -Seconds 9` du script PowerShell de toast Windows.
  - **Correction** : pousser les notifications dans une file bornée consommée par un thread dédié, indexer le cooldown par `(state_ptr, server.id)`, et détacher l'attente PowerShell du thread HTTP lors de `test_notify`.

- [ ] **[MC-C14] [P2 · M] Hygiène des processus enfants, resets, imports et identifiants horodatés**
  - **Constat** :
    1. `src/manage/scans.rs:649-660` : quand `pipe_dead` est vrai, `StdioHandle` est retiré de `state.children` et droppé sans `kill_tree(pid)`.
    2. `src/manage/actions.rs:1738-1745` : `reset_state("detection")` vide `state.servers` sans tuer ni purger `children`, `spawning` et `runtime` des serveurs supprimés.
    3. `src/manage/import.rs:234-263` : l'import `claude mcp add` ignore les flags `-e KEY=VAL` avant `--` et découpe les chemins quotés avec espaces sur `split_whitespace`.
    4. `src/manage/json.rs:5-10` : `now_compact()` est précis à la milliseconde sans compteur atomique (collision d'`id` en import rapide).
    5. `src/manage/actions.rs:1368-1377` : `set_settings` réécrit un `ui_bind` personnalisé (`::1` ou IP LAN) vers `127.0.0.1` dès qu'on sauvegarde un autre réglage.
    6. `src/manage/scans.rs:168, 564` : remplacer les deux `.lock().unwrap()` par `lockv(...)`.
  - **Correction** : appeler `kill_tree` + `wait` sur `pipe_dead` et `reset_state` ; parser `-e`/`--env` et les guillemets dans `import.rs` ; ajouter un `AtomicU32` monotone à `now_compact()` ; préserver un `ui_bind` valide existant dans `set_settings`.

- [ ] **[MC-C15] [P2 · M] Parité multi-OS (macOS/Linux) et conformité CI**
  - **Constat** :
    1. `src/probe.rs:667, 2247` : `build_port_owner_map` retourne une map vide sur macOS (pas de parseur `lsof -iTCP -sTCP:LISTEN -n -P`) et `process_creation` retourne toujours `None` sur Linux/macOS (`/proc/<pid>/stat` / `ps -o lstart=`).
    2. `src/manage/scans.rs:1971-2016` : `local_ips()` ignore les interfaces IPv6.
    3. `src/manage/autostart.rs:185, 329-348` : ignore `XDG_CONFIG_HOME` sous Linux et ne déséchappe pas les entités XML `.plist` (`&amp;`) ni les guillemets `.desktop`.
    4. `src/probe.rs:5184-5205` : `expand_env_vars` fait un `replace` multi-passes (si `FOO="${BAR}"`, la valeur de `FOO` est ré-interpolée).
    5. `.github/workflows/ci.yml:43, 86` : `continue-on-error: true` sur macOS masque les régressions, et le crate `desktop` n'est compilé en CI que sous Linux et Windows.
  - **Correction** : implémenter `lsof` (macOS) et `starttime` Unix, supporter `XDG_CONFIG_HOME`, interpoler `${VAR}` en une seule passe regex/curseur, et rendre le job macOS bloquant en CI (incluant `mcp-control-desktop`).

### 1.2 Optimisations (`MC-O*`)

- [ ] **[MC-O01] [P1 · M] Rendre `GET /api/state` et `state_etag` « zéro I/O disque / zéro Keyring »**
  - **Constat** : à chaque poll UI (toutes les 5–15 s), `state_json` (`src/manage/json.rs:63, 300, 338, 467, 614`) :
    1. scanne `%TEMP%\litemcp-registry` et vérifie les ACL Win32 *deux fois* sur un `200` (dans `state_etag` puis `state_json`) ;
    2. lit et parse chaque fichier `auth/clients/*.pin.json` jusqu'à `2 × N` fois (`client_keyid_for_server` + `paired_servers_json`) ;
    3. parcourt `%PATH%` (`fs::metadata`) dans `current_bridge_command()` ;
    4. et chaque sonde HTTP sortante interroge le Keyring OS (`keyring::Entry::get_password`, `src/auth/net.rs:968`).
  - **Optimisation** : mettre en cache mémoire dans `AppState` les annonces LiteMCP (rafraîchies dans `poll_all`), la table des `.pin.json` + clés clientes (invalidée sur `pair`/`unpair`), le chemin résolu du binaire `mcp-control` (`OnceLock`), et calculer `state_etag` via un compteur de révision atomique `state_rev: AtomicU64` incrémenté à chaque mutation.

- [x] **[MC-O02] [P1 · S] Activer le fast-path `ProbeHint` pour les serveurs `UP` à 0 outil et factoriser `probe_http_with` / `probe_http_host_with`**
  - **Constat** : `src/manage/scans.rs:109-113` exige `&& has_tools` pour accorder un `ProbeHint`, pénalisant les serveurs MCP valides exposant uniquement des `resources` ou `prompts` (5 connexions TCP par cycle au lieu de 2). De plus, `probe_http_with` (`src/probe.rs:4766-5091`) et `probe_http_host_with` (`5811-6078`) dupliquent ~260 lignes quasi identiques.
  - **Optimisation** : retirer `&& has_tools` dès que `s.state == ServerState::Up && s.protocol_version.is_some()`, mémoriser la famille loopback gagnante (`127.0.0.1` vs `::1`) dans `ProbeHint`, et unifier les deux fonctions HTTP.

- [ ] **[MC-O03] [P1 · S] Pré-construire et servir les assets statiques (`GET /`, `/api/store`, `/api/store-logos`, `/api/frameworks`) avec `OnceLock` + `ETag` / `304`**
  - **Constat** : `src/lib.rs:2331-2338, 2411-2429` reconstruit `ui::index_html()` (~84 Ko `String`) et re-sérialise les catalogues JSON immuables à chaque requête sans `ETag`.
  - **Optimisation** : mettre en cache les octets (et leur variante précompressée `gzip` si `Accept-Encoding: gzip`) + `ETag` basé sur `PKG_VERSION` dans un `OnceLock`.

- [ ] **[MC-O04] [P2 · S] Borner le fan-out de threads de `discover_peers`, alléger `save_cache` / `audit::recent` et activer ThinLTO**
  - **Constat** :
    1. `src/manage/peers.rs:97` spawne 1 thread OS par entrée ARP simultanément (centaines de threads sur un `/16`).
    2. `src/probe.rs:5268-5289` garde un thread lecteur `stderr` prenant un `Mutex` à chaque ligne pendant toute la vie d'un serveur `stdio`, alors que le buffer est gelé à 4 Ko après le handshake.
    3. `src/audit.rs:241-267` lit jusqu'à 8 Mo en `String` (`fs::read_to_string`) juste pour extraire les `limit` dernières lignes.
    4. `src/manage/cache.rs:25-88` clone et sérialise tout le JSON pour comparer à `last_cache_json` sous mutex, et `src/types.rs:293` conserve un champ mort `spawn_fails`.
    5. `Cargo.toml:48-49` n'active ni `lto = "thin"` ni `strip = "symbols"` en profil `release`.
  - **Optimisation** : pool borné à 16 workers pour `discover_peers` ; basculer le lecteur `stderr` vers `io::sink()` une fois le cap de 4 Ko atteint ; lire la fin de `auth-audit.log` par `SeekFrom::End` ; hasher l'état directement sans allocation intermédiaire ; activer `lto = "thin"` et `strip = "symbols"`.

---

## 2. Interface UI (`src/ui.rs`, `src/ui/app.js`, `src/ui/app.css`, `ui-dist/`)

### 2.1 Corrections (`UI-C*`)

- [x] **[UI-C01] [P1 · S] Réparer `hasAuthEnv(s)` (`Array` vs `Object`) — le badge `AUTH` et « Retirer l'env d'auth » ne s'affichent jamais**
  - **Constat** : `src/manage/json.rs:58, 69` sérialise `s.env` sous forme de tableau des noms de variables (`["LITEMCP_TRUSTED_KEYS", ...]`), mais `src/ui/app.js:1057-1063` teste `'LITEMCP_TRUSTED_KEYS' in e || 'MCC_AUTH_PUBKEY' in e`. En JavaScript, l'opérateur `in` sur un `Array` teste les index numériques (`0, 1, ...`), retournant **toujours `false`** ! Le badge `AUTH` (`line 1072`) et le bouton « Retirer l'env d'auth » (`line 2503`) sont donc morts.
  - **Correction** : écrire `Array.isArray(e) ? (e.includes('LITEMCP_TRUSTED_KEYS') || e.includes('MCC_AUTH_PUBKEY')) : ('LITEMCP_TRUSTED_KEYS' in e || 'MCC_AUTH_PUBKEY' in e)`.

- [x] **[UI-C02] [P1 · S] Remplacer les rafales N+1 `Promise.all` de `setAllTools()` et `refreshAll()` par des actions batch dédiées**
  - **Constat** :
    1. `setAllTools` (`src/ui/app.js:2034`) lance `Promise.all(names.map(... => api('toggle_tool', ...)))`. Sur un serveur ayant 50 à 229 outils (ex. `gh-rea`), le navigateur ouvre des dizaines de requêtes simultanées, dépasse `MAX_CONNS_PER_IP = 16` (`src/lib.rs:29` → erreurs `503`), et déclenche jusqu'à 229 réécritures séquentielles de `config.json` + spawns `icacls` !
    2. `refreshAll` (`src/ui/app.js:3055-3058`) fait de même avec `state.servers.map(s => api('refresh', {id: s.id}))` dès qu'il y a > 15 serveurs actifs → `503` + toast « au moins un appel a échoué ».
  - **Correction** : ajouter deux actions atomiques côté Rust (`set_all_tools` acceptant `{id, disabled: bool, tools?: [...]}` avec une seule sauvegarde `config.json`, et `refresh_all` lançant `poll_all` côté serveur), appelées en 1 requête HTTP par l'UI.

- [x] **[UI-C03] [P1 · S] Corriger le faux message « CLI absent » dans `sshBootstrap()` sur réponse asynchrone**
  - **Constat** : `ssh_bootstrap` (`src/manage/actions.rs:1964-1968`) tourne en tâche de fond et répond immédiatement `{ok: true, started: true}`. Or `src/ui/app.js:3897-3902` lit `j.cli_running` / `j.cli_installed` sur cette réponse immédiate et affiche à tort `✓ <ip> — CLI absent` sans lancer de polling comme le fait `sshDiscover` (`lines 3842-3858`).
  - **Correction** : gérer `if (j.started)` dans `sshBootstrap` avec un message « Déploiement SSH lancé en arrière-plan — suivi dans le journal… » et un rafraîchissement périodique de l'état des pairs.

- [x] **[UI-C04] [P1 · S] Corriger le toast prématuré « Scan réseau terminé » dans `doScanNet()` et harmoniser les retours d'erreur d'actions**
  - **Constat** : `scan_network` (`src/manage/actions.rs:953-954`) démarre un thread et rend `{ok: true, started: true}` en 1 ms, mais `src/ui/app.js:3039-3040` affiche immédiatement « Scan réseau terminé ». De plus, plusieurs actions (`ssh_forget` sur pair inconnu, `discover`, `reorder` — cf. **R4/R17**) renvoient `{ok: false}` sans champ `error`, affichant un toast générique « échec ».
  - **Correction** : afficher « Scan réseau démarré en arrière-plan » quand `j.started === true` (avec indicateur de scan en cours dans `/api/state`), et renseigner un champ `error` explicite sur tous les refus d'actions dans `actions.rs`.

- [ ] **[UI-C05] [P1 · S] Gérer la reconnexion post-mise à jour sans tomber sur `401` (`location.reload()`)**
  - **Constat** : après `install_update`, `src/ui/app.js:463, 483` appelle `location.reload()` dès que `/api/hello` répond à nouveau. Mais le nouveau moteur repart avec une table de sessions en RAM vide et l'URL du navigateur ne contient plus `#t=` → l'utilisateur atterrit systématiquement sur l'écran d'erreur `401 — session expirée`.
  - **Correction** : côté navigateur local, afficher un message clair invitant à relancer `mcp-control ui` (ou conserver les sessions actives chiffrées en mémoire temporaire de migration / régénérer le jeton automatiquement dans le wrapper Tauri Desktop cf. **MC-C01**).

- [ ] **[UI-C06] [P2 · S] Supporter les guillemets dans la saisie de commandes (`setStartCmd` & `addServer`) et permettre d'éditer un `start_cmd` existant**
  - **Constat** :
    1. `src/ui/app.js:2285, 4033, 4044` découpe les arguments avec `.split(/\s+/)`, ce qui casse tout chemin contenant un espace (`"C:\Program Files\nodejs\node.exe"` ou `--db "C:\Mes Docs\base.db"`).
    2. `startCmdForm(s)` (`src/ui/app.js:2266`) retourne `''` dès que `s.can_start` est vrai : une fois une commande de démarrage enregistrée, il est impossible de la modifier ou de la corriger depuis l'UI sans supprimer le serveur.
  - **Correction** : implémenter un petit tokenizer respectant les guillemets simples/doubles en JS (miroir de `split_shell_words_simple`), et ajouter un bouton « Modifier la commande » dans la fiche Détail même quand `s.can_start` est vrai.

- [ ] **[UI-C07] [P2 · S] Corriger ou retirer la page de secours morte `ui-dist/boot.js` (bloquée par la CSP Tauri)**
  - **Constat** : `ui-dist/boot.js:4-8` fait `fetch('http://127.0.0.1:' + p + '/api/hello')` puis `location.replace(...)` sans `#t=`, alors que `desktop/tauri.conf.json:12` impose `connect-src 'self' ipc: http://ipc.localhost` (bloquant tous ces `fetch` cross-origin).
  - **Correction** : remplacer `ui-dist/` par un écran statique minimaliste d'attente/erreur piloté uniquement par le process Rust Tauri (`desktop/src/main.rs`).

### 2.2 Optimisations (`UI-O*`)

- [ ] **[UI-O01] [P2 · M] Éliminer les `JSON.stringify` complets à chaque tick de polling et fermer `unsafe-inline` dans la CSP**
  - **Constat** : `src/ui/app.js:501, 859, 1335, 1442, 2049, 4743` recalcule plusieurs signatures par `JSON.stringify` sur `state.servers` complet (plusieurs centaines de Ko avec les schémas d'outils) à chaque cycle, alors que `_stateEtag` identifie déjà tout changement d'état. Par ailleurs, migrer progressivement les handlers `onclick="..."` vers une délégation d'événements `data-act` permettra de passer la CSP de `script-src 'self' 'unsafe-inline'` à `script-src 'self'`.

- [ ] **[UI-O02] [P2 · M] Unifier le schéma de validation des Réglages (`SETTINGS_SCHEMA` partagé Rust ↔ JS) et nettoyer le CSS/UI legacy**
  - **Constat** : la validation des plages de ports, `refresh_secs` et `ui_bind` est dupliquée entre `collectSettingsForm` (`src/ui/app.js:3626-3708`) et `set_settings` (`src/manage/actions.rs:1330-1430`) ; `src/ui/app.css:447-458, 529-575` embarque ~100 règles quantifiées `.ctxmenu[data-x=...]` / `#explorer[data-w=...]` remplaçables par une mise à jour de style JS ou `CSSStyleSheet` ; nettoyer aussi l'ancien flux `accessCode`/`loginDlg` prévu pour retrait en `0.9.0`.

---

## 3. Gateway & Bridge (`src/gateway.rs`, `src/bridge.rs`, `src/manage/litemcp.rs`, `docs/gateway.md`)

### 3.1 Corrections (`GW-C*`)

- [x] **[GW-C01] [P0 · S] Imposer la politique de confirmation humaine (`auth::confirm::decide`) sur l'endpoint hybride `POST /mcp/s/<id>`**
  - **Constat** : sur `POST /mcp`, l'outil `call_server_tool` vérifie le droit `execution` **puis** soumet l'appel à `confirm::decide` (`src/gateway.rs:392-400`). En revanche, sur `POST /mcp/s/<id>` (`serve_hybrid_stdio`, `src/gateway.rs:2062-2427`), la ligne `2074` vérifie uniquement `auth_ctx.has_right("execution")` et exécute **immédiatement** `call_stdio_tool` (`line 2322`), `ssh_call_tool` (`line 2349`) ou `call_stdio_rpc` (`line 2405`) sans jamais consulter `state.approval_policy` ni créer de `PendingRequest` ! Un agent IA victime de prompt injection contourne ainsi à 100 % la confirmation humaine locale en appelant `/mcp/s/<id>`.
  - **Correction** : appeler `crate::auth::confirm::decide(state, auth_ctx.client(), "hybrid_call", &summary)` avant toute exécution `is_exec` (`tools/call`, `resources/read`, `prompts/get`) dans `serve_hybrid_stdio`, journaliser dans `auth-audit.log` et renvoyer `403` JSON-RPC `-32023` (`approval_pending` / `approval_denied`) tant que l'utilisateur n'a pas approuvé localement.

- [x] **[GW-C02] [P1 · S] Rendre les en-têtes signés `Mcp-Method` et `Mcp-Name` (SEP-2243) obligatoires sur `POST /mcp/s/<id>`**
  - **Constat** : dans `src/gateway.rs:1979-2023`, `if let Some(ref hm) = mcp_method_hdr` et `if let Some(ref hn) = mcp_name_hdr` ne vérifient la cohérence en-tête ↔ corps JSON-RPC **que si le client a volontairement inclus ces en-têtes** ! Comme `SignatureVerifier` accepte le profil à 6 composants (sans `mcp-name`), un client omettant `Mcp-Method` ou `Mcp-Name` sur `/mcp/s/<id>` contourne la liaison signée SEP-2243 tout en passant la vérification de signature.
  - **Correction** : exiger `mcp_method_hdr.is_some()` sur toute requête `/mcp` et `/mcp/s/<id>` (hors notifications sans méthode si applicable, sinon toujours), et exiger `mcp_name_hdr.is_some()` dès que `matches!(rpc_method, "tools/call" | "prompts/get" | "resources/read")`.

- [ ] **[GW-C03] [P1 · S] Aligner les capacités annoncées par `initialize` sur `/mcp/s/<id>` et encadrer la branche `other => call_stdio_rpc`**
  - **Constat** : `serve_hybrid_stdio` (`src/gateway.rs:2209`) répond à `initialize` avec `"capabilities": { "tools": { "listChanged": false } }` uniquement (masquant `resources` et `prompts` aux clients MCP conformes), tandis que la branche finale `other =>` (`lines 2383-2418`) relaie aveuglément `resources/list`, `resources/read`, `prompts/list`, `prompts/get` et `completion/complete` vers `call_stdio_rpc(..., 60_000)` uniquement pour `stdio` (renvoyant `-32601` pour HTTP/SSH).
  - **Correction** : annoncer explicitement les capacités réellement supportées selon le transport ou restreindre la liste blanche des méthodes autorisées, et harmoniser le support `resources/*` / `prompts/*`.

- [x] **[GW-C04] [P1 · S] Borner la lecture `stdin` dans `mcp-control bridge` et recharger la clé locale sur `401`**
  - **Constat** :
    1. `src/bridge.rs:185` utilise `stdin.lock().lines()`, qui alloue sans limite (`String` infinie) si un client MCP envoie des Mo/Go sans `\n` → crash OOM.
    2. `src/bridge.rs:473-482` garde la même `SigningKey` en mémoire pendant toute la vie du bridge : après une rotation de clé (`client_rotate` ou re-pair), le bridge renvoie `401` en boucle jusqu'au redémarrage complet de Claude Desktop / Cursor.
  - **Correction** : lire `stdin` avec un lecteur borné à `32 Mio` par ligne (comme `MAX_STDIO_LINE` dans `probe.rs`), et en cas de réponse `401`, tenter une unique ré-ouverture de `identite_locale(&cfg)` avant de remonter l'erreur.

- [x] **[GW-C05] [P2 · S] Compléter `gateway::fingerprint` pour détecter les renommages, modifications de schémas et bascules d'outils à taille constante**
  - **Constat** : `src/gateway.rs:249-264` ne hache que `s.tools.len()` et `s.tools_disabled.len()` (ainsi que `s.id` et `s.state`). Désactiver l'outil A et activer l'outil B en même temps laisse `tools_disabled.len()` identique, renommer `s.label` ne met pas à jour `dynamic_resources`, et un changement de schéma d'outil au hot-reload d'un serveur MCP est ignoré par le cache `GatewayIndex`.
  - **Correction** : inclure dans le hash `s.label`, les noms + descriptions/tailles de schémas de `s.tools`, et les éléments triés de `s.tools_disabled`.

- [ ] **[GW-C06] [P2 · S] Mettre à jour `docs/gateway.md` et `README.md` (dérive documentaire majeure `v0.7` → `v0.8.9`)**
  - **Constat** : `docs/gateway.md:5-6, 52-57` documente encore l'ancien modèle v0.7 (`/mcp` sans auth en loopback, `?access=`, `X-MCC-Access` donnant tous les outils RCE, refus de `stdio`/SSH dans `call_server_tool`), et `README.md:119, 180, 245` cite encore le serveur retiré `Filesystem` ainsi que la limitation HTTP-only de `call_server_tool` et du testeur live.
  - **Correction** : réécrire `docs/gateway.md` pour refléter RFC 9421 Ed25519 + SEP-2243 + `POST /mcp/s/<id>` + `mcp-control bridge` + `ApprovalPolicy`, et aligner `README.md`.

### 3.2 Optimisations (`GW-O*`)

- [ ] **[GW-O01] [P1 · M] Réutiliser une connexion HTTP/TLS keep-alive et `Arc<rustls::ClientConfig>` dans `mcp-control bridge`**
  - **Constat** : `src/bridge.rs:526-570` ouvre une nouvelle socket TCP et reconstruit un `rustls::ClientConfig` complet + handshake TLS 1.3 (`src/auth/net.rs:460-472`) **à chaque message JSON-RPC** (`initialize`, `notifications/initialized`, `tools/list`, chaque `tools/call`). Sur un pair distant/WAN, cela triple la latence de chaque appel d'outil.
  - **Optimisation** : construire `Arc<rustls::ClientConfig>` une seule fois au démarrage du `bridge` et maintenir un flux keep-alive (`Connection: keep-alive`) avec reconnexion transparente sur coupure socket.

- [ ] **[GW-O02] [P2 · S] Éviter l'appel complet à `manage::state_json` dans `list_peers` et `mcp-control://servers`**
  - **Constat** : `src/gateway.rs:572` appelle `manage::state_json(&st)` (sérialisation de tous les serveurs, journaux, réglages, annonces LiteMCP et lectures disque `.pin.json`) uniquement pour lire `full["peers"]` et `full["lan_ok"]`.
  - **Optimisation** : sérialiser directement `state.peers` et `state.lan_ok` via un helper léger dédié.

---

## 4. Authentification & Sécurité (`src/auth/**`, `src/crypto.rs`, `docs/auth.md`)

### 4.1 Corrections (`AU-C*`)

- [x] **[AU-C01] [P0 · S] Éliminer l'usurpation d'aperçu de confirmation humaine (`Confirmation Spoofing` / CWE-451) dans `add_server` et `import_config`**
  - **Constat** :
    1. `src/auth/confirm.rs:414-427` n'affiche le programme à lancer que si `transport == "stdio"`. Or `gateway.rs:777-817` et `actions.rs:725-803` autorisent `"transport": "http"` avec `program`/`args`/`workdir` : la modale d'approbation affiche un simple ajout passif `HTTP 127.0.0.1:<port>/mcp`, puis le poller (`scans.rs:610`) exécute silencieusement le binaire arbitraire dès que le port est fermé !
    2. Différentiel de parseur sur `import_config` : `confirm::programmes_importes` (`confirm.rs:190-195`) inspecte `"mcpServers"`, `"mcp_servers"`, **`"servers"` et `"mcp"`**, alors que `import::parse_servers_doc` (`src/manage/import.rs:8-12`) n'inspecte que `"mcpServers"` et `"mcp_servers"` avant de retomber sur la racine JSON. Un JSON `{"servers": {"leurre": {"url": "..."}}, "reel": {"command": "calc.exe"}}` n'affiche que `leurre` à l'écran mais exécute `reel` ! De plus, `confirm.rs:207-236` omet `"cwd"`, tronque `args` à 8 éléments sans indicateur et masque au-delà de 10 serveurs.
  - **Correction** : afficher `program`/`args`/`workdir`/`env` dans `confirm.rs` dès que `!program.is_empty()` quel que soit `transport` ; partager exactement `import::parse_servers_doc` avec `confirm::programmes_importes` (incluant `cwd` et un marqueur explicite `(+N args)` / total complet des programmes).

- [x] **[AU-C02] [P0 · S] Sceller le port réel (`bound_ui_port`) dans `.grant` pour empêcher le vol du jeton `admin` sur repli de port (`+1..+9`) ou squat de `ui_port`**
  - **Constat** : sur une machine multi-utilisateurs (ou port `5090` occupé par un processus tiers), le moteur bascule sur `5091` (`src/lib.rs:1185`), mais `mcc-session-<id>.grant` (`src/auth/session.rs:586`) n'écrit que `"{nonce}\n{token}\n"` sans le port `5091`. Quand l'utilisateur lance ensuite `mcp-control auth list` (`src/auth/cli.rs:273-340`), la CLI lit `5090` dans `config.json` et envoie `POST http://127.0.0.1:5090/api/session` contenant le **vrai jeton `admin`** directement au socket du processus tiers (qui peut le rejouer sur `5091` pour obtenir une session `admin` complète). De même, `desktop/src/main.rs:78-91` accepte `reused = true` sur un simple `/api/hello` non authentifié.
  - **Correction** : écrire `"{nonce}\n{bound_port}\n{token}\n"` dans `.grant` ; faire utiliser exclusivement ce `bound_port` authentifié par `auth::cli` et `desktop/src/main.rs` ; refuser `reused = true` si `request_bootstrap` échoue.

- [x] **[AU-C03] [P1 · S] Réparer la mort définitive de la table `throttle` après 1024 blocages, le reset croisé du compteur IP et le contournement IPv6 `/64`**
  - **Constat** :
    1. `src/auth/throttle.rs:201` évince les entrées via `.min_by_key(|(_, c)| (c.bloque_jusqua > 0, c.dernier))` et ne remet jamais `bloque_jusqua` à `0` après expiration (`line 85`). Dès que `MAX_ENTREES` (1024) fausses clés ont été bloquées une fois dans la vie du moteur, toute 1025ᵉ clé attaquée (`bloque_jusqua == 0`) est **immédiatement évincée au 1er échec** : l'anti-force brute est définitivement neutralisé !
    2. Dans `src/lib.rs:2046` et `throttle.rs:165-171`, toute requête signée valide (`200`/`202`/`403`, même avec une clé `rights: []`) appelle `c.succes()` sur `ips[peer]`, remettant à zéro les 39 échecs précédents de l'IP.
    3. Les limiteurs (`throttle.rs:126`, `pair.rs:100`, `lib.rs:69`) indexent l'IPv6 sur 128 bits au lieu du préfixe `/64` SLAAC hors loopback.
  - **Correction** : comparer `c.bloque_jusqua > now` (et remettre `bloque_jusqua = 0` à l'expiration dans `elaguer(now)`) ; ne jamais réinitialiser `ips[peer]` sur succès d'une clé ; normaliser les IPv6 non-loopback par préfixe `/64`.

- [x] **[AU-C04] [P1 · S] Éliminer le TOCTOU sur les permis de confirmation « 10 minutes » (`window`) via `set_host` ou modification de commande**
  - **Constat** : `decide()` (`src/auth/confirm.rs:494-502, 922-936`) hache uniquement le JSON brut `args` (`{"server": "srv-1", ...}`) sans inclure l'hôte résolu ni `start_cmd`. Un client disposant de `pilotage` + `execution` peut obtenir un permis `window` (10 min) sur `srv-1` (`127.0.0.1`), appeler `set_host` (qui ne requiert aucune confirmation) pour rediriger `srv-1` vers une IP distante, puis réutiliser son permis 10 min vers la nouvelle cible sans approbation.
  - **Correction** : inclure l'endpoint résolu (`transport` + `host:port/path` ou `start_cmd`) dans `effect_of`, et calculer le `digest` du permis sur `(args, effet)`.

- [x] **[AU-C05] [P1 · S] Interdire l'envoi de requêtes signées en HTTP clair hors loopback (DA5) et bloquer la traversée de chemin dans `handle_server_unpair`**
  - **Constat** :
    1. Contrairement à `auth::cli` et `bridge`, `handle_server_pair` (`src/auth/net.rs:1229-1241`) accepte un `tls_fp` vide hors loopback, et `probe::http_request` (`src/probe.rs:4460-4486`) envoie les requêtes signées + arguments d'outils en clair sur `TcpStream` dès que `tls_fp` est vide.
    2. `handle_server_unpair` (`src/auth/net.rs:1088-1095`) lit `"keyid"` dans `.pin.json` sans vérifier `valid_keyid(&kid)` avant `remove_file(dir.join(format!("{kid}.key")))` (path traversal local si `.pin.json` altéré).
  - **Correction** : exiger `!tls_fp.is_empty()` hors loopback dans `handle_server_pair` et `probe::http_request`, et valider `valid_keyid(&kid)` avant toute suppression de fichier `.key`.

- [x] **[AU-C06] [P1 · S] Déplacer la mise à jour de `last_used_at` *après* la vérification Ed25519 et la persister périodiquement**
  - **Constat** : `RegistryStore::lookup` (`src/auth/registry.rs:551-567`) est appelé par `SignatureVerifier` **avant** de vérifier la signature Ed25519 : un attaquant non authentifié envoyant une fausse signature (`401`) sur un `keyid` connu efface son alerte d'inactivité `stale_180d` ! À l'inverse, `last_used_at` n'est jamais sauvegardé sur disque lors d'un usage légitime.
  - **Correction** : garder `lookup` strictement en lecture seule ; mettre à jour `last_used_at` uniquement après succès cryptographique (dans `note_hybrid_decision` hors `Unauthenticated`) avec sauvegarde différée/throttlée dans `config.json`.

- [x] **[AU-C07] [P1 · S] Réserver un quota dédié aux sessions CLI éphémères pour que `mcp-control auth revoke` fonctionne même si 8 onglets UI sont ouverts**
  - **Constat** : `src/auth/session.rs:64, 278-326` partage le plafond `MAX_SESSIONS = 8` entre les onglets UI (TTL 12–24 h) et les commandes CLI d'urgence (`auth revoke <keyid>`), et détruit le jeton `bootstraps.remove(idx)` (`line 286`) *avant* de tester `sessions.len() >= MAX_SESSIONS`.
  - **Correction** : séparer la réserve de sessions CLI éphémères (`MAX_EPHEMERAL_SESSIONS = 4` avec éviction FIFO) du plafond UI, et vérifier la capacité avant de consommer le jeton bootstrap.

- [ ] **[AU-C08] [P1 · M] Compléter le flux de rotation de clé (`mcp-control auth rotate`), `"public_key"` dans `auth pair` et le nettoyage des `.pin.json`**
  - **Constat** :
    1. Aucune commande CLI `mcp-control auth rotate` n'existe pour générer la nouvelle paire Ed25519, mettre à jour le Keyring et le `.pin.json` (l'UI `client_rotate` exige un copier-coller manuel de 64 hex, `src/ui/app.js:4984-5003`).
    2. `cmd_pair` (`src/auth/cli.rs:627-636`) omet `"public_key"` dans `<keyid>.pin.json` (cassant `server_auth_env` et `auth_fingerprint`), et `handle_server_pair` (`src/auth/net.rs:1301-1340`) écrase le Keyring sans purger les anciens `.pin.json` du même `(host, port)`.
  - **Correction** : ajouter `mcp-control auth rotate`, inclure `"public_key"` dans `cmd_pair`, et écrire le nouveau `.pin.json` puis purger les anciens pins du même `(host, port)`.

- [x] **[AU-C09] [P2 · M] Durcir `verify-log` avec trousseau actif, validité TLS ≤ 398 j, troncature HTTPS, loopback dual-stack `::ffff:127.0.0.1`, `Host` IPv6 et `ct_eq`**
  - **Constat** :
    1. `cmd_verify_log` (`src/auth/cli.rs:987`) teste `audit.key.exists()` sur disque sans interroger le Keyring OS : si `auth-audit.log` est supprimé alors que la clé HMAC est au Keyring (défaut !), `verify-log` retourne `0` (succès) !
    2. `rcgen::CertificateParams::new` (`src/auth/net.rs:290`) génère un `server.crt` valide de `1975` à `4096` (> 398 j → rejeté `ERR_CERT_VALIDITY_TOO_LONG` par Chrome/Edge/Safari en LAN).
    3. `https_exchange_raw` (`src/auth/net.rs:584-612`) ignore une troncature TLS (`body_bytes.len() < content_length` sur `UnexpectedEof`).
    4. `admit_conn` (`src/lib.rs:134`) est fail-open si `peer_ip == None`, `IpAddr::is_loopback()` dans `lib.rs` rejette `::ffff:127.0.0.1` en dual-stack IPv6 (utilisé par `throttle.rs:185`), et `ADMIN_ACTIONS` (`src/auth.rs:46-74`) omet `test_notify`, `litemcp_announcements`, `import_peer`, `set_favorite`.
    5. `auth::cli` (`src/auth/cli.rs:146, 228`) envoie `Host: ::1:5090` sans crochets en IPv6, et `ct_eq` (`src/crypto.rs:128-137`) n'utilise pas `std::hint::black_box` ni comparaison de SHA-256 à taille fixe pour `api_access`.
  - **Correction** : interroger `load_audit_key` dans `verify-log` ; borner `server.crt` à 365 jours ; vérifier `Content-Length` dans `https_exchange_raw` ; utiliser `ip.to_canonical().is_loopback()` partout + `peer_ip.is_some_and(...)` ; encadrer l'IPv6 `[::1]:port` dans `cli.rs` ; durcir `ct_eq` + mise à jour de `docs/auth.md`.

### 4.2 Optimisations (`AU-O*`)

- [ ] **[AU-O01] [P1 · S] Éliminer l'accès Keyring sur `access_list`, le double spawn `icacls.exe` et mettre en cache `VerifyingKey` + `ClientConfig` TLS 1.3**
  - **Constat** :
    1. `access_list` (`src/manage/actions.rs:2586-2615`) interroge le Keyring OS et recalcule la clé publique à chaque ouverture de l'écran « Accès » juste pour afficher `"keyring"` vs `"file_fallback"`.
    2. `write_secret_file` (`src/auth/secret.rs:127, 151`) et `write_head` (`src/audit.rs:183`) lancent `icacls.exe` deux fois (avant et après `fs::rename` sur le même volume NTFS).
    3. `RegistryStore::lookup` décompresse la clé Ed25519 deux fois par requête, `verifier_for` (`src/gateway.rs:199-214`) fuit les `AppState` dans un `HashMap<usize, ...>` statique, et `ConnStream::flush` (`src/lib.rs:1578`) envoie prématurément `send_close_notify()`.
  - **Optimisation** : mémoriser `SecretStore` dans `AuthState` au démarrage, supprimer le second `icacls.exe` post-rename, stocker le `Verifier` et les `VerifyingKey` pré-décodées directement dans `AppState`, partager `Arc<rustls::ClientConfig>` avec tickets TLS 1.3 PSK, et filtrer `serve_session_requests` sur le `mtime` du dossier + `wait_bg` (évitant 432 000 `read_dir`/jour au repos) avec `zeroize_slice` sur `Drop` des secrets en RAM.

---

## 5. Boutique (`src/manage/store.rs`, `src/store_glama.rs`, `src/store_github.rs`, `src/store_aitmpl.rs`)

### 5.1 Corrections (`BT-C*`)

- [x] **[BT-C01] [P0 · S] Corriger la duplication des arguments lors de l'installation de PostgreSQL depuis la Boutique**
  - **Constat** : `src/ui/app.js:4287-4300` envoie `body.args = ['-y', '@modelcontextprotocol/server-postgres', url.trim()]`, et `src/manage/actions.rs:2236-2237` concatène `req.args` **après** `it.args` (`['-y', '@modelcontextprotocol/server-postgres']`). Commande générée : `npx -y @modelcontextprotocol/server-postgres -y @modelcontextprotocol/server-postgres postgresql://...` → `server-postgres` lit `"-y"` comme URL de base de données et plante systématiquement au démarrage !
  - **Correction** : envoyer uniquement `body.args = [url.trim()]` dans `src/ui/app.js:4299`, et côté `actions.rs`, dédupliquer défensivement si `extra_args` commence par `it.args`.

- [x] **[BT-C02] [P1 · S] Remplacer la liste noire partielle d'`extra_args` dans `install_store` par le rejet strict de tout flag `-*`**
  - **Constat** : `src/manage/actions.rs:2206-2235` bloque `--package`, `--from`, `--with`, `--index-url`, mais laisse passer des flags critiques de `uvx`/`npx` tels que `--python=<binaire_arbitraire>`, `-f` / `--find-links`, `--config-file`, `--directory`, `--script-shell` ou `--userconfig`, permettant de détourner l'exécutable lancé via `install_store` / `install_store_server`.
  - **Correction** : comme `extra_args` ne sert qu'aux arguments positionnels (ex. URL PostgreSQL ou chemin de base SQLite), rejeter **tout** élément de `extra_args` commençant par `'-'` (et n'autoriser `extra_args` non vide que sur les items du catalogue déclarant explicitement accepter un argument positionnel).

- [ ] **[BT-C03] [P1 · M] Épingler les versions/commits des 14 serveurs `git+https://...` / `github:...` et des paquets `@latest` du catalogue**
  - **Constat** : 14 entrées dans `src/store_github.rs` (`lines 64, 88, 100, 112, 136, 152, 164, 184, 196, 213, 239, 253, 283, 299`) exécutent `uvx --from git+https://github.com/owner/repo` ou `npx -y github:owner/repo` directement sur la branche `HEAD` distante non épinglée, et plusieurs entrées `store.rs`/`store_aitmpl.rs` utilisent `@latest`. En cas de compromission d'un dépôt tiers ou d'un commit cassant sur `main`, du code arbitraire non audité est exécuté sur la machine de l'utilisateur.
  - **Correction** : épingler chaque entrée Git sur un tag de release vérifié ou un SHA de commit immuable (`git+https://github.com/owner/repo@<commit_sha>`) et remplacer `@latest` par des versions majeures/exactes testées.

- [x] **[BT-C04] [P1 · S] Corriger les chemins relatifs `test.db` (`sqlite`) et `--repo .` (`at-memex`) qui échouent en autostart Windows**
  - **Constat** : `sqlite` (`src/manage/store.rs:137`) passe `test.db` et `at-memex` (`src/store_aitmpl.rs:175`) passe `--repo .` avec `workdir: None`. Quand MCP Control démarre via la clé `Run` du registre Windows, le répertoire courant est `C:\Windows\System32` (ou `C:\Program Files\...`) → `PermissionDenied` immédiat.
  - **Correction** : définir un répertoire de données dédié sous `<config_dir>/data/<server_id>/` comme `workdir` par défaut des serveurs Boutique (ou demander le chemin DB/dossier à l'installation comme pour `postgres`).

- [ ] **[BT-C05] [P1 · M] Ajouter l'édition des variables d'environnement (`env`) d'un serveur installé sans désinstallation complète**
  - **Constat** : une fois un serveur Boutique installé, rappeler `install_store` renvoie `"déjà installé"` sans mettre à jour `env` (`src/manage/actions.rs:2248-2294`), et la vue Détail (`src/ui/app.js:2265-2282`) n'offre aucun éditeur de variables `env` : l'utilisateur voulant corriger une clé API (`GITHUB_TOKEN`, `BRAVE_API_KEY`…) est obligé de cliquer sur « Désinstaller » (qui supprime tout le cache `npx`/`uvx`) puis de réinstaller.
  - **Correction** : ajouter une action `set_server_env` (masquant les valeurs secrètes déjà saisies en UI, ne renvoyant jamais les secrets en clair dans `/api/state`) accessible depuis la vue Détail et depuis la carte Boutique d'un serveur déjà installé.

- [ ] **[BT-C06] [P2 · S] Réparer les 13 descriptions anglaises tronquées de `store_aitmpl.rs`, les logos `gh-*` et la compatibilité OS**
  - **Constat** :
    1. `src/store_aitmpl.rs:11, 91, 121, 131, 141, 151, 161, 171, 181, 191, 201, 241, 251` contient des descriptions anglaises brutes coupées en plein mot (`"create sound e..."`, `"JUPYT..."`, `"env :..."`, `"GOOGLE_OA..."`, `"fol..."`, `"wat..."`) au milieu d'une interface en français.
    2. `storeLogo(i)` dans `src/ui/app.js:4122` fait `i.id.replace(/^(at|gl)-/, '')` en oubliant `gh-` (contrairement à `srvStoreLogo` ligne `725`).
    3. Les serveurs exclusifs à un OS (`at-ios-simulator`, `at-local-mcp` macOS-only ; `windows-mcp` Windows-only) n'ont pas de métadonnée `os` et sont proposés sans avertissement sur les OS incompatibles.
  - **Correction** : rédiger des descriptions françaises propres et complètes dans `store_aitmpl.rs`, corriger la regex en `/^(at|gl|gh)-/`, et ajouter un champ optionnel `supported_os` avec badge/filtre d'avertissement dans la Boutique.

### 5.2 Optimisations (`BT-O*`)

- [x] **[BT-O01] [P2 · S] Enrichir la recherche et les filtres de la Boutique + mettre `store_json()` en cache `OnceLock`**
  - **Constat** : la recherche Boutique (`src/ui/app.js:4208-4215`) ne filtre que sur `name` et `desc` (chercher `"npx"`, `"uvx"`, `"GITHUB_TOKEN"`, `"postgres"` par ID ou `"python"` par runtime rate des résultats pertinents), et `store_json()` (`src/manage/store.rs:457-466`) reconstruit le `Vec<Value>` de 95 entrées à chaque appel HTTP ou MCP `store_catalog`.
  - **Optimisation** : indexer `id`, `cat`, `kind`, `program`, `args` et `env_vars` dans la recherche Boutique, ajouter des filtres rapides (`Sans clé API`, `Node / npx`, `Python / uvx`, `Compatible avec cet OS`), et cacher `store_json()` dans un `OnceLock<String>` (couplé à **MC-O03**).

---

## 6. Points historiques conservés & à arbitrer (issus des passes `v0.7.x`)

Les éléments suivants des anciens suivis (`R1`–`R17`) ont été re-vérifiés sur `v0.8.9` :
- **Déjà livrés en `v0.8.x`** (clos) : **R5** (vérification d'empreinte SHA-256 à la mise à jour livrée dans `src/manage/update.rs`), **R9** (saisie des variables `env` à l'installation Boutique livrée dans `openStoreInstallModal`).
- **Intégrés dans les tickets ci-dessus** : **R1** → `MC-C08` · **R4 / R17** → `UI-C04`.
- **Restent ouverts en `P3` (finition / arbitrage produit)** :
  - [ ] **[P3 · S] Isoler la découverte locale en mode test E2E (`R2` / `R6` / `R11`)** : ajouter une garde de configuration (`discover_all_listening_ports: bool` ou variable d'environnement du harnais E2E) pour éviter que `discover` et `poll_all` n'importent `%TEMP%\litemcp-registry` ni ne sondent tous les ports de l'hôte pendant les tests, et ne jamais appeler `set_autostart` réel sans mock en E2E.
  - [ ] **[P3 · S] Normaliser les fins de ligne de `src/ui/app-boot.js` (`R12`) et ajouter un test unitaire dédié à l'anti-request-smuggling HTTP (`TE` + `Content-Length` dupliqué)**.
  - [ ] **[P3 · S] Rafraîchir les captures d'écran du `README.md` (`R15`)** : mettre à jour `assets/readme-accueil.png`, `assets/readme-detail.png` et `assets/readme-rust.png` avec l'interface `v0.8.9` (volets Sécurité/Auth, Boutique 95 serveurs, approbations).

---

## Ordre d'exécution recommandé par lots

1. **Lot 1 — Urgences `P0` Sécurité & Disponibilité (~1,5 à 2 j)**
   `GW-C01` (confirmation humaine sur `/mcp/s/<id>`) → `AU-C01` (anti-spoofing `confirm.rs` sur `add_server` HTTP & `import_config`) → `AU-C02` (port authentifié dans `.grant`) → `BT-C01` (fix PostgreSQL Boutique) → `MC-C01` & `MC-C02` (session Desktop tray + `probe_hosts` loopback) → `MC-C03` (ordre d'arrêt NSIS) → `MC-C04` (rotation atomique `auth-audit.log`).
2. **Lot 2 — Durcissement Auth, Gateway & Supply-Chain Boutique (~2 à 2,5 j)**
   `GW-C02` à `GW-C04` (SEP-2243 strict, capacités hybrides, bridge `stdin` borné + reload `401`) → `AU-C03` à `AU-C09` (éviction `throttle` `> now`, anti-TOCTOU `window`, DA5 TLS sortant + `valid_keyid`, `last_used_at` post-sig, quota sessions CLI `revoke`, `auth rotate`, `verify-log` Keyring + cert ≤ 398 j) → `BT-C02` à `BT-C05` (rejet flags `-*`, pinning SHA Git, `workdir` données, édition `env`).
3. **Lot 3 — Fiabilité Supervision, Probe, SSH & Corrections UI (~2 j)**
   `MC-C05` à `MC-C13` (`ui_bind "::"`, `bound_ui_port`, `probe_stdio`, matching exact de ports, SSH Windows + sans `/tmp`, IPv6 `Host`, décodeur `chunked`, UNC mixtes, webhooks async) → `UI-C01` à `UI-C07` (`hasAuthEnv`, actions batch `set_all_tools`/`refresh_all`, `sshBootstrap`, `doScanNet`, tokenizer quotes, `ui-dist`).
4. **Lot 4 — Optimisations Performances, Multi-OS & Documentation (~2 j)**
   `MC-O01` à `MC-O04` (`/api/state` zéro I/O, `ProbeHint` 0-tool, assets `OnceLock` + `ETag`, ThinLTO) → `GW-O01` & `GW-O02` (bridge TLS keep-alive, `list_peers` direct) → `AU-O01` & `BT-O01` → `MC-C14`, `MC-C15`, `UI-O01`, `UI-O02`, `GW-C05`, `GW-C06`, `BT-C06` & finitions `P3`.
