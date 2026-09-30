use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;

use uuid::Uuid;

use super::{ListenerInfo, OperatorInfo, Store, StoreError};
use crate::message::ImplantInfo;
use crate::session::{Session, SessionStatus};
use crate::task::{Task, TaskResult, TaskStatus};

#[derive(Default)]
struct Inner {
    implants: HashMap<Uuid, ImplantInfo>,
    sessions: HashMap<usize, Session>,
    tasks: HashMap<Uuid, Task>,
    results: HashMap<Uuid, TaskResult>,
    listeners: HashMap<Uuid, ListenerInfo>,
    operators: HashMap<String, OperatorInfo>,
    implant_to_session: HashMap<Uuid, usize>,
    next_id: usize,
}

/// In-memory [`Store`] implementation.
pub struct MemoryStore {
    inner: Mutex<Inner>,
}

impl MemoryStore {
    pub fn new() -> Self {
        MemoryStore {
            inner: Mutex::new(Inner::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Store for MemoryStore {
    fn upsert_implant(&self, implant: ImplantInfo) -> Result<(), StoreError> {
        self.lock().implants.insert(implant.id, implant);
        Ok(())
    }

    fn touch_implant(&self, id: &Uuid) -> Result<(), StoreError> {
        let mut inner = self.lock();
        match inner.implants.get_mut(id) {
            Some(implant) => {
                implant.last_seen = SystemTime::now();
                Ok(())
            }
            None => Err(StoreError::NotFound(format!("implant {}", id))),
        }
    }

    fn get_implant(&self, id: &Uuid) -> Option<ImplantInfo> {
        self.lock().implants.get(id).cloned()
    }

    fn list_implants(&self) -> Vec<ImplantInfo> {
        self.lock().implants.values().cloned().collect()
    }

    fn create_session(
        &self,
        implant_id: Uuid,
        hostname: String,
        ip_address: String,
    ) -> Result<usize, StoreError> {
        let mut inner = self.lock();
        let id = inner.next_id;
        inner.next_id += 1;

        let session = Session::new(id, implant_id, hostname, ip_address);
        inner.sessions.insert(id, session);
        inner.implant_to_session.insert(implant_id, id);
        Ok(id)
    }

    fn get_session(&self, id: &usize) -> Option<Session> {
        self.lock().sessions.get(id).cloned()
    }

    fn list_sessions(&self) -> Vec<Session> {
        let mut sessions: Vec<Session> = self.lock().sessions.values().cloned().collect();
        sessions.sort_by_key(|s| s.id);
        sessions
    }

    fn remove_session(&self, id: &usize) -> bool {
        let mut inner = self.lock();
        let removed = inner.sessions.remove(id).is_some();
        if removed {
            inner.implant_to_session.retain(|_, session_id| session_id != id);

            let task_ids: Vec<Uuid> = inner
                .tasks
                .values()
                .filter(|task| task.session_id == *id)
                .map(|task| task.id)
                .collect();
            for task_id in task_ids {
                inner.tasks.remove(&task_id);
                inner.results.remove(&task_id);
            }
        }
        removed
    }

    fn session_id_for_implant(&self, implant_id: &Uuid) -> Option<usize> {
        self.lock().implant_to_session.get(implant_id).copied()
    }

    fn activate_session(&self, id: &usize) -> Result<(), StoreError> {
        match self.lock().sessions.get_mut(id) {
            Some(session) => {
                session.status = SessionStatus::Active;
                Ok(())
            }
            None => Err(StoreError::NotFound(format!("session {}", id))),
        }
    }

    fn kill_all_sessions(&self) {
        for session in self.lock().sessions.values_mut() {
            session.status = SessionStatus::Terminated;
        }
    }

    fn create_task(
        &self,
        session_id: usize,
        command: String,
        args: Vec<String>,
        data: Option<Vec<u8>>,
    ) -> Result<Uuid, StoreError> {
        let mut inner = self.lock();
        let implant_id = match inner.sessions.get(&session_id) {
            Some(session) => session.implant_id,
            None => {
                return Err(StoreError::NotFound(format!("session {}", session_id)));
            }
        };

        let task = Task {
            id: Uuid::new_v4(),
            implant_id,
            session_id,
            command,
            args,
            data,
            created_at: SystemTime::now(),
            status: TaskStatus::Pending,
        };
        let id = task.id;
        inner.tasks.insert(id, task);
        Ok(id)
    }

    fn get_task(&self, id: &Uuid) -> Option<Task> {
        self.lock().tasks.get(id).cloned()
    }

    fn list_pending_tasks(&self, session_id: usize) -> Vec<Task> {
        self.lock()
            .tasks
            .values()
            .filter(|task| task.session_id == session_id && task.status == TaskStatus::Pending)
            .cloned()
            .collect()
    }

    fn set_task_status(&self, id: &Uuid, status: TaskStatus) -> Result<(), StoreError> {
        match self.lock().tasks.get_mut(id) {
            Some(task) => {
                task.status = status;
                Ok(())
            }
            None => Err(StoreError::NotFound(format!("task {}", id))),
        }
    }

    fn submit_result(&self, result: TaskResult) -> Result<(), StoreError> {
        let mut inner = self.lock();
        if let Some(task) = inner.tasks.get_mut(&result.task_id) {
            task.status = result.status.clone();
        }
        inner.results.insert(result.task_id, result);
        Ok(())
    }

    fn get_result(&self, task_id: &Uuid) -> Option<TaskResult> {
        self.lock().results.get(task_id).cloned()
    }

    fn upsert_listener(&self, listener: ListenerInfo) -> Result<(), StoreError> {
        self.lock().listeners.insert(listener.id, listener);
        Ok(())
    }

    fn get_listener(&self, id: &Uuid) -> Option<ListenerInfo> {
        self.lock().listeners.get(id).cloned()
    }

    fn list_listeners(&self) -> Vec<ListenerInfo> {
        let mut listeners: Vec<ListenerInfo> = self.lock().listeners.values().cloned().collect();
        listeners.sort_by_key(|l| l.created_at);
        listeners
    }

    fn remove_listener(&self, id: &Uuid) -> bool {
        self.lock().listeners.remove(id).is_some()
    }

    fn upsert_operator(&self, operator: OperatorInfo) -> Result<(), StoreError> {
        self.lock().operators.insert(operator.name.clone(), operator);
        Ok(())
    }

    fn get_operator(&self, name: &str) -> Option<OperatorInfo> {
        self.lock().operators.get(name).cloned()
    }

    fn list_operators(&self) -> Vec<OperatorInfo> {
        let mut operators: Vec<OperatorInfo> = self.lock().operators.values().cloned().collect();
        operators.sort_by(|a, b| a.name.cmp(&b.name));
        operators
    }

    fn set_operator_revoked(&self, name: &str, revoked: bool) -> bool {
        match self.lock().operators.get_mut(name) {
            Some(operator) => {
                operator.revoked = revoked;
                true
            }
            None => false,
        }
    }

    fn revoked_operator_serials(&self) -> Vec<String> {
        self.lock()
            .operators
            .values()
            .filter(|operator| operator.revoked)
            .map(|operator| operator.serial_hex.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::CommandOutput;

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
    fn session_lifecycle() {
        let store = MemoryStore::new();
        let imp = implant();
        store.upsert_implant(imp.clone()).unwrap();

        let session_id = store
            .create_session(imp.id, "host".to_string(), "127.0.0.1".to_string())
            .unwrap();

        assert_eq!(store.session_id_for_implant(&imp.id), Some(session_id));
        assert_eq!(store.get_session(&session_id).unwrap().implant_hostname(), "host");

        store.activate_session(&session_id).unwrap();
        assert!(store.remove_session(&session_id));
        assert!(store.get_session(&session_id).is_none());
    }

    #[test]
    fn task_and_result_round_trip() {
        let store = MemoryStore::new();
        let imp = implant();
        store.upsert_implant(imp.clone()).unwrap();
        let session_id = store
            .create_session(imp.id, "host".to_string(), "127.0.0.1".to_string())
            .unwrap();

        let task_id = store
            .create_task(session_id, "whoami".to_string(), vec![], None)
            .unwrap();
        assert_eq!(store.list_pending_tasks(session_id).len(), 1);

        store
            .set_task_status(&task_id, TaskStatus::InProgress)
            .unwrap();
        assert!(store.list_pending_tasks(session_id).is_empty());

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
    }

    #[test]
    fn listener_registry_round_trip() {
        let store = MemoryStore::new();
        let info = ListenerInfo {
            id: Uuid::new_v4(),
            name: "HTTP_0.0.0.0:8080".to_string(),
            listener_type: "http".to_string(),
            bind_address: "0.0.0.0".to_string(),
            port: 8080,
            created_at: SystemTime::now(),
        };

        store.upsert_listener(info.clone()).unwrap();
        assert_eq!(store.list_listeners().len(), 1);
        assert_eq!(store.get_listener(&info.id).unwrap().port, 8080);
        assert!(store.remove_listener(&info.id));
        assert!(store.list_listeners().is_empty());
    }

    #[test]
    fn operator_revocation() {
        let store = MemoryStore::new();
        store
            .upsert_operator(OperatorInfo {
                name: "alice".to_string(),
                serial_hex: "0a0b".to_string(),
                created_at: SystemTime::now(),
                revoked: false,
            })
            .unwrap();

        assert_eq!(store.list_operators().len(), 1);
        assert!(store.revoked_operator_serials().is_empty());

        assert!(store.set_operator_revoked("alice", true));
        assert_eq!(store.revoked_operator_serials(), vec!["0a0b".to_string()]);
        assert!(store.get_operator("alice").unwrap().revoked);
    }
}
