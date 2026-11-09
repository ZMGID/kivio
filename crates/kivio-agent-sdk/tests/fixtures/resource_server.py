"""Real stdio MCP peer for connection/resource lifecycle tests."""
import json
import os
import sys

modern = os.environ.get("MCP_RESOURCE_PROTOCOL") == "2026"
resource_only = os.environ.get("MCP_RESOURCE_ONLY") == "1"
marker = os.environ.get("MCP_RESOURCE_MARKER")
subscriptions = {}
reads = 0
uri = "test://resource/current"


def send(message):
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", **message}) + "\n")
    sys.stdout.flush()


def updated(count=1):
    for number in range(count):
        for resource_uri, subscription_id in subscriptions.items():
            params = {"uri": resource_uri}
            if subscription_id is not None:
                params["_meta"] = {"io.modelcontextprotocol/subscriptionId": subscription_id}
            send({"method": "notifications/resources/updated", "params": params})


for line in sys.stdin:
    message = json.loads(line)
    request_id = message.get("id")
    method = message.get("method")
    params = message.get("params", {})
    if marker:
        with open(marker, "a", encoding="utf-8") as file:
            file.write(method + "\n")
    if request_id is None:
        continue
    if method == "initialize":
        if modern:
            send({"id": request_id, "error": {"code": -32601, "message": "Method not found"}})
            continue
        capabilities = {"resources": {"subscribe": True}}
        if not resource_only:
            capabilities["tools"] = {}
        result = {"protocolVersion": "2025-11-25", "capabilities": capabilities, "serverInfo": {"name": "resources", "version": "1"}}
    elif method == "server/discover":
        result = {"resultType": "complete", "supportedVersions": ["2026-07-28"], "capabilities": {"tools": {}, "resources": {"subscribe": True}}, "ttlMs": 0, "cacheScope": "public"}
    elif method == "tools/list":
        if resource_only:
            send({"id": request_id, "error": {"code": -32601, "message": "Method not found"}})
            continue
        result = {"tools": [{"name": "emit", "inputSchema": {"type": "object"}}]}
    elif method == "resources/list":
        if "cursor" not in params:
            result = {"resources": [{"uri": uri, "name": "Current"}], "nextCursor": "second"}
        else:
            result = {"resources": [{"uri": "test://resource/second", "name": "Second"}]}
    elif method == "resources/subscribe":
        subscriptions[params["uri"]] = None
        result = {}
    elif method == "subscriptions/listen":
        notifications = params["notifications"]
        for resource_uri in notifications.get("resourceSubscriptions", []):
            subscriptions[resource_uri] = request_id
        send({"method": "notifications/subscriptions/acknowledged", "params": {"notifications": notifications, "_meta": {"io.modelcontextprotocol/subscriptionId": request_id}}})
        continue
    elif method == "resources/read":
        if params["uri"] == "test://resource/hang":
            continue
        reads += 1
        result = {"contents": [{"uri": params["uri"], "mimeType": "text/plain", "text": "resource-body-" + str(reads)}]}
    elif method == "tools/call":
        arguments = params.get("arguments", {})
        if arguments.get("disconnect"):
            break
        updated(arguments.get("count", 1))
        result = {"content": [{"type": "text", "text": str(reads)}]}
    elif method == "ping":
        result = {}
    else:
        send({"id": request_id, "error": {"code": -32601, "message": "Method not found"}})
        continue
    send({"id": request_id, "result": result})
