use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use uuid::Uuid;

pub struct Client {
    pub addr: String,
    pub id: Uuid,
    // username: String, // To be added when authentication is implemented
    pub should_disconnect: Arc<AtomicBool>, // New field to signal disconnection
}

impl Client {
    pub fn new(addr: impl Into<String>) -> Client {
        Client {
            addr: addr.into(),
            id: Uuid::new_v4(),
            should_disconnect: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }

    // New method to check if client should disconnect
    pub fn should_disconnect(&self) -> bool {
        self.should_disconnect.load(Ordering::SeqCst)
    }

    // New method to signal client to disconnect
    pub fn signal_disconnect(&self) {
        self.should_disconnect.store(true, Ordering::SeqCst);
    }
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("id", &self.id)
            .field("addr", &self.addr)
            .field("should_disconnect", &self.should_disconnect())
            .finish()
    }
}

impl Clone for Client {
    fn clone(&self) -> Self {
        // Note: We can't clone the TcpStream, so the clone has no stream
        Client {
            addr: self.addr.clone(),
            id: self.id,
            should_disconnect: self.should_disconnect.clone(), // Clone the Arc
        }
    }
}
