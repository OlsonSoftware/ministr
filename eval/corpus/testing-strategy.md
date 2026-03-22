# Testing Strategy

## Test Pyramid

The test suite follows the test pyramid model with three layers:

### Unit Tests

Unit tests cover individual functions and methods in isolation. They live alongside source code in `#[cfg(test)]` modules. Coverage target: 80% line coverage.

Dependencies are injected via trait objects. External services are replaced with in-memory fakes, not mocks. This ensures tests remain fast and deterministic.

### Integration Tests

Integration tests verify interactions between components using real backing services. They live in the `tests/` directory and each test gets a fresh database via test containers.

Key integration scenarios:
- Full request lifecycle (HTTP → service → database → response)
- Authentication flow including token refresh
- Rate limiting behavior under concurrent load
- Database migration up and down cycles

### End-to-End Tests

End-to-end tests exercise the complete application from an external client perspective. They run against a deployed staging environment and verify critical user journeys.

E2E tests are tagged with `#[ignore]` and only run in CI on the staging environment.

## Property-Based Testing

Critical data transformation functions use property-based testing via the `proptest` crate. Properties tested:
- Serialization round-trips (encode → decode = identity)
- Idempotent operations produce the same result when applied twice
- Sort stability and ordering invariants

## Performance Testing

Load tests use k6 to simulate realistic traffic patterns. Baseline performance is captured after each release. Regressions exceeding 10% trigger an alert.

Benchmark tests for hot paths use Criterion.rs with statistical significance testing. Benchmarks run nightly in CI and results are tracked over time.

## Continuous Integration

The CI pipeline runs on every pull request:
1. Format check (`cargo fmt --check`)
2. Lint (`cargo clippy -- -D warnings`)
3. Unit tests (`cargo test`)
4. Integration tests (`cargo test -- --ignored`)
5. Coverage report upload
