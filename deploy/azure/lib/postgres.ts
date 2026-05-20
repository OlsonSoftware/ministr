// Azure Database for PostgreSQL Flexible Server — Burstable B1ms tier.
//
// Tenancy + billing + OAuth durability (F1.2, F1.5, ministr-mcp's
// OAuthBackend::Postgres) all need a real database. SQLite on the Azure
// Files mount survives single-replica restarts but doesn't scale across
// pods, and Azure Files SMB can't host WAL. B1ms is the entry tier for
// the cloud's hot data (auth state, jobs, tenants); HNSW indexes still
// live in blob (see F1.1 sub-bullet 5).
//
// SRP: this file provisions the Postgres server, its admin firewall
// rule, and the `ministr` database. The connection string is emitted as
// an output for downstream apps to read; this file does not wire it
// into the App or Indexer containers (that's app.ts / job.ts in F1.2).
//
// Opt-in via index.ts: createPostgres() is invoked only when
// `enablePostgres=true` in Pulumi config, so adding this module to the
// repo doesn't provision a server until the rest of F1.1 is ready.

import * as pulumi from "@pulumi/pulumi";
import * as resources from "@pulumi/azure-native/resources";
import * as pg from "@pulumi/azure-native/dbforpostgresql";
import * as pgEnums from "@pulumi/azure-native/types/enums/dbforpostgresql";

import { location as defaultLocation, named } from "./naming";

export interface PostgresArtifact {
  server: pg.Server;
  database: pg.Database;
  /** Fully-qualified domain name (`<name>.postgres.database.azure.com`). */
  host: pulumi.Output<string>;
  /**
   * Libpq connection URI suitable for sqlx / tokio-postgres /
   * any standard Postgres client:
   * `postgres://<user>:<password>@<host>:5432/ministr?sslmode=require`.
   *
   * Marked as a Pulumi secret because it embeds the admin password.
   */
  pgConnectionString: pulumi.Output<string>;
}

export interface PostgresInputs {
  rg: resources.ResourceGroup;
  /** Admin login name (e.g. "ministradmin"). Cannot be changed after creation. */
  adminLogin: string;
  /** Admin password — must be a Pulumi secret. */
  adminPassword: pulumi.Output<string>;
  /**
   * Azure region for the Postgres server. May differ from the global
   * `location` because some subscriptions are restricted from
   * provisioning Postgres Flex Burstable in certain regions
   * (`LocationIsOfferRestricted`). Cross-region latency between ACA
   * and Postgres is single-digit ms within the same continent.
   */
  pgLocation?: string;
}

export function createPostgres({
  rg,
  adminLogin,
  adminPassword,
  pgLocation,
}: PostgresInputs): PostgresArtifact {
  const location = pgLocation ?? defaultLocation;
  // Suffix the server name with the location. Azure's resource-provider
  // metadata caches the location of a previously-named server even after
  // the resource itself is gone, so picking a fresh name lets us
  // re-create cleanly in a different region after a failed attempt.
  const serverName = pgLocation ? named(`pg-${pgLocation}`) : named("pg-prod");
  const server = new pg.Server(named("pg"), {
    resourceGroupName: rg.name,
    serverName,
    location,
    // PostgreSQL 17 — current stable; major-version upgrade path available
    // via concepts-major-version-upgrade. 16 also acceptable. Avoid 18
    // until ecosystem (sqlx, pgbouncer) catches up.
    version: pgEnums.PostgresMajorVersion.PostgresMajorVersion_17,
    // Burstable B1ms: 1 vCore, 2 GB RAM. Cheapest production-supported
    // tier on Flex. CPU credits cover bursty OAuth + job-queue + audit
    // writes; tenant data lives here, hot path is small.
    sku: {
      name: "Standard_B1ms",
      tier: pgEnums.SkuTier.Burstable,
    },
    // 32 GB is the minimum for B1ms. Auto-grow guards against silent
    // out-of-space during F4.1 5K-repo cron expansion of usage_events /
    // audit_events rows.
    storage: {
      storageSizeGB: 32,
      autoGrow: pgEnums.StorageAutogrow.Enabled,
      type: pgEnums.StorageType.Premium_LRS,
    },
    backup: {
      backupRetentionDays: 7,
      geoRedundantBackup: pgEnums.GeoRedundantBackup.Disabled,
    },
    // Burstable does not support HA. Disabled is the only valid setting
    // for this SKU; promote to ZoneRedundant on a GeneralPurpose tier
    // before F5 Enterprise sales.
    highAvailability: {
      mode: pgEnums.PostgreSqlFlexibleServerHighAvailabilityMode.Disabled,
    },
    administratorLogin: adminLogin,
    administratorLoginPassword: adminPassword,
    // No customer VNet → public network + firewall rules below. The ACA
    // pool reaches the server through the Azure backbone; the
    // "AllowAzureServices" rule plus TLS gates traffic to the same
    // tenant's resources. Revisit when F5.6 ships CMK + VNet injection.
    network: {
      publicNetworkAccess: pgEnums.PublicNetworkAccessEnum.Enabled,
    },
  });

  // Allow access from Azure-internal services (Container Apps, Container
  // Apps Jobs, App Insights egress). The 0.0.0.0/0.0.0.0 sentinel is the
  // documented Azure pattern, NOT a public-internet rule — Azure
  // intercepts it as "any Azure-resident IP". See:
  //   learn.microsoft.com/azure/postgresql/flexible-server/concepts-firewall-rules
  new pg.FirewallRule(
    named("pg-fw-azure"),
    {
      resourceGroupName: rg.name,
      serverName: server.name,
      firewallRuleName: "AllowAllAzureServicesAndResourcesWithinAzureIps",
      startIpAddress: "0.0.0.0",
      endIpAddress: "0.0.0.0",
    },
    { dependsOn: [server] },
  );

  const database = new pg.Database(
    named("pg-db"),
    {
      resourceGroupName: rg.name,
      serverName: server.name,
      databaseName: "ministr",
      // UTF-8 matches every client we ship (sqlx, tokio-postgres,
      // psql via just-recipe).
      charset: "UTF8",
      collation: "en_US.utf8",
    },
    { dependsOn: [server] },
  );

  const host = server.fullyQualifiedDomainName;
  const pgConnectionString = pulumi.secret(
    pulumi
      .all([host, adminPassword])
      .apply(
        ([h, pw]) =>
          `postgres://${encodeURIComponent(adminLogin)}:${encodeURIComponent(pw)}@${h}:5432/ministr?sslmode=require`,
      ),
  );

  return { server, database, host, pgConnectionString };
}
