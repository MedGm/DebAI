#!/bin/bash
# DebAI Sandbox Runner
# Usage: sandbox.sh [--persist] <command>

PERSIST=false
if [ "$1" == "--persist" ]; then
    PERSIST=true
    shift
fi

COMMAND="$1"
if [ -z "$COMMAND" ]; then
    echo "Error: No command specified." >&2
    exit 1
fi

# Check if we are running as root
if [ "$EUID" -ne 0 ]; then
    # Non-root fallback: run in a user-level simulated mode
    echo "=== DebAI Non-Root Dry-Run Mode (Simulation) ==="
    echo "Proposed command: $COMMAND"
    
    # Simple simulation logic for common commands:
    if [[ "$COMMAND" =~ apt-get\ install\ -y\ (.*) ]] || [[ "$COMMAND" =~ apt-get\ install\ (.*) ]]; then
        PKG="${BASH_REMATCH[1]}"
        echo "Simulation: Package '${PKG}' would be downloaded and installed."
        echo "Simulation: Configuration files under /etc/${PKG} would be created."
        echo "Simulation: Service scripts in /lib/systemd/system/${PKG}.service would be added."
    elif [[ "$COMMAND" =~ rm\ -rf\ (.*) ]]; then
        echo "Simulation: Files and directories under '${BASH_REMATCH[1]}' would be recursively deleted."
    elif [[ "$COMMAND" =~ touch\ (.*) ]]; then
        echo "Simulation: File '${BASH_REMATCH[1]}' would be created."
    elif [[ "$COMMAND" == *"echo"* ]] && [[ "$COMMAND" == *">"* ]]; then
        echo "Simulation: Content would be redirected and written to a file."
    else
        echo "Simulation: Command executed successfully (mocked)."
    fi

    if [ "$PERSIST" = true ]; then
        # Create a simulated upper directory with mock changes so Commit is functional!
        SIM_SANDBOX_DIR=$(mktemp -d /tmp/debai_sim_sandbox.XXXXXX)
        mkdir -p "$SIM_SANDBOX_DIR/upper"
        
        # Touch files inside upper dir if we recognize the command
        if [[ "$COMMAND" =~ touch\ (.*) ]]; then
            FILE_PATH="${BASH_REMATCH[1]}"
            if [[ "$FILE_PATH" != /* ]]; then
                FILE_PATH="$(pwd)/$FILE_PATH"
            fi
            # Ensure subdirectory exists inside upper
            mkdir -p "$SIM_SANDBOX_DIR/upper/$(dirname "$FILE_PATH")"
            touch "$SIM_SANDBOX_DIR/upper/$FILE_PATH"
        elif [[ "$COMMAND" == *"echo"* ]] && [[ "$COMMAND" == *">"* ]]; then
            FILE_PATH=$(echo "$COMMAND" | awk -F '>' '{print $2}' | tr -d ' ' | tr -d '"' | tr -d "'")
            CONTENT=$(echo "$COMMAND" | awk -F '>' '{print $1}' | sed 's/echo//' | tr -d '"' | tr -d "'")
            if [[ "$FILE_PATH" != /* ]]; then
                FILE_PATH="$(pwd)/$FILE_PATH"
            fi
            mkdir -p "$SIM_SANDBOX_DIR/upper/$(dirname "$FILE_PATH")"
            echo "$CONTENT" > "$SIM_SANDBOX_DIR/upper/$FILE_PATH"
        fi
        
        echo "SANDBOX_DIR: $SIM_SANDBOX_DIR"
    fi
    exit 0
fi

# Root mode: Real OverlayFS namespace sandbox
SANDBOX_DIR=$(mktemp -d /tmp/debai_sandbox.XXXXXX)
UPPER_DIR="$SANDBOX_DIR/upper"
WORK_DIR="$SANDBOX_DIR/work"
MERGED_DIR="$SANDBOX_DIR/merged"

mkdir -p "$UPPER_DIR" "$WORK_DIR" "$MERGED_DIR"

# Clean up function
cleanup() {
    umount -l "$MERGED_DIR" 2>/dev/null || true
    if [ "$PERSIST" = false ]; then
        rm -rf "$SANDBOX_DIR"
    else
        echo "SANDBOX_DIR: $SANDBOX_DIR"
    fi
}
trap cleanup EXIT

# Mount the overlay filesystem
# lowerdir is / (read-only), upperdir is our temporary upper, workdir is work
mount -t overlay overlay -o lowerdir=/,upperdir="$UPPER_DIR",workdir="$WORK_DIR" "$MERGED_DIR"

# Run command inside mount namespace using chroot
# We share network to allow apt-get to work, but isolate the filesystem
# We use unshare to create a new mount namespace, then chroot
unshare -m sh -c "
    mount --make-rprivate /
    chroot \"$MERGED_DIR\" sh -c \"$COMMAND\"
"
EXIT_CODE=$?

# Print summary of modified files
echo "=== Sandbox Filesystem Changes ==="
if [ -d "$UPPER_DIR" ]; then
    CHANGES_FOUND=false
    while read -r file; do
        if [ -f "$file" ]; then
            # Map back to absolute path on the host
            REL_PATH=${file#"$UPPER_DIR"}
            echo "[CREATED/MODIFIED] $REL_PATH"
            CHANGES_FOUND=true
        fi
    done < <(find "$UPPER_DIR" -type f)
    
    if [ "$CHANGES_FOUND" = false ]; then
        echo "No files were modified."
    fi
fi

exit $EXIT_CODE
