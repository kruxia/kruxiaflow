# Raspberry Pi Deployment Guide

This guide covers cross-compiling, configuring, and running Kruxia Flow on Raspberry Pi ARM devices.

## Supported Platforms

| Device           | Architecture   | Target Triple                 | Notes                           |
|------------------|----------------|-------------------------------|---------------------------------|
| Pi Zero / Zero W | ARMv6 (32-bit) | `arm-unknown-linux-gnueabihf` | 512MB RAM, single-core, 1GHz    |
| Pi Zero 2 W      | ARMv8 (64-bit) | `aarch64-unknown-linux-gnu`   | 512MB RAM, quad-core, 1GHz      |
| Pi 3/4/5         | ARMv8 (64-bit) | `aarch64-unknown-linux-gnu`   | 1-8GB RAM, quad-core, 1.5-2.4GHz |

**Important**: Pi Zero W requires 32-bit Raspberry Pi OS and the `arm-unknown-linux-gnueabihf` target. It cannot run 64-bit binaries.

---

## Cross-Compilation

### Prerequisites (Build Machine)

Install the cross-compilation toolchain on your development machine (macOS or Linux).

#### macOS (Apple Silicon or Intel)

```bash
# Install Rust targets
rustup target add arm-unknown-linux-gnueabihf    # Pi Zero (32-bit)
rustup target add aarch64-unknown-linux-gnu      # Pi Zero 2 W, Pi 3/4/5 (64-bit)

# Install cross-compilation toolchain via Homebrew
brew install arm-linux-gnueabihf-binutils        # 32-bit ARM
brew install aarch64-unknown-linux-gnu           # 64-bit ARM

# Or use messense's prebuilt toolchains (recommended)
brew tap messense/macos-cross-toolchains
brew install arm-unknown-linux-gnueabihf         # 32-bit ARM
brew install aarch64-unknown-linux-gnu           # 64-bit ARM
```

#### Linux (Ubuntu/Debian)

```bash
# Install Rust targets
rustup target add arm-unknown-linux-gnueabihf
rustup target add aarch64-unknown-linux-gnu

# Install cross-compilation toolchain
sudo apt-get update
sudo apt-get install -y gcc-arm-linux-gnueabihf      # 32-bit ARM
sudo apt-get install -y gcc-aarch64-linux-gnu        # 64-bit ARM
```

### Cargo Configuration

The project includes `.cargo/config.toml` with linker configuration:

```toml
# .cargo/config.toml
[target.arm-unknown-linux-gnueabihf]
linker = "arm-linux-gnueabihf-gcc"

[target.aarch64-unknown-linux-gnu]
linker = "aarch64-linux-gnu-gcc"
```

### Build Commands

```bash
# For Raspberry Pi Zero (32-bit, ARMv6)
cargo build --release --target arm-unknown-linux-gnueabihf

# For Raspberry Pi Zero 2 W, Pi 3/4/5 (64-bit, ARMv8)
cargo build --release --target aarch64-unknown-linux-gnu

# Binary location
ls -la target/arm-unknown-linux-gnueabihf/release/kruxiaflow
ls -la target/aarch64-unknown-linux-gnu/release/kruxiaflow
```

### Binary Size

The release binary is optimized for size (`opt-level = 'z'`, LTO enabled, symbols stripped):

| Target                        | Binary Size |
|-------------------------------|-------------|
| `arm-unknown-linux-gnueabihf` | ~7-8 MB     |
| `aarch64-unknown-linux-gnu`   | ~7-8 MB     |

---

## Raspberry Pi Setup

### 1. Install Raspberry Pi OS

Use **Raspberry Pi OS Lite (64-bit)** for Pi Zero 2 W or newer. This guide assumes **Raspbian Trixie** or **Bookworm**.

```bash
# Flash using Raspberry Pi Imager
# - Select: Raspberry Pi OS Lite (64-bit)
# - Configure: hostname, WiFi, SSH, username/password
```

### 2. Install PostgreSQL 17

Raspbian Trixie includes PostgreSQL 17 in the official repositories:

```bash
# SSH into the Pi
ssh pi@raspberrypi.local

# Install PostgreSQL
sudo apt-get update
sudo apt-get install -y postgresql-17 postgresql-client-17

# Start and enable PostgreSQL
sudo systemctl enable postgresql
sudo systemctl start postgresql

# Create database and user
sudo -u postgres psql <<EOF
CREATE USER kruxiaflow WITH PASSWORD '${DB_PWD}';
CREATE DATABASE kruxiaflow OWNER kruxiaflow;
GRANT ALL PRIVILEGES ON DATABASE kruxiaflow TO kruxiaflow;
\c kruxiaflow
CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS pg_stat_statements;
EOF
```

**Note**: The `pg_stat_statements` extension must be created by the `postgres` superuser. The profiling-views migration runs `CREATE EXTENSION IF NOT EXISTS pg_stat_statements`, and the `kruxiaflow` role is not allowed to create it, so migrations fail with `permission denied to create extension "pg_stat_statements"` unless it already exists. Its views only return data if `pg_stat_statements` is also added to `shared_preload_libraries`; that is only needed for `kruxiaflow profile`, so it can be skipped on a Pi.

### 3. Configure PostgreSQL for Low Memory

Put Raspberry Pi tuning in a drop-in file, `/etc/postgresql/17/main/conf.d/pi.conf`, so it survives package upgrades of `postgresql.conf`.

**Connection limits**: Each Kruxia Flow command keeps a database connection pool, and every pooled connection is a PostgreSQL backend process. `max_connections` must leave room for Kruxia Flow's pool maximum (`KRUXIAFLOW_DB_MAX_CONNECTIONS`), plus the 3 connections PostgreSQL reserves for superusers (`superuser_reserved_connections`), plus a few for `psql` and one-off commands such as `kruxiaflow seed-llm`. With the pool settings recommended below (`serve` max 15, min 2), `max_connections = 25` leaves enough headroom. Do not go below that: with the default pool (min 10, max 100), a lower limit makes `serve` fail to connect.

**For Pi Zero W (single-core, 512MB RAM)** - most aggressive settings:

```ini
# Memory settings (minimal for Pi Zero W)
shared_buffers = 32MB
effective_cache_size = 64MB
work_mem = 2MB
maintenance_work_mem = 16MB

# Connection limits (see "Connection limits" above)
max_connections = 25

# Single core: parallel query only adds overhead
max_worker_processes = 2
max_parallel_workers = 0
max_parallel_workers_per_gather = 0

# Aggressive vacuuming (important for limited storage)
autovacuum_max_workers = 1
autovacuum_naptime = 60s
autovacuum_vacuum_cost_limit = 100

# WAL settings
wal_buffers = 2MB
checkpoint_completion_target = 0.9

# Logging (disable for production to save I/O)
logging_collector = off
```

**For Pi Zero 2 W / Pi 3+ (quad-core, 512MB+ RAM)**:

```ini
# Memory settings
shared_buffers = 64MB
effective_cache_size = 128MB
work_mem = 4MB
maintenance_work_mem = 32MB

# Connection limits (see "Connection limits" above)
max_connections = 25

# Aggressive vacuuming
autovacuum_naptime = 30s
autovacuum_vacuum_cost_limit = 200

# WAL settings
wal_buffers = 4MB
checkpoint_completion_target = 0.9

# Logging (optional)
log_min_duration_statement = 1000  # Log queries > 1s
```

**Reading memory use**: `top` shows each PostgreSQL process with a large `RES` value, but most of it is `SHR` - the same `shared_buffers` memory counted once per process. The private memory of an idle backend is about 2MB. `shared_buffers` is the real ceiling, because it fills up as data is read.

Restart PostgreSQL:

```bash
sudo systemctl restart postgresql
```

### 4. Deploy Kruxia Flow Binary

Copy the cross-compiled binary to the Pi:

```bash
# From your build machine
# For Pi Zero W (32-bit):
scp target/arm-unknown-linux-gnueabihf/release/kruxiaflow pi@raspberrypi.local:/home/kruxiaflow/

# For Pi Zero 2 W / Pi 3/4/5 (64-bit):
# scp target/aarch64-unknown-linux-gnu/release/kruxiaflow pi@raspberrypi.local:/home/kruxiaflow/

# On the Pi, move to /usr/local/bin
ssh pi@raspberrypi.local
sudo mv /home/kruxiaflow/kruxiaflow /usr/local/bin/
sudo chmod +x /usr/local/bin/kruxiaflow

# Verify
kruxiaflow version
```

### 5. Generate OAuth Keys

Kruxia Flow requires RSA keys for JWT authentication:

```bash
# Generate RSA key pair
openssl genrsa -out /home/kruxiaflow/kruxiaflow-private.pem 2048
openssl rsa -in /home/kruxiaflow/kruxiaflow-private.pem -pubout -out /home/kruxiaflow/kruxiaflow-public.pem

# Set permissions
chmod 600 /home/kruxiaflow/kruxiaflow-private.pem
chmod 644 /home/kruxiaflow/kruxiaflow-public.pem
```

### 6. Create Environment File

Create `/etc/kruxiaflow/env`:

```bash
sudo mkdir -p /etc/kruxiaflow
sudo tee /etc/kruxiaflow/env > /dev/null <<'EOF'
# Database
DATABASE_URL=postgres://kruxiaflow:your_secure_password@localhost:5432/kruxiaflow

# API Server
KRUXIAFLOW_API_PORT=8080
KRUXIAFLOW_API_BIND=0.0.0.0

# Worker (reduced for Pi Zero W's single-core CPU)
KRUXIAFLOW_WORKER_MAX_ACTIVITIES=2
KRUXIAFLOW_WORKER_POLL_MAX_ACTIVITIES=1

# Database pool (default min 10 / max 100; keep max below PostgreSQL max_connections)
KRUXIAFLOW_DB_MAX_CONNECTIONS=15
KRUXIAFLOW_DB_MIN_CONNECTIONS=2

# OAuth
KRUXIAFLOW_CLIENT_ID=kruxiaflow_internal
KRUXIAFLOW_CLIENT_SECRET=generate_a_secure_secret_here

# OAuth RSA keys - use _FILE suffix to load from file path
# (alternatively, set KRUXIAFLOW_OAUTH_RSA_PRIVATE_KEY_PEM with the actual PEM content)
KRUXIAFLOW_OAUTH_RSA_PRIVATE_KEY_PEM_FILE=/home/kruxiaflow/kruxiaflow-private.pem
KRUXIAFLOW_OAUTH_RSA_PUBLIC_KEY_PEM_FILE=/home/kruxiaflow/kruxiaflow-public.pem

# Logging
KRUXIAFLOW_LOG_LEVEL=info
KRUXIAFLOW_LOG_FORMAT=text

# Timeouts (increase for slower hardware)
KRUXIAFLOW_ACTIVITY_TIMEOUT=600
KRUXIAFLOW_SHUTDOWN_TIMEOUT=60

# LLM API Keys (add your own)
ANTHROPIC_API_KEY=sk-ant-...
OPENAI_API_KEY=sk-...
EOF

sudo chmod 600 /etc/kruxiaflow/env
```

**Note**: The `_FILE` suffix tells Kruxia Flow to read the secret from a file path rather than expecting the content directly in the environment variable. This is the Docker secrets pattern.

### 7. Create systemd Service

Create `/etc/systemd/system/kruxiaflow.service`:

```ini
[Unit]
Description=Kruxia Flow Workflow Orchestration
After=network.target postgresql.service
Requires=postgresql.service

[Service]
Type=simple
User=kruxiaflow
Group=kruxiaflow
EnvironmentFile=/etc/kruxiaflow/env
ExecStart=/usr/local/bin/kruxiaflow serve --migrate --seed-client
Restart=on-failure
RestartSec=10

# Resource limits for Raspberry Pi
# For Pi Zero W: use 150M to leave room for PostgreSQL
# For Pi Zero 2 W+: can use 200M
MemoryMax=150M
CPUQuota=80%

# Security hardening
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=/tmp

[Install]
WantedBy=multi-user.target
```

Enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable kruxiaflow
sudo systemctl start kruxiaflow

# Check status
sudo systemctl status kruxiaflow
journalctl -u kruxiaflow -f
```

---

## Configuration Reference

### Environment Variables

| Variable                                    | Default                      | Description                             |
|---------------------------------------------|------------------------------|-----------------------------------------|
| `DATABASE_URL`                              | (required)                   | PostgreSQL connection string            |
| `KRUXIAFLOW_API_PORT`                       | `8080`                       | API server port                         |
| `KRUXIAFLOW_API_BIND`                       | `0.0.0.0`                    | API server bind address                 |
| `KRUXIAFLOW_WORKER_MAX_ACTIVITIES`          | `16`                         | Max concurrent activities               |
| `KRUXIAFLOW_WORKER_POLL_MAX_ACTIVITIES`     | `5`                          | Activities claimed per poll             |
| `KRUXIAFLOW_DB_MAX_CONNECTIONS`             | `100` (`serve`)              | Database pool maximum                   |
| `KRUXIAFLOW_DB_MIN_CONNECTIONS`             | `10` (`serve`)               | Database pool idle minimum              |
| `KRUXIAFLOW_CLIENT_ID`                      | `kruxiaflow_internal_worker` | OAuth client ID                         |
| `KRUXIAFLOW_CLIENT_SECRET`                  | (required)                   | OAuth client secret                     |
| `KRUXIAFLOW_OAUTH_RSA_PRIVATE_KEY_PEM`      | (required)                   | RSA private key (PEM content)           |
| `KRUXIAFLOW_OAUTH_RSA_PRIVATE_KEY_PEM_FILE` | -                            | Path to RSA private key file            |
| `KRUXIAFLOW_OAUTH_RSA_PUBLIC_KEY_PEM_FILE`  | -                            | Path to RSA public key file             |
| `KRUXIAFLOW_ACTIVITY_TIMEOUT`               | `300`                        | Activity timeout (seconds)              |
| `KRUXIAFLOW_LOG_LEVEL`                      | `info`                       | Log level (trace/debug/info/warn/error) |
| `KRUXIAFLOW_LOG_FORMAT`                     | `text`                       | Log format (text/json)                  |

**Docker Secrets Pattern**: For any secret variable, you can use a `_FILE` suffix to load the value from a file path instead of setting the content directly. The file content will be trimmed of whitespace.

### Recommended Settings by Device

| Setting                                    | Pi Zero W (512MB) | Pi Zero 2 W (512MB) | Pi 4 (2GB+) |
|--------------------------------------------|-------------------|---------------------|-------------|
| `KRUXIAFLOW_WORKER_MAX_ACTIVITIES`         | 2                 | 4                   | 8-16        |
| `KRUXIAFLOW_WORKER_POLL_MAX_ACTIVITIES`    | 1                 | 2                   | 4-5         |
| `KRUXIAFLOW_DB_MAX_CONNECTIONS`            | 15                | 15                  | 40          |
| `KRUXIAFLOW_DB_MIN_CONNECTIONS`            | 2                 | 2                   | 5           |
| PostgreSQL `shared_buffers`                | 32MB              | 64MB                | 256MB       |
| PostgreSQL `max_connections`               | 25                | 25                  | 50          |

`KRUXIAFLOW_DB_MAX_CONNECTIONS` plus 3 superuser-reserved connections plus a few for admin tools must stay at or below PostgreSQL `max_connections`.

**Pi Zero W Notes**: The single-core ARM11 CPU limits parallelism. Use minimal concurrency settings and expect slower activity execution. Consider running PostgreSQL on a separate host for better performance.

---

## PostgreSQL 17 Compatibility

Kruxia Flow is compatible with PostgreSQL 17+. The `uuidv7()` function used for primary keys is:
- **PostgreSQL 18+**: Uses native `uuidv7()` function
- **PostgreSQL 17**: Uses custom PL/pgSQL implementation (created automatically by migrations)

### Performance Comparison

| Platform                              | PostgreSQL | Time per UUID | Max Throughput |
|---------------------------------------|------------|---------------|----------------|
| Apple M4 Pro (ARMv9, 64-bit)          | 18 native  | ~2 μs         | ~500k/sec      |
| Apple M4 Pro (ARMv9, 64-bit)          | 17 custom  | ~7 μs         | ~143k/sec      |
| Raspberry Pi Zero W (ARMv6, 32-bit)   | 17 custom  | ~1.4 ms       | ~720/sec       |

The Pi Zero W is ~200x slower than M4 Pro due to the single-core 1GHz ARM11 CPU and PL/pgSQL interpretation overhead. At ~720 UUIDs/second, this is still sufficient for typical workflow orchestration (each workflow creates ~5-10 UUIDs).

---

## Running Kruxia Flow

### Initialize Database

```bash
# Run migrations
kruxiaflow migrate

# Seed OAuth client
kruxiaflow seed-client

# Seed LLM model catalog
kruxiaflow seed-llm /path/to/llm_models.yaml
```

### Start Server

```bash
# All-in-one mode (recommended for Pi)
kruxiaflow serve

# Or with explicit options
kruxiaflow serve \
  --port 8080 \
  --max-activities 4 \
  --migrate \
  --seed-client
```

### Verify Installation

```bash
# Check health
curl http://localhost:8080/health

# Check API docs
curl http://localhost:8080/api/v1/docs

# Check version
curl http://localhost:8080/api/v1/info
```

---

## Upgrading

Release binaries are not published for ARMv6, so cross-compile the release tag (see [Cross-Compilation](#cross-compilation)) and copy the binary to the Pi. Then:

```bash
# Back up the database and the current binary
sudo -u postgres pg_dump -Fc kruxiaflow > ~/kruxiaflow-$(date +%Y%m%d).dump
cp /usr/local/bin/kruxiaflow ~/kruxiaflow.previous

# Install the new binary and restart; --migrate in the systemd unit applies new migrations
sudo systemctl stop kruxiaflow
sudo install -m 755 ~/kruxiaflow /usr/local/bin/kruxiaflow
sudo systemctl start kruxiaflow
journalctl -u kruxiaflow -n 50

# Refresh the LLM model catalog and pricing (run with the service's environment)
sudo systemd-run --pipe --wait --collect -p User=kruxiaflow \
  -p EnvironmentFile=/etc/kruxiaflow/env \
  /usr/local/bin/kruxiaflow seed-llm /etc/kruxiaflow/llm_models.yaml

# Verify
curl http://localhost:8080/api/v1/info
```

Migrations only move forward. To roll back, restore both the previous binary and the database dump.

---

## Performance Tuning

### Swap Configuration

For devices with limited RAM (512MB), configure swap:

```bash
# Increase swap size
sudo dphys-swapfile swapoff
sudo sed -i 's/CONF_SWAPSIZE=.*/CONF_SWAPSIZE=1024/' /etc/dphys-swapfile
sudo dphys-swapfile setup
sudo dphys-swapfile swapon

# Verify
free -h
```

### Storage Optimization

Use a high-quality microSD card (A2 rated) or USB SSD for better PostgreSQL performance:

```bash
# Check I/O performance
sudo hdparm -t /dev/mmcblk0

# For USB SSD boot, follow Raspberry Pi documentation
```

### Network Configuration

For production deployments, configure a static IP:

```bash
# Edit /etc/dhcpcd.conf
sudo tee -a /etc/dhcpcd.conf > /dev/null <<EOF
interface wlan0
static ip_address=192.168.1.100/24
static routers=192.168.1.1
static domain_name_servers=192.168.1.1 8.8.8.8
EOF

sudo systemctl restart dhcpcd
```

---

## Troubleshooting

### Binary Won't Execute

```bash
# Check architecture
file /usr/local/bin/kruxiaflow
uname -m

# For "Exec format error", you compiled for wrong architecture
# Pi Zero (original): arm-unknown-linux-gnueabihf
# Pi Zero 2 W / Pi 3/4/5: aarch64-unknown-linux-gnu
```

### Database Connection Fails

```bash
# Check PostgreSQL is running
sudo systemctl status postgresql

# Check connection
psql -h localhost -U kruxiaflow -d kruxiaflow -c "SELECT 1"

# Check pg_hba.conf allows local connections
sudo cat /etc/postgresql/17/main/pg_hba.conf | grep -v "^#"
```

### Out of Memory

```bash
# Check memory usage
free -h
htop

# Reduce worker concurrency
export KRUXIAFLOW_WORKER_MAX_ACTIVITIES=2

# Reduce idle database connections (one PostgreSQL backend each)
export KRUXIAFLOW_DB_MIN_CONNECTIONS=2

# Reduce PostgreSQL memory
# Edit /etc/postgresql/17/main/postgresql.conf
# shared_buffers = 32MB
```

### Service Fails to Start

```bash
# Check logs
journalctl -u kruxiaflow -n 100

# Common issues:
# - Missing DATABASE_URL
# - Missing OAuth keys
# - PostgreSQL not ready (increase RestartSec in systemd)
```

**`permission denied to create extension "pg_stat_statements"`**: Create the extension as the superuser, then restart the service:

```bash
sudo -u postgres psql -d kruxiaflow -c "CREATE EXTENSION IF NOT EXISTS pg_stat_statements"
```

**`migration <version> was previously applied but has been modified`**: The database was migrated by a binary whose copy of that migration differs from the new binary's, which typically happens with a build made from an uncommitted working tree. Released versions do not modify published migrations. If the database holds nothing you need, drop and recreate it (see [Install PostgreSQL 17](#2-install-postgresql-17), including both `CREATE EXTENSION` statements) and restart the service. Otherwise, compare the two versions of the migration before changing anything in `_sqlx_migrations`.

**`sorry, too many clients already` / pool timeouts**: Kruxia Flow's pool maximum (`KRUXIAFLOW_DB_MAX_CONNECTIONS`, default 100 for `serve`) exceeds PostgreSQL's `max_connections`. Lower the pool maximum or raise `max_connections` (see [Configure PostgreSQL for Low Memory](#3-configure-postgresql-for-low-memory)).

---

## Example: Complete Setup Script

```bash
#!/bin/bash
# setup-kruxiaflow.sh - Complete Raspberry Pi setup

set -e

echo "=== Installing PostgreSQL 17 ==="
sudo apt-get update
sudo apt-get install -y postgresql-17 postgresql-client-17

echo "=== Configuring PostgreSQL ==="
sudo -u postgres psql <<EOF
CREATE USER kruxiaflow WITH PASSWORD 'changeme';
CREATE DATABASE kruxiaflow OWNER kruxiaflow;
GRANT ALL PRIVILEGES ON DATABASE kruxiaflow TO kruxiaflow;
\c kruxiaflow
CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS pg_stat_statements;
EOF
sudo tee /etc/postgresql/17/main/conf.d/pi.conf > /dev/null <<EOF
shared_buffers = 32MB
work_mem = 2MB
maintenance_work_mem = 16MB
max_connections = 25
max_worker_processes = 2
max_parallel_workers = 0
max_parallel_workers_per_gather = 0
autovacuum_max_workers = 1
EOF
sudo systemctl restart postgresql

echo "=== Generating OAuth keys ==="
openssl genrsa -out ~/kruxiaflow-private.pem 2048
openssl rsa -in ~/kruxiaflow-private.pem -pubout -out ~/kruxiaflow-public.pem
chmod 600 ~/kruxiaflow-private.pem

echo "=== Creating environment file ==="
sudo mkdir -p /etc/kruxiaflow
CLIENT_SECRET=$(openssl rand -hex 32)
sudo tee /etc/kruxiaflow/env > /dev/null <<EOF
DATABASE_URL=postgres://kruxiaflow:changeme@localhost:5432/kruxiaflow
KRUXIAFLOW_API_PORT=8080
KRUXIAFLOW_WORKER_MAX_ACTIVITIES=2
KRUXIAFLOW_WORKER_POLL_MAX_ACTIVITIES=1
KRUXIAFLOW_DB_MAX_CONNECTIONS=15
KRUXIAFLOW_DB_MIN_CONNECTIONS=2
KRUXIAFLOW_CLIENT_ID=kruxiaflow_internal
KRUXIAFLOW_CLIENT_SECRET=$CLIENT_SECRET
KRUXIAFLOW_OAUTH_RSA_PRIVATE_KEY_PEM_FILE=$HOME/kruxiaflow-private.pem
KRUXIAFLOW_OAUTH_RSA_PUBLIC_KEY_PEM_FILE=$HOME/kruxiaflow-public.pem
KRUXIAFLOW_LOG_LEVEL=info
EOF
sudo chmod 600 /etc/kruxiaflow/env

echo "=== Installing kruxiaflow binary ==="
# Assumes binary is already copied to ~/kruxiaflow
sudo mv ~/kruxiaflow /usr/local/bin/
sudo chmod +x /usr/local/bin/kruxiaflow

echo "=== Running migrations ==="
source /etc/kruxiaflow/env
kruxiaflow migrate

echo "=== Creating systemd service ==="
sudo tee /etc/systemd/system/kruxiaflow.service > /dev/null <<'EOF'
[Unit]
Description=Kruxia Flow
After=network.target postgresql.service
Requires=postgresql.service

[Service]
Type=simple
User=pi
EnvironmentFile=/etc/kruxiaflow/env
ExecStart=/usr/local/bin/kruxiaflow serve --migrate --seed-client
Restart=on-failure
RestartSec=10
MemoryMax=150M

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable kruxiaflow
sudo systemctl start kruxiaflow

echo "=== Setup complete! ==="
echo "Check status: sudo systemctl status kruxiaflow"
echo "View logs: journalctl -u kruxiaflow -f"
echo "API: http://$(hostname -I | awk '{print $1}'):8080/health"
```

---

## See Also

- [Architecture Documentation](architecture.md) - System design overview
- [Raspberry Pi Demo Concepts](demos/raspberry-pi-zero-2w-demos.md) - Demo scenarios
- [Contributing Guide](../CONTRIBUTING.md) - Development setup
