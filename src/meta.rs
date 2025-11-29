use axum::{
    Json,
    extract::{Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

use crate::{auth::extract_token, bucket::BucketManager};

pub async fn list_files(
    State(manager): State<Arc<BucketManager>>,
    Path(bucket_name): Path<String>,
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

    match bucket.list_files().await {
        Ok(files) => Json(files).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
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
                        {"name": "path", "in": "path", "required": true, "schema": {"type": "string"}}
                    ],
                    "responses": {
                        "200": {"description": "File content"},
                        "404": {"description": "File or bucket not found"}
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
                        {"name": "bucket", "in": "path", "required": true, "schema": {"type": "string"}}
                    ],
                    "responses": {
                        "200": {
                            "description": "List of files",
                            "content": {"application/json": {"schema": {"type": "array", "items": {"type": "string"}}}}
                        },
                        "401": {"description": "Unauthorized"},
                        "403": {"description": "Forbidden"}
                    }
                }
            }
        },
        "components": {
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
