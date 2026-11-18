# Gateway MCP : référence

MCP Control expose un serveur MCP sur son propre port : `POST /mcp` (propulsé par [LiteMCP](https://github.com/DeamonDev888/litemcp), spec 2026-07-28). Cette page liste tout ce qu'un client y trouve. Pour la mise en route, voir le [README](../README.md#brancher-ton-assistant-ia).

```json
{ "mcpServers": { "mcp-control": { "url": "http://127.0.0.1:5090/mcp" } } }
```

## Les outils

| Groupe | Outils |
|---|---|
| **Superviser** (lecture) | `list_servers`, `server_detail`, `list_server_tools`, `journal`, `runtime_history`, `list_peers`, `store_catalog`, `list_frameworks`, `get_settings`, `check_update` |
| **Agir** (les boutons de l'interface) | `refresh`, `scan_ports`, `scan_network`, `discover_peers`, `add_server`, `remove_server`, `toggle_server`, `start_server`, `stop_server`, `set_start_command`, `set_host`, `toggle_tool`, `set_auto_restart`, `reorder_servers`, `import_config`, `install_store_server`, `ssh_save`, `ssh_forget`, `ssh_discover` |
| **Relayer** | `call_server_tool(server, tool, arguments)` : appelle un outil de n'importe quel serveur HTTP détecté UP, en respectant l'interrupteur par outil de l'interface |

Les schémas sont stricts : un paramètre inconnu est refusé (`-32602`) avant tout effet.

## Paramètres à connaître

| Outil | Paramètre |
|---|---|
| `list_servers` | `state` : `up`, `down`, `port_only` ou `disabled` |
| `journal` | `limit` (1 à 500, 40 par défaut) |
| `store_catalog` | `query` : recherche plein texte dans le catalogue |
| `scan_ports`, `scan_network` | `from`, `to` (sinon les plages réglées dans l'app) |
| `add_server` | `transport` (`http` ou `stdio`), puis `port` / `host` / `path`, ou `program` / `args` / `workdir` |
| `remove_server` | `durable: true` : empêche aussi un serveur de base d'être réinjecté au démarrage |
| `set_start_command` | `program: null` : retire la commande de lancement (l'autoboot du serveur est désactivé, le process en cours n'est pas tué) |
| `install_store_server` | `args`, et `env` (`{"GITHUB_TOKEN": "…"}`) pour les serveurs de la boutique qui demandent un jeton |
| `ssh_discover` | `ip` (sans `ip` : toutes les machines enregistrées) |
| `call_server_tool` | `server`, `tool`, `arguments` |

## Les ressources

- `mcp-control://servers` : aperçu de tous les serveurs
- `mcp-control://server/<id>` : une ressource par serveur détecté, mise à jour à chaque changement
- `mcp-control://journal`
- `mcp-control://settings`

## Installer un serveur de la boutique avec un jeton

L'outil `install_store_server` et l'action locale `install_store` acceptent `env` à l'installation. Les valeurs n'atteignent que le process lancé : ni `/api/state`, ni `/api/detail`, ni la réponse, ni le journal ne les renvoient (seuls les noms sont visibles).

```bash
curl -X POST -H "Content-Type: application/json" \
  -d '{"action":"install_store","id":"github","env":{"GITHUB_TOKEN":"<ton jeton>"}}' \
  http://127.0.0.1:5090/api/action
```

## Accès et limites

- Même port et même contrôle que l'API : en local, aucun code n'est demandé ; si l'app est ouverte au réseau, le **code d'accès** est exigé (en-tête `X-MCC-Access`, ou `?access=` pour ouvrir l'interface depuis une autre machine).
- `add_server` puis `start_server` lancent un programme au choix du client sur la machine : qui obtient le code d'accès obtient le droit d'exécuter du code. Il n'existe pas de réglage intermédiaire.
- Ne sont **pas** exposés, et restent des gestes de la fenêtre : `set_settings`, `set_autostart`, `set_notify`, `reset_state`, `reset_runtime`, `open_path`, `set_favorite`, `import_peer`, `test_notify`, `open_release_page`, `download_update`.
- `call_server_tool` ne couvre que les serveurs HTTP détectés UP. Les serveurs stdio, joints par SSH ou hors ligne sont refusés avec le motif exact.
- Transport : `Content-Type: application/json` exigé, notification → `202`, `GET` → `405`, réponse SSE à un seul événement si le client ne veut que ça, en-têtes `Mcp-Method` et `Mcp-Name` vérifiés contre le corps.
