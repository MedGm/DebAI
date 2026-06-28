use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::io::Write;
use std::sync::OnceLock;
use serde::{Deserialize, Serialize};
use intent::{IntentAction, ExecutionResult};
use tracing::{info, error, warn};

#[derive(Debug, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub hooks: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PreExecutePayload {
    pub actions: Vec<IntentAction>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PreExecuteResponse {
    pub status: String, // "approve" or "deny"
    pub actions: Vec<IntentAction>,
    #[serde(default)]
    pub error_message: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PostExecutePayload {
    pub actions: Vec<IntentAction>,
    pub execution_results: Vec<ExecutionResult>,
}

pub struct PluginManager {
    plugins_dir: PathBuf,
}

impl PluginManager {
    pub fn new() -> Self {
        let dir = if Path::new("plugins").exists() {
            PathBuf::from("plugins")
        } else {
            PathBuf::from("/usr/share/debai/plugins")
        };
        
        // Ensure directory exists for local testing
        let _ = fs::create_dir_all(&dir);
        
        PluginManager { plugins_dir: dir }
    }

    /// Scans the plugins directory and lists all executable files
    fn get_plugins(&self) -> Vec<PathBuf> {
        let mut list = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.plugins_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    // Check if file is executable (Unix check)
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if let Ok(metadata) = entry.metadata() {
                            if metadata.permissions().mode() & 0o111 != 0 {
                                list.push(path);
                            }
                        }
                    }
                    #[cfg(not(unix))]
                    list.push(path);
                }
            }
        }
        list
    }

    /// Triggers the pre_execute hook across all active plugins
    pub fn trigger_pre_execute(&self, mut actions: Vec<IntentAction>) -> Result<Vec<IntentAction>, String> {
        let plugins = self.get_plugins();
        for plugin in plugins {
            // Get plugin info to check supported hooks
            if let Some(info) = self.get_plugin_info(&plugin) {
                if info.hooks.contains(&"pre_execute".to_string()) {
                    info!("Running pre_execute hook on plugin '{}'", info.name);
                    
                    let payload = PreExecutePayload { actions: actions.clone() };
                    let payload_json = serde_json::to_string(&payload).unwrap();
                    
                    match self.run_plugin(&plugin, "pre_execute", &payload_json) {
                        Ok(stdout) => {
                            match serde_json::from_str::<PreExecuteResponse>(&stdout) {
                                Ok(resp) => {
                                    if resp.status == "deny" {
                                        let err = if resp.error_message.is_empty() {
                                            format!("Plugin '{}' denied execution.", info.name)
                                        } else {
                                            format!("Plugin '{}' denied execution: {}", info.name, resp.error_message)
                                        };
                                        warn!("{}", err);
                                        return Err(err);
                                    }
                                    actions = resp.actions;
                                }
                                Err(e) => {
                                    error!("Failed to parse pre_execute response from plugin '{}': {}", info.name, e);
                                }
                            }
                        }
                        Err(e) => {
                            error!("Plugin '{}' pre_execute hook execution failed: {}", info.name, e);
                        }
                    }
                }
            }
        }
        Ok(actions)
    }

    /// Triggers the post_execute hook across all active plugins
    pub fn trigger_post_execute(&self, actions: Vec<IntentAction>, results: Vec<ExecutionResult>) {
        let plugins = self.get_plugins();
        for plugin in plugins {
            if let Some(info) = self.get_plugin_info(&plugin) {
                if info.hooks.contains(&"post_execute".to_string()) {
                    info!("Running post_execute hook on plugin '{}'", info.name);
                    
                    let payload = PostExecutePayload {
                        actions: actions.clone(),
                        execution_results: results.clone(),
                    };
                    let payload_json = serde_json::to_string(&payload).unwrap();
                    
                    if let Err(e) = self.run_plugin(&plugin, "post_execute", &payload_json) {
                        error!("Plugin '{}' post_execute hook execution failed: {}", info.name, e);
                    }
                }
            }
        }
    }

    /// Queries plugin details using `--info` flag
    fn get_plugin_info(&self, plugin_path: &Path) -> Option<PluginInfo> {
        let output = Command::new(plugin_path)
            .arg("--info")
            .output()
            .ok()?;
            
        if output.status.success() {
            serde_json::from_slice::<PluginInfo>(&output.stdout).ok()
        } else {
            None
        }
    }

    /// Runs a plugin child process passing input JSON via stdin
    fn run_plugin(&self, plugin_path: &Path, hook: &str, input_json: &str) -> Result<String, String> {
        let mut child = Command::new(plugin_path)
            .arg("--hook")
            .arg(hook)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn plugin child: {}", e))?;
            
        {
            let mut stdin = child.stdin.take().ok_or("Failed to open stdin")?;
            stdin.write_all(input_json.as_bytes())
                .map_err(|e| format!("Failed to write to plugin stdin: {}", e))?;
        }
        
        let output = child.wait_with_output()
            .map_err(|e| format!("Failed to wait on plugin child: {}", e))?;
            
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).to_string())
        }
    }
}

pub fn plugin_manager() -> &'static PluginManager {
    static MANAGER: OnceLock<PluginManager> = OnceLock::new();
    MANAGER.get_or_init(PluginManager::new)
}
