-- Bambu Farm Database Schema
-- SQLite for embedded persistence of users, sessions, print jobs, and audit logs

-- Users table: staff accounts with roles
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    username TEXT UNIQUE NOT NULL,
    email TEXT,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL CHECK(role IN ('admin', 'teacher', 'assistant')),
    is_active BOOLEAN NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_users_username ON users(username);
CREATE INDEX idx_users_is_active ON users(is_active);

-- Sessions table: active user sessions with server-side revocation
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    ip_address TEXT NOT NULL,
    user_agent TEXT,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX idx_sessions_user_id ON sessions(user_id);
CREATE INDEX idx_sessions_expires_at ON sessions(expires_at);
CREATE INDEX idx_sessions_ip_address ON sessions(ip_address);

-- Login attempts table: for rate limiting and security audit
CREATE TABLE IF NOT EXISTS login_attempts (
    id TEXT PRIMARY KEY,
    ip_address TEXT NOT NULL,
    username TEXT,
    attempt_timestamp TEXT NOT NULL,
    success BOOLEAN NOT NULL
);

CREATE INDEX idx_login_attempts_ip_address ON login_attempts(ip_address, attempt_timestamp);
CREATE INDEX idx_login_attempts_username ON login_attempts(username, attempt_timestamp);

-- Print jobs table: student submissions and queue state
CREATE TABLE IF NOT EXISTS print_jobs (
    id TEXT PRIMARY KEY,
    student_name TEXT NOT NULL,
    class_period TEXT NOT NULL,
    filename TEXT NOT NULL,
    printer_model TEXT NOT NULL CHECK(printer_model IN ('a1', 'a1mini', 'p1p', 'p1s', 'x1c', 'x1e')),
    printer_id TEXT,
    file_path TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('queued', 'in_progress', 'completed', 'cancelled', 'error')),
    progress_percent INTEGER DEFAULT 0,
    error_message TEXT,
    submitted_at TEXT NOT NULL,
    started_at TEXT,
    completed_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (printer_id) REFERENCES printers(id)
);

CREATE INDEX idx_jobs_status ON print_jobs(status);
CREATE INDEX idx_jobs_printer_id ON print_jobs(printer_id);
CREATE INDEX idx_jobs_submitted_at ON print_jobs(submitted_at);

-- Job events table: audit trail of all job state changes
CREATE TABLE IF NOT EXISTS job_events (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    action TEXT NOT NULL,
    actor_type TEXT NOT NULL CHECK(actor_type IN ('student', 'staff')),
    actor_id TEXT,
    timestamp TEXT NOT NULL,
    notes TEXT,
    FOREIGN KEY (job_id) REFERENCES print_jobs(id) ON DELETE CASCADE,
    FOREIGN KEY (actor_id) REFERENCES users(id) ON DELETE SET NULL
);

CREATE INDEX idx_job_events_job_id ON job_events(job_id);
CREATE INDEX idx_job_events_timestamp ON job_events(timestamp);

-- Printers table: printer registry with live state
CREATE TABLE IF NOT EXISTS printers (
    id TEXT PRIMARY KEY,
    host TEXT NOT NULL,
    device_id TEXT NOT NULL,
    model TEXT NOT NULL CHECK(model IN ('unknown', 'a1', 'a1mini', 'p1p', 'p1s', 'x1c', 'x1e')),
    stream_type TEXT NOT NULL CHECK(stream_type IN ('rtsp', 'proprietary')),
    rtsp_port INTEGER NOT NULL,
    rtsp_path TEXT NOT NULL,
    username TEXT NOT NULL DEFAULT 'bblp',
    access_code TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_printers_model ON printers(model);

-- Audit log table: comprehensive audit trail of user actions
CREATE TABLE IF NOT EXISTS audit_log (
    id TEXT PRIMARY KEY,
    user_id TEXT,
    action TEXT NOT NULL,
    resource_type TEXT,
    resource_id TEXT,
    changes TEXT,
    timestamp TEXT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE SET NULL
);

CREATE INDEX idx_audit_log_user_id ON audit_log(user_id);
CREATE INDEX idx_audit_log_timestamp ON audit_log(timestamp);

-- Settings table: runtime configuration (key-value pairs)
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    data_type TEXT NOT NULL CHECK(data_type IN ('string', 'integer', 'boolean')),
    updated_at TEXT NOT NULL
);

-- Default settings (will be inserted if not exist)
INSERT OR IGNORE INTO settings (key, value, data_type, updated_at) VALUES
('max_file_size_mb', '100', 'integer', CURRENT_TIMESTAMP),
('session_timeout_hours', '8', 'integer', CURRENT_TIMESTAMP),
('rate_limit_max_attempts', '10', 'integer', CURRENT_TIMESTAMP),
('rate_limit_window_secs', '900', 'integer', CURRENT_TIMESTAMP),
('class_periods', 'Period 1,Period 2,Period 3,Period 4', 'string', CURRENT_TIMESTAMP);
