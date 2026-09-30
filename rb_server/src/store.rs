//! SQLite-backed [`Store`] implementation.
//!
//! Implants live in memory (they are ephemeral and re-check in), while sessions, tasks and results
//! are persisted so server state survives a restart.

use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use rb::message::{CommandOutput, ImplantInfo};
use rb::session::{Session, SessionStatus};
use rb::store::{ListenerInfo, OperatorInfo, Store, StoreError};
use rb::task::{Task, TaskResult, TaskStatus};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS implants (
    id          TEXT PRIMARY KEY,
    hostname    TEXT NOT NULL,
    ip_address  TEXT NOT NULL,
    os_info     TEXT NOT NULL,
    username    TEXT NOT NULL,
    process_id  INTEGER NOT NULL,
    first_seen  INTEGER NOT NULL,
    last_seen   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    implant_id  TEXT NOT NULL,
    hostname    TEXT NOT NULL,
    ip_address  TEXT NOT NULL,
    status      TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    last_seen   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sessions_implant ON sessions(implant_id);

CREATE TABLE IF NOT EXISTS tasks (
    id          TEXT PRIMARY KEY,
    session_id  INTEGER NOT NULL,
    implant_id  TEXT NOT NULL,
    command     TEXT NOT NULL,
    args        TEXT NOT NULL,
    status      TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tasks_session ON tasks(session_id);

CREATE TABLE IF NOT EXISTS results (
    task_id      TEXT PRIMARY KEY,
    session_id   INTEGER NOT NULL,
    implant_id   TEXT NOT NULL,
    output       TEXT NOT NULL,
    error        TEXT,
    status_code  INTEGER,
    status       TEXT NOT NULL,
    completed_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS listeners (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    listener_type TEXT NOT NULL,
    bind_address  TEXT NOT NULL,
    port          INTEGER NOT NULL,
    created_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS operators (
    name       TEXT PRIMARY KEY,
    serial_hex TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    revoked    INTEGER NOT NULL
);
"#;

fn db_error<E: std::fmt::Display>(e: E) -> StoreError {
    StoreError::Database(e.to_string())
}

fn now_unix() -> i64 {
    system_time_to_unix(SystemTime::now())
}

fn system_time_to_unix(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn unix_to_system_time(secs: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs.max(0) as u64)
}

fn session_status_to_str(status: &SessionStatus) -> &'static str {
    match status {
        SessionStatus::Active => "active",
        SessionStatus::Idle => "idle",
        SessionStatus::Disconnected => "disconnected",
        SessionStatus::Terminated => "terminated",
    }
}

fn session_status_from_str(status: &str) -> SessionStatus {
    match status {
        "active" => SessionStatus::Active,
        "idle" => SessionStatus::Idle,
        "terminated" => SessionStatus::Terminated,
        _ => SessionStatus::Disconnected,
    }
}

fn task_status_to_str(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Pending => "pending",
        TaskStatus::InProgress => "in_progress",
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
        TaskStatus::Cancelled => "cancelled",
    }
}

fn task_status_from_str(status: &str) -> TaskStatus {
    match status {
        "pending" => TaskStatus::Pending,
        "in_progress" => TaskStatus::InProgress,
        "completed" => TaskStatus::Completed,
        "failed" => TaskStatus::Failed,
        "cancelled" => TaskStatus::Cancelled,
        _ => TaskStatus::Pending,
    }
}

fn parse_uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).unwrap_or_default()
}

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    /// Open (or create) a SQLite database at `path`.
    pub fn open(path: &str) -> Result<Self, StoreError> {
        let conn = Connection::open(path).map_err(db_error)?;
        Self::init(conn)
    }

    /// Open an in-memory database, mostly for tests.
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory().map_err(db_error)?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self, StoreError> {
        conn.execute_batch(SCHEMA).map_err(db_error)?;

        // Implants only reconnect when they poll, so restored sessions start disconnected until
        // their implant checks back in.
        conn.execute("UPDATE sessions SET status = 'disconnected'", [])
            .map_err(db_error)?;

        Ok(SqliteStore {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        match self.conn.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

fn row_to_implant(row: &Row<'_>) -> rusqlite::Result<ImplantInfo> {
    Ok(ImplantInfo {
        id: parse_uuid(&row.get::<_, String>(0)?),
        hostname: row.get(1)?,
        ip_address: row.get(2)?,
        os_info: row.get(3)?,
        username: row.get(4)?,
        process_id: row.get::<_, i64>(5)? as u32,
        first_seen: unix_to_system_time(row.get(6)?),
        last_seen: unix_to_system_time(row.get(7)?),
    })
}

fn row_to_session(row: &Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        id: row.get::<_, i64>(0)? as usize,
        implant_id: parse_uuid(&row.get::<_, String>(1)?),
        implant_hostname: row.get(2)?,
        ip_address: row.get(3)?,
        status: session_status_from_str(&row.get::<_, String>(4)?),
        created_at: unix_to_system_time(row.get(5)?),
        last_seen: unix_to_system_time(row.get(6)?),
    })
}

fn row_to_task(row: &Row<'_>) -> rusqlite::Result<Task> {
    let args: String = row.get(4)?;
    Ok(Task {
        id: parse_uuid(&row.get::<_, String>(0)?),
        session_id: row.get::<_, i64>(1)? as usize,
        implant_id: parse_uuid(&row.get::<_, String>(2)?),
        command: row.get(3)?,
        args: serde_json::from_str(&args).unwrap_or_default(),
        status: task_status_from_str(&row.get::<_, String>(5)?),
        created_at: unix_to_system_time(row.get(6)?),
    })
}

fn row_to_result(row: &Row<'_>) -> rusqlite::Result<TaskResult> {
    let output: String = row.get(3)?;
    Ok(TaskResult {
        task_id: parse_uuid(&row.get::<_, String>(0)?),
        session_id: row.get::<_, i64>(1)? as usize,
        implant_id: parse_uuid(&row.get::<_, String>(2)?),
        output: serde_json::from_str(&output).unwrap_or(CommandOutput::None),
        error: row.get(4)?,
        status_code: row.get(5)?,
        status: task_status_from_str(&row.get::<_, String>(6)?),
        completed_at: unix_to_system_time(row.get(7)?),
    })
}

fn row_to_listener(row: &Row<'_>) -> rusqlite::Result<ListenerInfo> {
    Ok(ListenerInfo {
        id: parse_uuid(&row.get::<_, String>(0)?),
        name: row.get(1)?,
        listener_type: row.get(2)?,
        bind_address: row.get(3)?,
        port: row.get::<_, i64>(4)? as u16,
        created_at: unix_to_system_time(row.get(5)?),
    })
}

fn row_to_operator(row: &Row<'_>) -> rusqlite::Result<OperatorInfo> {
    Ok(OperatorInfo {
        name: row.get(0)?,
        serial_hex: row.get(1)?,
        created_at: unix_to_system_time(row.get(2)?),
        revoked: row.get::<_, i64>(3)? != 0,
    })
}

impl Store for SqliteStore {
    fn upsert_implant(&self, implant: ImplantInfo) -> Result<(), StoreError> {
        self.conn()
            .execute(
                "INSERT OR REPLACE INTO implants
                    (id, hostname, ip_address, os_info, username, process_id, first_seen, last_seen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    implant.id.to_string(),
                    implant.hostname,
                    implant.ip_address,
                    implant.os_info,
                    implant.username,
                    implant.process_id as i64,
                    system_time_to_unix(implant.first_seen),
                    system_time_to_unix(implant.last_seen)
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }

    fn touch_implant(&self, id: &Uuid) -> Result<(), StoreError> {
        let updated = self
            .conn()
            .execute(
                "UPDATE implants SET last_seen = ?1 WHERE id = ?2",
                params![now_unix(), id.to_string()],
            )
            .map_err(db_error)?;
        if updated == 0 {
            return Err(StoreError::NotFound(format!("implant {}", id)));
        }
        Ok(())
    }

    fn get_implant(&self, id: &Uuid) -> Option<ImplantInfo> {
        self.conn()
            .query_row(
                "SELECT id, hostname, ip_address, os_info, username, process_id, first_seen, last_seen
                 FROM implants WHERE id = ?1",
                params![id.to_string()],
                row_to_implant,
            )
            .optional()
            .ok()
            .flatten()
    }

    fn list_implants(&self) -> Vec<ImplantInfo> {
        let conn = self.conn();
        let mut stmt = match conn.prepare(
            "SELECT id, hostname, ip_address, os_info, username, process_id, first_seen, last_seen
             FROM implants",
        ) {
            Ok(stmt) => stmt,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], row_to_implant);
        match rows {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn create_session(
        &self,
        implant_id: Uuid,
        hostname: String,
        ip_address: String,
    ) -> Result<usize, StoreError> {
        let conn = self.conn();
        let now = now_unix();
        conn.execute(
            "INSERT INTO sessions (implant_id, hostname, ip_address, status, created_at, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                implant_id.to_string(),
                hostname,
                ip_address,
                session_status_to_str(&SessionStatus::Active),
                now,
                now
            ],
        )
        .map_err(db_error)?;
        Ok(conn.last_insert_rowid() as usize)
    }

    fn get_session(&self, id: &usize) -> Option<Session> {
        self.conn()
            .query_row(
                "SELECT id, implant_id, hostname, ip_address, status, created_at, last_seen
                 FROM sessions WHERE id = ?1",
                params![*id as i64],
                row_to_session,
            )
            .optional()
            .ok()
            .flatten()
    }

    fn list_sessions(&self) -> Vec<Session> {
        let conn = self.conn();
        let mut stmt = match conn.prepare(
            "SELECT id, implant_id, hostname, ip_address, status, created_at, last_seen
             FROM sessions ORDER BY id",
        ) {
            Ok(stmt) => stmt,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], row_to_session);
        match rows {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn remove_session(&self, id: &usize) -> bool {
        let conn = self.conn();
        let removed = conn
            .execute("DELETE FROM sessions WHERE id = ?1", params![*id as i64])
            .map(|n| n > 0)
            .unwrap_or(false);
        if removed {
            let _ = conn.execute("DELETE FROM tasks WHERE session_id = ?1", params![*id as i64]);
            let _ = conn.execute(
                "DELETE FROM results WHERE session_id = ?1",
                params![*id as i64],
            );
        }
        removed
    }

    fn session_id_for_implant(&self, implant_id: &Uuid) -> Option<usize> {
        self.conn()
            .query_row(
                "SELECT id FROM sessions WHERE implant_id = ?1 ORDER BY id DESC LIMIT 1",
                params![implant_id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .ok()
            .flatten()
            .map(|id| id as usize)
    }

    fn activate_session(&self, id: &usize) -> Result<(), StoreError> {
        let updated = self
            .conn()
            .execute(
                "UPDATE sessions SET status = 'active' WHERE id = ?1",
                params![*id as i64],
            )
            .map_err(db_error)?;
        if updated == 0 {
            return Err(StoreError::NotFound(format!("session {}", id)));
        }
        Ok(())
    }

    fn kill_all_sessions(&self) {
        let _ = self
            .conn()
            .execute("UPDATE sessions SET status = 'terminated'", []);
    }

    fn create_task(
        &self,
        session_id: usize,
        command: String,
        args: Vec<String>,
    ) -> Result<Uuid, StoreError> {
        let conn = self.conn();
        let implant_id: String = conn
            .query_row(
                "SELECT implant_id FROM sessions WHERE id = ?1",
                params![session_id as i64],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?
            .ok_or_else(|| StoreError::NotFound(format!("session {}", session_id)))?;

        let task_id = Uuid::new_v4();
        let args = serde_json::to_string(&args).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "INSERT INTO tasks (id, session_id, implant_id, command, args, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                task_id.to_string(),
                session_id as i64,
                implant_id,
                command,
                args,
                task_status_to_str(&TaskStatus::Pending),
                now_unix()
            ],
        )
        .map_err(db_error)?;
        Ok(task_id)
    }

    fn get_task(&self, id: &Uuid) -> Option<Task> {
        self.conn()
            .query_row(
                "SELECT id, session_id, implant_id, command, args, status, created_at
                 FROM tasks WHERE id = ?1",
                params![id.to_string()],
                row_to_task,
            )
            .optional()
            .ok()
            .flatten()
    }

    fn list_pending_tasks(&self, session_id: usize) -> Vec<Task> {
        let conn = self.conn();
        let mut stmt = match conn.prepare(
            "SELECT id, session_id, implant_id, command, args, status, created_at
             FROM tasks WHERE session_id = ?1 AND status = 'pending'",
        ) {
            Ok(stmt) => stmt,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map(params![session_id as i64], row_to_task);
        match rows {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn set_task_status(&self, id: &Uuid, status: TaskStatus) -> Result<(), StoreError> {
        let updated = self
            .conn()
            .execute(
                "UPDATE tasks SET status = ?1 WHERE id = ?2",
                params![task_status_to_str(&status), id.to_string()],
            )
            .map_err(db_error)?;
        if updated == 0 {
            return Err(StoreError::NotFound(format!("task {}", id)));
        }
        Ok(())
    }

    fn submit_result(&self, result: TaskResult) -> Result<(), StoreError> {
        let conn = self.conn();
        let status = task_status_to_str(&result.status);
        conn.execute(
            "UPDATE tasks SET status = ?1 WHERE id = ?2",
            params![status, result.task_id.to_string()],
        )
        .map_err(db_error)?;
        conn.execute(
            "INSERT OR REPLACE INTO results
                (task_id, session_id, implant_id, output, error, status_code, status, completed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                result.task_id.to_string(),
                result.session_id as i64,
                result.implant_id.to_string(),
                serde_json::to_string(&result.output).unwrap_or_default(),
                result.error,
                result.status_code,
                status,
                system_time_to_unix(result.completed_at)
            ],
        )
        .map_err(db_error)?;
        Ok(())
    }

    fn get_result(&self, task_id: &Uuid) -> Option<TaskResult> {
        self.conn()
            .query_row(
                "SELECT task_id, session_id, implant_id, output, error, status_code, status, completed_at
                 FROM results WHERE task_id = ?1",
                params![task_id.to_string()],
                row_to_result,
            )
            .optional()
            .ok()
            .flatten()
    }

    fn upsert_listener(&self, listener: ListenerInfo) -> Result<(), StoreError> {
        self.conn()
            .execute(
                "INSERT OR REPLACE INTO listeners
                    (id, name, listener_type, bind_address, port, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    listener.id.to_string(),
                    listener.name,
                    listener.listener_type,
                    listener.bind_address,
                    listener.port as i64,
                    system_time_to_unix(listener.created_at)
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }

    fn get_listener(&self, id: &Uuid) -> Option<ListenerInfo> {
        self.conn()
            .query_row(
                "SELECT id, name, listener_type, bind_address, port, created_at
                 FROM listeners WHERE id = ?1",
                params![id.to_string()],
                row_to_listener,
            )
            .optional()
            .ok()
            .flatten()
    }

    fn list_listeners(&self) -> Vec<ListenerInfo> {
        let conn = self.conn();
        let mut stmt = match conn.prepare(
            "SELECT id, name, listener_type, bind_address, port, created_at
             FROM listeners ORDER BY created_at",
        ) {
            Ok(stmt) => stmt,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], row_to_listener);
        match rows {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn remove_listener(&self, id: &Uuid) -> bool {
        self.conn()
            .execute("DELETE FROM listeners WHERE id = ?1", params![id.to_string()])
            .map(|n| n > 0)
            .unwrap_or(false)
    }

    fn upsert_operator(&self, operator: OperatorInfo) -> Result<(), StoreError> {
        self.conn()
            .execute(
                "INSERT OR REPLACE INTO operators (name, serial_hex, created_at, revoked)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    operator.name,
                    operator.serial_hex,
                    system_time_to_unix(operator.created_at),
                    operator.revoked as i64
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }

    fn get_operator(&self, name: &str) -> Option<OperatorInfo> {
        self.conn()
            .query_row(
                "SELECT name, serial_hex, created_at, revoked FROM operators WHERE name = ?1",
                params![name],
                row_to_operator,
            )
            .optional()
            .ok()
            .flatten()
    }

    fn list_operators(&self) -> Vec<OperatorInfo> {
        let conn = self.conn();
        let mut stmt = match conn
            .prepare("SELECT name, serial_hex, created_at, revoked FROM operators ORDER BY name")
        {
            Ok(stmt) => stmt,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], row_to_operator);
        match rows {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn set_operator_revoked(&self, name: &str, revoked: bool) -> bool {
        self.conn()
            .execute(
                "UPDATE operators SET revoked = ?1 WHERE name = ?2",
                params![revoked as i64, name],
            )
            .map(|n| n > 0)
            .unwrap_or(false)
    }

    fn revoked_operator_serials(&self) -> Vec<String> {
        let conn = self.conn();
        let mut stmt = match conn.prepare("SELECT serial_hex FROM operators WHERE revoked = 1") {
            Ok(stmt) => stmt,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], |row| row.get::<_, String>(0));
        match rows {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rb::task::TaskStatus;

    fn implant() -> ImplantInfo {
        let now = SystemTime::now();
        ImplantInfo {
            id: Uuid::new_v4(),
            hostname: "host".to_string(),
            ip_address: "127.0.0.1".to_string(),
            os_info: "test".to_string(),
            username: "user".to_string(),
            process_id: 1,
            first_seen: now,
            last_seen: now,
        }
    }

    #[test]
    fn task_result_round_trip() {
        let store = SqliteStore::open_in_memory().unwrap();
        let imp = implant();
        store.upsert_implant(imp.clone()).unwrap();
        let session_id = store
            .create_session(imp.id, "host".to_string(), "127.0.0.1".to_string())
            .unwrap();

        let task_id = store
            .create_task(session_id, "whoami".to_string(), vec!["a".to_string()])
            .unwrap();
        assert_eq!(store.list_pending_tasks(session_id).len(), 1);

        store
            .submit_result(TaskResult {
                task_id,
                implant_id: imp.id,
                session_id,
                output: CommandOutput::Text("user".to_string()),
                error: None,
                status_code: Some(0),
                status: TaskStatus::Completed,
                completed_at: SystemTime::now(),
            })
            .unwrap();

        assert_eq!(store.get_task(&task_id).unwrap().status, TaskStatus::Completed);
        assert!(store.get_result(&task_id).is_some());
        assert!(store.remove_session(&session_id));
        assert!(store.get_session(&session_id).is_none());
    }

    #[test]
    fn sessions_survive_reopen() {
        let path = std::env::temp_dir().join(format!("rb_store_test_{}.sqlite", Uuid::new_v4()));
        let path_str = path.to_str().unwrap().to_string();
        let imp = implant();

        {
            let store = SqliteStore::open(&path_str).unwrap();
            store.upsert_implant(imp.clone()).unwrap();
            store
                .create_session(imp.id, "host".to_string(), "127.0.0.1".to_string())
                .unwrap();
        }

        {
            let store = SqliteStore::open(&path_str).unwrap();
            // Implants are persisted so a restarted server still recognises their polls.
            assert!(store.get_implant(&imp.id).is_some());

            let sessions = store.list_sessions();
            assert_eq!(sessions.len(), 1);
            // Restored sessions start disconnected until the implant checks in again.
            assert_eq!(sessions[0].status(), "Disconnected");
            assert_eq!(store.session_id_for_implant(&imp.id), Some(sessions[0].id));
        }

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn listeners_survive_reopen() {
        let path = std::env::temp_dir().join(format!("rb_listener_test_{}.sqlite", Uuid::new_v4()));
        let path_str = path.to_str().unwrap().to_string();
        let id = Uuid::new_v4();

        {
            let store = SqliteStore::open(&path_str).unwrap();
            store
                .upsert_listener(ListenerInfo {
                    id,
                    name: "HTTP_0.0.0.0:8080".to_string(),
                    listener_type: "http".to_string(),
                    bind_address: "0.0.0.0".to_string(),
                    port: 8080,
                    created_at: SystemTime::now(),
                })
                .unwrap();
        }

        {
            let store = SqliteStore::open(&path_str).unwrap();
            let listener = store.get_listener(&id).expect("listener should persist");
            assert_eq!(listener.port, 8080);
            assert_eq!(store.list_listeners().len(), 1);
        }

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn operators_survive_reopen() {
        let path = std::env::temp_dir().join(format!("rb_op_test_{}.sqlite", Uuid::new_v4()));
        let path_str = path.to_str().unwrap().to_string();

        {
            let store = SqliteStore::open(&path_str).unwrap();
            store
                .upsert_operator(OperatorInfo {
                    name: "alice".to_string(),
                    serial_hex: "abcd".to_string(),
                    created_at: SystemTime::now(),
                    revoked: false,
                })
                .unwrap();
            assert!(store.set_operator_revoked("alice", true));
        }

        {
            let store = SqliteStore::open(&path_str).unwrap();
            assert!(store.get_operator("alice").unwrap().revoked);
            assert_eq!(store.revoked_operator_serials(), vec!["abcd".to_string()]);
        }

        let _ = std::fs::remove_file(&path);
    }
}
