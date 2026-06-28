#!/usr/bin/env python3
import sys
import json

if len(sys.argv) < 2:
    sys.exit(1)

mode = sys.argv[1]

if mode == "--info":
    info = {
        "name": "Safety Sentinel",
        "version": "1.0.0",
        "description": "Denies commands matching forbidden keywords",
        "hooks": ["pre_execute"]
    }
    print(json.dumps(info))
    sys.exit(0)

elif mode == "--hook":
    hook_name = sys.argv[2]
    input_data = json.loads(sys.stdin.read())
    
    if hook_name == "pre_execute":
        actions = input_data.get("actions", [])
        for action in actions:
            cmd = action.get("command", "").lower()
            if "prohibited" in cmd or "naughty" in cmd:
                response = {
                    "status": "deny",
                    "actions": actions,
                    "error_message": f"Command contains forbidden keyword: '{action.get('command')}'"
                }
                print(json.dumps(response))
                sys.exit(0)
                
        # Otherwise approve
        response = {
            "status": "approve",
            "actions": actions,
            "error_message": ""
        }
        print(json.dumps(response))
        sys.exit(0)
