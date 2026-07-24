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
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let buckets_dir =
            PathBuf::from(format!("/tmp/stathost-test-{}-{}", std::process::id(), id));
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
    assert_eq!(json["info"]["version"], env!("CARGO_PKG_VERSION"));

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

#[tokio::test]
async fn test_detailed_list() {
    let server = TestServer::start().await;
    let client = reqwest::Client::new();

    server.create_bucket("cam", "tok").await;

    let body = "0123456789abcdef";
    let resp = client
        .put(server.url("/cam/front/event.ts"))
        .header("Authorization", "Bearer tok")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    // Plain list unchanged
    let resp = client
        .get(server.url("/cam/_meta/list"))
        .header("Authorization", "Bearer tok")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let files: Vec<String> = resp.json().await.unwrap();
    assert_eq!(files, vec!["front/event.ts".to_string()]);

    // Detailed list
    let resp = client
        .get(server.url("/cam/_meta/list?detail=true"))
        .header("Authorization", "Bearer tok")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let entries: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["path"], "front/event.ts");
    assert_eq!(entries[0]["size"], body.len() as u64);
    let mtime = entries[0]["mtime"].as_u64().unwrap();
    assert!(mtime > 1_700_000_000);

    // detail=false behaves like the plain form
    let resp = client
        .get(server.url("/cam/_meta/list?detail=false"))
        .header("Authorization", "Bearer tok")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let files: Vec<String> = resp.json().await.unwrap();
    assert_eq!(files, vec!["front/event.ts".to_string()]);

    // Detailed list requires auth
    let resp = client
        .get(server.url("/cam/_meta/list?detail=true"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // Temp files are hidden from both list forms and from GET
    let tmp_name = "front/event.ts.deadbeef-1.stathost-tmp";
    fs::write(server.buckets_dir.join("cam").join(tmp_name), "partial")
        .await
        .unwrap();

    let resp = client
        .get(server.url("/cam/_meta/list"))
        .header("Authorization", "Bearer tok")
        .send()
        .await
        .unwrap();
    let files: Vec<String> = resp.json().await.unwrap();
    assert_eq!(files, vec!["front/event.ts".to_string()]);

    let resp = client
        .get(server.url("/cam/_meta/list?detail=true"))
        .header("Authorization", "Bearer tok")
        .send()
        .await
        .unwrap();
    let entries: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(entries.len(), 1);

    let resp = client
        .get(server.url(&format!("/cam/{}", tmp_name)))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    server.cleanup().await;
}

#[tokio::test]
async fn test_range_requests() {
    let server = TestServer::start().await;
    let client = reqwest::Client::new();

    server.create_bucket("vid", "tok").await;

    let resp = client
        .put(server.url("/vid/clip.bin"))
        .header("Authorization", "Bearer tok")
        .body("0123456789")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    // Normal GET advertises range support
    let resp = client
        .get(server.url("/vid/clip.bin"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["accept-ranges"], "bytes");
    assert_eq!(resp.text().await.unwrap(), "0123456789");

    // bytes=start-end
    let resp = client
        .get(server.url("/vid/clip.bin"))
        .header("Range", "bytes=0-3")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 206);
    assert_eq!(resp.headers()["content-range"], "bytes 0-3/10");
    assert_eq!(resp.headers()["content-length"], "4");
    assert_eq!(resp.text().await.unwrap(), "0123");

    // Open-ended bytes=start-
    let resp = client
        .get(server.url("/vid/clip.bin"))
        .header("Range", "bytes=4-")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 206);
    assert_eq!(resp.headers()["content-range"], "bytes 4-9/10");
    assert_eq!(resp.text().await.unwrap(), "456789");

    // Suffix bytes=-n
    let resp = client
        .get(server.url("/vid/clip.bin"))
        .header("Range", "bytes=-3")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 206);
    assert_eq!(resp.headers()["content-range"], "bytes 7-9/10");
    assert_eq!(resp.text().await.unwrap(), "789");

    // End clamped to file size
    let resp = client
        .get(server.url("/vid/clip.bin"))
        .header("Range", "bytes=5-999")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 206);
    assert_eq!(resp.headers()["content-range"], "bytes 5-9/10");
    assert_eq!(resp.text().await.unwrap(), "56789");

    // Unsatisfiable range
    let resp = client
        .get(server.url("/vid/clip.bin"))
        .header("Range", "bytes=100-")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 416);
    assert_eq!(resp.headers()["content-range"], "bytes */10");

    // Malformed range ignored -> full 200
    let resp = client
        .get(server.url("/vid/clip.bin"))
        .header("Range", "bytes=abc")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "0123456789");

    // Multi-range unsupported -> full 200
    let resp = client
        .get(server.url("/vid/clip.bin"))
        .header("Range", "bytes=0-1,3-4")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "0123456789");

    server.cleanup().await;
}

#[tokio::test]
async fn test_interrupted_upload_preserves_old_file() {
    use tokio::io::AsyncWriteExt;

    let server = TestServer::start().await;
    let client = reqwest::Client::new();

    server.create_bucket("nvr", "tok").await;

    let resp = client
        .put(server.url("/nvr/rec.ts"))
        .header("Authorization", "Bearer tok")
        .body("original content")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    // Start a PUT that declares more data than it sends, then drop the socket
    let mut stream = tokio::net::TcpStream::connect(server.addr).await.unwrap();
    let request = "PUT /nvr/rec.ts HTTP/1.1\r\n\
                   Host: localhost\r\n\
                   Authorization: Bearer tok\r\n\
                   Content-Length: 100000\r\n\
                   \r\n\
                   partial data";
    stream.write_all(request.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();
    drop(stream);

    sleep(Duration::from_millis(300)).await;

    // Old content untouched
    let resp = client.get(server.url("/nvr/rec.ts")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "original content");

    // No temp files left behind
    let mut entries = fs::read_dir(server.buckets_dir.join("nvr")).await.unwrap();
    while let Some(entry) = entries.next_entry().await.unwrap() {
        let name = entry.file_name().to_string_lossy().to_string();
        assert!(
            !name.ends_with(".stathost-tmp"),
            "stale temp file left behind: {}",
            name
        );
    }

    server.cleanup().await;
}

#[tokio::test]
async fn test_cleanup_temp_files() {
    let dir = PathBuf::from(format!("/tmp/stathost-cleanup-test-{}", std::process::id()));
    fs::create_dir_all(dir.join("bucket/sub")).await.unwrap();
    fs::write(dir.join("bucket/keep.txt"), "keep")
        .await
        .unwrap();
    fs::write(dir.join("bucket/old.ts.ab12-3.stathost-tmp"), "stale")
        .await
        .unwrap();
    fs::write(dir.join("bucket/sub/other.1-2.stathost-tmp"), "stale")
        .await
        .unwrap();

    stathost::cleanup_temp_files(&dir).await.unwrap();

    assert!(dir.join("bucket/keep.txt").exists());
    assert!(!dir.join("bucket/old.ts.ab12-3.stathost-tmp").exists());
    assert!(!dir.join("bucket/sub/other.1-2.stathost-tmp").exists());

    fs::remove_dir_all(&dir).await.unwrap();
}
