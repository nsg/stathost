use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;

use crate::{auth::extract_token, bucket::BucketManager};

pub async fn serve_file(
    State(manager): State<Arc<BucketManager>>,
    Path((bucket_name, file_path)): Path<(String, String)>,
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

    let Ok(file) = File::open(&path).await else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let Ok(metadata) = file.metadata().await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };

    let mime = mime_guess::from_path(&path)
        .first_or_octet_stream()
        .to_string();

    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    match Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, metadata.len())
        .body(body)
    {
        Ok(response) => response,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub async fn serve_bucket_root(
    State(manager): State<Arc<BucketManager>>,
    Path(bucket_name): Path<String>,
) -> Response {
    serve_file(State(manager), Path((bucket_name, String::new()))).await
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

    let mut file = match File::create(&path).await {
        Ok(f) => f,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let body = request.into_body();
    let mut stream = body.into_data_stream();

    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(data) => {
                if file.write_all(&data).await.is_err() {
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            }
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        }
    }

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
