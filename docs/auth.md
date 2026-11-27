# Authentification de MCP Control — spécification

Statut : **livré en 0.8.0**, aligné sur l'implémentation réelle (`M1`–`M5`, `S1`–`S4`, `R-fix`, `litemcp = "0.12.2"` publié sur crates.io).
Décisions appliquées : **DA1 à DA7 = option a**.

**Autorité.** La requête signée, les identités, les droits, la confirmation externe et TLS sont définis par le
`docs/auth.md` de `litemcp` 0.12.2, qui fait foi. Ce document ne les redéfinit pas : il fixe ce que MCP Control
choisit par-dessus, et rien de plus. Un lecteur qui n'a que ce fichier doit pouvoir écrire un client compatible ;
chaque « DOIT » est testable ; chaque choix cite sa décision.

## 1. Modèle de menace

### 1.1 Process local sans droits (script, autre application, service)

- **Avant 0.8.0** : contrôle total sans aucun secret en loopback (`/mcp` et `POST /api/action` ouverts sans identité,
  catalogue entier accessible, `add_server` et `start_server` compris).
- **En 0.8.0** : `/mcp` sans signature valide → `401` de corps constant (DA2) ; `/api/*` sans session locale complète
  (cookie `mcc_session` **et** en-tête `X-MCC-Session`, §6) → `401` ; les outils sont bornés par les droits du client
  (§4) et tout lancement passe par la confirmation humaine (§5).
  *Limite* : un processus qui s'exécute avec le **même compte utilisateur OS** reste dans le même périmètre de confiance
  (il peut lire le trousseau, le dossier de configuration ou le canal fichier local de session, §1 Hors de portée).

### 1.2 Page web ouverte dans un navigateur de la machine

- **En 0.8.0** : ne peut ni lire ni écrire. `Host` absent ou dupliqué → `400`, `Host` inconnu ou port étranger → `403`,
  `Origin` dupliqué → `400`, `Origin` étranger → `403`, `OPTIONS` → `403`, `POST /api/action` non JSON → `415`,
  `Access-Control-Allow-Origin` reflété seulement pour une origine autorisée. Le cookie `mcc_session` est
  `HttpOnly; SameSite=Strict`, l'en-tête `X-MCC-Session` (secret en mémoire JS) impose un préflight CORS refusé, et
  la CSP est stricte (`default-src 'none'; script-src 'self'; style-src 'self'`). Le rebinding DNS reste bloqué par
  `Host`.

### 1.3 Machine du LAN qui écoute (passive)

- **Avant 0.8.0** : capturait le code partagé en clair (`X-MCC-Access` ou `?access=`) sur du HTTP sans TLS.
- **En 0.8.0** : TLS 1.3 obligatoire hors loopback (DA5, §7) et signature Ed25519 par requête (DA7). Une requête
  capturée n'est pas rejouable (nonce unique, fenêtre ±60 s, cache borné par clé, refus des requêtes signées avant le
  démarrage courant `BOOT_TS` et vérification stricte du port dans `Host`, §2.2).

### 1.4 Machine du LAN active

- **En 0.8.0** : sans clé valide, `401` de corps constant et temps de réponse uniforme ; échecs comptés par adresse
  **et** par `keyid`, délai croissant (dès 8 échecs) puis blocage temporaire `429` (à 40 échecs), sans jamais bloquer
  le loopback depuis le LAN (`src/auth/throttle.rs`). Les requêtes avec en-têtes critiques dupliqués (`Host`, `Origin`,
  `Cookie`, `X-MCC-Session`, `Content-Length`) ou `Content-Length` non numérique sont rejetées en `400 Bad Request`.

### 1.5 Voleur de `config.json`

- **Avant 0.8.0** : obtenait `api_access` en clair (équivalent exécution) et les valeurs `env` stdio, sans ACL Windows.
- **En 0.8.0** : `config.json` ne contient aucune clé privée (§9 : clés publiques, droits, dates). Les fichiers de
  données (`config.json`, `state-cache.json`, `mcc.log`, `auth-audit.log`) sont écrits de façon atomique avec ACL
  explicite (`0600` sous Unix ; utilisateur courant + `SYSTEM` + `Administrateurs` sans héritage sous Windows). Les
  valeurs `env` stdio et les en-têtes webhook restent dans `config.json` (protégés par ces ACL) et ne ressortent jamais
  par l'API, le gateway ni les journaux. Le code `api_access` hérité disparaît en 0.9.0 (DA3). Un processus du même
  utilisateur peut toujours lire ces fichiers : hors de portée.

### 1.6 Voleur d'une clé client

- **En 0.8.0** : il signe à la place de ce client et obtient exactement ses droits ; `execution` exige en plus une
  approbation dans la fenêtre locale (§5) ; la révocation (`client_revoke` ou `mcp-control auth revoke`) prend effet à
  la requête suivante (lib §4) ; chaque décision est au journal chaîné (§11).

### 1.7 Serveur MCP supervisé malveillant

- **En 0.8.0** : l'injection de contenu dans une réponse d'outil reste possible (hors de portée), mais son effet est
  borné par les droits de l'appelant et par la confirmation humaine (§5). Les annonces écrites dans
  `%TEMP%\litemcp-registry` ne sont plus adoptées ni démarrées d'office : elles sont listées en lecture seule
  (`litemcp_announcements`, propriétaire vérifié, plafond 64 Kio / 32 entrées) et nécessitent un ajout explicite.

### 1.8 Agent IA manipulé par injection de prompt

- **En 0.8.0** : il ne peut que ce que ses droits permettent ; tout lancement ou appel d'outil distant (`execution`)
  produit une demande visible dans la fenêtre locale, avec le programme, les arguments exacts et l'identité du client
  (§5) ; la séquence complète reste au journal d'audit chaîné (§11).

### Hors de portée (écrit noir sur blanc)

Malware avec les droits de l'utilisateur (il lit le trousseau, pilote la fenêtre, remplace le binaire) ; binaire non
signé (SmartScreen/Gatekeeper restent le maillon suivant) ; OS ou dépendance compromis ; accès physique ; canaux
auxiliaires (temps, mémoire) ; déni de service au-delà des plafonds existants (64 connexions, 16 par adresse, 2 Mio —
`src/lib.rs:27-41`) ; confidentialité des arguments côté serveur ; terminaison TLS par un proxy ; mTLS ; un utilisateur
malveillant (même utilisateur = même autorité) ; signature des **réponses** du serveur (point non tranché n° 4).
Aucune évaluation par un tiers n'est revendiquée.

## 2. Requête signée

### 2.1 Ce que MCP Control ne redéfinit pas

Renvoi : `docs/auth.md` de `litemcp` 0.12.2, §2 en entier. Résumé en dix lignes :

1. `POST` seul ; `Signature-Input` et `Signature` (RFC 9421 §4.1/§4.2), exactement un membre chacun, même label.
2. Composants couverts, ordre imposé : `("@method" "@path" "@authority" "content-digest" "content-type" "mcp-method" "mcp-name")` ou la même liste moins `mcp-name`.
3. `Content-Digest` obligatoire (RFC 9530 §2), un seul membre `sha-256`, base64 standard avec padding.
4. Paramètres, dans l'ordre : `created`, `nonce` (22 caractères base64url = 16 octets du système), `keyid`, `alg="ed25519"`, `tag="litemcp-auth-v1"`.
5. Chaîne à signer : RFC 9421 §2.5, lignes séparées par LF, `@signature-params` sans LF final, ASCII seulement.
6. Ed25519 pur (RFC 8032 §5.1.6), signature de 64 octets, base64 standard dans `Signature: sig1=:…:`.
7. `created` dans `[now−60 s, now+60 s]` ; nonce neuf par `keyid`, cache de 4096 nonces par clé sur 120 s, refus si plein.
8. `@path` = chemin absolu sans query, `%XX` non décodés ; `@authority` = `Host` normalisé.
9. 401 de corps constant + `WWW-Authenticate: Signature realm="litemcp", tag="litemcp-auth-v1"` ; 403 `-32023` avec `data.reason`.
10. Ordre des contrôles : en-têtes → composants → digest → horloge → nonce → `keyid`/`KeyStore` → Ed25519 en dernier.

Un client qui n'applique pas ces dix points ne se connecte pas. MCP Control ne tolère aucune variante (§2.2).

### 2.2 Ce que MCP Control fixe lui-même

| Point | Valeur fixée | Pourquoi |
|---|---|---|
| Chemins couverts | `/mcp` exactement, en `POST`. Aucune autre route n'accepte de signature ; une requête signée vers un autre chemin rend 404 **avant** vérification. | Un seul point d'entrée signé : moins de surface (DA2). |
| `@authority` attendue | La valeur du `Host` reçu, qui DOIT être unique (doublon → `400`), figurer dans `allowed_hosts` (interfaces locales + `allowed_origins`), et porter exactement `ui_port` (port différent → `403`, port mal formé → `400` ; sur `/mcp`, `Host` sans port refusé en `403` hors ports par défaut `80`/`443`). `X-Forwarded-*` n'est jamais lu. | Empêche le rejeu inter-instances / inter-ports (`R-1`) et le rebinding DNS ; pas de proxy supposé. |
| Fenêtre d'horloge & redémarrage | ±60 s, non élargie. De plus, le moteur enregistre `BOOT_TS` au démarrage et rejette en `401` uniforme toute requête `/mcp` signée dont `created < boot_ts` (`R-2`). Un refus `clock_skew` est journalisé avec l'écart, jamais renvoyé au client. | Ferme le rejeu dans la fenêtre ±60 s après un redémarrage du moteur, sans E/S disque. |
| Cadrage & en-têtes critiques | Tout doublon de `Host`, `Origin`, `X-MCC-Session`, cookie `mcc_session=` ou `Content-Length`, ainsi qu'un `Content-Length` non numérique ou `Transfer-Encoding: chunked`, est rejeté en `400 Bad Request` (`R-3`). | Conformité RFC 9110 §7.2 ; prévient toute désynchronisation d'analyse. |
| Corps | ≤ 2 Mio (`src/lib.rs:27`), digest calculé sur les octets reçus après décadrage. | Le digest doit couvrir exactement ce que le noyau analyse (lib §2.2). |
| Délai de requête | 5 s (`src/lib.rs:41`), vérification comprise (Ed25519 ≈ 78 µs mesurés par l'audit LR). | Un refus doit coûter moins cher qu'une requête valide (lot M5). |
| `Mcp-Method` / `Mcp-Name` | Obligatoires et cohérents avec le corps, contrôlés par le transport avant la vérification. | SEP-2243 ; déjà en place. |
| Réponses | **Non signées.** L'intégrité des réponses repose sur le transport (loopback, ou TLS hors loopback). | Trou déclaré : point non tranché n° 4. |
| OAuth / jetons porteurs | Aucun. Pas de `Bearer`, pas de `/.well-known/oauth-protected-resource`, pas de document RFC 9728 — le servir laisserait croire à un flux que DA6=a exclut. | La spec MCP 2026-07-28 rend l'autorisation facultative et, sur HTTP, OAuth 2.1 ; M6 n'est pas lancé. |

## 3. Identités

### 3.1 `keyid` et empreinte

- Grammaire admise par la lib (fait foi) : 1 à 128 octets ASCII, `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$`.
- MCP Control **resserre** : `^mcc-[a-z0-9]{1,8}-[0-9a-f]{16}$` (ex. `mcc-ai-3f9a1c07b2d84e51`). Les 16 hexadécimaux
  viennent de 8 octets du système, tirés par le client. Un `keyid` déjà présent dans le registre → demande refusée
  (409), jamais écrasée.
- Empreinte : `ed25519:` + `SHA-256(clé publique)` en hexadécimal minuscule (68 caractères). Affichage court :
  `ed25519:` + les 16 premiers hexadécimaux. Comparaison en temps constant (`Fingerprint::ct_eq` de la lib).
- L'identité du serveur est fixe : `mcc-server`, créée au premier lancement (§9).

### 3.2 Registre des clients (`config.json`, `auth.clients[]`)

| Champ | Type | Règle |
|---|---|---|
| `keyid` | chaîne | §3.1, unique, immuable. |
| `label` | chaîne ≤ 64 | Fourni par le client, affiché **échappé** (`esc()`), jamais interprété comme du HTML. |
| `public_key` | 64 hex | 32 octets Ed25519. Jamais de clé privée ici. |
| `rights` | tableau | Sous-ensemble de `lecture`, `pilotage`, `execution` ; jamais `admin`. Ensemble vide = clé inerte. |
| `created_at` / `last_used_at` | ISO UTC | `last_used_at` écrit au plus une fois par minute. |
| `status` | `active` \| `revoked` | `revoked` = refus à la requête suivante (lib §4). |
| `revoked_at`, `rotated_to` | optionnels | Traçabilité d'une révocation ou d'une rotation. |
| `paired_from` | adresse IP | Celle de la demande d'appairage, affichée à l'approbation. |
| `expires_at` | optionnel | Absent par défaut : une clé n'expire pas seule. |

### 3.3 Cycle de vie

- **Appairage** : demande valable 2 minutes, 16 demandes en attente au plus (la plus ancienne évincée), 5 demandes par
  adresse et par minute. Rien n'est accordé avant une approbation locale.
- **Rotation** (`client_rotate`) : nouveau `keyid`, mêmes droits, ancien passé `revoked` avec `rotated_to`. Un client ne
  peut pas demander sa propre rotation depuis `/mcp` : c'est une action de la fenêtre locale (droit `admin`).
- **Révocation** (`client_revoke`) : immédiate, à la requête suivante ; testé au lot M1.
- **Expiration** : aucune par défaut ; l'écran « Accès » signale les clés sans usage depuis 180 jours, sans jamais
  révoquer d'office. Le grant de confirmation, lui, expire (10 min, §5), et la demande d'appairage aussi (2 min).

### 3.4 Protocole d'appairage, message par message

Tous les messages sont en JSON, sur le même port que l'UI. `tls_fp` est vide quand l'écoute est en loopback.

1. **Client (local)** : génère sa paire Ed25519 et son `keyid` (§3.1) ; la clé privée ne quitte jamais sa machine.
2. **Client → serveur** : `POST /auth/pair` avec `{ "keyid", "label", "public_key": "<64 hex>", "created": <s UNIX> }`
   (`created` hors ±60 s → refus). Réponse : `{ "pair_id": "<32 hex>", "expires_in": 120, "api": "mcc-auth-v1",
   "server_keyid": "mcc-server", "server_pub": "<64 hex>", "tls_fp": "sha256:<64 hex>"|"" }`.
   Le client compare `tls_fp` à l'empreinte observée pendant sa poignée de main TLS ; différentes → il abandonne.
3. **Calcul du code, des deux côtés** :
   `code = SHA-256("mcc-pair-v1" ‖ pair_id ‖ client_pub ‖ server_pub ‖ tls_fp)[0..4]`, lu en entier 32 bits
   big-endian, modulo 1 000 000, affiché en six chiffres groupés (`123 456`). Le client l'affiche ; la fenêtre affiche
   le même calcul pour la demande correspondante.
   *Le code n'est pas un secret* : il ne vaut aucune autorisation. Il sert uniquement à ce qu'un humain détecte une
   substitution de clé. Un attaquant qui relaie deux connexions distinctes obtient deux codes différents, puisque les
   deux côtés ne voient pas la même paire de clés ni le même `tls_fp`.
4. **Utilisateur (fenêtre locale, droit `admin`)** : lit `label`, `keyid`, empreinte courte, `paired_from`, compare le
   code, choisit les droits, approuve ou refuse. Refus et expiration sont définitifs pour ce `pair_id`.
5. **Client → serveur** : `GET /auth/pair/{pair_id}` toutes les 2 s (limité à 60 req/min/IP par le serveur, `429` au-delà).
   `{ "status": "pending"|"approved"|"rejected"|"expired", "rights": [...], "server_pub", "tls_fp" }`.
   Sur `approved`, le client épingle l'empreinte de `server_pub` **et** `tls_fp` (lib C9, `PinnedServer`), puis range
   `keyid` + clé privée au trousseau (§9). Sur `rejected`/`expired`, il efface la clé générée.

Un `pair_id` inconnu rend 404 : les demandes en attente ne sont pas énumérables sans connaître ces 16 octets.

## 4. Droits

`admin` n'est **jamais** accordé à un client du gateway : il n'existe que sur la session locale (§6), qui n'est jamais
valable sur `/mcp`. Les quatre droits sont des chaînes opaques pour la lib (lib §4) ; MCP Control en fixe quatre et
déclare chacun des 30 outils avec `require_right` (C2). **Tout outil enregistré sans droit déclaré est refusé**, même
pour un client `execution` : c'est l'échec fermé de la lib (§4), et le test du lot M2 échoue si un outil n'a pas de
droit.

| Outil | Droit | Motif |
|---|---|---|
| `list_servers` | `lecture` | État des serveurs. |
| `server_detail` | `lecture` | Détail d'un serveur. |
| `list_server_tools` | `lecture` | Noms d'outils exposés par un serveur. |
| `journal` | `lecture` | Journal applicatif ; jamais de secret (§11). |
| `runtime_history` | `lecture` | Historique runtime. |
| `list_peers` | `lecture` | Pairs du LAN (topologie). |
| `store_catalog` | `lecture` | Catalogue statique de la boutique. |
| `list_frameworks` | `lecture` | Table statique des frameworks. |
| `get_settings` | `lecture` | Réglages ; ne rend **jamais** `api_access`, un `env` ni un en-tête de webhook. |
| `check_update` | `lecture` | Un GET vers GitHub, aucun effet local. |
| `refresh` | `pilotage` | Re-sonde ; ne lance rien. |
| `scan_ports` | `pilotage` | Ouvre des sockets sortantes. |
| `scan_network` | `pilotage` | Balayage du LAN. |
| `discover_peers` | `pilotage` | ARP + `/api/hello` des pairs. |
| `toggle_tool` | `pilotage` | Coupe un outil du proxy. |
| `reorder_servers` | `pilotage` | Ordre d'affichage et de sondage. |
| `set_host` | `pilotage` | Réoriente un serveur HTTP ; aucun lancement. |
| `remove_server` | `pilotage` | Retire de la liste (destructif, journalisé). |
| `ssh_forget` | `pilotage` | Oublie une machine distante. |
| `stop_server` | `pilotage` | Tue un process (destructif, journalisé). |
| `add_server` | `execution` | Écrit une commande de lancement. |
| `start_server` | `execution` | Lance le programme → confirmation. |
| `toggle_server` | `execution` | **Déplacé** depuis `pilotage` : ON remet `start_cmd`, arme `auto_restart`, sauvegarde puis appelle `poll_one` — le process stdio démarre (`src/manage/actions.rs:69-121`). |
| `set_start_command` | `execution` | Écrit ce qui sera exécuté → confirmation. |
| `set_auto_restart` | `execution` | **Déplacé** depuis `pilotage` : armer la relance fait exécuter la commande au prochain boot, sans nouvelle requête (`src/gateway.rs:758-767`). |
| `install_store_server` | `execution` | Installe et enregistre une commande → confirmation. |
| `import_config` | `execution` | Importe des programmes → confirmation. |
| `call_server_tool` | `execution` | Exécute un outil distant arbitraire → confirmation. |
| `ssh_save` | `execution` | Enregistre la cible que `ssh_discover` contactera avec la clé SSH. |
| `ssh_discover` | `execution` | Exécute `ssh` vers une machine → confirmation. |

Répartition : `lecture` 10, `pilotage` 10, `execution` 10. Les deux déplacements répondent à un seul invariant :
**aucun chemin ne mène à l'exécution d'un programme sans le droit `execution`**. Un outil qui n'est pas dans ce
tableau n'existe pas ; tout ajout DOIT modifier ce tableau dans la même révision.

## 5. Confirmation humaine (DA4 = a)

- **Quels appels** : les dix outils `execution`, sans exception, y compris `call_server_tool` (un outil distant peut
  tout faire). Aucun autre outil n'est confirmé : `stop_server` et `remove_server` sont destructifs mais ne lancent
  rien, ils sont seulement journalisés.
- **Ce que voit l'utilisateur**, dans la fenêtre locale et dans la notification : libellé et empreinte courte du
  client, adresse source, outil, et l'effet exact — programme, arguments et dossier de travail pour un lancement
  (`start_server`, `toggle_server`, `set_start_command`, `set_auto_restart`, `install_store_server`, `import_config`) ;
  `ip` + `user` pour `ssh_*` ; serveur cible, nom d'outil et arguments pour `call_server_tool`. Les **valeurs** de
  variables d'environnement et les secrets sont remplacés par les noms des variables.
- **Durée** : approbation valable 10 minutes pour ce client **et** cet outil ; révocable depuis le même écran ; perdue
  au redémarrage du moteur.
- **En attendant** : le client reçoit `403`, code JSON-RPC `-32023`, `data.reason = "approval_pending"`,
  `data.retryable = true`, `data.retryAfterMs` — le handler n'a pas tourné et rien n'est mis en file (lib §5). Le
  client réessaie avec un nouveau corps, un nouvel `id`, un nouveau `created` et un nouveau nonce.
- **Bornes et isolation** : 32 demandes en attente au plus, expiration à 2 minutes. Un client ne peut ni approuver sa
  propre demande ni voir celles d'un autre. L'approbation n'est possible que par la session locale (droit `admin`),
  jamais par `/mcp`. Ce qui est approuvé est exactement ce qui s'exécute : la politique reçoit les arguments après
  validation et injection des défauts, et le handler reçoit la même valeur (lib §5).

## 6. Session locale de la fenêtre (DA2 = a, lot S1)

- **Obtention** : au lancement, le moteur tire un jeton d'amorçage (32 octets de l'OS, base64url — échec fermé si
  l'entropie OS est indisponible), valable 5 minutes et **à usage unique**. Le shell Tauri navigue vers
  `http://127.0.0.1:<port>/#t=<jeton>` (fragment : jamais envoyé au serveur, jamais dans `Referer`, jamais dans
  `mcc.log`). `app.js` échange le jeton sur `POST /api/session`, reçoit à la fois le cookie `mcc_session` **et** un
  secret de session en JSON (32 octets base64url, conservé uniquement en mémoire JS), puis purge immédiatement le
  fragment (`history.replaceState`).
- **Second lancement & CLI locale (canal fichier utilisateur)** : lorsqu'une instance tourne déjà (fenêtre rouverte,
  second lancement headless ou commandes CLI `auth list` / `auth revoke`), le processus local demande un jeton
  d'amorçage à usage unique (2 min) via deux fichiers éphémères (`mcc-session.req` / `mcc-session.grant`) dans le
  dossier de `config.json`. Chaque fichier est créé atomiquement (`.tmp` vide → ACL restreintes à l'utilisateur courant
  → écriture + `sync_all` → `rename`), supprimé dès lecture et jamais journalisé.
  *Limite* : un processus qui s'exécute avec le même compte utilisateur OS est déjà dans le périmètre de confiance (il
  lit le trousseau et `config.json`, §1 Hors de portée) ; un autre compte utilisateur OS ne peut ni déposer la demande
  ni lire la réponse.
- **Forme et double preuve** : toute requête `/api/*` exige **à la fois** :
  1. le cookie `mcc_session` (`HttpOnly; SameSite=Strict; Path=/`, sans `Secure` en loopback HTTP) ;
  2. l'en-tête `X-MCC-Session: <secret>` (secret reçu en JSON lors de `POST /api/session`, jamais stocké dans
     `localStorage` ni dans l'URL).
  Les deux valeurs sont comparées en temps constant (`ct_eq`). L'ancien en-tête `X-MCC-Request: 1` est supprimé : la
  présence de l'en-tête personnalisé `X-MCC-Session` suffit à imposer un préflight CORS refusé.
- **Sessions multiples et durées** : jusqu'à **8 sessions locales simultanées** (la 9ᵉ est refusée). Chaque session
  expire après **12 h d'inactivité** (`last_seen` mis à jour à chaque requête) ou **24 h absolues**, et meurt au
  redémarrage du moteur. La route `POST /api/session/close-others` révoque toutes les autres sessions tout en gardant
  la session courante active (exposée dans l'onglet *Identité & Sessions* de la fenêtre).
- **Protection CSRF et CSP** : `Host` et `Origin` vérifiés et uniques (`400`/`403`), `OPTIONS` → `403`, `POST /api/action`
  non JSON → `415`, cookie `SameSite=Strict`, en-tête secret `X-MCC-Session`, et CSP stricte (`default-src 'none';
  script-src 'self'; style-src 'self'`, 0 script/style inline).
- **Portée** : la session locale porte `admin` et n'est **jamais** acceptée sur `/mcp`. Elle ne remplace pas la
  signature : un client MCP, même local, signe.

## 7. TLS (DA5 = a)

- **Écoute** : loopback en HTTP (les requêtes y sont signées) ; hors loopback, TLS 1.3 obligatoire. Sans la feature
  `tls`, le moteur **refuse** d'écouter ailleurs qu'en loopback, reste en `127.0.0.1` et l'écrit dans le journal et
  dans l'interface. Aucun repli silencieux vers HTTP.
- **Certificat** : auto-signé, généré au premier lancement et **persisté par MCP Control** (la lib n'écrit jamais de
  clé sur disque, lib §9) ; SAN = `localhost`, `127.0.0.1`, `::1` et les adresses locales du moment, complétées au
  besoin. Une révocation de certificat se fait en supprimant les deux fichiers et en ré-appairant.
- **Liaison à l'identité** : le certificat n'est **pas** signé par la clé Ed25519 du serveur (cela supposerait une
  autorité, hors périmètre). Le lien est fait à l'appairage : le client épingle `server_pub` **et** `tls_fp` dans le
  même enregistrement, et le code de comparaison couvre les deux (§3.4). Un certificat substitué change le code.
- **Suites et version** : TLS 1.3 seulement ; les trois suites de `rustls` 0.23.46 (`TLS13_AES_256_GCM_SHA384`,
  `TLS13_AES_128_GCM_SHA256`, `TLS13_CHACHA20_POLY1305_SHA256`), non configurables ; ALPN `http/1.1` seulement. TLS 1.2
  échoue à la poignée de main.
- **Épinglage** : `PinnedServer` refuse toute empreinte différente, sans option de contournement ; `mcp-control auth pair`
  refuse un serveur dont l'identité ou le certificat a changé et le dit.
- **Pairs et serveurs LiteMCP verrouillés (lot M4)** : les pairs appairés sont interrogés via `POST /mcp` (`list_servers`)
  signé en `lecture` (le repli transitoire DA3 0.8.x reste actif pendant 0.8.x). Un serveur LiteMCP distant qui répond
  `401` avec `WWW-Authenticate: Signature` est classé « joignable mais verrouillé » (`locked: true`, badge cadenas) ;
  l'action locale `server_pair` (`admin`, inaccessible sur `/mcp`) l'appaire, range la clé cliente dans le trousseau
  (ou repli `auth/clients/<keyid>.key` + `<keyid>.pin.json`) et active les sondes ainsi que le proxy `call_server_tool`
  signés de bout en bout.

## 8. API à ajouter, et correspondance C1–C12

### 8.1 Modules et routes

| Ajout | Contenu |
|---|---|
| `src/auth.rs` + `src/auth/{registry,pair,session,confirm,secret,cli,throttle}.rs` | Registre (`KeyStore`), appairage, session locale S1, politique de confirmation M3, stockage atomique des secrets, CLI `auth`, anti-force brute M5. |
| `src/auth/net.rs` | TLS 1.3 entrant, épinglage sortant, action locale `server_pair` (M4). |
| `src/bridge.rs` | Pont stdio `mcp-control bridge` (M5). |
| `src/audit.rs` | Journal d'audit chaîné HMAC + ancre `auth-audit.head` (§11, M5). |
| `POST /auth/pair`, `GET /auth/pair/{id}` | Famille d'appairage (§3.4), non authentifiée, limitée par adresse (5 req/min sur `POST`, 60 req/min sur `GET`). |
| `POST /api/session`, `POST /api/session/close-others` | Échange du jeton d'amorçage contre cookie + secret `X-MCC-Session`, et révocation des autres sessions locales (§6). |
| `POST /mcp` | Vérifie `Host` (port), `BOOT_TS`, signature Ed25519 puis dispatche (C8) ; plus aucun appel direct au noyau. |
| `config.json` | `auth.server` (`keyid`, `public_key`, `created_at`), `auth.clients[]` (§3.2), `auth.session_bootstrap_secs`, `auth.confirm_secs`, `auth.audit_path`, `tls.cert_path`, `tls.key_path` (chemins seulement). |
| Suppressions | `?access=` supprimé en 0.8.0 (M2) ; `X-MCC-Request` supprimé en 0.8.0 (S1) ; `X-MCC-Access`, `X-MCC-Proof` et `/api/hello` conservés en 0.8.x pour le repli transitoire DA3 des pairs et supprimés en 0.9.0. |

### 8.2 Contrat de la lib, ligne par ligne

| # | Où MCP Control s'en sert |
|---|---|
| C1 | `AuthContext` : identité de l'appelant dans les handlers, le journal et l'écran « Accès » ; `auth::current()` dans `call_server_tool` pour journaliser le vrai client et non le proxy. |
| C2 | `require_right` : les 30 déclarations du §4, dans `build_builder`. |
| C3 | Filtrage des listes : `tools/list` ne rend que les outils du client (10 pour `lecture`) ; test du lot M2. |
| C4 | `on_auth_decision` : une ligne de journal par décision (§11) et les 100 dernières décisions de l'écran « Accès ». |
| C5 | `require_approval` + `approval_policy` : §5. |
| C6 | `KeyStore::lookup` : lit `auth.clients`, à chaque requête (une révocation prend effet tout de suite). |
| C7 | `SignatureVerifier::new(Arc<dyn KeyStore>, NonceCache)` pour `/mcp` — politique par défaut de la lib (4096 par clé, 120 s), non modifiée ; `Signer` pour le pont, les sondes et le proxy sortants. |
| C8 | `dispatch_verified(&VerifiedRequest) -> AuthResponse` remplace `core.dispatch_with_headers` (`src/gateway.rs:1329`) ; `McpHeaders::from_headers` construit les en-têtes depuis la requête brute. |
| C9 | `Signer` + `PinnedServer` : pont, `auth pair`, sondes et proxy vers un serveur LiteMCP verrouillé (M4). |
| C10 | **Non utilisable tel quel** : `TlsConfig::server_config` est `pub(crate)` (`litemcp/src/tls.rs:233`) et MCP Control sert son propre socket. MCP Control utilise `tls::CertificateFingerprint` et construit son `ServerConfig` avec `rustls` + `rcgen` (§7). Écart déclaré. |
| C11 | Sans objet côté MCP Control : pas de `/health` de la lib (le sien n'existe pas), pas de document RFC 9728 (§2.2). Les 401 viennent de C8. |
| C12 | Aucune signature publique de MCP Control ne change ; la contrainte de version et le littéral `McpHeaders` sont le lot M0 ; la version publiée de `litemcp` est le lot D. |

### 8.3 Features et crates propres à MCP Control

- `litemcp = "0.12.2"` (publié sur crates.io) : `auth-sign` **obligatoire** (sans lui, aucun vérificateur) ; `tls` pour
  DA5. Sans `tls`, le moteur compile et reste en loopback (§7).
- `keyring = "=4.2.0"` (licence MIT OR Apache-2.0, `default-features = false, features = ["v1"]`), derrière la feature
  par défaut `keyring`. Taille mesurée sous Windows release (lot S3) : **+1,63 Mo** (3,57 Mo sans feature → 5,20 Mo avec
  `keyring`) ; sans cette feature, le moteur compile et bascule sur le repli fichier `0600`/ACL journalisé.
- `rustls = "=0.23.46"` et `rcgen = "=0.14.7"` (`default-features = false`, backend `ring`, sans `tls12`), déclarées
  derrière la feature `tls` de MCP Control.

## 9. Stockage des clés

| Secret | Où | Repli si le trousseau est indisponible |
|---|---|---|
| Clé privée Ed25519 du serveur | trousseau du système, service `mcp-control`, compte `server-ed25519` | fichier `auth/server.key`, permissions 0600 (ACL utilisateur seule sous Windows), ligne de journal à chaque démarrage |
| Clé privée TLS | trousseau, compte `server-tls` | `tls/server.key`, mêmes permissions |
| Clés privées des clients sortants (pairs, serveurs verrouillés) | trousseau, compte `client-<hôte>:<port>` | `auth/clients/<keyid>.key`, mêmes permissions |
| Clé du journal d'audit | trousseau, compte `audit-hmac` | fichier `auth/audit.key`, mêmes permissions, journalisé |

- `config.json` ne contient **que** des clés publiques et des métadonnées (§3.2) : aucun test du lot M1 ne doit trouver
  une clé privée, une graine ou un secret dans ce fichier.
- Le repli fichier est un **repli** : il est signalé dans le journal et dans l'écran « Accès », et il est traité comme
  moins protecteur (lisible par le même utilisateur, §1.5).
- Aucun export de clé privée par l'API ni par le gateway. `auth fingerprint` n'affiche que des empreintes.

## 10. Migration depuis `api_access` (DA3 = a) et compatibilité des pairs

- **0.8.x — tolérance en lecture seule** : `X-MCC-Access` reste accepté sur `/api/*` et `/mcp`, mais ne donne plus que
  `lecture` ; chaque usage écrit un avertissement dans le journal et allume un bandeau dans l'interface. `?access=`
  disparaît **immédiatement** (M2), `config.json` conserve la valeur (`src/manage/config.rs:690`) pour ne pas casser un
  pair, et aucune nouvelle valeur n'est générée.
- **Pairs** : un pair qui présente une preuve HMAC valide (`X-MCC-Nonce` + `X-MCC-Proof`, `src/lib.rs:373`) est traité
  comme un client `lecture` et journalisé `legacy_proof`. Le vrai correctif est le lot M4 : un pair devient un client
  appairé, avec sa propre clé.
- **0.9.0 — suppression** : `api_access` est ignoré (un avertissement au boot s'il est présent), `MCC_API_ACCESS`
  n'est plus lu, `X-MCC-Access`, `?access=`, `X-MCC-Proof` et `/api/hello` n'existent plus.
- **Ce qui casse, et c'est annoncé** : les scripts `curl` avec `?access=` ou `X-MCC-Access`, les process locaux qui
  appelaient `/api/*` ou `/mcp` sans identité, les pairs non ré-appairés, et les clients MCP qui ne savent pas signer
  (solution : le pont, DA6=a). Le guide de migration est le lot D.

## 11. Journal d'audit chaîné

- **Fichier** : `auth-audit.log`, à côté de `config.json`, distinct de `mcc.log`. Une ligne par décision d'accès,
  écrite avant que la réponse ne parte.
- **Format** (une ligne, champs séparés par une espace, valeurs sans espace) :
  `seq=000042 ts=2026-11-15T12:00:00Z keyid=mcc-ai-3f9a1c07b2d84e51 fp=ed25519:3f9a1c07b2d84e51 ip=192.0.2.7
  method=tools/call tool=start_server verdict=approval_pending reason=- prev=<64 hex> mac=<64 hex>`
- **Chaînage** : `mac = HMAC-SHA256(k_audit, prev ‖ ligne_sans_mac)`, `prev` = `mac` de la ligne précédente,
  `prev` de la première ligne = 32 zéros. `mcp-control auth verify-log` recalcule toute la chaîne et rend non nul sur
  une ligne modifiée, supprimée ou insérée (trois tests, lot M5).
- **Rotation** : à 8 Mio, cinq fichiers (`.1` à `.5`) ; lorsque le plus ancien segment `.5` est évincé, le `mac` de
  ancrage est enregistré dans `auth-audit.head` (signé HMAC, écrit atomiquement avec ACL restreintes) afin que la chaîne
  reste vérifiable de bout en bout par `mcp-control auth verify-log` même après éviction.
- **Écriture des secrets** : tous les fichiers privés (`auth/server.key`, `auth/audit.key`, `tls/server.key`,
  `auth/clients/<keyid>.key`, `auth-audit.head`) sont écrits sans fenêtre TOCTOU (création `.tmp` vide → ACL
  restreintes → écriture + `sync_all` → `rename`, nettoyage du `.tmp` sur erreur).
- **Ce qui n'y figure jamais** : arguments d'outil, valeurs d'environnement, nonce, signature, clé (publique ou privée),
  code `api_access`, résultat d'outil, contenu de réponse. Le motif précis reste local ; le réseau ne reçoit qu'un
  refus générique.

## Points non tranchés (pour l'utilisateur, à trancher avant le lot concerné)

1. **Gabarits de ressources de la lib.** Le `docs/auth.md` de la lib promet l'union des droits de tous les gabarits
   couvrants et une couverture conservatrice des motifs non simples ; le ré-audit LR a mesuré le **premier** gabarit
   dans l'ordre des clés et un fail-open sur `{?x}`/`{+x}`. Aucun rapport postérieur ne prouve le correctif.
   Options : (a) ne déclarer aucun gabarit côté MCP Control — le gateway n'expose aucune ressource aujourd'hui, l'écart
   reste sans effet — et le revérifier au lot M1 ; (b) exiger du chantier lib un rapport de correctif avant M1 ;
   (c) n'accepter que des URI exactes côté MCP Control. *Recommandé : (a) + (b), car un correctif non prouvé est une
   promesse, pas un fait.*
2. **Trousseau : `keyring` ou DPAPI direct.** `keyring` 4.2.0 (MIT OR Apache-2.0) couvre Windows/macOS/Linux d'un coup,
   mais ajoute des dépendances et sa taille n'est pas mesurée ; `windows-sys` + DPAPI direct n'ajoute presque rien mais
   est Windows-only et demanderait du code `unsafe`. Options : (a) `keyring` partout, repli fichier ; (b) DPAPI direct
   sous Windows + `keyring` ailleurs ; (c) repli fichier 0600 uniquement, assumé. *Recommandé : (a).*
3. **Périmètre de la confirmation pour `call_server_tool`.** DA4 dit « les outils qui lancent un programme » ; un outil
   distant peut exécuter n'importe quoi. Options : (a) confirmer chaque couple (serveur, outil) comme les autres
   `execution` — retenu ici, coûteux pour un agent ; (b) ne confirmer que les lancements locaux et laisser
   `call_server_tool` au seul droit `execution` ; (c) confirmation par couple (serveur, outil) valable 10 minutes, ce
   qui est le comportement (a) mais avec un libellé distinct dans l'écran. *Recommandé : (a), puis (c) si l'usage réel
   le réclame.*
4. **Signature des réponses du serveur.** Rien ne signe les réponses : l'intégrité repose sur le transport. Options :
   (a) s'en tenir à TLS + épinglage et l'écrire comme limite (§2.2) ; (b) ajouter une signature de réponse hors contrat
   C1–C12, donc hors périmètre de la lib livrée ; (c) attendre un lot lib dédié. *Recommandé : (a) pour 0.8.0.*
5. **Expiration des clés clients.** Aucune expiration par défaut, seulement un signalement à 180 jours. Options :
   (a) garder ce choix ; (b) expiration à 90 jours avec ré-appairage ; (c) expiration par droit (les clés `execution`
   expirent, les `lecture` non). *Recommandé : (a) pour 0.8.0, (c) ensuite.*
