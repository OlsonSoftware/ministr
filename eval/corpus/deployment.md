# Deployment Guide

## Container Build

The application is packaged as a multi-stage Docker image. The build stage compiles from source using the official Rust image. The runtime stage uses distroless for a minimal attack surface.

Build arguments:
- `RUST_VERSION`: Compiler version (default: 1.85)
- `FEATURES`: Cargo feature flags (default: empty)
- `PROFILE`: Build profile (default: release)

## Kubernetes Deployment

### Pod Configuration

Each pod runs a single application container with resource limits:
- CPU request: 250m, limit: 1000m
- Memory request: 256Mi, limit: 512Mi

Liveness probe: HTTP GET `/health/live` every 10 seconds.
Readiness probe: HTTP GET `/health/ready` every 5 seconds with a 30-second initial delay.

### Horizontal Pod Autoscaler

The HPA scales between 2 and 20 replicas based on CPU utilization. The target CPU utilization is 70%. Scale-up cooldown is 60 seconds; scale-down cooldown is 300 seconds.

## Configuration Management

Configuration is loaded from environment variables with optional override from a TOML config file. Secrets are injected via Kubernetes secrets mounted as environment variables.

Configuration precedence (highest to lowest):
1. Environment variables
2. Config file (`/etc/app/config.toml`)
3. Default values compiled into the binary

## Rolling Updates

Deployments use a rolling update strategy with maxSurge=1 and maxUnavailable=0. This ensures zero downtime during releases.

Canary deployments are supported via Argo Rollouts with a 5-minute analysis window before full promotion.

## Monitoring

Application metrics are exposed on `/metrics` in Prometheus format. Key metrics:
- `http_request_duration_seconds` — request latency histogram
- `http_requests_total` — request counter by status code
- `db_pool_active_connections` — database connection pool gauge
