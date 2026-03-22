# Error Handling Guide

## Error Types

The application defines a hierarchy of error types using Rust's enum pattern. Each module has its own error enum that implements `std::error::Error` via the `thiserror` crate.

### Application Errors

Application errors are user-facing and include structured error codes. Each error code maps to an HTTP status code and a human-readable message.

Common error codes:
- `AUTH_001`: Invalid credentials
- `AUTH_002`: Token expired
- `AUTH_003`: Insufficient permissions
- `DATA_001`: Resource not found
- `DATA_002`: Conflict (duplicate resource)
- `RATE_001`: Rate limit exceeded

### Infrastructure Errors

Infrastructure errors represent failures in backing services (database, cache, external APIs). These are never exposed directly to users. Instead, they are mapped to generic 500 responses with a correlation ID.

## Error Propagation

Errors propagate upward through the call stack using the `?` operator. Each layer adds context using `.context()` from the `anyhow` crate.

Service layer errors are converted to transport-layer responses at the handler boundary. This is the only place where error-to-HTTP mapping occurs.

## Retry Policy

Transient errors (network timeouts, connection resets) are retried with exponential backoff. The default policy retries 3 times with a base delay of 100ms and a maximum delay of 5 seconds.

Non-transient errors (validation failures, authentication errors) are never retried.

## Observability

All errors are logged with structured fields including error type, message, correlation ID, and stack trace. Error rates are tracked as Prometheus counters with labels for error type and endpoint.

Alerts fire when the error rate exceeds 1% of total requests sustained over a 5-minute window.
