# Systemd Deployment

Instructions for deploying StatHost as a systemd service on Linux.

## Prerequisites

Download and extract the Linux binary from [releases](https://github.com/nsg/stathost/releases):

```bash
tar xzf stathost-x86_64-unknown-linux-gnu.tar.gz
sudo install -m 755 stathost /usr/local/bin/
```

## Installation

### 1. Create the service user

```bash
sudo useradd -r -s /bin/false stathost
```

### 2. Create directories

```bash
sudo mkdir -p /etc/stathost
sudo mkdir -p /var/lib/stathost/buckets
sudo chown -R stathost:stathost /var/lib/stathost
```

### 3. Install configuration and service

```bash
sudo cp systemd/stathost.toml /etc/stathost/
sudo cp systemd/stathost.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now stathost
```

## Creating a bucket

```bash
# Create bucket directory
sudo mkdir /var/lib/stathost/buckets/my-site

# Create bucket config
echo -e '[auth]\ntoken = "your-secret-token"' | sudo tee /var/lib/stathost/buckets/my-site/config.toml

# Set ownership
sudo chown -R stathost:stathost /var/lib/stathost/buckets/my-site
```

## External storage

If mounting external storage to `/var/lib/stathost/buckets`:

1. Mount the volume before starting the service
2. Ensure correct ownership:
   ```bash
   sudo chown -R stathost:stathost /var/lib/stathost/buckets
   ```
