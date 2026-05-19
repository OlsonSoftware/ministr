// ministr.ai cloud — top-level composition.
//
// SOLID: this file orchestrates; each `lib/*.ts` builds one cohesive
// resource group. Resource dependencies flow one direction: networking →
// registry / storage / insights → app / job → domain.

import * as pulumi from "@pulumi/pulumi";

import { createNetworking } from "./lib/networking";
import { createRegistry } from "./lib/registry";
import { createStorage } from "./lib/storage";
import { createInsights } from "./lib/insights";
import { createApp } from "./lib/app";
import { createIndexerJob } from "./lib/job";
import { bindCustomDomain } from "./lib/domain";

const cfg = new pulumi.Config();
const imageTag = cfg.get("imageTag") ?? "latest";
const customDomain = cfg.get("customDomain") ?? "";
const appCpu = cfg.get("appCpu") ?? "0.5";
const appMemory = cfg.get("appMemory") ?? "1Gi";
const jobCpu = cfg.get("jobCpu") ?? "4";
const jobMemory = cfg.get("jobMemory") ?? "8Gi";
// Colon-separated paths the container should index. Default to the
// `corpus/` subdir of the Azure Files mount; the operator drops repos
// there (or a v2 admin endpoint clones into it).
const corpusPaths = cfg.get("corpusPaths") ?? "/data/corpus";
const webhookSecret = cfg.getSecret("githubWebhookSecret");

const net = createNetworking();
const registry = createRegistry({ rg: net.rg });
const storage = createStorage({ rg: net.rg, env: net.env });
const insights = createInsights({ rg: net.rg, workspace: net.workspace });

const queryApp = createApp({
  rg: net.rg,
  env: net.env,
  registry,
  storage,
  insights,
  imageTag,
  cpu: appCpu,
  memory: appMemory,
  webhookSecret,
  corpusPaths,
});

const indexer = createIndexerJob({
  rg: net.rg,
  env: net.env,
  registry,
  storage,
  imageTag,
  cpu: jobCpu,
  memory: jobMemory,
  corpusPaths,
});

const domainBinding = customDomain
  ? bindCustomDomain({
      rg: net.rg,
      env: net.env,
      containerApp: queryApp.containerApp,
      domain: customDomain,
    })
  : undefined;

export const resourceGroup = net.rg.name;
export const registryServer = registry.loginServer;
export const storageAccount = storage.accountName;
export const appFqdn = queryApp.fqdn;
export const indexerJobName = indexer.name;
export const customDomainConfigured = customDomain || "(none)";
export const customDomainCertId = domainBinding?.apply((d) => d.certId);
