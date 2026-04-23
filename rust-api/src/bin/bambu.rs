//! bambu — CLI for managing the Bambu Live API.
//!
//! Usage:
//!   bambu list                          List all printers
//!   bambu add <id> <host> <device_id> <access_code>  Add a printer
//!   bambu add -f printers.json          Add printers from a JSON file
//!   bambu get <id>                      Get printer details
//!   bambu delete <id>                   Delete a printer
//!   bambu start <id>                    Start stream
//!   bambu stop <id>                     Stop stream
//!   bambu start-all                     Start all streams
//!   bambu stop-all                      Stop all streams
//!   bambu url <id>                      Get stream URL
//!   bambu health                        Health check
//!   bambu init                          Create a printers.json template

use clap::{Parser, Subcommand};
use colored::*;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::process;

#[derive(Parser)]
#[command(name = "bambu", version, about = "Manage Bambu Live API printers")]
struct Cli {
    /// API base URL
    #[arg(long, env = "BAMBU_API_URL", default_value = "http://127.0.0.1:8080")]
    url: String,

    /// Output as JSON (no pretty-printing)
    #[arg(long)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Health check
    Health,

    /// List all printers
    List,

    /// Add a printer (or batch from file)
    Add {
        /// Printer ID (e.g. "a1-mini-1")
        id: Option<String>,

        /// Printer IP/hostname
        host: Option<String>,

        /// Device serial (e.g. "03W00X123456789")
        device_id: Option<String>,

        /// Access code from Bambu Studio / printer label
        access_code: Option<String>,

        /// Printer model: unknown, a1, a1mini, p1p, p1s, x1c, x1e
        #[arg(long)]
        model: Option<String>,

        /// Username (default: bblp)
        #[arg(long, default_value = "bblp")]
        username: String,

        /// RTSP port (default: 322 for RTSPS models, 6000 for proprietary)
        #[arg(long)]
        rtsp_port: Option<u16>,

        /// RTSP path (default: /streaming/live/1)
        #[arg(long)]
        rtsp_path: Option<String>,

        /// Load printers from a JSON file
        #[arg(short, long)]
        file: Option<String>,
    },

    /// Get printer details
    Get {
        /// Printer ID
        id: String,
    },

    /// Delete a printer
    Delete {
        /// Printer ID
        id: String,
    },

    /// Start a stream
    Start {
        /// Printer ID
        id: String,
    },

    /// Stop a stream
    Stop {
        /// Printer ID
        id: String,
    },

    /// Start streams for all printers
    StartAll,

    /// Stop streams for all printers
    StopAll,

    /// Get stream URL
    Url {
        /// Printer ID
        id: String,
    },

    /// Create a template printers.json file
    Init {
        /// Output file path
        #[arg(default_value = "printers.json")]
        output: String,
    },
}

#[derive(Serialize, Deserialize)]
struct PrinterEntry {
    id: String,
    host: String,
    device_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    access_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    rtsp_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rtsp_path: Option<String>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let client = Client::new();
    let base = cli.url.trim_end_matches('/').to_string();

    let result = match cli.command {
        Commands::Health => cmd_health(&client, &base, cli.json).await,
        Commands::List => cmd_list(&client, &base, cli.json).await,
        Commands::Add {
            id,
            host,
            device_id,
            access_code,
            model,
            username,
            rtsp_port,
            rtsp_path,
            file,
        } => {
            cmd_add(
                &client, &base, cli.json, id, host, device_id,
                access_code, model, username, rtsp_port, rtsp_path, file,
            )
            .await
        }
        Commands::Get { id } => cmd_get(&client, &base, cli.json, &id).await,
        Commands::Delete { id } => cmd_delete(&client, &base, cli.json, &id).await,
        Commands::Start { id } => cmd_start(&client, &base, cli.json, &id).await,
        Commands::Stop { id } => cmd_stop(&client, &base, cli.json, &id).await,
        Commands::StartAll => cmd_start_all(&client, &base, cli.json).await,
        Commands::StopAll => cmd_stop_all(&client, &base, cli.json).await,
        Commands::Url { id } => cmd_url(&client, &base, cli.json, &id).await,
        Commands::Init { output } => cmd_init(&output),
    };

    if let Err(e) = result {
        eprintln!("{} {}", "error:".red().bold(), e);
        process::exit(1);
    }
}

async fn cmd_health(client: &Client, base: &str, raw_json: bool) -> Result<(), String> {
    let resp = client
        .get(format!("{base}/health"))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| format!("parse error: {e}"))?;

    if raw_json {
        println!("{}", serde_json::to_string_pretty(&body).unwrap());
    } else {
        let ok = body["ok"].as_bool().unwrap_or(false);
        let printers = body["printers_registered"].as_u64().unwrap_or(0);
        let streams = body["streams_running"].as_u64().unwrap_or(0);
        if ok {
            println!("{} — {} printer(s), {} stream(s) running", "✓ healthy".green().bold(), printers, streams);
        } else {
            println!("{} — unhealthy", "✗".red().bold());
        }
    }
    Ok(())
}

async fn cmd_list(client: &Client, base: &str, raw_json: bool) -> Result<(), String> {
    let resp = client
        .get(format!("{base}/v1/printers"))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    let printers: Vec<serde_json::Value> = resp.json().await.map_err(|e| format!("parse error: {e}"))?;

    if raw_json {
        println!("{}", serde_json::to_string_pretty(&printers).unwrap());
        return Ok(());
    }

    if printers.is_empty() {
        println!("No printers registered.");
        return Ok(());
    }

    println!(
        "{:<20} {:<18} {:<12} {:<10} {:<12} {}",
        "ID".bold(),
        "Host".bold(),
        "Model".bold(),
        "Stream".bold(),
        "Type".bold(),
        "Updated".bold()
    );
    println!("{}", "─".repeat(95));

    for p in &printers {
        let id = p["id"].as_str().unwrap_or("?");
        let host = p["host"].as_str().unwrap_or("?");
        let model = p["model"].as_str().unwrap_or("unknown");
        let state = p["stream_state"].as_str().unwrap_or("?");
        let stream_type = p["stream_type"].as_str().unwrap_or("?");
        let updated = p["updated_at"].as_str().unwrap_or("?");

        let state_colored = match state {
            "running" => state.green().to_string(),
            "starting" => state.yellow().to_string(),
            "error" => state.red().to_string(),
            _ => state.dimmed().to_string(),
        };

        let type_colored = match stream_type {
            "rtsp" => stream_type.cyan().to_string(),
            "proprietary" => stream_type.red().to_string(),
            _ => stream_type.to_string(),
        };

        println!("{:<20} {:<18} {:<12} {:<10} {:<12} {}", id, host, model, state_colored, type_colored, updated);
    }

    println!("\n{} printer(s)", printers.len());
    Ok(())
}

async fn cmd_add(
    client: &Client,
    base: &str,
    raw_json: bool,
    id: Option<String>,
    host: Option<String>,
    device_id: Option<String>,
    access_code: Option<String>,
    model: Option<String>,
    username: String,
    rtsp_port: Option<u16>,
    rtsp_path: Option<String>,
    file: Option<String>,
) -> Result<(), String> {
    // Batch from file
    if let Some(path) = file {
        return cmd_add_file(client, base, raw_json, &path).await;
    }

    // Single printer — require all positional args
    let id = id.ok_or("missing <id> argument. Usage: bambu add <id> <host> <device_id> <access_code>")?;
    let host = host.ok_or("missing <host> argument")?;
    let device_id = device_id.ok_or("missing <device_id> argument")?;
    let access_code = access_code.ok_or("missing <access_code> argument")?;

    let mut body = serde_json::json!({
        "id": id,
        "host": host,
        "device_id": device_id,
        "username": username,
        "access_code": access_code,
    });

    if let Some(ref m) = model {
        body["model"] = serde_json::Value::String(m.to_lowercase());
    }
    if rtsp_port.is_some() {
        body["rtsp_port"] = serde_json::json!(rtsp_port);
    }
    if rtsp_path.is_some() {
        body["rtsp_path"] = serde_json::json!(rtsp_path);
    }

    let resp = client
        .post(format!("{base}/v1/printers"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP {status}: {text}"));
    }

    let printer: serde_json::Value = resp.json().await.map_err(|e| format!("parse error: {e}"))?;

    if raw_json {
        println!("{}", serde_json::to_string_pretty(&printer).unwrap());
    } else {
        println!("{} printer {}", "✓ added".green().bold(), printer["id"].as_str().unwrap_or("?"));
    }
    Ok(())
}

async fn cmd_add_file(
    client: &Client,
    base: &str,
    raw_json: bool,
    path: &str,
) -> Result<(), String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {path}: {e}"))?;

    let printers: Vec<PrinterEntry> = serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse {path}: {e}"))?;

    if printers.is_empty() {
        println!("No printers in file.");
        return Ok(());
    }

    let body = serde_json::json!({ "printers": printers });

    let resp = client
        .post(format!("{base}/v1/printers/batch"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    let result: serde_json::Value = resp.json().await.map_err(|e| format!("parse error: {e}"))?;

    if raw_json {
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
        return Ok(());
    }

    let created = result["created"].as_array().map(|a| a.len()).unwrap_or(0);
    let updated = result["updated"].as_array().map(|a| a.len()).unwrap_or(0);
    let errors = result["errors"].as_array().cloned().unwrap_or_default();

    if created > 0 {
        println!("{} {} printer(s)", "✓ created".green().bold(), created);
    }
    if updated > 0 {
        println!("{} {} printer(s)", "↻ updated".yellow().bold(), updated);
    }
    for err in &errors {
        eprintln!(
            "{} {}: {}",
            "✗".red().bold(),
            err["id"].as_str().unwrap_or("?"),
            err["error"].as_str().unwrap_or("unknown error")
        );
    }

    if !errors.is_empty() {
        process::exit(1);
    }
    Ok(())
}

async fn cmd_get(client: &Client, base: &str, raw_json: bool, id: &str) -> Result<(), String> {
    let resp = client
        .get(format!("{base}/v1/printers/{id}"))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if resp.status().as_u16() == 404 {
        return Err(format!("printer '{id}' not found"));
    }
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let printer: serde_json::Value = resp.json().await.map_err(|e| format!("parse error: {e}"))?;

    if raw_json {
        println!("{}", serde_json::to_string_pretty(&printer).unwrap());
        return Ok(());
    }

    let p = &printer["printer"];
    println!("{} {}", "Printer".bold(), p["id"].as_str().unwrap_or("?"));
    println!("  Host:        {}", p["host"].as_str().unwrap_or("?"));
    println!("  Device ID:   {}", p["device_id"].as_str().unwrap_or("?"));
    println!("  Model:       {}", p["model"].as_str().unwrap_or("unknown"));
    println!("  Username:    {}", p["credentials"]["username"].as_str().unwrap_or("?"));
    println!("  RTSP Port:   {}", p["stream"]["rtsp_port"].as_u64().unwrap_or(322));
    println!("  RTSP Path:   {}", p["stream"]["rtsp_path"].as_str().unwrap_or("?"));
    println!("  Stream Type: {}", p["stream"]["stream_type"].as_str().unwrap_or("?"));
    println!("  Created:     {}", p["created_at"].as_str().unwrap_or("?"));
    println!("  Updated:     {}", p["updated_at"].as_str().unwrap_or("?"));

    let state = printer["stream_state"].as_str().unwrap_or("?");
    let state_colored = match state {
        "running" => state.green().to_string(),
        "starting" => state.yellow().to_string(),
        "error" => state.red().to_string(),
        _ => state.dimmed().to_string(),
    };
    println!("  Stream:     {}", state_colored);

    if let Some(url) = printer["stream_url"].as_str() {
        println!("  WebRTC URL: {}", url.cyan());
    }
    println!("  RTSP Source: {}", printer["rtsp_source_url"].as_str().unwrap_or("?").dimmed());
    println!("  RTSP Publish: {}", printer["rtsp_publish_url"].as_str().unwrap_or("?").dimmed());

    Ok(())
}

async fn cmd_delete(client: &Client, base: &str, raw_json: bool, id: &str) -> Result<(), String> {
    let resp = client
        .delete(format!("{base}/v1/printers/{id}"))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if resp.status().as_u16() == 404 {
        return Err(format!("printer '{id}' not found"));
    }
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    if raw_json {
        println!("{{\"deleted\": \"{}\"}}", id);
    } else {
        println!("{} printer {}", "✓ deleted".green().bold(), id);
    }
    Ok(())
}

async fn cmd_start(client: &Client, base: &str, raw_json: bool, id: &str) -> Result<(), String> {
    let resp = client
        .post(format!("{base}/v1/printers/{id}/stream/start"))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if resp.status().as_u16() == 404 {
        return Err(format!("printer '{id}' not found"));
    }
    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP error: {text}"));
    }

    let result: serde_json::Value = resp.json().await.map_err(|e| format!("parse error: {e}"))?;

    if raw_json {
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
    } else {
        let state = result["state"].as_str().unwrap_or("?");
        let url = result["url"].as_str().unwrap_or("pending");
        println!("{} stream for {} — state: {}", "▶".green().bold(), id, state);
        if url != "pending" && !url.is_empty() {
            println!("  URL: {}", url.cyan());
        }
    }
    Ok(())
}

async fn cmd_stop(client: &Client, base: &str, raw_json: bool, id: &str) -> Result<(), String> {
    let resp = client
        .post(format!("{base}/v1/printers/{id}/stream/stop"))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if resp.status().as_u16() == 404 {
        return Err(format!("printer '{id}' not found"));
    }
    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP error: {text}"));
    }

    let result: serde_json::Value = resp.json().await.map_err(|e| format!("parse error: {e}"))?;

    if raw_json {
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
    } else {
        println!("{} stream for {}", "■".yellow().bold(), id);
    }
    Ok(())
}

async fn cmd_start_all(client: &Client, base: &str, raw_json: bool) -> Result<(), String> {
    let resp = client
        .post(format!("{base}/v1/streams/start"))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP error: {text}"));
    }

    let result: serde_json::Value = resp.json().await.map_err(|e| format!("parse error: {e}"))?;

    if raw_json {
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
        return Ok(());
    }

    let started = result["started"].as_array().map(|a| a.len()).unwrap_or(0);
    let errors = result["errors"].as_array().cloned().unwrap_or_default();

    if started > 0 {
        println!("{} {} stream(s)", "▶".green().bold(), started);
    }
    for err in &errors {
        eprintln!(
            "{} {}: {}",
            "✗".red().bold(),
            err["id"].as_str().unwrap_or("?"),
            err["error"].as_str().unwrap_or("unknown error")
        );
    }

    if !errors.is_empty() {
        process::exit(1);
    }
    Ok(())
}

async fn cmd_stop_all(client: &Client, base: &str, raw_json: bool) -> Result<(), String> {
    let resp = client
        .post(format!("{base}/v1/streams/stop"))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP error: {text}"));
    }

    let result: serde_json::Value = resp.json().await.map_err(|e| format!("parse error: {e}"))?;

    if raw_json {
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
        return Ok(());
    }

    let stopped = result["stopped"].as_array().map(|a| a.len()).unwrap_or(0);
    let errors = result["errors"].as_array().cloned().unwrap_or_default();

    if stopped > 0 {
        println!("{} {} stream(s)", "■".yellow().bold(), stopped);
    }
    for err in &errors {
        eprintln!(
            "{} {}: {}",
            "✗".red().bold(),
            err["id"].as_str().unwrap_or("?"),
            err["error"].as_str().unwrap_or("unknown error")
        );
    }

    if !errors.is_empty() {
        process::exit(1);
    }
    Ok(())
}

async fn cmd_url(client: &Client, base: &str, raw_json: bool, id: &str) -> Result<(), String> {
    let resp = client
        .get(format!("{base}/v1/printers/{id}/stream/url"))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if resp.status().as_u16() == 404 {
        return Err(format!("printer '{id}' not found"));
    }
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let result: serde_json::Value = resp.json().await.map_err(|e| format!("parse error: {e}"))?;

    if raw_json {
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
    } else {
        let state = result["state"].as_str().unwrap_or("?");
        match result["url"].as_str() {
            Some(url) if !url.is_empty() => {
                println!("{} stream {} — {}", state.green(), id, url.cyan());
            }
            _ => {
                println!("{} stream {} — no URL (state: {})", "⏸".yellow(), id, state);
            }
        }
    }
    Ok(())
}

fn cmd_init(output: &str) -> Result<(), String> {
    let template = vec![
        PrinterEntry {
            id: "x1c-1".to_string(),
            host: "192.168.1.100".to_string(),
            device_id: "03W00X123456789".to_string(),
            model: Some("x1c".to_string()),
            username: Some("bblp".to_string()),
            access_code: "12345678".to_string(),
            rtsp_port: None,
            rtsp_path: None,
        },
        PrinterEntry {
            id: "p1s-1".to_string(),
            host: "192.168.1.101".to_string(),
            device_id: "03W00X987654321".to_string(),
            model: Some("p1s".to_string()),
            username: None,
            access_code: "87654321".to_string(),
            rtsp_port: None,
            rtsp_path: None,
        },
        PrinterEntry {
            id: "a1mini-1".to_string(),
            host: "192.168.1.102".to_string(),
            device_id: "03W00X111222333".to_string(),
            model: Some("a1mini".to_string()),
            username: None,
            access_code: "11223344".to_string(),
            rtsp_port: None,
            rtsp_path: None,
        },
    ];

    let json = serde_json::to_string_pretty(&template)
        .map_err(|e| format!("serialize error: {e}"))?;

    std::fs::write(output, json)
        .map_err(|e| format!("write error: {e}"))?;

    println!("{} template written to {}", "✓".green().bold(), output);
    println!("Edit the file, then run: bambu add -f {}", output);
    Ok(())
}
