use axum::{Router, extract::Request, middleware::Next, response::Response, routing::get};
use stathost::BucketManager;
use stathost::config::AppConfig;
use std::{path::PathBuf, sync::Arc, time::Instant};

async fn access_log(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let start = Instant::now();

    let response = next.run(request).await;

    let elapsed = start.elapsed();
    println!(
        "{} {} {} {}ms",
        method,
        uri,
        response.status().as_u16(),
        elapsed.as_millis()
    );

    response
}

#[tokio::main]
async fn main() {
    let config_path = std::env::args()
        .skip_while(|a| a != "--config")
        .nth(1)
        .map(PathBuf::from);

    let config = AppConfig::load(config_path.as_deref()).unwrap_or_else(|e| {
        eprintln!("Failed to load config: {}", e);
        std::process::exit(1);
    });

    let buckets_dir = PathBuf::from(&config.server.buckets_dir);
    if !buckets_dir.exists() {
        std::fs::create_dir_all(&buckets_dir).unwrap_or_else(|e| {
            eprintln!("Failed to create buckets directory: {}", e);
            std::process::exit(1);
        });
    }

    let manager = Arc::new(BucketManager::new(buckets_dir));

    let app = Router::new()
        .route("/", get(stathost::serve_root_index))
        .route("/openapi.json", get(stathost::openapi))
        .route("/{bucket}", get(stathost::serve_bucket_root))
        .route("/{bucket}/", get(stathost::serve_bucket_root))
        .route("/{bucket}/_meta/list", get(stathost::list_files))
        .route(
            "/{bucket}/{*path}",
            get(stathost::serve_file)
                .put(stathost::upload_file)
                .delete(stathost::delete_file),
        )
        .layer(axum::middleware::from_fn(access_log))
        .with_state(manager);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    println!("StatHost listening on {}", addr);

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Failed to bind to {}: {}", addr, e);
            std::process::exit(1);
        }
    };

    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("Server error: {}", e);
        std::process::exit(1);
    }
}
