# StatHost

A lightweight static file hosting service written in Rust. Serves files from "buckets" (folders) with a simple API for uploading and managing content.

## Features

- **Static File Serving** - Serve files from bucket directories with proper MIME types
- **Large File Support** - Streaming uploads and downloads for handling large files
- **Simple Auth** - Per-bucket token authentication via `config.toml`
- **Multiple Buckets** - Host multiple independent buckets at different paths

## Bucket Structure

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

## Bucket Configuration

Each bucket requires a `config.toml`:

```toml
[auth]
token = "your-secret-token"
```

## API

The `_meta/` prefix is reserved for special endpoints and cannot be used as a file path.

### Serve Files

```
GET /{bucket}/{path}
```

Returns the file at the given path. Requests to `/{bucket}/` serve `index.html` if present. The `config.toml` file is protected and cannot be downloaded.

### Upload/Update File

```
PUT /{bucket}/{path}
Authorization: Bearer <token>
Content-Type: application/octet-stream

<file body>
```

Uploads or updates a file. Creates directories as needed.

### Delete File

```
DELETE /{bucket}/{path}
Authorization: Bearer <token>
```

Deletes a file from the bucket.

### List Files

```
GET /{bucket}/_meta/list
Authorization: Bearer <token>
```

Returns a JSON list of files in the bucket.

### OpenAPI Spec

```
GET /openapi.json
```

Returns the OpenAPI specification for the API.

## Configuration

Server configuration via environment variables or `stathost.toml`:

```toml
[server]
host = "0.0.0.0"
port = 8080
buckets_dir = "./buckets"
```

## Usage

```bash
# Run with default settings
stathost

# Or specify config
stathost --config /path/to/stathost.toml
```

## Building

```bash
cargo build --release
```

## License

MIT
