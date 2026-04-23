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
$env:API_KEY = "mysecret"; $env:MEDIAMTX_RTSP_PUBLISH = "rtsp://127.0.0.1:8554"; cargo run
```

### Windows (Command Prompt)

```cmd
cd rust-api
copy .env.example .env
rem Edit .env, then set each variable:
set API_BIND=0.0.0.0:8080
set API_KEY=mysecret
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

**Windows examples:**

```powershell
# PowerShell — set env vars
$env:BAMBU_API_URL = "http://127.0.0.1:8080"
$env:BAMBU_API_KEY = "mysecret"
cargo run --bin bambu -- list
```

```cmd
REM Command Prompt — set env vars
set BAMBU_API_URL=http://127.0.0.1:8080
set BAMBU_API_KEY=mysecret
cargo run --bin bambu -- list
```

```powershell
# Or use flags (works on all platforms)
cargo run --bin bambu -- --url http://127.0.0.1:8080 --key mysecret list
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

### Health

**Linux / macOS:**
```bash
curl http://127.0.0.1:8080/health
```

**Windows (PowerShell):**
```powershell
Invoke-RestMethod http://127.0.0.1:8080/health
```

### Upsert printer

**Linux / macOS:**
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

**Windows (PowerShell):**
```powershell
$headers = @{ Authorization = "Bearer $($env:API_KEY)" }
$body = @{
    id = "printer-1"
    host = "10.0.0.10"
    device_id = "03W00X123456789"
    username = "bblp"
    access_code = "12345678"
} | ConvertTo-Json

Invoke-RestMethod -Uri http://127.0.0.1:8080/v1/printers `
  -Method Post -Headers $headers -ContentType "application/json" -Body $body
```

### List printers

**Linux / macOS:**
```bash
curl -H "Authorization: Bearer $API_KEY" http://127.0.0.1:8080/v1/printers
```

**Windows (PowerShell):**
```powershell
$headers = @{ Authorization = "Bearer $($env:API_KEY)" }
Invoke-RestMethod -Uri http://127.0.0.1:8080/v1/printers -Headers $headers
```

### Get printer details

**Linux / macOS:**
```bash
curl -H "Authorization: Bearer $API_KEY" http://127.0.0.1:8080/v1/printers/printer-1
```

**Windows (PowerShell):**
```powershell
$headers = @{ Authorization = "Bearer $($env:API_KEY)" }
Invoke-RestMethod -Uri http://127.0.0.1:8080/v1/printers/printer-1 -Headers $headers
```

### Start stream

**Linux / macOS:**
```bash
curl -X POST http://127.0.0.1:8080/v1/printers/printer-1/stream/start \
  -H "Authorization: Bearer $API_KEY"
```

**Windows (PowerShell):**
```powershell
$headers = @{ Authorization = "Bearer $($env:API_KEY)" }
Invoke-RestMethod -Uri http://127.0.0.1:8080/v1/printers/printer-1/stream/start `
  -Method Post -Headers $headers
```

### Stop stream

**Linux / macOS:**
```bash
curl -X POST http://127.0.0.1:8080/v1/printers/printer-1/stream/stop \
  -H "Authorization: Bearer $API_KEY"
```

**Windows (PowerShell):**
```powershell
$headers = @{ Authorization = "Bearer $($env:API_KEY)" }
Invoke-RestMethod -Uri http://127.0.0.1:8080/v1/printers/printer-1/stream/stop `
  -Method Post -Headers $headers
```

### Get stream URL

**Linux / macOS:**
```bash
curl -H "Authorization: Bearer $API_KEY" http://127.0.0.1:8080/v1/printers/printer-1/stream/url
```

**Windows (PowerShell):**
```powershell
$headers = @{ Authorization = "Bearer $($env:API_KEY)" }
Invoke-RestMethod -Uri http://127.0.0.1:8080/v1/printers/printer-1/stream/url -Headers $headers
```

### Delete printer

**Linux / macOS:**
```bash
curl -X DELETE http://127.0.0.1:8080/v1/printers/printer-1 \
  -H "Authorization: Bearer $API_KEY"
```

**Windows (PowerShell):**
```powershell
$headers = @{ Authorization = "Bearer $($env:API_KEY)" }
Invoke-RestMethod -Uri http://127.0.0.1:8080/v1/printers/printer-1 `
  -Method Delete -Headers $headers
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

**Linux / macOS:**
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

**Windows (PowerShell):**
```powershell
$headers = @{ Authorization = "Bearer $($env:API_KEY)" }
$body = @{
    printers = @(
        @{ id = "p1"; host = "10.0.0.1"; device_id = "DEV001"; access_code = "11111111" },
        @{ id = "p2"; host = "10.0.0.2"; device_id = "DEV002"; access_code = "22222222" }
    )
} | ConvertTo-Json -Depth 3

Invoke-RestMethod -Uri http://127.0.0.1:8080/v1/printers/batch `
  -Method Post -Headers $headers -ContentType "application/json" -Body $body
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
| `API_KEY` | `change-me` | Bearer token for authenticated endpoints |
| `FFMPEG_BIN` | `ffmpeg` | Path to ffmpeg binary (Windows: use full path e.g. `C:\tools\ffmpeg\bin\ffmpeg.exe`) |
| `MEDIAMTX_RTSP_PUBLISH` | `rtsp://127.0.0.1:8554` | MediaMTX RTSP publish base URL |
| `WEBRTC_URL_TEMPLATE` | `http://127.0.0.1:8889/{id}/` | WebRTC URL template (`{id}` = printer ID) |
| `MAX_CONCURRENT_STREAMS` | `25` | Max concurrent FFmpeg processes |
| `PRINTERS_FILE` | _(none)_ | Path to JSON file with printer definitions (loaded on startup) |

## Windows Notes

- **`curl` is an alias**: PowerShell's `curl` is an alias for `Invoke-WebRequest`, not the real curl. Use `Invoke-RestMethod` (as shown in the API examples above) or install real curl via `winget install curl.curl`
- **FFmpeg**: Download from [ffmpeg.org](https://ffmpeg.org/download.html), extract, and either add to `PATH` or set `FFMPEG_BIN` to the full executable path (e.g. `C:\tools\ffmpeg\bin\ffmpeg.exe`)
- **MediaMTX**: Download the Windows binary from [github.com/bluenviron/mediamtx/releases](https://github.com/bluenviron/mediamtx/releases)
- **Process management**: The API uses `taskkill /F /T /PID` on Windows to properly kill FFmpeg process trees (prevents orphaned processes)
- **Graceful shutdown**: Ctrl+C works on all platforms; SIGTERM is only available on Unix
- **Test script**: Use `test-api.ps1` (PowerShell) instead of `test-api.sh` (bash)
- **JSON quoting**: In PowerShell, use `ConvertTo-Json` to build request bodies instead of raw JSON strings — it handles all escaping automatically
