/**
 * Single source of truth for the pricing matrix.
 *
 * Both the `/pricing` page and any landing-page snippet read tiers
 * from this module. Changes go HERE — never duplicate per-page.
 */

export interface Tier {
  /** Stable slug used in URLs (`?from=pro`) and as the React key. */
  slug: 'local' | 'pro' | 'team' | 'enterprise';
  /** Display name. */
  name: string;
  /** Price headline (string so we can carry units + footnote markers). */
  price: string;
  /** Per-row tagline. */
  tagline: string;
  /** Bullet points the customer sees on the pricing card. */
  bullets: string[];
  /** Call-to-action label + href. `href` may be relative (`/billing/...`)
   *  or absolute (`https://github.com/...`). */
  cta: { label: string; href: string };
  /** When `true`, the card is rendered with the accent colour to mark
   *  the recommended tier. Currently Pro. */
  highlighted?: boolean;
}

export const TIERS: readonly Tier[] = [
  {
    slug: 'local',
    name: 'Local',
    price: 'Free · MIT',
    tagline:
      'Everything that runs on your machine. Forever free, never demoted to a paid tier.',
    bullets: [
      'All 20 MCP tools',
      'SOLID detector + 13 cross-language bridge detectors',
      '~40 language parsers',
      'Unlimited corpora on your disk',
      'Private repos via PAT-in-URL',
      'OAuth issuer included for self-hosted serve',
    ],
    cta: { label: 'Install ministr', href: '/install' },
  },
  {
    slug: 'pro',
    name: 'Pro',
    price: '$20 / month',
    tagline:
      'Hosted code intelligence for solo devs. The polyglot index and Atlas network, managed for you.',
    bullets: [
      '10 hosted corpora',
      'Shared fast-lane indexing (≤2 min queue p95)',
      'Private repos via GitHub App (no PAT)',
      'Unlimited Atlas reads',
      'Cost + latency badges in the desktop app',
    ],
    cta: { label: 'Start Pro', href: '/billing/upgrade?from=pro' },
    highlighted: true,
  },
  {
    slug: 'team',
    name: 'Team',
    price: '$30 / seat / mo',
    tagline:
      '3-seat minimum ($90/mo floor). Orgs, ACL, named API keys, webhooks, audit.',
    bullets: [
      '50 corpora per org',
      'Priority queue (jumps Pro)',
      'Atlas + private annotation overlays',
      'Orgs, ACL, dashboard, named API keys',
      'Slack / Discord webhooks',
      'Bridge graph web visualizer',
      'Audit-light (90-day retention)',
    ],
    cta: { label: 'Start Team', href: '/billing/upgrade?from=team' },
  },
  {
    slug: 'enterprise',
    name: 'Enterprise',
    price: 'Contact us',
    tagline:
      'SSO, immutable audit, on-prem image, customer-managed keys. For compliance-bound teams.',
    bullets: [
      'Unlimited corpora + dedicated indexing pool',
      'SSO / SAML, OIDC federation',
      'Immutable audit log + SIEM export',
      'On-prem (Helm + Docker Compose), license-key gated',
      'In-VPC Atlas mirror',
      'Customer-managed encryption keys (Azure Key Vault BYOK)',
      'SLA: 99.5% uptime, p95 ≤200ms',
    ],
    cta: { label: 'Contact sales', href: 'mailto:sales@ministr.ai' },
  },
];

/** Positioning one-liner for the landing page. */
export const POSITIONING_LINE =
  'Hosted, polyglot, agent-aware code intelligence — the MCP layer every AI agent calls into. MIT core, paid cloud.';
