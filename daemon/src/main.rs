mod plugins;

use clap::Parser;
use intent::{Method, Request, Response, ResponseResult, ResponseError};
use std::path::PathBuf;
use std::fs;
use std::sync::{Mutex, OnceLock};
use tokio::net::{UnixListener, UnixStream};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{info, error, warn};

fn active_sandboxes() -> &'static Mutex<Vec<PathBuf>> {
    static INSTANCE: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
    INSTANCE.get_or_init(|| Mutex::new(Vec::new()))
}

#[derive(Parser, Debug)]
#[command(author, version, about = "DebAI System Daemon")]
struct Args {
    /// Path to the Unix domain socket
    #[arg(short, long, default_value = "/tmp/debai_aid.sock")]
    socket: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    let socket_path = args.socket;

    // Clean up existing socket file if it exists
    if socket_path.exists() {
        info!("Removing existing socket file: {:?}", socket_path);
        fs::remove_file(&socket_path)?;
    }

    // Ensure parent directory exists
    if let Some(parent) = socket_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }

    info!("Starting DebAI Daemon (aid)...");
    let listener = UnixListener::bind(&socket_path)?;
    
    // Set socket permissions to 0666 so all local users can write/connect to it
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = fs::metadata(&socket_path) {
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o666);
        if let Err(e) = fs::set_permissions(&socket_path, permissions) {
            warn!("Failed to set permissions on socket: {:?}", e);
        }
    }
    
    info!("Listening on Unix socket: {:?}", socket_path);

    // Register signal handlers for clean exit
    let socket_path_clone = socket_path.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        info!("Shutting down daemon and cleaning up socket...");
        if socket_path_clone.exists() {
            let _ = fs::remove_file(&socket_path_clone);
        }
        std::process::exit(0);
    });

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream).await {
                        error!("Error handling connection: {:?}", e);
                    }
                });
            }
            Err(e) => {
                error!("Failed to accept connection: {:?}", e);
            }
        }
    }
}

async fn handle_connection(stream: UnixStream) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line).await?;
        if bytes_read == 0 {
            // Connection closed
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request: Request = match serde_json::from_str(trimmed) {
            Ok(req) => req,
            Err(e) => {
                let err_resp = Response {
                    jsonrpc: "2.0".to_string(),
                    id: 0,
                    result: None,
                    error: Some(ResponseError {
                        code: -32700,
                        message: format!("Parse error: {}", e),
                    }),
                };
                let serialized = serde_json::to_string(&err_resp)? + "\n";
                writer.write_all(serialized.as_bytes()).await?;
                continue;
            }
        };

        info!("Received request ID {}: {:?}", request.id, request.method);

        // Process request
        let response = process_request(request).await;
        let serialized = serde_json::to_string(&response)? + "\n";
        writer.write_all(serialized.as_bytes()).await?;
    }

    Ok(())
}

use intent::{ActionCategory, RiskLevel, IntentAction};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct LlmResponse {
    status: String,
    output: String,
    #[serde(default)]
    steps: Vec<String>,
    #[serde(default)]
    actions: Vec<IntentAction>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct RiskOverrideRule {
    pattern: String,
    risk_level: RiskLevel,
    #[serde(default)]
    category: Option<ActionCategory>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Policy {
    #[serde(default)]
    blocked_commands: Vec<String>,
    #[serde(default)]
    allowed_commands: Vec<String>,
    #[serde(default)]
    risk_overrides: Vec<RiskOverrideRule>,
    #[serde(default = "default_max_direct_execution_risk")]
    max_direct_execution_risk: RiskLevel,
}

fn default_max_direct_execution_risk() -> RiskLevel {
    RiskLevel::Medium
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            blocked_commands: vec![
                "rm -rf /".to_string(),
                "dd if=".to_string(),
                "mkfs.".to_string(),
            ],
            allowed_commands: vec![],
            risk_overrides: vec![],
            max_direct_execution_risk: RiskLevel::Medium,
        }
    }
}

fn load_policy() -> Policy {
    let paths = vec![
        PathBuf::from("/etc/debai/policy.json"),
        PathBuf::from("debai_policy.json"),
    ];

    for path in paths {
        if path.exists() {
            info!("Loading security policy from: {:?}", path);
            match fs::read_to_string(&path) {
                Ok(content) => match serde_json::from_str::<Policy>(&content) {
                    Ok(policy) => {
                        info!("Successfully loaded security policy from {:?}", path);
                        return policy;
                    }
                    Err(e) => {
                        error!("Failed to parse security policy at {:?}: {}", path, e);
                    }
                },
                Err(e) => {
                    error!("Failed to read security policy at {:?}: {}", path, e);
                }
            }
        }
    }

    info!("Using default built-in security policy");
    Policy::default()
}

fn current_policy() -> &'static Policy {
    static POLICY: OnceLock<Policy> = OnceLock::new();
    POLICY.get_or_init(load_policy)
}

fn verify_and_sanitize_actions(actions: &mut Vec<IntentAction>) {
    let policy = current_policy();

    for action in actions.iter_mut() {
        let cmd = action.command.trim().to_lowercase();
        
        // 1. Check blocked commands
        let mut is_blocked = false;
        for blocked in &policy.blocked_commands {
            if cmd.contains(&blocked.to_lowercase()) {
                is_blocked = true;
                break;
            }
        }
        
        if is_blocked {
            action.risk_level = RiskLevel::Critical;
            action.explanation = format!(
                "[BLOCKED BY POLICY ENGINE] This command matches an explicitly blocked pattern in the security policy! Original: {}", 
                action.explanation
            );
            continue;
        }

        // 2. Check allowed commands (override to Low risk)
        let mut is_allowed = false;
        for allowed in &policy.allowed_commands {
            if cmd == allowed.to_lowercase() {
                is_allowed = true;
                break;
            }
        }
        
        if is_allowed {
            action.risk_level = RiskLevel::Low;
            action.explanation = format!(
                "[ALLOWED BY POLICY ENGINE] Pre-approved command. Original: {}", 
                action.explanation
            );
            continue;
        }

        // 3. Apply custom risk overrides
        for rule in &policy.risk_overrides {
            if cmd.contains(&rule.pattern.to_lowercase()) {
                action.risk_level = rule.risk_level;
                if let Some(cat) = rule.category {
                    action.category = cat;
                }
                action.explanation = format!(
                    "[OS POLICY OVERRIDE] Risk re-classified to {:?}. Original: {}",
                    rule.risk_level, action.explanation
                );
            }
        }

        // 4. Default categories and risk levels fallback if not classified
        if action.category == ActionCategory::Unknown {
            if cmd.contains("apt ") || cmd.contains("apt-get ") || cmd.contains("dpkg ") {
                action.category = ActionCategory::PackageManagement;
            } else if cmd.contains("systemctl ") || cmd.contains("service ") {
                action.category = ActionCategory::ServiceControl;
            } else if cmd.contains("ip ") || cmd.contains("ifconfig ") || cmd.contains("ufw ") || cmd.contains("iptables ") {
                action.category = ActionCategory::NetworkConfiguration;
            }
        }
    }
}

async fn call_ollama(
    system_prompt: &str,
    user_prompt: &str,
) -> Result<LlmResponse, Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::new();
    let res = client
        .post("http://localhost:11434/v1/chat/completions")
        .json(&serde_json::json!({
            "model": "qwen2.5:1.5b",
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": user_prompt }
            ],
            "temperature": 0.0,
            "response_format": { "type": "json_object" }
        }))
        .send()
        .await?;

    if !res.status().is_success() {
        return Err(format!("Ollama server returned error status: {}", res.status()).into());
    }

    let json_resp: serde_json::Value = res.json().await?;
    let content = json_resp["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("Failed to extract content from Ollama completions")?;

    let parsed: LlmResponse = match serde_json::from_str(content) {
        Ok(p) => p,
        Err(e) => {
            error!("Serde deserialization failed for raw JSON: {}", content);
            return Err(e.into());
        }
    };
    info!("Parsed LLM response successfully: {}", content);
    Ok(parsed)
}

async fn execute_actions(actions: Vec<IntentAction>, dry_run: bool) -> ResponseResult {
    let mut results = Vec::new();
    let mut all_success = true;
    let mut detected_sandbox_path = String::new();

    // Trigger pre_execute hook
    let actions = match plugins::plugin_manager().trigger_pre_execute(actions) {
        Ok(a) => a,
        Err(e) => {
            return ResponseResult {
                status: "failure".to_string(),
                output: format!("Execution blocked by plugin: {}", e),
                steps: vec![],
                actions: vec![],
                execution_results: vec![intent::ExecutionResult {
                    command: "Plugin Hook".to_string(),
                    exit_code: None,
                    stdout: "".to_string(),
                    stderr: format!("Blocked by plugin pre_execute hook: {}", e),
                    success: false,
                }],
                sandbox_path: "".to_string(),
            };
        }
    };

    for action in &actions {
        let action = action.clone();
        info!("Executing command (dry_run={}): {}", dry_run, action.command);
        
        let policy = current_policy();
        
        // Safety check 1: Direct host execution exceeds policy limit
        if !dry_run && action.risk_level > policy.max_direct_execution_risk {
            warn!("Execution blocked: Risk level {:?} exceeds max_direct_execution_risk limit {:?}", action.risk_level, policy.max_direct_execution_risk);
            results.push(intent::ExecutionResult {
                command: action.command.clone(),
                exit_code: None,
                stdout: "".to_string(),
                stderr: format!(
                    "Security Policy Blocked: Direct execution on host is denied because the risk level ({:?}) exceeds the policy limit ({:?}). Please use [d]ry-run in sandbox to test this action.",
                    action.risk_level, policy.max_direct_execution_risk
                ),
                success: false,
            });
            all_success = false;
            continue;
        }

        // Safety check 2: Critical commands are always blocked from direct host execution
        if !dry_run && action.risk_level == RiskLevel::Critical {
            warn!("Execution blocked: Command is marked CRITICAL: {}", action.command);
            results.push(intent::ExecutionResult {
                command: action.command.clone(),
                exit_code: None,
                stdout: "".to_string(),
                stderr: "Security Policy Blocked: Critical-risk actions cannot be executed directly on the host system under any circumstances.".to_string(),
                success: false,
            });
            all_success = false;
            continue;
        }

        // Determine run command based on dry_run mode
        let output = if dry_run {
            // Find scripts/sandbox.sh in current directory or fall back
            let sandbox_path = if std::path::Path::new("scripts/sandbox.sh").exists() {
                "scripts/sandbox.sh"
            } else {
                "/usr/local/bin/debai-sandbox"
            };
            tokio::process::Command::new(sandbox_path)
                .arg("--persist")
                .arg(&action.command)
                .output()
                .await
        } else {
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&action.command)
                .output()
                .await
        };

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                let exit_code = out.status.code();
                let success = out.status.success();

                if !success {
                    all_success = false;
                }

                // Check if a sandbox directory path was outputted by sandbox.sh
                for line in stdout.lines() {
                    if line.starts_with("SANDBOX_DIR: ") {
                        let path_str = line.trim_start_matches("SANDBOX_DIR: ").trim().to_string();
                        if !path_str.is_empty() {
                            let path = PathBuf::from(&path_str);
                            active_sandboxes().lock().unwrap().push(path);
                            detected_sandbox_path = path_str;
                        }
                    }
                }

                results.push(intent::ExecutionResult {
                    command: action.command.clone(),
                    exit_code,
                    stdout,
                    stderr,
                    success,
                });
            }
            Err(e) => {
                all_success = false;
                results.push(intent::ExecutionResult {
                    command: action.command.clone(),
                    exit_code: None,
                    stdout: "".to_string(),
                    stderr: format!("Failed to start command process: {}", e),
                    success: false,
                });
            }
        }
    }

    let status = if all_success {
        "success".to_string()
    } else {
        "failure".to_string()
    };

    // Trigger post_execute hook
    plugins::plugin_manager().trigger_post_execute(actions.clone(), results.clone());

    ResponseResult {
        status,
        output: "Execution completed.".to_string(),
        steps: vec![],
        actions: vec![],
        execution_results: results,
        sandbox_path: detected_sandbox_path,
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CacheEntry {
    query: String,
    method: Method,
    response: LlmResponse,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct Cache {
    entries: Vec<CacheEntry>,
}

fn load_cache() -> Cache {
    let path = PathBuf::from("debai_cache.json");
    if path.exists() {
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(cache) = serde_json::from_str::<Cache>(&content) {
                info!("Loaded {} entries from query cache", cache.entries.len());
                return cache;
            }
        }
    }
    Cache::default()
}

fn save_cache(cache: &Cache) {
    let path = PathBuf::from("debai_cache.json");
    if let Ok(content) = serde_json::to_string_pretty(cache) {
        if let Err(e) = fs::write(&path, content) {
            error!("Failed to save cache to {:?}: {}", path, e);
        }
    }
}

fn global_cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(load_cache()))
}


async fn process_request(request: Request) -> Response {
    if request.method == Method::ExecuteActions {
        let result = execute_actions(request.params.actions, request.params.dry_run).await;
        return Response {
            jsonrpc: "2.0".to_string(),
            id: request.id,
            result: Some(result),
            error: None,
        };
    }

    if request.method == Method::CommitSandbox {
        let sandbox_path = request.params.sandbox_path;
        let path = PathBuf::from(&sandbox_path);
        
        let exists = {
            let active = active_sandboxes().lock().unwrap();
            active.contains(&path)
        };
        
        if exists {
            info!("Committing sandbox changes from: {:?}", path);
            
            let output = tokio::process::Command::new("cp")
                .arg("-aT")
                .arg(format!("{}/upper/", sandbox_path))
                .arg("/")
                .output()
                .await;
                
            let success = match &output {
                Ok(out) => out.status.success(),
                Err(_) => false,
            };
            
            let success = if !success {
                info!("Standard cp failed or returned error; trying fallback cp with --no-preserve...");
                let fallback_output = tokio::process::Command::new("cp")
                    .arg("-RT")
                    .arg("--no-preserve=ownership,timestamps")
                    .arg(format!("{}/upper/", sandbox_path))
                    .arg("/")
                    .output()
                    .await;
                    
                match fallback_output {
                    Ok(out) => {
                        if !out.status.success() {
                            error!("Fallback cp command failed: {}", String::from_utf8_lossy(&out.stderr));
                        }
                        out.status.success()
                    }
                    Err(e) => {
                        error!("Failed to spawn fallback cp command: {:?}", e);
                        false
                    }
                }
            } else {
                true
            };
            
            let _ = fs::remove_dir_all(&path);
            {
                let mut active = active_sandboxes().lock().unwrap();
                active.retain(|p| p != &path);
            }
            
            let status = if success { "success".to_string() } else { "failure".to_string() };
            return Response {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: Some(ResponseResult {
                    status,
                    output: "Sandbox committed successfully.".to_string(),
                    steps: vec![],
                    actions: vec![],
                    execution_results: vec![],
                    sandbox_path: "".to_string(),
                }),
                error: None,
            };
        } else {
            return Response {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: None,
                error: Some(ResponseError {
                    code: -32602,
                    message: "Invalid or inactive sandbox path provided.".to_string(),
                }),
            };
        }
    }

    if request.method == Method::CleanupSandbox {
        let sandbox_path = request.params.sandbox_path;
        let path = PathBuf::from(&sandbox_path);
        
        let exists = {
            let active = active_sandboxes().lock().unwrap();
            active.contains(&path)
        };
        
        if exists {
            info!("Cleaning up sandbox from: {:?}", path);
            let _ = fs::remove_dir_all(&path);
            {
                let mut active = active_sandboxes().lock().unwrap();
                active.retain(|p| p != &path);
            }
            
            return Response {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: Some(ResponseResult {
                    status: "success".to_string(),
                    output: "Sandbox cleaned up successfully.".to_string(),
                    steps: vec![],
                    actions: vec![],
                    execution_results: vec![],
                    sandbox_path: "".to_string(),
                }),
                error: None,
            };
        } else {
            return Response {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: None,
                error: Some(ResponseError {
                    code: -32602,
                    message: "Invalid or inactive sandbox path provided.".to_string(),
                }),
            };
        }
    }

    let query = request.params.query.trim().to_string();
    let method = request.method;

    {
        let cache = global_cache().lock().unwrap();
        if let Some(entry) = cache.entries.iter().find(|e| e.query.eq_ignore_ascii_case(&query) && e.method == method) {
            info!("Cache HIT for query: '{}' [{:?}]", query, method);
            let mut result_actions = entry.response.actions.clone();
            // Re-apply security policy rules dynamically on cached actions
            verify_and_sanitize_actions(&mut result_actions);
            
            return Response {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: Some(ResponseResult {
                    status: entry.response.status.clone(),
                    output: entry.response.output.clone(),
                    steps: entry.response.steps.clone(),
                    actions: result_actions,
                    execution_results: vec![],
                    sandbox_path: "".to_string(),
                }),
                error: None,
            };
        }
    }

    info!("Cache MISS for query: '{}' [{:?}]", query, method);

    let (system_prompt, user_prompt) = match request.method {
        Method::ExplainCommand => (
            "You are DebAI, a command explainer service. Explain the user's shell command including its purpose, arguments/flags, and safety considerations. Respond ONLY with a JSON object of this structure: { \"status\": \"success\", \"output\": \"Markdown explanation text\", \"steps\": [], \"actions\": [ { \"category\": \"read_filesystem\", \"command\": \"command name\", \"risk_level\": \"low\", \"explanation\": \"explanation\" } ] } (category must be one of: read_filesystem, write_filesystem, package_management, service_control, network_configuration, unknown. risk_level must be one of: low, medium, high, critical).",
            format!("Explain this command: {}", request.params.query)
        ),
        Method::ExploreDir => (
            "You are DebAI, a directory explorer service. Explain the role of the user's directory path in a standard Linux system (FHS) and what files/subdirectories are typically found there. Respond ONLY with a JSON object of this structure: { \"status\": \"success\", \"output\": \"Markdown directory summary\", \"steps\": [], \"actions\": [ { \"category\": \"read_filesystem\", \"command\": \"command\", \"risk_level\": \"low\", \"explanation\": \"explanation\" } ] } (category must be one of: read_filesystem, write_filesystem, package_management, service_control, network_configuration, unknown. risk_level must be one of: low, medium, high, critical).",
            format!("Explore this directory: {}", request.params.query)
        ),
        Method::SysQuery => (
            "You are DebAI, a system query service. Answer the user's question about Linux logs, services, packages, or system state in Markdown. You MUST output a JSON object containing the keys: \"status\" (string), \"output\" (string Markdown), \"steps\" (array of strings), and \"actions\" (array of proposed action objects representing commands to run).
Example structure:
{
  \"status\": \"success\",
  \"output\": \"Markdown answer\",
  \"steps\": [\"Check date\"],
  \"actions\": [
    {
      \"category\": \"read_filesystem\",
      \"command\": \"date\",
      \"risk_level\": \"low\",
      \"explanation\": \"Checks system date\"
    }
  ]
}",
            format!("Query: {}", request.params.query)
        ),
        Method::GeneratePlan => (
            "You are DebAI, an execution planner service. Produce a step-by-step plan to accomplish the user's task. You MUST output a JSON object containing the keys: \"status\" (string), \"output\" (string Markdown), \"steps\" (array of strings), and \"actions\" (array of proposed action objects representing commands to run).
Example structure:
{
  \"status\": \"success\",
  \"output\": \"Markdown description\",
  \"steps\": [\"Step 1\"],
  \"actions\": [
    {
      \"category\": \"package_management\",
      \"command\": \"apt-get install git\",
      \"risk_level\": \"medium\",
      \"explanation\": \"Installs git package\"
    }
  ]
}",
            format!("Plan task: {}", request.params.query)
        ),
        Method::ExecuteActions => unreachable!(),
        Method::CommitSandbox => unreachable!(),
        Method::CleanupSandbox => unreachable!(),
    };

    match call_ollama(system_prompt, &user_prompt).await {
        Ok(mut llm_res) => {
            // Save to cache before modifying actions with the policy engine
            {
                let mut cache = global_cache().lock().unwrap();
                cache.entries.push(CacheEntry {
                    query: query.clone(),
                    method,
                    response: llm_res.clone(),
                });
                save_cache(&cache);
            }

            // Apply security policy engine validation and sanitization
            verify_and_sanitize_actions(&mut llm_res.actions);

            Response {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: Some(ResponseResult {
                    status: llm_res.status.clone(),
                    output: llm_res.output.clone(),
                    steps: llm_res.steps.clone(),
                    actions: llm_res.actions.clone(),
                    execution_results: vec![],
                    sandbox_path: "".to_string(),
                }),
                error: None,
            }
        }
        Err(e) => {
            error!("Ollama call failed: {:?}", e);
            Response {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: None,
                error: Some(ResponseError {
                    code: -32603,
                    message: format!("Internal model inference error: {}", e),
                }),
            }
        }
    }
}
