use clap::{Parser, Subcommand};
use intent::{Method, Params, Request, Response};
use std::path::PathBuf;
use tokio::net::UnixStream;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Parser, Debug)]
#[command(author, version, about = "DebAI Client Terminal (aiterm)")]
struct Cli {
    /// Path to the Unix domain socket
    #[arg(short, long)]
    socket: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Explain a shell command
    Explain {
        /// The shell command to explain
        query: String,
    },
    /// Explore a directory and summarize its purpose
    Explore {
        /// The path to the directory
        query: String,
    },
    /// Ask a question about the local system (packages, services, logs)
    Query {
        /// The system query/question
        query: String,
    },
    /// Generate an execution plan for a requested task
    Plan {
        /// The task description to plan
        query: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Map CLI subcommand to Method and Query
    let (method, query) = match cli.command {
        Commands::Explain { query } => (Method::ExplainCommand, query),
        Commands::Explore { query } => (Method::ExploreDir, query),
        Commands::Query { query } => (Method::SysQuery, query),
        Commands::Plan { query } => (Method::GeneratePlan, query),
    };

    // Dynamically resolve socket path
    let socket_path = match cli.socket {
        Some(path) => path,
        None => {
            let run_sock = PathBuf::from("/run/debai/aid.sock");
            let tmp_sock = PathBuf::from("/tmp/debai_aid.sock");
            if run_sock.exists() {
                run_sock
            } else if tmp_sock.exists() {
                tmp_sock
            } else {
                run_sock // default fallback
            }
        }
    };

    // Connect to Unix domain socket
    let stream = match UnixStream::connect(&socket_path).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: Could not connect to DebAI daemon at {:?}.", socket_path);
            eprintln!("Is the daemon 'aid' running?");
            eprintln!("Details: {}", e);
            std::process::exit(1);
        }
    };

    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    // Prepare Request
    let request = Request {
        jsonrpc: "2.0".to_string(),
        id: 1, // Simple ID for now
        method,
        params: Params { query, actions: vec![], dry_run: false, sandbox_path: "".to_string() },
    };

    // Serialize and send
    let mut serialized = serde_json::to_string(&request)?;
    serialized.push('\n');
    writer.write_all(serialized.as_bytes()).await?;

    // Read Response
    let mut line = String::new();
    if reader.read_line(&mut line).await? > 0 {
        let response: Response = serde_json::from_str(&line)?;
        if let Some(err) = response.error {
            eprintln!("Error [{}]: {}", err.code, err.message);
            std::process::exit(1);
        }

        if let Some(result) = response.result {
            println!("{}", result.output);
            if !result.steps.is_empty() {
                println!("\nSteps:");
                for (i, step) in result.steps.iter().enumerate() {
                    println!("{}. {}", i + 1, step);
                }
            }

            if !result.actions.is_empty() {
                println!("\nProposed Actions:");
                for action in result.actions.iter() {
                    let risk_color = match action.risk_level {
                        intent::RiskLevel::Low => "\x1b[32m[LOW]\x1b[0m",
                        intent::RiskLevel::Medium => "\x1b[33m[MEDIUM]\x1b[0m",
                        intent::RiskLevel::High => "\x1b[31m[HIGH]\x1b[0m",
                        intent::RiskLevel::Critical => "\x1b[1;31m[CRITICAL]\x1b[0m",
                        intent::RiskLevel::Unknown => "\x1b[37m[UNKNOWN]\x1b[0m",
                    };
                    println!(
                        "  {} [{:?}] Command: `{}`\n     Explanation: {}",
                        risk_color, action.category, action.command, action.explanation
                    );
                }

                // Prompt user for execution
                print!("\nWhat would you like to do? [e]xecute on host, [d]ry-run in sandbox, [c]ancel [default: d]: ");
                use std::io::Write;
                std::io::stdout().flush()?;

                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                let choice = input.trim().to_lowercase();

                let (execute, dry_run) = match choice.as_str() {
                    "e" | "execute" => (true, false),
                    "d" | "dry-run" | "dryrun" | "" => (true, true),
                    _ => (false, false),
                };

                if execute {
                    if dry_run {
                        println!("\nSending dry-run (sandboxed) execution request to daemon...");
                    } else {
                        println!("\nSending real host execution request to daemon...");
                    }
                    
                    // Re-connect to the daemon socket
                    let stream = match UnixStream::connect(&socket_path).await {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("Error: Could not reconnect to daemon to run actions: {}", e);
                            std::process::exit(1);
                        }
                    };

                    let (reader, mut writer) = stream.into_split();
                    let mut reader = BufReader::new(reader);

                    let exec_req = Request {
                        jsonrpc: "2.0".to_string(),
                        id: 2,
                        method: intent::Method::ExecuteActions,
                        params: intent::Params {
                            query: "".to_string(),
                            actions: result.actions,
                            dry_run,
                            sandbox_path: "".to_string(),
                        },
                    };

                    let mut serialized = serde_json::to_string(&exec_req)?;
                    serialized.push('\n');
                    writer.write_all(serialized.as_bytes()).await?;

                    let mut line = String::new();
                    if reader.read_line(&mut line).await? > 0 {
                        let exec_resp: Response = serde_json::from_str(&line)?;
                        if let Some(err) = exec_resp.error {
                            eprintln!("Execution Error [{}]: {}", err.code, err.message);
                            std::process::exit(1);
                        }

                        if let Some(exec_res) = exec_resp.result {
                            println!("\nExecution Results:");
                            for (i, res) in exec_res.execution_results.iter().enumerate() {
                                let status_str = if res.success {
                                    "\x1b[32m[SUCCESS]\x1b[0m"
                                } else {
                                    "\x1b[31m[FAILED]\x1b[0m"
                                };
                                println!("{}. {} Command: `{}`", i + 1, status_str, res.command);
                                if !res.stdout.is_empty() {
                                    println!("   Stdout:\n   {}", res.stdout.replace('\n', "\n   ").trim());
                                }
                                if !res.stderr.is_empty() {
                                    println!("   Stderr:\n   {}", res.stderr.replace('\n', "\n   ").trim());
                                }
                            }

                            if !exec_res.sandbox_path.is_empty() {
                                print!("\nWould you like to commit the sandboxed changes to your host system? [y/N]: ");
                                use std::io::Write;
                                std::io::stdout().flush()?;
                                let mut commit_input = String::new();
                                std::io::stdin().read_line(&mut commit_input)?;
                                let commit_choice = commit_input.trim().to_lowercase();
                                
                                let commit = commit_choice == "y" || commit_choice == "yes";
                                
                                // Send Commit or Cleanup request
                                let stream = match UnixStream::connect(&socket_path).await {
                                    Ok(s) => s,
                                    Err(e) => {
                                        eprintln!("Error: Could not reconnect to daemon to commit/cleanup sandbox: {}", e);
                                        std::process::exit(1);
                                    }
                                };
                                let (reader, mut writer) = stream.into_split();
                                let mut reader = BufReader::new(reader);
                                
                                let method = if commit {
                                    intent::Method::CommitSandbox
                                } else {
                                    intent::Method::CleanupSandbox
                                };
                                
                                let commit_req = Request {
                                    jsonrpc: "2.0".to_string(),
                                    id: 3,
                                    method,
                                    params: intent::Params {
                                        query: "".to_string(),
                                        actions: vec![],
                                        dry_run: false,
                                        sandbox_path: exec_res.sandbox_path.clone(),
                                    },
                                };
                                
                                let mut serialized = serde_json::to_string(&commit_req)?;
                                serialized.push('\n');
                                writer.write_all(serialized.as_bytes()).await?;
                                
                                let mut line = String::new();
                                if reader.read_line(&mut line).await? > 0 {
                                    let commit_resp: Response = serde_json::from_str(&line)?;
                                    if let Some(err) = commit_resp.error {
                                        eprintln!("Error [{}]: {}", err.code, err.message);
                                    } else if let Some(res) = commit_resp.result {
                                        if commit {
                                            if res.status == "success" {
                                                println!("\x1b[32mSuccessfully committed sandboxed changes to the host filesystem!\x1b[0m");
                                            } else {
                                                eprintln!("Failed to commit changes to host (permissions or I/O error).");
                                            }
                                        } else {
                                            println!("Sandboxed changes successfully discarded.");
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        eprintln!("Error: Connection closed by daemon before execution completed.");
                    }
                } else {
                    println!("Execution canceled.");
                }
            }
        } else {
            eprintln!("Error: Received empty result and no error from daemon.");
            std::process::exit(1);
        }
    } else {
        eprintln!("Error: Connection closed by daemon before response was received.");
        std::process::exit(1);
    }

    Ok(())
}
