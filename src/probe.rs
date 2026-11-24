use crate::types::ToolInfo;
use serde_json::{json, Value};
use std::io::{BufRead, Read, Write};
use std::net::{SocketAddr, TcpStream};
#[cfg_attr(not(windows), allow(unused_imports))] // Stdio : chemins cfg(windows)
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// En-têtes utiles d'une réponse HTTP (inférence du langage serveur + session MCP).
#[derive(Default, Clone)]
pub struct RespHead {
    pub content_type: String,
    pub server: String,
    pub powered_by: String,
    pub session_id: String,
}

impl RespHead {
    pub fn sse(&self) -> bool {
        self.content_type.contains("text/event-stream")
    }
}

pub struct McpProbe {
    pub mcp_up: bool,
    #[allow(dead_code)] // réservé : distinguer port fermé vs MCP muet côté UI
    pub port_open: bool,
    /// Serveur LiteMCP verrouillé (a répondu 401 + `WWW-Authenticate` de la lib,
    /// ou est sondé avec une identité cliente signée — badge cadenas sur UP/DOWN).
    pub locked: bool,
    pub pid: Option<u32>,
    pub host: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    pub protocol: Option<String>,
    /// Réponses JSON-RPC encodées SSE (Content-Type: text/event-stream sur
    /// l'initialize). NE suffit PAS à qualifier un transport SSE : FastMCP
    /// httpStream répond par défaut ainsi (enableJsonResponse=false) tout en
    /// restant du streamable HTTP.
    pub sse: bool,
    /// Endpoint SSE legacy co-servé (GET /sse → 200 + `event:`). Vrai sur un
    /// FastMCP httpStream (compat 2024-11-05) ET sur un vrai serveur SSE.
    pub sse_endpoint: bool,
    /// Le serveur accepte une upgrade WebSocket (101 à un GET avec
    /// `Upgrade: websocket`) — badge de transport « WS » dans l'UI.
    pub ws: bool,
    /// Langage inféré (stdio : la commande ; HTTP : en-têtes Server/X-Powered-By).
    /// La spec MCP n'expose PAS le langage dans serverInfo.
    pub lang: Option<String>,
    /// Framework détecté par fingerprint — "litemcp" si le serveur répond
    /// initialize avec la version 2026-07-28 SANS négocier (le probe demande
    /// 2024-11-05, litemcp ignore la version demandée) ET que sa sonde
    /// GET /health répond 200 « ok » (deux signatures documentées de litemcp).
    pub framework: Option<String>,
    /// Vrai si le serveur tourne dans un conteneur (Docker/Podman/containerd).
    pub in_container: bool,
    /// Nom du conteneur si détecté (`docker ps`, `--name`, `docker exec`).
    pub container_name: Option<String>,
    pub tools: Vec<ToolInfo>,
    /// Vrai quand une sonde légère sur serveur déjà UP (nom + version inchangés)
    /// a sauté `tools/list` et les sondes annexes (`/sse`, WS, `/health`).
    /// L'appelant conserve alors les métadonnées/outils en cache.
    pub metadata_reused: bool,
    pub latency_ms: u64,
    pub error: Option<String>,
}

/// Indice de sonde légère pour un serveur déjà UP (P1 rang 2) : si `initialize`
/// renvoie les mêmes `name` et `version`, la sonde saute `tools/list` et les
/// vérifications annexes (`/sse`, WS, `/health`, `port_owner_lang`).
#[derive(Clone, Copy, Debug, Default)]
pub struct ProbeHint<'a> {
    pub reuse_metadata_if_unchanged: bool,
    pub prev_name: Option<&'a str>,
    pub prev_version: Option<&'a str>,
}

impl McpProbe {
    pub fn fail(err: String, port_open: bool) -> McpProbe {
        McpProbe {
            mcp_up: false,
            port_open,
            locked: false,
            pid: None,
            host: None,
            name: None,
            version: None,
            protocol: None,
            sse: false,
            sse_endpoint: false,
            ws: false,
            lang: None,
            framework: None,
            in_container: false,
            container_name: None,
            tools: vec![],
            metadata_reused: false,
            latency_ms: 0,
            error: Some(err),
        }
    }

    /// Serveur LiteMCP joignable mais verrouillé (`401` + `WWW-Authenticate`
    /// `Signature realm="litemcp", tag="litemcp-auth-v1"`, lot M4 point 4).
    pub fn locked_litemcp(host: String, health: bool, latency_ms: u64) -> McpProbe {
        let fw = crate::frameworks::detect(
            None,
            Some(crate::LITEMCP_PROTOCOL),
            health,
            false,
            false,
            Some("rust"),
        )
        .or_else(|| crate::frameworks::by_id("litemcp"));
        McpProbe {
            mcp_up: false,
            port_open: true,
            locked: true,
            pid: None,
            host: Some(host),
            name: None,
            version: None,
            protocol: Some(crate::LITEMCP_PROTOCOL.to_string()),
            sse: false,
            sse_endpoint: false,
            ws: false,
            lang: Some("rust".to_string()),
            framework: fw.map(|f| f.id.to_string()),
            in_container: false,
            container_name: None,
            tools: vec![],
            metadata_reused: false,
            latency_ms,
            error: Some(crate::auth::net::LOCKED_SERVER_MSG.to_string()),
        }
    }
}

/// Canal JSON-RPC 2.0 vivant vers un serveur MCP stdio déjà initialisé.
/// Permet au pont hybride Streamable HTTP (`POST /mcp/s/<id>`) et au testeur
/// d'outils de l'UI de relayer des requêtes sans relancer le process.
pub struct StdioRpc {
    stdin: Box<dyn Write + Send>,
    rx: std::sync::mpsc::Receiver<String>,
    next_id: u64,
}

/// Compare un champ JSON-RPC `id` (entier ou chaîne numérique) à l'identifiant attendu.
fn rpc_id_matches(v: Option<&Value>, expected: u64) -> bool {
    match v {
        Some(Value::Number(n)) => n.as_u64() == Some(expected),
        Some(Value::String(s)) => s.parse::<u64>().ok() == Some(expected),
        _ => false,
    }
}

#[cfg(test)]
struct LineChannelWriter {
    tx: std::sync::mpsc::Sender<String>,
    buf: Vec<u8>,
}

#[cfg(test)]
impl Write for LineChannelWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let text = String::from_utf8_lossy(&line).to_string();
            if self.tx.send(text).is_err() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "mock stdio receiver closed",
                ));
            }
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Construit un [`StdioHandle`] de test purement en mémoire (sans spawn externe)
/// dont le canal JSON-RPC répond via la closure `responder`.
#[cfg(test)]
pub(crate) fn test_stdio_handle_with_mock<F>(mut responder: F) -> StdioHandle
where
    F: FnMut(Value) -> Option<Value> + Send + 'static,
{
    let (tx_in, rx_in) = std::sync::mpsc::channel::<String>();
    let (tx_out, rx_out) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in rx_in {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(req) = serde_json::from_str::<Value>(trimmed) else {
                continue;
            };
            if let Some(resp) = responder(req) {
                if tx_out.send(resp.to_string()).is_err() {
                    break;
                }
            }
        }
    });
    StdioHandle {
        pid: 0,
        started: None,
        exit: std::sync::Arc::new(std::sync::Mutex::new(None)),
        rpc: Some(std::sync::Arc::new(std::sync::Mutex::new(StdioRpc {
            stdin: Box::new(LineChannelWriter {
                tx: tx_in,
                buf: Vec::new(),
            }),
            rx: rx_out,
            next_id: 100,
        }))),
    }
}

/// Handle d'un serveur lancé (mode "start"). Le process est surveillé par un
/// watcher — un thread dédié au wait() bloquant, hors du cycle de supervision :
/// `exit` passe à Some(code) dès que le process est sorti, déterministe sur
/// tous les OS (pas de course WNOHANG, pas de sémantique ECHILD divergente
/// entre Windows, Linux et macOS — cf. Release #36, échec CI du lot 2).
pub struct StdioHandle {
    /// PID du process lancé (kill_tree, affichage).
    pub pid: u32,
    /// Date de création du process au moment du spawn (identité : un PID
    /// recyclé n'a jamais la même date). `None` si l'API n'a pas répondu.
    pub started: Option<u64>,
    /// Code de sortie ; None tant que le process est en cours. Récolté par le
    /// watcher (signal non exprimable → -1).
    pub exit: std::sync::Arc<std::sync::Mutex<Option<i32>>>,
    /// Canal JSON-RPC stdio persistant (pont hybride HTTP + `call_tool`).
    pub rpc: Option<std::sync::Arc<std::sync::Mutex<StdioRpc>>>,
}

/// Construit le handle et démarre le watcher qui récolte la sortie du process.
fn handle_de_child(mut child: Child, rpc: Option<StdioRpc>) -> StdioHandle {
    let pid = child.id();
    let started = process_creation(pid);
    let exit = std::sync::Arc::new(std::sync::Mutex::new(None));
    let slot = std::sync::Arc::clone(&exit);
    std::thread::spawn(move || {
        let code = child.wait().ok().and_then(|st| st.code()).unwrap_or(-1);
        if let Ok(mut slot) = slot.lock() {
            *slot = Some(code);
        }
    });
    StdioHandle {
        pid,
        started,
        exit,
        rpc: rpc.map(|r| std::sync::Arc::new(std::sync::Mutex::new(r))),
    }
}

/// Exécute une requête JSON-RPC 2.0 synchrone sur un canal stdio vivant.
pub fn call_stdio_rpc(
    rpc: &std::sync::Arc<std::sync::Mutex<StdioRpc>>,
    method: &str,
    params: Value,
    timeout_ms: u64,
) -> Result<(Value, u64), String> {
    let t0 = Instant::now();
    let mut guard = rpc.lock().unwrap_or_else(|p| p.into_inner());
    let req_id = guard.next_id;
    guard.next_id = guard.next_id.saturating_add(1);
    let payload = json!({
        "jsonrpc": "2.0",
        "id": req_id,
        "method": method,
        "params": params,
    });
    writeln!(guard.stdin, "{}", payload).map_err(|e| format!("écriture stdio : {e}"))?;
    guard
        .stdin
        .flush()
        .map_err(|e| format!("flush stdio : {e}"))?;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(format!("timeout stdio sur {method} ({timeout_ms} ms)"));
        }
        match guard.rx.recv_timeout(deadline - now) {
            Ok(line) => {
                let t = line.trim().trim_start_matches('\u{feff}');
                if !t.starts_with('{') {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<Value>(t) else {
                    continue;
                };
                if v.get("method").is_some() || !rpc_id_matches(v.get("id"), req_id) {
                    continue;
                }
                if let Some(err) = v.get("error") {
                    let msg = err
                        .get("message")
                        .and_then(|x| x.as_str())
                        .unwrap_or("erreur JSON-RPC");
                    let code = err.get("code").and_then(|x| x.as_i64()).unwrap_or(-32000);
                    return Err(format!("erreur MCP ({code}) : {msg}"));
                }
                let res = v.get("result").cloned().unwrap_or(Value::Null);
                return Ok((res, t0.elapsed().as_millis() as u64));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                return Err(format!("timeout stdio sur {method} ({timeout_ms} ms)"));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("process stdio terminé (stdout fermé)".to_string());
            }
        }
    }
}

/// Appelle un outil MCP (`tools/call`) sur un canal stdio vivant.
pub fn call_stdio_tool(
    rpc: &std::sync::Arc<std::sync::Mutex<StdioRpc>>,
    tool: &str,
    arguments: Value,
    timeout_ms: u64,
) -> Result<(Value, u64), String> {
    call_stdio_rpc(
        rpc,
        "tools/call",
        json!({ "name": tool, "arguments": arguments }),
        timeout_ms,
    )
}

/// Interroge `tools/list` (avec pagination `nextCursor`) sur un canal stdio vivant.
pub fn list_stdio_tools(
    rpc: &std::sync::Arc<std::sync::Mutex<StdioRpc>>,
    timeout_ms: u64,
) -> Result<(Vec<ToolInfo>, u64), String> {
    let t0 = Instant::now();
    let mut tools = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..10 {
        let params = match &cursor {
            Some(c) => json!({ "cursor": c }),
            None => json!({}),
        };
        let (res, _) = call_stdio_rpc(rpc, "tools/list", params, timeout_ms)?;
        if let Some(arr) = res.get("tools").and_then(|x| x.as_array()) {
            for t in arr {
                tools.push(parse_tool_info(t));
            }
        }
        cursor = res
            .get("nextCursor")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }
    Ok((tools, t0.elapsed().as_millis() as u64))
}

fn resolve_windows_cmd(program: &str) -> String {
    #[cfg(windows)]
    {
        // Windows : CreateProcess ne résout que les .exe — npx/uvx peuvent être
        // des .cmd/.bat/.exe. On nettoie guillemets et espaces sur chaque entrée PATH.
        let p = std::path::Path::new(program);
        if p.extension().is_none() {
            if program.contains('\\') || program.contains('/') {
                for ext in ["cmd", "bat", "exe"] {
                    let cand = std::path::PathBuf::from(format!("{program}.{ext}"));
                    if cand.exists() {
                        return cand.to_string_lossy().to_string();
                    }
                }
            } else {
                let path_var = std::env::var("PATH").unwrap_or_default();
                for raw_dir in path_var.split(';') {
                    let dir = raw_dir.trim().trim_matches('"').trim();
                    if dir.is_empty() {
                        continue;
                    }
                    for ext in ["cmd", "bat", "exe"] {
                        let cand = std::path::Path::new(dir).join(format!("{program}.{ext}"));
                        if cand.exists() {
                            return cand.to_string_lossy().to_string();
                        }
                    }
                }
            }
        }
        program.to_string()
    }
    #[cfg(not(windows))]
    {
        program.to_string()
    }
}

/// Si `dir` contient `node.exe` et `node_modules/npm/bin/npx-cli.js`,
/// renvoie `(node_exe, npx_cli_js)` pour contourner `cmd.exe` / `npx.cmd`.
#[cfg_attr(not(windows), allow(dead_code))]
fn resolve_npx_in_dir(dir: &std::path::Path) -> Option<(String, String)> {
    let node_exe = dir.join("node.exe");
    let npx_cli = dir
        .join("node_modules")
        .join("npm")
        .join("bin")
        .join("npx-cli.js");
    if node_exe.is_file() && npx_cli.is_file() {
        Some((
            node_exe.to_string_lossy().to_string(),
            npx_cli.to_string_lossy().to_string(),
        ))
    } else {
        None
    }
}

/// Résout la commande réelle à spawner pour un serveur stdio :
/// sous Windows, contourne `npx.cmd` (`cmd.exe`) en lançant `node.exe`
/// directement sur `npx-cli.js` dès que possible.
pub(crate) fn resolve_stdio_command(program: &str, args: &[String]) -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        let resolved = resolve_windows_cmd(program);
        let p = std::path::Path::new(&resolved);
        let stem = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let ext = p
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if stem == "npx" && matches!(ext.as_str(), "cmd" | "bat") {
            if let Some(parent) = p.parent() {
                if let Some((node_exe, npx_cli)) = resolve_npx_in_dir(parent) {
                    let mut full_args = Vec::with_capacity(args.len() + 1);
                    full_args.push(npx_cli);
                    full_args.extend(args.iter().cloned());
                    return (node_exe, full_args);
                }
            }
        }
        (resolved, args.to_vec())
    }
    #[cfg(not(windows))]
    {
        let _ = (resolve_windows_cmd,);
        (program.to_string(), args.to_vec())
    }
}

/// Applique CREATE_NO_WINDOW : aucune fenêtre console ne s'ouvre, même quand le
/// parent est une app GUI (Tauri). À utiliser pour TOUT process console lancé par l'app.
pub fn hidden(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

// ---------- outils système par chemin ABSOLU (lot S4, point 5) ----------

/// Chemin absolu d'un outil système Windows : un `netstat.exe` (ou `reg.exe`,
/// `taskkill.exe`…) déposé dans le dossier courant ne peut plus être pris à la
/// place de l'outil du système, quel que soit le PATH ou le cwd de l'app.
/// Hors Windows, les outils (`ss`, `lsof`, `kill`, `ip`…) restent résolus par
/// le PATH : la convention y est `/usr/bin` ou `/bin`, sans dossier courant.
pub fn system_tool(name: &str) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
        let root = std::path::PathBuf::from(root);
        match name {
            // PowerShell 5.1 n'est pas à la racine de System32
            "powershell" => root
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe"),
            // explorer.exe vit à la racine de %SystemRoot%, pas dans System32
            "explorer" => root.join("explorer.exe"),
            // OpenSSH Windows vit dans System32\OpenSSH (repli PATH si absent)
            "ssh" => {
                let p = root.join("System32").join("OpenSSH").join("ssh.exe");
                if p.exists() {
                    p
                } else {
                    std::path::PathBuf::from("ssh")
                }
            }
            other => root.join("System32").join(format!("{other}.exe")),
        }
    }
    #[cfg(not(windows))]
    {
        std::path::PathBuf::from(name)
    }
}

/// `Command` sur un outil système, chemin absolu sous Windows (voir
/// [`system_tool`]) et `CREATE_NO_WINDOW` appliqué automatiquement pour qu'aucune
/// fenêtre console ne s'ouvre depuis l'app GUI (Tauri). Tous les appels hors tests
/// passent par ici.
pub fn system_command(name: &str) -> Command {
    let mut cmd = Command::new(system_tool(name));
    hidden(&mut cmd);
    cmd
}

// ---------- identité d'un process (lot S4, point 1) ----------

/// Date de création d'un process (ticks FILETIME Windows, 100 ns depuis 1601).
/// Sert d'identité : un PID recyclé par l'OS n'a pas la même date de création,
/// donc un arrêt ne peut plus viser un process différent de celui lancé.
/// `None` = process inexistant ou information indisponible (hors Windows).
#[cfg(windows)]
pub fn process_creation(pid: u32) -> Option<u64> {
    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn GetProcessTimes(
            handle: isize,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }
    // PROCESS_QUERY_LIMITED_INFORMATION : suffit pour GetProcessTimes, et
    // n'exige pas plus de droits que l'affichage de l'état du process.
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h == 0 {
            return None;
        }
        let mut creation = FileTime { low: 0, high: 0 };
        let mut exit = FileTime { low: 0, high: 0 };
        let mut kernel = FileTime { low: 0, high: 0 };
        let mut user = FileTime { low: 0, high: 0 };
        let ok = GetProcessTimes(h, &mut creation, &mut exit, &mut kernel, &mut user);
        CloseHandle(h);
        if ok == 0 {
            return None;
        }
        Some(((creation.high as u64) << 32) | creation.low as u64)
    }
}

#[cfg(not(windows))]
pub fn process_creation(_pid: u32) -> Option<u64> {
    None
}

/// Indique si le processus `pid` est encore vivant (non terminé et non zombie).
/// Retourne `false` pour `pid == 0`, pour un PID inexistant, ou (sous Windows)
/// lorsque le handle est encore tenu mais que `GetExitCodeProcess` ne vaut plus
/// `STILL_ACTIVE` (259).
#[cfg(windows)]
pub fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn GetExitCodeProcess(handle: isize, exit_code: *mut u32) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    const ERROR_ACCESS_DENIED: i32 = 5;
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h == 0 {
            return std::io::Error::last_os_error().raw_os_error() == Some(ERROR_ACCESS_DENIED);
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(h, &mut code);
        CloseHandle(h);
        ok != 0 && code == STILL_ACTIVE
    }
}

/// Hors Windows : sonde par `kill(pid, 0)` (borné à `i32::MAX` pour éviter tout
/// débordement négatif) et écarte les états zombie/mort (`Z`, `X`, `x`) dans
/// `/proc/{pid}/stat` lorsqu'il est disponible.
#[cfg(not(windows))]
pub fn process_alive(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    let rc = unsafe { kill(pid as i32, 0) };
    if rc == 0 {
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            if let Some((_, rest)) = stat.rsplit_once(')') {
                if let Some(state) = rest.trim_start().chars().next() {
                    if matches!(state, 'Z' | 'X' | 'x') {
                        return false;
                    }
                }
            }
        }
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(1)
}

/// Nom d'image d'un PID (`tasklist` sous Windows, `ps` ailleurs) — pour NOMMER
/// le process qui tient un port dans un refus, jamais pour l'exécuter.
pub fn process_image_name(pid: u32) -> Option<String> {
    #[cfg(windows)]
    {
        let mut c = system_command("tasklist");
        let out = hidden(&mut c)
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let first = text.lines().next()?.trim();
        if !first.starts_with('"') {
            return None;
        }
        Some(first.split('"').nth(1)?.to_string())
    }
    #[cfg(not(windows))]
    {
        let mut c = Command::new("ps");
        let out = c
            .args(["-p", &pid.to_string(), "-o", "comm="])
            .output()
            .ok()?;
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!name.is_empty()).then_some(name)
    }
}

/// Tue une liste de PID (taskkill /F sous Windows, kill -9 ailleurs).
pub fn kill_pids(pids: &[u32]) {
    for pid in pids {
        kill_tree(*pid);
    }
}

// ---------- SID utilisateur / propriétaire d'un fichier (points 2 et 4) ----------

/// SID du compte qui exécute l'application (`S-1-5-21-…`). Sert à écrire des
/// ACL indépendantes de la langue du système (`Administrateurs` /
/// `Administrators`).
#[cfg(windows)]
pub fn current_user_sid() -> Option<String> {
    #[link(name = "advapi32")]
    extern "system" {
        fn OpenProcessToken(process: isize, access: u32, token: *mut isize) -> i32;
        fn GetTokenInformation(
            token: isize,
            class: u32,
            info: *mut u8,
            len: u32,
            ret: *mut u32,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn CloseHandle(handle: isize) -> i32;
    }
    const TOKEN_QUERY: u32 = 0x0008;
    // TOKEN_USER = 1 : SID_AND_ATTRIBUTES, le SID commence au premier champ.
    const TOKEN_USER_CLASS: u32 = 1;
    unsafe {
        let mut token: isize = 0;
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let mut len: u32 = 0;
        GetTokenInformation(token, TOKEN_USER_CLASS, std::ptr::null_mut(), 0, &mut len);
        let mut buf = vec![0u8; len.max(64) as usize];
        let ok = GetTokenInformation(
            token,
            TOKEN_USER_CLASS,
            buf.as_mut_ptr(),
            buf.len() as u32,
            &mut len,
        );
        CloseHandle(token);
        if ok == 0 {
            return None;
        }
        let sid = std::ptr::read_unaligned(buf.as_ptr() as *const *mut u8);
        sid_to_string(sid)
    }
}

#[cfg(windows)]
unsafe fn sid_to_string(sid: *mut u8) -> Option<String> {
    #[link(name = "advapi32")]
    extern "system" {
        fn ConvertSidToStringSidW(sid: *mut u8, out: *mut *mut u16) -> i32;
        fn LocalFree(mem: isize) -> isize;
    }
    if sid.is_null() {
        return None;
    }
    let mut wide: *mut u16 = std::ptr::null_mut();
    if ConvertSidToStringSidW(sid, &mut wide) == 0 || wide.is_null() {
        return None;
    }
    let mut len = 0usize;
    while *wide.add(len) != 0 {
        len += 1;
    }
    let s = String::from_utf16_lossy(std::slice::from_raw_parts(wide, len));
    LocalFree(wide as isize);
    Some(s)
}

#[cfg(not(windows))]
pub fn current_user_sid() -> Option<String> {
    extern "C" {
        fn geteuid() -> u32;
    }
    Some(unsafe { geteuid() }.to_string())
}

/// SID du PROPRIÉTAIRE d'un fichier. `None` = illisible ou non pris en charge
/// (hors Windows) — l'appelant refuse alors par prudence.
#[cfg(windows)]
pub fn file_owner_sid(path: &std::path::Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "advapi32")]
    extern "system" {
        fn GetNamedSecurityInfoW(
            object: *const u16,
            object_type: u32,
            security_info: u32,
            owner: *mut *mut u8,
            group: *mut *mut u8,
            dacl: *mut *mut u8,
            sacl: *mut *mut u8,
            descriptor: *mut *mut u8,
        ) -> u32;
        fn LocalFree(mem: isize) -> isize;
    }
    const SE_FILE_OBJECT: u32 = 1;
    const OWNER_SECURITY_INFORMATION: u32 = 1;
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let mut owner: *mut u8 = std::ptr::null_mut();
        let mut descriptor: *mut u8 = std::ptr::null_mut();
        let rc = GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        );
        if rc != 0 {
            return None;
        }
        let sid = sid_to_string(owner);
        if !descriptor.is_null() {
            LocalFree(descriptor as isize);
        }
        sid
    }
}

/// Hors Windows : uid numérique du propriétaire du fichier (comparé à
/// `geteuid()` via [`current_user_sid`]).
#[cfg(not(windows))]
pub fn file_owner_sid(path: &std::path::Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let owner = std::fs::metadata(path).ok()?.uid();
    Some(owner.to_string())
}

/// Sous Windows, le jeton du processus courant est-il élevé (ex. Administrateur
/// / runner CI) ? Un processus élevé crée par défaut ses fichiers avec le
/// propriétaire NTFS `BUILTIN\Administrators` (`S-1-5-32-544`).
#[cfg(windows)]
fn current_token_is_elevated() -> bool {
    #[link(name = "advapi32")]
    extern "system" {
        fn OpenProcessToken(process: isize, access: u32, token: *mut isize) -> i32;
        fn GetTokenInformation(
            token: isize,
            class: u32,
            info: *mut u8,
            len: u32,
            ret: *mut u32,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn CloseHandle(handle: isize) -> i32;
    }
    const TOKEN_QUERY: u32 = 0x0008;
    const TOKEN_ELEVATION_CLASS: u32 = 20;
    unsafe {
        let mut token: isize = 0;
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevated: u32 = 0;
        let mut ret: u32 = 0;
        let ok = GetTokenInformation(
            token,
            TOKEN_ELEVATION_CLASS,
            (&mut elevated as *mut u32).cast::<u8>(),
            std::mem::size_of::<u32>() as u32,
            &mut ret,
        );
        CloseHandle(token);
        ok != 0 && elevated != 0
    }
}

/// Le fichier appartient-il au compte courant ? `None` = impossible à établir.
pub fn file_owned_by_current_user(path: &std::path::Path) -> Option<bool> {
    let (file, me) = (file_owner_sid(path)?, current_user_sid()?);
    #[cfg(windows)]
    {
        if file == me {
            return Some(true);
        }
        if file == "S-1-5-32-544" && current_token_is_elevated() {
            return Some(true);
        }
        Some(false)
    }
    #[cfg(not(windows))]
    {
        Some(file == me)
    }
}

/// Tue l'arbre complet d'un PID (cmd.exe → node.exe). `Child::kill()` ne tue que
/// le lanceur .cmd : les serveurs détachés survivent et gardent leur console.
pub fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let mut c = std::process::Command::new(system_tool("taskkill"));
        let _ = hidden(&mut c)
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        kill_pid_unix(pid);
    }
}

/// Tue le process qui ÉCOUTE réellement sur un port local (méthode du launcher
/// MCP MONITOR v7.3) : `child.kill()` ne tue que le lanceur, pas le serveur
/// détaché qu'il a spawné. netstat -ano → PIDs LISTENING → taskkill /F /PID.
/// PIDs écoutant EXACTEMENT sur `port` (parse par colonne : ":3141" ne matche plus
/// ":31410"), en excluant notre propre PID (on ne se suicide pas si l'id vise notre port).
pub fn pids_listening_on(netstat_text: &str, port: u16) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    for line in netstat_text.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.len() < 5 || !toks.contains(&"LISTENING") {
            continue;
        }
        let Some(addr) = toks.get(1) else { continue };
        if addr.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()) != Some(port) {
            continue;
        }
        if let Some(pid) = toks.last().and_then(|t| t.parse::<u32>().ok()) {
            if pid != std::process::id() && !out.contains(&pid) {
                out.push(pid);
            }
        }
    }
    out
}

#[cfg(windows)]
pub fn kill_port(port: u16) {
    let mut c = Command::new(system_tool("netstat"));
    if let Ok(out) = hidden(&mut c).args(["-ano"]).output() {
        let text = String::from_utf8_lossy(&out.stdout);
        for pid in pids_listening_on(&text, port) {
            let mut t = Command::new(system_tool("taskkill"));
            let _ = hidden(&mut t)
                .args(["/F", "/PID", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

/// PIDs qui écoutent sur `port` (notre propre PID exclu) + nom d'image.
pub fn port_listeners(port: u16) -> Vec<(u32, String)> {
    #[cfg(windows)]
    let pids = {
        let mut c = system_command("netstat");
        match hidden(&mut c).args(["-ano"]).output() {
            Ok(out) => pids_listening_on(&String::from_utf8_lossy(&out.stdout), port),
            Err(_) => Vec::new(),
        }
    };
    #[cfg(not(windows))]
    let pids = listening_pids_unix(port);
    pids.into_iter()
        .map(|pid| {
            let image = process_image_name(pid).unwrap_or_else(|| "process".to_string());
            (pid, image)
        })
        .collect()
}

#[cfg(not(windows))]
fn listening_pids_unix(port: u16) -> Vec<u32> {
    let mut c = Command::new("ss");
    probe_hidden_cmd(&mut c);
    if let Ok(out) = c.args(["-tlnp"]).output() {
        let text = String::from_utf8_lossy(&out.stdout);
        let pids = pids_listening_on_ss(&text, port);
        if !pids.is_empty() {
            return pids;
        }
    }
    let mut out = Vec::new();
    let mut c = Command::new("lsof");
    probe_hidden_cmd(&mut c);
    if let Ok(o) = c
        .args(["-ti", &format!(":{}", port), "-sTCP:LISTEN"])
        .output()
    {
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            if let Ok(pid) = line.trim().parse::<u32>() {
                if pid != std::process::id() && !out.contains(&pid) {
                    out.push(pid);
                }
            }
        }
    }
    out
}

/// Parse la sortie `ss -tlnp` : PIDs (notre propre PID exclu) des listeners sur
/// `port` EXACT. Format : `LISTEN 0 128 0.0.0.0:8080 0.0.0.0:* users:(("node",pid=1234,fd=20))`
pub fn pids_listening_on_ss(text: &str, port: u16) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    for line in text.lines() {
        if !line.contains("LISTEN") {
            continue;
        }
        let on_port = line.split_whitespace().any(|t| {
            t.rsplit_once(':')
                .map(|(_, p)| p.parse::<u16>() == Ok(port))
                .unwrap_or(false)
        });
        if !on_port {
            continue;
        }
        for tok in line.split_whitespace() {
            for seg in tok.split(',') {
                if let Some(pid) = seg.strip_prefix("pid=") {
                    if let Ok(pid) = pid.trim_end_matches(')').parse::<u32>() {
                        if pid != std::process::id() && !out.contains(&pid) {
                            out.push(pid);
                        }
                    }
                }
            }
        }
    }
    out
}

/// Unix : tue le process qui ÉCOUTE réellement sur un port (parité Windows).
///
/// 1) `ss -tlnp` (iproute2) — sans privilèges, seuls les process de
///    l'utilisateur sont visibles ;
/// 2) repli `lsof -ti :PORT -sTCP:LISTEN` (macOS).
#[cfg(not(windows))]
pub fn kill_port(port: u16) {
    let mut c = Command::new("ss");
    probe_hidden_cmd(&mut c);
    if let Ok(out) = c.args(["-tlnp"]).output() {
        let text = String::from_utf8_lossy(&out.stdout);
        let pids = pids_listening_on_ss(&text, port);
        if !pids.is_empty() {
            for pid in pids {
                kill_pid_unix(pid);
            }
            return;
        }
    }
    let mut c = Command::new("lsof");
    probe_hidden_cmd(&mut c);
    if let Ok(out) = c
        .args(["-ti", &format!(":{}", port), "-sTCP:LISTEN"])
        .output()
    {
        if out.status.success() {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                if let Ok(pid) = line.trim().parse::<u32>() {
                    if pid != std::process::id() {
                        kill_pid_unix(pid);
                    }
                }
            }
        }
    }
}

#[cfg(not(windows))]
fn kill_pid_unix(pid: u32) {
    let mut c = Command::new("kill");
    probe_hidden_cmd(&mut c);
    let _ = c.arg("-9").arg(pid.to_string()).status();
}

/// Spawn un process serveur (HTTP/HTTPS lancé par MCP Control) sans handshake
/// JSON-RPC : le probe HTTP classique fera le handshake. stdout/stderr nullés
/// (pas de pipe = pas de risque de blocage buffer plein).
pub fn spawn_process(
    program: &str,
    args: &[String],
    workdir: Option<&str>,
    env: &[(String, String)],
) -> Result<StdioHandle, String> {
    let program = resolve_windows_cmd(program);
    let mut cmd = Command::new(&program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // env additionnel de la commande de lancement (import mcpServers, lot 3) :
    // les valeurs ne vivent que dans le process enfant — jamais journalisées.
    cmd.envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS :
        // l'enfant survit au shell/lanceur et à un Ctrl+C dans la console de l'app.
        cmd.creation_flags(0x0800_0000 | 0x0000_0200 | 0x0000_0008);
    }
    if let Some(wd) = workdir {
        cmd.current_dir(wd);
    }
    let child = cmd
        .spawn()
        .map_err(|e| format!("spawn {}: {}", program, e))?;
    Ok(handle_de_child(child, None))
}

/// Langage inféré d'un serveur stdio depuis sa commande de lancement.
/// (La spec MCP n'expose pas le langage : serverInfo = name/title/version only.)
pub fn detect_lang_stdio(program: &str, args: &[String]) -> Option<String> {
    let p = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_lowercase();
    let p = p
        .trim_end_matches(".exe")
        .trim_end_matches(".cmd")
        .trim_end_matches(".bat");
    let lang = match p {
        "node" | "npx" | "bun" | "deno" | "tsx" | "ts-node" => "node",
        "python" | "python3" | "py" | "uv" | "uvx" | "pipx" | "poetry" | "pdm" => "python",
        "cargo" | "rustc" => "rust",
        "go" => "go",
        "java" | "javaw" | "mvn" | "gradle" => "java",
        "dotnet" => "dotnet",
        "ruby" | "bundle" => "ruby",
        "php" => "php",
        _ => {
            // binaire inconnu : indices dans les args (script .py/.js/.ts)
            let joined = args.join(" ").to_lowercase();
            if joined.contains(".py") {
                "python"
            } else if joined.contains(".js") || joined.contains(".ts") || joined.contains(".mjs") {
                "node"
            } else {
                return None;
            }
        }
    };
    Some(lang.to_string())
}

/// Langage inféré d'un serveur HTTP depuis les en-têtes de réponse
/// (Server / X-Powered-By) ou, à défaut, le nom du serveur MCP.
pub fn detect_lang_http(head: &RespHead, name: Option<&str>) -> Option<String> {
    let h = format!("{} {}", head.server, head.powered_by).to_lowercase();
    let lang = if [
        "uvicorn",
        "hypercorn",
        "gunicorn",
        "werkzeug",
        "aiohttp",
        "starlette",
        "fastapi",
        "quart",
    ]
    .iter()
    .any(|k| h.contains(k))
    {
        "python"
    } else if ["express", "next.js", "fastify", "node"]
        .iter()
        .any(|k| h.contains(k))
    {
        "node"
    } else if ["hyper", "axum", "actix", "rocket", "warp", "salvo"]
        .iter()
        .any(|k| h.contains(k))
    {
        "rust"
    } else if ["kestrel", "asp.net"].iter().any(|k| h.contains(k)) {
        "dotnet"
    } else if ["jetty", "tomcat", "undertow", "netty"]
        .iter()
        .any(|k| h.contains(k))
    {
        "java"
    } else if ["net/http", "gofiber", "fasthttp"]
        .iter()
        .any(|k| h.contains(k))
    {
        "go"
    } else {
        // pas d'en-tête exploitable : indices dans le nom du serveur
        let n = name.unwrap_or("").to_lowercase();
        if n.contains("rust") {
            "rust"
        } else if n.contains("python") {
            "python"
        } else {
            return None;
        }
    };
    Some(lang.to_string())
}

/// Normalise un nom d'exécutable (dernier segment, minuscules, sans .exe/.cmd/.bat).
fn command_stem(program: &str) -> String {
    let p = program
        .trim()
        .trim_matches('"')
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    p.trim_end_matches(".exe")
        .trim_end_matches(".cmd")
        .trim_end_matches(".bat")
        .to_string()
}

fn is_direct_container_cli(stem: &str) -> bool {
    matches!(
        stem,
        "docker" | "docker-compose" | "podman" | "podman-compose" | "nerdctl"
    )
}

fn is_valid_env_assignment(tok: &str) -> bool {
    let Some((k, _)) = tok.split_once('=') else {
        return false;
    };
    let mut chars = k.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn split_shell_words_simple(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut has_tok = false;
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        if in_single {
            if c == '\'' {
                in_single = false;
            } else {
                cur.push(c);
            }
        } else if in_double {
            if c == '"' {
                in_double = false;
            } else if c == '\\' && matches!(chars.peek(), Some('"' | '\\')) {
                if let Some(next) = chars.next() {
                    cur.push(next);
                }
            } else {
                cur.push(c);
            }
        } else if c.is_whitespace() {
            if has_tok {
                out.push(std::mem::take(&mut cur));
                has_tok = false;
            }
        } else if c == '\'' {
            in_single = true;
            has_tok = true;
        } else if c == '"' {
            in_double = true;
            has_tok = true;
        } else {
            cur.push(c);
            has_tok = true;
        }
    }
    if has_tok {
        out.push(cur);
    }
    let mut i = 0usize;
    while i < out.len() {
        if out[i] == "env" || out[i] == "/usr/bin/env" {
            i += 1;
            while i < out.len() && (out[i].starts_with('-') || is_valid_env_assignment(&out[i])) {
                i += 1;
            }
            continue;
        }
        if is_valid_env_assignment(&out[i]) {
            i += 1;
            continue;
        }
        break;
    }
    if i > 0 {
        out.drain(..i);
    }
    out
}

fn valid_container_name(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s.len() > 128 {
        None
    } else {
        Some(s.to_string())
    }
}

fn extract_container_name(stem: &str, args: &[String]) -> Option<String> {
    let mut i = 0usize;
    if !matches!(stem, "docker-compose" | "podman-compose") {
        // Options globales docker/podman/nerdctl avant la sous-commande
        while i < args.len() {
            let a = args[i].as_str();
            if !a.starts_with('-') {
                break;
            }
            if matches!(
                a,
                "-H" | "--host"
                    | "-c"
                    | "--context"
                    | "--config"
                    | "-l"
                    | "--log-level"
                    | "--tlscacert"
                    | "--tlscert"
                    | "--tlskey"
            ) {
                i += 2;
            } else {
                i += 1;
            }
        }
    }
    let mut subcmd = args.get(i).map(|s| s.to_ascii_lowercase())?;
    i += 1;
    if subcmd == "compose" || matches!(stem, "docker-compose" | "podman-compose") {
        if subcmd != "compose" {
            // Pour docker-compose, le premier non-flag lu après options globales est déjà subcmd ;
            // on recule i d'un cran pour sauter d'éventuelles options compose (-f, -p, etc.)
            i -= 1;
        }
        while i < args.len() {
            let a = args[i].as_str();
            if !a.starts_with('-') {
                break;
            }
            if matches!(
                a,
                "-f" | "--file"
                    | "-p"
                    | "--project-name"
                    | "--env-file"
                    | "--profile"
                    | "--project-directory"
            ) {
                i += 2;
            } else {
                i += 1;
            }
        }
        subcmd = args.get(i).map(|s| s.to_ascii_lowercase())?;
        i += 1;
    }
    match subcmd.as_str() {
        "run" | "create" => {
            while i < args.len() {
                let a = args[i].as_str();
                if a == "--name" {
                    return args.get(i + 1).and_then(|v| valid_container_name(v));
                }
                if let Some(val) = a.strip_prefix("--name=") {
                    return valid_container_name(val);
                }
                if a == "--" || !a.starts_with('-') {
                    break;
                }
                if matches!(
                    a,
                    "-e" | "--env"
                        | "--env-file"
                        | "-p"
                        | "--publish"
                        | "-v"
                        | "--volume"
                        | "--mount"
                        | "-w"
                        | "--workdir"
                        | "-u"
                        | "--user"
                        | "--network"
                        | "--net"
                        | "--entrypoint"
                        | "--label"
                        | "-l"
                        | "-h"
                        | "--hostname"
                        | "--add-host"
                        | "--platform"
                        | "--cpus"
                        | "-m"
                        | "--memory"
                        | "--restart"
                        | "--pid"
                        | "--ipc"
                        | "--cap-add"
                        | "--cap-drop"
                        | "--device"
                        | "--dns"
                        | "--gpus"
                        | "--cidfile"
                        | "--cgroup-parent"
                        | "--expose"
                        | "--ip"
                        | "--log-driver"
                        | "--log-opt"
                        | "--pull"
                        | "--runtime"
                        | "--shm-size"
                        | "--tmpfs"
                        | "--ulimit"
                        | "--volumes-from"
                ) {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            None
        }
        "exec" => {
            while i < args.len() {
                let a = args[i].as_str();
                if a == "--" {
                    i += 1;
                    break;
                }
                if !a.starts_with('-') {
                    return valid_container_name(a);
                }
                if matches!(
                    a,
                    "-e" | "--env"
                        | "--env-file"
                        | "-u"
                        | "--user"
                        | "-w"
                        | "--workdir"
                        | "--detach-keys"
                        | "-l"
                        | "--label"
                        | "--index"
                ) {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            args.get(i)
                .filter(|s| !s.starts_with('-'))
                .and_then(|v| valid_container_name(v))
        }
        _ => None,
    }
}

fn detect_container_cmd_depth(
    program: &str,
    args: &[String],
    depth: u8,
) -> (bool, Option<String>) {
    if depth > 3 {
        return (false, None);
    }
    let stem = command_stem(program);
    if is_direct_container_cli(&stem) {
        return (true, extract_container_name(&stem, args));
    }
    if stem == "wsl" {
        let mut i = 0usize;
        while i < args.len() {
            let a = args[i].as_str();
            if matches!(a, "--" | "-e" | "--exec") {
                i += 1;
                break;
            }
            if matches!(
                a,
                "-d" | "--distribution" | "-u" | "--user" | "--cd" | "--shell-type"
            ) {
                i += 2;
                continue;
            }
            if a.starts_with('-') {
                i += 1;
                continue;
            }
            break;
        }
        if let Some(inner_prog) = args.get(i) {
            return detect_container_cmd_depth(inner_prog, &args[i + 1..], depth + 1);
        }
        return (false, None);
    }
    if stem == "cmd" {
        let mut i = 0usize;
        while i < args.len() {
            let low = args[i].to_ascii_lowercase();
            if matches!(low.as_str(), "/c" | "/k" | "-c" | "-k") {
                let rest = &args[i + 1..];
                if rest.len() == 1 {
                    let toks = split_shell_words_simple(&rest[0]);
                    if let Some(first) = toks.first() {
                        return detect_container_cmd_depth(first, &toks[1..], depth + 1);
                    }
                } else if let Some(first) = rest.first() {
                    if first.contains(char::is_whitespace) || first.starts_with('"') {
                        let toks = split_shell_words_simple(first);
                        if let Some(head) = toks.first() {
                            let mut combined = toks[1..].to_vec();
                            combined.extend_from_slice(&rest[1..]);
                            return detect_container_cmd_depth(head, &combined, depth + 1);
                        }
                    } else {
                        return detect_container_cmd_depth(first, &rest[1..], depth + 1);
                    }
                }
                return (false, None);
            }
            i += 1;
        }
        return (false, None);
    }
    if matches!(stem.as_str(), "sh" | "bash" | "zsh" | "ash" | "dash") {
        for (i, a) in args.iter().enumerate() {
            let is_c = a == "-c"
                || (a.starts_with('-')
                    && !a.starts_with("--")
                    && a.ends_with('c')
                    && a[1..].chars().all(|ch| ch.is_ascii_alphabetic()));
            if is_c {
                if let Some(script) = args.get(i + 1) {
                    let toks = split_shell_words_simple(script);
                    if let Some(first) = toks.first() {
                        return detect_container_cmd_depth(first, &toks[1..], depth + 1);
                    }
                }
                return (false, None);
            }
        }
        return (false, None);
    }
    if matches!(stem.as_str(), "powershell" | "pwsh") {
        for (i, a) in args.iter().enumerate() {
            let low = a.to_ascii_lowercase();
            if matches!(low.as_str(), "-c" | "-command" | "/c" | "/command") {
                let rest = &args[i + 1..];
                if rest.len() == 1 {
                    let mut toks = split_shell_words_simple(&rest[0]);
                    if toks.first().is_some_and(|t| t == "&") {
                        toks.remove(0);
                    }
                    if let Some(first) = toks.first() {
                        return detect_container_cmd_depth(first, &toks[1..], depth + 1);
                    }
                } else if !rest.is_empty() {
                    let start = usize::from(rest[0] == "&");
                    if let Some(first) = rest.get(start) {
                        return detect_container_cmd_depth(first, &rest[start + 1..], depth + 1);
                    }
                }
                return (false, None);
            }
        }
        return (false, None);
    }
    (false, None)
}

/// Détecte si une commande de lancement exécute le serveur MCP dans un conteneur
/// (`docker`, `docker-compose`, `podman`, `podman-compose`, `nerdctl`, ou via
/// `wsl`/`cmd`/`sh`/`bash`/`powershell`).
///
/// Ne déclenche PAS de faux positif sur `npx @modelcontextprotocol/server-docker`
/// ou `uvx docker-mcp` (qui pilotent Docker depuis l'hôte).
pub fn detect_container_cmd(program: &str, args: &[String]) -> (bool, Option<String>) {
    detect_container_cmd_depth(program, args, 0)
}

/// Détecte si un `Server` lance un conteneur via son transport stdio ou sa `start_cmd`.
pub fn server_cmd_in_container(s: &crate::types::Server) -> (bool, Option<String>) {
    if let crate::types::Transport::Stdio { program, args, .. } = &s.transport {
        let res = detect_container_cmd(program, args);
        if res.0 {
            return res;
        }
    }
    if let Some(ref cmd) = s.start_cmd {
        return detect_container_cmd(&cmd.program, &cmd.args);
    }
    (false, None)
}

/// Vrai si le nom d'image de processus qui écoute un port local correspond à un
/// proxy/runtime de conteneur (Docker Desktop, docker-proxy, Podman, containerd).
/// N'inclut PAS `wslrelay` seul (utilisé aussi par WSL2 hors conteneur).
pub fn is_container_proxy_image(image: &str) -> bool {
    let i = image.trim().to_ascii_lowercase();
    let i = i.trim_end_matches(".exe");
    matches!(
        i,
        "com.docker.backend"
            | "docker-proxy"
            | "vpnkit"
            | "com.docker.vpnkit"
            | "gvproxy"
            | "dockerd"
            | "podman"
            | "rootlesskit"
            | "slirp4netns"
            | "pasta"
            | "conmon"
            | "containerd-shim"
            | "containerd-shim-runc-v2"
            | "limactl"
            | "lima-guestagent"
    )
}

/// Vrai si le texte `/proc/<pid>/cgroup` indique un processus isolé dans un
/// conteneur (Docker, Kubernetes/containerd, CRI-O, Podman/libpod, LXC),
/// sans faux positif sur `docker.service` côté hôte.
pub fn is_cgroup_in_container(cgroup_text: &str) -> bool {
    for raw in cgroup_text.lines() {
        let low = raw.trim().to_ascii_lowercase();
        if low.is_empty() {
            continue;
        }
        if low.contains("/docker/")
            || (low.contains("/docker-") && low.contains(".scope"))
            || low.contains("/kubepods")
            || low.contains("/cri-containerd-")
            || low.contains("/containerd/")
            || low.contains("/crio-")
            || low.contains("/libpod-")
            || low.contains("/libpod/")
            || low.contains("/lxc/")
            || low.contains("/lxc.payload.")
        {
            return true;
        }
    }
    false
}

/// Parse la sortie de `docker ps --format '{{.Names}}|{{.Ports}}'` (ou Podman)
/// en table `port TCP hôte publié -> nom du conteneur`.
/// Ignore les ports exposés sans mapping hôte (sans `->`) et les ports UDP/SCTP.
pub fn parse_docker_ps_ports(output: &str) -> std::collections::HashMap<u16, String> {
    let mut map = std::collections::HashMap::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((raw_name, raw_ports)) = line.split_once('|').or_else(|| line.split_once('\t'))
        else {
            continue;
        };
        let name = raw_name.trim();
        if name.is_empty() || raw_ports.trim().is_empty() {
            continue;
        }
        for entry in raw_ports.split(',') {
            let entry = entry.trim();
            let Some((host_part, container_part)) = entry.split_once("->") else {
                continue;
            };
            let proto_low = container_part.trim().to_ascii_lowercase();
            if proto_low.ends_with("/udp") || proto_low.ends_with("/sctp") {
                continue;
            }
            let port_spec = host_part.rsplit(':').next().unwrap_or(host_part).trim();
            if let Some((a_s, b_s)) = port_spec.split_once('-') {
                if let (Ok(a), Ok(b)) = (a_s.trim().parse::<u16>(), b_s.trim().parse::<u16>()) {
                    if a > 0 && a <= b && (b - a) < 64 {
                        for p in a..=b {
                            map.entry(p).or_insert_with(|| name.to_string());
                        }
                    }
                }
            } else if let Ok(p) = port_spec.parse::<u16>() {
                if p > 0 {
                    map.entry(p).or_insert_with(|| name.to_string());
                }
            }
        }
    }
    map
}

/// Parse `ss -tlnp` en table `port -> Vec<(Option<pid>, nom_process)>`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_ss_port_owners(
    text: &str,
) -> std::collections::HashMap<u16, Vec<(Option<u32>, String)>> {
    let mut map: std::collections::HashMap<u16, Vec<(Option<u32>, String)>> =
        std::collections::HashMap::new();
    for line in text.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        let Some(listen_idx) = toks.iter().position(|&t| t == "LISTEN") else {
            continue;
        };
        let Some(local_addr) = toks.get(listen_idx + 3) else {
            continue;
        };
        let Some(port) = local_addr
            .rsplit_once(':')
            .and_then(|(_, p)| p.parse::<u16>().ok())
        else {
            continue;
        };
        let Some(users_pos) = line.find("users:(") else {
            continue;
        };
        let mut rest = &line[users_pos + "users:(".len()..];
        while let Some(q_start) = rest.find("(\"") {
            let after_q = &rest[q_start + 2..];
            let Some(q_end) = after_q.find('"') else {
                break;
            };
            let comm = after_q[..q_end].trim().to_ascii_lowercase();
            let after_comm = &after_q[q_end + 1..];
            let seg_end = after_comm.find(')').unwrap_or(after_comm.len());
            let seg = &after_comm[..seg_end];
            let pid = seg.split(',').find_map(|part| {
                part.trim()
                    .strip_prefix("pid=")
                    .and_then(|v| v.parse::<u32>().ok())
            });
            if !comm.is_empty() {
                let entry = map.entry(port).or_default();
                if !entry.iter().any(|(ep, ec)| *ep == pid && ec == &comm) {
                    entry.push((pid, comm));
                }
            }
            rest = &after_comm[seg_end..];
        }
    }
    map
}

/// Analyse la sortie combinée `docker ps` + `ss -tlnp` + `/proc/<pid>/cgroup`
/// collectée lors d'une sonde SSH.
pub fn parse_ssh_container_probe(
    docker_out: &str,
    ss_out: &str,
    port: u16,
) -> (bool, Option<String>) {
    if let Some(name) = parse_docker_ps_ports(docker_out).get(&port) {
        return (true, Some(name.clone()));
    }
    let owners = parse_ss_port_owners(ss_out);
    if owners
        .get(&port)
        .is_some_and(|list| list.iter().any(|(_, img)| is_container_proxy_image(img)))
    {
        return (true, None);
    }
    if is_cgroup_in_container(ss_out) {
        return (true, None);
    }
    (false, None)
}

fn resolve_container_cli(name: &str) -> Option<std::path::PathBuf> {
    let cwd = std::env::current_dir().ok();
    let is_cwd = |dir: &std::path::Path| -> bool {
        let Some(ref c) = cwd else {
            return false;
        };
        #[cfg(windows)]
        {
            dir.to_string_lossy()
                .eq_ignore_ascii_case(&c.to_string_lossy())
        }
        #[cfg(not(windows))]
        {
            dir == c
        }
    };
    #[cfg(windows)]
    {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        for env_key in ["ProgramFiles", "ProgramW6432"] {
            if let Ok(pf) = std::env::var(env_key) {
                if name == "docker" {
                    candidates.push(
                        std::path::Path::new(&pf)
                            .join("Docker")
                            .join("Docker")
                            .join("resources")
                            .join("bin")
                            .join("docker.exe"),
                    );
                } else if name == "podman" {
                    candidates.push(
                        std::path::Path::new(&pf)
                            .join("RedHat")
                            .join("Podman")
                            .join("podman.exe"),
                    );
                }
            }
        }
        if name == "docker" {
            if let Ok(local_app) = std::env::var("LocalAppData") {
                candidates.push(
                    std::path::Path::new(&local_app)
                        .join("Programs")
                        .join("Docker")
                        .join("Docker")
                        .join("resources")
                        .join("bin")
                        .join("docker.exe"),
                );
            }
        }
        for c in candidates {
            if c.is_file() {
                return Some(c);
            }
        }
        let path_var = std::env::var("PATH").unwrap_or_default();
        for raw_dir in path_var.split(';') {
            let dir = raw_dir.trim().trim_matches('"').trim();
            if dir.is_empty() {
                continue;
            }
            let dir_path = std::path::Path::new(dir);
            if !dir_path.is_absolute() || is_cwd(dir_path) {
                continue;
            }
            let cand = dir_path.join(format!("{name}.exe"));
            if cand.is_file() {
                return Some(cand);
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        for prefix in ["/usr/bin", "/usr/local/bin", "/opt/homebrew/bin"] {
            let cand = std::path::Path::new(prefix).join(name);
            if cand.is_file() {
                return Some(cand);
            }
        }
        let path_var = std::env::var("PATH").unwrap_or_default();
        for raw_dir in path_var.split(':') {
            let dir = raw_dir.trim();
            if dir.is_empty() {
                continue;
            }
            let dir_path = std::path::Path::new(dir);
            if !dir_path.is_absolute() || is_cwd(dir_path) {
                continue;
            }
            let cand = dir_path.join(name);
            if cand.is_file() {
                return Some(cand);
            }
        }
        None
    }
}

fn run_docker_ps_ports() -> Option<std::collections::HashMap<u16, String>> {
    const MAX_DOCKER_PS_BYTES: usize = 256 * 1024;
    for cli in ["docker", "podman"] {
        let Some(bin) = resolve_container_cli(cli) else {
            continue;
        };
        let mut cmd = Command::new(&bin);
        cmd.args(["ps", "--format", "{{.Names}}|{{.Ports}}"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        probe_hidden_cmd(&mut cmd);
        let Ok(mut child) = cmd.spawn() else {
            continue;
        };
        let out_reader = child
            .stdout
            .take()
            .map(|sout| std::thread::spawn(move || read_bounded_pipe(sout, MAX_DOCKER_PS_BYTES)));
        let deadline = Instant::now() + Duration::from_millis(1500);
        let mut timed_out = false;
        let status = loop {
            match child.try_wait() {
                Ok(Some(st)) => break Some(st),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        timed_out = true;
                        let _ = child.kill();
                        let _ = child.wait();
                        break None;
                    }
                    std::thread::sleep(Duration::from_millis(15));
                }
                Err(_) => break None,
            }
        };
        let bytes = out_reader.and_then(|h| h.join().ok()).unwrap_or_default();
        if timed_out || !status.is_some_and(|s| s.success()) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        return Some(parse_docker_ps_ports(&text));
    }
    None
}

/// Génération du cycle de sondage pour mutualiser la table port → process
/// (P-3 : une seule construction `netstat` + `tasklist` + `docker ps` par cycle,
/// partagée par tous les workers sans tempête de spawns concurrents).
static PORT_OWNER_CYCLE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Démarre un nouveau cycle de sondage : invalide la table port → process
/// construite au cycle précédent (appelé au début de `poll_all`).
pub fn begin_port_owner_cycle() {
    PORT_OWNER_CYCLE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

struct PortOwnerCache {
    cycle: u64,
    built_at: Instant,
    owners: std::collections::HashMap<u16, Vec<(Option<u32>, String)>>,
    docker_ports: std::collections::HashMap<u16, String>,
    docker_retry_after: Option<Instant>,
}

fn with_port_owner_cache<F, R>(f: F) -> R
where
    F: FnOnce(&PortOwnerCache) -> R,
{
    use std::collections::HashMap;
    use std::sync::atomic::Ordering;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<PortOwnerCache>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| {
        Mutex::new(PortOwnerCache {
            cycle: 0,
            built_at: Instant::now() - Duration::from_secs(60),
            owners: HashMap::new(),
            docker_ports: HashMap::new(),
            docker_retry_after: None,
        })
    });
    let cur_cycle = PORT_OWNER_CYCLE.load(Ordering::Relaxed);
    let mut g = cache.lock().unwrap_or_else(|p| p.into_inner());
    let fresh = g.cycle == cur_cycle && g.built_at.elapsed() < Duration::from_secs(10);
    if !fresh {
        g.owners = build_port_owner_map();
        let can_docker = g
            .docker_retry_after
            .is_none_or(|until| Instant::now() >= until);
        if can_docker {
            match run_docker_ps_ports() {
                Some(dp) => {
                    g.docker_ports = dp;
                    g.docker_retry_after = None;
                }
                None => {
                    g.docker_ports.clear();
                    g.docker_retry_after = Some(Instant::now() + Duration::from_secs(60));
                }
            }
        }
        g.cycle = cur_cycle;
        g.built_at = Instant::now();
    }
    f(&g)
}

/// Langage inféré du process qui écoute réellement un port local
/// (Windows : netstat -ano → PID → tasklist → image ; Linux : ss -tlnp).
/// Mutualisé par cycle de sondage (verrou unique : 12 workers parallèles ne
/// déclenchent qu'UN SEUL `netstat` + `tasklist`, plus repli TTL 10 s hors cycle).
pub fn port_owner_lang(port: u16) -> Option<String> {
    with_port_owner_cache(|g| {
        g.owners
            .get(&port)
            .and_then(|list| list.iter().find_map(|(_, img)| detect_lang_from_image(img)))
    })
}

/// Détecte si un port TCP local est servi par un conteneur (Docker/Podman/containerd).
/// Sources : `docker ps` / `podman ps`, nom d'image proxy de conteneur, et sous
/// Linux `/proc/<pid>/cgroup`.
pub fn port_owner_container(port: u16) -> (bool, Option<String>) {
    with_port_owner_cache(|g| {
        if let Some(name) = g.docker_ports.get(&port) {
            return (true, Some(name.clone()));
        }
        if let Some(list) = g.owners.get(&port) {
            if list.iter().any(|(_, img)| is_container_proxy_image(img)) {
                return (true, None);
            }
            #[cfg(target_os = "linux")]
            for (pid_opt, _) in list {
                if let Some(pid) = pid_opt {
                    if let Ok(cg) = std::fs::read_to_string(format!("/proc/{pid}/cgroup")) {
                        if is_cgroup_in_container(&cg) {
                            return (true, None);
                        }
                    }
                }
            }
        }
        (false, None)
    })
}

/// port → liste des `(pid, nom d'image)` des process propriétaires (Windows).
#[cfg(windows)]
fn build_port_owner_map() -> std::collections::HashMap<u16, Vec<(Option<u32>, String)>> {
    use std::collections::HashMap;
    let mut ports: Vec<(u16, u32)> = Vec::new();
    let mut c = Command::new(system_tool("netstat"));
    if let Ok(out) = hidden(&mut c).args(["-ano"]).output() {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let toks: Vec<&str> = line.split_whitespace().collect();
            if toks.len() >= 5 && toks.contains(&"LISTENING") {
                if let (Some(addr), Some(pid)) = (toks.get(1), toks.last()) {
                    if let (Some(p), Ok(pid)) = (
                        addr.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()),
                        pid.parse::<u32>(),
                    ) {
                        ports.push((p, pid));
                    }
                }
            }
        }
    }
    let mut names: HashMap<u32, String> = HashMap::new();
    let mut t = Command::new(system_tool("tasklist"));
    if let Ok(out) = hidden(&mut t).args(["/FO", "CSV", "/NH"]).output() {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let cols: Vec<&str> = line.split("\",\"").collect();
            if cols.len() > 1 {
                let name = cols[0].trim_matches('"').to_lowercase();
                if let Ok(pid) = cols[1].trim_matches('"').parse::<u32>() {
                    names.insert(pid, name);
                }
            }
        }
    }
    let mut map: HashMap<u16, Vec<(Option<u32>, String)>> = HashMap::new();
    for (p, pid) in ports {
        if let Some(n) = names.get(&pid) {
            let entry = map.entry(p).or_default();
            if !entry.iter().any(|(ep, en)| *ep == Some(pid) && en == n) {
                entry.push((Some(pid), n.clone()));
            }
        }
    }
    map
}

#[cfg(target_os = "linux")]
fn build_port_owner_map() -> std::collections::HashMap<u16, Vec<(Option<u32>, String)>> {
    let mut c = Command::new("ss");
    probe_hidden_cmd(&mut c);
    match c.args(["-tlnp"]).output() {
        Ok(out) => parse_ss_port_owners(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => std::collections::HashMap::new(),
    }
}

#[cfg(all(not(windows), not(target_os = "linux")))]
fn build_port_owner_map() -> std::collections::HashMap<u16, Vec<(Option<u32>, String)>> {
    std::collections::HashMap::new()
}

/// Image de process → langage (node.exe, python.exe, java.exe…).
pub fn detect_lang_from_image(image: &str) -> Option<String> {
    let i = image.to_lowercase();
    let i = i.trim_end_matches(".exe");
    let lang = if ["node", "deno", "bun"].contains(&i) {
        "node"
    } else if i.starts_with("python") {
        "python"
    } else if ["java", "javaw"].contains(&i) {
        "java"
    } else if i == "dotnet" {
        "dotnet"
    } else if i == "ruby" {
        "ruby"
    } else if i == "php" {
        "php"
    } else {
        return None;
    };
    Some(lang.to_string())
}

fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

pub fn socket_addr(host: &str, port: u16) -> Result<SocketAddr, std::net::AddrParseError> {
    // crochets uniquement pour IPv6 ("[::1]:p") ; "127.0.0.1:p" tel quel
    let s = if host.contains(':') {
        format!("[{}]:{}", host, port)
    } else {
        format!("{}:{}", host, port)
    };
    s.parse()
}

pub fn port_open(host: &str, port: u16, timeout: Duration) -> bool {
    match socket_addr(host, port) {
        Ok(addr) => TcpStream::connect_timeout(&addr, timeout).is_ok(),
        Err(_) => false,
    }
}

/// En-tête `Host` conforme RFC 7230 §5.4 : crochets obligatoires sur un littéral IPv6.
fn http_host_header(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{}]:{}", host, port)
    } else {
        format!("{}:{}", host, port)
    }
}

/// Sanitize un chemin HTTP avant insertion dans la request-line (anti-CRLF / request splitting).
fn sanitize_http_path<'a>(path: &'a str, fallback: &'a str) -> &'a str {
    if !path.is_empty()
        && path.starts_with('/')
        && path.chars().all(|c| !c.is_control() && c != ' ')
    {
        path
    } else {
        fallback
    }
}

/// Le endpoint SSE legacy (GET, généralement /sse) sert-il un flux SSE ?
/// Un 200 dont le début de réponse contient `event:` = flux SSE ouvert.
/// Seule signature fiable du transport SSE legacy : un FastMCP httpStream
/// co-sert /sse (compat 2024-11-05), donc ce flag n'a de sens qu'associé au
/// résultat du POST initialize (répondu = streamable HTTP, quelle que soit
/// l'encodage de ses réponses).
pub fn sse_get_host(host: &str, port: u16, path: &str, timeout_ms: u64) -> bool {
    let addr = match socket_addr(host, port) {
        Ok(a) => a,
        Err(_) => return false,
    };
    let timeout = Duration::from_millis(timeout_ms);
    let mut stream = match TcpStream::connect_timeout(&addr, timeout) {
        Ok(s) => s,
        Err(_) => return false,
    };
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(timeout)).ok();
    let path = sanitize_http_path(path, "/sse");
    let host_hdr = http_host_header(host, port);
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n",
        path, host_hdr
    );
    if stream.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut data = Vec::new();
    let mut tmp = [0u8; 4096];
    let deadline = Instant::now() + timeout;
    loop {
        let remain = deadline.saturating_duration_since(Instant::now());
        if remain.is_zero() {
            break;
        }
        stream.set_read_timeout(Some(remain)).ok();
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                data.extend_from_slice(&tmp[..n]);
                // premier événement reçu : suffit à qualifier le flux
                if find_sub(&data, b"event:").is_some() {
                    break;
                }
                if data.len() > MAX_PROBE_READ_SMALL {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    if data.is_empty() {
        return false;
    }
    let head = String::from_utf8_lossy(&data).to_lowercase();
    (head.starts_with("http/1.1 2") || head.starts_with("http/1.0 2"))
        && find_sub(&data, b"event:").is_some()
}

/// sse_get_host sur ::1 puis 127.0.0.1 (binds FastMCP = IPv6-only).
pub fn sse_get(port: u16, path: &str, timeout_ms: u64) -> bool {
    sse_get_host("::1", port, path, timeout_ms) || sse_get_host("127.0.0.1", port, path, timeout_ms)
}

/// Le serveur accepte-t-il une upgrade WebSocket sur `path` ? Envoie un GET
/// avec les en-têtes `Upgrade: websocket` et regarde si la réponse est un
/// `101 Switching Protocols`. Détection seule : aucun frame WS n'est échangé.
/// Sert au badge de transport « WS » de l'UI (les trois modes affichés sont
/// stdio, http et ws).
pub fn ws_upgrade_host(host: &str, port: u16, path: &str, timeout_ms: u64) -> bool {
    let addr = match socket_addr(host, port) {
        Ok(a) => a,
        Err(_) => return false,
    };
    let timeout = Duration::from_millis(timeout_ms.clamp(150, 1500));
    let mut stream = match TcpStream::connect_timeout(&addr, timeout) {
        Ok(s) => s,
        Err(_) => return false,
    };
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(timeout)).ok();
    let path = sanitize_http_path(path, "/mcp");
    let host_hdr = http_host_header(host, port);
    // clé factice RFC 6455 (16 octets base64) — le serveur peut répondre 101
    // sans qu'on poursuive le protocole.
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
        path, host_hdr
    );
    if stream.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut data = Vec::new();
    let mut tmp = [0u8; 2048];
    // une seule lecture suffit : la ligne de statut arrive en tête de réponse
    match stream.read(&mut tmp) {
        Ok(n) if n > 0 => data.extend_from_slice(&tmp[..n]),
        _ => return false,
    }
    let head = String::from_utf8_lossy(&data).to_lowercase();
    head.starts_with("http/1.1 101") || head.starts_with("http/1.0 101")
}

/// Passerelles par défaut IPv4 de la machine (0 à 2 entrées).
/// Sert à mesurer si le réseau LOCAL est joignable : quand un VPN capture le
/// trafic (kill switch), la passerelle LAN ne répond plus alors que la machine
/// est bien connectée — les serveurs distants paraissent alors « en panne ».
/// Ajoute `cand` à `out` si c'est une IPv4 valide non nulle et pas déjà vue.
fn push_gateway(out: &mut Vec<String>, cand: &str) {
    let ip = cand.trim().trim_matches(|c: char| c == '(' || c == ')');
    let ok = ip.split('.').count() == 4
        && ip.split('.').all(|o| o.parse::<u8>().is_ok())
        && ip != "0.0.0.0";
    if ok && !out.iter().any(|x| x == ip) {
        out.push(ip.to_string());
    }
}

pub fn default_gateways() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if cfg!(windows) {
        let mut c = Command::new(system_tool("ipconfig"));
        hidden(&mut c);
        if let Ok(o) = c.output() {
            let text = String::from_utf8_lossy(&o.stdout).to_string();
            for line in text.lines() {
                let low = line.to_lowercase();
                if (low.contains("passerelle") || low.contains("default gateway"))
                    && line.contains(':')
                {
                    if let Some(v) = line.split(':').nth(1) {
                        push_gateway(&mut out, v);
                    }
                }
            }
        }
    } else {
        let mut c = Command::new("ip");
        probe_hidden_cmd(&mut c);
        if let Ok(o) = c.args(["route", "show", "default"]).output() {
            let text = String::from_utf8_lossy(&o.stdout).to_string();
            for line in text.lines() {
                let toks: Vec<&str> = line.split_whitespace().collect();
                if let Some(i) = toks.iter().position(|t| *t == "via") {
                    if let Some(v) = toks.get(i + 1) {
                        push_gateway(&mut out, v);
                    }
                }
            }
        }
        if out.is_empty() {
            // macOS
            let mut c2 = Command::new(system_tool("netstat"));
            probe_hidden_cmd(&mut c2);
            if let Ok(o) = c2.args(["-rn"]).output() {
                let text = String::from_utf8_lossy(&o.stdout).to_string();
                for line in text.lines() {
                    let low = line.to_lowercase();
                    if low.starts_with("default") {
                        if let Some(v) = line.split_whitespace().nth(1) {
                            push_gateway(&mut out, v);
                        }
                    }
                }
            }
        }
    }
    out
}

/// Ping ICMP court (1 paquet) — vrai si la cible répond. Utilisé pour vérifier
/// la passerelle LAN ; jamais bloquant au-delà de `timeout_ms`.
pub fn ping_ok(host: &str, timeout_ms: u64) -> bool {
    let ms = timeout_ms.clamp(200, 3000);
    let mut c = Command::new(system_tool("ping"));
    hidden(&mut c);
    let status = if cfg!(windows) {
        c.args(["-n", "1", "-w", &ms.to_string(), host]).output()
    } else {
        // -W : secondes sous Linux, millisecondes sous macOS — 1 reste sûr.
        c.args(["-c", "1", "-W", "1", host]).output()
    };
    matches!(status, Ok(o) if o.status.success())
}

/// Le réseau local est-il joignable ? (au moins une passerelle répond)
/// `None` = aucune passerelle détectée (pas de réseau local à surveiller).
pub fn lan_reachable() -> Option<bool> {
    let gws = default_gateways();
    if gws.is_empty() {
        return None;
    }
    Some(gws.iter().any(|g| ping_ok(g, 800)))
}

/// GET /health sur un hôte précis — vrai si 2xx avec un corps contenant « ok ».
/// Sonde de vie documentée de litemcp ; second signal du fingerprint du framework.
pub fn health_ok_host(host: &str, port: u16, timeout_ms: u64) -> bool {
    let timeout = Duration::from_millis(timeout_ms);
    let host_hdr = http_host_header(host, port);
    let req = format!(
        "GET /health HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        host_hdr
    );
    if let Ok(Some(ident)) = crate::auth::net::outbound_identity_for(host, port) {
        if !ident.tls_fp.is_empty() {
            let Ok((data, _)) = crate::auth::net::https_exchange_raw(
                host,
                port,
                req.as_bytes(),
                timeout,
                Some(&ident.tls_fp),
                MAX_PROBE_READ_SMALL,
            ) else {
                return false;
            };
            let Some(sep) = find_sub(&data, b"\r\n\r\n") else {
                return false;
            };
            let head = String::from_utf8_lossy(&data[..sep]).to_lowercase();
            return (head.starts_with("http/1.1 2") || head.starts_with("http/1.0 2"))
                && String::from_utf8_lossy(&data[sep + 4..])
                    .to_lowercase()
                    .contains("ok");
        }
    }
    let addr = match socket_addr(host, port) {
        Ok(a) => a,
        Err(_) => return false,
    };
    let mut stream = match TcpStream::connect_timeout(&addr, timeout) {
        Ok(s) => s,
        Err(_) => return false,
    };
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(timeout)).ok();
    if stream.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut data = Vec::new();
    let mut tmp = [0u8; 2048];
    let deadline = Instant::now() + timeout;
    loop {
        let remain = deadline.saturating_duration_since(Instant::now());
        if remain.is_zero() {
            break;
        }
        stream.set_read_timeout(Some(remain)).ok();
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                data.extend_from_slice(&tmp[..n]);
                if data.len() > MAX_PROBE_READ_SMALL {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let sep = match find_sub(&data, b"\r\n\r\n") {
        Some(p) => p,
        None => return false,
    };
    let head = String::from_utf8_lossy(&data[..sep]).to_lowercase();
    (head.starts_with("http/1.1 2") || head.starts_with("http/1.0 2"))
        && String::from_utf8_lossy(&data[sep + 4..])
            .to_lowercase()
            .contains("ok")
}

/// Fingerprint du framework serveur (pur, testé) : litemcp répond initialize
/// avec la version 2026-07-28 sans négocier la version demandée par le client
/// ET expose GET /health → 200 « ok ». Les deux signatures à la fois écartent
/// les serveurs qui partagent l'une ou l'autre (specs à jour, health endpoints
/// génériques).
/// Ancienne API conservée pour les tests : signature comportementale seule.
#[cfg(test)]
fn detect_framework(
    protocol: Option<&str>,
    health: bool,
    sse: bool,
    sse_endpoint: bool,
) -> Option<&'static str> {
    crate::frameworks::detect_signature(protocol, health, sse, sse_endpoint).map(|f| f.id)
}

/// Sonde un port sur les chemins MCP usuels, dans l'ordre : /mcp, /, /api/mcp.
/// Retourne le premier chemin qui répond + sa sonde complète — un serveur
/// monté à la racine ou sous /api/mcp est découvert, pas seulement /mcp.
pub fn probe_mcp_any_path(port: u16, timeout_ms: u64) -> Option<(&'static str, McpProbe)> {
    for path in ["/mcp", "/", "/api/mcp"] {
        let p = probe_http(port, path, timeout_ms);
        if p.mcp_up || p.locked {
            return Some((path, p));
        }
    }
    None
}

// ---------- découverte SSH d'une machine distante ----------

/// Sanitize d'un hôte/utilisateur pour la ligne de commande ssh (anti-injection :
/// ces valeurs arrivent de l'UI). Hôte : lettres/chiffres/point/tiret/deux-points
/// (IPv6) ; utilisateur : lettres/chiffres/point/tiret/underscore.
fn ssh_safe(s: &str) -> Option<String> {
    let ok = !s.is_empty()
        && s.len() <= 253
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'));
    if ok {
        Some(s.to_string())
    } else {
        None
    }
}

/// Ports système écartés de la découverte (écouteurs Windows/Linux sans MCP).
const SSH_PORTS_EXCLUDES: &[u16] = &[
    22, 53, 88, 135, 139, 389, 445, 464, 636, 2179, 3268, 3269, 3389, 5040, 5353, 5355, 5357, 5985,
    47001,
];

/// Parse la sortie `netstat -ano | findstr LISTENING` (Windows) ou `ss -tln` /
/// `netstat -tln` (Linux) en liste de ports en écoute, ports système écartés.
pub fn parse_listening_ports(text: &str) -> Vec<u16> {
    let mut ports = std::collections::BTreeSet::new();
    for line in text.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        // Windows : "TCP    0.0.0.0:8080    0.0.0.0:0    LISTENING    1234"
        if toks.len() >= 4 && toks.contains(&"LISTENING") {
            if let Some(p) = toks[1]
                .rsplit(':')
                .next()
                .and_then(|x| x.parse::<u16>().ok())
            {
                ports.insert(p);
            }
            continue;
        }
        // Linux (ss / netstat -tln) : "LISTEN 0 128 0.0.0.0:8080 ..."
        //  — séparateur ':' ;
        // BSD/macOS (netstat -an -p tcp) : "tcp4 0 0 127.0.0.1.8080 *.* LISTEN"
        //  — séparateur '.', y compris l'IPv6 imprimée "::1.8080" (zone "%lo0"
        //  possible avant le point).
        if toks.contains(&"LISTEN") {
            for t in &toks {
                if let Some((addr, port)) = t.rsplit_once(':') {
                    if (addr.contains('.') || addr == "*" || addr.starts_with('['))
                        && port.parse::<u16>().map(|p| p > 0).unwrap_or(false)
                    {
                        ports.insert(port.parse::<u16>().unwrap_or(0));
                        break;
                    }
                }
                // forme BSD : le port suit le DERNIER point, l'adresse avant
                // ("127.0.0.1.8080", "*.8080", "::1.8080", "fe80::1%lo0.22")
                if let Some((addr, port)) = t.rsplit_once('.') {
                    if (addr.contains('.') || addr.contains('*') || addr.contains(':'))
                        && port.parse::<u16>().map(|p| p > 0).unwrap_or(false)
                    {
                        ports.insert(port.parse::<u16>().unwrap_or(0));
                        break;
                    }
                }
            }
        }
    }
    ports
        .into_iter()
        .filter(|p| !SSH_PORTS_EXCLUDES.contains(p))
        .collect()
}

/// CREATE_NO_WINDOW pour tout process utilitaire (ssh, powershell…) : aucune
/// console ne doit s'ouvrir depuis une app GUI.
pub fn probe_hidden_cmd(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// Indice diagnostic sur une erreur `ssh` (pur, testable).
/// NB : « connect to host … port 22: Permission denied » (blocage socket/pare-feu
/// local) doit être classé comme machine injoignable AVANT le test « permission
/// denied » d'un refus d'authentification par clé publique.
fn ssh_error_hint(stderr: &str) -> &'static str {
    let low = stderr.to_lowercase();
    if low.contains("could not resolve")
        || low.contains("timed out")
        || low.contains("connection refused")
        || low.contains("no route to host")
        || low.contains("network is unreachable")
        || low.contains("connect to host")
    {
        " — machine injoignable (LAN coupé / VPN actif, ou machine éteinte)"
    } else if low.contains("permission denied") || low.contains("publickey") {
        " — clé SSH refusée (ajoute ta clé publique dans authorized_keys de la machine ; \
le mot de passe n'est pas possible en non-interactif)"
    } else {
        ""
    }
}

/// Cible SSH complète (hôte, utilisateur, port optionnel, clé privée optionnelle).
#[derive(Clone, Copy, Debug)]
pub struct SshTarget<'a> {
    pub host: &'a str,
    pub user: &'a str,
    pub port: Option<u16>,
    pub identity_file: Option<&'a str>,
}

impl<'a> SshTarget<'a> {
    pub fn simple(host: &'a str, user: &'a str) -> Self {
        Self {
            host,
            user,
            port: None,
            identity_file: None,
        }
    }

    pub fn from_host(h: &'a crate::types::SshHost) -> Self {
        Self {
            host: &h.ip,
            user: &h.user,
            port: h.port,
            identity_file: h.identity_file.as_deref(),
        }
    }
}

/// Sanitize un chemin local passé à `ssh -i` (non vide, <= 512, aucun caractère
/// de contrôle ni `-` en tête qui serait interprété comme un drapeau).
pub fn ssh_path_safe(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty()
        || t.len() > 512
        || t.starts_with('-')
        || t.chars().any(|c| c.is_control() || c == '\0')
    {
        return None;
    }
    Some(t.to_string())
}

/// Échappe une chaîne quelconque pour un shell POSIX (`'...'` avec `'\''`).
/// Refuse `\0`, `\r`, `\n`.
pub fn posix_shell_quote(s: &str) -> Result<String, String> {
    if s.chars().any(|c| matches!(c, '\0' | '\r' | '\n')) {
        return Err("caractère de contrôle interdit dans un argument shell".to_string());
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    Ok(out)
}

fn env_key_safe(k: &str) -> bool {
    let mut chars = k.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    k.len() <= 128
        && (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn ps_quote(s: &str) -> Result<String, String> {
    if s.chars().any(|c| matches!(c, '\0' | '\r' | '\n')) {
        return Err("caractère de contrôle interdit dans un argument PowerShell".to_string());
    }
    Ok(format!("'{}'", s.replace('\'', "''")))
}

pub(crate) fn build_ssh_args(
    target: &SshTarget<'_>,
    remote_cmd: &str,
    connect_timeout: u64,
) -> Result<Vec<String>, String> {
    let host = ssh_safe(target.host).ok_or_else(|| "hôte invalide".to_string())?;
    let user = ssh_safe(target.user).ok_or_else(|| "utilisateur SSH invalide".to_string())?;
    let mut args = vec![
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=accept-new".to_string(),
        "-o".to_string(),
        format!("ConnectTimeout={}", connect_timeout.clamp(2, 15)),
    ];
    if let Some(port) = target.port {
        if port == 0 {
            return Err("port SSH invalide".to_string());
        }
        if port != 22 {
            args.push("-p".to_string());
            args.push(port.to_string());
        }
    }
    if let Some(raw_key) = target
        .identity_file
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let key = ssh_path_safe(raw_key).ok_or_else(|| "chemin de clé SSH invalide".to_string())?;
        args.push("-i".to_string());
        args.push(key);
    }
    args.push(format!("{user}@{host}"));
    args.push(remote_cmd.to_string());
    Ok(args)
}

/// Exécute une commande sur la machine distante via le client ssh système
/// (aucune dépendance : ssh.exe/OpenSSH livré avec Windows). Auth par clé
/// obligatoire : BatchMode interdit toute invite. Retourne stdout, ou l'erreur
/// ssh en clair (avec indication) pour l'UI.
fn ssh_exec_target(
    target: &SshTarget<'_>,
    remote_cmd: &str,
    connect_timeout: u64,
) -> Result<String, String> {
    let args = build_ssh_args(target, remote_cmd, connect_timeout)?;
    let mut cmd = system_command("ssh");
    cmd.args(&args);
    probe_hidden_cmd(&mut cmd);
    let out = cmd
        .output()
        .map_err(|e| format!("client ssh introuvable ou injoignable : {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if !out.status.success() && stdout.trim().is_empty() {
        let hint = ssh_error_hint(&stderr);
        return Err(format!(
            "ssh {}@{} : {}{}",
            target.user,
            target.host,
            stderr.trim(),
            hint
        ));
    }
    Ok(stdout)
}

#[allow(dead_code)]
fn ssh_exec(
    host: &str,
    user: &str,
    remote_cmd: &str,
    connect_timeout: u64,
) -> Result<String, String> {
    ssh_exec_target(&SshTarget::simple(host, user), remote_cmd, connect_timeout)
}

fn read_bounded_pipe<R: Read>(mut r: R, max_bytes: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    while out.len() < max_bytes {
        match r.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let take = n.min(max_bytes - out.len());
                out.extend_from_slice(&buf[..take]);
            }
        }
    }
    out
}

/// Exécute une commande SSH en poussant `stdin_bytes` puis en fermant stdin (EOF),
/// avec lecture bornée (4 Mio) et échéance globale.
fn ssh_exec_stdin(
    target: &SshTarget<'_>,
    remote_cmd: &str,
    stdin_bytes: &[u8],
    connect_timeout: u64,
    io_timeout_secs: u64,
) -> Result<String, String> {
    const MAX_SSH_PIPE: usize = 4 * 1024 * 1024;
    let args = build_ssh_args(target, remote_cmd, connect_timeout)?;
    let mut cmd = system_command("ssh");
    cmd.args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    probe_hidden_cmd(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("client ssh introuvable ou injoignable : {e}"))?;

    let payload = stdin_bytes.to_vec();
    let writer = child.stdin.take().map(|mut sin| {
        std::thread::spawn(move || {
            let _ = sin.write_all(&payload);
            let _ = sin.flush();
            // sin est droppé ici → EOF côté distant
        })
    });
    let out_reader = child
        .stdout
        .take()
        .map(|sout| std::thread::spawn(move || read_bounded_pipe(sout, MAX_SSH_PIPE)));
    let err_reader = child
        .stderr
        .take()
        .map(|serr| std::thread::spawn(move || read_bounded_pipe(serr, MAX_SSH_PIPE)));

    let deadline = Instant::now() + Duration::from_secs(io_timeout_secs.clamp(2, 60));
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if Instant::now() >= deadline {
                    timed_out = true;
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => break None,
        }
    };
    if let Some(w) = writer {
        let _ = w.join();
    }
    let stdout_bytes = out_reader.and_then(|h| h.join().ok()).unwrap_or_default();
    let stderr_bytes = err_reader.and_then(|h| h.join().ok()).unwrap_or_default();
    let stdout = String::from_utf8_lossy(&stdout_bytes).to_string();
    let stderr = String::from_utf8_lossy(&stderr_bytes).to_string();

    if timed_out {
        return Err(format!(
            "ssh {}@{} : délai d'exécution dépassé ({} s)",
            target.user, target.host, io_timeout_secs
        ));
    }
    let ok = status.is_some_and(|s| s.success());
    if !ok && stdout.trim().is_empty() {
        let hint = ssh_error_hint(&stderr);
        return Err(format!(
            "ssh {}@{} : {}{}",
            target.user,
            target.host,
            stderr.trim(),
            hint
        ));
    }
    Ok(stdout)
}

/// Health-check SSH : la machine est-elle joignable et l'authentification par
/// clé fonctionne-t-elle ? Vrai contrôle en une commande (pas de scan) —
/// à appeler AVANT toute découverte pour éviter un scan de 30 s sur une
/// machine injoignable, et pour distinguer "machine éteinte" de "clé absente".
pub fn ssh_health_target(target: &SshTarget<'_>, timeout_secs: u64) -> Result<(), String> {
    let out = ssh_exec_target(target, "echo __MCC_OK__", timeout_secs)?;
    if out.contains("__MCC_OK__") {
        Ok(())
    } else {
        Err(format!(
            "ssh {}@{} : connexion établie mais la commande distante n'a pas répondu",
            target.user, target.host
        ))
    }
}

pub fn ssh_health(host: &str, user: &str, timeout_secs: u64) -> Result<(), String> {
    ssh_health_target(&SshTarget::simple(host, user), timeout_secs)
}

/// Liste les ports en écoute d'une machine distante (netstat/ss exécutés sur place).
pub fn ssh_netstat_target(target: &SshTarget<'_>, timeout_secs: u64) -> Result<Vec<u16>, String> {
    let stdout = ssh_exec_target(
        target,
        "netstat -ano | findstr LISTENING || ss -tln",
        timeout_secs,
    )?;
    Ok(parse_listening_ports(&stdout))
}

pub fn ssh_netstat(host: &str, user: &str, timeout_secs: u64) -> Result<Vec<u16>, String> {
    ssh_netstat_target(&SshTarget::simple(host, user), timeout_secs)
}

/// Ports TCP en écoute SUR CETTE machine (toutes interfaces), ports système
/// écartés — la source de la découverte locale : l'OS sait déjà ce qui écoute,
/// le scan au boot n'a plus à deviner une plage (un serveur sur 8787 restait
/// invisible avec scan_from/scan_to = 3000-3200).
pub fn local_listening_ports() -> Vec<u16> {
    let out = if cfg!(windows) {
        let mut c = Command::new(system_tool("netstat"));
        hidden(&mut c).args(["-ano"]).output()
    } else if cfg!(target_os = "macos") {
        // BSD netstat : ni `ss` ni `-tln` — `-an -p tcp` liste les sockets TCP
        // et le parseur filtre sur l'état LISTEN (format à séparateur '.').
        let mut c = Command::new(system_tool("netstat"));
        probe_hidden_cmd(&mut c);
        c.args(["-an", "-p", "tcp"]).output()
    } else {
        // ss (iproute2) d'abord, netstat -tln en repli (même format addr:port)
        let mut c = Command::new("ss");
        probe_hidden_cmd(&mut c);
        match c.args(["-tln"]).output() {
            Ok(o) if o.status.success() => Ok(o),
            _ => {
                let mut c2 = Command::new(system_tool("netstat"));
                probe_hidden_cmd(&mut c2);
                c2.args(["-tln"]).output()
            }
        }
    };
    match out {
        Ok(o) => parse_listening_ports(&String::from_utf8_lossy(&o.stdout)),
        Err(_) => Vec::new(),
    }
}

/// Chemin MCP sanitisé (arrive de la config) : commence par /, aucun espace
/// ni quote — il est injecté dans une commande distante.
fn mcp_path_safe(path: &str) -> String {
    let p = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    if !p.is_empty()
        && p.len() <= 128
        && p.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.' | '~'))
    {
        p
    } else {
        "/mcp".to_string()
    }
}

/// Payload JSON-RPC d'un curl interne (aucune single-quote : protégé par '...').
fn mcp_remote_payload(method: &str, id: u64) -> String {
    let params = if method == "tools/list" {
        json!({})
    } else {
        json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "mcp-control", "version": env!("CARGO_PKG_VERSION")}
        })
    };
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

/// Commande curl interne d'un appel MCP sur 127.0.0.1 (à exécuter SUR la machine).
/// `hostport` peut contenir une expression shell SÛRE (${p} pour le batch) :
/// l'URL est en quotes doubles, aucun $ autre que celui voulu (path sanitizé).
fn mcp_remote_curl(hostport: &str, path: &str, payload: &str, curl_timeout: u64) -> String {
    format!(
        "curl -s -m {t} -X POST \"http://127.0.0.1:{hostport}{path}\" \
         -H 'Content-Type: application/json' \
         -H 'Accept: application/json, text/event-stream' \
         -H 'MCP-Protocol-Version: 2024-11-05' \
         -d '{payload}'",
        t = curl_timeout.clamp(1, 15),
        hostport = hostport,
        path = mcp_path_safe(path),
        payload = payload
    )
}

pub const MCC_PORT_MARKER: &str = "__MCC_PORT_";

/// Sonde par lot DEPUIS la machine distante (plusieurs chemins MCP candidats) :
/// un seul exec ssh boucle sur les ports avec curl interne sur 127.0.0.1.
/// Retourne `(port, path, initialize_json)` pour le premier chemin MCP valide par port.
pub fn ssh_probe_mcp_batch_target(
    target: &SshTarget<'_>,
    ports: &[u16],
    paths: &[&str],
    curl_timeout: u64,
) -> Result<Vec<(u16, String, Value)>, String> {
    if ports.is_empty() {
        return Ok(vec![]);
    }
    let mut safe_paths: Vec<String> = Vec::new();
    for raw in paths {
        let p = mcp_path_safe(raw);
        if !safe_paths.contains(&p) {
            safe_paths.push(p);
        }
    }
    if safe_paths.is_empty() {
        safe_paths.push("/mcp".to_string());
    }
    let list = ports
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let payload = mcp_remote_payload("initialize", 1);
    let mut inner = String::new();
    for (idx, mp) in safe_paths.iter().enumerate() {
        let curl = mcp_remote_curl("${p}", mp, &payload, curl_timeout);
        inner.push_str(&format!(
            "echo \"{MCC_PORT_MARKER}${{p}}__{idx}__\"; {curl}; echo; "
        ));
    }
    let remote = format!("for p in {list}; do {inner}done");
    let out = ssh_exec_target(target, &remote, 8)?;
    let mut results: Vec<(u16, String, Value)> = Vec::new();
    for seg in out.split(MCC_PORT_MARKER).skip(1) {
        let Some((pstr, rest)) = seg.split_once("__") else {
            continue;
        };
        let Ok(port) = pstr.trim().parse::<u16>() else {
            continue;
        };
        let Some((idx_str, body)) = rest.split_once("__") else {
            continue;
        };
        let Ok(idx) = idx_str.trim().parse::<usize>() else {
            continue;
        };
        let Some(path) = safe_paths.get(idx) else {
            continue;
        };
        if results.iter().any(|(existing, _, _)| *existing == port) {
            continue;
        }
        if let Some(v) = extract_json(body.as_bytes()) {
            // VALIDATION STRICTE (même règle que la sonde locale/réseau) :
            // serverInfo.name, protocolVersion ou capabilities. Un port qui
            // répond `{"result":{}}` n'est PAS un serveur MCP.
            if crate::validate::is_mcp(&v) {
                results.push((port, path.clone(), v));
            }
        }
    }
    Ok(results)
}

/// Sonde par lot DEPUIS la machine distante : un seul exec ssh boucle sur les
/// ports avec curl interne sur 127.0.0.1 (passe les binds localhost et ufw).
/// Retourne les ports qui répondent à initialize avec un serverInfo + le résultat.
pub fn ssh_probe_mcp_batch(
    host: &str,
    user: &str,
    ports: &[u16],
    path: &str,
    curl_timeout: u64,
) -> Result<Vec<(u16, Value)>, String> {
    let found =
        ssh_probe_mcp_batch_target(&SshTarget::simple(host, user), ports, &[path], curl_timeout)?;
    Ok(found.into_iter().map(|(p, _, v)| (p, v)).collect())
}

/// Sonde COMPLÈTE d'un serveur MCP (initialize + tools/list) exécutée DEPUIS la
/// machine distante : un seul exec ssh, les deux réponses séparées par un
/// marqueur. Utilisé pour le poll des serveurs via_ssh — cf. Server.via_ssh.
pub fn ssh_probe_server_target(
    target: &SshTarget<'_>,
    port: u16,
    path: &str,
    curl_timeout: u64,
) -> McpProbe {
    let t0 = Instant::now();
    let path = mcp_path_safe(path);
    let init = mcp_remote_payload("initialize", 1);
    let tools = mcp_remote_payload("tools/list", 2);
    // Certains serveurs (SDK officiels) attribuent un Mcp-Session-Id à
    // l'initialize et REJETENT tools/list sans lui : en-têtes capturés (-D),
    // session sanitisée (uuid) puis rejouée. AUCUNE double-quote dans la
    // commande distante (l'échappement Windows ssh les mutilerait) : l'en-tête
    // est toujours ajouté — vide quand le serveur n'en émet pas (ignoré).
    let tmp = "/tmp/.mcc_session_$$";
    let remote = format!(
        "{init_curl} -D {tmp}; sid=$(grep -i mcp-session-id {tmp} | head -n 1 | tr -d '\\r' | sed 's/^[^:]*: *//' | tr -cd 'A-Za-z0-9-_' | head -c 100); \
         rm -f {tmp}; echo __MCC_SEP__; {tools_curl} -H 'Mcp-Session-Id:'$sid; echo; \
         echo __MCC_DOCKER__; \
         if command -v docker >/dev/null 2>&1; then \
           (command -v timeout >/dev/null 2>&1 && timeout 2 docker ps --format '{{{{.Names}}}}|{{{{.Ports}}}}' || docker ps --format '{{{{.Names}}}}|{{{{.Ports}}}}') 2>/dev/null; \
         elif command -v podman >/dev/null 2>&1; then \
           (command -v timeout >/dev/null 2>&1 && timeout 2 podman ps --format '{{{{.Names}}}}|{{{{.Ports}}}}' || podman ps --format '{{{{.Names}}}}|{{{{.Ports}}}}') 2>/dev/null; \
         fi; \
         echo __MCC_SS__; \
         ss -tlnp 2>/dev/null | grep -E ':{port}([^0-9]|$)'; \
         for p in $(ss -tlnp 2>/dev/null | grep -E ':{port}([^0-9]|$)' | grep -o 'pid=[0-9]*' | cut -d= -f2); do cat /proc/$p/cgroup 2>/dev/null; done",
        init_curl = mcp_remote_curl(&port.to_string(), &path, &init, curl_timeout),
        tools_curl = mcp_remote_curl(&port.to_string(), &path, &tools, curl_timeout),
        tmp = tmp,
        port = port,
    );
    let out = match ssh_exec_target(target, &remote, 8) {
        Ok(o) => o,
        Err(e) => return McpProbe::fail(e, false),
    };
    let latency = t0.elapsed().as_millis() as u64;
    let (init_body, rest) = out.split_once("__MCC_SEP__").unwrap_or((out.as_str(), ""));
    let (tools_body, container_rest) = rest.split_once("__MCC_DOCKER__").unwrap_or((rest, ""));
    let (docker_out, ss_out) = container_rest
        .split_once("__MCC_SS__")
        .unwrap_or((container_rest, ""));
    let remote_open = !init_body.trim().is_empty();
    let Some(v) = extract_json(init_body.as_bytes()) else {
        return McpProbe::fail(
            "via ssh : pas de réponse initialize".to_string(),
            remote_open,
        );
    };
    let Some(res) = v.get("result") else {
        return McpProbe::fail("via ssh : initialize rejeté".to_string(), true);
    };
    let si = res.get("serverInfo");
    let name = si
        .and_then(|s| s.get("name"))
        .and_then(|x| x.as_str())
        .map(String::from);
    let version = si
        .and_then(|s| s.get("version"))
        .and_then(|x| x.as_str())
        .map(String::from);
    let protocol = res
        .get("protocolVersion")
        .and_then(|x| x.as_str())
        .map(String::from);
    let lang_hint = detect_lang_http(&RespHead::default(), name.as_deref());
    let fw = crate::frameworks::detect(
        name.as_deref(),
        protocol.as_deref(),
        false,
        false,
        false,
        lang_hint.as_deref(),
    );
    let framework = fw.map(|f| f.id.to_string());
    let lang = lang_hint.or_else(|| fw.map(|f| f.lang.to_string()));
    let mut tools = Vec::new();
    if let Some(tv) = extract_json(tools_body.as_bytes()) {
        if let Some(arr) = tv
            .get("result")
            .and_then(|r| r.get("tools"))
            .and_then(|t| t.as_array())
        {
            for t in arr {
                tools.push(parse_tool_info(t));
            }
        }
    }
    let (in_container, container_name) = parse_ssh_container_probe(docker_out, ss_out, port);
    McpProbe {
        mcp_up: true,
        port_open: true,
        locked: false,
        pid: None,
        host: Some(target.host.to_string()),
        name,
        version,
        protocol,
        sse: false,
        sse_endpoint: false,
        ws: false,
        lang,
        framework,
        in_container,
        container_name,
        tools,
        metadata_reused: false,
        latency_ms: latency,
        error: None,
    }
}

pub fn ssh_probe_server(
    host: &str,
    user: &str,
    port: u16,
    path: &str,
    curl_timeout: u64,
) -> McpProbe {
    ssh_probe_server_target(&SshTarget::simple(host, user), port, path, curl_timeout)
}

/// Informations relevées sur une machine SSH distante lors du premier contact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteEnvInfo {
    pub os: String,
    pub arch: String,
    pub cli_path: Option<String>,
    pub cli_version: Option<String>,
    pub cli_running: bool,
    pub cli_port: Option<u16>,
    pub cli_config_dir: Option<String>,
}

fn normalize_remote_os(raw: &str) -> String {
    let low = raw.trim().to_ascii_lowercase();
    if low.contains("linux") {
        "linux".to_string()
    } else if low.contains("darwin") || low.contains("macos") {
        "macos".to_string()
    } else if low.contains("win") {
        "windows".to_string()
    } else {
        low
    }
}

fn normalize_remote_arch(raw: &str) -> String {
    let low = raw.trim().to_ascii_lowercase();
    match low.as_str() {
        "amd64" | "x86_64" | "x64" => "x86_64".to_string(),
        "arm64" | "aarch64" => "aarch64".to_string(),
        other => other.to_string(),
    }
}

/// Parse la sortie structurée de [`ssh_inspect_remote`].
pub fn parse_remote_env_output(stdout: &str) -> Result<RemoteEnvInfo, String> {
    if !stdout.contains("__MCC_OK__") {
        return Err("réponse d'inspection SSH invalide".to_string());
    }
    let mut os = String::new();
    let mut arch = String::new();
    let mut cli_path = None;
    let mut cli_version = None;
    let mut cli_port = None;
    let mut cli_config_dir = None;
    let mut hello_raw = None;
    for line in stdout.lines() {
        let l = line.trim();
        if let Some(v) = l.strip_prefix("OS=") {
            if !v.trim().is_empty() {
                os = normalize_remote_os(v);
            }
        } else if let Some(v) = l.strip_prefix("ARCH=") {
            if !v.trim().is_empty() {
                arch = normalize_remote_arch(v);
            }
        } else if let Some(v) = l.strip_prefix("CLI=") {
            let t = v.trim();
            if !t.is_empty() {
                cli_path = Some(t.to_string());
            }
        } else if let Some(v) = l.strip_prefix("CFGDIR=") {
            let t = v.trim();
            if !t.is_empty() {
                cli_config_dir = Some(t.to_string());
            }
        } else if let Some(v) = l.strip_prefix("VER=") {
            let t = v.trim();
            if !t.is_empty() {
                cli_version = Some(t.to_string());
            }
        } else if let Some(v) = l.strip_prefix("PORT=") {
            if let Ok(p) = v.trim().parse::<u16>() {
                if p > 0 {
                    cli_port = Some(p);
                }
            }
        } else if let Some(v) = l.strip_prefix("HELLO=") {
            let t = v.trim();
            if !t.is_empty() {
                hello_raw = Some(t.to_string());
            }
        }
    }
    if os.is_empty() {
        return Err("OS distant indéterminable".to_string());
    }
    if arch.is_empty() {
        arch = "unknown".to_string();
    }
    if cli_version.is_none() {
        if let Some(ref h) = hello_raw {
            if let Ok(v) = serde_json::from_str::<Value>(h) {
                cli_version = v
                    .get("version")
                    .and_then(|x| x.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from);
            }
        }
    }
    let cli_running = cli_port.is_some();
    Ok(RemoteEnvInfo {
        os,
        arch,
        cli_path,
        cli_version,
        cli_running,
        cli_port,
        cli_config_dir,
    })
}

/// Inspecte la machine distante en une connexion SSH : OS, arch, présence du
/// binaire `mcp-control`, dossier de config et port actif éventuel (5090/5091).
pub fn ssh_inspect_remote(
    target: &SshTarget<'_>,
    connect_timeout: u64,
) -> Result<RemoteEnvInfo, String> {
    let posix_cmd = "echo __MCC_OK__; \
        if command -v uname >/dev/null 2>&1; then echo OS=$(uname -s); echo ARCH=$(uname -m); fi; \
        CLI=''; \
        for cand in \"$(command -v mcp-control 2>/dev/null || true)\" \"$HOME/.local/share/mcp-control/mcp-control\" \"$HOME/.local/bin/mcp-control\" \"$HOME/.cargo/bin/mcp-control\" /usr/local/bin/mcp-control; do \
          if [ -n \"$cand\" ] && [ -x \"$cand\" ] && [ ! -d \"$cand\" ]; then CLI=\"$cand\"; break; fi; \
        done; \
        echo \"CLI=$CLI\"; \
        CFGDIR=''; \
        if [ -n \"$CLI\" ]; then \
          REAL=$(readlink -f \"$CLI\" 2>/dev/null || readlink \"$CLI\" 2>/dev/null || echo \"$CLI\"); \
          case \"$REAL\" in /*) ;; *) REAL=\"$(dirname \"$CLI\")/$REAL\" ;; esac; \
          CFGDIR=$(dirname \"$REAL\"); \
        fi; \
        if [ -z \"$CFGDIR\" ] && [ -f \"$HOME/.local/share/mcp-control/config.json\" ]; then CFGDIR=\"$HOME/.local/share/mcp-control\"; fi; \
        echo \"CFGDIR=$CFGDIR\"; \
        RUN_PORT=''; HELLO=''; \
        for port in 5090 5091; do \
          H=''; \
          if command -v curl >/dev/null 2>&1; then H=$(curl -fsS -m 1 \"http://127.0.0.1:$port/api/hello\" 2>/dev/null || true); \
          elif command -v wget >/dev/null 2>&1; then H=$(wget -qO- -T 1 \"http://127.0.0.1:$port/api/hello\" 2>/dev/null || true); fi; \
          if [ -n \"$H\" ] && printf '%s' \"$H\" | grep -q '\"mcp-control\"'; then RUN_PORT=\"$port\"; HELLO=\"$H\"; break; fi; \
        done; \
        echo \"PORT=$RUN_PORT\"; echo \"HELLO=$HELLO\"";

    match ssh_exec_target(target, posix_cmd, connect_timeout) {
        Ok(out) if out.contains("__MCC_OK__") && out.contains("OS=") => {
            return parse_remote_env_output(&out);
        }
        Ok(out) if !out.contains("OS=") && out.trim().is_empty() => {}
        Err(e) if e.contains("clé SSH refusée") || e.contains("machine injoignable") => {
            return Err(e);
        }
        _ => {}
    }

    // Repli Windows OpenSSH (cmd.exe / PowerShell)
    let ps_script = "Write-Output '__MCC_OK__'; Write-Output 'OS=windows'; \
        Write-Output ('ARCH=' + $env:PROCESSOR_ARCHITECTURE); \
        $cands = @((Get-Command mcp-control -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source -First 1), (Join-Path $env:LOCALAPPDATA 'mcp-control\\mcp-control.exe')); \
        $cli = ($cands | Where-Object { $_ -and (Test-Path $_ -PathType Leaf) } | Select-Object -First 1); \
        Write-Output ('CLI=' + ($cli -as [string])); \
        $cfg = if ($cli) { Split-Path -Parent $cli } else { Join-Path $env:LOCALAPPDATA 'mcp-control' }; \
        Write-Output ('CFGDIR=' + $cfg); \
        $rport = ''; $hello = ''; \
        foreach ($p in 5090, 5091) { \
          try { $r = Invoke-RestMethod -TimeoutSec 1 -Uri ('http://127.0.0.1:' + $p + '/api/hello'); if ($r.app -eq 'mcp-control') { $rport = [string]$p; $hello = ($r | ConvertTo-Json -Compress); break } } catch {} \
        }; \
        Write-Output ('PORT=' + $rport); Write-Output ('HELLO=' + $hello)";
    let win_cmd = format!(
        "powershell -NoProfile -NonInteractive -Command {}",
        ps_quote(ps_script)?
    );
    let out = ssh_exec_target(target, &win_cmd, connect_timeout)?;
    parse_remote_env_output(&out)
}

pub(crate) fn validate_cli_install_params(url: &str, expected_sha256: &str) -> Result<(), String> {
    const PREFIX: &str = "https://github.com/DeamonDev888/mcp-control/releases/download/";
    if !url.starts_with(PREFIX)
        || url.len() > 512
        || !url
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | ':' | '.' | '-' | '_'))
    {
        return Err("URL de binaire CLI hors périmètre autorisé".to_string());
    }
    if expected_sha256.len() != 64
        || !expected_sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("empreinte SHA-256 attendue invalide".to_string());
    }
    Ok(())
}

/// Installe le binaire CLI `mcp-control` sur la machine distante en vérifiant
/// strictement son empreinte SHA-256 avant de le placer en destination finale.
pub fn ssh_install_cli(
    target: &SshTarget<'_>,
    os: &str,
    url: &str,
    expected_sha256: &str,
) -> Result<String, String> {
    validate_cli_install_params(url, expected_sha256)?;
    let os_norm = normalize_remote_os(os);
    if os_norm == "windows" {
        let ps_script = format!(
            "$ErrorActionPreference = 'Stop'; \
            $dir = Join-Path $env:LOCALAPPDATA 'mcp-control'; \
            New-Item -ItemType Directory -Force -Path $dir | Out-Null; \
            $tmp = Join-Path $dir ('.mcp-control.tmp.' + $PID + '.exe'); \
            $bin = Join-Path $dir 'mcp-control.exe'; \
            Invoke-WebRequest -UseBasicParsing -TimeoutSec 60 -Uri {url_q} -OutFile $tmp; \
            $got = (Get-FileHash $tmp -Algorithm SHA256).Hash.ToLowerInvariant(); \
            if ($got -ne {sha_q}) {{ Remove-Item -Force $tmp -ErrorAction SilentlyContinue; throw ('SHA-256 invalide : ' + $got) }}; \
            Move-Item -Force $tmp $bin; \
            Write-Output ('__MCC_INSTALLED__' + $bin)",
            url_q = ps_quote(url)?,
            sha_q = ps_quote(expected_sha256)?,
        );
        let cmd = format!(
            "powershell -NoProfile -NonInteractive -Command {}",
            ps_quote(&ps_script)?
        );
        let out = ssh_exec_target(target, &cmd, 15)?;
        return out
            .lines()
            .find_map(|l| l.trim().strip_prefix("__MCC_INSTALLED__").map(String::from))
            .ok_or_else(|| "installation CLI Windows distante incomplète".to_string());
    }
    if os_norm != "linux" && os_norm != "macos" {
        return Err(format!("OS distant non pris en charge pour CLI : {os}"));
    }
    let url_q = posix_shell_quote(url)?;
    let sha_q = posix_shell_quote(expected_sha256)?;
    let cmd = format!(
        "set -e; \
        DIR=\"$HOME/.local/share/mcp-control\"; \
        BIN=\"$DIR/mcp-control\"; \
        TMP=\"$DIR/.mcp-control.tmp.$$\"; \
        mkdir -p \"$DIR\" \"$HOME/.local/bin\"; \
        rm -f \"$TMP\"; \
        if command -v curl >/dev/null 2>&1; then curl -fsSL -m 60 -o \"$TMP\" {url_q}; \
        elif command -v wget >/dev/null 2>&1; then wget -q -T 60 -O \"$TMP\" {url_q}; \
        else echo 'curl ou wget requis' >&2; exit 1; fi; \
        if command -v sha256sum >/dev/null 2>&1; then GOT=$(sha256sum \"$TMP\" | awk '{{print $1}}'); \
        elif command -v shasum >/dev/null 2>&1; then GOT=$(shasum -a 256 \"$TMP\" | awk '{{print $1}}'); \
        elif command -v openssl >/dev/null 2>&1; then GOT=$(openssl dgst -sha256 \"$TMP\" | awk '{{print $NF}}'); \
        else rm -f \"$TMP\"; echo 'sha256sum/shasum introuvable' >&2; exit 1; fi; \
        GOT=$(printf '%s' \"$GOT\" | tr 'A-F' 'a-f'); \
        if [ \"$GOT\" != {sha_q} ]; then rm -f \"$TMP\"; echo \"SHA-256 invalide : $GOT\" >&2; exit 1; fi; \
        chmod 700 \"$TMP\"; \
        mv -f \"$TMP\" \"$BIN\"; \
        ln -sf \"$BIN\" \"$HOME/.local/bin/mcp-control\"; \
        echo \"__MCC_INSTALLED__$BIN\""
    );
    let out = ssh_exec_target(target, &cmd, 15)?;
    out.lines()
        .find_map(|l| l.trim().strip_prefix("__MCC_INSTALLED__").map(String::from))
        .ok_or_else(|| "installation CLI distante incomplète".to_string())
}

/// Démarre `mcp-control` en mode détaché sur la machine distante s'il ne répond pas déjà,
/// puis attend qu'il écoute sur `127.0.0.1:5090` ou `5091`.
pub fn ssh_ensure_cli_running(
    target: &SshTarget<'_>,
    os: &str,
    cli_path: &str,
) -> Result<u16, String> {
    let os_norm = normalize_remote_os(os);
    if os_norm == "windows" {
        let cli_q = ps_quote(cli_path)?;
        let ps_script = format!(
            "$p = $null; \
            foreach ($cand in 5090, 5091) {{ try {{ $r = Invoke-RestMethod -TimeoutSec 1 -Uri ('http://127.0.0.1:' + $cand + '/api/hello'); if ($r.app -eq 'mcp-control') {{ $p = $cand; break }} }} catch {{}} }}; \
            if (-not $p) {{ \
              Start-Process -WindowStyle Hidden -FilePath {cli_q} -ArgumentList '--hidden'; \
              for ($i = 0; $i -lt 8; $i++) {{ \
                Start-Sleep -Milliseconds 500; \
                foreach ($cand in 5090, 5091) {{ try {{ $r = Invoke-RestMethod -TimeoutSec 1 -Uri ('http://127.0.0.1:' + $cand + '/api/hello'); if ($r.app -eq 'mcp-control') {{ $p = $cand; break }} }} catch {{}} }}; \
                if ($p) {{ break }} \
              }} \
            }}; \
            if ($p) {{ Write-Output ('__MCC_RUNNING_PORT__' + $p) }} else {{ throw 'mcp-control distant non joignable après lancement' }}"
        );
        let cmd = format!(
            "powershell -NoProfile -NonInteractive -Command {}",
            ps_quote(&ps_script)?
        );
        let out = ssh_exec_target(target, &cmd, 12)?;
        return out
            .lines()
            .find_map(|l| l.trim().strip_prefix("__MCC_RUNNING_PORT__"))
            .and_then(|s| s.parse::<u16>().ok())
            .filter(|&p| p > 0)
            .ok_or_else(|| "mcp-control Windows distant non démarré".to_string());
    }

    let cli_q = posix_shell_quote(cli_path)?;
    let cmd = format!(
        "probe_hello() {{ \
          for port in 5090 5091; do \
            H=''; \
            if command -v curl >/dev/null 2>&1; then H=$(curl -fsS -m 1 \"http://127.0.0.1:$port/api/hello\" 2>/dev/null || true); \
            elif command -v wget >/dev/null 2>&1; then H=$(wget -qO- -T 1 \"http://127.0.0.1:$port/api/hello\" 2>/dev/null || true); fi; \
            if [ -n \"$H\" ] && printf '%s' \"$H\" | grep -q '\"mcp-control\"'; then echo \"$port\"; return 0; fi; \
          done; \
          return 1; \
        }}; \
        P=$(probe_hello || true); \
        if [ -z \"$P\" ]; then \
          nohup {cli_q} </dev/null >/dev/null 2>&1 & \
          i=0; \
          while [ $i -lt 8 ]; do \
            sleep 0.5; \
            P=$(probe_hello || true); \
            if [ -n \"$P\" ]; then break; fi; \
            i=$((i + 1)); \
          done; \
        fi; \
        if [ -n \"$P\" ]; then echo \"__MCC_RUNNING_PORT__$P\"; else echo 'non démarré' >&2; exit 1; fi"
    );
    let out = ssh_exec_target(target, &cmd, 12)?;
    out.lines()
        .find_map(|l| l.trim().strip_prefix("__MCC_RUNNING_PORT__"))
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|&p| p > 0)
        .ok_or_else(|| "mcp-control distant non démarré".to_string())
}

/// Lit l'état assaini du pair `mcp-control` distant via SSH :
/// 1. `mcp-control auth state` (CLI >= 0.8.9) ;
/// 2. repli `state-cache.json` (zéro secret, compatible v0.8.8).
pub fn ssh_read_remote_peer_state(
    target: &SshTarget<'_>,
    os: &str,
    cli_path: Option<&str>,
    cli_config_dir: Option<&str>,
    _cli_port: u16,
) -> Option<Value> {
    let os_norm = normalize_remote_os(os);
    if os_norm == "windows" {
        if let Some(cli) = cli_path {
            if let Ok(cli_q) = ps_quote(cli) {
                let ps = format!("& {cli_q} auth state");
                if let Ok(cmd) = ps_quote(&ps) {
                    if let Ok(out) = ssh_exec_target(
                        target,
                        &format!("powershell -NoProfile -NonInteractive -Command {cmd}"),
                        8,
                    ) {
                        if let Ok(v) = serde_json::from_str::<Value>(out.trim()) {
                            if v.get("servers").is_some() && v.get("stats").is_some() {
                                return Some(v);
                            }
                        }
                    }
                }
            }
        }
        if let Some(cfg) = cli_config_dir {
            if let Ok(cache_q) = ps_quote(&format!("{cfg}\\state-cache.json")) {
                let ps = format!("if (Test-Path {cache_q}) {{ Get-Content -Raw {cache_q} }}");
                if let Ok(cmd) = ps_quote(&ps) {
                    if let Ok(out) = ssh_exec_target(
                        target,
                        &format!("powershell -NoProfile -NonInteractive -Command {cmd}"),
                        8,
                    ) {
                        if let Ok(v) = serde_json::from_str::<Value>(out.trim()) {
                            if let Some(arr) = v.get("servers").and_then(|x| x.as_array()) {
                                let up = arr
                                    .iter()
                                    .filter(|s| {
                                        s.get("state").and_then(|x| x.as_str()) == Some("UP")
                                    })
                                    .count();
                                return Some(json!({
                                    "servers": arr,
                                    "stats": {"total": arr.len(), "up": up}
                                }));
                            }
                        }
                    }
                }
            }
        }
        return None;
    }

    if let Some(cli) = cli_path {
        if let Ok(cli_q) = posix_shell_quote(cli) {
            if let Ok(out) = ssh_exec_target(target, &format!("{cli_q} auth state 2>/dev/null"), 8)
            {
                if let Ok(v) = serde_json::from_str::<Value>(out.trim()) {
                    if v.get("servers").is_some() && v.get("stats").is_some() {
                        return Some(v);
                    }
                }
            }
        }
    }

    let mut cache_candidates: Vec<String> = Vec::new();
    if let Some(cfg) = cli_config_dir.map(str::trim).filter(|s| !s.is_empty()) {
        cache_candidates.push(format!("{}/state-cache.json", cfg.trim_end_matches('/')));
    }
    for cand in cache_candidates {
        if let Ok(cand_q) = posix_shell_quote(&cand) {
            if let Ok(out) = ssh_exec_target(target, &format!("cat {cand_q} 2>/dev/null"), 8) {
                if let Ok(v) = serde_json::from_str::<Value>(out.trim()) {
                    if let Some(arr) = v.get("servers").and_then(|x| x.as_array()) {
                        let up = arr
                            .iter()
                            .filter(|s| s.get("state").and_then(|x| x.as_str()) == Some("UP"))
                            .count();
                        return Some(json!({
                            "servers": arr,
                            "stats": {"total": arr.len(), "up": up}
                        }));
                    }
                }
            }
        }
    }
    None
}

/// Appelle un outil (`tools/call`) sur un serveur MCP distant via SSH :
/// le payload JSON-RPC est envoyé exclusivement sur **stdin** (`--data-binary @-`),
/// sans jamais interpoler les arguments dans le shell distant.
pub fn ssh_call_tool(
    target: &SshTarget<'_>,
    port: u16,
    path: &str,
    protocol: Option<&str>,
    tool: &str,
    arguments: Value,
    timeout_secs: u64,
) -> Result<(Value, u64), String> {
    if port == 0 {
        return Err("port invalide pour appel SSH".to_string());
    }
    let t0 = Instant::now();
    let safe_path = mcp_path_safe(path);
    let proto = protocol
        .map(str::trim)
        .filter(|p| {
            !p.is_empty()
                && p.len() <= 32
                && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        .unwrap_or("2024-11-05");
    let init_payload = mcp_remote_payload("initialize", 1);
    let tmp = "/tmp/.mcc_call_$$";
    let curl_t = timeout_secs.clamp(2, 60);
    let init_curl = format!(
        "curl -s -m {t} -X POST 'http://127.0.0.1:{port}{safe_path}' \
         -H 'Content-Type: application/json' \
         -H 'Accept: application/json, text/event-stream' \
         -H 'MCP-Protocol-Version: {proto}' \
         -d '{init_payload}'",
        t = curl_t.min(10),
    );
    let call_curl = format!(
        "curl -s -m {curl_t} -X POST 'http://127.0.0.1:{port}{safe_path}' \
         -H 'Content-Type: application/json' \
         -H 'Accept: application/json, text/event-stream' \
         -H 'MCP-Protocol-Version: {proto}' \
         -H 'Mcp-Session-Id:'$sid \
         --data-binary @-"
    );
    let remote = format!(
        "{init_curl} -D {tmp} </dev/null >/dev/null; \
         sid=$(grep -i mcp-session-id {tmp} | head -n 1 | tr -d '\\r' | sed 's/^[^:]*: *//' | tr -cd 'A-Za-z0-9-_' | head -c 100); \
         rm -f {tmp}; echo __MCC_CALL_SEP__; {call_curl}; echo"
    );
    let call_body = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": tool,
            "arguments": arguments,
        }
    })
    .to_string();

    let out = ssh_exec_stdin(target, &remote, call_body.as_bytes(), 8, curl_t + 5)?;
    let elapsed = t0.elapsed().as_millis() as u64;
    let body = out
        .split_once("__MCC_CALL_SEP__")
        .map(|(_, after)| after)
        .unwrap_or(out.as_str());
    let v = extract_json(body.as_bytes())
        .ok_or_else(|| format!("via ssh : réponse non JSON ({} octets)", body.len()))?;
    if let Some(err) = v.get("error") {
        return Err(format!("rpc error: {err}"));
    }
    let res = v.get("result").cloned().unwrap_or(Value::Null);
    Ok((res, elapsed))
}

/// Démarre un processus détaché sur la machine distante via SSH.
/// Tous les arguments, variables d'env et chemins sont strictement validés et quotés.
pub fn ssh_spawn_remote(
    target: &SshTarget<'_>,
    os_hint: Option<&str>,
    cmd: &crate::types::StartCmd,
) -> Result<Option<u32>, String> {
    if cmd.program.trim().is_empty() {
        return Err("programme de lancement vide".to_string());
    }
    if os_hint.is_some_and(|o| normalize_remote_os(o) == "windows") {
        let prog_q = ps_quote(cmd.program.trim())?;
        let mut arg_items = Vec::with_capacity(cmd.args.len());
        for a in &cmd.args {
            arg_items.push(ps_quote(a)?);
        }
        let mut env_cmds = String::new();
        for (k, v) in &cmd.env {
            if !env_key_safe(k) {
                return Err(format!("nom de variable d'environnement invalide : {k}"));
            }
            env_cmds.push_str(&format!("$env:{k} = {}; ", ps_quote(v)?));
        }
        let wd_part = match cmd
            .workdir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(wd) => format!(" -WorkingDirectory {}", ps_quote(wd)?),
            None => String::new(),
        };
        let args_part = if arg_items.is_empty() {
            String::new()
        } else {
            format!(" -ArgumentList @({})", arg_items.join(", "))
        };
        let ps_script = format!(
            "{env_cmds}$p = Start-Process -WindowStyle Hidden -PassThru -FilePath {prog_q}{args_part}{wd_part}; \
             Write-Output ('__MCC_PID_' + $p.Id + '__')"
        );
        let win_cmd = format!(
            "powershell -NoProfile -NonInteractive -Command {}",
            ps_quote(&ps_script)?
        );
        let out = ssh_exec_target(target, &win_cmd, 8)?;
        return Ok(parse_remote_pid(&out));
    }

    let mut env_prefix = String::new();
    for (k, v) in &cmd.env {
        if !env_key_safe(k) {
            return Err(format!("nom de variable d'environnement invalide : {k}"));
        }
        env_prefix.push_str(&format!("{k}={} ", posix_shell_quote(v)?));
    }
    let cd_part = match cmd
        .workdir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(wd) => format!("cd {} || exit 1; ", posix_shell_quote(wd)?),
        None => String::new(),
    };
    let mut parts = Vec::with_capacity(1 + cmd.args.len());
    parts.push(posix_shell_quote(cmd.program.trim())?);
    for a in &cmd.args {
        parts.push(posix_shell_quote(a)?);
    }
    let remote = format!(
        "{cd_part}{env_prefix}nohup {} </dev/null >/dev/null 2>&1 & echo __MCC_PID_$!__",
        parts.join(" ")
    );
    let out = ssh_exec_target(target, &remote, 8)?;
    Ok(parse_remote_pid(&out))
}

fn parse_remote_pid(out: &str) -> Option<u32> {
    let (_, rest) = out.split_once("__MCC_PID_")?;
    let (digits, _) = rest.split_once("__")?;
    digits.trim().parse::<u32>().ok().filter(|&p| p > 0)
}

/// Parse les PID et noms de processus écoutant sur `port` depuis `ss -tlnp`,
/// `lsof`, `netstat -tlnp` ou `netstat -ano`.
pub fn parse_remote_port_listeners(text: &str, port: u16) -> Vec<(u32, String)> {
    let mut out: Vec<(u32, String)> = Vec::new();
    let suffix_colon = format!(":{port}");
    let suffix_dot = format!(".{port}");
    for line in text.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.is_empty() {
            continue;
        }
        // Windows netstat -ano : TCP 0.0.0.0:3000 0.0.0.0:0 LISTENING 1234
        if toks.len() >= 5 && toks.contains(&"LISTENING") && toks[1].ends_with(&suffix_colon) {
            if let Ok(pid) = toks.last().unwrap_or(&"").parse::<u32>() {
                if pid > 0 && !out.iter().any(|(p, _)| *p == pid) {
                    out.push((pid, "remote".to_string()));
                }
            }
            continue;
        }
        // lsof -nP -iTCP:<port> -sTCP:LISTEN : node 1234 user ... TCP *:3000 (LISTEN)
        if toks.len() >= 9 && line.contains("(LISTEN)") && line.contains(&suffix_colon) {
            if let Ok(pid) = toks[1].parse::<u32>() {
                if pid > 0 && !out.iter().any(|(p, _)| *p == pid) {
                    out.push((pid, toks[0].to_string()));
                }
            }
            continue;
        }
        // ss -tlnp : LISTEN 0 128 0.0.0.0:3000 ... users:(("node",pid=1234,fd=18))
        let matches_port = toks
            .iter()
            .any(|t| t.ends_with(&suffix_colon) || t.ends_with(&suffix_dot));
        if !matches_port {
            continue;
        }
        if let Some(pos) = line.find("pid=") {
            let after = &line[pos + 4..];
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(pid) = digits.parse::<u32>() {
                let proc_name = line[..pos]
                    .rsplit('"')
                    .nth(1)
                    .unwrap_or("remote")
                    .to_string();
                if pid > 0 && !out.iter().any(|(p, _)| *p == pid) {
                    out.push((pid, proc_name));
                }
                continue;
            }
        }
        // netstat -tlnp : tcp 0 0 0.0.0.0:3000 0.0.0.0:* LISTEN 1234/node
        if toks.contains(&"LISTEN") {
            if let Some(last) = toks.last() {
                if let Some((pid_s, name)) = last.split_once('/') {
                    if let Ok(pid) = pid_s.parse::<u32>() {
                        if pid > 0 && !out.iter().any(|(p, _)| *p == pid) {
                            out.push((pid, name.to_string()));
                        }
                    }
                }
            }
        }
    }
    out
}

/// Liste `(pid, process)` écoutant sur `port` sur la machine distante SSH.
pub fn ssh_port_listeners(
    target: &SshTarget<'_>,
    os_hint: Option<&str>,
    port: u16,
) -> Vec<(u32, String)> {
    if port == 0 {
        return Vec::new();
    }
    let remote = if os_hint.is_some_and(|o| normalize_remote_os(o) == "windows") {
        "netstat -ano | findstr LISTENING".to_string()
    } else {
        format!(
            "ss -tlnp 2>/dev/null || lsof -nP -iTCP:{port} -sTCP:LISTEN 2>/dev/null || netstat -tlnp 2>/dev/null || true"
        )
    };
    match ssh_exec_target(target, &remote, 8) {
        Ok(out) => parse_remote_port_listeners(&out, port),
        Err(_) => Vec::new(),
    }
}

/// Termine une liste de PIDs sur la machine distante SSH (`force` = SIGKILL / `/F`).
pub fn ssh_kill_pids(
    target: &SshTarget<'_>,
    os_hint: Option<&str>,
    pids: &[u32],
    force: bool,
) -> Result<(), String> {
    let valid: Vec<u32> = pids.iter().copied().filter(|&p| p > 0).collect();
    if valid.is_empty() {
        return Ok(());
    }
    let remote = if os_hint.is_some_and(|o| normalize_remote_os(o) == "windows") {
        let mut args = String::from("taskkill /T");
        if force {
            args.push_str(" /F");
        }
        for p in &valid {
            args.push_str(&format!(" /PID {p}"));
        }
        args
    } else {
        let sig = if force { "-KILL" } else { "-TERM" };
        let list = valid
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        format!("kill {sig} {list} 2>/dev/null || true")
    };
    ssh_exec_target(target, &remote, 8).map(|_| ())
}

/// Vrai si l'erreur de [`http_request`] correspond à un hôte qui n'a JAMAIS
/// accepté la connexion (« addr … » ou échec de connect — forme
/// « {host}: {erreur io} »). Les erreurs POST-connexion ont toutes une forme
/// distincte et prévisible : « {host} write: … », « {host} read: … »,
/// « {host}: réponse vide », « {host}: réponse HTTP non-2xx … »,
/// « {host}: réponse trop grande … ». Utilisée par [`http_post`] pour publier
/// l'erreur de l'hôte qui a accepté (même correctif que probe_http,
/// audit 03 constat 4, reporté ici au lot A).
fn connexion_manquee(err: &str, host: &str) -> bool {
    if err.starts_with("addr ") {
        return true;
    }
    let pfx = format!("{host}: ");
    if !err.starts_with(&pfx) {
        // « write: » / « read: » : la connexion était établie
        return false;
    }
    let reste = &err[pfx.len()..];
    !(reste.starts_with("réponse vide")
        || reste.starts_with("réponse HTTP non-2xx")
        || reste.starts_with("réponse trop grande"))
}

/// Vrai si l'erreur de [`http_request`] signifie « aucune réponse » : la
/// connexion a été ACCEPTÉE mais aucun octet n'est arrivé avant l'échéance
/// (« {host} read: … » — le motif n'est publié que si `data` est vide) ou
/// l'hôte a refermé sans rien écrire (« {host}: réponse vide »). C'est le
/// signal d'un port MUET (lot A) : probe_http cesse alors de rejouer les
/// formes suivantes. Tout le reste (write, non-2xx, trop grande, JSON
/// invalide…) prouve une activité du serveur et justifie les autres formes.
fn zero_octets(err: &str, host: &str) -> bool {
    err.starts_with(&format!("{host} read: ")) || err.starts_with(&format!("{host}: réponse vide"))
}

/// POST JSON vers localhost (essaye ::1 puis 127.0.0.1), lit toute la réponse (gère chunked).
/// Renvoie (host_utilisé, corps, transport SSE détecté).
pub fn http_post(
    port: u16,
    path: &str,
    body: &str,
    timeout_ms: u64,
) -> Result<(String, Vec<u8>, RespHead), String> {
    // MOTIF D'ERREUR (lot A — même correctif que probe_http) : l'erreur de
    // l'hôte de repli (::1 ou 127.0.0.1) qui n'a jamais accepté la connexion
    // ne doit pas écraser celle de l'hôte qui a accepté (délai dépassé,
    // réponse invalide…). On publie en priorité la 1re erreur post-connexion.
    let mut err_accept: Option<String> = None;
    let mut err_dernier = String::new();
    for host in ["::1", "127.0.0.1"] {
        match http_post_host(host, port, path, body, timeout_ms, &[]) {
            Ok((b, h)) => return Ok((host.to_string(), b, h)),
            Err(e) => {
                if !connexion_manquee(&e, host) && err_accept.is_none() {
                    err_accept = Some(e);
                } else {
                    err_dernier = e;
                }
            }
        }
    }
    Err(match err_accept {
        Some(e) => e,
        None => err_dernier,
    })
}

/// Cap mémoire d'une réponse de sonde : un serveur LAN rapide ne doit pas pouvoir
/// gonfler la RAM pendant la fenêtre de timeout (audit 2026-10-24 — le parseur
/// d'entrée HTTP était capé, pas le client).
const MAX_PROBE_READ: usize = 2 * 1024 * 1024;
/// Cap des sondes qui ne lisent que le début d'un flux (SSE, /health).
const MAX_PROBE_READ_SMALL: usize = 256 * 1024;

/// POST JSON vers un hôte précis (localhost OU hôte du réseau local).
/// `extra_headers` : en-têtes additionnels (preuve de pair pour /api/state).
pub fn http_post_host(
    host: &str,
    port: u16,
    path: &str,
    body: &str,
    timeout_ms: u64,
    extra_headers: &[(&str, &str)],
) -> Result<(Vec<u8>, RespHead), String> {
    http_request(
        "POST",
        host,
        port,
        path,
        Some(body),
        timeout_ms,
        extra_headers,
    )
}

/// GET HTTP vers un hôte précis — préauthentification des pairs (/api/hello).
pub fn http_get_host(
    host: &str,
    port: u16,
    path: &str,
    timeout_ms: u64,
) -> Result<(Vec<u8>, RespHead), String> {
    http_request("GET", host, port, path, None, timeout_ms, &[])
}

/// POST/PUT HTTP générique (webhook de notification) : pas d'en-tête MCP,
/// pas de negotiated protocol — un simple appel HTTP avec un corps JSON.
/// `extra_headers` : paires (nom, valeur) ajoutées à la requête.
pub fn http_post_raw(
    host: &str,
    port: u16,
    path: &str,
    body: &str,
    extra_headers: &[(&str, &str)],
    method: &str,
    timeout_ms: u64,
) -> Result<(), String> {
    let (bytes, _head) = http_request_trusted(
        method,
        host,
        port,
        path,
        Some(body),
        timeout_ms,
        extra_headers,
    )?;
    // code HTTP : 2e token de la ligne de statut ("HTTP/1.1 200 OK" → 200).
    let status_line = String::from_utf8_lossy(&bytes)
        .lines()
        .next()
        .unwrap_or("")
        .to_string();
    let code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
        .unwrap_or(0);
    if (200..400).contains(&code) {
        Ok(())
    } else {
        Err(format!("HTTP {code} ({status_line})"))
    }
}
/// Variante de [`http_request`] pour les en-têtes de CONFIANCE (webhook
/// configuré par l'opérateur) : la valeur peut contenir des espaces
/// (« Authorization: Bearer xxx ») — on exige seulement l'absence de
/// CR/LF et une longueur bornée (le request splitting reste impossible).
fn http_request_trusted(
    method: &str,
    host: &str,
    port: u16,
    path: &str,
    body: Option<&str>,
    timeout_ms: u64,
    trusted_headers: &[(&str, &str)],
) -> Result<(Vec<u8>, RespHead), String> {
    let timeout = Duration::from_millis(timeout_ms);
    let addr: SocketAddr = match socket_addr(host, port) {
        Ok(a) => a,
        Err(e) => return Err(format!("addr {}: {}", host, e)),
    };
    let mut last_err = String::new();
    match TcpStream::connect_timeout(&addr, timeout) {
        Ok(mut stream) => {
            stream.set_read_timeout(Some(timeout)).ok();
            stream.set_write_timeout(Some(timeout)).ok();
            let host_hdr = http_host_header(host, port);
            let path = sanitize_http_path(path, "/");
            let mut req = format!("{} {} HTTP/1.1\r\nHost: {}\r\n", method, path, host_hdr);
            for (n, v) in trusted_headers {
                // SÉCURITÉ : un CR/LF Allow-Injection d'en-têtes (request
                // splitting) reste interdit — seuls les espaces et la
                // ponctuation sont tolérés.
                if v.is_empty() || v.len() > 512 || v.contains('\r') || v.contains('\n') {
                    continue;
                }
                if n.contains('\r') || n.contains('\n') || n.contains(':') {
                    continue;
                }
                req.push_str(&format!("{}: {}\r\n", n, v));
            }
            if body.is_some() {
                req.push_str("Content-Type: application/json\r\n");
            }
            match body {
                Some(b) => req.push_str(&format!(
                    "Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    b.len(),
                    b
                )),
                None => req.push_str("Connection: close\r\n\r\n"),
            }
            if let Err(e) = stream.write_all(req.as_bytes()) {
                last_err = format!("écriture: {e}");
            } else {
                let mut data = Vec::new();
                let mut tmp = [0u8; 16384];
                let deadline = Instant::now() + timeout;
                let mut header_len: Option<usize> = None;
                let mut status_ok = false;
                loop {
                    let remain = deadline.saturating_duration_since(Instant::now());
                    if remain.is_zero() {
                        break;
                    }
                    if header_len.is_none() {
                        if let Some(p) = find_sub(&data, b"\r\n\r\n") {
                            header_len = Some(p + 4);
                            let head = String::from_utf8_lossy(&data[..p]).to_lowercase();
                            status_ok =
                                head.starts_with("http/1.1 2") || head.starts_with("http/1.0 2");
                        }
                    }
                    if let Some(hl) = header_len {
                        let head = String::from_utf8_lossy(&data[..hl]).to_lowercase();
                        let body_bytes = &data[hl..];
                        let complete = if head.contains("transfer-encoding: chunked") {
                            find_sub(body_bytes, b"\r\n0\r\n").is_some()
                        } else if let Some(cl) = head.lines().find_map(|l| {
                            l.strip_prefix("content-length:")
                                .map(|v| v.trim().to_string())
                        }) {
                            body_bytes.len() >= cl.parse::<usize>().unwrap_or(usize::MAX)
                        } else {
                            false
                        };
                        if complete {
                            break;
                        }
                    }
                    match stream.read(&mut tmp) {
                        Ok(0) => break,
                        Ok(n) => data.extend_from_slice(&tmp[..n]),
                        Err(e) => {
                            last_err = format!("lecture: {e}");
                            break;
                        }
                    }
                }
                let head_end = find_sub(&data, b"\r\n\r\n").map(|p| p + 4).unwrap_or(0);
                let head = String::from_utf8_lossy(&data[..head_end]).to_string();
                let mut content_type = String::new();
                let mut server = String::new();
                let mut powered_by = String::new();
                let mut session_id = String::new();
                for line in head.lines() {
                    let lower = line.to_lowercase();
                    if let Some(v) = lower.strip_prefix("content-type:") {
                        content_type = v.trim().to_string();
                    } else if let Some(v) = lower.strip_prefix("server:") {
                        server = v.trim().to_string();
                    } else if let Some(v) = lower.strip_prefix("x-powered-by:") {
                        powered_by = v.trim().to_string();
                    } else if let Some(v) = lower.strip_prefix("mcp-session-id:") {
                        session_id = v.trim().to_string();
                    }
                }
                if status_ok {
                    return Ok((
                        data,
                        RespHead {
                            content_type,
                            server,
                            powered_by,
                            session_id,
                        },
                    ));
                }
                // une erreur d'E/S diagnostiquée (« lecture: … ») est plus
                // parlante qu'un « réponse vide » générique : on la garde
                if last_err.is_empty() {
                    last_err = if data.is_empty() {
                        "réponse vide".to_string()
                    } else {
                        format!(
                            "HTTP non 2xx : {}",
                            String::from_utf8_lossy(&data).lines().next().unwrap_or("?")
                        )
                    };
                }
            }
        }
        Err(e) => last_err = format!("{e}"),
    }
    Err(last_err)
}
/// Extrait la méthode JSON-RPC et éventuellement `params.name` / `params.uri`
/// d'un corps de requête JSON-RPC sortant pour aligner les en-têtes
/// `Mcp-Method` / `Mcp-Name` avant signature RFC 9421.
fn extract_rpc_meta(body: &str) -> (Option<String>, Option<String>) {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return (None, None);
    };
    let m = v
        .get("method")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    let name = match m.as_deref() {
        Some("tools/call" | "prompts/get") => v
            .get("params")
            .and_then(|p| p.get("name"))
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from),
        Some("resources/read") => v
            .get("params")
            .and_then(|p| p.get("uri"))
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from),
        _ => None,
    };
    (m, name)
}

fn http_request(
    method: &str,
    host: &str,
    port: u16,
    path: &str,
    body: Option<&str>,
    timeout_ms: u64,
    extra_headers: &[(&str, &str)],
) -> Result<(Vec<u8>, RespHead), String> {
    let timeout = Duration::from_millis(timeout_ms);
    let mut last_err = String::new();
    let outbound = if path != "/api/state" && path != "/api/hello" {
        crate::auth::net::outbound_identity_for(host, port)
            .map_err(|e| format!("{}: {}", host, e))?
    } else {
        crate::auth::net::outbound_identity_for(host, port)
            .ok()
            .flatten()
    };

    // Host exact (le "localhost" historique casse les vhosts sur hôte distant)
    let host_hdr = http_host_header(host, port);
    // path sans CR/LF ni espace : il est placé tel quel dans la ligne de
    // requête — un path configuré ne doit pas pouvoir injecter d'en-têtes
    let path = sanitize_http_path(path, "/mcp");
    let mut req = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nAccept: application/json, text/event-stream\r\n",
        method, path, host_hdr
    );
    let mut req_headers: Vec<(String, String)> = Vec::new();
    if body.is_some() {
        req.push_str("Content-Type: application/json\r\n");
        req_headers.push(("Content-Type".to_string(), "application/json".to_string()));
        // Le protocole par défaut est 2024-11-05. Un appelant qui
        // sonde le protocole stateless fournit SON en-tête
        // (2026-07-28) : envoyer les deux valeurs dans la même
        // requête fait répondre 400 aux serveurs stricts.
        let has_proto = extra_headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("MCP-Protocol-Version"));
        if !has_proto {
            req.push_str("MCP-Protocol-Version: 2024-11-05\r\n");
            req_headers.push(("MCP-Protocol-Version".to_string(), "2024-11-05".to_string()));
        }
    }
    for (n, v) in extra_headers {
        // SÉCURITÉ (audit 2026-10-25 + lot M4) : la valeur ne doit contenir
        // ni CR/LF ni caractère de contrôle (request splitting impossible) ;
        // les caractères ASCII imprimables (0x20..=0x7E) sont autorisés pour
        // supporter les en-têtes RFC 9421 (`Signature-Input`, `Signature`).
        let name_ok = !n.is_empty()
            && n.len() <= 128
            && n.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        let val_ok =
            !v.is_empty() && v.len() <= 1024 && v.bytes().all(|b| (0x20..=0x7E).contains(&b));
        if name_ok && val_ok {
            req.push_str(&format!("{}: {}\r\n", n, v));
            req_headers.push(((*n).to_string(), (*v).to_string()));
        }
    }
    if let (Some(ident), Some(b)) = (outbound.as_ref(), body) {
        if method.eq_ignore_ascii_case("POST") && path != "/api/state" {
            let (rpc_method, rpc_name) = extract_rpc_meta(b);
            if let Some(m) = rpc_method {
                let has_method = req_headers
                    .iter()
                    .any(|(k, _)| k.eq_ignore_ascii_case("Mcp-Method"));
                if !has_method && m.bytes().all(|c| (0x21..=0x7E).contains(&c)) {
                    req.push_str(&format!("Mcp-Method: {}\r\n", m));
                    req_headers.push(("Mcp-Method".to_string(), m));
                }
            }
            if let Some(nm) = rpc_name {
                let has_name = req_headers
                    .iter()
                    .any(|(k, _)| k.eq_ignore_ascii_case("Mcp-Name"));
                if !has_name && nm.bytes().all(|c| (0x20..=0x7E).contains(&c)) {
                    req.push_str(&format!("Mcp-Name: {}\r\n", nm));
                    req_headers.push(("Mcp-Name".to_string(), nm));
                }
            }
            let hdr_refs: Vec<(&str, &str)> = req_headers
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            for (k, v) in ident.sign_headers("POST", path, &host_hdr, &hdr_refs, b.as_bytes()) {
                req.push_str(&format!("{}: {}\r\n", k, v));
            }
        }
    }
    match body {
        Some(b) => req.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n{}",
            b.len(),
            b
        )),
        None => req.push_str("Connection: close\r\n\r\n"),
    }

    let mut data: Vec<u8> = Vec::new();
    let mut header_len: Option<usize> = None;
    let mut status_ok = false;

    if let Some(ident) = outbound.as_ref().filter(|id| !id.tls_fp.is_empty()) {
        let (tls_data, _) = crate::auth::net::https_exchange_raw(
            host,
            port,
            req.as_bytes(),
            timeout,
            Some(&ident.tls_fp),
            MAX_PROBE_READ,
        )?;
        data = tls_data;
        if let Some(p) = find_sub(&data, b"\r\n\r\n") {
            header_len = Some(p + 4);
            let head = String::from_utf8_lossy(&data[..p]).to_lowercase();
            status_ok = head.starts_with("http/1.1 2") || head.starts_with("http/1.0 2");
        }
    } else {
        let addr: SocketAddr = match socket_addr(host, port) {
            Ok(a) => a,
            Err(e) => return Err(format!("addr {}: {}", host, e)),
        };
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(mut stream) => {
                stream.set_read_timeout(Some(timeout)).ok();
                stream.set_write_timeout(Some(timeout)).ok();
                if let Err(e) = stream.write_all(req.as_bytes()) {
                    return Err(format!("{} write: {}", host, e));
                }
                let mut tmp = [0u8; 16384];
                let deadline = Instant::now() + timeout;
                // lit l'en-tête puis s'arrête dès que le corps est complet (Content-Length / 0-chunk)
                loop {
                    let remain = deadline.saturating_duration_since(Instant::now());
                    if remain.is_zero() {
                        break;
                    }
                    if header_len.is_none() {
                        if let Some(p) = find_sub(&data, b"\r\n\r\n") {
                            header_len = Some(p + 4);
                            let head = String::from_utf8_lossy(&data[..p]).to_lowercase();
                            status_ok =
                                head.starts_with("http/1.1 2") || head.starts_with("http/1.0 2");
                        }
                    }
                    if let Some(hl) = header_len {
                        let head = String::from_utf8_lossy(&data[..hl]).to_lowercase();
                        let body_bytes = &data[hl..];
                        let complete = if head.contains("transfer-encoding: chunked") {
                            find_sub(body_bytes, b"\r\n0\r\n").is_some()
                        } else if let Some(cl) = head.lines().find_map(|l| {
                            l.strip_prefix("content-length:")
                                .map(|v| v.trim().to_string())
                        }) {
                            body_bytes.len() >= cl.parse::<usize>().unwrap_or(usize::MAX)
                        } else {
                            false
                        };
                        if complete {
                            break;
                        }
                    }
                    stream.set_read_timeout(Some(remain)).ok();
                    match stream.read(&mut tmp) {
                        Ok(0) => break,
                        Ok(n) => {
                            data.extend_from_slice(&tmp[..n]);
                            if data.len() > MAX_PROBE_READ {
                                break; // cap mémoire, le parse des octets reçus décidera
                            }
                        }
                        Err(e) => {
                            if data.is_empty() {
                                last_err = format!("{} read: {}", host, e);
                            }
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                return Err(format!("{}: {}", host, e));
            }
        }
    }

    if data.is_empty() {
        if last_err.is_empty() {
            last_err = format!("{}: réponse vide", host);
        }
        return Err(last_err);
    }
    if data.len() > MAX_PROBE_READ {
        // Dépassement explicite (audit 02, constat 6) : au lieu de
        // laisser le parse conclure « réponse non JSON (N octets) »
        // sur un corps tronqué, on nomme la vraie cause : la lecture
        // a été coupée au plafond mémoire.
        return Err(format!(
            "{}: réponse trop grande (plafond de lecture de {} Mio dépassé)",
            host,
            MAX_PROBE_READ / (1024 * 1024)
        ));
    }
    if header_len.is_some() && !status_ok {
        let sep = find_sub(&data, b"\r\n\r\n").unwrap_or(0);
        let raw_head = String::from_utf8_lossy(&data[..sep]);
        let first_line = raw_head.lines().next().unwrap_or("");
        let status_code = first_line
            .split_whitespace()
            .nth(1)
            .and_then(|c| c.parse::<u16>().ok())
            .unwrap_or(0);
        let www_auth = raw_head
            .lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                if k.trim().eq_ignore_ascii_case("www-authenticate") {
                    Some(v.trim().to_string())
                } else {
                    None
                }
            })
            .unwrap_or_default();
        if status_code == 401 && crate::auth::net::is_litemcp_auth_challenge(&www_auth) {
            return Err(format!("{}: {}", host, crate::auth::net::LOCKED_SERVER_MSG));
        }
        return Err(format!(
            "{}: réponse HTTP non-2xx (port occupé par un non-MCP ?)",
            host
        ));
    }
    // ROBUSTESSE (audit 2026-10-25) : une réponse sans séparateur
    // CRLFCRLF (serveur qui répond 1-3 octets puis ferme) faisait
    // panicer sur `data[sep + 4..]` (sep=0, len<4) → le thread du
    // POLLER mourait en silence (fenêtre desktop : aucune console)
    // et la supervision des serveurs s'arrêtait définitivement.
    // On borne l'extraction du corps au contenu réellement reçu.
    let sep = find_sub(&data, b"\r\n\r\n").unwrap_or(0);
    let raw_head = String::from_utf8_lossy(&data[..sep]);
    let head = raw_head.to_lowercase();
    let header = |name: &str| -> String {
        head.lines()
            .find_map(|l| l.strip_prefix(name))
            .map(|v| v.trim().to_string())
            .unwrap_or_default()
    };
    // Mcp-Session-Id : nom d'en-tête insensible à la casse, mais la
    // VALEUR du jeton de session doit conserver sa casse exacte.
    let session_id = raw_head
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            if k.trim().eq_ignore_ascii_case("mcp-session-id") {
                Some(v.trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default();
    let resp_head = RespHead {
        content_type: header("content-type:"),
        server: header("server:"),
        powered_by: header("x-powered-by:"),
        session_id,
    };
    let body_start = sep.saturating_add(4).min(data.len());
    let mut body_bytes = data[body_start..].to_vec();
    if head.contains("transfer-encoding: chunked") {
        body_bytes = dechunk(&body_bytes);
    }
    Ok((body_bytes, resp_head))
}

fn dechunk(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < data.len() {
        // ROBUSTESSE (audit 2026-10-25) : deux panics sur réponse chunked
        // tronquée ou à taille absurde —
        //  (1) `&data[i..]` quand i > len (slice hors bornes) ;
        //  (2) `start + size` qui déborde (usize overflow sur un size héxa
        //      énorme, ex. ffffffffffffffff).
        // Les deux tuaient le thread du POLLER en silence (fenêtre desktop :
        // aucune console) et arrêtaient la supervision des serveurs.
        let Some(p) = find_sub(&data[i..], b"\r\n") else {
            break;
        };
        let line_end = i + p;
        let size_str = String::from_utf8_lossy(&data[i..line_end]);
        let size = usize::from_str_radix(size_str.trim().split(';').next().unwrap_or("0"), 16)
            .unwrap_or(0);
        if size == 0 {
            break;
        }
        let start = line_end + 2;
        // saturating_add : jamais de débordement, et la borne `end > data.len()`
        // rejette proprement un chunk tronqué.
        let end = start.saturating_add(size);
        if start > data.len() || end > data.len() {
            break;
        }
        out.extend_from_slice(&data[start..end]);
        // après le chunk, on saute le CRLF final ; s'il manque, on s'arrête
        let next = end.saturating_add(2);
        if next > data.len() {
            break;
        }
        i = next;
    }
    out
}

/// Extrait le JSON-RPC d'une réponse (JSON brut ou flux SSE avec lignes "data:").
pub fn extract_json(body: &[u8]) -> Option<Value> {
    let text = String::from_utf8_lossy(body);
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("data:") {
            let rest = rest.trim();
            if let Ok(v) = serde_json::from_str::<Value>(rest) {
                if v.get("result").is_some() || v.get("error").is_some() {
                    return Some(v);
                }
            }
        }
    }
    if let Some(start) = text.find('{') {
        if let Ok(v) = serde_json::from_str::<Value>(text[start..].trim()) {
            return Some(v);
        }
        for line in text.lines() {
            let t = line.trim();
            if t.starts_with('{') {
                if let Ok(v) = serde_json::from_str::<Value>(t) {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// Un appel JSON-RPC HTTP complet (POST + parse).
pub fn rpc_call(
    port: u16,
    path: &str,
    method: &str,
    params: Value,
    id: u64,
    timeout_ms: u64,
) -> Result<(Value, String, RespHead), String> {
    let body = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
    let (host, bytes, head) = http_post(port, path, &body, timeout_ms)?;
    let v =
        extract_json(&bytes).ok_or_else(|| format!("réponse non JSON ({} octets)", bytes.len()))?;
    if let Some(err) = v.get("error") {
        return Err(format!("rpc error: {}", err));
    }
    // Même validation stricte que le chemin par hôte : un port local qui
    // n'est pas un serveur MCP doit être rejeté (faux positifs du scan local).
    if method == "initialize" {
        match crate::validate::validate_initialize(&v) {
            crate::validate::McpValid::Valid { .. } => {}
            crate::validate::McpValid::NotMcp => {
                return Err("initialize sans serverInfo : ce port ne parle pas MCP".to_string())
            }
            crate::validate::McpValid::NotJsonRpc => {
                return Err("réponse non JSON-RPC : ce port ne parle pas MCP".to_string())
            }
        }
    }
    Ok((v.get("result").cloned().unwrap_or(Value::Null), host, head))
}

/// Probe MCP streamable-HTTP complète : initialize + tools/list.
pub fn probe_http(port: u16, path: &str, timeout_ms: u64) -> McpProbe {
    probe_http_with(port, path, timeout_ms, &ProbeHint::default())
}

/// Probe MCP streamable-HTTP avec indice de sonde légère (P1 rang 2).
pub fn probe_http_with(port: u16, path: &str, timeout_ms: u64, hint: &ProbeHint<'_>) -> McpProbe {
    let quick = Duration::from_millis(timeout_ms.min(1500));
    let open_host = if port_open("::1", port, quick) {
        "::1"
    } else if port_open("127.0.0.1", port, quick) {
        "127.0.0.1"
    } else {
        return McpProbe::fail(
            "port fermé (rien n'écoute sur ::1 ni 127.0.0.1)".to_string(),
            false,
        );
    };
    let client_info = json!({
        "name": "mcp-control",
        "version": env!("CARGO_PKG_VERSION")
    });
    let init_params = json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": client_info.clone()
    });
    // Protocole 2026-07-28 (stateless) : certains serveurs (rust-mcp-sdk 2.x)
    // REFUSENT un initialize nu — il leur faut `Mcp-Method` + les clés `_meta`
    // io.modelcontextprotocol/*. On sonde donc en 2 temps : legacy d'abord
    // (majorité des serveurs), stateless en repli.
    let strict_meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": client_info.clone(),
        "io.modelcontextprotocol/clientCapabilities": {}
    });
    let init_strict = json!({
        "protocolVersion": "2026-07-28",
        "capabilities": {},
        "clientInfo": client_info,
        "_meta": strict_meta.clone()
    });
    let strict_init_hdrs: [(&str, &str); 2] = [
        ("MCP-Protocol-Version", "2026-07-28"),
        ("Mcp-Method", "initialize"),
    ];
    let other_host = if open_host == "::1" {
        "127.0.0.1"
    } else {
        "::1"
    };
    let t0 = Instant::now();
    // MOTIF D'ERREUR (audit 03, constat 4) : l'hôte de repli (::1 ou
    // 127.0.0.1) n'a jamais accepté la connexion — son erreur (souvent
    // « connexion refusée ») ne doit pas écraser celle de l'hôte qui a
    // ACCEPTÉ (délai dépassé, réponse invalide…). On publie toujours
    // l'erreur de l'hôte joignable en priorité.
    let mut attempt: Option<(serde_json::Value, String, RespHead, u64, bool)> = None;
    let mut err_hote: Option<String> = None;
    let mut err_repli: Option<String> = None;
    let mut muet = false;
    'probe: for (params, hdrs, stateless) in [
        (&init_params, &[][..], false),
        (&init_strict, &strict_init_hdrs[..], true),
    ] {
        for host in [open_host, other_host] {
            match rpc_call_host_headers(
                host,
                port,
                path,
                "initialize",
                params.clone(),
                1,
                timeout_ms,
                hdrs,
            ) {
                Ok((r, h)) => {
                    attempt = Some((
                        r,
                        host.to_string(),
                        h,
                        t0.elapsed().as_millis() as u64,
                        stateless,
                    ));
                    break 'probe;
                }
                Err(e) => {
                    if e.contains(crate::auth::net::LOCKED_SERVER_MSG) {
                        err_hote = Some(e);
                        muet = true;
                        break 'probe;
                    }
                    if host == open_host {
                        // PORT MUET (lot A) : la forme a expiré sans AUCUN octet
                        // de l'hôte vérifié ouvert — rejouer les formes suivantes
                        // (strict, server/discover) ne peut rien donner : un port
                        // qui n'écrit jamais un octet n'en écrira pas davantage
                        // pour une autre requête. L'ancien code encaissait les
                        // 3 formes (~18-25 s de classement pour un muet).
                        let muet_forme = zero_octets(&e, open_host);
                        err_hote = Some(e);
                        if muet_forme {
                            muet = true;
                            break 'probe;
                        }
                    } else {
                        err_repli = Some(e);
                    }
                }
            }
        }
    }
    // 3e forme : serveurs stateless 2026-07-28 qui n'implémentent PAS
    // `initialize` (rust-mcp-sdk 2.x répond « No handler is implemented for
    // custom requests »). La découverte passe par `server/discover`, et le
    // serverInfo vit sous result._meta["io.modelcontextprotocol/serverInfo"].
    let mut discovered: Option<serde_json::Value> = None;
    // PORT MUET : la 3e forme est sautée elle aussi — un port qui n'a répondu
    // aucun octet aux deux premières n'en répondra pas davantage à celle-ci.
    if attempt.is_none() && !muet {
        let discover_hdrs: [(&str, &str); 2] = [
            ("MCP-Protocol-Version", "2026-07-28"),
            ("Mcp-Method", "server/discover"),
        ];
        for host in [open_host, other_host] {
            if let Ok((r, h)) = rpc_call_host_headers(
                host,
                port,
                path,
                "server/discover",
                json!({ "_meta": strict_meta.clone() }),
                1,
                timeout_ms,
                &discover_hdrs,
            ) {
                // VALIDATION STRICTE du repli stateless : `server/discover`
                // doit prouver qu'il est un serveur MCP, sinon un port qui
                // répond {"result":{}} à n'importe quoi passe (faux positif).
                // La preuve est serverInfo sous _meta (spec 2026-07-28).
                let proof = r
                    .get("_meta")
                    .and_then(|m| m.get("io.modelcontextprotocol/serverInfo"))
                    .and_then(|si| si.get("name"))
                    .and_then(|n| n.as_str())
                    .map(|n| !n.trim().is_empty())
                    .unwrap_or(false);
                if !proof {
                    let motif =
                        "server/discover sans serverInfo : ce port ne parle pas MCP".to_string();
                    if host == open_host {
                        err_hote = Some(motif);
                    } else {
                        err_repli = Some(motif);
                    }
                    continue;
                }
                discovered = Some(r);
                attempt = Some((
                    serde_json::Value::Null,
                    host.to_string(),
                    h,
                    t0.elapsed().as_millis() as u64,
                    true,
                ));
                break;
            }
        }
    }
    let (res, host, head, latency, stateless_mode) = match attempt {
        Some(x) => x,
        None => {
            let motif = err_hote
                .or(err_repli)
                .unwrap_or_else(|| "pas de réponse initialize".to_string());
            if motif.contains(crate::auth::net::LOCKED_SERVER_MSG) {
                let health = health_ok_host(open_host, port, timeout_ms.clamp(400, 1200));
                let mut probe = McpProbe::locked_litemcp(
                    open_host.to_string(),
                    health,
                    t0.elapsed().as_millis() as u64,
                );
                let (in_container, container_name) = port_owner_container(port);
                probe.in_container = in_container;
                probe.container_name = container_name;
                return probe;
            }
            return McpProbe::fail(motif, true);
        }
    };
    // serverInfo : à la racine (initialize) ou sous _meta (server/discover).
    let si = discovered
        .as_ref()
        .and_then(|d| {
            d.get("_meta")
                .and_then(|m| m.get("io.modelcontextprotocol/serverInfo"))
        })
        .or_else(|| res.get("serverInfo"));
    let name = si
        .and_then(|s| s.get("name"))
        .and_then(|x| x.as_str())
        .map(String::from);
    let version = si
        .and_then(|s| s.get("version"))
        .and_then(|x| x.as_str())
        .map(String::from);
    let protocol = res
        .get("protocolVersion")
        .and_then(|x| x.as_str())
        .map(String::from)
        .or_else(|| {
            discovered
                .as_ref()
                .and_then(|d| d.get("supportedVersions"))
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|x| x.as_str())
                .map(String::from)
        });

    // Sonde légère (P1 rang 2) : si le serveur était déjà UP et que
    // serverInfo.name + version n'ont pas changé, on saute /sse, WS,
    // port_owner_lang, /health et tools/list (réutilisés en cache).
    let sse = head.sse();
    if hint.reuse_metadata_if_unchanged
        && name.as_deref() == hint.prev_name
        && version.as_deref() == hint.prev_version
    {
        let locked = crate::auth::net::outbound_identity_for(&host, port)
            .ok()
            .flatten()
            .is_some();
        return McpProbe {
            mcp_up: true,
            port_open: true,
            locked,
            pid: None,
            host: Some(host),
            name,
            version,
            protocol,
            sse,
            sse_endpoint: false,
            ws: false,
            lang: None,
            framework: None,
            in_container: false,
            container_name: None,
            tools: Vec::new(),
            metadata_reused: true,
            latency_ms: latency,
            error: None,
        };
    }

    // Classification transport — le Content-Type de l'initialize NE SUFFIT PAS :
    // FastMCP httpStream répond en text/event-stream (mcp-proxy encode les
    // réponses JSON-RPC en événements SSE, enableJsonResponse=false par défaut)
    // tout en restant du streamable HTTP, et co-sert /sse (compat 2024-11-05).
    // sse = encodage SSE des réponses ; sse_endpoint = /sse legacy co-servi.
    // Le transport réel est streamable HTTP dans tous les cas ici (le POST
    // initialize a été répondu) — un vrai serveur SSE-only échouerait avant.
    let sse_path = if path == "/sse" { path } else { "/sse" };
    let sse_endpoint = sse_get_host(&host, port, sse_path, timeout_ms.clamp(500, 1200));
    // Transport WS : upgrade WebSocket acceptée (101) → badge « WS » dans l'UI.
    let ws = ws_upgrade_host(&host, port, path, timeout_ms.clamp(300, 800));
    // Langage : 1) en-têtes Server/X-Powered-By ; 2) process qui écoute le port.
    // Il sert aussi à arbitrer FastMCP Python vs TypeScript.
    let lang_hint = detect_lang_http(&head, name.as_deref()).or_else(|| port_owner_lang(port));
    // Fingerprint : la sonde /health ne part que si l'initialize a répondu
    // 2026-07-28 (requête émise : 2024-11-05) — aucune requête de plus pour les
    // serveurs qui négocient normalement. Les signatures fortes (litemcp,
    // FastMCP) passent avant la détection par nom du framework.
    let health = protocol.as_deref() == Some(crate::LITEMCP_PROTOCOL)
        && health_ok_host(&host, port, timeout_ms.clamp(400, 1200));
    let fw = crate::frameworks::detect(
        name.as_deref(),
        protocol.as_deref(),
        health,
        sse,
        sse_endpoint,
        lang_hint.as_deref(),
    );
    let framework = fw.map(|f| f.id.to_string());
    // 3) sinon le framework (litemcp = lib Rust par définition, idem rmcp…)
    let lang = lang_hint.or_else(|| fw.map(|f| f.lang.to_string()));
    let mut session_hdr: Vec<(&str, &str)> = Vec::new();
    if !head.session_id.is_empty() {
        session_hdr.push(("Mcp-Session-Id", head.session_id.as_str()));
    }
    // Un serveur stateless 2026-07-28 attend les en-têtes `Mcp-Method` et le
    // bloc `_meta` sur CHAQUE requête, pas seulement sur initialize.
    if stateless_mode {
        session_hdr.push(("MCP-Protocol-Version", "2026-07-28"));
        session_hdr.push(("Mcp-Method", "tools/list"));
    }
    let tools_params = if stateless_mode {
        json!({ "_meta": strict_meta })
    } else {
        json!({})
    };
    let mut tools = Vec::new();
    if let Ok((tr, _)) = rpc_call_host_headers(
        &host,
        port,
        path,
        "tools/list",
        tools_params,
        2,
        timeout_ms,
        &session_hdr,
    ) {
        if let Some(arr) = tr.get("tools").and_then(|t| t.as_array()) {
            for t in arr {
                tools.push(parse_tool_info(t));
            }
        }
    }
    let locked = crate::auth::net::outbound_identity_for(&host, port)
        .ok()
        .flatten()
        .is_some();
    let (in_container, container_name) = port_owner_container(port);
    McpProbe {
        mcp_up: true,
        port_open: true,
        locked,
        pid: None,
        host: Some(host),
        name,
        version,
        protocol,
        sse,
        sse_endpoint,
        ws,
        lang,
        framework,
        in_container,
        container_name,
        tools,
        metadata_reused: false,
        latency_ms: latency,
        error: None,
    }
}

/// Lit au plus `MAX_PROBE_READ` octets jusqu'à `\n` sans jamais allouer une
/// ligne géante en mémoire AVANT troncature (audit 2026-10-25 — `read_until`
/// nu lisait les 300 Mo en RAM avant de rendre la main).
fn read_bounded_line<R: std::io::BufRead>(
    reader: &mut R,
    buf: &mut Vec<u8>,
) -> std::io::Result<usize> {
    buf.clear();
    let n = reader
        .take((MAX_PROBE_READ as u64) + 1)
        .read_until(b'\n', buf)?;
    if buf.len() > MAX_PROBE_READ {
        buf.truncate(MAX_PROBE_READ);
    }
    Ok(n)
}

/// Lignes stderr conservées par process stdio (diagnostic de démarrage).
const STDERR_TAIL_LINES: usize = 16;
/// Longueur max du résumé stderr ajouté à un message d'erreur.
const STDERR_SUMMARY_MAX: usize = 300;

/// Résumé lisible des dernières lignes stderr : ignore le bruit npm
/// (« npm warn … ») et garde les 2 dernières lignes utiles, tronquées.
fn stderr_summary(tail: &std::collections::VecDeque<String>) -> String {
    let utiles: Vec<&str> = tail
        .iter()
        .map(|l| l.as_str())
        .filter(|l| {
            let b = l.to_ascii_lowercase();
            !(b.starts_with("npm warn") || b.starts_with("npm notice"))
        })
        .collect();
    // Crash Node/Python : la ligne « …Error: … » ou « …Exception: … » est la
    // seule parlante (la pile et « Node.js vXX » qui suivent n'apprennent rien).
    let s = match utiles
        .iter()
        .rev()
        .find(|l| l.contains("Error:") || l.contains("Exception:"))
    {
        Some(l) => l.to_string(),
        None => utiles[utiles.len().saturating_sub(2)..].join(" | "),
    };
    if s.chars().count() > STDERR_SUMMARY_MAX {
        let mut t: String = s.chars().take(STDERR_SUMMARY_MAX).collect();
        t.push('…');
        t
    } else {
        s
    }
}

/// Probe/handshake MCP stdio (JSON délimité par nouvelles lignes sur stdin/stdout).
/// keep_alive=true → ne tue pas le process, renvoie un StdioHandle.
pub fn probe_stdio(
    program: &str,
    args: &[String],
    workdir: Option<&str>,
    timeout_ms: u64,
    keep_alive: bool,
    env: &[(String, String)],
) -> Result<(McpProbe, Option<StdioHandle>), String> {
    let t0 = Instant::now();
    let (launch_program, launch_args) = resolve_stdio_command(program, args);
    let mut cmd = Command::new(&launch_program);
    cmd.args(&launch_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Hygiène par défaut sur les sous-processus stdio (UTF-8 Python sous Windows,
    // stdout Python non bufferisé, suppression du bruit couleur/notifier npm) —
    // chaque variable explicitement fournie par l'utilisateur reste prioritaire.
    for (k, v) in [
        ("PYTHONUTF8", "1"),
        ("PYTHONUNBUFFERED", "1"),
        ("NO_COLOR", "1"),
        ("NPM_CONFIG_UPDATE_NOTIFIER", "false"),
        ("NPM_CONFIG_FUND", "false"),
        ("NPM_CONFIG_AUDIT", "false"),
        ("npm_config_yes", "true"),
    ] {
        if !env.iter().any(|(ek, _)| ek.eq_ignore_ascii_case(k)) {
            cmd.env(k, v);
        }
    }
    // env additionnel du transport stdio (import mcpServers, lot 3) : les
    // valeurs ne vivent que dans le process enfant — jamais journalisées.
    cmd.envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    if let Some(wd) = workdir {
        cmd.current_dir(wd);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn {}: {}", program, e))?;
    let pid = child.id();
    let mut stdin = child.stdin.take().ok_or("stdin indisponible")?;
    let stdout = child.stdout.take().ok_or("stdout indisponible")?;
    // stderr : vidé en continu (sinon un enfant bavard bloquerait sur un pipe
    // plein) et seules les dernières lignes sont gardées, bornées, pour
    // expliquer un échec de démarrage (« Credentials not found… ») au lieu
    // d'un faux « timeout ».
    let stderr_tail = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::<
        String,
    >::new()));
    if let Some(se) = child.stderr.take() {
        let tail = stderr_tail.clone();
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(se);
            let mut buf: Vec<u8> = Vec::new();
            while let Ok(n) = read_bounded_line(&mut reader, &mut buf) {
                if n == 0 {
                    break;
                }
                let l = String::from_utf8_lossy(&buf).trim().to_string();
                if l.is_empty() {
                    continue;
                }
                if let Ok(mut t) = tail.lock() {
                    if t.len() >= STDERR_TAIL_LINES {
                        t.pop_front();
                    }
                    t.push_back(l);
                }
            }
        });
    }

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stdout);
        // ROBUSTESSE (audit 2026-10-25) : `BufRead::lines()` et `read_until` nu
        // allouent la ligne ENTIÈRE en mémoire, sans plafond — un serveur stdio
        // qui écrit 300 Mo sur une seule ligne (sans \n) ferait exploser la RAM
        // du moteur. `read_bounded_line` borne la lecture via `Read::take`
        // AVANT allocation.
        let mut buf: Vec<u8> = Vec::new();
        loop {
            match read_bounded_line(&mut reader, &mut buf) {
                Ok(0) => break, // EOF
                Ok(_) => {
                    let l = String::from_utf8_lossy(&buf).to_string();
                    if tx.send(l).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let init = json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05", "capabilities": {},
            "clientInfo": {"name": "mcp-control", "version": env!("CARGO_PKG_VERSION")}
        }
    });
    if let Err(e) = writeln!(stdin, "{}", init) {
        kill_tree(pid);
        return Err(format!("write init: {}", e));
    }
    let _ = stdin.flush();

    let mut deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut got_init = false;
    let mut tools_req_id: u64 = 2;
    const MAX_TOOLS_PAGES: u64 = 10;
    let mut name: Option<String> = None;
    let mut version: Option<String> = None;
    let mut protocol: Option<String> = None;
    let mut tools: Vec<ToolInfo> = Vec::new();
    let mut init_err: Option<String> = None;
    // stdout fermé avant la réponse : le process s'est arrêté (pas un timeout)
    let mut stdout_closed = false;

    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let wait = deadline - now;
        match rx.recv_timeout(wait) {
            Ok(line) => {
                let t = line.trim().trim_start_matches('\u{feff}');
                if !t.starts_with('{') {
                    continue;
                } // logs/bruits
                let v: Value = match serde_json::from_str(t) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if v.get("method").is_some() {
                    continue;
                }
                if rpc_id_matches(v.get("id"), 1) && !got_init {
                    if let Some(err) = v.get("error") {
                        init_err = Some(format!("initialize rejeté: {}", err));
                        break;
                    }
                    let res = v.get("result").cloned().unwrap_or(Value::Null);
                    if let Some(si) = res.get("serverInfo") {
                        name = si.get("name").and_then(|x| x.as_str()).map(String::from);
                        version = si.get("version").and_then(|x| x.as_str()).map(String::from);
                    }
                    protocol = res
                        .get("protocolVersion")
                        .and_then(|x| x.as_str())
                        .map(String::from);
                    got_init = true;
                    // Fenêtre dédiée à `tools/list` pour qu'une installation à froid lente
                    // sur `initialize` ne prive pas l'inventaire des outils de budget.
                    deadline = Instant::now() + Duration::from_millis(20_000);
                    let _ = writeln!(
                        stdin,
                        "{}",
                        json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
                    );
                    let _ = writeln!(
                        stdin,
                        "{}",
                        json!({"jsonrpc": "2.0", "id": tools_req_id, "method": "tools/list", "params": {}})
                    );
                    let _ = stdin.flush();
                } else if got_init && rpc_id_matches(v.get("id"), tools_req_id) {
                    let mut next_cursor: Option<String> = None;
                    if let Some(res) = v.get("result") {
                        if let Some(arr) = res.get("tools").and_then(|x| x.as_array()) {
                            for t in arr {
                                tools.push(parse_tool_info(t));
                            }
                        }
                        next_cursor = res
                            .get("nextCursor")
                            .and_then(|x| x.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string);
                    }
                    if let Some(cursor) = next_cursor {
                        if tools_req_id < 2 + MAX_TOOLS_PAGES - 1 {
                            tools_req_id += 1;
                            deadline = Instant::now() + Duration::from_millis(15_000);
                            let _ = writeln!(
                                stdin,
                                "{}",
                                json!({
                                    "jsonrpc": "2.0",
                                    "id": tools_req_id,
                                    "method": "tools/list",
                                    "params": { "cursor": cursor }
                                })
                            );
                            let _ = stdin.flush();
                            continue;
                        }
                    }
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                stdout_closed = true;
                break;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
        }
    }

    let latency = t0.elapsed().as_millis() as u64;
    if init_err.is_some() || !got_init {
        // code de sortie si le process est déjà mort (laisser au lecteur
        // stderr quelques ms pour vider les dernières lignes)
        let mut exit = None;
        if stdout_closed {
            let fin = Instant::now() + Duration::from_millis(500);
            while Instant::now() < fin {
                if let Ok(Some(st)) = child.try_wait() {
                    exit = Some(st);
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        kill_tree(pid);
        let _ = child.wait();
        let detail = stderr_tail
            .lock()
            .map(|t| stderr_summary(&t))
            .unwrap_or_default();
        let msg = init_err.unwrap_or_else(|| {
            let base = if stdout_closed {
                match exit.and_then(|st| st.code()) {
                    Some(c) => format!(
                        "{} s'est arrêté (code {}) avant de répondre à initialize",
                        program, c
                    ),
                    None => format!("{} s'est arrêté avant de répondre à initialize", program),
                }
            } else {
                format!(
                    "timeout {}: pas de réponse initialize en {} ms",
                    program, timeout_ms
                )
            };
            if detail.is_empty() {
                base
            } else {
                format!("{} — {}", base, detail)
            }
        });
        return Err(msg);
    }
    // Framework d'un serveur stdio : nom annoncé ou paquet lancé (npx/uvx).
    let lang = detect_lang_stdio(&program, args);
    let args_join = args.join(" ").to_ascii_lowercase();
    let fw = crate::frameworks::detect(
        name.as_deref(),
        protocol.as_deref(),
        false,
        false,
        false,
        lang.as_deref(),
    )
    .or_else(|| crate::frameworks::detect_by_name(&args_join));
    let framework = fw.map(|f| f.id.to_string());
    let lang = lang.or_else(|| fw.map(|f| f.lang.to_string()));
    let (in_container, container_name) = detect_container_cmd(program, args);
    if keep_alive {
        Ok((
            McpProbe {
                mcp_up: true,
                port_open: true,
                locked: false,
                pid: Some(pid),
                host: None,
                name,
                version,
                protocol,
                sse: false,
                sse_endpoint: false,
                ws: false,
                lang,
                framework,
                in_container,
                container_name,
                tools,
                metadata_reused: false,
                latency_ms: latency,
                error: None,
            },
            Some(handle_de_child(
                child,
                Some(StdioRpc {
                    stdin: Box::new(stdin),
                    rx,
                    next_id: 100,
                }),
            )),
        ))
    } else {
        kill_tree(pid);
        let _ = child.wait();
        Ok((
            McpProbe {
                mcp_up: true,
                port_open: true,
                locked: false,
                pid: Some(pid),
                host: None,
                name,
                version,
                protocol,
                sse: false,
                sse_endpoint: false,
                ws: false,
                lang,
                framework,
                in_container,
                container_name,
                tools,
                metadata_reused: false,
                latency_ms: latency,
                error: None,
            },
            None,
        ))
    }
}

/// Un appel JSON-RPC HTTP vers un hôte précis (POST + parse).
pub fn rpc_call_host(
    host: &str,
    port: u16,
    path: &str,
    method: &str,
    params: Value,
    id: u64,
    timeout_ms: u64,
) -> Result<(Value, RespHead), String> {
    rpc_call_host_headers(host, port, path, method, params, id, timeout_ms, &[])
}

/// Extrait un [`ToolInfo`] complet (nom, description jusqu'à 4096 car., schéma JSON
/// `inputSchema` éventuel) depuis une entrée `tools/list`.
pub fn parse_tool_info(t: &Value) -> ToolInfo {
    ToolInfo {
        name: t
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("?")
            .to_string(),
        desc: t
            .get("description")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .chars()
            .take(4096)
            .collect(),
        input_schema: t.get("inputSchema").filter(|v| !v.is_null()).cloned(),
    }
}

/// Exécute un appel `tools/call` sur un serveur MCP HTTP (mode stateless LiteMCP
/// `2026-07-28` ou MCP HTTP standard avec repli automatique `initialize` pour
/// récupérer `Mcp-Session-Id`). Retourne `(résultat, durée_ms)`.
pub fn call_http_tool(
    host: &str,
    port: u16,
    path: &str,
    protocol: Option<&str>,
    tool: &str,
    arguments: Value,
    timeout_ms: u64,
) -> Result<(Value, u64), String> {
    let t0 = Instant::now();
    let is_litemcp = protocol == Some(crate::LITEMCP_PROTOCOL);
    let mut params = json!({
        "name": tool,
        "arguments": arguments,
    });
    if is_litemcp {
        if let Some(obj) = params.as_object_mut() {
            obj.insert(
                "_meta".into(),
                json!({"mcpProtocolVersion": crate::LITEMCP_PROTOCOL}),
            );
        }
        let hdrs = [
            ("Mcp-Method", "tools/call"),
            ("Mcp-Name", tool),
            ("MCP-Protocol-Version", crate::LITEMCP_PROTOCOL),
        ];
        let (res, _) = rpc_call_host_headers(
            host,
            port,
            path,
            "tools/call",
            params,
            10,
            timeout_ms,
            &hdrs,
        )?;
        return Ok((res, t0.elapsed().as_millis() as u64));
    }

    // Mode MCP HTTP standard : essai direct, puis repli initialize + Mcp-Session-Id
    match rpc_call_host_headers(
        host,
        port,
        path,
        "tools/call",
        params.clone(),
        10,
        timeout_ms,
        &[],
    ) {
        Ok((res, _)) => Ok((res, t0.elapsed().as_millis() as u64)),
        Err(first_err) => {
            let init_params = json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "mcp-control", "version": env!("CARGO_PKG_VERSION")},
            });
            let Ok((_, head)) = rpc_call_host_headers(
                host,
                port,
                path,
                "initialize",
                init_params,
                1,
                timeout_ms,
                &[],
            ) else {
                return Err(first_err);
            };
            if head.session_id.is_empty() {
                return Err(first_err);
            }
            let hdrs = [("Mcp-Session-Id", head.session_id.as_str())];
            let (res, _) = rpc_call_host_headers(
                host,
                port,
                path,
                "tools/call",
                params,
                10,
                timeout_ms,
                &hdrs,
            )?;
            Ok((res, t0.elapsed().as_millis() as u64))
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn rpc_call_host_headers(
    host: &str,
    port: u16,
    path: &str,
    method: &str,
    params: Value,
    id: u64,
    timeout_ms: u64,
    extra_headers: &[(&str, &str)],
) -> Result<(Value, RespHead), String> {
    let body = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
    let (bytes, head) = http_post_host(host, port, path, &body, timeout_ms, extra_headers)?;
    let v =
        extract_json(&bytes).ok_or_else(|| format!("réponse non JSON ({} octets)", bytes.len()))?;
    if let Some(err) = v.get("error") {
        return Err(format!("rpc error: {}", err));
    }
    // VALIDATION STRICTE sur `initialize` : c'est LA preuve qu'un port ouvert
    // parle MCP. Un port qui répond autre chose (API REST, page HTML, 200
    // vide) est rejeté ici — sinon le scan réseau invente des serveurs.
    // Les autres méthodes (tools/list, notifications…) ne sont PAS validées :
    // elles n'ont pas serverInfo par nature.
    if method == "initialize" {
        match crate::validate::validate_initialize(&v) {
            crate::validate::McpValid::Valid { .. } => {}
            crate::validate::McpValid::NotMcp => {
                return Err("initialize sans serverInfo : ce port ne parle pas MCP".to_string())
            }
            crate::validate::McpValid::NotJsonRpc => {
                return Err("réponse non JSON-RPC : ce port ne parle pas MCP".to_string())
            }
        }
    }
    Ok((v.get("result").cloned().unwrap_or(Value::Null), head))
}

/// Probe MCP streamable-HTTP sur un hôte précis : initialize + tools/list.
pub fn probe_http_host(host: &str, port: u16, path: &str, timeout_ms: u64) -> McpProbe {
    probe_http_host_with(host, port, path, timeout_ms, &ProbeHint::default())
}

/// Probe MCP streamable-HTTP sur un hôte précis avec indice de sonde légère (P1 rang 2).
pub fn probe_http_host_with(
    host: &str,
    port: u16,
    path: &str,
    timeout_ms: u64,
    hint: &ProbeHint<'_>,
) -> McpProbe {
    if !port_open(host, port, Duration::from_millis(timeout_ms.min(1500))) {
        return McpProbe::fail(format!("{}:{} injoignable", host, port), false);
    }
    let t0 = Instant::now();
    let client_info = json!({
        "name": "mcp-control",
        "version": env!("CARGO_PKG_VERSION")
    });
    let init_params = json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": client_info.clone()
    });
    let strict_meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": client_info.clone(),
        "io.modelcontextprotocol/clientCapabilities": {}
    });
    let init_strict = json!({
        "protocolVersion": "2026-07-28",
        "capabilities": {},
        "clientInfo": client_info,
        "_meta": strict_meta.clone()
    });
    let strict_init_hdrs: [(&str, &str); 2] = [
        ("MCP-Protocol-Version", "2026-07-28"),
        ("Mcp-Method", "initialize"),
    ];

    let mut attempt: Option<(serde_json::Value, RespHead, u64, bool)> = None;
    let mut err_hote: Option<String> = None;
    let mut muet = false;
    for (params, hdrs, stateless) in [
        (&init_params, &[][..], false),
        (&init_strict, &strict_init_hdrs[..], true),
    ] {
        match rpc_call_host_headers(
            host,
            port,
            path,
            "initialize",
            params.clone(),
            1,
            timeout_ms,
            hdrs,
        ) {
            Ok((r, h)) => {
                attempt = Some((r, h, t0.elapsed().as_millis() as u64, stateless));
                break;
            }
            Err(e) => {
                let is_locked = e.contains(crate::auth::net::LOCKED_SERVER_MSG);
                let muet_forme = zero_octets(&e, host);
                err_hote = Some(e);
                if is_locked || muet_forme {
                    muet = true;
                    break;
                }
            }
        }
    }

    let mut discovered: Option<serde_json::Value> = None;
    if attempt.is_none() && !muet {
        let discover_hdrs: [(&str, &str); 2] = [
            ("MCP-Protocol-Version", "2026-07-28"),
            ("Mcp-Method", "server/discover"),
        ];
        if let Ok((r, h)) = rpc_call_host_headers(
            host,
            port,
            path,
            "server/discover",
            json!({ "_meta": strict_meta.clone() }),
            1,
            timeout_ms,
            &discover_hdrs,
        ) {
            let proof = r
                .get("_meta")
                .and_then(|m| m.get("io.modelcontextprotocol/serverInfo"))
                .and_then(|si| si.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| !n.trim().is_empty())
                .unwrap_or(false);
            if proof {
                discovered = Some(r);
                attempt = Some((
                    serde_json::Value::Null,
                    h,
                    t0.elapsed().as_millis() as u64,
                    true,
                ));
            } else {
                err_hote =
                    Some("server/discover sans serverInfo : ce port ne parle pas MCP".to_string());
            }
        }
    }

    let (res, head, latency, stateless_mode) = match attempt {
        Some(x) => x,
        None => {
            let motif = err_hote.unwrap_or_else(|| "pas de réponse initialize".to_string());
            if motif.contains(crate::auth::net::LOCKED_SERVER_MSG) {
                let health = health_ok_host(host, port, timeout_ms.clamp(400, 1200));
                let mut probe = McpProbe::locked_litemcp(
                    host.to_string(),
                    health,
                    t0.elapsed().as_millis() as u64,
                );
                if crate::is_loopback_bind(host) {
                    let (in_container, container_name) = port_owner_container(port);
                    probe.in_container = in_container;
                    probe.container_name = container_name;
                }
                return probe;
            }
            return McpProbe::fail(motif, true);
        }
    };
    let si = discovered
        .as_ref()
        .and_then(|d| {
            d.get("_meta")
                .and_then(|m| m.get("io.modelcontextprotocol/serverInfo"))
        })
        .or_else(|| res.get("serverInfo"));
    let name = si
        .and_then(|s| s.get("name"))
        .and_then(|x| x.as_str())
        .map(String::from);
    let version = si
        .and_then(|s| s.get("version"))
        .and_then(|x| x.as_str())
        .map(String::from);
    let protocol = res
        .get("protocolVersion")
        .and_then(|x| x.as_str())
        .map(String::from)
        .or_else(|| {
            discovered
                .as_ref()
                .and_then(|d| d.get("supportedVersions"))
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|x| x.as_str())
                .map(String::from)
        });

    // Sonde légère (P1 rang 2) : si le serveur était déjà UP et que
    // serverInfo.name + version n'ont pas changé, on saute /sse, WS, /health
    // et tools/list (réutilisés en cache).
    let sse = head.sse();
    if hint.reuse_metadata_if_unchanged
        && name.as_deref() == hint.prev_name
        && version.as_deref() == hint.prev_version
    {
        let locked = crate::auth::net::outbound_identity_for(host, port)
            .ok()
            .flatten()
            .is_some();
        return McpProbe {
            mcp_up: true,
            port_open: true,
            locked,
            pid: None,
            host: Some(host.to_string()),
            name,
            version,
            protocol,
            sse,
            sse_endpoint: false,
            ws: false,
            lang: None,
            framework: None,
            in_container: false,
            container_name: None,
            tools: Vec::new(),
            metadata_reused: true,
            latency_ms: latency,
            error: None,
        };
    }

    // Même classification que probe_http (voir le commentaire là-bas) :
    // POST initialize répondu = streamable HTTP ; sse = encodage SSE des
    // réponses ; sse_endpoint = /sse legacy co-servi (FastMCP).
    let sse_path = if path == "/sse" { path } else { "/sse" };
    let sse_endpoint = sse_get_host(host, port, sse_path, timeout_ms.clamp(500, 1200));
    // Transport WS : upgrade WebSocket acceptée (101) → badge « WS » dans l'UI.
    let ws = ws_upgrade_host(host, port, path, timeout_ms.clamp(300, 800));
    let is_loopback = crate::is_loopback_bind(host);
    let lang_hint = detect_lang_http(&head, name.as_deref())
        .or_else(|| if is_loopback { port_owner_lang(port) } else { None });
    let health = protocol.as_deref() == Some(crate::LITEMCP_PROTOCOL)
        && health_ok_host(host, port, timeout_ms.clamp(400, 1200));
    let fw = crate::frameworks::detect(
        name.as_deref(),
        protocol.as_deref(),
        health,
        sse,
        sse_endpoint,
        lang_hint.as_deref(),
    );
    let framework = fw.map(|f| f.id.to_string());
    let lang = lang_hint.or_else(|| fw.map(|f| f.lang.to_string()));
    let mut session_hdr: Vec<(&str, &str)> = Vec::new();
    if !head.session_id.is_empty() {
        session_hdr.push(("Mcp-Session-Id", head.session_id.as_str()));
    }
    if stateless_mode {
        session_hdr.push(("MCP-Protocol-Version", "2026-07-28"));
        session_hdr.push(("Mcp-Method", "tools/list"));
    }
    let tools_params = if stateless_mode {
        json!({ "_meta": strict_meta })
    } else {
        json!({})
    };
    let mut tools = Vec::new();
    if let Ok((tr, _)) = rpc_call_host_headers(
        host,
        port,
        path,
        "tools/list",
        tools_params,
        2,
        timeout_ms,
        &session_hdr,
    ) {
        if let Some(arr) = tr.get("tools").and_then(|t| t.as_array()) {
            for t in arr {
                tools.push(parse_tool_info(t));
            }
        }
    }
    let locked = crate::auth::net::outbound_identity_for(host, port)
        .ok()
        .flatten()
        .is_some();
    let (in_container, container_name) = if is_loopback {
        port_owner_container(port)
    } else {
        (false, None)
    };
    McpProbe {
        mcp_up: true,
        port_open: true,
        locked,
        pid: None,
        host: Some(host.to_string()),
        name,
        version,
        protocol,
        sse,
        sse_endpoint,
        ws,
        lang,
        framework,
        in_container,
        container_name,
        tools,
        metadata_reused: false,
        latency_ms: latency,
        error: None,
    }
}

// ---------- découverte de pairs MCP Control sur le LAN ----------

pub struct PeerProbe {
    pub alive: bool,
    pub servers: Option<Value>,
}

/// Sonde un moteur MCP Control distant (cache de pairs).
///
/// Lot M4 (point 3, `docs/auth.md` §10) :
/// 1. Si ce pair est appairé (identité cliente épinglée dans `auth/clients/`),
///    la sonde utilise `POST /mcp` (`tools/call` `list_servers`, droit
///    `lecture`), signé avec l'identité cliente et vérifiant `PinnedServer`
///    en TLS 1.3 hors loopback — plus aucun secret partagé `X-MCC-Proof`.
/// 2. Repli transitoire DA3 (0.8.x) : si aucun appairage n'existe encore pour
///    cette cible, conserve le chemin lecture seule `/api/hello` + `/api/state`
///    avec `X-MCC-Proof` (ou sans preuve sur loopback).
pub fn probe_peer(ip: &str, port: u16, access: Option<&str>) -> PeerProbe {
    match crate::auth::net::outbound_identity_for(ip, port) {
        Err(_) => {
            return PeerProbe {
                alive: false,
                servers: None,
            };
        }
        Ok(Some(_)) => {
            let hdrs = [("Mcp-Method", "tools/call"), ("Mcp-Name", "list_servers")];
            let Ok((res, _)) = rpc_call_host_headers(
                ip,
                port,
                "/mcp",
                "tools/call",
                json!({ "name": "list_servers", "arguments": {} }),
                1,
                2000,
                &hdrs,
            ) else {
                return PeerProbe {
                    alive: false,
                    servers: None,
                };
            };
            let doc = res
                .get("structuredContent")
                .cloned()
                .or_else(|| {
                    res.get("content")
                        .and_then(|c| c.as_array())
                        .and_then(|a| a.first())
                        .and_then(|item| item.get("text"))
                        .and_then(|t| t.as_str())
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                })
                .unwrap_or(res);
            if doc.get("servers").is_some() && doc.get("stats").is_some() {
                return PeerProbe {
                    alive: true,
                    servers: Some(doc),
                };
            }
            return PeerProbe {
                alive: false,
                servers: None,
            };
        }
        Ok(None) => {}
    }

    let nonce: Option<String> = match http_get_host(ip, port, "/api/hello", 1200) {
        Ok((bytes, _)) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(v) if v.get("app").and_then(|x| x.as_str()) == Some("mcp-control") => {
                v.get("nonce").and_then(|x| x.as_str()).map(String::from)
            }
            _ => {
                return PeerProbe {
                    alive: false,
                    servers: None,
                }
            }
        },
        Err(_) => {
            return PeerProbe {
                alive: false,
                servers: None,
            }
        }
    };
    let extras: Vec<(String, String)> = match (access, nonce) {
        (Some(code), Some(n)) if !code.is_empty() => vec![
            ("X-MCC-Nonce".to_string(), n.clone()),
            (
                "X-MCC-Proof".to_string(),
                crate::crypto::hmac_sha256_hex(
                    code.as_bytes(),
                    format!("mcc-proof-v1:{}", n).as_bytes(),
                ),
            ),
        ],
        _ => Vec::new(),
    };
    let extra_refs: Vec<(&str, &str)> = extras
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let body = http_post_host(ip, port, "/api/state", "", 1200, &extra_refs).map(|(b, _)| b);
    match body {
        Ok(bytes) => {
            // /api/state répond à tout POST : valider que c'est du JSON avec "servers"
            match serde_json::from_slice::<Value>(&bytes) {
                Ok(v) if v.get("servers").is_some() && v.get("stats").is_some() => PeerProbe {
                    alive: true,
                    servers: Some(v),
                },
                _ => PeerProbe {
                    alive: false,
                    servers: None,
                },
            }
        }
        Err(_) => PeerProbe {
            alive: false,
            servers: None,
        },
    }
}

/// IPs voisines depuis le cache ARP local (Windows: arp -a, Linux/mac: arp -a).
/// Filtrées sur le /24 demandé, sans self.
pub fn arp_hosts(prefix: &str, self_ip: &str) -> Vec<String> {
    let mut c = Command::new(system_tool("arp"));
    let out = hidden(&mut c).arg("-a").output();
    let stdout = match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => return Vec::new(),
    };
    let mut ips = std::collections::BTreeSet::new();
    for line in stdout.lines() {
        for tok in line.split_whitespace() {
            let tok = tok.trim_matches(|c| c == '(' || c == ')');
            if looks_like_ip(tok) && tok.starts_with(prefix) && tok != self_ip {
                // écarter réseau (x.x.x.0) et broadcast (x.x.x.255)
                if tok
                    .rsplit_once('.')
                    .map(|(_, last)| last == "0" || last == "255")
                    .unwrap_or(true)
                {
                    continue;
                }
                ips.insert(tok.to_string());
            }
        }
    }
    ips.into_iter().collect()
}

pub fn looks_like_ip(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    parts.iter().all(|p| {
        !p.is_empty()
            && p.len() <= 3
            && p.chars().all(|c| c.is_ascii_digit())
            && p.parse::<u8>().is_ok()
    })
}

/// Un moteur MCP Control tourne-t-il déjà sur CE port local (127.0.0.1/::1) ?
/// Retourne l'URL de son UI. Utilisé par run_headless pour se RECONNECTER à un
/// moteur existant au lieu d'en lancer un second — deux moteurs sur la même
/// config se disputeraient les spawns et réécriraient le config.json en boucle.
pub fn control_ui_at(port: u16) -> Option<String> {
    control_ui_at_hosts(port, &["127.0.0.1", "::1"])
}

/// Cherche un moteur MCP Control sur le port donné, en interrogeant les
/// hôtes indiqués (le bind configuré en premier : un moteur en bind LAN
/// n'est PAS joignable sur 127.0.0.1 si le bind est une IP précise).
/// Timeout large : sous forte charge (scan réseau), 800 ms ne suffisaient pas
/// et la détection échouait — la nouvelle instance prenait alors le port
/// suivant et fuyait un moteur de plus à chaque lancement.
pub fn control_ui_at_hosts(port: u16, hosts: &[&str]) -> Option<String> {
    for host in hosts.iter().copied() {
        if let Ok((bytes, _)) = http_get_host(host, port, "/api/hello", 2000) {
            if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                if v.get("app").and_then(|x| x.as_str()) == Some("mcp-control") {
                    let h = if host.contains(':') {
                        format!("[{}]:{}", host, port)
                    } else {
                        format!("{}:{}", host, port)
                    };
                    return Some(format!("http://{}", h));
                }
            }
        } else if !crate::is_loopback_bind(host) {
            let host_hdr = http_host_header(host, port);
            let req = format!(
                "GET /api/hello HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                host_hdr
            );
            if let Ok((raw, _)) = crate::auth::net::https_exchange_raw(
                host,
                port,
                req.as_bytes(),
                Duration::from_millis(2000),
                None,
                MAX_PROBE_READ_SMALL,
            ) {
                if let Some(sep) = find_sub(&raw, b"\r\n\r\n") {
                    if let Ok(v) = serde_json::from_slice::<Value>(&raw[sep + 4..]) {
                        if v.get("app").and_then(|x| x.as_str()) == Some("mcp-control") {
                            let h = if host.contains(':') {
                                format!("[{}]:{}", host, port)
                            } else {
                                format!("{}:{}", host, port)
                            };
                            return Some(format!("http://{}", h));
                        }
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    /// Un serveur stdio qui meurt au démarrage (ex. server-gdrive sans
    /// identifiants) doit remonter son stderr et son code de sortie, vite,
    /// au lieu d'un faux « timeout … 45000 ms ».
    #[cfg(windows)]
    #[test]
    fn probe_stdio_mort_au_demarrage_remonte_stderr_et_code() {
        let args: Vec<String> = [
            "/c",
            "echo npm warn bruit 1>&2 & echo Credentials not found. Please run with 'auth' argument first. 1>&2 & exit 3",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let t0 = Instant::now();
        let err = probe_stdio("cmd", &args, None, 20_000, false, &[])
            .err()
            .expect("doit échouer");
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "pas d'attente du timeout"
        );
        assert!(err.contains("s'est arrêté (code 3)"), "{err}");
        assert!(err.contains("Credentials not found"), "{err}");
        assert!(!err.contains("npm warn"), "{err}");
        assert!(!err.contains("timeout"), "{err}");
    }

    #[test]
    fn stderr_summary_garde_les_deux_dernieres_lignes_utiles() {
        let t: std::collections::VecDeque<String> = ["a", "npm warn x", "b", "c", "npm notice y"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(stderr_summary(&t), "b | c");
        let crash: std::collections::VecDeque<String> = [
            "Error: Cannot find package 'playwright-core'",
            "    at legacyMainResolve (node:internal)",
            "}",
            "Node.js v26.12.1",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            stderr_summary(&crash),
            "Error: Cannot find package 'playwright-core'"
        );
    }

    /// Lit la requête HTTP COMPLÈTE (en-têtes + corps annoncé par
    /// `Content-Length`) au lieu d'un `read` unique. Sous charge, un seul
    /// `read` peut ne rendre que les en-têtes : la fixture qui décide sur
    /// `req.contains("method")` répond alors à côté, et fermer une socket
    /// avec un corps non lu envoie un RST au client (sonde en erreur). C'est
    /// la cause des flakes entre tests `probe_http_*` parallèles (audit 01
    /// constat 7, lot F point 6) — PAS une collision de ports.
    fn lire_requete(s: &mut std::net::TcpStream) -> String {
        let mut buf: Vec<u8> = Vec::new();
        let mut tmp = [0u8; 4096];
        loop {
            if let Some(p) = find_sub(&buf, b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..p]).to_ascii_lowercase();
                let mut cl = 0usize;
                for line in head.lines().skip(1) {
                    if let Some(v) = line.trim().strip_prefix("content-length:") {
                        cl = v.trim().parse().unwrap_or(0);
                        break;
                    }
                }
                if buf.len() >= p + 4 + cl {
                    break;
                }
            }
            if buf.len() > MAX_PROBE_READ * 2 {
                break;
            }
            match s.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&tmp[..n]),
                Err(_) => break,
            }
        }
        String::from_utf8_lossy(&buf).to_string()
    }

    /// RÉGRESSION faux positifs : un port HTTP qui répond du JSON-RPC SANS
    /// serverInfo (`{"result":{}}`) ne doit JAMAIS être découvert comme
    /// serveur MCP — ni par `initialize`, ni par le repli `server/discover`
    /// (stateless 2026-07-28). Avant la validation stricte, ce port était
    /// accepté et le scan réseau inventait un serveur « port-XXXX ».
    #[test]
    fn port_non_mcp_avec_result_vide_est_rejete() {
        // RÉGRESSION faux positifs : un port HTTP qui répond du JSON-RPC SANS
        // serverInfo ne doit JAMAIS être découvert comme serveur MCP — ni par
        // initialize, ni par le repli server/discover (stateless 2026-07-28).
        // Avant la validation stricte, ce port était accepté et le scan réseau
        // inventait un serveur « port-XXXX ».
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in l.incoming() {
                let Ok(mut s) = stream else { continue };
                let req = lire_requete(&mut s);
                // ne répondre qu'aux vraies requêtes JSON-RPC : sinon cette
                // fixture empoisonne les sondes /health, /sse et WS des autres
                // tests parallèles (tests flaky).
                if !req.contains("\"method\"") {
                    continue;
                }
                use std::io::Write;
                let body = r#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
                let _ = s.flush();
            }
        });
        std::thread::sleep(std::time::Duration::from_millis(150));
        // aucune des trois routes ne doit produire un serveur MCP
        assert!(
            probe_mcp_any_path(port, 2500).is_none(),
            "un port non-MCP ne doit jamais être découvert (faux positif)"
        );
        // et la sonde directe doit échouer aussi
        let p = probe_http(port, "/mcp", 2500);
        assert!(!p.mcp_up, "probe_http doit refuser un result vide");
    }

    #[test]
    fn find_sub_trouve_et_pas_trouve() {
        assert_eq!(find_sub(b"hello\r\n\r\nworld", b"\r\n\r\n"), Some(5));
        assert_eq!(find_sub(b"abc", b"xyz"), None);
        assert_eq!(find_sub(b"ab", b"abcdef"), None);
        assert_eq!(find_sub(b"", b"a"), None);
    }

    #[test]
    fn dechunk_simple_et_multiple() {
        let data = b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        assert_eq!(dechunk(data), b"hello world".to_vec());
    }

    #[test]
    fn dechunk_avec_extension_et_tronque() {
        let data = b"4;foo=bar\r\nabcd\r\n0\r\n\r\n";
        assert_eq!(dechunk(data), b"abcd".to_vec());
        // chunk complet puis chunk tronqué : les données complètes survivent
        let tronque = b"5\r\nhello\r\nff\r\ncourt";
        assert_eq!(dechunk(tronque), b"hello".to_vec());
        // premier chunk déjà tronqué : rien de récupérable
        assert_eq!(dechunk(b"ff\r\ncourt"), Vec::<u8>::new());
    }

    #[test]
    fn dechunk_responses_hostiles_ne_paniquent_pas() {
        // Régression audit 2026-10-25 : deux panics dans dechunk() —
        //  (1) slice `&data[i..]` hors bornes quand i dépasse len ;
        //  (2) `start + size` en overflow usize sur une taille hexa absurde.
        // Un chunked tronqué suffisait à TUER le thread du poller en silence.
        // Aucune de ces entrées ne doit paniquer.
        let hostiles: Vec<&[u8]> = vec![
            b"5\r\nhello",           // chunk tronqué (pas de CRLF final)
            b"5\r\nhello\r\n",       // chunk complet, CRLF final absent
            b"3\r\nab",              // taille > contenu
            b"ffffffffffffffff\r\n", // taille hexa absurde → overflow
            b"ffffffff\r\nx\r\n",    // absurdité suivie d'un chunk
            b"0\r\n\r\n",            // fin normale
            b"\r\n",                 // ligne vide
            b"zz\r\nabc",            // taille non hexa
        ];
        for h in hostiles {
            let out = dechunk(h);
            // on vérifie surtout l'absence de panic (le test échoue si ça panic)
            assert!(
                out.len() <= h.len(),
                "dechunk ne doit jamais produire plus que l'entrée"
            );
        }
        // un chunked valide reste correctement décodé
        assert_eq!(
            dechunk(b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n"),
            b"hello world".to_vec()
        );
    }

    #[test]
    fn extract_json_depuis_sse() {
        let body =
            b": keepalive\n\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n\n";
        let v = extract_json(body).expect("SSE doit etre parse");
        assert_eq!(v["result"]["ok"], serde_json::json!(true));
    }

    #[test]
    fn ws_upgrade_detecte_101_et_rejette_le_reste() {
        // 1) serveur qui accepte l'upgrade WebSocket → 101 → true
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in l.incoming() {
                let Ok(mut s) = stream else { return };
                let req = lire_requete(&mut s);
                let resp = if req.contains("Upgrade: websocket") {
                    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n"
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                };
                let _ = s.write_all(resp.as_bytes());
            }
        });
        assert!(
            ws_upgrade_host("127.0.0.1", port, "/mcp", 1500),
            "un serveur qui répond 101 doit être détecté comme WS"
        );

        // 2) serveur HTTP ordinaire → 200 → false (pas de faux badge WS)
        let l2 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port2 = l2.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in l2.incoming() {
                let Ok(mut s) = stream else { return };
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let _ = s.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                );
            }
        });
        assert!(
            !ws_upgrade_host("127.0.0.1", port2, "/mcp", 1500),
            "un serveur HTTP qui ne fait pas d'upgrade ne doit pas être marqué WS"
        );

        // 3) port fermé → false, jamais de panic. Le port rendu par bind(:0)
        //    puis RELÂCHÉ peut être repris par un test parallèle entre le drop
        //    et la sonde (collision de ports éphémères) : on retire un port
        //    tant que celui qu'on vient de relâcher répond encore.
        let mut ferme_verifie = false;
        for _ in 0..8 {
            let l3 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port3 = l3.local_addr().unwrap().port();
            drop(l3);
            let repris = std::net::TcpStream::connect_timeout(
                &std::net::SocketAddr::from(([127, 0, 0, 1], port3)),
                std::time::Duration::from_millis(200),
            )
            .is_ok();
            if repris {
                continue;
            }
            assert!(!ws_upgrade_host("127.0.0.1", port3, "/mcp", 600));
            ferme_verifie = true;
            break;
        }
        assert!(
            ferme_verifie,
            "aucun port resté fermé après 8 tirages (machine anormalement chargée)"
        );
    }

    #[test]
    fn sse_get_detecte_un_flux_event_stream_et_rejette_l_encodage_sse_du_streamable() {
        // 1. endpoint SSE legacy : 200 + "event:" dans le flux → true
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let mut buf = [0u8; 8192];
                let _ = s.read(&mut buf);
                let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\nevent: endpoint\ndata: /messages?sessionId=abc\n\n";
                let _ = s.write_all(resp.as_bytes());
            }
        });
        assert!(
            sse_get(port, "/sse", 2000),
            "un endpoint SSE legacy doit etre detecte"
        );

        // 2. FastMCP httpStream : Content-Type text/event-stream MAIS corps JSON
        //    (réponses encodées, aucun "event:") → false. C'est la confusion
        //    exacte qui faisait badge tous les serveurs FastMCP en "SSE".
        let l2 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port2 = l2.local_addr().unwrap().port();
        thread::spawn(move || {
            if let Ok((mut s, _)) = l2.accept() {
                let mut buf = [0u8; 8192];
                let _ = s.read(&mut buf);
                let body = "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        assert!(
            !sse_get(port2, "/sse", 2000),
            "du JSON encodé SSE (streamable httpStream) ne doit PAS passer pour un endpoint SSE"
        );
    }

    #[test]
    fn extract_json_depuis_json_brut_et_texte_avant() {
        let v = extract_json(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[]}}").unwrap();
        assert!(v.get("result").is_some());
        let v2 = extract_json(
            b"HTTP/1.1 200 OK\r\n\r\n{\"jsonrpc\":\"2.0\",\"id\":3,\"error\":{\"code\":-1}}",
        )
        .unwrap();
        assert!(v2.get("error").is_some());
    }

    #[test]
    fn extract_json_rejete_du_bruit() {
        assert!(extract_json(b"Use /health").is_none());
        assert!(extract_json(b"").is_none());
        assert!(extract_json(b"{invalide").is_none());
    }

    fn spawn_fake_http(responses: Vec<String>) -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let mut pending = responses.into_iter();
            for stream in listener.incoming() {
                let mut stream = match stream {
                    Ok(s) => s,
                    Err(_) => return,
                };
                let req = lire_requete(&mut stream);
                // sonde de port (TCP nu, pas de JSON-RPC) : on ferme sans consommer de réponse
                if !req.contains("\"method\"") {
                    continue;
                }
                let payload = match pending.next() {
                    Some(r) => r,
                    None => return,
                };
                let body = payload.into_bytes();
                let head = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(format!("{:x}\r\n", body.len()).as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.write_all(b"\r\n0\r\n\r\n");
                let _ = stream.flush();
            }
        });
        port
    }

    #[test]
    fn detect_framework_exige_les_deux_signatures() {
        // litemcp : protocol 2026-07-28 non négocié + /health « ok »
        assert_eq!(
            detect_framework(Some("2026-07-28"), true, false, false),
            Some("litemcp"),
            "les deux signatures = litemcp"
        );
        // un seul signal litemcp ne suffit pas (specs à jour ailleurs, /health génériques)
        assert_eq!(
            detect_framework(Some("2026-07-28"), false, false, false),
            None
        );
        // 2026-07-28 sans /health MAIS signature SSE+/sse complète → fastmcp
        assert_eq!(
            detect_framework(Some("2026-07-28"), false, true, true),
            Some("fastmcp-py")
        );
        assert_eq!(
            detect_framework(Some("2024-11-05"), true, false, false),
            None
        );
        assert_eq!(detect_framework(None, true, false, false), None);
        // fastmcp : réponses SSE + endpoint /sse legacy co-servi
        assert_eq!(
            detect_framework(Some("2024-11-05"), false, true, true),
            Some("fastmcp-py"),
            "SSE + /sse legacy = fastmcp"
        );
        // un seul des deux signaux fastmcp ne suffit pas
        assert_eq!(
            detect_framework(Some("2024-11-05"), false, true, false),
            None
        );
        assert_eq!(
            detect_framework(Some("2024-11-05"), false, false, true),
            None
        );
        // précédence : les deux fingerprints à la fois → litemcp (le plus spécifique)
        assert_eq!(
            detect_framework(Some("2026-07-28"), true, true, true),
            Some("litemcp")
        );
    }

    #[test]
    fn probe_mcp_any_path_trouve_racine_et_rejete_non_mcp() {
        // serveur MCP monté à la RACINE (ne répond QUE sur "POST / ")
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in l.incoming() {
                let Ok(mut s) = stream else { return };
                let req = lire_requete(&mut s);
                if !req.starts_with("POST / ") {
                    let _ = s.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                    continue;
                }
                let body = "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2024-11-05\",\"serverInfo\":{\"name\":\"racine\",\"version\":\"1.0\"}}}";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        let found = probe_mcp_any_path(port, 4000);
        assert!(
            found.is_some(),
            "un serveur monté à la racine doit être découvert"
        );
        let (path, p) = found.unwrap();
        assert_eq!(path, "/");
        assert_eq!(p.name.as_deref(), Some("racine"));

        // non-MCP (404 sur tous les chemins) → None
        let l2 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port2 = l2.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in l2.incoming() {
                let Ok(mut s) = stream else { return };
                let mut buf = [0u8; 16384];
                let _ = s.read(&mut buf);
                let _ = s.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        assert!(probe_mcp_any_path(port2, 2000).is_none());
    }

    #[test]
    fn parse_listening_ports_windows_linux_et_exclusions() {
        // Windows netstat
        let win = "\
  TCP    0.0.0.0:8080          0.0.0.0:0              LISTENING   1234\r\n\
  TCP    127.0.0.1:5091        0.0.0.0:0              LISTENING   5678\r\n\
  TCP    0.0.0.0:135           0.0.0.0:0              LISTENING   999\r\n\
  TCP    0.0.0.0:445           0.0.0.0:0              LISTENING   999\r\n\
  UDP    0.0.0.0:5353          *:*                                999\r\n";
        let mut p = parse_listening_ports(win);
        p.sort();
        assert_eq!(
            p,
            vec![5091, 8080],
            "ports MCP gardés, système (135/445/5353) écartés"
        );

        // Linux ss -tln
        let linux = "State  Recv-Q Send-Q Local Address:Port  Peer Address:Port\n\
LISTEN  0      128        0.0.0.0:3099       0.0.0.0:*\n\
LISTEN  0      128           [::]:22            [::]:*\n\
LISTEN  0      128              *:3141             *:*";
        let mut p2 = parse_listening_ports(linux);
        p2.sort();
        assert_eq!(
            p2,
            vec![3099, 3141],
            "port 22 système écarté, ports MCP gardés"
        );
    }

    #[test]
    fn parse_listening_ports_bsd_macos() {
        // macOS/BSD `netstat -an -p tcp` : séparateur '.', IPv6 "::1.port",
        // zone "%lo0" possible avant le point ; ports système (22, 135) écartés
        let bsd = "\
Active Internet connections (including servers)\n\
Proto Recv-Q Send-Q  Local Address          Foreign Address        (state)\n\
tcp4       0      0  127.0.0.1.3166         *.*                    LISTEN\n\
tcp6       0      0  ::1.3167               *.*                    LISTEN\n\
tcp4       0      0  *.3168                 *.*                    LISTEN\n\
tcp6       0      0  fe80::1%lo0.3169       *.*                    LISTEN\n\
tcp4       0      0  127.0.0.1.22           *.*                    LISTEN\n\
tcp4       0      0  127.0.0.1.135          *.*                    LISTEN\n";
        let mut p = parse_listening_ports(bsd);
        p.sort();
        assert_eq!(
            p,
            vec![3166, 3167, 3168, 3169],
            "format BSD : ports MCP gardés, système écartés"
        );
    }

    #[test]
    fn ssh_safe_rejete_les_injections() {
        assert_eq!(ssh_safe("192.168.1.42").as_deref(), Some("192.168.1.42"));
        assert_eq!(ssh_safe("mon-pc.local").as_deref(), Some("mon-pc.local"));
        assert_eq!(ssh_safe("Deamon_1").as_deref(), Some("Deamon_1"));
        assert_eq!(ssh_safe("::1").as_deref(), Some("::1"));
        // injection classique : tout caractère hors liste est rejeté
        assert_eq!(ssh_safe("x; rm -rf /"), None);
        assert_eq!(ssh_safe("a$(reboot)"), None);
        assert_eq!(ssh_safe("a b"), None);
        assert_eq!(ssh_safe(""), None);
        assert_eq!(ssh_safe("a\"b"), None);
    }

    #[test]
    fn health_ok_detecte_la_sonde_litemcp_et_rejete_le_reste() {
        // 1. sonde litemcp : 200 + corps « ok » → true
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
                let _ = s.write_all(resp.as_bytes());
            }
        });
        assert!(health_ok_host("127.0.0.1", port, 2000));

        // 2. /health qui répond autre chose → false
        let l2 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port2 = l2.local_addr().unwrap().port();
        thread::spawn(move || {
            if let Ok((mut s, _)) = l2.accept() {
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"status\":\"up\"}";
                let _ = s.write_all(resp.as_bytes());
            }
        });
        assert!(
            !health_ok_host("127.0.0.1", port2, 2000),
            "un /health JSON sans « ok » ne doit pas passer"
        );
    }

    #[test]
    fn probe_http_fingerprint_litemcp() {
        // Faux serveur litemcp : initialize répondu en 2026-07-28 (sans négocier),
        // tools/list, et GET /health → 200 « ok ». Le probe doit poser
        // framework = "litemcp" et inférer le langage rust.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let init = "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2026-07-28\",\"serverInfo\":{\"name\":\"hello\",\"version\":\"0.1.0\"}}}";
            let tools = "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[{\"name\":\"hello\",\"description\":\"Dit bonjour\"}]}}";
            let mut pending: Vec<&str> = vec![init, tools];
            for stream in listener.incoming() {
                let mut stream = match stream {
                    Ok(s) => s,
                    Err(_) => return,
                };
                let req = lire_requete(&mut stream);
                if req.starts_with("GET /health") {
                    let resp = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
                    let _ = stream.write_all(resp.as_bytes());
                    continue;
                }
                let Some(payload) = pending.first().copied() else {
                    return;
                };
                if req.contains("\"method\"") {
                    pending.remove(0);
                }
                let body = payload.as_bytes();
                let head = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: ";
                let _ = stream.write_all(
                    format!("{}{}\r\nConnection: close\r\n\r\n", head, body.len()).as_bytes(),
                );
                let _ = stream.write_all(body);
                let _ = stream.flush();
            }
        });
        let p = probe_http(port, "/mcp", 5000);
        assert!(p.mcp_up, "doit etre UP, erreur: {:?}", p.error);
        assert_eq!(
            p.framework.as_deref(),
            Some("litemcp"),
            "2026-07-28 non négocié + /health ok = litemcp"
        );
        assert_eq!(p.lang.as_deref(), Some("rust"), "litemcp = lib Rust");
        assert_eq!(p.protocol.as_deref(), Some("2026-07-28"));
    }

    #[test]
    fn probe_http_sans_sonde_health_pas_de_fingerprint() {
        // protocol 2026-07-28 mais /health sans réponse (le faux serveur ignore
        // les GET sans "method") : un seul signal → PAS de fingerprint.
        let port = spawn_fake_http(vec![
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2026-07-28\",\"serverInfo\":{\"name\":\"x\",\"version\":\"1\"}}}"
                .to_string(),
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[]}}".to_string(),
        ]);
        let p = probe_http(port, "/mcp", 5000);
        assert!(p.mcp_up);
        assert_eq!(
            p.framework, None,
            "sans /health « ok », pas de badge litemcp (un seul signal)"
        );
    }

    #[test]
    fn probe_http_initialise_et_liste_les_tools() {
        let init = "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2024-11-05\",\"serverInfo\":{\"name\":\"test-mcp\",\"version\":\"9.9.9\"}}}";
        let tools = "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[{\"name\":\"tool_a\",\"description\":\"premiere\"},{\"name\":\"tool_b\",\"description\":\"deuxieme\"}]}}";
        let port = spawn_fake_http(vec![init.to_string(), tools.to_string()]);
        let p = probe_http(port, "/mcp", 5000);
        assert!(p.mcp_up, "doit etre UP, erreur: {:?}", p.error);
        assert_eq!(p.name.as_deref(), Some("test-mcp"));
        assert_eq!(p.version.as_deref(), Some("9.9.9"));
        assert_eq!(p.protocol.as_deref(), Some("2024-11-05"));
        assert_eq!(p.tools.len(), 2);
        assert_eq!(p.tools[0].name, "tool_a");
        assert_eq!(
            p.host.as_deref(),
            Some("127.0.0.1"),
            "fallback ::1 -> 127.0.0.1"
        );
        assert!(p.latency_ms < 5000);
    }

    #[test]
    fn probe_http_rejete_un_port_non_mcp() {
        let port = spawn_fake_http(vec!["Use /health".to_string(), "Use /health".to_string()]);
        let p = probe_http(port, "/mcp", 3000);
        assert!(!p.mcp_up, "un serveur non-MCP ne doit pas etre UP");
        assert!(p.error.is_some());
    }

    #[test]
    fn probe_peer_accepte_le_format_api_state() {
        // contrat (deux phases, audit 2026-10-24) : /api/hello donne un nonce,
        // puis /api/state (avec ou sans preuve selon le code disponible) répond
        // "servers" + "stats". Le nonce doit être UNIQUEMENT utilisé, jamais un secret.
        let corps = r#"{"servers":[{"id":"seed-overmind","label":"Overmind","transport":"http","transport_info":{"host":null,"port":3099,"path":"/mcp"},"enabled":true,"state":"UP","server_name":"OverMind-MCP","version":"3.10.5","tools_count":16}],"log":[],"stats":{"up":4,"total":7,"tools":59},"settings":{"refresh_secs":15}}"#;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let corps = corps.to_string();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { return };
                let req = lire_requete(&mut s);
                let resp = if req.starts_with("GET /api/hello") {
                    let body = r#"{"app":"mcp-control","version":"0.7.1","nonce":"aabbccdd00112233aabbccdd00112233aabbccdd00112233aabbccdd00112233"}"#;
                    format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body)
                } else if req.starts_with("POST /api/state") {
                    // aucun code en clair ne doit transiter : seulement nonce + preuve
                    assert!(
                        !req.contains("X-MCC-Access:"),
                        "le code d'accès ne doit JAMAIS être envoyé au pair"
                    );
                    format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", corps.len(), corps)
                } else {
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_string()
                };
                let _ = s.write_all(resp.as_bytes());
            }
        });
        let pr = probe_peer("127.0.0.1", port, None);
        assert!(
            pr.alive,
            "un moteur MCP Control doit être détecté comme pair"
        );
        let servers = pr.servers.expect("servers doit être présent");
        assert_eq!(servers["stats"]["total"], serde_json::json!(7));
        assert_eq!(
            servers["servers"][0]["server_name"],
            serde_json::json!("OverMind-MCP")
        );
    }

    #[test]
    fn probe_peer_rejete_un_serveur_qui_ne_est_pas_un_moteur_control() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let _ = lire_requete(&mut s);
                let body = b"Use /health";
                let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), String::from_utf8_lossy(body));
                let _ = s.write_all(resp.as_bytes());
            }
        });
        let pr = probe_peer("127.0.0.1", port, None);
        assert!(!pr.alive, "un service étranger ne doit pas être un pair");
        assert!(pr.servers.is_none());
    }

    #[test]
    fn probe_http_port_ferme_est_down() {
        // Un port rendu par bind(:0) puis RELÂCHÉ peut être repris par un autre
        // test parallèle entre le drop et la sonde (collision de ports
        // éphémères, audit 01 constat 7) : on ne conclut que sur un port resté
        // fermé, en retirant un port si celui-ci a été repris.
        for _ in 0..8 {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            drop(listener);
            let p = probe_http(port, "/mcp", 800);
            if p.port_open {
                continue; // port repris par un test parallèle : on retire
            }
            assert!(!p.mcp_up, "un port fermé ne doit jamais être UP");
            return;
        }
        panic!("aucun port resté fermé après 8 tirages (machine anormalement chargée)");
    }

    #[test]
    fn probe_http_reponse_courte_sans_separateur_ne_panique_pas() {
        // Régression audit 2026-10-25 : une réponse de 1-3 octets sans CRLFCRLF
        // faisait panicer sur `data[sep + 4..]` → le thread du POLLER mourait
        // en silence et la supervision des serveurs s'arrêtait définitivement.
        // Le probe doit renvoyer une erreur, jamais paniquer.
        for reponse in [
            b"X".to_vec(),
            b"OK".to_vec(),
            b"abc".to_vec(),
            b"\r\n".to_vec(),
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let value = reponse.clone();
            thread::spawn(move || {
                for _ in 0..3 {
                    let Ok((mut s, _)) = listener.accept() else {
                        return;
                    };
                    let mut buf = [0u8; 8192];
                    let _ = s.read(&mut buf);
                    let _ = s.write_all(&value);
                    let _ = s.flush();
                }
            });
            let p = probe_http(port, "/mcp", 2000);
            assert!(!p.mcp_up, "une réponse non-HTTP ne doit pas être UP");
            assert!(
                p.error.is_some(),
                "le probe doit signaler une erreur, pas paniquer (réponse {:?})",
                String::from_utf8_lossy(&reponse)
            );
        }
    }

    #[test]
    fn pids_listening_on_parse_par_colonne_exacte() {
        // Régression audit : ":3141" matchait aussi ":31410" par sous-chaîne → taskkill du mauvais PID
        let own = std::process::id().to_string();
        let netstat = format!(
            "  Proto  Adresse locale        Adresse distante  État        PID\r\n\
             TCP    0.0.0.0:3141          0.0.0.0:0         LISTENING   1111\r\n\
             TCP    0.0.0.0:31410         0.0.0.0:0         LISTENING   2222\r\n\
             TCP    [::]:3141             [::]:0            LISTENING   3333\r\n\
             TCP    127.0.0.1:5090        0.0.0.0:0         LISTENING   {}\r\n\
             UDP    0.0.0.0:3141          *:*                             4444\r\n",
            own
        );
        let mut pids = pids_listening_on(&netstat, 3141);
        pids.sort();
        assert_eq!(
            pids,
            vec![1111, 3333],
            ":31410 ne doit pas matcher, ni la ligne UDP, ni notre propre PID"
        );
        assert_eq!(pids_listening_on(&netstat, 31410), vec![2222]);
        assert!(
            pids_listening_on(&netstat, 5090).is_empty(),
            "notre propre PID est exclu (pas de suicide si l'id vise notre port UI)"
        );
    }

    #[test]
    fn pids_listening_on_ss_parse_par_colonne_exacte() {
        // Régression audit : kill_port était un no-op sur unix — parse ss -tlnp
        // par port exact, propre PID exclu (même contrat que pids_listening_on).
        let own = std::process::id().to_string();
        let ss = format!(
            "State  Recv-Q Send-Q Local Address:Port Peer Address:Port Process\n\
             LISTEN 0      128          0.0.0.0:3141       0.0.0.0:*    users:((\"node\",pid=1111,fd=20))\n\
             LISTEN 0      128        [::]:3141            [::]:*       users:((\"python3\",pid=2222,fd=19))\n\
             LISTEN 0      128          0.0.0.0:31410      0.0.0.0:*    users:((\"node\",pid=3333,fd=18))\n\
             LISTEN 0      128        127.0.0.1:5090       0.0.0.0:*    users:((\"mcp-control\",pid={},fd=12))\n",
            own
        );
        let mut pids = pids_listening_on_ss(&ss, 3141);
        pids.sort();
        assert_eq!(
            pids,
            vec![1111, 2222],
            ":31410 ne doit pas matcher, notre propre PID exclu"
        );
        assert_eq!(pids_listening_on_ss(&ss, 31410), vec![3333]);
        assert!(
            pids_listening_on_ss(&ss, 5090).is_empty(),
            "notre propre PID est exclu (pas de suicide)"
        );
    }

    #[test]
    fn control_ui_at_detecte_le_moteur_et_rejete_les_etrangers() {
        // un moteur MCP Control (hello avec app=mcp-control) → Some(url)
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in l.incoming() {
                let Ok(mut s) = stream else { return };
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let req = String::from_utf8_lossy(&buf).to_string();
                let body = if req.starts_with("GET /api/hello") {
                    r#"{"app":"mcp-control","version":"0.7.2","nonce":"aa"}"#.to_string()
                } else {
                    String::new()
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        assert!(
            control_ui_at(port).is_some(),
            "un moteur MCP Control doit être détecté (reconnexion au lieu d'un second)"
        );

        // un occupant étranger (autre logiciel HTTP) → None
        let l2 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port2 = l2.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in l2.incoming() {
                let Ok(mut s) = stream else { return };
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let body = "<html>une autre application</html>";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        assert!(
            control_ui_at(port2).is_none(),
            "un service étranger ne doit pas être pris pour un moteur"
        );

        // rien n'écoute → None (rapide, sans panic)
        let l3 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port3 = l3.local_addr().unwrap().port();
        drop(l3);
        assert!(control_ui_at(port3).is_none());
    }

    #[test]
    fn control_ui_at_hosts_trouve_un_moteur_bindi_sur_une_ip_precise() {
        // Serveur qui ne répond QUE sur l'adresse de loopback : on vérifie
        // que la liste d'hôtes est bien respectée (le bind configuré d'abord).
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in l.incoming() {
                let Ok(mut s) = stream else { return };
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let body = r#"{"app":"mcp-control","version":"0.7.2","nonce":"x"}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        // Le bind configuré est prioritaire : il est trouvé.
        assert!(control_ui_at_hosts(port, &["127.0.0.1", "::1"]).is_some());
        // Un bind sur une adresse qui n'écoute pas → pas de faux positif.
        assert!(control_ui_at_hosts(port, &["192.0.2.1"]).is_none());
    }

    #[test]
    fn sanitize_http_path_et_host_header_bloquent_crlf_et_bracketent_ipv6() {
        assert_eq!(http_host_header("::1", 8080), "[::1]:8080");
        assert_eq!(http_host_header("127.0.0.1", 8080), "127.0.0.1:8080");
        assert_eq!(sanitize_http_path("/mcp", "/mcp"), "/mcp");
        assert_eq!(sanitize_http_path("/", "/mcp"), "/");
        assert_eq!(
            sanitize_http_path("/mcp HTTP/1.1\r\nX-Inj: 1", "/mcp"),
            "/mcp"
        );
        assert_eq!(sanitize_http_path("/mcp\r\nX-Inj: 1", "/mcp"), "/mcp");
        assert_eq!(sanitize_http_path("sans-slash", "/mcp"), "/mcp");
        assert_eq!(sanitize_http_path("", "/sse"), "/sse");

        // Vérifie que ws_upgrade_host n'injecte jamais d'en-tête sur un path hostile
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let _ = tx.send(lire_requete(&mut s));
                let _ = s.write_all(b"HTTP/1.1 101 Switching Protocols\r\n\r\n");
            }
        });
        assert!(ws_upgrade_host(
            "127.0.0.1",
            port,
            "/mcp HTTP/1.1\r\nX-Injected: pwn",
            1000
        ));
        let req = rx.recv().unwrap();
        assert!(
            req.starts_with("GET /mcp HTTP/1.1\r\n"),
            "le path hostile doit retomber sur /mcp: {req}"
        );
        assert!(
            !req.contains("X-Injected"),
            "aucun en-tête injecté ne doit apparaître"
        );
        assert!(
            req.contains(&format!("Host: 127.0.0.1:{port}\r\n")),
            "Host doit inclure le port: {req}"
        );
    }

    #[test]
    fn read_bounded_line_plafonne_avant_allocation() {
        // Une ligne de 5 Mo sans '\n' ne doit jamais dépasser MAX_PROBE_READ (2 Mo) dans buf
        let big = vec![b'A'; MAX_PROBE_READ * 2 + 1234];
        let mut cursor = std::io::BufReader::new(std::io::Cursor::new(big));
        let mut buf = Vec::new();
        let n = read_bounded_line(&mut cursor, &mut buf).unwrap();
        assert!(n > 0);
        assert_eq!(buf.len(), MAX_PROBE_READ);
        // Une ligne normale avec '\n' est lue intégralement
        let normal = b"{\"jsonrpc\":\"2.0\",\"id\":1}\nreste";
        let mut cursor2 = std::io::BufReader::new(std::io::Cursor::new(&normal[..]));
        let n2 = read_bounded_line(&mut cursor2, &mut buf).unwrap();
        assert_eq!(n2, 25);
        assert_eq!(&buf[..], b"{\"jsonrpc\":\"2.0\",\"id\":1}\n");
    }

    #[test]
    fn probe_http_et_host_propagent_mcp_session_id() {
        // Serveur MCP stateful : émet Mcp-Session-Id (casse mixte) sur initialize
        // et REJETTE tools/list si Mcp-Session-Id est absent ou altéré.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { return };
                let req = lire_requete(&mut s);
                if req.contains("\"initialize\"") {
                    let body = "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2025-03-26\",\"serverInfo\":{\"name\":\"stateful-mcp\",\"version\":\"1.0.0\"}}}";
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMcp-Session-Id: Sess_AbCd-1234\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = s.write_all(resp.as_bytes());
                } else if req.contains("\"tools/list\"") {
                    let has_sid = req
                        .lines()
                        .any(|l| l.trim() == "Mcp-Session-Id: Sess_AbCd-1234");
                    let body = if has_sid {
                        "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[{\"name\":\"secret_tool\",\"description\":\"ok\"}]}}"
                    } else {
                        "{\"jsonrpc\":\"2.0\",\"id\":2,\"error\":{\"code\":-32000,\"message\":\"missing session\"}}"
                    };
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = s.write_all(resp.as_bytes());
                } else {
                    let _ = s.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                }
            }
        });
        let p = probe_http(port, "/mcp", 4000);
        assert!(p.mcp_up);
        assert_eq!(
            p.tools.len(),
            1,
            "probe_http doit rejouer Mcp-Session-Id sur tools/list"
        );
        assert_eq!(p.tools[0].name, "secret_tool");

        let p2 = probe_http_host("127.0.0.1", port, "/mcp", 4000);
        assert!(p2.mcp_up);
        assert_eq!(
            p2.tools.len(),
            1,
            "probe_http_host doit rejouer Mcp-Session-Id sur tools/list"
        );
    }

    #[test]
    fn ssh_error_hint_distingue_socket_bloque_et_cle_refusee() {
        assert_eq!(
            ssh_error_hint("ssh: connect to host 192.168.2.159 port 22: Permission denied"),
            " — machine injoignable (LAN coupé / VPN actif, ou machine éteinte)"
        );
        assert_eq!(
            ssh_error_hint("ssh: connect to host 192.168.2.159 port 22: Connection timed out"),
            " — machine injoignable (LAN coupé / VPN actif, ou machine éteinte)"
        );
        assert!(
            ssh_error_hint("user@192.168.2.159: Permission denied (publickey).")
                .contains("clé SSH refusée")
        );
    }

    /// Port muet : accepte les connexions IPv4 et ne répond JAMAIS (chaque
    /// socket acceptée est gardée ouverte pour que la lecture client expire).
    fn fixture_muet() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind muet");
        let port = listener.local_addr().expect("local_addr muet").port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                // on garde la socket ouverte sans jamais lire ni écrire :
                // le client attend son délai (comme un service figé réel)
                std::mem::forget(stream);
            }
        });
        port
    }

    #[test]
    fn probe_http_publie_l_erreur_de_l_hote_qui_a_accepte() {
        // Régression audit 03 constat 4 : un port muet (accepte sur 127.0.0.1,
        // ne répond jamais) ne doit PAS afficher « connexion refusée » —
        // l'erreur de l'hôte de repli (::1, jamais joint) ne doit pas écraser
        // celle de l'hôte qui a accepté la connexion (délai dépassé).
        let port = fixture_muet();
        let p = probe_http(port, "/mcp", 900);
        assert!(!p.mcp_up);
        let err = p.error.expect("erreur attendue");
        assert!(
            !err.contains("10061") && !err.contains("refus"),
            "l'erreur de l'hôte de repli ne doit pas être publiée : {err}"
        );
        assert!(
            err.contains("127.0.0.1"),
            "l'erreur doit venir de l'hôte qui a accepté la connexion : {err}"
        );
    }

    #[test]
    fn probe_http_port_muet_une_seule_forme_envoyee() {
        // Régression lot A : un port muet (accepte, ne répond jamais) doit être
        // classé après UNE SEULE forme de sonde. L'ancien code rejouait les
        // 3 formes (legacy, strict, server/discover) — le muet encaissait
        // ~18-25 s (3 × timeout) et fixait la période du cycle entier.
        // Comptage par connexions acceptées : 1 port_open + 1 forme = 2
        // (l'ancien comportement en donnait 4 : port_open + 3 formes).
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind muet lot A");
        let port = listener.local_addr().expect("local_addr muet lot A").port();
        let compteur = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c2 = compteur.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                c2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                std::mem::forget(stream); // accepte, ne répond jamais
            }
        });
        let p = probe_http(port, "/mcp", 900);
        assert!(!p.mcp_up);
        let n = compteur.load(std::sync::atomic::Ordering::SeqCst);
        // 2 = la sonde de port + la forme unique. Le seuil rejette l'ancien
        // comportement (3 formes rejouées = 4 et plus). La tolérance à 1
        // couvre les sondes qui n'ouvrent qu'une connexion ; la cause des
        // flakes entre tests parallèles (port relâché puis repris, requête
        // lue à moitié) est corrigée dans les fixtures (lire_requete +
        // retirage des ports relâchés), pas ici.
        assert!(
            n <= 2,
            "un port muet ne doit subir qu'une seule forme de sonde (port_open + 1 forme), \
             pas les 3 formes rejouées de l'ancien code — compté : {n}"
        );
        let err = p.error.expect("erreur attendue");
        assert!(
            err.contains("127.0.0.1"),
            "l'erreur publiée doit venir de la forme qui a expiré : {err}"
        );
    }

    #[test]
    fn http_post_publie_l_erreur_de_l_hote_qui_a_accepte() {
        // Régression lot A (même défaut que probe_http, corrigé au lot 2) :
        // un port muet sur ::1 (127.0.0.1 fermé) ne doit PAS afficher le
        // « connexion refusée » du repli 127.0.0.1 — l'erreur de l'hôte qui
        // a accepté la connexion (délai dépassé) doit être publiée.
        let listener = std::net::TcpListener::bind("[::1]:0").expect("bind muet ::1");
        let port = listener.local_addr().expect("local_addr muet ::1").port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                std::mem::forget(stream); // accepte, ne répond jamais
            }
        });
        let err = match http_post(port, "/mcp", "{}", 900) {
            Err(e) => e,
            Ok(_) => panic!("un port muet doit échouer"),
        };
        assert!(
            err.contains("::1"),
            "l'erreur doit venir de l'hôte qui a accepté (::1) : {err}"
        );
        assert!(
            !err.contains("10061") && !err.contains("refus"),
            "l'erreur du repli (127.0.0.1 refusé) ne doit pas être publiée : {err}"
        );
    }

    #[test]
    fn reponse_au_dela_du_plafond_message_explicite() {
        // Régression audit 02 constat 6 : une réponse dépassant le plafond de
        // lecture (2 Mio) est refusée avec un message de dépassement de taille
        // explicite — plus jamais « réponse non JSON (0 octets) ».
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind gros");
        let port = listener.local_addr().expect("local_addr gros").port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { return };
                let mut buf = [0u8; 16384];
                let _ = s.read(&mut buf);
                let taille = 3 * 1024 * 1024;
                let corps = vec![b'A'; taille];
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {taille}\r\nConnection: close\r\n\r\n"
                );
                let _ = s.write_all(head.as_bytes());
                // le client coupe sa lecture au plafond : l'écriture peut donc
                // échouer (connexion fermée), on l'ignore
                let _ = s.write_all(&corps);
            }
        });
        let err = match rpc_call_host(
            "127.0.0.1",
            port,
            "/mcp",
            "initialize",
            serde_json::json!({"protocolVersion": "2024-11-05"}),
            1,
            8000,
        ) {
            Err(e) => e,
            Ok(_) => panic!("la réponse de 3 Mio doit être refusée"),
        };
        assert!(
            err.contains("plafond"),
            "message de dépassement explicite attendu : {err}"
        );
        assert!(err.contains("2 Mio"), "le plafond doit être nommé : {err}");
        assert!(
            !err.contains("réponse non JSON"),
            "l'ancien message trompeur ne doit plus apparaître : {err}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn system_tool_utilise_chemin_absolu_et_ignore_le_dossier_courant() {
        for nom in [
            "netstat",
            "tasklist",
            "taskkill",
            "reg",
            "icacls",
            "powershell",
            "explorer",
            "rundll32",
            "ipconfig",
            "ping",
            "arp",
        ] {
            let p = system_tool(nom);
            assert!(p.is_absolute(), "{nom} doit être un chemin absolu : {p:?}");
            assert!(
                p.exists(),
                "{nom} introuvable à l'emplacement système : {p:?}"
            );
        }
        // Négatif : un faux reg.exe déposé dans le dossier de travail n'est pas
        // pris par system_command (seul System32\reg.exe est exécuté).
        let dir = std::env::temp_dir().join(format!("mcc-systool-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let leurre = dir.join("reg.exe");
        std::fs::write(&leurre, b"ceci n'est pas un executable valide").unwrap();
        assert_ne!(system_tool("reg"), leurre);
        let out = system_command("reg")
            .arg("/?")
            .current_dir(&dir)
            .output()
            .expect("reg.exe système");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            out.status.success(),
            "le vrai reg.exe doit répondre même avec un leurre dans cwd"
        );
    }

    #[test]
    fn resolve_npx_in_dir_trouve_node_et_npx_cli_ou_rend_none() {
        let dir = std::env::temp_dir().join(format!("mcc-npx-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(resolve_npx_in_dir(&dir), None);

        std::fs::write(dir.join("node.exe"), b"").unwrap();
        assert_eq!(resolve_npx_in_dir(&dir), None);

        let cli = dir
            .join("node_modules")
            .join("npm")
            .join("bin")
            .join("npx-cli.js");
        std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
        std::fs::write(&cli, b"// npx").unwrap();

        let (node_path, cli_path) = resolve_npx_in_dir(&dir).expect("doit résoudre node + npx-cli");
        assert!(node_path.ends_with("node.exe"), "{node_path}");
        assert!(cli_path.ends_with("npx-cli.js"), "{cli_path}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn call_stdio_rpc_ignore_bom_et_requete_entrante_meme_id() {
        let (tx_in, rx_in) = std::sync::mpsc::channel::<String>();
        let (tx_out, rx_out) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in rx_in {
                let req: Value = serde_json::from_str(line.trim()).unwrap();
                let id = req["id"].as_u64().unwrap();
                // 1. Requête serveur->client partageant le même id (à ignorer)
                let _ = tx_out.send(
                    json!({"jsonrpc":"2.0","id":id,"method":"roots/list","params":{}}).to_string(),
                );
                // 2. Bruit non-JSON
                let _ = tx_out.send("debug banner".to_string());
                // 3. Réponse valide précédée d'un BOM UTF-8
                let _ = tx_out.send(format!(
                    "\u{feff}{}",
                    json!({"jsonrpc":"2.0","id":id,"result":{"ok":true,"echo":req["method"]}})
                ));
            }
        });
        let rpc = std::sync::Arc::new(std::sync::Mutex::new(StdioRpc {
            stdin: Box::new(LineChannelWriter {
                tx: tx_in,
                buf: Vec::new(),
            }),
            rx: rx_out,
            next_id: 100,
        }));
        let (res, _ms) = call_stdio_rpc(&rpc, "tools/list", json!({}), 2_000).unwrap();
        assert_eq!(res["ok"], json!(true));
        assert_eq!(res["echo"], json!("tools/list"));
    }

    #[test]
    fn posix_shell_quote_et_ssh_path_safe_protegent_des_injections() {
        assert_eq!(posix_shell_quote("simple").unwrap(), "'simple'");
        assert_eq!(
            posix_shell_quote("a'b $PATH `id`").unwrap(),
            "'a'\\''b $PATH `id`'"
        );
        assert!(posix_shell_quote("bad\nline").is_err());
        assert!(posix_shell_quote("bad\0nul").is_err());

        assert_eq!(
            ssh_path_safe(" ~/.ssh/id_ed25519 "),
            Some("~/.ssh/id_ed25519".to_string())
        );
        assert_eq!(ssh_path_safe("-oProxyCommand=evil"), None);
        assert_eq!(ssh_path_safe("key\n-oEvil"), None);
        assert_eq!(ssh_path_safe(""), None);
    }

    #[test]
    fn build_ssh_args_gere_port_cle_et_batchmode() {
        let t_def = SshTarget::simple("192.168.1.50", "ubuntu");
        let args_def = build_ssh_args(&t_def, "echo ok", 8).unwrap();
        assert!(!args_def.contains(&"-p".to_string()));
        assert!(!args_def.contains(&"-i".to_string()));
        assert_eq!(args_def.last().map(String::as_str), Some("echo ok"));

        let t_22 = SshTarget {
            host: "192.168.1.50",
            user: "ubuntu",
            port: Some(22),
            identity_file: None,
        };
        assert!(!build_ssh_args(&t_22, "echo ok", 8)
            .unwrap()
            .contains(&"-p".to_string()));

        let t_custom = SshTarget {
            host: "192.168.1.50",
            user: "ubuntu",
            port: Some(2222),
            identity_file: Some("C:\\Users\\demo\\.ssh\\id_ed25519"),
        };
        let args_c = build_ssh_args(&t_custom, "echo ok", 8).unwrap();
        assert!(args_c.windows(2).any(|w| w == ["-p", "2222"]));
        assert!(args_c
            .windows(2)
            .any(|w| w == ["-i", "C:\\Users\\demo\\.ssh\\id_ed25519"]));
        assert!(args_c.contains(&"BatchMode=yes".to_string()));
    }

    #[test]
    fn parse_remote_env_output_linux_macos_windows_et_cli() {
        let linux_out = "__MCC_OK__\nOS=Linux\nARCH=x86_64\nCLI=/home/u/.local/share/mcp-control/mcp-control\nCFGDIR=/home/u/.local/share/mcp-control\nPORT=5090\nHELLO={\"app\":\"mcp-control\",\"version\":\"0.8.8\",\"nonce\":\"abc\"}\n";
        let info = parse_remote_env_output(linux_out).unwrap();
        assert_eq!(info.os, "linux");
        assert_eq!(info.arch, "x86_64");
        assert_eq!(
            info.cli_path.as_deref(),
            Some("/home/u/.local/share/mcp-control/mcp-control")
        );
        assert!(info.cli_running);
        assert_eq!(info.cli_port, Some(5090));
        assert_eq!(info.cli_version.as_deref(), Some("0.8.8"));

        let mac_absent = "__MCC_OK__\nOS=Darwin\nARCH=arm64\nCLI=\nCFGDIR=\nPORT=\nHELLO=\n";
        let info2 = parse_remote_env_output(mac_absent).unwrap();
        assert_eq!(info2.os, "macos");
        assert_eq!(info2.arch, "aarch64");
        assert_eq!(info2.cli_path, None);
        assert!(!info2.cli_running);
        assert_eq!(info2.cli_port, None);

        let win_out = "__MCC_OK__\nOS=windows\nARCH=AMD64\nCLI=C:\\Users\\u\\AppData\\Local\\mcp-control\\mcp-control.exe\nCFGDIR=C:\\Users\\u\\AppData\\Local\\mcp-control\nPORT=\nHELLO=\n";
        let info3 = parse_remote_env_output(win_out).unwrap();
        assert_eq!(info3.os, "windows");
        assert_eq!(info3.arch, "x86_64");
        assert!(info3.cli_path.is_some());
        assert!(!info3.cli_running);

        assert!(parse_remote_env_output("bad").is_err());
    }

    #[test]
    fn ssh_install_cli_refuse_url_ou_sha256_invalides() {
        let t = SshTarget::simple("192.0.2.10", "demo");
        let ok_sha = "a".repeat(64);
        let ok_url = "https://github.com/DeamonDev888/mcp-control/releases/download/v0.8.8/mcp-control-cli-linux-x86_64";
        assert!(validate_cli_install_params(ok_url, &ok_sha).is_ok());

        // Mauvaise URL ou injection
        assert!(ssh_install_cli(&t, "linux", "https://evil.example/bin", &ok_sha).is_err());
        assert!(ssh_install_cli(&t, "linux", &format!("{ok_url};id"), &ok_sha).is_err());
        // Mauvais SHA-256
        assert!(ssh_install_cli(&t, "linux", ok_url, "abcd").is_err());
        assert!(ssh_install_cli(&t, "linux", ok_url, &"Z".repeat(64)).is_err());
    }

    #[test]
    fn parse_remote_port_listeners_ss_lsof_et_netstat() {
        let ss = "LISTEN 0 128 127.0.0.1:3099 0.0.0.0:* users:((\"node\",pid=4321,fd=19))\n";
        assert_eq!(
            parse_remote_port_listeners(ss, 3099),
            vec![(4321, "node".to_string())]
        );

        let lsof = "python3 8877 demo 9u IPv4 0x123 0t0 TCP 127.0.0.1:8787 (LISTEN)\n";
        assert_eq!(
            parse_remote_port_listeners(lsof, 8787),
            vec![(8877, "python3".to_string())]
        );

        let win = "  TCP    127.0.0.1:4500         0.0.0.0:0              LISTENING       9012\n";
        assert_eq!(
            parse_remote_port_listeners(win, 4500),
            vec![(9012, "remote".to_string())]
        );
    }

    #[test]
    fn detect_container_cmd_docker_podman_wrappers_et_negatifs() {
        let s = |arr: &[&str]| arr.iter().map(|x| x.to_string()).collect::<Vec<_>>();

        assert_eq!(
            detect_container_cmd("docker", &s(&["run", "-i", "--rm", "mcp/github"])),
            (true, None)
        );
        assert_eq!(
            detect_container_cmd("docker", &s(&["run", "--name", "my-mcp", "-i", "img"])),
            (true, Some("my-mcp".to_string()))
        );
        assert_eq!(
            detect_container_cmd("docker", &s(&["run", "-i", "--name=eq-mcp", "img"])),
            (true, Some("eq-mcp".to_string()))
        );
        assert_eq!(
            detect_container_cmd("docker", &s(&["exec", "-i", "my-ctr", "node", "srv.js"])),
            (true, Some("my-ctr".to_string()))
        );
        assert_eq!(
            detect_container_cmd(
                r"C:\Program Files\Docker\Docker\resources\bin\docker.exe",
                &s(&["run", "--name", "win-mcp", "img"]),
            ),
            (true, Some("win-mcp".to_string()))
        );
        assert_eq!(
            detect_container_cmd(
                "docker-compose",
                &s(&["-f", "compose.yml", "exec", "-T", "svc", "cmd"]),
            ),
            (true, Some("svc".to_string()))
        );
        assert_eq!(
            detect_container_cmd("podman", &s(&["run", "--rm", "-i", "mcp/git"])),
            (true, None)
        );
        assert_eq!(
            detect_container_cmd(
                "wsl",
                &s(&["-d", "Ubuntu", "--", "docker", "run", "--name", "wsl-mcp", "img"]),
            ),
            (true, Some("wsl-mcp".to_string()))
        );
        assert_eq!(
            detect_container_cmd("cmd", &s(&["/c", "docker", "run", "-i", "img"])),
            (true, None)
        );
        assert_eq!(
            detect_container_cmd("sh", &s(&["-c", "docker run --name sh-mcp img"])),
            (true, Some("sh-mcp".to_string()))
        );

        // Négatifs : outils hôtes qui gèrent Docker sans tourner dedans
        assert_eq!(
            detect_container_cmd("npx", &s(&["-y", "@modelcontextprotocol/server-docker"])),
            (false, None)
        );
        assert_eq!(
            detect_container_cmd("uvx", &s(&["docker-mcp"])),
            (false, None)
        );
        assert_eq!(
            detect_container_cmd("node", &s(&[r"C:\docker\index.js"])),
            (false, None)
        );
    }

    #[test]
    fn parse_docker_ps_ports_ipv4_ipv6_ranges_et_filtres() {
        let out = "\
mcp-a|0.0.0.0:3000->3000/tcp, 127.0.0.1:5433->5433/tcp\n\
mcp-b|[::]:8080->8080/tcp, :::8081->8081/tcp, 3001/tcp\n\
mcp-range|0.0.0.0:9000-9002->9000-9002/tcp, 0.0.0.0:5353->5353/udp\n";
        let map = parse_docker_ps_ports(out);
        assert_eq!(map.get(&3000).map(String::as_str), Some("mcp-a"));
        assert_eq!(map.get(&5433).map(String::as_str), Some("mcp-a"));
        assert_eq!(map.get(&8080).map(String::as_str), Some("mcp-b"));
        assert_eq!(map.get(&8081).map(String::as_str), Some("mcp-b"));
        assert_eq!(map.get(&3001), None, "port exposé sans mapping hôte ignoré");
        assert_eq!(map.get(&9000).map(String::as_str), Some("mcp-range"));
        assert_eq!(map.get(&9001).map(String::as_str), Some("mcp-range"));
        assert_eq!(map.get(&9002).map(String::as_str), Some("mcp-range"));
        assert_eq!(map.get(&5353), None, "port UDP ignoré");
    }

    #[test]
    fn is_container_proxy_image_et_cgroup_detection() {
        for img in [
            "com.docker.backend.exe",
            "docker-proxy",
            "vpnkit.exe",
            "gvproxy.exe",
        ] {
            assert!(is_container_proxy_image(img), "{img} attendu proxy conteneur");
        }
        for img in ["node.exe", "python.exe", "wslrelay.exe"] {
            assert!(
                !is_container_proxy_image(img),
                "{img} ne doit PAS être classé proxy conteneur"
            );
        }

        assert!(is_cgroup_in_container("12:pids:/docker/3f8a1234abcd\n"));
        assert!(is_cgroup_in_container(
            "0::/system.slice/docker-3f8a1234abcd.scope\n"
        ));
        assert!(is_cgroup_in_container(
            "0::/kubepods.slice/kubepods-burstable.slice/cri-containerd-abc.scope\n"
        ));
        assert!(is_cgroup_in_container(
            "0::/user.slice/user-1000.slice/libpod-abcdef.scope/container\n"
        ));
        assert!(!is_cgroup_in_container("0::/system.slice/docker.service\n"));
        assert!(!is_cgroup_in_container("0::/user.slice/user-1000.slice\n"));
    }

    #[test]
    fn parse_ssh_container_probe_toutes_sources() {
        assert_eq!(
            parse_ssh_container_probe("mcp-remote|0.0.0.0:3100->3100/tcp\n", "", 3100),
            (true, Some("mcp-remote".to_string()))
        );
        let ss_proxy = "LISTEN 0 128 0.0.0.0:3100 0.0.0.0:* users:((\"docker-proxy\",pid=999,fd=4))\n";
        assert_eq!(parse_ssh_container_probe("", ss_proxy, 3100), (true, None));
        let ss_cg = "LISTEN 0 128 127.0.0.1:3100 0.0.0.0:* users:((\"node\",pid=777,fd=8))\n0::/system.slice/docker-abc.scope\n";
        assert_eq!(parse_ssh_container_probe("", ss_cg, 3100), (true, None));
        let ss_host = "LISTEN 0 128 127.0.0.1:3100 0.0.0.0:* users:((\"node\",pid=777,fd=8))\n0::/user.slice/user-1000.slice\n";
        assert_eq!(parse_ssh_container_probe("", ss_host, 3100), (false, None));
    }
}
