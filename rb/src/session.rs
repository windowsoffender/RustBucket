use std::time::SystemTime;
use uuid::Uuid;

/// Status of a session
#[derive(Debug, Clone, PartialEq)]
pub enum SessionStatus {
    Active,
    Idle,
    Disconnected,
    Terminated,
}

impl ToString for SessionStatus {
    fn to_string(&self) -> String {
        match self {
            SessionStatus::Active => "Active".to_string(),
            SessionStatus::Idle => "Idle".to_string(),
            SessionStatus::Disconnected => "Disconnected".to_string(),
            SessionStatus::Terminated => "Terminated".to_string(),
        }
    }
}

/// A session is one implant's connection to the C2. Tasks and results live in the [`Store`], not
/// here.
///
/// [`Store`]: crate::store::Store
#[derive(Debug, Clone)]
pub struct Session {
    /// Unique ID for this session
    pub id: usize,

    /// Implant ID
    pub implant_id: Uuid,

    /// Implant hostname
    pub implant_hostname: String,

    /// Address the implant connected from
    pub ip_address: String,

    /// When the session was created
    pub created_at: SystemTime,

    /// Last time communication was received from this session
    pub last_seen: SystemTime,

    /// Current status of the session
    pub status: SessionStatus,
}

impl Session {
    /// Create a new session
    pub fn new(id: usize, implant_id: Uuid, implant_hostname: String, ip_address: String) -> Self {
        let now = SystemTime::now();
        Session {
            id,
            implant_id,
            implant_hostname,
            ip_address,
            created_at: now,
            last_seen: now,
            status: SessionStatus::Active,
        }
    }

    /// Get session ID
    pub fn id(&self) -> usize {
        self.id
    }

    /// Get implant hostname
    pub fn implant_hostname(&self) -> &str {
        &self.implant_hostname
    }

    /// Get address
    pub fn address(&self) -> &str {
        &self.ip_address
    }

    /// Get last seen timestamp as a relative string
    pub fn last_seen(&self) -> String {
        match self.last_seen.elapsed() {
            Ok(elapsed) => {
                if elapsed.as_secs() < 60 {
                    "Just now".to_string()
                } else if elapsed.as_secs() < 3600 {
                    format!("{} minutes ago", elapsed.as_secs() / 60)
                } else if elapsed.as_secs() < 86400 {
                    format!("{} hours ago", elapsed.as_secs() / 3600)
                } else {
                    format!("{} days ago", elapsed.as_secs() / 86400)
                }
            }
            Err(_) => "Time error".to_string(),
        }
    }

    /// Get session status
    pub fn status(&self) -> String {
        self.status.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_creation() {
        let addr = "192.168.0.1";
        let implant_id = Uuid::new_v4();
        let session = Session::new(0, implant_id, "test-implant".to_string(), addr.to_string());

        assert_eq!(session.id(), 0);
        assert_eq!(session.implant_id, implant_id);
        assert_eq!(session.implant_hostname(), "test-implant");
        assert_eq!(session.address(), addr);
        assert_eq!(session.status(), "Active");
    }
}
