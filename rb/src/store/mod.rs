//! Single source of truth for implants, sessions, tasks and results.
//!
//! The server, the HTTP listeners and the command layer all go through this interface. The in-memory
//! implementation lives in [`memory::MemoryStore`]; a persistent implementation can live outside
//! this crate so the implant's dependency graph stays free of a database.

use crate::message::ImplantInfo;
use crate::session::Session;
use crate::task::{Task, TaskResult, TaskStatus};
use std::time::SystemTime;
use uuid::Uuid;

pub mod memory;

/// Errors returned by the store.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("database error: {0}")]
    Database(String),

    #[error("internal error: {0}")]
    Internal(String),
}

/// A listener configuration record.
///
/// The live actix handle lives in the server; this is the persisted definition used to bring a
/// listener back up after a restart.
#[derive(Debug, Clone)]
pub struct ListenerInfo {
    pub id: Uuid,
    pub name: String,
    /// Listener type, currently only `"http"`.
    pub listener_type: String,
    pub bind_address: String,
    pub port: u16,
    pub created_at: SystemTime,
}

/// An operator profile issued by the server's CA.
#[derive(Debug, Clone)]
pub struct OperatorInfo {
    pub name: String,
    /// Certificate serial number as lowercase hex, used for revocation.
    pub serial_hex: String,
    pub created_at: SystemTime,
    pub revoked: bool,
}

/// Persistent and in-memory state for the C2.
///
/// All methods are synchronous: SQLite is fast enough locally that the async callers don't need an
/// async interface, and the in-memory store is a mutex around a few maps.
pub trait Store: Send + Sync {
    // Implants

    /// Insert or replace an implant record.
    fn upsert_implant(&self, implant: ImplantInfo) -> Result<(), StoreError>;

    /// Update an implant's `last_seen` timestamp.
    fn touch_implant(&self, id: &Uuid) -> Result<(), StoreError>;

    /// Look up an implant by ID.
    fn get_implant(&self, id: &Uuid) -> Option<ImplantInfo>;

    /// List all known implants.
    fn list_implants(&self) -> Vec<ImplantInfo>;

    // Sessions

    /// Create a session for an implant and return its ID.
    fn create_session(
        &self,
        implant_id: Uuid,
        hostname: String,
        ip_address: String,
    ) -> Result<usize, StoreError>;

    /// Look up a session by ID.
    fn get_session(&self, id: &usize) -> Option<Session>;

    /// List all sessions.
    fn list_sessions(&self) -> Vec<Session>;

    /// Remove a session and all of its tasks and results. Returns whether it existed.
    fn remove_session(&self, id: &usize) -> bool;

    /// Find the session ID for an implant.
    fn session_id_for_implant(&self, implant_id: &Uuid) -> Option<usize>;

    /// Mark a session as active.
    fn activate_session(&self, id: &usize) -> Result<(), StoreError>;

    /// Terminate every session.
    fn kill_all_sessions(&self);

    // Tasks

    /// Create a pending task for a session and return its ID.
    fn create_task(
        &self,
        session_id: usize,
        command: String,
        args: Vec<String>,
        data: Option<Vec<u8>>,
    ) -> Result<Uuid, StoreError>;

    /// Look up a task by ID.
    fn get_task(&self, id: &Uuid) -> Option<Task>;

    /// List pending tasks for a session.
    fn list_pending_tasks(&self, session_id: usize) -> Vec<Task>;

    /// Update a task's status.
    fn set_task_status(&self, id: &Uuid, status: TaskStatus) -> Result<(), StoreError>;

    // Results

    /// Store a task result and mark the task with the result's status.
    fn submit_result(&self, result: TaskResult) -> Result<(), StoreError>;

    /// Look up a task result by task ID.
    fn get_result(&self, task_id: &Uuid) -> Option<TaskResult>;

    // Listeners

    /// Insert or replace a listener record.
    fn upsert_listener(&self, listener: ListenerInfo) -> Result<(), StoreError>;

    /// Look up a listener by ID.
    fn get_listener(&self, id: &Uuid) -> Option<ListenerInfo>;

    /// List all listener records.
    fn list_listeners(&self) -> Vec<ListenerInfo>;

    /// Remove a listener record. Returns whether it existed.
    fn remove_listener(&self, id: &Uuid) -> bool;

    // Operators

    /// Insert or replace an operator profile record.
    fn upsert_operator(&self, operator: OperatorInfo) -> Result<(), StoreError>;

    /// Look up an operator by name.
    fn get_operator(&self, name: &str) -> Option<OperatorInfo>;

    /// List all operator profiles.
    fn list_operators(&self) -> Vec<OperatorInfo>;

    /// Mark an operator as revoked (or not). Returns whether it existed.
    fn set_operator_revoked(&self, name: &str, revoked: bool) -> bool;

    /// Serial numbers (lowercase hex) of every revoked operator, for the CRL.
    fn revoked_operator_serials(&self) -> Vec<String>;
}
