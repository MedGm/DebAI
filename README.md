# DebAI

An AI-native Linux Distribution layer where Artificial Intelligence is treated as a first-class operating system service instead of an external application.

This repository implements the daemon (`aid`) and the CLI client terminal (`aiterm`) for **DebAI v0.1**.

## Repository Structure

* **`intent/`**: Shared Rust library containing serialization types and the JSON-RPC-based IPC protocol.
* **`daemon/`**: Rust binary for `aid`, the background AI service running on the host/guest system.
* **`terminal/`**: Rust binary for `aiterm`, the CLI client used to query the daemon with natural language.

---

## How to Build & Run

### 1. Requirements
Ensure you have the Rust toolchain (Cargo) installed on your system.

### 2. Build the Workspace
To build all crates in the workspace:
```bash
cargo build
```

### 3. Run the Daemon
By default, the daemon binds to `/tmp/debai_aid.sock` (configurable via parameters):
```bash
cargo run --bin aid
```

### 4. Query with the Client Terminal
Open another terminal pane and run the `aiterm` client with any of the supported commands:

```bash
# Explain a command
cargo run --bin aiterm -- explain "ls -la"

# Explore a directory
cargo run --bin aiterm -- explore "/etc"

# Ask a system query
cargo run --bin aiterm -- query "is nginx running?"

# Generate an execution plan
cargo run --bin aiterm -- plan "install postgresql"
```

To configure a custom Unix socket path, pass `--socket <path>` to both binaries.
