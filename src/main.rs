use axum::{
    body::StreamBody,
    routing::get,
    Router,
    extract::Path,
    response::{IntoResponse, Response},
    http::{StatusCode, header},
};
use std::{net::SocketAddr, path::PathBuf, time::Duration};
use tokio::{fs, io::BufReader, signal};
use tokio_util::io::ReaderStream;
use tower_http::timeout::TimeoutLayer;

// Custom error type for our application
#[derive(Debug)]
enum AppError {
    NotFound(String),
    IoError(std::io::Error),
    InvalidPath(String),
}

// Implement error responses
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::NotFound(path) => 
                (StatusCode::NOT_FOUND, format!("File not found: {}", path)),
            AppError::IoError(err) => 
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Server error: {}", err)),
            AppError::InvalidPath(path) => 
                (StatusCode::BAD_REQUEST, format!("Invalid path: {}", path)),
        };

        (status, message).into_response()
    }
}

#[tokio::main]
async fn main() {
    // Create router with simpler middleware stack
    let app = Router::new()
        .route("/*path", get(handle_get))
        // Add just timeout middleware
        .layer(TimeoutLayer::new(Duration::from_secs(30)));

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("File server running on http://{}", addr);

    // Build server with graceful shutdown
    let server = axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(shutdown_signal());

    // Start server
    if let Err(err) = server.await {
        eprintln!("Server error: {}", err);
        std::process::exit(1);
    }
}

async fn handle_get(Path(path): Path<String>) -> Result<Response, AppError> {
    // Sanitize and validate path
    let path = PathBuf::from(path);
    
    // Prevent directory traversal attacks
    if path.components().any(|c| c.as_os_str() == "..") {
        return Err(AppError::InvalidPath("Path contains '..' which is not allowed".into()));
    }

    // Check if file exists and is actually a file
    let metadata = fs::metadata(&path).await
        .map_err(|_| AppError::NotFound(path.display().to_string()))?;

    if !metadata.is_file() {
        return Err(AppError::InvalidPath(format!("{} is not a file", path.display())));
    }

    // Open the file
    let file = fs::File::open(&path)
        .await
        .map_err(AppError::IoError)?;
    
    let metadata = file.metadata()
        .await
        .map_err(AppError::IoError)?;

    // Create a buffered reader with a reasonable buffer size (64KB)
    let stream = ReaderStream::new(BufReader::with_capacity(65536, file));
    let body = StreamBody::new(stream);

    // Try to guess the MIME type
    let mime_type = mime_guess::from_path(&path)
        .first_or_octet_stream()
        .to_string();

    // Build response with proper headers
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime_type)
        .header(header::CONTENT_LENGTH, metadata.len())
        .body(body)
        .unwrap()
        .into_response())
}

// Graceful shutdown handler
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    println!("shutdown signal received, starting graceful shutdown");
}
