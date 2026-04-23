# Bambu Live API (Rust)

Rust web API for LAN-mode Bambu printer live feed orchestration.

## Scope

- API key protected endpoints
- Printer registration (host/device/access-code)
- Batch printer import from JSON
- Config-file printer loading on startup
- CLI tool (`bambu`) for easy printer management
- Per-printer stream process control (FFmpeg -> MediaMTX)
- WebRTC URL retrieval for active streams
- Designed to scale to 20+ printers with concurrency guardrails

## Requirements

- Rust stable toolchain
- ffmpeg installed
- MediaMTX running and reachable
- Printers with LAN mode liveview enabled

## Quick Start

1. Copy environment file and edit values.
2. Start service.

```bash
cd rust-api
cp .env.example .env
export $(grep -v '^#' .env | xargs)
cargo run
```

## CLI Tool

The `bambu` CLI makes it easy to manage printers without crafting curl commands:

```bash
# Build the CLI
cargo build --bin bambu

# Health check
cargo run --bin bambu -- health

# Add a single printer
cargo run --bin bambu -- add my-printer 192.168.1.100 03W00X123456789 12345678

# Add with custom options
cargo run --bin bambu -- add my-printer 192.168.1.100 03W00X123456789 12345678 \
  --username bblp --rtsp-port 322

# Batch add from JSON file
cargo run --bin bambu -- add -f printers.json

# List all printers
cargo run --bin bambu -- list

# Get printer details
cargo run --bin bambu -- get my-printer

# Start/stop stream
cargo run --bin bambu -- start my-printer
cargo run --bin bambu -- stop my-printer

# Get stream URL
cargo run --bin bambu -- url my-printer

# Delete a printer
cargo run --bin bambu -- delete my-printer

# Create a template printers.json
cargo run --bin bambu -- init
```

The CLI reads `BAMBU_API_URL` and `BAMBU_API_KEY` from environment, or use `--url` and `--key` flags.

## Printers Config File

You can pre-load printers on startup by setting `PRINTERS_FILE` to a JSON file path:

```bash
PRINTERS_FILE=printers.json cargo run
```

Example `printers.json`:

```json
[
  {
    "id": "a1-mini-1",
    "host": "192.168.1.100",
    "device_id": "03W00X123456789",
    "access_code": "12345678"
  },
  {
    "id": "x1c-1",
    "host": "192.168.1.101",
    "device_id": "03W00X987654321",
    "access_code": "87654321",
    "username": "bblp",
    "rtsp_port": 322,
    "rtsp_path": "/streaming/live/1"
  }
]
```

Generate a template with: `cargo run --bin bambu -- init`

## API

Health:

```bash
curl http://127.0.0.1:8080/health
```

Upsert printer:

```bash
curl -X POST http://127.0.0.1:8080/v1/printers \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "id":"printer-1",
    "host":"10.0.0.10",
    "device_id":"03W00X123456789",
    "username":"bblp",
    "access_code":"12345678"
  }'
```

Start stream:

```bash
curl -X POST http://127.0.0.1:8080/v1/printers/printer-1/stream/start \
  -H "Authorization: Bearer $API_KEY"
```

Stop stream:

```bash
curl -X POST http://127.0.0.1:8080/v1/printers/printer-1/stream/stop \
  -H "Authorization: Bearer $API_KEY"
```

Get stream URL:

```bash
curl http://127.0.0.1:8080/v1/printers/printer-1/stream/url \
  -H "Authorization: Bearer $API_KEY"
```

Delete printer:

```bash
curl -X DELETE http://127.0.0.1:8080/v1/printers/printer-1 \
  -H "Authorization: Bearer $API_KEY"
```

## Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/health` | No | Health check |
| POST | `/v1/printers` | Yes | Upsert (create/update) printer |
| POST | `/v1/printers/batch` | Yes | Batch upsert multiple printers |
| GET | `/v1/printers` | Yes | List all printers |
| GET | `/v1/printers/{id}` | Yes | Get printer details |
| DELETE | `/v1/printers/{id}` | Yes | Delete printer (stops stream if running) |
| POST | `/v1/printers/{id}/stream/start` | Yes | Start FFmpeg stream |
| POST | `/v1/printers/{id}/stream/stop` | Yes | Stop FFmpeg stream |
| GET | `/v1/printers/{id}/stream/url` | Yes | Get WebRTC stream URL |

### Batch Upsert

```bash
curl -X POST http://127.0.0.1:8080/v1/printers/batch \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "printers": [
      {"id":"p1","host":"10.0.0.1","device_id":"DEV001","access_code":"11111111"},
      {"id":"p2","host":"10.0.0.2","device_id":"DEV002","access_code":"22222222"}
    ]
  }'
```

Response:

```json
{
  "created": ["p1", "p2"],
  "updated": [],
  "errors": []
}
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `API_BIND` | `0.0.0.0:8080` | Bind address for the HTTP server |
| `API_KEY` | `change-me` | Bearer token for authenticated endpoints |
| `FFMPEG_BIN` | `ffmpeg` | Path to ffmpeg binary |
| `MEDIAMTX_RTSP_PUBLISH` | `rtsp://127.0.0.1:8554` | MediaMTX RTSP publish base URL |
| `WEBRTC_URL_TEMPLATE` | `http://127.0.0.1:8889/{id}/` | WebRTC URL template (`{id}` = printer ID) |
| `MAX_CONCURRENT_STREAMS` | `25` | Max concurrent FFmpeg processes |
| `PRINTERS_FILE` | _(none)_ | Path to JSON file with printer definitions (loaded on startup) |
