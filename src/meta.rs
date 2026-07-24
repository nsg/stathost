use axum::{
    Json,
    extract::{Path, Query, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::sync::Arc;

use crate::{auth::extract_token, bucket::BucketManager};

#[derive(Deserialize)]
pub struct ListParams {
    #[serde(default)]
    detail: bool,
}

pub async fn list_files(
    State(manager): State<Arc<BucketManager>>,
    Path(bucket_name): Path<String>,
    Query(params): Query<ListParams>,
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

    if params.detail {
        match bucket.list_files_detailed().await {
            Ok(entries) => Json(entries).into_response(),
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    } else {
        match bucket.list_files().await {
            Ok(files) => Json(files).into_response(),
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }
}

pub async fn openapi() -> Response {
    let spec = serde_json::json!({
        "openapi": "3.0.3",
        "info": {
            "title": "StatHost API",
            "version": "1.0.0",
            "description": "A lightweight static file hosting service"
        },
        "paths": {
            "/{bucket}/{path}": {
                "get": {
                    "summary": "Serve a file",
                    "parameters": [
                        {"name": "bucket", "in": "path", "required": true, "schema": {"type": "string"}},
                        {"name": "path", "in": "path", "required": true, "schema": {"type": "string"}},
                        {"name": "Range", "in": "header", "required": false, "schema": {"type": "string"},
                         "description": "Single byte range (e.g. bytes=0-1023, bytes=1024-, bytes=-500). Multi-range requests are answered with the full body."}
                    ],
                    "responses": {
                        "200": {"description": "File content (Accept-Ranges: bytes)"},
                        "206": {"description": "Partial file content with Content-Range: bytes start-end/total"},
                        "404": {"description": "File or bucket not found"},
                        "416": {"description": "Range not satisfiable; Content-Range: bytes */total"}
                    }
                },
                "put": {
                    "summary": "Upload or update a file",
                    "security": [{"bearerAuth": []}],
                    "parameters": [
                        {"name": "bucket", "in": "path", "required": true, "schema": {"type": "string"}},
                        {"name": "path", "in": "path", "required": true, "schema": {"type": "string"}}
                    ],
                    "requestBody": {
                        "content": {"application/octet-stream": {"schema": {"type": "string", "format": "binary"}}}
                    },
                    "responses": {
                        "201": {"description": "File created/updated"},
                        "401": {"description": "Unauthorized"},
                        "403": {"description": "Forbidden"}
                    }
                },
                "delete": {
                    "summary": "Delete a file",
                    "security": [{"bearerAuth": []}],
                    "parameters": [
                        {"name": "bucket", "in": "path", "required": true, "schema": {"type": "string"}},
                        {"name": "path", "in": "path", "required": true, "schema": {"type": "string"}}
                    ],
                    "responses": {
                        "204": {"description": "File deleted"},
                        "401": {"description": "Unauthorized"},
                        "403": {"description": "Forbidden"},
                        "404": {"description": "File not found"}
                    }
                }
            },
            "/{bucket}/_meta/list": {
                "get": {
                    "summary": "List files in bucket",
                    "security": [{"bearerAuth": []}],
                    "parameters": [
                        {"name": "bucket", "in": "path", "required": true, "schema": {"type": "string"}},
                        {"name": "detail", "in": "query", "required": false, "schema": {"type": "boolean", "default": false},
                         "description": "When true, returns objects with path, size (bytes) and mtime (Unix seconds) instead of plain path strings."}
                    ],
                    "responses": {
                        "200": {
                            "description": "List of files (array of strings, or of FileEntry objects when detail=true)",
                            "content": {"application/json": {"schema": {"oneOf": [
                                {"type": "array", "items": {"type": "string"}},
                                {"type": "array", "items": {"$ref": "#/components/schemas/FileEntry"}}
                            ]}}}
                        },
                        "401": {"description": "Unauthorized"},
                        "403": {"description": "Forbidden"}
                    }
                }
            }
        },
        "components": {
            "schemas": {
                "FileEntry": {
                    "type": "object",
                    "required": ["path", "size", "mtime"],
                    "properties": {
                        "path": {"type": "string"},
                        "size": {"type": "integer", "format": "int64", "description": "File size in bytes"},
                        "mtime": {"type": "integer", "format": "int64", "description": "Last modification time, Unix seconds (UTC)"}
                    }
                }
            },
            "securitySchemes": {
                "bearerAuth": {
                    "type": "http",
                    "scheme": "bearer"
                }
            }
        }
    });

    Json(spec).into_response()
}
