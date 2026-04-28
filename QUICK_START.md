# Bambu Farm - Quick Start Guide

## Build & Run

### Prerequisites
- Rust 1.83.0-nightly (current toolchain)
- Tokio async runtime
- All dependencies listed in Cargo.toml

### Build
```bash
cd rust-api
cargo build --release
```

### Run
```bash
cargo run --bin bambu-live-api
# Server starts on http://0.0.0.0:8080
```

---

### Build (PowerShell)
```powershell
cd rust-api
cargo build --release
```

### Run (PowerShell)
```powershell
cargo run --bin bambu-live-api
# Server starts on http://0.0.0.0:8080
```

## Quick Test Sequence

### 1. Login as Admin
A seed admin account is created automatically on first server startup:

| Field | Value |
|-------|-------|
| **Username** | `admin` |
| **Password** | `Admin1234!` |
| **Role** | `admin` |

```bash
curl -X POST http://localhost:8080/auth/login \
  -H "Content-Type: application/json" \
  -d '{"username":"admin","password":"Admin1234!"}'

# Response:
# {
#   "user_id": "abc123...",
#   "username": "admin",
#   "role": "admin",
#   "token": "def456..."
# }

# Save the token for subsequent requests
TOKEN="def456..."
```

```powershell
# PowerShell
$response = Invoke-RestMethod -Uri "http://localhost:8080/auth/login" `
  -Method POST `
  -ContentType "application/json" `
  -Body '{"username":"admin","password":"Admin1234!"}'

# Response is auto-parsed; save the token:
$TOKEN = $response.token
```

### 2. Change Admin Password (Recommended)
Change the seed admin password on first login. Self-service requires the current password:

```bash
# Get your user ID from the login response or /auth/me
ADMIN_USER_ID="abc123..."

curl -X PUT http://localhost:8080/admin/users/$ADMIN_USER_ID/password \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"current_password":"Admin1234!","new_password":"MyNewSecurePass1!"}'
```

```powershell
# PowerShell
$ADMIN_USER_ID = $response.user_id
$headers = @{ "Authorization" = "Bearer $TOKEN" }
Invoke-RestMethod -Uri "http://localhost:8080/admin/users/$ADMIN_USER_ID/password" `
  -Method PUT `
  -ContentType "application/json" `
  -Headers $headers `
  -Body '{"current_password":"Admin1234!","new_password":"MyNewSecurePass1!"}'
```

> **Note:** Changing the password revokes all active sessions — you'll need to log in again afterward.
>
> An admin can also reset another user's password without knowing the current one — just omit `current_password` from the request body.

### 3. Create Teacher Account
```bash
curl -X POST http://localhost:8080/admin/users \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "username": "teacher1",
    "email": "teacher@school.edu",
    "password": "TeacherPass123",
    "role": "teacher"
  }'
```

```powershell
# PowerShell
$headers = @{ "Authorization" = "Bearer $TOKEN" }
Invoke-RestMethod -Uri "http://localhost:8080/admin/users" `
  -Method POST `
  -ContentType "application/json" `
  -Headers $headers `
  -Body '{
    "username": "teacher1",
    "email": "teacher@school.edu",
    "password": "TeacherPass123",
    "role": "teacher"
  }'
```

### 4. Submit Print Job (Public - No Auth Required)
```bash
curl -X POST http://localhost:8080/api/v2/jobs/submit \
  -H "Content-Type: application/json" \
  -d '{
    "student_name": "Alice Johnson",
    "class_period": "Period 2",
    "filename": "bracket_assembly.3mf",
    "printer_model": "A1"
  }'

# Response:
# {
#   "id": "job123...",
#   "student_name": "Alice Johnson",
#   "status": "queued",
#   "progress_percent": 0,
#   ...
# }

JOB_ID="job123..."
```

```powershell
# PowerShell
$job = Invoke-RestMethod -Uri "http://localhost:8080/api/v2/jobs/submit" `
  -Method POST `
  -ContentType "application/json" `
  -Body '{
    "student_name": "Alice Johnson",
    "class_period": "Period 2",
    "filename": "bracket_assembly.3mf",
    "printer_model": "A1"
  }'

# Response is auto-parsed; save the job ID:
$JOB_ID = $job.id
```

### 5. View Job Queue (Teacher Auth Required)
```bash
TEACHER_TOKEN="teacher_token_from_step2"

curl http://localhost:8080/api/v2/jobs/queue \
  -H "Authorization: Bearer $TEACHER_TOKEN"

# Returns list of queued jobs in order
```

```powershell
# PowerShell
$TEACHER_TOKEN = "teacher_token_from_step2"
$headers = @{ "Authorization" = "Bearer $TEACHER_TOKEN" }
Invoke-RestMethod -Uri "http://localhost:8080/api/v2/jobs/queue" -Headers $headers
```

### 6. Dispatch Job to Printer (Teacher Auth Required)
```bash
PRINTER_ID="printer_01"

curl -X POST http://localhost:8080/api/v2/jobs/$JOB_ID/dispatch/$PRINTER_ID \
  -H "Authorization: Bearer $TEACHER_TOKEN"

# Response shows job status changed to "in_progress"
# The printer card on the dashboard now shows:
#   - Print status: Printing
#   - Task: (from printer telemetry)
#   - Task Info: Student name, class period, filename
```

```powershell
# PowerShell
$PRINTER_ID = "printer_01"
$headers = @{ "Authorization" = "Bearer $TEACHER_TOKEN" }
Invoke-RestMethod -Uri "http://localhost:8080/api/v2/jobs/$JOB_ID/dispatch/$PRINTER_ID" `
  -Method POST `
  -Headers $headers
```

### 7. Check Job Status (Public - Anyone)
```bash
curl http://localhost:8080/api/v2/jobs/$JOB_ID

# Response shows current job status, progress, printer assignment
```

```powershell
# PowerShell
Invoke-RestMethod -Uri "http://localhost:8080/api/v2/jobs/$JOB_ID"
```

### 8. Access Admin Console
Open browser: **http://localhost:8080/admin**

Features:
- Login with admin/teacher credentials
- Create new users
- View all jobs and queue
- Dashboard with metrics

### 9. View Printer Dashboard
Open browser: **http://localhost:8080/**

Each printer card shows:
- **Print status** badge (Printing / Finished / Idle)
- **Task** name from printer telemetry
- **Task info** panel with student name, class period, and filename (when a job is dispatched)
- Progress bar, temperatures, layers, remaining time
- Live stream preview

## Available Roles

### Admin
- Full system access
- Create/delete users
- Manage job queue
- Dispatch jobs to printers
- View analytics

### Teacher
- Manage job queue
- Dispatch jobs to printers
- View analytics
- Cannot create users

### Assistant
- View-only access
- Cannot manage queue

## API Reference

### Authentication
| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| POST | `/auth/login` | None | Login and get token |
| POST | `/auth/logout` | Bearer | Logout and revoke token |
| GET | `/auth/me` | Bearer | Get current user profile |

### User Management
| Method | Endpoint | Auth | Role Required | Description |
|--------|----------|------|---------------|-------------|
| POST | `/admin/users` | Bearer | Admin | Create new user |
| GET | `/admin/users` | Bearer | Admin | List all users |
| PUT | `/admin/users/{id}` | Bearer | Admin | Update user (username, email, role, active) |
| PUT | `/admin/users/{id}/password` | Bearer | Admin or Self | Change password (self=current required, admin=no current needed) |
| DELETE | `/admin/users/{id}` | Bearer | Admin | Delete user (cannot delete self) |

### Job Submission & Management
| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| POST | `/api/v2/jobs/submit` | None | Submit new job (public) |
| GET | `/api/v2/jobs/{id}` | None | Get job status (public) |
| GET | `/api/v2/jobs` | Bearer | List all jobs (staff) |
| GET | `/api/v2/jobs/queue` | Bearer | List queued jobs (staff) |
| POST | `/api/v2/jobs/{id}/cancel` | Bearer | Cancel job (staff) |
| POST | `/api/v2/jobs/{id}/dispatch/{printer_id}` | Bearer | Dispatch to printer (staff) |

## Common Usage Patterns

### Pattern 1: Student Submits Job
1. Student POST to `/api/v2/jobs/submit` (no auth needed)
2. Gets back job ID
3. Can check status with GET `/api/v2/jobs/{id}`

### Pattern 2: Teacher Manages Queue
1. Teacher logs in, gets token
2. GET `/api/v2/jobs/queue` to see pending jobs
3. POST `/api/v2/jobs/{id}/dispatch/{printer_id}` to start print
4. Monitor job status via GET `/api/v2/jobs/{id}`
5. View printer dashboard — card shows student name, period, and filename under Task Info

### Pattern 3: Admin Creates Accounts
1. Admin logs in, gets token
2. POST `/admin/users` to create teacher/assistant accounts
3. Share credentials with staff
4. Staff can now manage jobs

## Error Codes

| Code | Meaning |
|------|---------|
| 200 | Success |
| 201 | Created |
| 400 | Bad request (validation error) |
| 401 | Unauthorized (invalid/missing token) |
| 403 | Forbidden (insufficient permissions) |
| 404 | Not found |
| 500 | Server error |

## Troubleshooting

### "No token provided" Error
→ Add `Authorization: Bearer <token>` header

### "Invalid username or password"
→ Check credentials, user must exist and be active

### "Insufficient permissions"
→ User role cannot perform this action

### "Job not found"
→ Job ID is invalid or doesn't exist

### "Account locked due to too many failed attempts"
→ Too many failed login attempts; wait 15 minutes

## Configuration

Set environment variables before running:
```bash
export BIND_ADDR=0.0.0.0:8080          # Default listen address
export RUST_LOG=info                   # Log level (debug, info, warn, error)
export MAX_CONCURRENT_STREAMS=20       # Printer stream limit
```

```powershell
# PowerShell
$env:BIND_ADDR = "0.0.0.0:8080"          # Default listen address
$env:RUST_LOG = "info"                   # Log level (debug, info, warn, error)
$env:MAX_CONCURRENT_STREAMS = "20"       # Printer stream limit
```

## Admin Console

Access at: **http://localhost:8080/admin**

### Dashboard Tab
- Shows total users, total jobs, queued jobs

### Users Tab
- Create new users with role assignment
- View existing users and roles

### Print Jobs Tab
- View all submitted and completed jobs
- Check job progress and status

### Job Queue Tab
- See pending jobs in order
- Cancel queued jobs
- Dispatch jobs to printers

## Printer Dashboard

Access at: **http://localhost:8080/**

Each printer card displays:

| Section | Content |
|---------|--------|
| Print Status | Color-coded badge: Printing (blue), Finished (green), Idle (gray) |
| Task | Current task name from printer telemetry |
| Task Info | Student name, class period, filename (from job queue) |
| Progress | Bar with percentage |
| Metrics | Remaining time, layers, stream status |
| Temps | Nozzle, bed, chamber |
| Preview | Live camera stream or snapshot |

## Performance Notes

- In-memory storage optimized for < 10,000 concurrent jobs
- Session validation: O(1) lookup
- Job queue: O(n) for sequential dispatch (n = queue length)
- User lookup: O(n) by username (consider database for > 1000 users)

## Next Steps

1. **Bootstrap Admin Account**: Manually create first admin user via API
2. **Add More Users**: Use admin console to create teacher/assistant accounts
3. **Test Workflows**: Run through common job submission and dispatch patterns
4. **Monitor Logs**: Check `RUST_LOG=debug` for detailed operation traces
5. **Scale Up**: When ready for production, migrate to external database

## Support

For detailed implementation information, see `IMPLEMENTATION_SUMMARY.md`

For architecture details, see existing printer streaming and telemetry modules in `src/`
