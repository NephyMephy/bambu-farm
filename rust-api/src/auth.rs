use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// User role with permission boundaries
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Teacher,
    Assistant,
}

impl Role {
    pub fn can_manage_users(&self) -> bool {
        matches!(self, Role::Admin)
    }

    #[allow(dead_code)]
    pub fn can_manage_settings(&self) -> bool {
        matches!(self, Role::Admin)
    }

    pub fn can_manage_queue(&self) -> bool {
        !matches!(self, Role::Assistant)
    }

    #[allow(dead_code)]
    pub fn can_view_analytics(&self) -> bool {
        matches!(self, Role::Admin | Role::Teacher)
    }

    pub fn can_dispatch_jobs(&self) -> bool {
        !matches!(self, Role::Assistant)
    }
}

/// User account
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub password_hash: String,
    pub role: Role,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Active session
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub token_hash: String,
    pub ip_address: String,
    pub expires_at: DateTime<Utc>,
    #[allow(dead_code)]
    pub created_at: DateTime<Utc>,
}

/// Login request
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// Login response
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub user_id: String,
    pub username: String,
    pub role: Role,
    pub token: String,
}

/// In-memory user store
pub struct UserStore {
    users: Arc<RwLock<HashMap<String, User>>>,
    sessions: Arc<RwLock<HashMap<String, Session>>>,
    login_attempts: Arc<RwLock<Vec<LoginAttempt>>>,
}

#[derive(Clone)]
struct LoginAttempt {
    ip_address: String,
    username: String,
    timestamp: DateTime<Utc>,
    success: bool,
}

impl UserStore {
    pub fn new() -> Self {
        Self {
            users: Arc::new(RwLock::new(HashMap::new())),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            login_attempts: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Simple password hash (in production, use bcrypt from a compatible crate)
    fn hash_password(plain: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        plain.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    /// Verify password
    fn verify_password(plain: &str, hash: &str) -> bool {
        Self::hash_password(plain) == hash
    }

    /// Create a new user
    pub async fn create_user(
        &self,
        username: String,
        email: Option<String>,
        password: String,
        role: Role,
    ) -> Result<User, String> {
        // Validate inputs
        if username.len() < 3 || username.len() > 32 {
            return Err("Username must be 3-32 characters".to_string());
        }
        if !username.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err("Username must be alphanumeric or underscore only".to_string());
        }
        if password.len() < 10 {
            return Err("Password must be at least 10 characters".to_string());
        }
        if !password.chars().any(|c| c.is_uppercase()) {
            return Err("Password must contain uppercase letter".to_string());
        }
        if !password.chars().any(|c| c.is_lowercase()) {
            return Err("Password must contain lowercase letter".to_string());
        }
        if !password.chars().any(|c| c.is_numeric()) {
            return Err("Password must contain digit".to_string());
        }

        let mut users = self.users.write().await;
        if users.values().any(|u| u.username == username) {
            return Err("Username already exists".to_string());
        }

        let user = User {
            id: uuid_simple(),
            username,
            email,
            password_hash: Self::hash_password(&password),
            role,
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        users.insert(user.id.clone(), user.clone());
        Ok(user)
    }

    /// Get user by username
    pub async fn get_user_by_username(&self, username: &str) -> Option<User> {
        let users = self.users.read().await;
        users.values().find(|u| u.username == username).cloned()
    }

    /// Get user by ID
    pub async fn get_user_by_id(&self, id: &str) -> Option<User> {
        self.users.read().await.get(id).cloned()
    }

    /// List all users
    pub async fn list_users(&self) -> Vec<User> {
        self.users
            .read()
            .await
            .values()
            .cloned()
            .collect()
    }

    /// Authenticate user
    pub async fn authenticate(
        &self,
        username: &str,
        password: &str,
        ip_address: &str,
    ) -> Result<User, String> {
        // Check rate limiting
        self.check_rate_limit(username, ip_address).await?;

        let user = self
            .get_user_by_username(username)
            .await
            .ok_or_else(|| "Invalid username or password".to_string())?;

        if !user.is_active {
            self.log_login_attempt(username, ip_address, false)
                .await;
            return Err("Account is inactive".to_string());
        }

        if !Self::verify_password(password, &user.password_hash) {
            self.log_login_attempt(username, ip_address, false)
                .await;
            return Err("Invalid username or password".to_string());
        }

        self.log_login_attempt(username, ip_address, true)
            .await;
        Ok(user)
    }

    /// Create session
    pub async fn create_session(&self, user_id: String, ip_address: String) -> Session {
        let session = Session {
            id: uuid_simple(),
            user_id,
            token_hash: format!("{:x}", std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()),
            ip_address,
            expires_at: Utc::now() + chrono::Duration::hours(8),
            created_at: Utc::now(),
        };

        self.sessions
            .write()
            .await
            .insert(session.id.clone(), session.clone());

        session
    }

    /// Verify session
    pub async fn verify_session(&self, token_hash: &str, ip_address: &str) -> Option<User> {
        let session_opt = {
            let sessions = self.sessions.read().await;
            sessions.values().find(|s| s.token_hash == token_hash).cloned()
        };

        let session = session_opt?;

        // Check expiry
        if session.expires_at < Utc::now() {
            self.sessions.write().await.remove(&session.id);
            return None;
        }

        // Check IP (optional: strict IP pinning)
        if session.ip_address != ip_address {
            return None;
        }

        self.get_user_by_id(&session.user_id).await
    }

    /// Revoke session
    pub async fn revoke_session(&self, token_hash: &str) {
        let session_id = {
            let sessions = self.sessions.read().await;
            sessions.values().find(|s| s.token_hash == token_hash).map(|s| s.id.clone())
        };

        if let Some(id) = session_id {
            self.sessions.write().await.remove(&id);
        }
    }

    /// Check rate limiting
    async fn check_rate_limit(&self, username: &str, ip_address: &str) -> Result<(), String> {
        let attempts = self.login_attempts.read().await;
        let now = Utc::now();
        let window = now - chrono::Duration::minutes(15);

        // Count failed attempts for username in last 15 min
        let failed_user_attempts = attempts
            .iter()
            .filter(|a| a.username == username && a.timestamp > window && !a.success)
            .count();

        if failed_user_attempts >= 10 {
            return Err("Account locked due to too many failed attempts".to_string());
        }

        // Count attempts for IP
        let ip_attempts: Vec<_> = attempts
            .iter()
            .filter(|a| a.ip_address == ip_address && a.timestamp > window)
            .collect();

        if ip_attempts.len() > 5 {
            // Exponential backoff would go here
            return Err("Too many login attempts from your IP".to_string());
        }

        Ok(())
    }

    /// Log login attempt
    async fn log_login_attempt(&self, username: &str, ip_address: &str, success: bool) {
        let attempt = LoginAttempt {
            ip_address: ip_address.to_string(),
            username: username.to_string(),
            timestamp: Utc::now(),
            success,
        };

        self.login_attempts.write().await.push(attempt);
    }

    /// Update user profile (username, email, role, active status)
    pub async fn update_user(
        &self,
        user_id: &str,
        username: Option<String>,
        email: Option<Option<String>>,
        role: Option<Role>,
        is_active: Option<bool>,
    ) -> Result<User, String> {
        // Validate username uniqueness before acquiring write lock
        if let Some(ref new_username) = username {
            if new_username.len() < 3 || new_username.len() > 32 {
                return Err("Username must be 3-32 characters".to_string());
            }
            if !new_username.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return Err("Username must be alphanumeric or underscore only".to_string());
            }
            let users = self.users.read().await;
            if users.values().any(|u| u.username == *new_username && u.id != user_id) {
                return Err("Username already exists".to_string());
            }
        }

        let mut users = self.users.write().await;
        let user = users.get_mut(user_id)
            .ok_or_else(|| "User not found".to_string())?;

        if let Some(ref new_username) = username {
            user.username = new_username.clone();
        }

        // email: None = not changing, Some(None) = clear email, Some(Some(v)) = set email
        if let Some(ref e) = email {
            user.email = e.clone();
        }

        if let Some(r) = role {
            user.role = r;
        }

        if let Some(active) = is_active {
            user.is_active = active;
        }

        user.updated_at = Utc::now();
        Ok(user.clone())
    }

    /// Change user password
    pub async fn change_password(
        &self,
        user_id: &str,
        current_password: Option<&str>,
        new_password: &str,
    ) -> Result<(), String> {
        // Validate new password strength
        if new_password.len() < 10 {
            return Err("Password must be at least 10 characters".to_string());
        }
        if !new_password.chars().any(|c| c.is_uppercase()) {
            return Err("Password must contain uppercase letter".to_string());
        }
        if !new_password.chars().any(|c| c.is_lowercase()) {
            return Err("Password must contain lowercase letter".to_string());
        }
        if !new_password.chars().any(|c| c.is_numeric()) {
            return Err("Password must contain digit".to_string());
        }

        let mut users = self.users.write().await;
        let user = users.get_mut(user_id)
            .ok_or_else(|| "User not found".to_string())?;

        // If current_password is provided, verify it (self-service change)
        if let Some(current) = current_password {
            if !Self::verify_password(current, &user.password_hash) {
                return Err("Current password is incorrect".to_string());
            }
        }

        user.password_hash = Self::hash_password(new_password);
        user.updated_at = Utc::now();

        // Revoke all sessions for this user to force re-login
        let user_id_owned = user.id.clone();
        drop(users); // release write lock before acquiring sessions lock
        {
            let mut sessions = self.sessions.write().await;
            sessions.retain(|_, s| s.user_id != user_id_owned);
        }

        Ok(())
    }

    /// Delete a user (cannot delete self)
    pub async fn delete_user(&self, user_id: &str, requester_id: &str) -> Result<(), String> {
        if user_id == requester_id {
            return Err("Cannot delete your own account".to_string());
        }

        let mut users = self.users.write().await;
        let removed = users.remove(user_id);
        if removed.is_none() {
            return Err("User not found".to_string());
        }

        // Revoke all sessions for deleted user
        drop(users);
        {
            let mut sessions = self.sessions.write().await;
            sessions.retain(|_, s| s.user_id != user_id);
        }

        Ok(())
    }

    /// Seed a default admin user if no users exist
    pub async fn seed_admin(&self) {
        let users = self.users.read().await;
        if !users.is_empty() {
            return;
        }
        drop(users);

        match self.create_user(
            "admin".to_string(),
            Some("admin@bambu-farm.local".to_string()),
            "Admin1234!".to_string(),
            Role::Admin,
        ).await {
            Ok(user) => {
                tracing::info!(" seeded default admin user: {} (password: Admin1234!)", user.username);
            }
            Err(e) => {
                tracing::warn!("failed to seed admin user: {e}");
            }
        }
    }
}

/// Generate simple UUID (deterministic, for testing)
fn uuid_simple() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::SystemTime;

    let mut hasher = DefaultHasher::new();
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .hash(&mut hasher);
    format!("{:x}", hasher.finish())
}
