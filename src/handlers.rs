use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, SeekFrom};
use tokio_util::io::ReaderStream;

use crate::{
    auth::extract_token,
    bucket::{BucketManager, TMP_SUFFIX},
};

enum RangeSpec {
    FromTo(u64, u64),
    From(u64),
    Suffix(u64),
}

fn parse_range(value: &str) -> Option<RangeSpec> {
    let spec = value.strip_prefix("bytes=")?.trim();
    if spec.contains(',') {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    match (start.is_empty(), end.is_empty()) {
        (false, false) => {
            let start: u64 = start.parse().ok()?;
            let end: u64 = end.parse().ok()?;
            if start > end {
                return None;
            }
            Some(RangeSpec::FromTo(start, end))
        }
        (false, true) => Some(RangeSpec::From(start.parse().ok()?)),
        (true, false) => Some(RangeSpec::Suffix(end.parse().ok()?)),
        (true, true) => None,
    }
}

fn resolve_range(spec: RangeSpec, total: u64) -> Option<(u64, u64)> {
    match spec {
        RangeSpec::FromTo(start, end) => (start < total).then(|| (start, end.min(total - 1))),
        RangeSpec::From(start) => (start < total).then(|| (start, total - 1)),
        RangeSpec::Suffix(n) => (n > 0 && total > 0).then(|| (total.saturating_sub(n), total - 1)),
    }
}

pub async fn serve_file(
    State(manager): State<Arc<BucketManager>>,
    Path((bucket_name, file_path)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let Some(bucket) = manager.get_bucket(&bucket_name) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let file_path = if file_path.is_empty() || file_path.ends_with('/') {
        format!("{}index.html", file_path)
    } else {
        file_path
    };

    let Some(path) = bucket.resolve_path(&file_path) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let Ok(mut file) = File::open(&path).await else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let Ok(metadata) = file.metadata().await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let total = metadata.len();

    let mime = mime_guess::from_path(&path)
        .first_or_octet_stream()
        .to_string();

    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_range);

    if let Some(spec) = range {
        let Some((start, end)) = resolve_range(spec, total) else {
            return match Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{}", total))
                .header(header::ACCEPT_RANGES, "bytes")
                .body(Body::empty())
            {
                Ok(response) => response,
                Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            };
        };

        if file.seek(SeekFrom::Start(start)).await.is_err() {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }

        let len = end - start + 1;
        let stream = ReaderStream::new(file.take(len));
        let body = Body::from_stream(stream);

        return match Response::builder()
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_TYPE, mime)
            .header(header::CONTENT_LENGTH, len)
            .header(
                header::CONTENT_RANGE,
                format!("bytes {}-{}/{}", start, end, total),
            )
            .header(header::ACCEPT_RANGES, "bytes")
            .body(body)
        {
            Ok(response) => response,
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
    }

    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    match Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, total)
        .header(header::ACCEPT_RANGES, "bytes")
        .body(body)
    {
        Ok(response) => response,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub async fn serve_bucket_root(
    State(manager): State<Arc<BucketManager>>,
    Path(bucket_name): Path<String>,
    headers: HeaderMap,
) -> Response {
    serve_file(State(manager), Path((bucket_name, String::new())), headers).await
}

pub async fn serve_root_index(
    State(manager): State<Arc<BucketManager>>,
    headers: HeaderMap,
) -> Response {
    serve_file(
        State(manager),
        Path(("index".to_string(), String::new())),
        headers,
    )
    .await
}

fn temp_path(path: &std::path::Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!("{}.{:x}-{:x}{}", name, nanos, unique, TMP_SUFFIX))
}

// Removes the temp file even if the handler future is dropped mid-upload
// (client disconnect); disarmed once the file is renamed into place.
struct TempFileGuard {
    path: PathBuf,
    armed: bool,
}

impl TempFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if self.armed {
            let path = std::mem::take(&mut self.path);
            tokio::spawn(async move {
                let _ = tokio::fs::remove_file(path).await;
            });
        }
    }
}

async fn write_body(file: &mut File, body: Body) -> Result<(), StatusCode> {
    use futures_util::StreamExt;

    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(data) => {
                if file.write_all(&data).await.is_err() {
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                }
            }
            Err(_) => return Err(StatusCode::BAD_REQUEST),
        }
    }

    if file.flush().await.is_err() || file.sync_all().await.is_err() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(())
}

pub async fn upload_file(
    State(manager): State<Arc<BucketManager>>,
    Path((bucket_name, file_path)): Path<(String, String)>,
    request: Request,
) -> Response {
    let Some(bucket) = manager.get_bucket(&bucket_name) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let Some(token) = extract_token(request.headers()) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    if !bucket.validate_token(token) {
        return StatusCode::FORBIDDEN.into_response();
    }

    let Some(path) = bucket.resolve_path(&file_path) else {
        return (StatusCode::BAD_REQUEST, "Invalid path").into_response();
    };

    if let Some(parent) = path.parent()
        && tokio::fs::create_dir_all(parent).await.is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let tmp = temp_path(&path);
    let mut file = match File::create(&tmp).await {
        Ok(f) => f,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let mut guard = TempFileGuard::new(tmp.clone());

    if let Err(status) = write_body(&mut file, request.into_body()).await {
        return status.into_response();
    }

    drop(file);
    if tokio::fs::rename(&tmp, &path).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    guard.disarm();

    StatusCode::CREATED.into_response()
}

pub async fn delete_file(
    State(manager): State<Arc<BucketManager>>,
    Path((bucket_name, file_path)): Path<(String, String)>,
    request: Request,
) -> Response {
    let Some(bucket) = manager.get_bucket(&bucket_name) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let Some(token) = extract_token(request.headers()) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    if !bucket.validate_token(token) {
        return StatusCode::FORBIDDEN.into_response();
    }

    let Some(path) = bucket.resolve_path(&file_path) else {
        return (StatusCode::BAD_REQUEST, "Invalid path").into_response();
    };

    match tokio::fs::remove_file(&path).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
