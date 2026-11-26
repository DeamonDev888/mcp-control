// Boutique MCP — catalogue statique + parsing des noms (porté de l'app Hermes desktop : mcp-doc.ts, mcp-import.ts).
// (split v0.7.20 : manage.rs reste la racine — API stable via réexport)
use super::*;

// ---------- boutique MCP (catalogue + import, porté de l'app Hermes desktop :
// mcp-doc.ts pour le parsing des configs, mcp-import.ts pour « collez n'importe quoi ») ----------

pub struct StoreItem {
    pub id: &'static str,
    pub name: &'static str,
    pub desc: &'static str,
    pub cat: &'static str,
    pub kind: &'static str, // npm | python
    pub program: &'static str,
    pub args: &'static [&'static str],
    pub env_vars: &'static [&'static str],
}

pub const STORE: &[StoreItem] = &[
    StoreItem {
        id: "gdrive",
        name: "Google Drive",
        desc: "Recherche et lecture des fichiers Google Drive",
        cat: "Fichiers",
        kind: "npm",
        program: "npx",
        args: &["-y", "@modelcontextprotocol/server-gdrive"],
        env_vars: &["GDRIVE_OAUTH_PATH", "GDRIVE_CREDENTIALS_PATH"],
    },
    StoreItem {
        id: "git",
        name: "Git",
        desc: "Lecture et manipulation de dépôts git (status, diff, log, commit)",
        cat: "Dev",
        kind: "python",
        program: "uvx",
        args: &["mcp-server-git"],
        env_vars: &[],
    },
    StoreItem {
        id: "github",
        name: "GitHub",
        desc: "API GitHub : repos, issues, PR (GITHUB_TOKEN requis en env)",
        cat: "Dev",
        kind: "npm",
        program: "npx",
        args: &["-y", "@modelcontextprotocol/server-github"],
        env_vars: &["GITHUB_PERSONAL_ACCESS_TOKEN"],
    },
    StoreItem {
        id: "memory",
        name: "Memory",
        desc: "Graph de connaissances persistant (mémoire longue)",
        cat: "Utilitaires",
        kind: "npm",
        program: "npx",
        args: &["-y", "@modelcontextprotocol/server-memory"],
        env_vars: &[],
    },
    StoreItem {
        id: "seqthink",
        name: "Sequential Thinking",
        desc: "Raisonnement structuré étape par étape",
        cat: "Utilitaires",
        kind: "npm",
        program: "npx",
        args: &["-y", "@modelcontextprotocol/server-sequential-thinking"],
        env_vars: &[],
    },
    StoreItem {
        id: "everything",
        name: "Everything",
        desc: "Serveur de démo/test qui expose tous les comportements MCP",
        cat: "Utilitaires",
        kind: "npm",
        program: "npx",
        args: &["-y", "@modelcontextprotocol/server-everything"],
        env_vars: &[],
    },
    StoreItem {
        id: "time",
        name: "Time",
        desc: "Heure et conversions de fuseaux horaires",
        cat: "Utilitaires",
        kind: "python",
        program: "uvx",
        args: &["mcp-server-time"],
        env_vars: &[],
    },
    StoreItem {
        id: "fetch",
        name: "Fetch",
        desc: "Récupération de pages web et conversion en markdown",
        cat: "Web",
        kind: "python",
        program: "uvx",
        args: &["mcp-server-fetch"],
        env_vars: &[],
    },
    StoreItem {
        id: "puppeteer",
        name: "Puppeteer",
        desc: "Automatisation de navigateur : screenshots, clics, scraping",
        cat: "Web",
        kind: "npm",
        program: "npx",
        args: &["-y", "@modelcontextprotocol/server-puppeteer"],
        env_vars: &[],
    },
    StoreItem {
        id: "brave",
        name: "Brave Search",
        desc: "Recherche web Brave (BRAVE_API_KEY requis en env)",
        cat: "Web",
        kind: "npm",
        program: "npx",
        args: &["-y", "@modelcontextprotocol/server-brave-search"],
        env_vars: &["BRAVE_API_KEY"],
    },
    StoreItem {
        id: "postgres",
        name: "PostgreSQL",
        desc: "Accès lecture seule à une base PostgreSQL (URL en args)",
        cat: "Données",
        kind: "npm",
        program: "npx",
        args: &["-y", "@modelcontextprotocol/server-postgres"],
        env_vars: &[],
    },
    StoreItem {
        id: "sqlite",
        name: "SQLite",
        desc: "Requêtes et analyse d'une base SQLite locale",
        cat: "Données",
        kind: "python",
        program: "uvx",
        args: &["--with", "mcp>=1,<2", "mcp-server-sqlite", "--db-path", "test.db"],
        env_vars: &[],
    },
    StoreItem {
        id: "slack",
        name: "Slack",
        desc: "Lecture/écriture de canaux Slack (tokens requis en env)",
        cat: "Données",
        kind: "npm",
        program: "npx",
        args: &["-y", "@modelcontextprotocol/server-slack"],
        env_vars: &["SLACK_BOT_TOKEN", "SLACK_TEAM_ID"],
    },
    StoreItem {
        id: "playwright",
        name: "Playwright",
        desc: "Automatisation de navigateur moderne (Microsoft) : navigation, clics, screenshots, snapshots d'accessibilité",
        cat: "Web",
        kind: "npm",
        program: "npx",
        args: &["-y", "@playwright/mcp@latest"],
        env_vars: &[],
    },
    StoreItem {
        id: "context7",
        name: "Context7",
        desc: "Documentation à jour des libs et frameworks injectée dans le contexte de l'agent (Upstash)",
        cat: "Dev",
        kind: "npm",
        program: "npx",
        args: &["-y", "@upstash/context7-mcp"],
        env_vars: &[],
    },
    StoreItem {
        id: "duckduckgo",
        name: "DuckDuckGo Search",
        desc: "Recherche web DuckDuckGo sans clé API + extraction de contenu de pages",
        cat: "Web",
        kind: "python",
        program: "uvx",
        args: &["duckduckgo-mcp-server"],
        env_vars: &[],
    },
    StoreItem {
        id: "kubernetes",
        name: "Kubernetes",
        desc: "Gestion d'un cluster k8s via kubectl (kubeconfig local requis)",
        cat: "Dev",
        kind: "npm",
        program: "npx",
        args: &["-y", "mcp-server-kubernetes"],
        env_vars: &[],
    },
    StoreItem {
        id: "supabase",
        name: "Supabase",
        desc: "Bases, tables et edge functions Supabase (SUPABASE_ACCESS_TOKEN requis en env, ou OAuth navigateur)",
        cat: "Données",
        kind: "npm",
        program: "npx",
        args: &["-y", "@supabase/mcp-server-supabase@latest"],
        env_vars: &["SUPABASE_ACCESS_TOKEN"],
    },
    StoreItem {
        id: "mongodb",
        name: "MongoDB",
        desc: "Requêtes et analyse MongoDB (MDB_MCP_CONNECTION_STRING requis en env)",
        cat: "Données",
        kind: "npm",
        program: "npx",
        args: &["-y", "mongodb-mcp-server"],
        env_vars: &["MDB_MCP_CONNECTION_STRING"],
    },
    StoreItem {
        id: "stripe",
        name: "Stripe",
        desc: "Paiements et clients Stripe (STRIPE_SECRET_KEY en env, ou --api-key en args)",
        cat: "Données",
        kind: "npm",
        program: "npx",
        args: &["-y", "@stripe/mcp", "--tools=all"],
        env_vars: &["STRIPE_SECRET_KEY"],
    },
    StoreItem {
        id: "notion",
        name: "Notion",
        desc: "API Notion officielle (NOTION_TOKEN en env, ou OPENAPI_MCP_HEADERS)",
        cat: "Données",
        kind: "npm",
        program: "npx",
        args: &["-y", "@notionhq/notion-mcp-server"],
        env_vars: &["NOTION_TOKEN"],
    },
    StoreItem {
        id: "firecrawl",
        name: "Firecrawl",
        desc: "Scraping et extraction web (FIRECRAWL_API_KEY en env ; mode sans clé disponible côté Firecrawl)",
        cat: "Web",
        kind: "npm",
        program: "npx",
        args: &["-y", "firecrawl-mcp"],
        env_vars: &["FIRECRAWL_API_KEY"],
    },
    StoreItem {
        id: "exa",
        name: "Exa Search",
        desc: "Recherche web sémantique (EXA_API_KEY requis en env)",
        cat: "Web",
        kind: "npm",
        program: "npx",
        args: &["-y", "exa-mcp-server"],
        env_vars: &["EXA_API_KEY"],
    },
    StoreItem {
        id: "blender",
        name: "Blender",
        desc: "Pilote une session Blender vivante — addon BlenderMCP requis dans Blender (repo ahujasid/blender-mcp, pin du catalogue Hermes)",
        cat: "Création",
        kind: "python",
        program: "uvx",
        args: &["--with", "mcp>=1,<2", "blender-mcp==1.6.4"],
        env_vars: &[],
    },
    StoreItem {
        id: "figma",
        name: "Figma (Framelink)",
        desc: "Contexte design Figma pour générer du code (FIGMA_API_KEY requis en env) — l'officiel distant est OAuth/HTTPS, pas encore sondable ici",
        cat: "Design",
        kind: "npm",
        program: "npx",
        args: &["-y", "figma-developer-mcp", "--stdio"],
        env_vars: &["FIGMA_API_KEY"],
    },
    StoreItem {
        id: "n8n",
        name: "n8n",
        desc: "Docs des nodes n8n hors de la boîte ; gestion d'instance via N8N_API_URL + N8N_API_KEY en env",
        cat: "Automatisation",
        kind: "npm",
        program: "npx",
        args: &["-y", "n8n-mcp"],
        env_vars: &["N8N_API_URL", "N8N_API_KEY"],
    },
    StoreItem {
        id: "todoist",
        name: "Todoist",
        desc: "Tâches Todoist officiel (Doist) : projets, tâches, échéances (TODOIST_API_KEY requis en env)",
        cat: "Productivité",
        kind: "npm",
        program: "npx",
        args: &["-y", "-p", "@todoist/mcp", "todoist-mcp"],
        env_vars: &["TODOIST_API_KEY"],
    },
    StoreItem {
        id: "taskmaster",
        name: "Task Master",
        desc: "Décompose une spec en tâches, gère les dépendances et délègue aux agents (task-master-ai ; ANTHROPIC_API_KEY requis en env)",
        cat: "Productivité",
        kind: "npm",
        program: "npx",
        args: &["-y", "--package=task-master-ai", "task-master-ai"],
        env_vars: &["ANTHROPIC_API_KEY"],
    },
    StoreItem {
        id: "drawio",
        name: "draw.io",
        desc: "Serveur MCP officiel draw.io (jgraph/drawio-mcp) : génère et ouvre des diagrammes éditables (XML, Mermaid, CSV) directement dans draw.io",
        cat: "Design",
        kind: "npm",
        program: "npx",
        args: &["-y", "@drawio/mcp"],
        env_vars: &[],
    },
    // ---- lot 2026-11-24 : 11 serveurs vérifiés sur npm / PyPI + README officiels ----
    StoreItem {
        id: "atlassian",
        name: "Jira & Confluence",
        desc: "Tickets Jira et pages Confluence (Cloud ou Server/DC) : recherche JQL/CQL, lecture, création, transitions (mcp-atlassian ; JIRA_URL + JIRA_API_TOKEN requis en env, Confluence optionnel)",
        cat: "Productivité",
        kind: "python",
        program: "uvx",
        args: &["mcp-atlassian"],
        env_vars: &[
            "JIRA_URL",
            "JIRA_USERNAME",
            "JIRA_API_TOKEN",
            "CONFLUENCE_URL",
            "CONFLUENCE_USERNAME",
            "CONFLUENCE_API_TOKEN",
        ],
    },
    StoreItem {
        id: "obsidian",
        name: "Obsidian",
        desc: "Notes d'un coffre Obsidian via le plugin communautaire Local REST API : lister, chercher, lire, compléter (OBSIDIAN_API_KEY requis en env ; hôte/port réglables en env)",
        cat: "Productivité",
        kind: "python",
        program: "uvx",
        args: &["mcp-obsidian"],
        env_vars: &["OBSIDIAN_API_KEY", "OBSIDIAN_HOST"],
    },
    StoreItem {
        id: "desktop-commander",
        name: "Desktop Commander",
        desc: "Terminal, fichiers et processus : exécution de commandes, édition chirurgicale, recherche, surveillance système (contrôle étendu de la machine ; Node 22.12+)",
        cat: "Utilitaires",
        kind: "npm",
        program: "npx",
        args: &["-y", "@wonderwhy-er/desktop-commander"],
        env_vars: &[],
    },
    StoreItem {
        id: "windows-mcp",
        name: "Windows MCP",
        desc: "Pilote l'interface Windows 10/11 : applications, fenêtres, souris/clavier, captures d'écran et arbre UIA (CursorTouch ; Python 3.14 téléchargé par uv au premier lancement)",
        cat: "Utilitaires",
        kind: "python",
        program: "uvx",
        args: &["--python", "3.14", "windows-mcp", "serve"],
        env_vars: &[],
    },
    StoreItem {
        id: "youtube-transcript",
        name: "YouTube Transcript",
        desc: "Transcriptions et métadonnées de vidéos YouTube : texte, horodatages, langues disponibles (jkawamoto/mcp-youtube-transcript)",
        cat: "Web",
        kind: "python",
        program: "uvx",
        args: &["mcp-youtube-transcript"],
        env_vars: &[],
    },
    StoreItem {
        id: "tavily",
        name: "Tavily Search",
        desc: "Recherche web et extraction de pages conçues pour les agents IA (TAVILY_API_KEY requis en env)",
        cat: "Web",
        kind: "npm",
        program: "npx",
        args: &["-y", "tavily-mcp"],
        env_vars: &["TAVILY_API_KEY"],
    },
    StoreItem {
        id: "semgrep",
        name: "Semgrep",
        desc: "Analyse statique et scan de vulnérabilités du code (5 000+ règles), AST et règles personnalisées via le CLI officiel (SEMGREP_APP_TOKEN optionnel en env)",
        cat: "Dev",
        kind: "python",
        program: "uvx",
        args: &["--from", "semgrep", "semgrep", "mcp"],
        env_vars: &["SEMGREP_APP_TOKEN"],
    },
    StoreItem {
        id: "grafana",
        name: "Grafana",
        desc: "Dashboards, datasources, requêtes Prometheus/Loki, incidents et alertes de ton instance Grafana (officiel Grafana Labs ; GRAFANA_URL et jeton de compte de service en env)",
        cat: "Dev",
        kind: "python",
        program: "uvx",
        args: &["mcp-grafana"],
        env_vars: &["GRAFANA_URL", "GRAFANA_SERVICE_ACCOUNT_TOKEN"],
    },
    StoreItem {
        id: "clickhouse",
        name: "ClickHouse",
        desc: "Requêtes SQL en lecture seule, bases et tables ClickHouse, plus moteur chDB embarqué (officiel ClickHouse ; CLICKHOUSE_HOST/USER/PASSWORD requis en env)",
        cat: "Données",
        kind: "python",
        program: "uvx",
        args: &["mcp-clickhouse"],
        env_vars: &["CLICKHOUSE_HOST", "CLICKHOUSE_USER", "CLICKHOUSE_PASSWORD"],
    },
    StoreItem {
        id: "qdrant",
        name: "Qdrant",
        desc: "Mémoire sémantique sur une base vectorielle Qdrant : stockage et recherche de souvenirs avec embeddings FastEmbed (officiel Qdrant ; QDRANT_URL et COLLECTION_NAME requis en env)",
        cat: "Données",
        kind: "python",
        program: "uvx",
        args: &["mcp-server-qdrant"],
        env_vars: &["QDRANT_URL", "QDRANT_API_KEY", "COLLECTION_NAME"],
    },
    StoreItem {
        id: "airtable",
        name: "Airtable",
        desc: "Schémas, lecture et écriture des bases Airtable (AIRTABLE_API_KEY requis en env)",
        cat: "Données",
        kind: "npm",
        program: "npx",
        args: &["-y", "airtable-mcp-server"],
        env_vars: &["AIRTABLE_API_KEY"],
    },
];

/// Item de boutique par id — principal, puis aitmpl, puis glama, puis github.
pub(crate) fn find_store_item(sid: &str) -> Option<&'static StoreItem> {
    STORE
        .iter()
        .find(|s| s.id == sid)
        .or_else(|| {
            crate::store_aitmpl::STORE_AITMPL
                .iter()
                .find(|s| s.id == sid)
        })
        .or_else(|| crate::store_glama::STORE_GLAMA.iter().find(|s| s.id == sid))
        .or_else(|| {
            crate::store_github::STORE_GITHUB
                .iter()
                .find(|s| s.id == sid)
        })
}

/// Tous les items (principal + aitmpl + glama).
pub(super) fn all_store_items() -> impl Iterator<Item = &'static StoreItem> {
    STORE
        .iter()
        .chain(crate::store_aitmpl::STORE_AITMPL.iter())
        .chain(crate::store_glama::STORE_GLAMA.iter())
        .chain(crate::store_github::STORE_GITHUB.iter())
}

pub fn store_json() -> Value {
    // BT-O01 : catalogue statique compilé — construit une seule fois en mémoire.
    static CACHE: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            json!(all_store_items()
                .map(|s| json!({
                    "id": s.id, "name": s.name, "desc": s.desc,
                    "cat": s.cat, "kind": s.kind,
                    "program": s.program, "args": s.args,
                    "env_vars": s.env_vars,
                }))
                .collect::<Vec<_>>())
        })
        .clone()
}

/// Porté de mcp-import.ts : "@modelcontextprotocol/server-filesystem" → "filesystem".
pub(super) fn cleanup_name(base: &str) -> String {
    let mut b = base.to_string();
    if let Some(slash) = b.rfind('/') {
        b = b[slash + 1..].to_string();
    }
    b = regex_like_strip(&b, &["cjs", "js", "mjs", "py", "ts"]);
    for p in ["mcp-server-", "server-", "mcp-"] {
        if b.starts_with(p) {
            b = b[p.len()..].to_string();
        }
    }
    for s in ["-mcp-server", "-mcp", "-server"] {
        if b.ends_with(s) {
            b = b[..b.len() - s.len()].to_string();
        }
    }
    if b.is_empty() {
        base.to_string()
    } else {
        b
    }
}

pub(super) fn regex_like_strip(name: &str, exts: &[&str]) -> String {
    for e in exts {
        if let Some(stripped) = name.strip_suffix(&format!(".{}", e)) {
            return stripped.to_string();
        }
    }
    name.to_string()
}
