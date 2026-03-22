# Database Design

## Schema Conventions

All tables use UUID primary keys generated at the application layer. Timestamps are stored as UTC ISO 8601 strings. Column names follow snake_case convention.

## Migration Strategy

Database migrations are managed with a numbered migration system. Each migration is an idempotent SQL script stored in the `migrations/` directory. Migrations run automatically on application startup.

Rollback scripts are required for every migration. The rollback naming convention is `NNNN_down.sql` corresponding to `NNNN_up.sql`.

## Connection Pooling

The application uses a connection pool with configurable min/max connections. Default pool size is 10 connections with a 30-second idle timeout. Connections are validated with a lightweight health check before use.

## Indexing Strategy

Primary lookup patterns drive index design. Every foreign key column has a corresponding index. Composite indexes follow the leftmost prefix rule.

Full-text search uses PostgreSQL's tsvector with GIN indexes. Search queries support stemming, phrase matching, and prefix matching.

## Query Patterns

All queries use parameterized statements to prevent SQL injection. Bulk operations use batch inserts with ON CONFLICT clauses for upsert semantics.

Read replicas handle analytics and reporting queries. Write operations are always directed to the primary database. Replication lag is monitored and queries that require strong consistency are pinned to the primary.
