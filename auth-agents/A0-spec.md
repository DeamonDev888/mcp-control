# A0 — Spécification et modèle de menace (MCP Control, branche `auth`)
Aucun fichier source, aucune dépendance, aucun build, aucune instance lancée. Livrables : `docs/auth.md` (399 lignes) et
ce rapport. Base lue : `TODO-AUTH.md`, `litemcp/docs/auth.md` et `auth-rapports/L0..LR`, `src/lib.rs`, `src/gateway.rs`,
`src/crypto.rs`, `src/manage/{peers,litemcp,config}.rs`, `src/probe.rs`, `src/ui/app.js`, les deux `05-securite.md`,
RFC 9421/9530/8032/9728/8707 et la page « Authorization » de MCP 2026-07-28.

## 1. Tableau

| Point | État | Preuve |
|---|---|---|
| §1 Modèle de menace : 8 attaquants, « aujourd'hui » avec fichier:ligne, « après », hors de portée écrit | FAIT | `docs/auth.md` §1 ; ex. `src/lib.rs:216-218` (loopback libre), `:582-597` (sas), `src/manage/config.rs:690` (code en clair), `:737-745` (ACL Windows absente) |
| §2 Requête signée : renvoi au doc de la lib + ce que MCP Control fixe (chemins, `@authority`, horloge, corps, délai, réponses non signées, pas d'OAuth) — écrire un client seul exige les deux documents | FAIT | §2 ; valeurs tirées de `src/lib.rs:27,41,544-552` |
| §3 Identités : `keyid` resserré, registre à 10 champs, cycle de vie, appairage en 5 messages, formule du code | FAIT | §3 ; grammaire et empreinte conformes à `litemcp/docs/auth.md` §3 |
| §4 Table définitive des 30 outils | FAIT | §4 : `lecture` 10, `pilotage` 10, `execution` 10 ; deux déplacements justifiés (`actions.rs:69-121`, `gateway.rs:758-767`) |
| §5 Confirmation DA4 : périmètre, aperçu, 10 min, réponse `approval_pending` | FAIT | §5 ; code `-32023` et `data.reason` conformes à la lib §5 |
| §6 Session locale DA2 : jeton d'amorçage, cookie, headless, CSRF | FAIT | §6 ; contrôles existants cités (`src/lib.rs:536-570,597`) |
| §7 TLS DA5 : TLS 1.3 seul, suites, certificat persistant, épinglage | PARTIEL | §7 ; la liaison certificat ↔ identité Ed25519 n'est **pas** cryptographique (voir §3 des écarts) |
| §8 API, routes, champs, C1–C12, features, crates | FAIT | §8 ; C10 déclaré inutilisable (`litemcp/src/tls.rs:233` est `pub(crate)`) |
| §9 Stockage : trousseau, repli fichier, contenu de `config.json` | PARTIEL | §9 ; taille binaire de `keyring` et dernier audit public **non établis** (build interdit) |
| §10 Migration `api_access` DA3 et pairs | FAIT | §10 ; `X-MCC-Proof` et `/api/hello` cités à leur place (`src/lib.rs:373`, `:1257`) |
| §11 Journal d'audit chaîné : ligne, HMAC, rotation, exclusions | FAIT | §11 |
| Aucun nom d'API inventé | FAIT | tous relus dans `litemcp/src/{auth,tls,server,gateway}.rs` : `dispatch_verified`, `McpHeaders::from_headers`, `SignatureVerifier::new`, `PinnedServer`, `CertificateFingerprint`, `auth::current` |
| Vocabulaire interdit absent | FAIT | recherche des trois termes du chantier : 0 occurrence (seul « certificat » y figure) |

## 2. Points non tranchés (options dans `docs/auth.md`, fin de document)

1. **Gabarits de ressources de la lib** : la doc promet l'union des droits et une couverture conservatrice ; le ré-audit
   LR a mesuré le premier gabarit et un fail-open sur `{?x}`/`{+x}`, sans rapport de correctif postérieur. Options :
   (a) n'en déclarer aucun côté MCP Control + revérifier au M1 ; (b) exiger un correctif prouvé avant M1 ; (c) URI
   exactes seulement.
2. **Trousseau** : `keyring` 4.2.0 partout / DPAPI direct sous Windows + `keyring` ailleurs / repli fichier assumé.
3. **Périmètre de la confirmation pour `call_server_tool`** : tout confirmer (retenu) / le laisser au seul droit
   `execution` / confirmation par couple (serveur, outil).
4. **Signature des réponses du serveur** : aucune aujourd'hui ; s'en tenir à TLS + épinglage / lot lib dédié.
5. **Expiration des clés clients** : aucune (retenu) / 90 jours / par droit.

## 3. Écarts assumés par rapport à `TODO-AUTH.md`

1. `toggle_server` et `set_auto_restart` passent de `pilotage` à `execution` : le premier démarre le process stdio
   (`src/manage/actions.rs:69-121`), le second arme un lancement au boot. Sans eux, l'invariant « aucun chemin vers
   l'exécution sans `execution` » est faux.
2. La confirmation couvre **les dix** outils `execution`, alors que DA4 parle des « outils qui lancent un programme » :
   `import_config` ou `install_store_server` n'exécutent rien dans l'appel mais préparent une exécution. Interprétation
   signalée, pas silencieuse.
3. **C10 inutilisable** : `TlsConfig::server_config` est `pub(crate)`. MCP Control construira son `ServerConfig` avec
   `rustls` + `rcgen` épinglés : le lot M4 devra déclarer ces deux crates, ce que `TODO-AUTH.md` n'avait pas prévu.
4. Le certificat n'est **pas** signé par la clé Ed25519 du serveur (« lié à son identité » au lot M4) : le lien est
   fait à l'appairage (deux empreintes épinglées, code de comparaison qui les couvre).
5. `GET /auth/server` supprimé du protocole : la clé publique du serveur voyage dans la réponse de `POST /auth/pair`,
   pour rester dans les routes autorisées au lot M1. `POST /api/session` est ajouté (lot M2).
6. Aucun document RFC 9728 n'est servi : C11 est sans objet puisque MCP Control n'utilise pas le transport de la lib.

## 4. Découvert en chemin et NON corrigé

- `%TEMP%\litemcp-registry` : une annonce locale (`src/manage/litemcp.rs:30`) devient une commande de lancement. Aucun
  lot ne la traite ; seule la confirmation la rend inoffensive. À vérifier au lot R (attaque n° 3).
- `config.json` n'a toujours aucune ACL sous Windows (`src/manage/config.rs:737-745`) alors que le lot M1 doit y écrire
  des clés publiques : la correction de l'audit 05 #10 est à moitié faite et doit être finie au M1.
- Le littéral `McpHeaders` de `src/gateway.rs` est en cours de modification par le lot M0 : les numéros de ligne de ce
  fichier cités ici et dans `docs/auth.md` peuvent glisser, les noms non.
