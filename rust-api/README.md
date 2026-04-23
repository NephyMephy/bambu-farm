# Bambu Live API (Rust)

Rust web API for LAN-mode Bambu printer live feed orchestration.

## Scope

- Printer registration (host/device/access-code)
- Batch printer import from JSON
- Config-file printer loading on startup
- CLI tool (`bambu`) for easy printer management
- Per-printer stream process control (FFmpeg -> MediaMTX)
- WebRTC URL retrieval for active streams
- Live dashboard with all streams in a grid view
- Designed to scale to 20+ printers with concurrency guardrails
- Cross-platform: Linux, macOS, and Windows

## Requirements

- Rust stable toolchain
- ffmpeg installed (on Windows: install and add to PATH, or set `FFMPEG_BIN` to full path)
- MediaMTX running and reachable
- Printers with LAN mode liveview enabled

## Quick Start

### Linux / macOS

```bash
cd rust-api
cp .env.example .env
# Edit .env with your values
export $(grep -v '^#' .env | xargs)
cargo run
```

### Windows (PowerShell)

```powershell
cd rust-api
copy .env.example .env
# Edit .env with your values, then load env vars:
Get-Content .env | Where-Object { $_ -notmatch '^\s*#' -and $_ -match '=' } | ForEach-Object {
    $name, $value = $_.Split('=', 2)
    Set-Item -Path "env:$name" -Value $value
}
cargo run
```

Or set env vars inline:

```powershell
$env:MEDIAMTX_RTSP_PUBLISH = "rtsp://127.0.0.1:8554"; cargo run
```

### Windows (Command Prompt)

```cmd
cd rust-api
copy .env.example .env
rem Edit .env, then set each variable:
set API_BIND=0.0.0.0:8080
set FFMPEG_BIN=ffmpeg
set MEDIAMTX_RTSP_PUBLISH=rtsp://127.0.0.1:8554
set WEBRTC_URL_TEMPLATE=http://127.0.0.1:8889/{id}/
set MAX_CONCURRENT_STREAMS=25
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

# Add with model (recommended — auto-configures stream type)
cargo run --bin bambu -- add my-x1c 192.168.1.100 03W00X123456789 12345678 --model x1c

# Add with custom options
cargo run --bin bambu -- add my-printer 192.168.1.100 03W00X123456789 12345678 \
  --model x1c --username bblp --rtsp-port 322

# Batch add from JSON file
cargo run --bin bambu -- add -f printers.json

# List all printers
cargo run --bin bambu -- list

# Get printer details
cargo run --bin bambu -- get my-printer

# Start/stop stream
cargo run --bin bambu -- start my-printer
cargo run --bin bambu -- stop my-printer

# Start/stop all streams
cargo run --bin bambu -- start-all
cargo run --bin bambu -- stop-all

# Get stream URL
cargo run --bin bambu -- url my-printer

# Delete a printer
cargo run --bin bambu -- delete my-printer

# Create a template printers.json
cargo run --bin bambu -- init
```

The CLI reads `BAMBU_API_URL` from environment, or use `--url` flag.

**Windows examples:**

```powershell
# PowerShell — set env var
$env:BAMBU_API_URL = "http://127.0.0.1:8080"
cargo run --bin bambu -- list
```

```cmd
REM Command Prompt — set env var
set BAMBU_API_URL=http://127.0.0.1:8080
cargo run --bin bambu -- list
```

```powershell
# Or use flag (works on all platforms)
cargo run --bin bambu -- --url http://127.0.0.1:8080 list
```

## Printers Config File

You can pre-load printers on startup by setting `PRINTERS_FILE` to a JSON file path:

**Linux / macOS:**
```bash
PRINTERS_FILE=printers.json cargo run
```

**Windows (PowerShell):**
```powershell
$env:PRINTERS_FILE = "printers.json"
cargo run
```

**Windows (cmd):**
```cmd
set PRINTERS_FILE=printers.json
cargo run
```

Example `printers.json`:

```json
[
  {
    "id": "x1c-1",
    "host": "192.168.1.100",
    "device_id": "03W00X123456789",
    "model": "x1c",
    "access_code": "12345678"
  },
  {
    "id": "p1s-1",
    "host": "192.168.1.101",
    "device_id": "03W00X987654321",
    "model": "p1s",
    "access_code": "87654321"
  },
  {
    "id": "a1mini-1",
    "host": "192.168.1.102",
    "device_id": "03W00X111222333",
    "model": "a1mini",
    "access_code": "11223344"
  }
]
```

Generate a template with: `cargo run --bin bambu -- init`

## Printer Models

The API supports different Bambu printer models with model-aware stream configuration. Set the `model` field when adding a printer to get the correct defaults automatically.

| Model | Value | Stream Type | Port | FFmpeg Direct |
|-------|-------|-------------|------|---------------|
| X1 Carbon | `x1c` | RTSPS | 322 | ✅ Yes |
| X1E | `x1e` | RTSPS | 322 | ✅ Yes |
| P1P | `p1p` | Proprietary TCP | 6000 | ❌ No |
| P1S | `p1s` | Proprietary TCP | 6000 | ❌ No |
| A1 | `a1` | Proprietary TCP | 6000 | ❌ No |
| A1 Mini | `a1mini` | Proprietary TCP | 6000 | ❌ No |
| Unknown | `unknown` | RTSPS (assumed) | 322 | ✅ (if RTSPS-capable) |

### RTSPS Models (X1C, X1E)

These models have a native RTSPS server. FFmpeg connects directly:

```
rtsps://bblp:<access_code>@<host>:322/streaming/live/1
```

**Requirements:**
- LAN Mode Liveview must be enabled on the printer
- Only one RTSPS client can connect at a time (close Bambu Studio camera view first)

### Proprietary Models (P1P, P1S, A1, A1 Mini)

These models use a proprietary TCP JPEG streaming protocol on port 6000. **FFmpeg cannot connect directly.** To stream from these printers, you need a bridge:

1. **[BambuP1Streamer](https://github.com/slynn1324/BambuP1Streamer)** — Converts the proprietary stream to MJPEG/RTSP
2. **[go2rtc](https://github.com/AlexxIT/go2rtc)** — Can consume the BambuP1Streamer output and publish to MediaMTX

Once you have a bridge running, you can override the stream settings:

```bash
# Example: P1S with go2rtc bridge on port 8554
cargo run --bin bambu -- add my-p1s 192.168.1.101 03W00X987654321 87654321 \
  --model p1s --rtsp-port 8554 --rtsp-path /my-p1s
```

Or via the API, set `stream_type` to `"rtsp"` with custom `rtsp_port`/`rtsp_path` pointing to your bridge.

## Dashboard

Open `http://127.0.0.1:8080/` in your browser to see the live dashboard. It shows all registered printers in a grid with:

- Printer ID and host IP labels
- Live stream state badges (running / starting / stopped / error)
- Embedded WebRTC stream iframes for running printers
- Per-printer Start / Stop buttons
- **Start All** / **Stop All** buttons in the header
- Auto-refreshes every 10 seconds

The dashboard is a single HTML page served by the API — no additional build step or frontend server needed.

## Printer Models

Different Bambu printer models use different streaming protocols. The API uses the `model` field to automatically configure the correct stream type:

| Model | Value | Stream Type | Port | FFmpeg Direct? |
|-------|-------|-------------|------|----------------|
| X1 Carbon | `x1c` | RTSPS | 322 | ✅ Yes |
| X1E | `x1e` | RTSPS | 322 | ✅ Yes |
| P1P | `p1p` | Proprietary TCP | 6000 | ❌ No — needs bridge |
| P1S | `p1s` | Proprietary TCP | 6000 | ❌ No — needs bridge |
| A1 | `a1` | Proprietary TCP | 6000 | ❌ No — needs bridge |
| A1 Mini | `a1mini` | Proprietary TCP | 6000 | ❌ No — needs bridge |
| Unknown | `unknown` | RTSPS (assumed) | 322 | ✅ (assumed) |

### RTSPS Models (X1C, X1E)

These models have a built-in RTSP server. FFmpeg connects directly:

```
rtsps://bblp:<access_code>@<host>:322/streaming/live/1
```

**Requirements:**
- LAN Mode Liveview must be enabled on the printer
- Only one RTSP client at a time (Bambu Studio and this API cannot view simultaneously)

### Proprietary Models (P1P, P1S, A1, A1 Mini)

These models use a proprietary TCP JPEG streaming protocol on port 6000. **FFmpeg cannot connect directly.** You need a bridge:

1. **[BambuP1Streamer](https://github.com/slynn1324/BambuP1Streamer)** — Converts the proprietary stream to MJPEG/RTSP
2. **[go2rtc](https://github.com/AlexxIT/go2rtc)** — Can consume BambuP1Streamer output and republish as WebRTC

Once you have a bridge running, configure the printer with custom `rtsp_port` and `rtsp_path` pointing to the bridge:

```bash
# Example: bridge running on same host at rtsp://127.0.0.1:8554/p1s-1
cargo run --bin bambu -- add p1s-1 192.168.1.101 03W00X987654321 87654321 \
  --model p1s --rtsp-port 8554 --rtsp-path /p1s-1
```

Or in `printers.json`:
```json
{
  "id": "p1s-1",
  "host": "192.168.1.101",
  "device_id": "03W00X987654321",
  "model": "p1s",
  "access_code": "87654321",
  "rtsp_port": 8554,
  "rtsp_path": "/p1s-1"
}
```

### Adding Printers with Model

```bash
# X1 Carbon — works out of the box
cargo run --bin bambu -- add my-x1c 192.168.1.100 03W00X123456789 12345678 --model x1c

# P1S — will show warning, needs bridge for streaming
cargo run --bin bambu -- add my-p1s 192.168.1.101 03W00X987654321 87654321 --model p1s

# A1 Mini — same as P1S, needs bridge
cargo run --bin bambu -- add my-a1mini 192.168.1.102 03W00X111222333 11223344 --model a1mini
```

When you try to start a stream on a proprietary model without a bridge, the API returns a clear error explaining the requirement.

## API

### Health

```bash
curl http://127.0.0.1:8080/health
```

**Windows (PowerShell):**
```powershell
Invoke-RestMethod http://127.0.0.1:8080/health
```

### Upsert printer

```bash
curl -X POST http://127.0.0.1:8080/v1/printers \
  -H "Content-Type: application/json" \
  -d '{
    "id":"printer-1",
    "host":"10.0.0.10",
    "device_id":"03W00X123456789",
    "model":"x1c",
    "username":"bblp",
    "access_code":"12345678"
  }'
```

**Windows (PowerShell):**
```powershell
$body = @{
    id = "printer-1"
    host = "10.0.0.10"
    device_id = "03W00X123456789"
    model = "x1c"
    username = "bblp"
    access_code = "12345678"
} | ConvertTo-Json

Invoke-RestMethod -Uri http://127.0.0.1:8080/v1/printers `
  -Method Post -ContentType "application/json" -Body $body
```

### List printers

```bash
curl http://127.0.0.1:8080/v1/printers
```

**Windows (PowerShell):**
```powershell
Invoke-RestMethod -Uri http://127.0.0.1:8080/v1/printers
```

### Get printer details

```bash
curl http://127.0.0.1:8080/v1/printers/printer-1
```

**Windows (PowerShell):**
```powershell
Invoke-RestMethod -Uri http://127.0.0.1:8080/v1/printers/printer-1
```

### Start stream

```bash
curl -X POST http://127.0.0.1:8080/v1/printers/printer-1/stream/start
```

**Windows (PowerShell):**
```powershell
Invoke-RestMethod -Uri http://127.0.0.1:8080/v1/printers/printer-1/stream/start -Method Post
```

### Stop stream

```bash
curl -X POST http://127.0.0.1:8080/v1/printers/printer-1/stream/stop
```

**Windows (PowerShell):**
```powershell
Invoke-RestMethod -Uri http://127.0.0.1:8080/v1/printers/printer-1/stream/stop -Method Post
```

### Start all streams

```bash
curl -X POST http://127.0.0.1:8080/v1/streams/start
```

**Windows (PowerShell):**
```powershell
Invoke-RestMethod -Uri http://127.0.0.1:8080/v1/streams/start -Method Post
```

### Stop all streams

```bash
curl -X POST http://127.0.0.1:8080/v1/streams/stop
```

**Windows (PowerShell):**
```powershell
Invoke-RestMethod -Uri http://127.0.0.1:8080/v1/streams/stop -Method Post
```

### Get stream URL

```bash
curl http://127.0.0.1:8080/v1/printers/printer-1/stream/url
```

**Windows (PowerShell):**
```powershell
Invoke-RestMethod -Uri http://127.0.0.1:8080/v1/printers/printer-1/stream/url
```

### Delete printer

```bash
curl -X DELETE http://127.0.0.1:8080/v1/printers/printer-1
```

**Windows (PowerShell):**
```powershell
Invoke-RestMethod -Uri http://127.0.0.1:8080/v1/printers/printer-1 -Method Delete
```

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Health check |
| POST | `/v1/printers` | Upsert (create/update) printer |
| POST | `/v1/printers/batch` | Batch upsert multiple printers |
| GET | `/v1/printers` | List all printers |
| GET | `/v1/printers/{id}` | Get printer details |
| DELETE | `/v1/printers/{id}` | Delete printer (stops stream if running) |
| POST | `/v1/printers/{id}/stream/start` | Start FFmpeg stream |
| POST | `/v1/printers/{id}/stream/stop` | Stop FFmpeg stream |
| GET | `/v1/printers/{id}/stream/url` | Get WebRTC stream URL |
| POST | `/v1/streams/start` | Start streams for all printers |
| POST | `/v1/streams/stop` | Stop streams for all printers |

### Batch Upsert

```bash
curl -X POST http://127.0.0.1:8080/v1/printers/batch \
  -H "Content-Type: application/json" \
  -d '{
    "printers": [
      {"id":"p1","host":"10.0.0.1","device_id":"DEV001","access_code":"11111111"},
      {"id":"p2","host":"10.0.0.2","device_id":"DEV002","access_code":"22222222"}
    ]
  }'
```

**Windows (PowerShell):**
```powershell
$body = @{
    printers = @(
        @{ id = "p1"; host = "10.0.0.1"; device_id = "DEV001"; access_code = "11111111" },
        @{ id = "p2"; host = "10.0.0.2"; device_id = "DEV002"; access_code = "22222222" }
    )
} | ConvertTo-Json -Depth 3

Invoke-RestMethod -Uri http://127.0.0.1:8080/v1/printers/batch `
  -Method Post -ContentType "application/json" -Body $body
```

Response:

```json
{
  "created": ["p1", "p2"],
  "updated": [],
  "errors": []
}
```

## Testing

**Linux / macOS:**
```bash
bash test-api.sh
```

**Windows (PowerShell):**
```powershell
.\test-api.ps1
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `API_BIND` | `0.0.0.0:8080` | Bind address for the HTTP server |
| `FFMPEG_BIN` | `ffmpeg` | Path to ffmpeg binary (Windows: use full path e.g. `C:\tools\ffmpeg\bin\ffmpeg.exe`) |
| `MEDIAMTX_RTSP_PUBLISH` | `rtsp://127.0.0.1:8554` | MediaMTX RTSP publish base URL |
| `WEBRTC_URL_TEMPLATE` | `http://127.0.0.1:8889/{id}/` | WebRTC URL template (`{id}` = printer ID) |
| `MAX_CONCURRENT_STREAMS` | `25` | Max concurrent FFmpeg processes |
| `PRINTERS_FILE` | _(none)_ | Path to JSON file with printer definitions (loaded on startup) |

## Windows Notes

- **FFmpeg**: Download from [ffmpeg.org](https://ffmpeg.org/download.html), extract, and either add to `PATH` or set `FFMPEG_BIN` to the full executable path (e.g. `C:\tools\ffmpeg\bin\ffmpeg.exe`)
- **MediaMTX**: Download the Windows binary from [github.com/bluenviron/mediamtx/releases](https://github.com/bluenviron/mediamtx/releases)
- **Process management**: The API uses `taskkill /F /T /PID` on Windows to properly kill FFmpeg process trees (prevents orphaned processes)
- **Graceful shutdown**: Ctrl+C works on all platforms; SIGTERM is only available on Unix
- **Test script**: Use `test-api.ps1` (PowerShell) instead of `test-api.sh` (bash)
- **PowerShell API calls**: Use `Invoke-RestMethod` as shown in the examples above — PowerShell's `curl` is an alias for `Invoke-WebRequest`, not the real curl
