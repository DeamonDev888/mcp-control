# Guide de migration : `0.7.x` → `0.8.0`

La version **0.8.0** remplace le code d'accès partagé (`api_access`) par une authentification par **clés Ed25519 signées** (RFC 9421 / RFC 9530), une **session locale** pour la fenêtre, une **confirmation humaine** avant toute exécution, et **TLS 1.3 obligatoire** hors loopback.

La spécification technique complète est dans [docs/auth.md](auth.md).

---

## 1. Ce qui change (et ce qui casse)

| Changement | Comportement en `0.7.x` | Comportement en `0.8.0` | Action requise |
|---|---|---|---|
| **Requêtes locales sans identité sur `/mcp`** | Acceptées en loopback sans authentification | Refusées en `401` (`Signature` Ed25519 requise) | Appairer son assistant avec `mcp-control auth pair` puis utiliser `mcp-control bridge` |
| **Requêtes locales sans session sur `/api/*`** | Acceptées en loopback sans authentification | Refusées en `401` (cookie `mcc_session` + en-tête `X-MCC-Session` requis) | Utiliser l'interface (session ouverte au lancement) ou la CLI (`auth list`, `auth revoke`) |
| **Paramètre d'URL `?access=`** | Accepté | **Supprimé** (`401`) | Migrer vers une clé client Ed25519 |
| **En-tête `X-MCC-Access` (hérité)** | Donnait tous les droits (y compris exécution) | Restreint à **`lecture` seule** pendant `0.8.x` (avertissement au journal + bandeau UI) ; **supprimé en `0.9.0`** | Appairer les clients et scripts qui l'utilisaient |
| **En-tête `X-MCC-Request: 1`** | Exigé sur les requêtes mutantes de l'UI | **Supprimé**, remplacé par l'en-tête secret `X-MCC-Session` | Aucune côté UI (automatique) |
| **Pairs MCP Control sur le LAN** | Preuve HMAC (`X-MCC-Proof`) sur HTTP en clair | Identité Ed25519 signée (`lecture` sur `POST /mcp`) ; repli transitoire `0.8.x` conservé | Ré-appairer les pairs avant `0.9.0` |
| **Écoute hors loopback (`0.0.0.0` / LAN)** | HTTP en clair | **TLS 1.3 obligatoire** (certificat auto-signé persisté + épinglage `tls_fp`). Sans la feature `tls`, le bind LAN est refusé et le moteur reste en `127.0.0.1` | Utiliser un binaire compilé avec `--features tls` et passer les URL distantes en `https://` |
| **Annonces LiteMCP (`%TEMP%\litemcp-registry`)** | Adoptées automatiquement | **Non suivies par défaut** : listées en lecture seule dans « Détectés, non suivis » | Cliquer **Suivre** dans l'interface pour ajouter explicitement un serveur |
| **Arrêt d'un processus tiers sur un port** | `stop_server` tuait le processus occupant le port | Refusé avec nom/PID de l'occupant (`needs_force: true`) | Confirmer via `stop_server_force` si l'arrêt est voulu |
| **Installation neuve (`default_servers`)** | Serveurs d'exemple pré-remplis | Liste vide par défaut (les `seed-*` déjà dans `config.json` sont conservés) | Aucune pour une mise à jour existante |

---

## 2. Migrer ton assistant IA (Claude Desktop, Claude Code, Cursor)

En `0.7.x`, l'assistant pointait directement sur `http://127.0.0.1:5090/mcp`. En `0.8.0`, `/mcp` exige une signature Ed25519 par requête. Le pont stdio **`mcp-control bridge`** signe automatiquement les requêtes pour tout client MCP stdio.

### Étape 1 — Appairer ton poste

```bash
mcp-control auth pair http://127.0.0.1:5090 --label "Claude"
```

1. La commande affiche un **code de vérification à 6 chiffres** (ex. `482 193`).
2. Ouvre MCP Control → **Accès → Appairage**.
3. Vérifie que le code à 6 chiffres correspond, coche les droits souhaités (`lecture`, `pilotage`, `execution`) puis clique **Approuver**.
4. La clé privée est enregistrée dans le trousseau du système (ou en fichier `0600` / ACL utilisateur en repli) et l'identité du serveur est épinglée.

### Étape 2 — Remplacer la configuration MCP de l'assistant

Remplace l'ancienne entrée `"url": "http://127.0.0.1:5090/mcp"` par le pont stdio :

**Claude Desktop** (`claude_desktop_config.json`), **Claude Code** (`.mcp.json`) ou **Cursor** (`.cursor/mcp.json`) :

```json
{
  "mcpServers": {
    "mcp-control": {
      "command": "mcp-control",
      "args": ["bridge"]
    }
  }
}
```

Avec une URL explicite (port personnalisé ou machine distante en HTTPS) :

```json
{
  "mcpServers": {
    "mcp-control": {
      "command": "mcp-control",
      "args": ["bridge", "--url", "https://192.168.1.42:5090"]
    }
  }
}
```

### Étape 3 — Comprendre les 3 droits et la confirmation humaine

- **`lecture`** (10 outils) : lister les serveurs, voir détails, outils, journal, historique, catalogue, réglages.
- **`pilotage`** (10 outils) : rafraîchir, scanner, couper un outil, arrêter ou retirer un serveur.
- **`execution`** (10 outils) : ajouter/démarrer un serveur, modifier une commande de lancement, importer, installer depuis la boutique, `ssh_*`, `call_server_tool`.

Même avec le droit `execution`, **chaque appel d'outil `execution` demande une confirmation humaine** dans **Accès → Confirmations** (choix *Une fois*, *Permis 10 min* ou *Refuser*), en affichant le programme et les arguments exacts liés par SHA-256.

---

## 3. Migrer un accès depuis une autre machine (LAN)

1. Sur la machine serveur : dans **Réglages → Réseau**, passe l'écoute sur `0.0.0.0` (ou une IP LAN). Le moteur génère et persiste un certificat **TLS 1.3** auto-signé (l'empreinte SHA-256 est visible dans **Accès → Identité & Sessions**).
   - *Attention* : si le binaire a été compilé sans `--features tls`, l'écoute hors loopback est refusée et reste en `127.0.0.1`.
2. Sur la machine cliente :

```bash
mcp-control auth pair https://<ip-du-serveur>:5090 --label "Portable"
```

3. Compare le **code à 6 chiffres** (qui couvre à la fois la clé Ed25519 du serveur et l'empreinte `tls_fp` du certificat TLS 1.3), puis approuve dans la fenêtre du serveur.
4. Le client épingle la clé du serveur **et** le certificat TLS : toute substitution ultérieure est refusée sans contournement.

---

## 4. Commandes CLI d'administration (`0.8.0`)

Les commandes `auth list` et `auth revoke` s'authentifient automatiquement auprès du moteur local via le canal fichier utilisateur S1 (aucun secret imprimé).

| Commande | Rôle |
|---|---|
| `mcp-control auth pair <url> [--label <nom>]` | Crée une paire Ed25519, lance l'appairage, affiche le code à 6 chiffres et épingle le serveur après approbation |
| `mcp-control bridge [--url <url>]` | Pont stdio ↔ HTTP(S) signé pour les assistants IA |
| `mcp-control auth list` | Liste les clients enregistrés, leurs droits, leur statut et leur dernière activité |
| `mcp-control auth revoke <keyid>` | Révoque immédiatement un client (effet dès la requête suivante) |
| `mcp-control auth fingerprint` | Affiche l'empreinte Ed25519 du serveur local (`mcc-server`) |
| `mcp-control auth verify-log [--path <fichier>]` | Vérifie l'intégrité cryptographique de toute la chaîne HMAC du journal `auth-audit.log` (y compris après rotation) |

---

## 5. Serveurs LiteMCP verrouillés (`401`)

Si un serveur MCP distant exige une signature LiteMCP (`401` avec `WWW-Authenticate: Signature`), MCP Control l'affiche avec un **badge cadenas** (« joignable mais verrouillé »).

Dans le détail du serveur (ou depuis le Dashboard), clique **Appairer ce serveur** : MCP Control crée une clé cliente dédiée, l'enregistre dans le trousseau système (ou en repli fichier `0600`/ACL) et signe automatiquement les sondes et les appels relayés via `call_server_tool`.

---

## 6. Préparer la `0.9.0`

Pendant tout le cycle `0.8.x`, `X-MCC-Access` (limité à `lecture`) et `X-MCC-Proof` (pairs) restent acceptés avec avertissement pour faciliter la transition. Ils seront **définitivement supprimés en `0.9.0`** ainsi que `/api/hello` et `MCC_API_ACCESS`.
