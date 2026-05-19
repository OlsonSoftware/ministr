// Custom domain binding for `mcp.ministr.ai`.
//
// Two-step process Pulumi expresses declaratively:
//   1. Create a Managed Certificate on the ACA environment (free, ACA
//      handles renewal). Requires a CNAME `<domain>` → `<env-default-domain>`
//      pre-existing in DNS so ACA can validate ownership.
//   2. Update the Container App's ingress.customDomains[] to bind the
//      domain to the cert.
//
// On a fresh deployment the cert provisioning takes ~5 min. If the CNAME
// isn't in place, `pulumi up` will hang on the cert resource — that's the
// expected failure mode pointing at missing DNS.

import * as pulumi from "@pulumi/pulumi";
import * as app from "@pulumi/azure-native/app";
import * as resources from "@pulumi/azure-native/resources";

import { location, named } from "./naming";

export interface DomainInputs {
  rg: resources.ResourceGroup;
  env: app.ManagedEnvironment;
  containerApp: app.ContainerApp;
  domain: string;
}

export function bindCustomDomain({ rg, env, containerApp, domain }: DomainInputs) {
  const cert = new app.ManagedCertificate(
    named("cert"),
    {
      resourceGroupName: rg.name,
      environmentName: env.name,
      managedCertificateName: domain.replace(/\./g, "-"),
      location,
      properties: {
        subjectName: domain,
        domainControlValidation: "CNAME",
      },
    },
    { dependsOn: [containerApp] },
  );

  // ContainerApp.update with customDomains is not a separate Pulumi
  // resource; the binding is expressed via `configuration.ingress.customDomains`
  // on the ContainerApp itself. Callers wire this in by passing the cert ID
  // back into the app config. See README for the two-pass approach.
  // Pulumi cleanly handles this via .apply on `cert.id`:
  return pulumi.all([cert.id, containerApp.id]).apply(() => ({
    certId: cert.id,
    domain,
  }));
}
