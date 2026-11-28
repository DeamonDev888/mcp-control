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
    pub kind: &'static str, // npm | python | rust
    pub program: &'static str,
    pub args: &'static [&'static str],
    pub env_vars: &'static [&'static str],
    pub version: &'static str,
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
        version: "2025.1.14",
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
        version: "2026.11.27",
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
        version: "2025.4.8",
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
        version: "2.2.2",
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
        version: "2.2.0",
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
        version: "2.4.0",
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
        version: "2026.11.27",
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
        version: "2026.11.27",
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
        version: "2025.5.12",
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
        version: "0.6.2",
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
        version: "0.6.2",
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
        version: "2025.4.25",
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
        version: "2025.4.25",
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
        version: "0.0.86",
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
        version: "4.9.1",
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
        version: "0.7.0",
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
        version: "4.2.3",
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
        version: "0.21.0",
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
        version: "3.4.0",
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
        version: "0.3.4",
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
        version: "2.6.1",
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
        version: "3.42.0",
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
        version: "3.5.1",
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
        version: "1.6.4",
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
        version: "0.13.2",
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
        version: "2.104.2",
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
        version: "19.1.1",
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
        version: "0.43.1",
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
        version: "1.13.1",
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
        version: "0.24.8",
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
        version: "0.2.3",
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
        version: "0.3.4",
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
        version: "0.9.0",
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
        version: "0.3.5",
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
        version: "0.2.22",
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
        version: "1.187.0",
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
        version: "2.2.0",
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
        version: "0.7.0",
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
        version: "0.8.1",
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
        version: "1.14.0",
    },
    StoreItem {
        id: "swarm-control",
        name: "Swarm Control",
        desc: "Orchestrateur multi-agents MCP natif en Rust : pilote Claude Code, Hermes et Antigravity, agents résidents et sessions persistantes (DeamonDev888/swarm-control)",
        cat: "Automatisation",
        kind: "rust",
        program: "swarm-control",
        args: &["--port", "23080"],
        env_vars: &[],
        version: "0.1.2",
    },
];

/// Spécification d'un serveur MCP distribué en binaire natif via GitHub Releases.
pub struct GitHubReleaseSpec {
    pub id: &'static str,
    pub repo: &'static str,
    pub bin_name: &'static str,
    pub default_port: u16,
    pub port_end: u16,
    pub mcp_path: &'static str,
}

pub const STORE_GITHUB_RELEASES: &[GitHubReleaseSpec] = &[GitHubReleaseSpec {
    id: "swarm-control",
    repo: "DeamonDev888/swarm-control",
    bin_name: "swarm-control",
    default_port: 23080,
    port_end: 23099,
    mcp_path: "/mcp",
}];

pub(crate) fn find_github_release(sid: &str) -> Option<&'static GitHubReleaseSpec> {
    STORE_GITHUB_RELEASES.iter().find(|r| r.id == sid)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub download_url: String,
    pub sha256: Option<String>,
}

/// Nom du fichier binaire installé localement pour l'OS courant.
pub(crate) fn release_local_bin_name(spec: &GitHubReleaseSpec, os: &str) -> String {
    if os == "windows" {
        format!("{}.exe", spec.bin_name)
    } else {
        spec.bin_name.to_string()
    }
}

/// Sélectionne l'actif binaire direct correspondant à `(os, arch)` dans une release GitHub.
/// Ignore les archives (`.zip`, `.tar.gz`) et fichiers de sommes/signatures (`.sha256`, `.sig`).
pub(crate) fn select_release_asset(
    spec: &GitHubReleaseSpec,
    os: &str,
    arch: &str,
    assets: &[ReleaseAsset],
) -> Option<ReleaseAsset> {
    let os_norm = os.trim().to_ascii_lowercase();
    let arch_norm = arch.trim().to_ascii_lowercase();
    let b = spec.bin_name;
    let candidates: Vec<String> = match (os_norm.as_str(), arch_norm.as_str()) {
        ("windows", "x86_64" | "amd64") => vec![
            format!("{b}-windows-x86_64.exe"),
            format!("{b}-windows-amd64.exe"),
        ],
        ("windows", "aarch64" | "arm64") => vec![
            format!("{b}-windows-aarch64.exe"),
            format!("{b}-windows-arm64.exe"),
            format!("{b}-windows-x86_64.exe"),
        ],
        ("linux", "x86_64" | "amd64") => {
            vec![format!("{b}-linux-x86_64"), format!("{b}-linux-amd64")]
        }
        ("linux", "aarch64" | "arm64") => {
            vec![format!("{b}-linux-aarch64"), format!("{b}-linux-arm64")]
        }
        ("macos" | "darwin", "aarch64" | "arm64") => vec![
            format!("{b}-macos-aarch64"),
            format!("{b}-macos-arm64"),
            format!("{b}-macos-universal"),
        ],
        ("macos" | "darwin", "x86_64" | "amd64") => {
            vec![format!("{b}-macos-x86_64"), format!("{b}-macos-universal")]
        }
        _ => return None,
    };
    let expected_prefix = format!("https://github.com/{}/releases/download/", spec.repo);
    let is_direct_binary = |a: &ReleaseAsset| -> bool {
        let low = a.name.to_ascii_lowercase();
        !low.ends_with(".sha256")
            && !low.ends_with(".sig")
            && !low.ends_with(".asc")
            && !low.ends_with(".txt")
            && !low.ends_with(".zip")
            && !low.ends_with(".tar.gz")
            && !low.ends_with(".tgz")
            && a.download_url.starts_with(&expected_prefix)
    };
    for want in &candidates {
        if let Some(a) = assets
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(want) && is_direct_binary(a))
        {
            return Some(a.clone());
        }
    }
    None
}

/// Parse un objet JSON de l'API GitHub `/releases/latest` en `(tag_name, assets)`.
pub(crate) fn parse_github_release_json(v: &Value) -> Option<(String, Vec<ReleaseAsset>)> {
    let tag = v
        .get("tag_name")
        .and_then(|x| x.as_str())?
        .trim()
        .to_string();
    if tag.is_empty() || tag.len() > 64 {
        return None;
    }
    let assets = v
        .get("assets")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let name = item.get("name")?.as_str()?.trim().to_string();
                    let download_url = item
                        .get("browser_download_url")?
                        .as_str()?
                        .trim()
                        .to_string();
                    let sha256 = item
                        .get("digest")
                        .and_then(|d| d.as_str())
                        .and_then(|d| d.strip_prefix("sha256:"))
                        .map(|h| h.trim().to_ascii_lowercase())
                        .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()));
                    Some(ReleaseAsset {
                        name,
                        download_url,
                        sha256,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some((tag, assets))
}

/// Interroge `https://api.github.com/repos/<repo>/releases/latest` (curl puis repli PowerShell).
pub(crate) fn fetch_github_latest_release(
    repo: &str,
) -> Result<(String, Vec<ReleaseAsset>), String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let mut c = crate::probe::system_command("curl");
    probe::probe_hidden_cmd(&mut c);
    if let Ok(out) = c
        .args([
            "-fsSL",
            "--proto",
            "=https",
            "--tlsv1.2",
            "-m",
            "10",
            "-H",
            "User-Agent: mcp-control",
            "-H",
            "Accept: application/vnd.github+json",
            &url,
        ])
        .output()
    {
        if out.status.success() {
            if let Ok(v) = serde_json::from_slice::<Value>(&out.stdout) {
                if let Some(parsed) = parse_github_release_json(&v) {
                    return Ok(parsed);
                }
            }
        }
    }
    let mut cmd = crate::probe::system_command("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &format!(
            "(Invoke-WebRequest -UseBasicParsing -TimeoutSec 10 -Headers @{{ 'User-Agent' = 'mcp-control'; 'Accept' = 'application/vnd.github+json' }} '{url}').Content"
        ),
    ]);
    probe::probe_hidden_cmd(&mut cmd);
    let out = cmd
        .output()
        .map_err(|e| format!("API GitHub injoignable pour {repo} : {e}"))?;
    if !out.status.success() {
        return Err(format!("API GitHub injoignable pour {repo}"));
    }
    let v: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("réponse GitHub invalide pour {repo} : {e}"))?;
    parse_github_release_json(&v)
        .ok_or_else(|| format!("aucune release publiée trouvée pour {repo}"))
}

/// Télécharge le binaire de la dernière release GitHub pour l'OS courant dans `dest_dir`,
/// vérifie son empreinte SHA-256 lorsqu'elle est publiée par GitHub, puis retourne
/// `(chemin_binaire, version_sans_v)`.
#[cfg_attr(test, allow(dead_code))]
pub(crate) fn download_store_release_binary(
    spec: &GitHubReleaseSpec,
    dest_dir: &std::path::Path,
) -> Result<(std::path::PathBuf, String), String> {
    const MAX_BYTES: u64 = 150 * 1024 * 1024;
    let (tag, assets) = fetch_github_latest_release(spec.repo)?;
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let asset = select_release_asset(spec, os, arch, &assets).ok_or_else(|| {
        format!(
            "aucun binaire publié pour {os}/{arch} dans {} ({tag})",
            spec.repo
        )
    })?;
    std::fs::create_dir_all(dest_dir)
        .map_err(|e| format!("impossible de créer le dossier d'installation : {e}"))?;
    let bin_name = release_local_bin_name(spec, os);
    let final_path = dest_dir.join(&bin_name);
    let tmp_path = dest_dir.join(format!("{bin_name}.tmp"));
    let _ = std::fs::remove_file(&tmp_path);

    let mut downloaded = false;
    let mut c = crate::probe::system_command("curl");
    probe::probe_hidden_cmd(&mut c);
    if let Ok(out) = c
        .args([
            "-fsSL",
            "--proto",
            "=https",
            "--tlsv1.2",
            "--max-time",
            "180",
            "--max-filesize",
            &MAX_BYTES.to_string(),
            "-H",
            "User-Agent: mcp-control",
            "-o",
        ])
        .arg(&tmp_path)
        .arg(&asset.download_url)
        .output()
    {
        if out.status.success() {
            downloaded = true;
        }
    }
    if !downloaded {
        let mut ps = crate::probe::system_command("powershell");
        let tmp_str = tmp_path.to_string_lossy().replace('\'', "''");
        let url_str = asset.download_url.replace('\'', "''");
        ps.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!(
                "Invoke-WebRequest -UseBasicParsing -TimeoutSec 180 -Headers @{{ 'User-Agent' = 'mcp-control' }} -Uri '{url_str}' -OutFile '{tmp_str}'"
            ),
        ]);
        probe::probe_hidden_cmd(&mut ps);
        let out = ps
            .output()
            .map_err(|e| format!("téléchargement impossible ({}) : {e}", asset.name))?;
        if !out.status.success() {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(format!("téléchargement échoué pour {} ({tag})", asset.name));
        }
    }

    if let Some(ref expected_sha) = asset.sha256 {
        crate::manage::update::verify_file_sha256(&tmp_path, expected_sha)?;
    } else {
        let meta = std::fs::metadata(&tmp_path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp_path);
            format!("binaire téléchargé illisible : {e}")
        })?;
        if meta.len() == 0 || meta.len() > MAX_BYTES {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(format!(
                "taille du binaire hors bornes ({} octets)",
                meta.len()
            ));
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("chmod +x impossible : {e}"))?;
    }
    if final_path.exists() {
        let _ = std::fs::remove_file(&final_path);
    }
    std::fs::rename(&tmp_path, &final_path)
        .or_else(|_| {
            std::fs::copy(&tmp_path, &final_path).map(|_| {
                let _ = std::fs::remove_file(&tmp_path);
            })
        })
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp_path);
            format!("installation du binaire impossible : {e}")
        })?;

    let version = tag.strip_prefix('v').unwrap_or(&tag).to_string();
    cache_store_version(spec.id, version.clone());
    Ok((final_path, version))
}

/// Extrait une version épinglée statiquement dans `args` (ex. `blender-mcp==1.6.4` → `"1.6.4"`,
/// `pkg@1.2.3` → `"1.2.3"`). Ignore `@latest`, les scopes `@org/pkg` et les plages `>=1,<2`.
pub(crate) fn extract_pinned_version(args: &[&str]) -> Option<String> {
    for arg in args {
        let a = arg.trim();
        if let Some((_, ver)) = a.split_once("==") {
            let v = ver.trim().trim_start_matches('v');
            if !v.is_empty() && v.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                return Some(v.to_string());
            }
        }
        // Format npm `pkg@1.2.3` ou `@scope/pkg@1.2.3` : on cherche le `@` après le dernier `/`.
        let after_slash = a.rfind('/').map(|i| i + 1).unwrap_or(0);
        if let Some(rel_at) = a[after_slash..].find('@') {
            let at = after_slash + rel_at;
            if at > 0 {
                let v = a[at + 1..].trim().trim_start_matches('v');
                if !v.is_empty()
                    && !v.eq_ignore_ascii_case("latest")
                    && v.chars().next().is_some_and(|c| c.is_ascii_digit())
                {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

type VersionCache =
    std::sync::Mutex<std::collections::HashMap<String, (String, std::time::Instant)>>;

fn version_cache() -> &'static VersionCache {
    static CACHE: std::sync::OnceLock<VersionCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

pub(crate) fn cache_store_version(id: &str, version: String) {
    if let Ok(mut map) = version_cache().lock() {
        map.insert(id.to_string(), (version, std::time::Instant::now()));
    }
}

pub(crate) fn cached_store_version(id: &str) -> Option<String> {
    const TTL: std::time::Duration = std::time::Duration::from_secs(30 * 60);
    version_cache()
        .lock()
        .ok()
        .and_then(|m| m.get(id).cloned())
        .filter(|(_, at)| at.elapsed() < TTL)
        .map(|(v, _)| v)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StorePackageSource {
    GitHubRelease(&'static str),
    GitHubRepo(String),
    Npm(String),
    PyPi(String),
}

/// Détermine le registre et le nom du paquet/dépôt d'un item de la boutique
/// afin de vérifier sa dernière version publiée (npm, PyPI ou GitHub).
pub(crate) fn extract_store_package(it: &StoreItem) -> Option<StorePackageSource> {
    if let Some(spec) = find_github_release(it.id) {
        return Some(StorePackageSource::GitHubRelease(spec.repo));
    }
    for arg in it.args {
        let a = arg.trim();
        if let Some(rest) = a.strip_prefix("git+https://github.com/") {
            let repo = rest
                .trim_matches('/')
                .strip_suffix(".git")
                .unwrap_or(rest.trim_matches('/'));
            let repo = repo.split('@').next().unwrap_or(repo).trim();
            if repo.contains('/') {
                return Some(StorePackageSource::GitHubRepo(repo.to_string()));
            }
        }
        if let Some(rest) = a.strip_prefix("github:") {
            let repo = rest.trim_matches('/');
            let repo = repo.split('#').next().unwrap_or(repo).trim();
            if repo.contains('/') {
                return Some(StorePackageSource::GitHubRepo(repo.to_string()));
            }
        }
    }
    match it.kind {
        "npm" => {
            let mut i = 0;
            let mut pkg: Option<&str> = None;
            while i < it.args.len() {
                let a = it.args[i].trim();
                if let Some(v) = a.strip_prefix("--package=") {
                    pkg = Some(v);
                    break;
                }
                if (a == "-p" || a == "--package") && i + 1 < it.args.len() {
                    pkg = Some(it.args[i + 1].trim());
                    break;
                }
                if a.starts_with('-') {
                    i += 1;
                    continue;
                }
                pkg = Some(a);
                break;
            }
            let raw = pkg?;
            let after_slash = raw.rfind('/').map(|x| x + 1).unwrap_or(0);
            let name = match raw[after_slash..].find('@') {
                Some(rel) if after_slash + rel > 0 => &raw[..after_slash + rel],
                _ => raw,
            };
            (!name.is_empty()).then(|| StorePackageSource::Npm(name.to_string()))
        }
        "python" => {
            let mut i = 0;
            let mut pkg: Option<&str> = None;
            while i < it.args.len() {
                let a = it.args[i].trim();
                if a == "--from" && i + 1 < it.args.len() {
                    pkg = Some(it.args[i + 1].trim());
                    break;
                }
                if matches!(a, "--python" | "--with" | "-p") {
                    i += 2;
                    continue;
                }
                if a.starts_with('-') {
                    i += 1;
                    continue;
                }
                pkg = Some(a);
                break;
            }
            let raw = pkg?;
            let name = raw
                .split("==")
                .next()
                .unwrap_or(raw)
                .split('@')
                .next()
                .unwrap_or(raw)
                .trim();
            (!name.is_empty()).then(|| StorePackageSource::PyPi(name.to_string()))
        }
        _ => None,
    }
}

#[cfg(not(test))]
fn http_get_text(url: &str, accept: Option<&str>, timeout_secs: u64) -> Option<String> {
    let mut c = crate::probe::system_command("curl");
    probe::probe_hidden_cmd(&mut c);
    let t_str = timeout_secs.to_string();
    let mut args = vec![
        "-fsSL",
        "--proto",
        "=https",
        "--tlsv1.2",
        "-m",
        &t_str,
        "-H",
        "User-Agent: mcp-control",
    ];
    if let Some(acc) = accept {
        args.push("-H");
        args.push(acc);
    }
    args.push(url);
    if let Ok(out) = c.args(&args).output() {
        if out.status.success() {
            if let Ok(s) = String::from_utf8(out.stdout) {
                return Some(s);
            }
        }
    }
    None
}

#[cfg(not(test))]
fn encode_pkg_segment(pkg: &str) -> String {
    pkg.replace('@', "%40").replace('/', "%2F")
}

#[cfg(not(test))]
fn fetch_live_store_version(src: &StorePackageSource) -> Option<String> {
    match src {
        StorePackageSource::GitHubRelease(repo) => fetch_github_latest_release(repo)
            .ok()
            .map(|(tag, _)| tag.strip_prefix('v').unwrap_or(&tag).to_string()),
        StorePackageSource::Npm(pkg) => {
            let url = format!(
                "https://registry.npmjs.org/{}/latest",
                encode_pkg_segment(pkg)
            );
            let body = http_get_text(&url, None, 6)?;
            let v: Value = serde_json::from_str(&body).ok()?;
            v.get("version")
                .and_then(|x| x.as_str())
                .map(|s| s.trim().trim_start_matches('v').to_string())
                .filter(|s| !s.is_empty())
        }
        StorePackageSource::PyPi(pkg) => {
            let url = format!("https://pypi.org/pypi/{}/json", encode_pkg_segment(pkg));
            let body = http_get_text(&url, None, 6)?;
            let v: Value = serde_json::from_str(&body).ok()?;
            v.get("info")
                .and_then(|i| i.get("version"))
                .and_then(|x| x.as_str())
                .map(|s| s.trim().trim_start_matches('v').to_string())
                .filter(|s| !s.is_empty())
        }
        StorePackageSource::GitHubRepo(repo) => {
            if let Ok((tag, _)) = fetch_github_latest_release(repo) {
                let ver = tag.strip_prefix('v').unwrap_or(&tag).trim().to_string();
                if !ver.is_empty() {
                    return Some(ver);
                }
            }
            let pkg_url = format!("https://raw.githubusercontent.com/{repo}/HEAD/package.json");
            if let Some(body) = http_get_text(&pkg_url, None, 6) {
                if let Ok(v) = serde_json::from_str::<Value>(&body) {
                    if let Some(ver) = v.get("version").and_then(|x| x.as_str()) {
                        let ver = ver.trim().trim_start_matches('v').to_string();
                        if !ver.is_empty() {
                            return Some(ver);
                        }
                    }
                }
            }
            let py_url = format!("https://raw.githubusercontent.com/{repo}/HEAD/pyproject.toml");
            if let Some(body) = http_get_text(&py_url, None, 6) {
                for line in body.lines() {
                    let t = line.trim();
                    if let Some(rest) = t.strip_prefix("version") {
                        let rest = rest.trim_start();
                        if let Some(rhs) = rest.strip_prefix('=') {
                            let ver = rhs
                                .trim()
                                .trim_matches('"')
                                .trim_matches('\'')
                                .trim_start_matches('v')
                                .to_string();
                            if !ver.is_empty()
                                && ver.chars().next().is_some_and(|c| c.is_ascii_digit())
                            {
                                return Some(ver);
                            }
                        }
                    }
                }
            }
            None
        }
    }
}

/// Renvoie un objet JSON `{ "<store_id>": "<version>" }` pour tous les serveurs de la boutique
/// (version du catalogue + cache en ligne 30 min + releases GitHub).
pub fn store_versions_json() -> Value {
    const TTL: std::time::Duration = std::time::Duration::from_secs(30 * 60);
    let mut out = serde_json::Map::new();
    for it in all_store_items() {
        if let Some(v) = extract_pinned_version(it.args) {
            out.insert(it.id.to_string(), json!(v));
        } else if !it.version.is_empty() {
            out.insert(it.id.to_string(), json!(it.version));
        }
    }
    if let Ok(map) = version_cache().lock() {
        for (id, (ver, at)) in map.iter() {
            if at.elapsed() < TTL {
                out.insert(id.clone(), json!(ver));
            }
        }
    }
    for spec in STORE_GITHUB_RELEASES {
        let fresh = version_cache()
            .lock()
            .ok()
            .and_then(|m| m.get(spec.id).map(|(_, at)| at.elapsed() < TTL))
            .unwrap_or(false);
        if fresh {
            continue;
        }
        #[cfg(not(test))]
        if let Ok((tag, _)) = fetch_github_latest_release(spec.repo) {
            let ver = tag.strip_prefix('v').unwrap_or(&tag).to_string();
            cache_store_version(spec.id, ver.clone());
            out.insert(spec.id.to_string(), json!(ver));
        }
    }
    Value::Object(out)
}

/// Vérifie en ligne (npm, PyPI, GitHub) les dernières versions des serveurs de la boutique
/// (tous si `ids` est `None`, ou seulement la liste fournie), met en cache les résultats
/// et renvoie la table `{ "<store_id>": "<version>" }` mise à jour.
pub fn refresh_store_versions(ids: Option<&[String]>) -> Value {
    #[cfg(not(test))]
    {
        let targets: Vec<(&'static str, StorePackageSource)> = all_store_items()
            .filter(|it| match ids {
                Some(list) => list.iter().any(|x| x == it.id),
                None => true,
            })
            .filter_map(|it| extract_store_package(it).map(|src| (it.id, src)))
            .collect();
        if !targets.is_empty() {
            let workers = targets.len().min(8);
            let chunk_size = targets.len().div_ceil(workers);
            std::thread::scope(|s| {
                for chunk in targets.chunks(chunk_size) {
                    s.spawn(move || {
                        for (id, src) in chunk {
                            if let Some(ver) = fetch_live_store_version(src) {
                                cache_store_version(id, ver);
                            }
                        }
                    });
                }
            });
        }
    }
    #[cfg(test)]
    let _ = ids;
    store_versions_json()
}

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
                .map(|s| {
                    let version = extract_pinned_version(s.args)
                        .or_else(|| (!s.version.is_empty()).then(|| s.version.to_string()));
                    let repo = find_github_release(s.id).map(|r| r.repo);
                    json!({
                        "id": s.id, "name": s.name, "desc": s.desc,
                        "cat": s.cat, "kind": s.kind,
                        "program": s.program, "args": s.args,
                        "env_vars": s.env_vars,
                        "version": version,
                        "repo": repo,
                    })
                })
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
