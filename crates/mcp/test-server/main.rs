//! A small MCP server over stdio for the tests. Its tools: `echo`, `add`,
//! `fail` (an error result), `slow` (waits, unless cancelled), `pinged`
//! (whether the client answered the server's ping) and `crash` (exits).

use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn main() {
    eprintln!("test server starting");
    let out = Arc::new(Mutex::new(std::io::stdout()));
    let send = {
        let out = out.clone();
        move |v: Value| {
            let mut out = out.lock().unwrap();
            writeln!(out, "{v}").unwrap();
            out.flush().unwrap();
        }
    };
    let cancelled = Arc::new(Mutex::new(HashSet::<String>::new()));
    let mut pinged = false;
    // Some servers print a banner on stdout; the client must skip it.
    println!("MCP test server v1");
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let id = msg["id"].clone();
        match msg["method"].as_str() {
            Some("initialize") => send(json!({
                "jsonrpc": "2.0", "id": id,
                "result": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": { "tools": { "listChanged": true } },
                    "serverInfo": { "name": "test-server", "version": "1.0.0" },
                    "instructions": "Use echo to repeat things."
                }
            })),
            Some("notifications/initialized") => {
                send(json!({ "jsonrpc": "2.0", "id": "server-ping", "method": "ping" }));
            }
            Some("notifications/cancelled") => {
                cancelled
                    .lock()
                    .unwrap()
                    .insert(msg["params"]["requestId"].to_string());
                eprintln!("cancelled {}", msg["params"]["requestId"]);
            }
            Some("tools/list") => send(json!({
                "jsonrpc": "2.0", "id": id,
                "result": { "tools": [
                    { "name": "echo", "description": "Repeats the text.", "inputSchema": {
                        "$schema": "http://json-schema.org/draft-07/schema#",
                        "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] } },
                    { "name": "add", "description": "Adds two numbers.", "inputSchema": {
                        "type": "object", "properties": { "a": { "type": "number" }, "b": { "type": "number" } } } },
                    { "name": "fail", "description": "Always fails." },
                    { "name": "slow", "inputSchema": { "type": "object", "properties": { "ms": { "type": "integer" } } } },
                    { "name": "pinged", "inputSchema": { "type": "object" } },
                    { "name": "crash", "inputSchema": { "type": "object" } }
                ] }
            })),
            Some("tools/call") => {
                let args = msg["params"]["arguments"].clone();
                let text = |t: String| json!({ "content": [{ "type": "text", "text": t }] });
                let result = match msg["params"]["name"].as_str() {
                    Some("echo") => text(args["text"].as_str().unwrap_or_default().to_string()),
                    Some("add") => text(
                        (args["a"].as_f64().unwrap_or(0.0) + args["b"].as_f64().unwrap_or(0.0))
                            .to_string(),
                    ),
                    Some("fail") => {
                        json!({ "content": [{ "type": "text", "text": "it broke" }], "isError": true })
                    }
                    Some("pinged") => text(pinged.to_string()),
                    Some("crash") => {
                        eprintln!("crashing on purpose");
                        std::process::exit(3)
                    }
                    Some("slow") => {
                        let ms = args["ms"].as_u64().unwrap_or(1000);
                        let send = send.clone();
                        let cancelled = cancelled.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(ms));
                            if !cancelled.lock().unwrap().contains(&id.to_string()) {
                                send(
                                    json!({ "jsonrpc": "2.0", "id": id, "result": text("done".into()) }),
                                );
                            }
                        });
                        continue;
                    }
                    _ => {
                        send(
                            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32602, "message": "Unknown tool" } }),
                        );
                        continue;
                    }
                };
                send(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
            }
            // The client's answer to our ping.
            None if id == json!("server-ping") && msg.get("result").is_some() => pinged = true,
            Some(_) if !id.is_null() => send(json!({
                "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "Method not found" }
            })),
            _ => {}
        }
    }
}
