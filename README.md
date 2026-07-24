<p align="center">
  <img src="stathost.png" alt="StatHost" width="300">
</p>

<h1 align="center">StatHost</h1>

<p align="center">
  A lightweight static file hosting service written in Rust.<br>
  Serves files from "buckets" (folders) with a simple API for uploading and managing content.
</p>

---

## ✨ Features

- **Static File Serving** — Serve files from bucket directories with proper MIME types
- **Large File Support** — Streaming uploads and downloads for handling large files
- **Atomic Uploads** — Uploads are staged to a temp file and renamed into place; readers never see partial files
- **Range Requests** — Single-range `Range: bytes=...` support for efficient seeking in large files
- **Simple Auth** — Per-bucket token authentication via `config.toml`
- **Multiple Buckets** — Host multiple independent buckets at different paths

---

## 🚀 Quick Start

### 1. Install

Download a pre-built binary from [Releases](https://github.com/nsg/stathost/releases), or build from source:

```bash
cargo build --release
```

### 2. Create a Bucket

```bash
mkdir -p buckets/my-site
echo '[auth]
token = "my-secret-token"' > buckets/my-site/config.toml
echo '<h1>Hello World</h1>' > buckets/my-site/index.html
```

### 3. Run

```bash
./stathost
```

### 4. Access

- Browse: http://localhost:8080/my-site/
- Upload: `curl -X PUT -H "Authorization: Bearer my-secret-token" --data-binary @file.txt http://localhost:8080/my-site/file.txt`

---

## 📁 Bucket Structure

Each bucket is a folder containing a `config.toml` and your static files:

```
buckets/
├── my-site/
│   ├── config.toml      # Protected, not downloadable
│   ├── index.html
│   ├── style.css
│   └── assets/
│       └── large-video.mp4
└── another-bucket/
    ├── config.toml
    └── ...
```

### Bucket Configuration

Each bucket requires a `config.toml`:

```toml
[auth]
token = "your-secret-token"
```

---

## 🔌 API Reference

The `_meta/` prefix is reserved for special endpoints and cannot be used as a file path.

### Root Index

```http
GET /
```

If a bucket named `index` exists, serves its `index.html`. Returns 404 otherwise.

### Serve Files

```http
GET /{bucket}/{path}
```

Returns the file at the given path. Requests to `/{bucket}/` serve `index.html` if present. The `config.toml` file is protected and cannot be downloaded.

Single byte ranges are supported: `Range: bytes=0-1023`, `bytes=1024-`, or `bytes=-500` return `206 Partial Content` with a `Content-Range` header. Unsatisfiable ranges return `416`; multi-range requests fall back to the full `200` response.

### Upload/Update File

```http
PUT /{bucket}/{path}
Authorization: Bearer <token>
Content-Type: application/octet-stream

<file body>
```

Uploads or updates a file. Creates directories as needed. The upload is written to a temporary `*.stathost-tmp` file and atomically renamed into place once fully received, so an interrupted upload never corrupts or truncates an existing file. Stale temp files are cleaned up on server startup.

### Delete File

```http
DELETE /{bucket}/{path}
Authorization: Bearer <token>
```

Deletes a file from the bucket.

### List Files

```http
GET /{bucket}/_meta/list
Authorization: Bearer <token>
```

Returns a JSON array of file paths in the bucket. Add `?detail=true` to instead receive objects with size and modification time:

```json
[
  {"path": "images/photo.jpg", "size": 38925024, "mtime": 1784879516}
]
```

`size` is in bytes and `mtime` is Unix seconds (UTC).

### OpenAPI Spec

```http
GET /openapi.json
```

Returns the OpenAPI 3.0 specification for the API.

---

## ⚙️ Configuration

Create a `stathost.toml` file to configure the server:

```toml
[server]
host = "0.0.0.0"
port = 8080
buckets_dir = "./buckets"
```

All settings are optional and have sensible defaults.

### Command Line

```bash
# Run with default settings
stathost

# Run with custom config
stathost --config /path/to/stathost.toml
```

---

## 📖 Examples

### Upload a file

```bash
curl -X PUT \
  -H "Authorization: Bearer my-secret-token" \
  --data-binary @photo.jpg \
  http://localhost:8080/my-site/images/photo.jpg
```

### List files in a bucket

```bash
curl -H "Authorization: Bearer my-secret-token" \
  http://localhost:8080/my-site/_meta/list
```

### Delete a file

```bash
curl -X DELETE \
  -H "Authorization: Bearer my-secret-token" \
  http://localhost:8080/my-site/old-file.txt
```

---

## 📄 License

MIT
