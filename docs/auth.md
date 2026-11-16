# Authentification de MCP Control — spécification

Statut : lot A0, figé le 2026-11-15, branche `auth` (`bd650dd`, lot M0 en cours). Portée : MCP Control 0.8.0.
Décisions appliquées : **DA1 à DA7 = option a** (`TODO-AUTH.md` §3).

**Autorité.** La requête signée, les identités, les droits, la confirmation externe et TLS sont définis par le
`docs/auth.md` de `litemcp` 0.12.0, qui fait foi. Ce document ne les redéfinit pas : il fixe ce que MCP Control
choisit par-dessus, et rien de plus. Un lecteur qui n'a que ce fichier doit pouvoir écrire un client compatible ;
chaque « DOIT » est testable ; chaque choix cite sa décision.

Les numéros de ligne sont ceux du 2026-11-15. Le lot M0 (compatibilité 0.12.0) décale `src/gateway.rs` sans changer
les noms de fonctions ni d'outils.

## 1. Modèle de menace

### 1.1 Process local sans droits (script, autre application, service)

- **Aujourd'hui** : contrôle total sans aucun secret. `effective_require_access` rend faux dès que la connexion vient
  du loopback (`src/lib.rs:216-218`), donc le sas de `check_access` ne s'applique pas (`src/lib.rs:531`, `:579-597`) ;
  `/mcp` est servi (`src/lib.rs:1179`) et `POST /api/action` atteint `handle_action` (`src/lib.rs:1305`) ; le
  catalogue entier répond, `add_server` et `start_server` compris (`src/gateway.rs:1584`).
- **Après** : `/mcp` sans signature valide → 401 de corps constant (DA2) ; `/api/*` sans session locale → 401 (§6) ;
  les outils sont bornés par les droits du client (§4) et tout lancement passe par la confirmation (§5).

### 1.2 Page web ouverte dans un navigateur de la machine

- **Aujourd'hui** : ne peut ni lire ni écrire. `Host` inconnu → 403 (`src/lib.rs:544-552`), `Origin` étranger → 403
  (`:565-570`), `OPTIONS` → 403 (`:536`), `POST /api/action` non JSON → 415 (`:597`), `Access-Control-Allow-Origin`
  reflété seulement pour une origine autorisée (`:1204-1210`). Elle peut émettre des GET aveugles et mesurer des temps.
- **Après** : inchangé, plus le cookie de session `SameSite=Strict` (§6) qui n'est jamais envoyé en cross-site et
  l'absence de tout jeton dans l'URL. Le rebinding DNS reste bloqué par `Host`.

### 1.3 Machine du LAN qui écoute (passive)

- **Aujourd'hui** : capture le code partagé en clair — `X-MCC-Access` (`src/lib.rs:586`) ou `?access=` (`:588`) sur du
  HTTP sans TLS — puis l'utilise comme en 1.4. Aucun TLS dans le produit ; les sondes de pairs parlent en clair
  (`src/probe.rs:2505-2540`).
- **Après** : TLS 1.3 hors loopback (DA5, §7) et signature par requête (DA7). Une requête capturée n'est pas rejouable
  (nonce, fenêtre ±60 s, cache borné par clé — lib §2.3).

### 1.4 Machine du LAN active

- **Aujourd'hui** : `GET /api/hello` est pré-authentifié et non signé (`src/lib.rs:579`, `:1257`) ; qui présente le code
  obtient les 30 outils ; seuls freinent les plafonds de connexions (`src/lib.rs:27-41`) et 30 req/60 s sur
  `/api/hello` (`:300`).
- **Après** : sans clé, 401 de corps constant ; échecs comptés par adresse **et** par `keyid`, délai croissant puis
  blocage temporaire, et une adresse locale ne peut pas être bloquée depuis le LAN (lot M5).

### 1.5 Voleur de `config.json`

- **Aujourd'hui** : obtient `api_access` en clair (`src/manage/config.rs:690`) — donc l'exécution de programmes
  (audit 05 #2) — et les valeurs `env` des serveurs stdio (`:672-676`). Permissions : 0600 sous Unix, **rien sous
  Windows** (`:737-745`).
- **Après** : le fichier ne contient plus aucun secret (§9) : clés publiques, droits, dates. Le code hérité disparaît
  en 0.9 (DA3). Le fichier reste lisible par le même utilisateur : hors de portée.

### 1.6 Voleur d'une clé client

- **Aujourd'hui** : pas de clé client ; l'équivalent est le code partagé, qui vaut tous les droits (audit 05 #2).
- **Après** : il signe à la place de ce client et obtient exactement ses droits ; `execution` exige en plus une
  approbation dans la fenêtre locale (§5) ; la révocation prend effet à la requête suivante (lib §4) ; chaque décision
  est au journal (§11).

### 1.7 Serveur MCP supervisé malveillant

- **Aujourd'hui** : ses réponses reviennent telles quelles à l'agent (`call_server_tool`) ; il peut y glisser des
  instructions qui feront appeler `add_server`/`start_server`, autorisés sans identité (1.1). Il peut aussi se faire
  adopter comme commande de lancement par une annonce écrite dans `%TEMP%\litemcp-registry` (`src/manage/litemcp.rs:30`).
- **Après** : l'injection de contenu reste possible (hors de portée) mais son effet est borné par les droits de
  l'appelant et par la confirmation (§5) ; l'adoption d'annonce ne crée plus de commande exécutable sans `execution`
  et confirmation (§4).

### 1.8 Agent IA manipulé par injection de prompt

- **Aujourd'hui** : il pilote tout. Le scénario central : un prompt détourné appelle `add_server` puis `start_server`,
  et le programme tourne (audit 05 #2).
- **Après** : il ne peut que ce que ses droits permettent ; un lancement produit une demande visible dans la fenêtre,
  avec le programme et les arguments exacts (§5) ; la séquence reste au journal (§11).

### Hors de portée (écrit noir sur blanc)

Malware avec les droits de l'utilisateur (il lit le trousseau, pilote la fenêtre, remplace le binaire) ; binaire non
signé (SmartScreen/Gatekeeper restent le maillon suivant) ; OS ou dépendance compromis ; accès physique ; canaux
auxiliaires (temps, mémoire) ; déni de service au-delà des plafonds existants (64 connexions, 16 par adresse, 2 Mio —
`src/lib.rs:27-41`) ; confidentialité des arguments côté serveur ; terminaison TLS par un proxy ; mTLS ; un utilisateur
malveillant (même utilisateur = même autorité) ; signature des **réponses** du serveur (point non tranché n° 4).
Aucune évaluation par un tiers n'est revendiquée.

## 2. Requête signée

### 2.1 Ce que MCP Control ne redéfinit pas

Renvoi : `docs/auth.md` de `litemcp` 0.12.0, §2 en entier. Résumé en dix lignes :

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
| `@authority` attendue | La valeur du `Host` reçu, qui DOIT déjà figurer dans `allowed_hosts` (interfaces locales + `ui_port` + `allowed_origins`), sinon 403 avant vérification (`src/lib.rs:544-552`). `X-Forwarded-*` n'est jamais lu. | La liaison au destinataire (lib §2.6) porte sur ce que la machine voit réellement ; pas de proxy supposé. |
| Fenêtre d'horloge | ±60 s, non élargie. Un refus `clock_skew` est journalisé avec l'écart, jamais renvoyé au client. | Une fenêtre plus large rendrait le rejeu possible plus longtemps (RFC 9421 §7.2.2). |
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
5. **Client → serveur** : `GET /auth/pair/{pair_id}` toutes les 2 s (5 req/min).
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

## 6. Session locale de la fenêtre (DA2 = a)

- **Obtention** : au lancement, le moteur tire un jeton d'amorçage (32 octets du système, base64url), valable 5 minutes
  et **à usage unique**. Le shell Tauri navigue vers `http://127.0.0.1:<port>/#t=<jeton>` (fragment : jamais envoyé au
  serveur, jamais dans `Referer`, jamais dans `mcc.log`). `app.js` échange le jeton sur `POST /api/session`, reçoit le
  cookie, puis efface le fragment (`history.replaceState`).
- **Mode headless** (aucune fenêtre) : le moteur imprime la même URL à usage unique sur la console au démarrage ;
  l'utilisateur l'ouvre une fois dans son navigateur. Passé 5 minutes, il faut relancer le moteur — aucun jeton ne
  traîne sur disque.
- **Forme** : cookie `mcc_session`, `HttpOnly`, `SameSite=Strict`, `Path=/`, sans `Secure` en loopback (HTTP),
  `Max-Age` = durée de vie du moteur (cookie de session). Aucune valeur en `localStorage`, aucun en-tête `Authorization`
  côté UI.
- **Durée** : le cookie meurt avec le process ; une fenêtre déjà ouverte le garde jusqu'à l'arrêt du moteur. Aucun
  rafraîchissement silencieux.
- **Protection CSRF conservée** : `Host` et `Origin` restent vérifiés avant tout (`src/lib.rs:544-570`), `OPTIONS`
  reste 403, `POST /api/action` reste 415 sans JSON, le cookie est `SameSite=Strict`, et toute requête qui change
  l'état DOIT porter `X-MCC-Request: 1` (un en-tête personnalisé impose un préflight, que le serveur refuse).
- **Portée** : la session locale porte `admin` et n'est **jamais** acceptée sur `/mcp` (test du lot M2). Elle ne
  remplace pas la signature : un client MCP, même local, signe.

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
  refuse un serveur dont l'identité a changé et le dit.

## 8. API à ajouter, et correspondance C1–C12

### 8.1 Modules et routes

| Ajout | Contenu |
|---|---|
| `src/auth.rs` + `src/auth/{registry,pair,session,confirm}.rs` | Registre (`KeyStore`), appairage, session locale, politique de confirmation. |
| `src/auth/net.rs` | TLS entrant, épinglage sortant. |
| `src/bridge.rs` | Pont stdio (M5). |
| `src/audit.rs` | Journal chaîné (§11). |
| `POST /auth/pair`, `GET /auth/pair/{id}` | Famille d'appairage (§3.4), non authentifiée, limitée par adresse. |
| `POST /api/session` | Échange du jeton d'amorçage contre le cookie (§6). |
| `POST /mcp` | Vérifie puis dispatche (C8) ; plus aucun appel direct au noyau. |
| `config.json` | `auth.server` (`keyid`, `public_key`, `created_at`), `auth.clients[]` (§3.2), `auth.session_bootstrap_secs`, `auth.confirm_secs`, `auth.audit_path`, `tls.cert_path`, `tls.key_path` (chemins seulement). |
| Suppressions | `?access=` (M2) ; `X-MCC-Access` et la preuve `X-MCC-Proof` en 0.9 (DA3) ; `/api/hello` et ses nonces quand plus rien ne les utilise (M4). |

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

- `litemcp` : `auth-sign` **obligatoire** (sans lui, aucun vérificateur) ; `tls` pour DA5. Sans `tls`, le moteur compile
  et reste en loopback (§7).
- `keyring` **4.2.0**, licence MIT OR Apache-2.0, publiée le 2026-08-29, MSRV 1.88, magasins par défaut
  `windows-native-keyring-store`, `apple-native-keyring-store`, `zbus-secret-service-keyring-store`. Feature `keychain`,
  version épinglée (`=4.2.0`). **Taille ajoutée au binaire : non mesurée** (aucun build autorisé au lot A0) — mesure
  exigée au lot M1 avec la méthode du §7 de la lib. **Dernier audit public connu : non établi** (aucune référence
  consultée à ce jour) — à vérifier au lot M1.
- `rustls =0.23.46` et `rcgen =0.14.7`, mêmes versions que la lib (lib §7), déclarées directement parce que C10 ne les
  expose pas ; derrière la feature `tls` de MCP Control.

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
  `seq=000042 ts=2026-11-15T12:00:00Z keyid=mcc-ai-3f9a1c07b2d84e51 fp=ed25519:3f9a1c07b2d84e51 ip=192.168.40.7
  method=tools/call tool=start_server verdict=approval_pending reason=- prev=<64 hex> mac=<64 hex>`
- **Chaînage** : `mac = HMAC-SHA256(k_audit, prev ‖ ligne_sans_mac)`, `prev` = `mac` de la ligne précédente,
  `prev` de la première ligne = 32 zéros. `mcp-control auth verify-log` recalcule toute la chaîne et rend non nul sur
  une ligne modifiée, supprimée ou insérée (trois tests, lot M5).
- **Rotation** : à 8 Mio, cinq fichiers ; le `mac` de la dernière ligne passe dans un fichier `head` pour que la chaîne
  continue d'un fichier à l'autre.
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
