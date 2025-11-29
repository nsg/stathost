use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use tokio::fs;
use tokio::time::sleep;

struct TestServer {
    addr: SocketAddr,
    buckets_dir: PathBuf,
    shutdown: tokio::sync::oneshot::Sender<()>,
}

impl TestServer {
    async fn start() -> Self {
        let buckets_dir = PathBuf::from(format!("/tmp/stathost-test-{}", std::process::id()));
        fs::create_dir_all(&buckets_dir).await.unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

        let buckets_dir_clone = buckets_dir.clone();
        tokio::spawn(async move {
            run_server(listener, buckets_dir_clone, shutdown_rx).await;
        });

        sleep(Duration::from_millis(50)).await;

        Self {
            addr,
            buckets_dir,
            shutdown: shutdown_tx,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    async fn create_bucket(&self, name: &str, token: &str) {
        let bucket_path = self.buckets_dir.join(name);
        fs::create_dir_all(&bucket_path).await.unwrap();
        let config = format!("[auth]\ntoken = \"{}\"", token);
        fs::write(bucket_path.join("config.toml"), config)
            .await
            .unwrap();
    }

    async fn cleanup(self) {
        let _ = self.shutdown.send(());
        let _ = fs::remove_dir_all(&self.buckets_dir).await;
    }
}

async fn run_server(
    listener: tokio::net::TcpListener,
    buckets_dir: PathBuf,
    shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    use axum::{Router, routing::get};
    use std::sync::Arc;

    // Import from the main crate
    let manager = Arc::new(stathost::BucketManager::new(buckets_dir));

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
        .with_state(manager);

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            shutdown.await.ok();
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn test_full_workflow() {
    let server = TestServer::start().await;
    let client = reqwest::Client::new();

    // Create test buckets
    server.create_bucket("site1", "token1").await;
    server.create_bucket("site2", "token2").await;

    // Test: OpenAPI endpoint
    let resp = client
        .get(server.url("/openapi.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["openapi"], "3.0.3");

    // Test: Upload files
    let resp = client
        .put(server.url("/site1/index.html"))
        .header("Authorization", "Bearer token1")
        .body("<h1>Hello Site 1</h1>")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    let resp = client
        .put(server.url("/site1/assets/style.css"))
        .header("Authorization", "Bearer token1")
        .body("body { color: red; }")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    let resp = client
        .put(server.url("/site2/page.html"))
        .header("Authorization", "Bearer token2")
        .body("<p>Site 2</p>")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    // Test: Serve files
    let resp = client.get(server.url("/site1/")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "<h1>Hello Site 1</h1>");

    let resp = client
        .get(server.url("/site1/assets/style.css"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "body { color: red; }");

    // Test: List files (authenticated)
    let resp = client
        .get(server.url("/site1/_meta/list"))
        .header("Authorization", "Bearer token1")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let files: Vec<String> = resp.json().await.unwrap();
    assert!(files.contains(&"index.html".to_string()));
    assert!(files.contains(&"assets/style.css".to_string()));

    // Test: List files without auth fails
    let resp = client
        .get(server.url("/site1/_meta/list"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Test: List files with wrong token fails
    let resp = client
        .get(server.url("/site1/_meta/list"))
        .header("Authorization", "Bearer wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);

    // Test: Upload without auth fails
    let resp = client
        .put(server.url("/site1/new.txt"))
        .body("content")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Test: Upload with wrong token fails
    let resp = client
        .put(server.url("/site1/new.txt"))
        .header("Authorization", "Bearer token2")
        .body("content")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);

    // Test: Update existing file
    let resp = client
        .put(server.url("/site1/index.html"))
        .header("Authorization", "Bearer token1")
        .body("<h1>Updated</h1>")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    let resp = client
        .get(server.url("/site1/index.html"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.text().await.unwrap(), "<h1>Updated</h1>");

    // Test: Delete file
    let resp = client
        .delete(server.url("/site1/assets/style.css"))
        .header("Authorization", "Bearer token1")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    // Verify deleted
    let resp = client
        .get(server.url("/site1/assets/style.css"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // Test: Delete without auth fails
    let resp = client
        .delete(server.url("/site1/index.html"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Test: Cannot access config.toml
    let resp = client
        .get(server.url("/site1/config.toml"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // Test: Cannot upload to config.toml
    let resp = client
        .put(server.url("/site1/config.toml"))
        .header("Authorization", "Bearer token1")
        .body("hacked")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    // Test: Non-existent bucket
    let resp = client
        .get(server.url("/nonexistent/file.txt"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // Test: Root without index bucket returns 404
    let resp = client.get(server.url("/")).send().await.unwrap();
    assert_eq!(resp.status(), 404);

    // Test: Root with index bucket serves index.html
    server.create_bucket("index", "indextoken").await;
    let resp = client
        .put(server.url("/index/index.html"))
        .header("Authorization", "Bearer indextoken")
        .body("<h1>Welcome</h1>")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    let resp = client.get(server.url("/")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "<h1>Welcome</h1>");

    server.cleanup().await;
}
