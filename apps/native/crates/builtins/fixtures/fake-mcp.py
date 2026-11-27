#!/usr/bin/python3
"""Synthetic Powermove tool server over MCP stdio, standing in for
`powermove-native --powermove-mcp --agent-run <grant>` in headless tests."""
import json
import sys

TOOLS = ["get_project_state", "apply_commands", "export_video"]

for line in sys.stdin:
    message = json.loads(line)
    if 'id' not in message:
        continue
    method = message.get('method')
    if method == 'initialize':
        result = {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
                  "serverInfo": {"name": "powermove", "version": "1"}}
    elif method == 'tools/list':
        result = {"tools": [{"name": n, "description": f"The {n} tool.",
                             "inputSchema": {"type": "object", "properties": {}}} for n in TOOLS]}
    elif method == 'tools/call':
        name = message['params']['name']
        result = {"content": [{"type": "text", "text": f"{name} ran: Synthetic, 0 layers"}]}
    else:
        print(json.dumps({"jsonrpc": "2.0", "id": message['id'],
                          "error": {"code": -32601, "message": "Method not found"}}), flush=True)
        continue
    print(json.dumps({"jsonrpc": "2.0", "id": message['id'], "result": result}), flush=True)
