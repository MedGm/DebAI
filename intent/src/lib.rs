use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: u64,
    pub method: Method,
    pub params: Params,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    ExplainCommand,
    ExploreDir,
    SysQuery,
    GeneratePlan,
    ExecuteActions,
    CommitSandbox,
    CleanupSandbox,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Params {
    pub query: String,
    #[serde(default)]
    pub actions: Vec<IntentAction>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub sandbox_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: u64,
    pub result: Option<ResponseResult>,
    pub error: Option<ResponseError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseResult {
    pub status: String,
    pub output: String,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub actions: Vec<IntentAction>,
    #[serde(default)]
    pub execution_results: Vec<ExecutionResult>,
    #[serde(default)]
    pub sandbox_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub command: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentAction {
    pub category: ActionCategory,
    pub command: String,
    pub risk_level: RiskLevel,
    pub explanation: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionCategory {
    ReadFilesystem,
    WriteFilesystem,
    PackageManagement,
    ServiceControl,
    NetworkConfiguration,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseError {
    pub code: i32,
    pub message: String,
}
