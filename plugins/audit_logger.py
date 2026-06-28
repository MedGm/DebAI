#!/usr/bin/env python3
import sys
import json

if len(sys.argv) < 2:
    print("Usage: plugin.py [--info | --hook <hook_name>]", file=sys.stderr)
    sys.exit(1)

mode = sys.argv[1]

if mode == "--info":
    info = {
        "name": "Audit Logger",
        "version": "1.0.0",
        "description": "Logs executed commands to /tmp/debai_audit.log",
        "hooks": ["pre_execute", "post_execute"]
    }
    print(json.dumps(info))
    sys.exit(0)

elif mode == "--hook":
    if len(sys.argv) < 3:
        print("Error: Missing hook name", file=sys.stderr)
        sys.exit(1)
        
    hook_name = sys.argv[2]
    input_data = json.loads(sys.stdin.read())
    
    if hook_name == "pre_execute":
        # Simply approve all commands
        response = {
            "status": "approve",
            "actions": input_data.get("actions", []),
            "error_message": ""
        }
        print(json.dumps(response))
        sys.exit(0)
        
    elif hook_name == "post_execute":
        # Log commands to a file
        log_path = "/tmp/debai_audit.log"
        actions = input_data.get("actions", [])
        results = input_data.get("execution_results", [])
        
        with open(log_path, "a") as f:
            for action, result in zip(actions, results):
                status_str = "SUCCESS" if result.get("success") else "FAILED"
                f.write(f"[AUDIT] Cmd: {action.get('command')} | Status: {status_str} | Code: {result.get('exit_code')}\n")
        
        sys.exit(0)
