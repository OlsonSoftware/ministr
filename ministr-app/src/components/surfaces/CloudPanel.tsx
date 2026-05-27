/**
 * CloudPanel — settings UI for the `mcp.ministr.ai` remote MCP connection.
 *
 * v1 scope: endpoint configuration, manual Bearer-token entry, live
 * `/healthz` probe. The full OAuth deep-link flow + SSE indexer events
 * are deliberate follow-ups; this surface is the slot they land in.
 *
 * SOLID note: the panel is purely a renderer over [`cloudClient`]
 * (`src/lib/cloudClient.ts`). All Tauri ↔ HTTP plumbing lives there.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowRight,
  BarChart3,
  Check,
  ChevronDown,
  ChevronRight,
  CloudOff,
  Copy,
  CreditCard,
  Database,
  Download,
  Eye,
  GitBranch,
  Key,
  Link,
  Loader2,
  LogIn,
  Plus,
  RefreshCw,
  Send,
  Share2,
  ShieldAlert,
  Trash2,
  TrendingUp,
  Webhook,
  X,
} from "lucide-react";

// Inline GitHub Octocat mark. Lucide-react doesn't ship brand logos
// (intentionally vendor-neutral) so the icon for the federated sign-in
// button is a hand-trimmed copy of the canonical Octocat SVG. Mirrors
// the GitHub Logos usage guidelines: monochrome, no modifications
// beyond stroke/colour, and only on buttons that initiate a GitHub
// auth flow.
function GitHubMark({ className }: { className?: string }) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden="true"
      className={className}
    >
      <path d="M12 .5C5.65.5.5 5.65.5 12c0 5.08 3.29 9.39 7.86 10.91.58.11.79-.25.79-.55 0-.27-.01-1.17-.02-2.13-3.2.7-3.87-1.36-3.87-1.36-.52-1.32-1.27-1.67-1.27-1.67-1.04-.71.08-.7.08-.7 1.15.08 1.76 1.18 1.76 1.18 1.02 1.75 2.68 1.25 3.34.96.1-.74.4-1.25.73-1.54-2.55-.29-5.24-1.28-5.24-5.7 0-1.26.45-2.29 1.18-3.1-.12-.29-.51-1.45.11-3.02 0 0 .96-.31 3.15 1.18.91-.25 1.88-.38 2.85-.39.97.01 1.94.14 2.85.39 2.18-1.49 3.15-1.18 3.15-1.18.62 1.57.23 2.73.11 3.02.74.81 1.18 1.84 1.18 3.1 0 4.44-2.7 5.41-5.27 5.69.41.36.78 1.06.78 2.14 0 1.55-.01 2.8-.01 3.18 0 .31.21.67.8.55C20.21 21.38 23.5 17.08 23.5 12 23.5 5.65 18.35.5 12 .5z" />
    </svg>
  );
}

import { Button } from "../ui/button";
import { ContentTray } from "../ui/content-tray";
import { ErrorCallout } from "../ui/error-callout";
import { SurfaceSidebar, type SidebarItem } from "../ui/surface-sidebar";
import { ConfirmDialog } from "../ui/confirm-dialog";
import { OnboardingWizard } from "../onboarding/OnboardingWizard";
import { useDialog } from "../../hooks/useDialog";
import {
  cloudClient,
  type CloudAclEntry,
  type CloudApiKey,
  type CloudCorpusInfo,
  type CloudCreatedApiKey,
  type CloudCreatedWebhookSub,
  type CloudHealth,
  type CloudOrg,
  type CloudOrgUsage,
  type CloudProgressEvent,
  type CloudSessionBundle,
  type CloudSessionSummary,
  type CloudStatus,
  type CloudUsage,
  type CloudWebhookSub,
  type CloudWebhookTestResult,
} from "../../lib/cloudClient";
import { cn } from "../../lib/utils";

const DEFAULT_ENDPOINT = "https://mcp.ministr.ai";

const CLOUD_NAV_FULL: readonly SidebarItem[] = [
  { id: "connection", label: "Connection", icon: Link },
  { id: "corpora", label: "Corpora", icon: Database },
  { id: "api-keys", label: "API Keys", icon: Key },
  { id: "webhooks", label: "Webhooks", icon: Webhook },
  { id: "usage", label: "Usage", icon: BarChart3 },
  { id: "sessions", label: "Sessions", icon: Eye },
];

// Sign-in gate: visitors who are not signed in to ministr Cloud only
// see the Connection sub-section. The full feature surface (Corpora,
// API Keys, Webhooks, Usage, Sessions) is hidden until they authenticate.
// The Cloud icon in the nav rail stays visible as the entry point.
const CLOUD_NAV_GATED: readonly SidebarItem[] = [
  { id: "connection", label: "Connection", icon: Link },
];

export function CloudPanel() {
  const [activeSection, setActiveSection] = useState("connection");
  const [status, setStatus] = useState<CloudStatus | null>(null);
  const [endpointDraft, setEndpointDraft] = useState("");
  const [tokenDraft, setTokenDraft] = useState("");
  const [health, setHealth] = useState<CloudHealth | null>(null);
  const [healthError, setHealthError] = useState<string | null>(null);
  const [busy, setBusy] = useState<
    null
    | "save-endpoint"
    | "save-token"
    | "sign-in"
    | "sign-in-github"
    | "probe"
    | "disconnect"
    | "manage-billing"
    | "upgrade"
  >(null);
  const [signInError, setSignInError] = useState<string | null>(null);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [usage, setUsage] = useState<CloudUsage | null>(null);

  const refreshUsage = useCallback(async () => {
    try {
      const u = await cloudClient.billingUsage();
      setUsage(u);
    } catch {
      // Endpoint absent (self-hosted) or auth missing — keep badges hidden.
      setUsage(null);
    }
  }, []);

  const refreshStatus = useCallback(async () => {
    const s = await cloudClient.status();
    setStatus(s);
    setEndpointDraft(s.endpoint || DEFAULT_ENDPOINT);
  }, []);

  useEffect(() => {
    void refreshStatus();
  }, [refreshStatus]);

  useEffect(() => {
    if (status?.authenticated) {
      void refreshUsage();
    } else {
      setUsage(null);
    }
  }, [status?.authenticated, refreshUsage]);

  // F2.7 — top-level corpora count probe, used by the OnboardingWizard
  // to mark step 4 complete. The `CorporaSection` further down owns
  // its own corpora list; this probe is read-only and intentionally
  // sparse (one fetch per auth flip) so the wizard's signal stays
  // accurate without duplicating CorporaSection's reactive loop.
  const [corporaCount, setCorporaCount] = useState<number | null>(null);
  useEffect(() => {
    if (!status?.authenticated) {
      setCorporaCount(null);
      return;
    }
    let cancelled = false;
    void cloudClient
      .listCorpora()
      .then((list) => {
        if (!cancelled) setCorporaCount(list.length);
      })
      .catch(() => {
        if (!cancelled) setCorporaCount(null);
      });
    return () => {
      cancelled = true;
    };
  }, [status?.authenticated]);

  const onSaveEndpoint = async () => {
    setBusy("save-endpoint");
    try {
      await cloudClient.setEndpoint(endpointDraft);
      await refreshStatus();
    } finally {
      setBusy(null);
    }
  };

  const onSaveToken = async () => {
    setBusy("save-token");
    try {
      await cloudClient.setBearerToken(tokenDraft);
      setTokenDraft("");
      await refreshStatus();
    } finally {
      setBusy(null);
    }
  };

  const onSignIn = async () => {
    setBusy("sign-in");
    setSignInError(null);
    try {
      // Make sure the endpoint is saved first — otherwise the OAuth flow
      // would target whatever the persisted endpoint is, which may not
      // match what the user just typed.
      if (endpointDraft.trim() && endpointDraft !== status?.endpoint) {
        await cloudClient.setEndpoint(endpointDraft);
      }
      await cloudClient.authenticate();
      await refreshStatus();
    } catch (e) {
      setSignInError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const onSignInGitHub = async () => {
    setBusy("sign-in-github");
    setSignInError(null);
    try {
      if (endpointDraft.trim() && endpointDraft !== status?.endpoint) {
        await cloudClient.setEndpoint(endpointDraft);
      }
      await cloudClient.authenticateGitHub();
      await refreshStatus();
    } catch (e) {
      setSignInError(String(e));
    } finally {
      setBusy(null);
    }
  };

  // F2.4 — open the Stripe Customer Portal in the system browser.
  // The cloud handler validates the bearer token and mints a portal
  // session bound to the user's stripe_customer_id; the URL is
  // short-lived and single-use.
  const onManageBilling = async () => {
    setBusy("manage-billing");
    setSignInError(null);
    try {
      await cloudClient.billingPortal();
    } catch (e) {
      setSignInError(String(e));
    } finally {
      setBusy(null);
    }
  };

  // F2.4 — start a Stripe Checkout flow for the target plan. The user
  // pays in Stripe-hosted UI; the cloud webhook flips
  // `users.plan_id` once payment completes.
  const onUpgrade = async (plan: "pro" | "team") => {
    setBusy("upgrade");
    setSignInError(null);
    try {
      await cloudClient.billingCheckout(plan);
    } catch (e) {
      setSignInError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const onProbe = async () => {
    setBusy("probe");
    setHealthError(null);
    try {
      const h = await cloudClient.healthCheck();
      setHealth(h);
      // Probing /healthz is also a natural moment to refresh
      // billable-usage counters, so the badges stay live.
      if (status?.authenticated) {
        void refreshUsage();
      }
    } catch (e) {
      setHealth(null);
      setHealthError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const onDisconnect = async () => {
    setBusy("disconnect");
    try {
      await cloudClient.disconnect();
      setHealth(null);
      setHealthError(null);
      await refreshStatus();
    } finally {
      setBusy(null);
    }
  };

  const authenticated = !!status?.authenticated;
  const navItems = authenticated ? CLOUD_NAV_FULL : CLOUD_NAV_GATED;
  // If the user signs out while viewing a gated section, snap back to Connection.
  const visibleSection = authenticated ? activeSection : "connection";

  return (
    <SurfaceSidebar
      title="Cloud"
      items={navItems}
      active={visibleSection}
      onSelect={setActiveSection}
    >
      {visibleSection === "connection" && (
      <div className="flex flex-col gap-6">

      <OnboardingWizard
        signals={{
          authenticated: status?.authenticated ?? false,
          plan: usage?.plan ?? null,
          hasCorpus: (corporaCount ?? 0) > 0,
          // F2.7 v0 — keyed off whether the user has ever submitted
          // an installation ID in the clone dialog. Persisted to
          // localStorage by the clone flow when an ID is supplied;
          // read here so dismissing the wizard sticks.
          hasGithubAppInstallation:
            typeof window !== "undefined" &&
            window.localStorage.getItem("ministr.github_app.installation_seen") === "1",
        }}
        handlers={{
          onSignInGitHub: onSignInGitHub,
          onUpgradePro: () => onUpgrade("pro"),
          onInstallGitHubApp: async () => {
            // The "Install GitHub App" deep-link lives on github.com;
            // open it in the system browser. Stripe Customer Portal
            // and onboarding both use the same `open_url` path on the
            // Tauri side, so a fresh window opens without leaving
            // the app.
            window.open(
              "https://github.com/apps/ministr/installations/new",
              "_blank",
              "noopener,noreferrer",
            );
            window.localStorage.setItem("ministr.github_app.installation_seen", "1");
          },
          // F2.7 — scroll to the corpora section and signal it to
          // open the clone dialog. CorporaSection owns its own
          // cloneOpen state; we communicate via a custom event so we
          // don't have to lift that state out.
          onCloneFirstRepo: () => {
            window.dispatchEvent(new CustomEvent("ministr.cloud.open-clone"));
          },
        }}
      />

      <ContentTray className="space-y-5 !p-5">
      <section className="flex flex-col gap-3">
        <label className="flex flex-col gap-1.5">
          <span className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
            Endpoint
          </span>
          <input
            type="url"
            value={endpointDraft}
            onChange={(e) => setEndpointDraft(e.target.value)}
            placeholder={DEFAULT_ENDPOINT}
            className="h-9 px-3 rounded-md border border-border bg-surface font-mono text-sm text-text focus:outline-none focus:border-border-hover"
          />
        </label>
        <div className="flex gap-2">
          <Button
            size="sm"
            onClick={onSaveEndpoint}
            disabled={busy === "save-endpoint" || endpointDraft === (status?.endpoint ?? "")}
          >
            {busy === "save-endpoint" ? <Loader2 className="size-3.5 animate-spin" /> : null}
            Save endpoint
          </Button>
          <Button size="sm" variant="ghost" onClick={refreshStatus}>
            <RefreshCw className="size-3.5" />
            Reload
          </Button>
        </div>
      </section>

      <section className="flex flex-col gap-3">
        <div className="flex items-center justify-between">
          <span className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
            Authentication
          </span>
          {status?.authenticated && (
            <div className="flex items-center gap-2 text-xs font-mono">
              {usage?.plan && <PlanBadge plan={usage.plan} />}
              <span className="text-accent flex items-center gap-1">
                <Check className="size-3.5" /> signed in
              </span>
            </div>
          )}
        </div>
        <div className="flex gap-2 flex-wrap">
          <Button
            size="sm"
            onClick={onSignInGitHub}
            disabled={busy === "sign-in-github" || busy === "sign-in"}
          >
            {busy === "sign-in-github" ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <GitHubMark className="size-3.5" />
            )}
            {status?.authenticated ? "Re-sign in with GitHub" : "Sign in with GitHub"}
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={onSignIn}
            disabled={busy === "sign-in" || busy === "sign-in-github"}
          >
            {busy === "sign-in" ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <LogIn className="size-3.5" />
            )}
            Use OAuth (self-hosted)
          </Button>
        </div>
        <p className="text-xs text-text-muted">
          {"Sign in with GitHub"} opens your browser, federates the
          sign-in through the cloud's GitHub OAuth App, and stores the
          resulting bearer token in your OS keychain. The OAuth fallback
          is for self-hosted deployments where the cloud is not
          configured with GitHub credentials. Either flow times out after
          3 minutes.
        </p>
        {signInError && <ErrorCallout message={signInError} />}

        {status?.authenticated && (
          <div className="flex gap-2 flex-wrap pt-1 border-t border-border-soft mt-1">
            <Button
              size="sm"
              variant="ghost"
              onClick={() => void onManageBilling()}
              disabled={busy === "manage-billing"}
            >
              {busy === "manage-billing" ? (
                <Loader2 className="size-3.5 animate-spin" />
              ) : (
                <CreditCard className="size-3.5" />
              )}
              Manage billing
            </Button>
            {usage?.plan === "pro" && (
              <Button
                size="sm"
                variant="ghost"
                onClick={() => void onUpgrade("team")}
                disabled={busy === "upgrade"}
              >
                {busy === "upgrade" ? (
                  <Loader2 className="size-3.5 animate-spin" />
                ) : null}
                Upgrade to Team
              </Button>
            )}
          </div>
        )}

        <button
          type="button"
          onClick={() => setAdvancedOpen((v) => !v)}
          className="flex items-center gap-1.5 text-xs text-text-muted hover:text-text transition-colors self-start"
        >
          {advancedOpen ? (
            <ChevronDown className="size-3.5" />
          ) : (
            <ChevronRight className="size-3.5" />
          )}
          Advanced: paste token manually
        </button>
        {advancedOpen && (
          <div className="flex flex-col gap-2 border-l border-border-soft pl-3 ml-1">
            <label className="flex flex-col gap-1.5">
              <span className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
                Bearer token
              </span>
              <input
                type="password"
                value={tokenDraft}
                onChange={(e) => setTokenDraft(e.target.value)}
                placeholder={
                  status?.authenticated
                    ? "•••••••• (token saved — type to replace)"
                    : "Paste a token from any OAuth flow"
                }
                className="h-9 px-3 rounded-md border border-border bg-surface font-mono text-sm text-text focus:outline-none focus:border-border-hover"
              />
            </label>
            <div className="flex gap-2">
              <Button
                size="sm"
                variant="outline"
                onClick={onSaveToken}
                disabled={busy === "save-token" || tokenDraft.trim() === ""}
              >
                {busy === "save-token" ? <Loader2 className="size-3.5 animate-spin" /> : null}
                Save token
              </Button>
            </div>
            <p className="text-xs text-text-muted flex items-start gap-1.5">
              <ShieldAlert className="size-3.5 mt-0.5 shrink-0" />
              OS-keychain storage is a v2 hardening step.
            </p>
          </div>
        )}
      </section>

      <section className="flex flex-col gap-3 border-t border-border-soft pt-4">
        <div className="flex items-center justify-between">
          <h3 className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
            Connection
          </h3>
          <Button
            size="sm"
            variant="outline"
            onClick={onProbe}
            disabled={busy === "probe" || !status?.configured}
          >
            {busy === "probe" ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <RefreshCw className="size-3.5" />
            )}
            Ping /healthz
          </Button>
        </div>
        <ConnectionStatus
          status={status}
          health={health}
          healthError={healthError}
        />
        {status?.authenticated && (
          <UsageBadges usage={usage} latencyMs={health?.latency_ms ?? null} />
        )}
      </section>
      </ContentTray>

      <section className="flex flex-col gap-2 border-t border-border-soft pt-5">
        <Button
          size="sm"
          variant="danger"
          onClick={onDisconnect}
          disabled={busy === "disconnect" || !status?.configured}
        >
          <CloudOff className="size-3.5" />
          Disconnect & clear local credentials
        </Button>
      </section>
      </div>
      )}

      {authenticated && visibleSection === "corpora" && (
        <CorporaSection authenticated={authenticated} />
      )}

      {authenticated && visibleSection === "api-keys" && (
        <ApiKeysSection authenticated={authenticated} />
      )}

      {authenticated && visibleSection === "webhooks" && (
        <WebhooksSection authenticated={authenticated} />
      )}

      {authenticated && visibleSection === "usage" && (
        <OrgUsageSection authenticated={authenticated} />
      )}

      {authenticated && visibleSection === "sessions" && (
        <SessionInspectorSection authenticated={authenticated} />
      )}
    </SurfaceSidebar>
  );
}

// ── Corpora section ─────────────────────────────────────────────────────────

interface CorporaSectionProps {
  authenticated: boolean;
}

function CorporaSection({ authenticated }: CorporaSectionProps) {
  const [corpora, setCorpora] = useState<CloudCorpusInfo[]>([]);
  const [listError, setListError] = useState<string | null>(null);
  const [cloneOpen, setCloneOpen] = useState(false);
  const [registerOpen, setRegisterOpen] = useState(false);
  const [deleteCandidate, setDeleteCandidate] = useState<string | null>(null);
  const [progressFor, setProgressFor] = useState<string | null>(null);
  const [shareFor, setShareFor] = useState<string | null>(null);
  const [busy, setBusy] = useState<null | "list" | "delete" | "reindex">(null);

  // F2.7 — onboarding wizard's "Clone first repo" step dispatches a
  // custom event so this section can open the clone dialog without
  // CloudPanel lifting cloneOpen out of here. SRP: the event surface
  // is internal to CloudPanel.tsx; no global event-bus pattern needed.
  useEffect(() => {
    const handler = () => {
      if (authenticated) setCloneOpen(true);
    };
    window.addEventListener("ministr.cloud.open-clone", handler);
    return () => window.removeEventListener("ministr.cloud.open-clone", handler);
  }, [authenticated]);

  const refresh = useCallback(async () => {
    if (!authenticated) {
      setCorpora([]);
      return;
    }
    setBusy("list");
    setListError(null);
    try {
      setCorpora(await cloudClient.listCorpora());
    } catch (e) {
      setListError(String(e));
    } finally {
      setBusy(null);
    }
  }, [authenticated]);

  // Initial + 5s poll while authenticated.
  useEffect(() => {
    void refresh();
    if (!authenticated) return;
    const t = window.setInterval(() => void refresh(), 5000);
    return () => window.clearInterval(t);
  }, [authenticated, refresh]);

  const onDelete = async () => {
    if (!deleteCandidate) return;
    setBusy("delete");
    try {
      await cloudClient.unregisterCorpus(deleteCandidate);
      setDeleteCandidate(null);
      await refresh();
    } catch (e) {
      setListError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const onReindex = async (id: string) => {
    setBusy("reindex");
    try {
      await cloudClient.triggerReindex(id);
      setProgressFor(id);
    } catch (e) {
      setListError(String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <h3 className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
          Corpora
        </h3>
        <div className="flex gap-2">
          <Button
            size="sm"
            variant="outline"
            onClick={refresh}
            disabled={!authenticated || busy === "list"}
          >
            {busy === "list" ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <RefreshCw className="size-3.5" />
            )}
            Refresh
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => setRegisterOpen(true)}
            disabled={!authenticated}
          >
            <Plus className="size-3.5" />
            Register path
          </Button>
          <Button
            size="sm"
            onClick={() => setCloneOpen(true)}
            disabled={!authenticated}
          >
            <GitBranch className="size-3.5" />
            Clone repo
          </Button>
        </div>
      </div>

      {!authenticated && (
        <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 text-sm text-text-muted">
          Sign in (save a Bearer token above) to manage corpora.
        </div>
      )}

      {listError && (
        <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-xs text-text font-mono">
          {listError}
        </div>
      )}

      {authenticated && corpora.length === 0 && !listError && (
        <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 text-sm text-text-muted">
          No corpora registered yet. Clone a repo or register a server-side path.
        </div>
      )}

      {corpora.length > 0 && (
        <CorporaTable
          corpora={corpora}
          busy={busy === "delete" || busy === "reindex"}
          onDeleteRequest={setDeleteCandidate}
          onReindex={(id) => void onReindex(id)}
          onShowProgress={setProgressFor}
          onShareRequest={setShareFor}
        />
      )}

      {cloneOpen && (
        <CloneDialog
          onClose={() => setCloneOpen(false)}
          onSuccess={(corpusId) => {
            setCloneOpen(false);
            setProgressFor(corpusId);
            void refresh();
          }}
        />
      )}
      {registerOpen && (
        <RegisterDialog
          onClose={() => setRegisterOpen(false)}
          onSuccess={() => {
            setRegisterOpen(false);
            void refresh();
          }}
        />
      )}
      {progressFor && (
        <ProgressDrawer
          corpusId={progressFor}
          onClose={() => setProgressFor(null)}
        />
      )}
      {shareFor && (
        <ShareDialog
          corpusId={shareFor}
          onClose={() => setShareFor(null)}
          onTransferred={() => void refresh()}
        />
      )}

      <ConfirmDialog
        open={!!deleteCandidate}
        title={`Unregister ${deleteCandidate ?? ""}?`}
        body={
          <>
            This removes the corpus from the remote server. Indexed data on
            the server is dropped. Local desktop corpora are unaffected.
          </>
        }
        confirmLabel="Unregister"
        cancelLabel="Keep"
        tone="danger"
        onConfirm={() => void onDelete()}
        onCancel={() => setDeleteCandidate(null)}
      />
    </section>
  );
}

interface CorporaTableProps {
  corpora: CloudCorpusInfo[];
  busy: boolean;
  onDeleteRequest: (corpusId: string) => void;
  onReindex: (corpusId: string) => void;
  onShowProgress: (corpusId: string) => void;
  onShareRequest: (corpusId: string) => void;
}

function CorporaTable({
  corpora,
  busy,
  onDeleteRequest,
  onReindex,
  onShowProgress,
  onShareRequest,
}: CorporaTableProps) {
  return (
    <div className="rounded-lg border border-border-soft bg-surface overflow-hidden">
      <table className="w-full text-sm">
        <thead className="bg-surface-overlay border-b border-border-soft">
          <tr className="text-left text-xs font-mono uppercase tracking-[0.08em] text-text-muted">
            <th className="px-3 py-2 font-semibold">Corpus</th>
            <th className="px-3 py-2 font-semibold">Source</th>
            <th className="px-3 py-2 font-semibold w-24">Status</th>
            <th className="px-3 py-2 font-semibold w-40 text-right">Actions</th>
          </tr>
        </thead>
        <tbody>
          {corpora.map((c) => (
            <tr
              key={c.corpus_id}
              className="border-b border-border-soft last:border-b-0"
            >
              <td className="px-3 py-2 align-top">
                <div className="font-mono text-xs text-text">{c.corpus_id}</div>
                {c.display_name && (
                  <div className="text-xs text-text-muted">{c.display_name}</div>
                )}
              </td>
              <td className="px-3 py-2 align-top">
                <div className="text-xs text-text-muted font-mono break-all">
                  {c.paths.join(", ") || "—"}
                </div>
              </td>
              <td className="px-3 py-2 align-top">
                <span className="text-xs text-text-muted">
                  {c.indexing_status ?? "ready"}
                </span>
              </td>
              <td className="px-3 py-2 align-top">
                <div className="flex justify-end gap-1.5">
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => onShowProgress(c.corpus_id)}
                    title="Show indexing progress"
                  >
                    <RefreshCw className="size-3.5" />
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={busy}
                    onClick={() => onReindex(c.corpus_id)}
                    title="Reindex"
                  >
                    Reindex
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={busy}
                    onClick={() => onShareRequest(c.corpus_id)}
                    title="Share with org"
                  >
                    <Share2 className="size-3.5" />
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={busy}
                    onClick={() => onDeleteRequest(c.corpus_id)}
                    title="Unregister"
                  >
                    <Trash2 className="size-3.5" />
                  </Button>
                </div>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

interface CloneDialogProps {
  onClose: () => void;
  onSuccess: (corpusId: string) => void;
}

function CloneDialog({ onClose, onSuccess }: CloneDialogProps) {
  const [repo, setRepo] = useState("");
  const [branch, setBranch] = useState("");
  const [label, setLabel] = useState("");
  const [installationId, setInstallationId] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const onSubmit = async () => {
    setError(null);
    setBusy(true);
    try {
      const res = await cloudClient.cloneRepo(
        repo.trim(),
        branch.trim() || undefined,
        label.trim() || undefined,
        installationId.trim() || undefined,
      );
      onSuccess(res.corpus_id);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <DialogShell title="Clone repo on remote server" onClose={onClose}>
      <LabeledInput
        label="Git URL"
        placeholder="https://github.com/owner/repo.git"
        value={repo}
        onChange={setRepo}
        type="url"
      />
      <LabeledInput
        label="Branch (optional)"
        placeholder="main"
        value={branch}
        onChange={setBranch}
      />
      <LabeledInput
        label="Label / slug (optional)"
        placeholder="auto-derived from URL"
        value={label}
        onChange={setLabel}
      />
      <LabeledInput
        label="GitHub App installation ID (private repos)"
        placeholder="leave blank for public repos"
        value={installationId}
        onChange={setInstallationId}
      />
      <p className="text-xs text-text-muted -mt-1">
        For private repos, install the ministr GitHub App on the target
        repo or org, then paste the installation ID here. The cloud
        mints a short-lived access token server-side — your local
        machine never sees a PAT.
      </p>
      {error && (
        <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-xs font-mono text-text">
          {error}
        </div>
      )}
      <div className="flex justify-end gap-2 pt-1">
        <Button size="sm" variant="ghost" onClick={onClose} disabled={busy}>
          Cancel
        </Button>
        <Button size="sm" onClick={() => void onSubmit()} disabled={busy || !repo.trim()}>
          {busy ? <Loader2 className="size-3.5 animate-spin" /> : null}
          Clone & index
        </Button>
      </div>
    </DialogShell>
  );
}

interface RegisterDialogProps {
  onClose: () => void;
  onSuccess: () => void;
}

function RegisterDialog({ onClose, onSuccess }: RegisterDialogProps) {
  const [pathsText, setPathsText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const paths = useMemo(
    () =>
      pathsText
        .split("\n")
        .map((p) => p.trim())
        .filter((p) => p.length > 0),
    [pathsText],
  );

  const onSubmit = async () => {
    setError(null);
    setBusy(true);
    try {
      await cloudClient.registerCorpus(paths);
      onSuccess();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <DialogShell
      title="Register server-side path"
      onClose={onClose}
      hint="Paths resolve on the remote container, not your local desktop."
    >
      <label className="flex flex-col gap-1.5">
        <span className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
          Path(s), one per line
        </span>
        <textarea
          rows={4}
          value={pathsText}
          onChange={(e) => setPathsText(e.target.value)}
          placeholder="/data/some-repo&#10;/data/another"
          className="px-3 py-2 rounded-md border border-border bg-surface font-mono text-sm text-text focus:outline-none focus:border-border-hover"
        />
      </label>
      {error && (
        <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-xs font-mono text-text">
          {error}
        </div>
      )}
      <div className="flex justify-end gap-2 pt-1">
        <Button size="sm" variant="ghost" onClick={onClose} disabled={busy}>
          Cancel
        </Button>
        <Button
          size="sm"
          onClick={() => void onSubmit()}
          disabled={busy || paths.length === 0}
        >
          {busy ? <Loader2 className="size-3.5 animate-spin" /> : null}
          Register
        </Button>
      </div>
    </DialogShell>
  );
}

interface ShareDialogProps {
  corpusId: string;
  onClose: () => void;
  /**
   * F3.2-iv-b — fires after a successful corpus transfer. The parent
   * refreshes its corpus list so the now-org-owned corpus re-renders
   * with the new tenant context.
   */
  onTransferred?: () => void;
}

/**
 * F3.2-ii — share a corpus with one of the user's orgs. Lists current
 * shares so the owner can revoke; org dropdown filters to orgs the
 * caller is a member of (the cloud rejects sharing with a non-member
 * org with 403, so the dropdown mirrors what the server admits).
 */
function ShareDialog({ corpusId, onClose, onTransferred }: ShareDialogProps) {
  const [orgs, setOrgs] = useState<CloudOrg[]>([]);
  const [shares, setShares] = useState<CloudAclEntry[]>([]);
  const [selectedOrgId, setSelectedOrgId] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<null | "load" | "share" | "revoke" | "transfer">(null);
  // F3.2-iv-b — transfer flow runs through a confirm step so a stray
  // click doesn't move the corpus off the user's tenant. Tracks the
  // org id the user intends to transfer to, or null when no
  // confirmation is open.
  const [transferTargetOrgId, setTransferTargetOrgId] = useState<string | null>(null);
  const [transferSelectId, setTransferSelectId] = useState("");
  const [transferNote, setTransferNote] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setBusy("load");
    setError(null);
    try {
      const [orgList, shareList] = await Promise.all([
        cloudClient.listOrgs(),
        cloudClient.listCorpusShares(corpusId),
      ]);
      setOrgs(orgList);
      setShares(shareList);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }, [corpusId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // The cloud rejects sharing with an org the caller doesn't belong to
  // (HTTP 403). Hide already-shared orgs from the picker so the owner
  // doesn't waste a click on a duplicate POST (the backend is idempotent,
  // but the UI shouldn't suggest a no-op).
  const sharedOrgIds = useMemo(
    () => new Set(shares.map((s) => s.org_id).filter((id): id is string => !!id)),
    [shares],
  );
  const shareableOrgs = useMemo(
    () => orgs.filter((o) => !sharedOrgIds.has(o.id)),
    [orgs, sharedOrgIds],
  );

  // F3.2-iv-b — transfer requires owner/admin role on the target org
  // (the cloud rejects member-only callers with 403). Restrict the
  // picker to orgs where the caller has the necessary role; hide a
  // confusing "tried it but got 403" feedback loop.
  const transferableOrgs = useMemo(
    () => orgs.filter((o) => o.role === "owner" || o.role === "admin"),
    [orgs],
  );

  const onShare = async () => {
    if (!selectedOrgId) return;
    setBusy("share");
    setError(null);
    try {
      await cloudClient.shareCorpus(corpusId, selectedOrgId);
      setSelectedOrgId("");
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const onRevoke = async (orgId: string) => {
    setBusy("revoke");
    setError(null);
    try {
      await cloudClient.revokeCorpusShare(corpusId, orgId);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const orgName = (orgId: string | null | undefined) =>
    (orgId && orgs.find((o) => o.id === orgId)?.name) || orgId || "—";

  const onTransferConfirm = async () => {
    if (!transferTargetOrgId) return;
    setBusy("transfer");
    setError(null);
    setTransferNote(null);
    try {
      const resp = await cloudClient.transferCorpusToOrg(corpusId, transferTargetOrgId);
      setTransferNote(
        resp.transferred
          ? `Transferred to ${orgName(transferTargetOrgId)}.`
          : `Already owned by ${orgName(transferTargetOrgId)} — no change.`,
      );
      setTransferTargetOrgId(null);
      setTransferSelectId("");
      // Always refresh — even on the idempotent path, parent state
      // may have been stale.
      await refresh();
      onTransferred?.();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <DialogShell
      title={`Share corpus`}
      onClose={onClose}
      hint={`Members of the selected org can read ${corpusId}. Revoke at any time.`}
    >
      <label className="flex flex-col gap-1.5">
        <span className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
          Share with org
        </span>
        <select
          value={selectedOrgId}
          onChange={(e) => setSelectedOrgId(e.target.value)}
          disabled={busy === "load" || shareableOrgs.length === 0}
          className="h-9 px-3 rounded-md border border-border bg-surface font-mono text-sm text-text focus:outline-none focus:border-border-hover"
        >
          <option value="">
            {busy === "load"
              ? "Loading…"
              : shareableOrgs.length === 0
                ? orgs.length === 0
                  ? "You're not in any orgs yet"
                  : "Already shared with every org you're in"
                : "Select an org…"}
          </option>
          {shareableOrgs.map((o) => (
            <option key={o.id} value={o.id}>
              {o.name} ({o.role})
            </option>
          ))}
        </select>
      </label>

      <div className="flex justify-end">
        <Button
          size="sm"
          onClick={() => void onShare()}
          disabled={!selectedOrgId || busy === "share"}
        >
          {busy === "share" ? <Loader2 className="size-3.5 animate-spin" /> : (
            <Share2 className="size-3.5" />
          )}
          Share
        </Button>
      </div>

      <div className="flex flex-col gap-1.5 border-t border-border-soft pt-3">
        <span className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
          Current shares
        </span>
        {shares.length === 0 ? (
          <div className="text-xs text-text-muted">
            {busy === "load" ? "Loading…" : "Not shared with any org yet."}
          </div>
        ) : (
          <ul className="flex flex-col gap-1.5">
            {shares.map((s) => (
              <li
                key={`${s.org_id ?? s.user_id ?? "row"}-${s.created_at}`}
                className="flex items-center justify-between rounded-md border border-border-soft bg-surface-overlay px-3 py-2"
              >
                <div className="flex flex-col">
                  <span className="font-mono text-xs text-text">
                    {orgName(s.org_id)}
                  </span>
                  <span className="text-xs text-text-muted">
                    {s.scope} · granted {s.created_at.slice(0, 10)}
                  </span>
                </div>
                {s.org_id && (
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => void onRevoke(s.org_id!)}
                    disabled={busy === "revoke"}
                    title="Revoke share"
                  >
                    <Trash2 className="size-3.5" />
                  </Button>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>

      {/* F3.2-iv-b — transfer ownership. Sits below the share UI
          because it's a more destructive action: direct ownership is
          replaced by org-member access. Restricted to orgs where the
          caller is owner/admin (cloud rejects member-only with 403). */}
      <div className="flex flex-col gap-1.5 border-t border-border-soft pt-3">
        <span className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
          Transfer ownership
        </span>
        <p className="text-xs text-text-muted">
          Move this corpus into an org. Members of the org will see it; you'll
          still see it via your org membership but the org owns it.
        </p>
        <label className="flex flex-col gap-1.5">
          <select
            value={transferSelectId}
            onChange={(e) => setTransferSelectId(e.target.value)}
            disabled={busy === "load" || transferableOrgs.length === 0}
            className="h-9 px-3 rounded-md border border-border bg-surface font-mono text-sm text-text focus:outline-none focus:border-border-hover"
          >
            <option value="">
              {busy === "load"
                ? "Loading…"
                : transferableOrgs.length === 0
                  ? orgs.length === 0
                    ? "You're not in any orgs yet"
                    : "No org where you're owner/admin"
                  : "Select an org…"}
            </option>
            {transferableOrgs.map((o) => (
              <option key={o.id} value={o.id}>
                {o.name} ({o.role})
              </option>
            ))}
          </select>
        </label>
        <div className="flex justify-end">
          <Button
            size="sm"
            variant="outline"
            onClick={() => setTransferTargetOrgId(transferSelectId)}
            disabled={!transferSelectId || busy === "transfer"}
            title="Transfer corpus to the selected org"
          >
            <ArrowRight className="size-3.5" />
            Transfer…
          </Button>
        </div>
      </div>

      {transferNote && (
        <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 text-xs font-mono text-text-muted">
          {transferNote}
        </div>
      )}

      {error && (
        <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-xs font-mono text-text">
          {error}
        </div>
      )}

      <div className="flex justify-end gap-2 pt-1">
        <Button size="sm" variant="ghost" onClick={onClose}>
          Close
        </Button>
      </div>

      <ConfirmDialog
        open={!!transferTargetOrgId}
        title="Transfer this corpus?"
        body={
          <>
            <p>
              <span className="font-mono">{corpusId}</span> will move into{" "}
              <span className="font-semibold">{orgName(transferTargetOrgId)}</span>.
            </p>
            <p className="mt-2">
              Org members will be able to see it. You will still see it via
              your org membership but the org owns it.
            </p>
          </>
        }
        confirmLabel="Transfer"
        cancelLabel="Cancel"
        tone="danger"
        onConfirm={() => void onTransferConfirm()}
        onCancel={() => setTransferTargetOrgId(null)}
      />
    </DialogShell>
  );
}

// ── API keys section (F3.4b) ───────────────────────────────────────────────

interface ApiKeysSectionProps {
  authenticated: boolean;
}

/**
 * F3.4b — service-account API keys management surface. Lists the
 * caller's active keys (showing only prefix + last_used), lets them
 * mint a new key (secret shown once), and revoke existing ones.
 *
 * SOLID note: state is local to the section — CloudPanel's parent does
 * not need to know about API keys. Mirrors CorporaSection's shape.
 */
function ApiKeysSection({ authenticated }: ApiKeysSectionProps) {
  const [keys, setKeys] = useState<CloudApiKey[]>([]);
  const [listError, setListError] = useState<string | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [revokeCandidate, setRevokeCandidate] = useState<string | null>(null);
  const [createdKey, setCreatedKey] = useState<CloudCreatedApiKey | null>(null);
  const [busy, setBusy] = useState<null | "list" | "revoke">(null);

  const refresh = useCallback(async () => {
    if (!authenticated) {
      setKeys([]);
      return;
    }
    setBusy("list");
    setListError(null);
    try {
      setKeys(await cloudClient.listApiKeys());
    } catch (e) {
      setListError(String(e));
    } finally {
      setBusy(null);
    }
  }, [authenticated]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const onRevoke = async () => {
    if (!revokeCandidate) return;
    setBusy("revoke");
    try {
      await cloudClient.revokeApiKey(revokeCandidate);
      setRevokeCandidate(null);
      await refresh();
    } catch (e) {
      setListError(String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <h3 className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
          API keys
        </h3>
        <div className="flex gap-2">
          <Button
            size="sm"
            variant="outline"
            onClick={refresh}
            disabled={!authenticated || busy === "list"}
          >
            {busy === "list" ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <RefreshCw className="size-3.5" />
            )}
            Refresh
          </Button>
          <Button
            size="sm"
            onClick={() => setCreateOpen(true)}
            disabled={!authenticated}
          >
            <Plus className="size-3.5" />
            New key
          </Button>
        </div>
      </div>

      <p className="text-xs text-text-muted">
        Long-lived service-account tokens (
        <span className="font-mono">mst_pk_…</span>) authenticate the same
        way as your session token. Use them for CI / scripts that need
        to call the cloud without an interactive sign-in.
      </p>

      {!authenticated && (
        <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 text-sm text-text-muted">
          Sign in to manage API keys.
        </div>
      )}

      {listError && (
        <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-xs text-text font-mono">
          {listError}
        </div>
      )}

      {authenticated && keys.length === 0 && !listError && (
        <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 text-sm text-text-muted">
          No API keys yet. Click <span className="font-mono">New key</span> to
          mint one.
        </div>
      )}

      {keys.length > 0 && (
        <ApiKeysTable
          keys={keys}
          busy={busy === "revoke"}
          onRevokeRequest={setRevokeCandidate}
        />
      )}

      {createOpen && (
        <CreateApiKeyDialog
          onClose={() => setCreateOpen(false)}
          onSuccess={(created) => {
            setCreateOpen(false);
            setCreatedKey(created);
            void refresh();
          }}
        />
      )}

      {createdKey && (
        <ShowApiKeyDialog
          created={createdKey}
          onClose={() => setCreatedKey(null)}
        />
      )}

      <ConfirmDialog
        open={!!revokeCandidate}
        title="Revoke API key?"
        body={
          <>
            The key will stop authenticating immediately. Any CI jobs or
            scripts using it will return 401 on their next call. This
            cannot be undone.
          </>
        }
        confirmLabel="Revoke"
        cancelLabel="Keep"
        tone="danger"
        onConfirm={() => void onRevoke()}
        onCancel={() => setRevokeCandidate(null)}
      />
    </section>
  );
}

interface ApiKeysTableProps {
  keys: CloudApiKey[];
  busy: boolean;
  onRevokeRequest: (id: string) => void;
}

/**
 * F3.4c-ii — staleness threshold matches the backend's
 * `DEFAULT_STALE_API_KEY_DAYS` (90). Keeping the same number on both
 * sides means the badge fires for the same rows the cron emits
 * `api_key.stale` events for.
 */
const STALE_API_KEY_THRESHOLD_DAYS = 90;
const STALE_API_KEY_THRESHOLD_MS = STALE_API_KEY_THRESHOLD_DAYS * 24 * 60 * 60 * 1000;

/**
 * Mirror of the backend's stale-detection rule:
 *   COALESCE(last_used_at, created_at) < now() - interval '90 days'
 * Returns true when the key hasn't authenticated in the last 90 days
 * (or was never used and is older than 90 days).
 */
function isStaleApiKey(k: CloudApiKey, now: number): boolean {
  const anchor = k.last_used_at ?? k.created_at;
  if (!anchor) return false;
  const t = Date.parse(anchor);
  if (Number.isNaN(t)) return false;
  return now - t > STALE_API_KEY_THRESHOLD_MS;
}

function ApiKeysTable({ keys, busy, onRevokeRequest }: ApiKeysTableProps) {
  // Single Date.now() per render so all rows judge against the same
  // wall-clock; otherwise borderline cases could flip mid-paint.
  const now = Date.now();
  return (
    <div className="rounded-lg border border-border-soft bg-surface overflow-hidden">
      <table className="w-full text-sm">
        <thead className="bg-surface-overlay border-b border-border-soft">
          <tr className="text-left text-xs font-mono uppercase tracking-[0.08em] text-text-muted">
            <th className="px-3 py-2 font-semibold">Name</th>
            <th className="px-3 py-2 font-semibold">Prefix</th>
            <th className="px-3 py-2 font-semibold">Last used</th>
            <th className="px-3 py-2 font-semibold w-24 text-right">Actions</th>
          </tr>
        </thead>
        <tbody>
          {keys.map((k) => {
            const stale = isStaleApiKey(k, now);
            return (
              <tr key={k.id} className="border-b border-border-soft last:border-b-0">
                <td className="px-3 py-2 align-top">
                  <div className="flex items-center gap-2">
                    <span className="text-xs text-text">{k.name}</span>
                    {stale && (
                      <span
                        className="rounded-md border border-warning/40 bg-warning/10 px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-[0.08em] text-warning"
                        title={`No use in the last ${STALE_API_KEY_THRESHOLD_DAYS} days. The cloud's weekly cron will record an api_key.stale audit event.`}
                      >
                        stale
                      </span>
                    )}
                  </div>
                  <div className="text-xs text-text-muted">{k.scopes}</div>
                </td>
                <td className="px-3 py-2 align-top">
                  <span className="font-mono text-xs text-text-muted">
                    mst_pk_{k.prefix}…
                  </span>
                </td>
                <td className="px-3 py-2 align-top">
                  <span className="text-xs text-text-muted">
                    {k.last_used_at?.slice(0, 10) ?? "never"}
                  </span>
                </td>
                <td className="px-3 py-2 align-top">
                  <div className="flex justify-end">
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={busy}
                      onClick={() => onRevokeRequest(k.id)}
                      title="Revoke"
                    >
                      <Trash2 className="size-3.5" />
                    </Button>
                  </div>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

interface CreateApiKeyDialogProps {
  onClose: () => void;
  onSuccess: (created: CloudCreatedApiKey) => void;
}

/**
 * F3.4b — mint a new API key. The cloud returns the raw token in the
 * response; we hand it to the parent which opens [`ShowApiKeyDialog`]
 * to display + copy it once. The token is never stored on this side.
 */
/**
 * F3.4c-i — closed-vocabulary scope list. Mirrors
 * `ministr_cloud::api_keys::ALLOWED_API_KEY_SCOPES`. If the backend
 * ever adds a fifth scope, update this list in lockstep — the server
 * already rejects unknown tokens with 400, so a stale frontend won't
 * silently mint broken keys.
 */
const API_KEY_SCOPE_OPTIONS: Array<{ value: string; label: string; hint: string }> = [
  { value: "ministr:read", label: "Read", hint: "Survey, read, extract — all read-only tool calls." },
  { value: "ministr:write", label: "Write", hint: "Register / reindex / unregister corpora." },
  { value: "ministr:bundle:read", label: "Bundle read", hint: "Export a corpus bundle." },
  { value: "ministr:bundle:write", label: "Bundle write", hint: "Import a corpus bundle." },
];

/** Defaults match the server's behaviour when the request omits scopes. */
const DEFAULT_API_KEY_SCOPES = new Set(["ministr:read", "ministr:write"]);

function CreateApiKeyDialog({ onClose, onSuccess }: CreateApiKeyDialogProps) {
  const [name, setName] = useState("");
  const [selectedScopes, setSelectedScopes] = useState<Set<string>>(
    () => new Set(DEFAULT_API_KEY_SCOPES),
  );
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const toggleScope = (scope: string) => {
    setSelectedScopes((prev) => {
      const next = new Set(prev);
      if (next.has(scope)) next.delete(scope);
      else next.add(scope);
      return next;
    });
  };

  const onSubmit = async () => {
    setError(null);
    if (selectedScopes.size === 0) {
      setError("Select at least one scope.");
      return;
    }
    setBusy(true);
    try {
      // Preserve API_KEY_SCOPE_OPTIONS order so the canonical string
      // matches what the server would produce on a same-input request.
      const scopes = API_KEY_SCOPE_OPTIONS
        .filter((opt) => selectedScopes.has(opt.value))
        .map((opt) => opt.value)
        .join(" ");
      const created = await cloudClient.createApiKey(name.trim(), scopes);
      onSuccess(created);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <DialogShell title="New API key" onClose={onClose}>
      <LabeledInput
        label="Name"
        placeholder="ci-prod, deploy-bot, …"
        value={name}
        onChange={setName}
      />
      <fieldset className="flex flex-col gap-1.5">
        <legend className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
          Scopes
        </legend>
        {API_KEY_SCOPE_OPTIONS.map((opt) => (
          <label
            key={opt.value}
            className="flex items-start gap-2 rounded-md border border-border-soft bg-surface-overlay px-3 py-2 cursor-pointer hover:border-border-hover"
          >
            <input
              type="checkbox"
              checked={selectedScopes.has(opt.value)}
              onChange={() => toggleScope(opt.value)}
              disabled={busy}
              className="mt-0.5"
            />
            <span className="flex flex-col gap-0.5">
              <span className="text-xs text-text">
                <span className="font-semibold">{opt.label}</span>
                <span className="ml-2 font-mono text-text-muted">{opt.value}</span>
              </span>
              <span className="text-xs text-text-muted">{opt.hint}</span>
            </span>
          </label>
        ))}
      </fieldset>
      {error && (
        <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-xs font-mono text-text">
          {error}
        </div>
      )}
      <div className="flex justify-end gap-2 pt-1">
        <Button size="sm" variant="ghost" onClick={onClose} disabled={busy}>
          Cancel
        </Button>
        <Button
          size="sm"
          onClick={() => void onSubmit()}
          disabled={busy || !name.trim() || selectedScopes.size === 0}
        >
          {busy ? <Loader2 className="size-3.5 animate-spin" /> : <Key className="size-3.5" />}
          Mint key
        </Button>
      </div>
    </DialogShell>
  );
}

interface ShowApiKeyDialogProps {
  created: CloudCreatedApiKey;
  onClose: () => void;
}

/**
 * F3.4b — display the raw token EXACTLY ONCE after creation. The
 * cloud cannot recover the secret after this response; if the user
 * dismisses this dialog without copying, they must mint a new key.
 *
 * The Copy button writes to the system clipboard via the browser
 * Clipboard API; Tauri's webview supports `navigator.clipboard.writeText`
 * out of the box for HTTPS-equivalent origins.
 */
function ShowApiKeyDialog({ created, onClose }: ShowApiKeyDialogProps) {
  const [copied, setCopied] = useState(false);

  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(created.token);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      // Best-effort: if the clipboard API is unavailable, the user
      // can still manually select + copy the text below.
    }
  };

  return (
    <DialogShell
      title="API key minted"
      onClose={onClose}
      hint="Save this token now — the cloud cannot show it again. If you lose it, revoke the key and mint a new one."
    >
      <div className="flex flex-col gap-1">
        <span className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
          {created.name}
        </span>
        <div className="flex items-center gap-2">
          <code className="flex-1 rounded-md border border-border bg-surface-overlay px-3 py-2 font-mono text-xs text-text break-all">
            {created.token}
          </code>
          <Button size="sm" variant="outline" onClick={() => void onCopy()}>
            {copied ? (
              <Check className="size-3.5" />
            ) : (
              <Copy className="size-3.5" />
            )}
            {copied ? "Copied" : "Copy"}
          </Button>
        </div>
      </div>
      <p className="text-xs text-text-muted">
        Use as <span className="font-mono">Authorization: Bearer {created.token.slice(0, 14)}…</span>{" "}
        — every cloud endpoint that accepts your session token also
        accepts this key.
      </p>
      <div className="flex justify-end gap-2 pt-1">
        <Button size="sm" onClick={onClose}>
          I've saved it
        </Button>
      </div>
    </DialogShell>
  );
}

// ── Webhooks section (F3.5b-ii) ────────────────────────────────────────────

interface WebhooksSectionProps {
  authenticated: boolean;
}

/**
 * F3.5b-ii — outbound webhook subscription management. Webhooks are
 * org-scoped, so the section starts with an org picker (populated from
 * `cloud_list_orgs`). The selected org's subscriptions show below
 * with create / test / delete actions.
 *
 * SOLID note: mirrors `ApiKeysSection` (F3.4b) with the addition of
 * the org-picker. The CreateDialog returns a one-time HMAC secret
 * displayed via `ShowWebhookSecretDialog` — identical pattern to
 * `ShowApiKeyDialog`.
 */
function WebhooksSection({ authenticated }: WebhooksSectionProps) {
  const [orgs, setOrgs] = useState<CloudOrg[]>([]);
  const [selectedOrgId, setSelectedOrgId] = useState<string>("");
  const [subs, setSubs] = useState<CloudWebhookSub[]>([]);
  const [listError, setListError] = useState<string | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [createdSub, setCreatedSub] = useState<CloudCreatedWebhookSub | null>(null);
  const [deleteCandidate, setDeleteCandidate] = useState<string | null>(null);
  const [busy, setBusy] = useState<null | "list-orgs" | "list-subs" | "delete" | "test">(null);
  const [testResult, setTestResult] = useState<{
    subId: string;
    outcome: CloudWebhookTestResult;
  } | null>(null);

  // Load orgs once the user authenticates; the picker only renders
  // when there's at least one org. v0 stops short of remembering the
  // user's most-recently-used org — refreshing the section just
  // re-defaults to the first one in the list.
  useEffect(() => {
    if (!authenticated) {
      setOrgs([]);
      setSelectedOrgId("");
      return;
    }
    let cancelled = false;
    setBusy("list-orgs");
    void cloudClient
      .listOrgs()
      .then((list) => {
        if (cancelled) return;
        setOrgs(list);
        if (list.length > 0 && !selectedOrgId) {
          setSelectedOrgId(list[0].id);
        }
      })
      .catch(() => {
        if (!cancelled) setOrgs([]);
      })
      .finally(() => {
        if (!cancelled) setBusy(null);
      });
    return () => {
      cancelled = true;
    };
    // selectedOrgId intentionally NOT in deps — only fetch orgs once
    // per auth flip, not on every selection change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [authenticated]);

  const refreshSubs = useCallback(async () => {
    if (!authenticated || !selectedOrgId) {
      setSubs([]);
      return;
    }
    setBusy("list-subs");
    setListError(null);
    try {
      setSubs(await cloudClient.listWebhookSubs(selectedOrgId));
    } catch (e) {
      setListError(String(e));
      setSubs([]);
    } finally {
      setBusy(null);
    }
  }, [authenticated, selectedOrgId]);

  useEffect(() => {
    void refreshSubs();
  }, [refreshSubs]);

  const onDelete = async () => {
    if (!deleteCandidate || !selectedOrgId) return;
    setBusy("delete");
    try {
      await cloudClient.deleteWebhookSub(selectedOrgId, deleteCandidate);
      setDeleteCandidate(null);
      await refreshSubs();
    } catch (e) {
      setListError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const onTest = async (subId: string) => {
    if (!selectedOrgId) return;
    setBusy("test");
    try {
      const outcome = await cloudClient.testWebhookSub(selectedOrgId, subId);
      setTestResult({ subId, outcome });
      // Auto-clear after a few seconds so a successful test doesn't
      // permanently dim other rows' visual state.
      window.setTimeout(() => {
        setTestResult((prev) => (prev?.subId === subId ? null : prev));
      }, 5000);
      await refreshSubs(); // refresh last_delivered_at on success
    } catch (e) {
      setListError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const selectedOrgName =
    orgs.find((o) => o.id === selectedOrgId)?.name ?? "(select an org)";

  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <h3 className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
          Webhooks
        </h3>
        <div className="flex gap-2">
          <Button
            size="sm"
            variant="outline"
            onClick={refreshSubs}
            disabled={!authenticated || !selectedOrgId || busy === "list-subs"}
          >
            {busy === "list-subs" ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <RefreshCw className="size-3.5" />
            )}
            Refresh
          </Button>
          <Button
            size="sm"
            onClick={() => setCreateOpen(true)}
            disabled={!authenticated || !selectedOrgId}
          >
            <Plus className="size-3.5" />
            New webhook
          </Button>
        </div>
      </div>

      <p className="text-xs text-text-muted">
        HMAC-SHA256 signed POSTs fire when audit events in the
        selected org match the subscription's filter (
        <span className="font-mono">share.granted</span>,{" "}
        <span className="font-mono">api_key.created</span>,{" "}
        <span className="font-mono">corpus.*</span>, …). Use{" "}
        <span className="font-mono">*</span> to subscribe to every audit
        action.
      </p>

      {!authenticated && (
        <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 text-sm text-text-muted">
          Sign in to manage webhooks.
        </div>
      )}

      {authenticated && orgs.length === 0 && !listError && (
        <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 text-sm text-text-muted">
          {busy === "list-orgs"
            ? "Loading orgs…"
            : "You're not in any orgs yet — webhooks are org-scoped."}
        </div>
      )}

      {authenticated && orgs.length > 0 && (
        <label className="flex flex-col gap-1.5">
          <span className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
            Org
          </span>
          <select
            value={selectedOrgId}
            onChange={(e) => setSelectedOrgId(e.target.value)}
            className="h-9 px-3 rounded-md border border-border bg-surface font-mono text-sm text-text focus:outline-none focus:border-border-hover"
          >
            {orgs.map((o) => (
              <option key={o.id} value={o.id}>
                {o.name} ({o.role})
              </option>
            ))}
          </select>
        </label>
      )}

      {listError && (
        <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-xs text-text font-mono">
          {listError}
        </div>
      )}

      {authenticated && selectedOrgId && subs.length === 0 && !listError && (
        <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 text-sm text-text-muted">
          No webhooks for <span className="font-mono">{selectedOrgName}</span>.
          Click <span className="font-mono">New webhook</span> to mint one.
        </div>
      )}

      {subs.length > 0 && (
        <WebhooksTable
          subs={subs}
          busy={busy === "delete" || busy === "test"}
          testResult={testResult}
          onDeleteRequest={setDeleteCandidate}
          onTest={(id) => void onTest(id)}
        />
      )}

      {createOpen && selectedOrgId && (
        <CreateWebhookDialog
          orgId={selectedOrgId}
          onClose={() => setCreateOpen(false)}
          onSuccess={(created) => {
            setCreateOpen(false);
            setCreatedSub(created);
            void refreshSubs();
          }}
        />
      )}

      {createdSub && (
        <ShowWebhookSecretDialog
          created={createdSub}
          onClose={() => setCreatedSub(null)}
        />
      )}

      <ConfirmDialog
        open={!!deleteCandidate}
        title="Delete webhook?"
        body={
          <>
            The receiver will stop receiving events immediately. If you
            need it back, mint a new subscription — secrets can't be
            recovered after deletion.
          </>
        }
        confirmLabel="Delete"
        cancelLabel="Keep"
        tone="danger"
        onConfirm={() => void onDelete()}
        onCancel={() => setDeleteCandidate(null)}
      />
    </section>
  );
}

interface WebhooksTableProps {
  subs: CloudWebhookSub[];
  busy: boolean;
  testResult: { subId: string; outcome: CloudWebhookTestResult } | null;
  onDeleteRequest: (id: string) => void;
  onTest: (id: string) => void;
}

function WebhooksTable({
  subs,
  busy,
  testResult,
  onDeleteRequest,
  onTest,
}: WebhooksTableProps) {
  return (
    <div className="rounded-lg border border-border-soft bg-surface overflow-hidden">
      <table className="w-full text-sm">
        <thead className="bg-surface-overlay border-b border-border-soft">
          <tr className="text-left text-xs font-mono uppercase tracking-[0.08em] text-text-muted">
            <th className="px-3 py-2 font-semibold">URL</th>
            <th className="px-3 py-2 font-semibold">Filter</th>
            <th className="px-3 py-2 font-semibold">Last delivered</th>
            <th className="px-3 py-2 font-semibold w-28 text-right">Actions</th>
          </tr>
        </thead>
        <tbody>
          {subs.map((s) => {
            const tr = testResult?.subId === s.id ? testResult.outcome : null;
            return (
              <tr key={s.id} className="border-b border-border-soft last:border-b-0">
                <td className="px-3 py-2 align-top">
                  <div className="font-mono text-xs text-text break-all">{s.url}</div>
                  {tr && (
                    <div className="text-xs mt-1">
                      {tr.succeeded ? (
                        <span className="text-accent">
                          ✓ delivered ({tr.attempts} attempt
                          {tr.attempts === 1 ? "" : "s"})
                        </span>
                      ) : (
                        <span className="text-danger">
                          ✗ failed (status {tr.final_status ?? "—"} after{" "}
                          {tr.attempts} attempts)
                        </span>
                      )}
                    </div>
                  )}
                </td>
                <td className="px-3 py-2 align-top">
                  <span className="font-mono text-xs text-text-muted">
                    {s.event_filter}
                  </span>
                </td>
                <td className="px-3 py-2 align-top">
                  <span className="text-xs text-text-muted">
                    {s.last_delivered_at?.slice(0, 10) ?? "never"}
                  </span>
                </td>
                <td className="px-3 py-2 align-top">
                  <div className="flex justify-end gap-1.5">
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={busy}
                      onClick={() => onTest(s.id)}
                      title="Send synthetic test payload"
                    >
                      <Send className="size-3.5" />
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={busy}
                      onClick={() => onDeleteRequest(s.id)}
                      title="Delete"
                    >
                      <Trash2 className="size-3.5" />
                    </Button>
                  </div>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

interface CreateWebhookDialogProps {
  orgId: string;
  onClose: () => void;
  onSuccess: (created: CloudCreatedWebhookSub) => void;
}

function CreateWebhookDialog({ orgId, onClose, onSuccess }: CreateWebhookDialogProps) {
  const [url, setUrl] = useState("");
  const [eventFilter, setEventFilter] = useState("*");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const onSubmit = async () => {
    setError(null);
    setBusy(true);
    try {
      const created = await cloudClient.createWebhookSub(
        orgId,
        url.trim(),
        eventFilter.trim() || undefined,
      );
      onSuccess(created);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <DialogShell title="New webhook" onClose={onClose}>
      <LabeledInput
        label="Receiver URL"
        placeholder="https://hooks.slack.com/services/…"
        value={url}
        onChange={setUrl}
        type="url"
      />
      <LabeledInput
        label="Event filter"
        placeholder="* or share.granted,api_key.created"
        value={eventFilter}
        onChange={setEventFilter}
      />
      <p className="text-xs text-text-muted -mt-1">
        <span className="font-mono">*</span> admits every audit event.
        Otherwise a comma-separated list of exact action names
        (<span className="font-mono">share.granted</span>,{" "}
        <span className="font-mono">corpus.created</span>, …). v0 doesn't
        support wildcards inside action names.
      </p>
      {error && (
        <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-xs font-mono text-text">
          {error}
        </div>
      )}
      <div className="flex justify-end gap-2 pt-1">
        <Button size="sm" variant="ghost" onClick={onClose} disabled={busy}>
          Cancel
        </Button>
        <Button
          size="sm"
          onClick={() => void onSubmit()}
          disabled={busy || !url.trim()}
        >
          {busy ? (
            <Loader2 className="size-3.5 animate-spin" />
          ) : (
            <Webhook className="size-3.5" />
          )}
          Mint subscription
        </Button>
      </div>
    </DialogShell>
  );
}

interface ShowWebhookSecretDialogProps {
  created: CloudCreatedWebhookSub;
  onClose: () => void;
}

function ShowWebhookSecretDialog({
  created,
  onClose,
}: ShowWebhookSecretDialogProps) {
  const [copied, setCopied] = useState(false);

  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(created.secret);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      // Best-effort; user can still manually select + copy.
    }
  };

  return (
    <DialogShell
      title="Webhook secret minted"
      onClose={onClose}
      hint={
        "Save this signing secret now — the cloud cannot show it again. " +
        "Your receiver verifies inbound deliveries by recomputing " +
        "HMAC-SHA256(secret, timestamp + \".\" + body) and comparing " +
        "to the X-Ministr-Signature header."
      }
    >
      <div className="flex flex-col gap-1">
        <span className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
          {created.url}
        </span>
        <div className="flex items-center gap-2">
          <code className="flex-1 rounded-md border border-border bg-surface-overlay px-3 py-2 font-mono text-xs text-text break-all">
            {created.secret}
          </code>
          <Button size="sm" variant="outline" onClick={() => void onCopy()}>
            {copied ? (
              <Check className="size-3.5" />
            ) : (
              <Copy className="size-3.5" />
            )}
            {copied ? "Copied" : "Copy"}
          </Button>
        </div>
      </div>
      <p className="text-xs text-text-muted">
        Filter: <span className="font-mono">{created.event_filter}</span>
      </p>
      <div className="flex justify-end gap-2 pt-1">
        <Button size="sm" onClick={onClose}>
          I've saved it
        </Button>
      </div>
    </DialogShell>
  );
}

// ── Org usage section (F3.3b) ──────────────────────────────────────────────

interface OrgUsageSectionProps {
  authenticated: boolean;
}

/**
 * Per-member totals across the lookback window, derived client-side
 * from the F3.3a per-day-per-kind rollups.
 */
interface MemberTotals {
  user_id: string;
  email: string;
  /** Rolled-up totals per kind (e.g. query.served → 1234). */
  rollup: Map<string, number>;
  /** Today's not-yet-rolled-up partial totals per kind. */
  partial: Map<string, number>;
}

/**
 * F3.3b — org-level usage dashboard surface.
 *
 * Reuses the WebhooksSection org-picker pattern: members across all
 * orgs the user belongs to flow through a single dropdown. The
 * selected org's per-member totals render below as a table.
 *
 * SOLID note: aggregation happens client-side because F3.3c (cost
 * projection + CSV) will need the raw per-day shape anyway; deriving
 * member totals here keeps the backend response shape stable.
 */
function OrgUsageSection({ authenticated }: OrgUsageSectionProps) {
  const [orgs, setOrgs] = useState<CloudOrg[]>([]);
  const [selectedOrgId, setSelectedOrgId] = useState<string>("");
  const [usage, setUsage] = useState<CloudOrgUsage | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [busy, setBusy] = useState<
    null | "list-orgs" | "load-usage" | "export-csv" | "transfer-personal"
  >(null);
  const [exportNote, setExportNote] = useState<string | null>(null);
  // F3.1c-iii UI — transfer flow state. `confirmOpen` gates the
  // ConfirmDialog so an accidental click on the section can't cancel
  // a Stripe subscription. `note` carries the post-call inline status.
  const [transferConfirmOpen, setTransferConfirmOpen] = useState(false);
  const [transferNote, setTransferNote] = useState<string | null>(null);

  useEffect(() => {
    if (!authenticated) {
      setOrgs([]);
      setSelectedOrgId("");
      setUsage(null);
      return;
    }
    let cancelled = false;
    setBusy("list-orgs");
    void cloudClient
      .listOrgs()
      .then((list) => {
        if (cancelled) return;
        setOrgs(list);
        if (list.length > 0 && !selectedOrgId) {
          setSelectedOrgId(list[0].id);
        }
      })
      .catch(() => {
        if (!cancelled) setOrgs([]);
      })
      .finally(() => {
        if (!cancelled) setBusy(null);
      });
    return () => {
      cancelled = true;
    };
    // selectedOrgId intentionally NOT in deps — fetch once per auth flip.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [authenticated]);

  const refresh = useCallback(async () => {
    if (!authenticated || !selectedOrgId) {
      setUsage(null);
      return;
    }
    setBusy("load-usage");
    setListError(null);
    try {
      setUsage(await cloudClient.getOrgUsage(selectedOrgId));
    } catch (e) {
      setListError(String(e));
      setUsage(null);
    } finally {
      setBusy(null);
    }
  }, [authenticated, selectedOrgId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Aggregate the raw rollups into per-member totals. v0 strategy:
  // walk every row, accumulate Map<kind, total> per user. Today's
  // partial folds into a separate map so the UI can render
  // "1,234 + 56 today".
  const members = useMemo(() => {
    if (!usage) return [] as MemberTotals[];
    const byUser = new Map<string, MemberTotals>();
    const get = (user_id: string, email: string): MemberTotals => {
      let m = byUser.get(user_id);
      if (!m) {
        m = { user_id, email, rollup: new Map(), partial: new Map() };
        byUser.set(user_id, m);
      }
      return m;
    };
    for (const r of usage.rollups) {
      const m = get(r.user_id, r.email);
      m.rollup.set(r.kind, (m.rollup.get(r.kind) ?? 0) + r.total);
    }
    for (const p of usage.today_partial) {
      const m = get(p.user_id, p.email);
      m.partial.set(p.kind, (m.partial.get(p.kind) ?? 0) + p.total);
    }
    return Array.from(byUser.values()).sort((a, b) =>
      a.email.localeCompare(b.email),
    );
  }, [usage]);

  // Org-wide totals (sum across all members) for the header summary.
  const totals = useMemo(() => {
    const rollup = new Map<string, number>();
    const partial = new Map<string, number>();
    for (const m of members) {
      for (const [k, v] of m.rollup) rollup.set(k, (rollup.get(k) ?? 0) + v);
      for (const [k, v] of m.partial) partial.set(k, (partial.get(k) ?? 0) + v);
    }
    return { rollup, partial };
  }, [members]);

  // F3.3c — linear end-of-month projection. Take the rolled-up total
  // for the lookback window and extrapolate to a 30-day calendar
  // month. Excludes `today_partial` so a low-volume morning doesn't
  // distort the rate. v0: same denominator (`range_days`) regardless
  // of how much usage actually accumulated — good enough for a
  // dashboard signal, will tighten if finance asks for cycle anchoring.
  const projection = useMemo(() => {
    if (!usage || usage.range_days <= 0) return null;
    const projected = new Map<string, number>();
    for (const [kind, v] of totals.rollup) {
      projected.set(kind, Math.round((v / usage.range_days) * 30));
    }
    return projected;
  }, [usage, totals]);

  const exportCsv = useCallback(async () => {
    if (!authenticated || !selectedOrgId) return;
    setBusy("export-csv");
    setExportNote(null);
    setListError(null);
    try {
      const saved = await cloudClient.exportOrgUsageCsv(selectedOrgId);
      setExportNote(saved ? `Saved to ${saved}` : "Save cancelled.");
    } catch (e) {
      setListError(String(e));
    } finally {
      setBusy(null);
    }
  }, [authenticated, selectedOrgId]);

  // F3.1c-iii UI — the selected org's role determines whether the
  // Move-Pro action is offered. Cloud rejects member-only callers with
  // 403; mirroring that here avoids a 403-feedback round-trip.
  const selectedOrg = useMemo(
    () => orgs.find((o) => o.id === selectedOrgId) ?? null,
    [orgs, selectedOrgId],
  );
  const canTransferPersonal =
    selectedOrg !== null &&
    (selectedOrg.role === "owner" || selectedOrg.role === "admin");

  const onTransferPersonalConfirm = useCallback(async () => {
    if (!selectedOrgId) return;
    setBusy("transfer-personal");
    setListError(null);
    setTransferNote(null);
    try {
      const resp = await cloudClient.transferPersonalSub(selectedOrgId);
      const orgLabel = selectedOrg?.name ?? selectedOrgId;
      if (resp.outcome === "cancelled") {
        const subLabel = resp.subscription_id ?? "your subscription";
        setTransferNote(
          `Cancelled ${subLabel}. Run "Upgrade to Team" on ${orgLabel}'s billing surface to seat yourself.`,
        );
      } else if (resp.outcome === "no_active_subscription") {
        setTransferNote("No active personal subscription to move.");
      } else if (resp.outcome === "no_personal_customer") {
        setTransferNote(
          "No personal Stripe customer detected — sign in via the cloud flow first.",
        );
      } else {
        setTransferNote(`Unexpected outcome: ${resp.outcome}`);
      }
      setTransferConfirmOpen(false);
    } catch (e) {
      setListError(String(e));
    } finally {
      setBusy(null);
    }
  }, [selectedOrgId, selectedOrg]);

  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <h3 className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
          Team usage
        </h3>
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            variant="outline"
            onClick={exportCsv}
            disabled={
              !authenticated ||
              !selectedOrgId ||
              !usage ||
              members.length === 0 ||
              busy === "export-csv"
            }
            title="Save the per-member usage rollup as CSV for finance."
          >
            {busy === "export-csv" ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <Download className="size-3.5" />
            )}
            Export CSV
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={refresh}
            disabled={!authenticated || !selectedOrgId || busy === "load-usage"}
          >
            {busy === "load-usage" ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <RefreshCw className="size-3.5" />
            )}
            Refresh
          </Button>
        </div>
      </div>

      <p className="text-xs text-text-muted">
        Per-seat breakdown of <span className="font-mono">query.served</span>,{" "}
        <span className="font-mono">index.minutes</span>, and{" "}
        <span className="font-mono">atlas.queries</span> from the last{" "}
        {usage?.range_days ?? 30} days. Sums match the live Stripe invoice
        within ±1.
      </p>

      {!authenticated && (
        <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 text-sm text-text-muted">
          Sign in to view team usage.
        </div>
      )}

      {authenticated && orgs.length === 0 && (
        <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 text-sm text-text-muted">
          {busy === "list-orgs"
            ? "Loading orgs…"
            : "You're not in any orgs yet — usage is org-scoped."}
        </div>
      )}

      {authenticated && orgs.length > 0 && (
        <label className="flex flex-col gap-1.5">
          <span className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
            Org
          </span>
          <select
            value={selectedOrgId}
            onChange={(e) => setSelectedOrgId(e.target.value)}
            className="h-9 px-3 rounded-md border border-border bg-surface font-mono text-sm text-text focus:outline-none focus:border-border-hover"
          >
            {orgs.map((o) => (
              <option key={o.id} value={o.id}>
                {o.name} ({o.role})
              </option>
            ))}
          </select>
        </label>
      )}

      {/* F3.1c-iii UI — Move personal Pro into the selected org.
          Restricted to owner/admin role on that org; cloud also enforces
          this with a 403 so the UI gate mirrors the server gate. The
          action is irreversible from this UI (Stripe Customer Portal can
          re-subscribe), so the ConfirmDialog uses tone=danger. */}
      {authenticated && canTransferPersonal && (
        <div className="flex flex-col gap-1.5 border border-border-soft bg-surface-overlay rounded-md px-3 py-2">
          <span className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
            Personal subscription
          </span>
          <p className="text-xs text-text-muted">
            Cancel your personal Pro subscription so you can switch to{" "}
            <span className="font-semibold">{selectedOrg?.name ?? "this org"}</span>'s
            Team plan via Checkout. Stripe will refund the unused portion at the
            next invoice cycle.
          </p>
          <div className="flex justify-end">
            <Button
              size="sm"
              variant="outline"
              onClick={() => {
                setTransferNote(null);
                setTransferConfirmOpen(true);
              }}
              disabled={busy === "transfer-personal"}
              title="Cancel personal Pro to move to this org's Team plan"
            >
              {busy === "transfer-personal" ? (
                <Loader2 className="size-3.5 animate-spin" />
              ) : (
                <ArrowRight className="size-3.5" />
              )}
              Move my Pro into this org…
            </Button>
          </div>
        </div>
      )}

      {transferNote && (
        <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 text-xs font-mono text-text-muted">
          {transferNote}
        </div>
      )}

      {listError && (
        <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-xs text-text font-mono">
          {listError}
        </div>
      )}

      {exportNote && !listError && (
        <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 text-xs text-text-muted font-mono">
          {exportNote}
        </div>
      )}

      {authenticated && selectedOrgId && members.length === 0 && !listError && (
        <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 text-sm text-text-muted">
          {busy === "load-usage"
            ? "Loading usage…"
            : "No usage recorded for this org in the selected window."}
        </div>
      )}

      {members.length > 0 && (
        <>
          <UsageTotalsRow totals={totals} memberCount={members.length} />
          {projection && (
            <UsageProjectionRow projection={projection} rangeDays={usage?.range_days ?? 30} />
          )}
          <UsageMembersTable members={members} />
        </>
      )}

      <ConfirmDialog
        open={transferConfirmOpen}
        title="Cancel personal Pro?"
        body={
          <>
            <p>
              Your personal Pro subscription will be cancelled in Stripe so you
              can run Checkout for{" "}
              <span className="font-semibold">{selectedOrg?.name ?? "this org"}</span>'s
              Team plan against the org's billing.
            </p>
            <p className="mt-2">
              Stripe will issue a proration credit for the unused portion of
              your Pro period; that credit is applied automatically when the
              next invoice posts.
            </p>
          </>
        }
        confirmLabel="Cancel personal Pro"
        cancelLabel="Keep Pro"
        tone="danger"
        onConfirm={() => void onTransferPersonalConfirm()}
        onCancel={() => setTransferConfirmOpen(false)}
      />
    </section>
  );
}

/**
 * Render `1,234` (or `1,234 (+56 today)` when there's a partial). The
 * Tauri panel's monospace cells don't need a charting library — a
 * dense numeric table is the right shape for a 3-seat team's totals.
 */
function formatUsageCell(rollup: number, partial: number): string {
  const base = rollup.toLocaleString();
  return partial > 0 ? `${base} (+${partial.toLocaleString()} today)` : base;
}

function UsageTotalsRow({
  totals,
  memberCount,
}: {
  totals: { rollup: Map<string, number>; partial: Map<string, number> };
  memberCount: number;
}) {
  const kinds: Array<{ key: string; label: string }> = [
    { key: "query.served", label: "Queries" },
    { key: "index.minutes", label: "Index min" },
    { key: "atlas.queries", label: "Atlas" },
  ];
  return (
    <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 flex items-center gap-4">
      <div className="flex items-center gap-1.5">
        <BarChart3 className="size-3.5 text-text-muted" />
        <span className="font-mono text-xs uppercase tracking-[0.08em] text-text-muted">
          Org total · {memberCount} member{memberCount === 1 ? "" : "s"}
        </span>
      </div>
      <div className="flex gap-4 ml-auto">
        {kinds.map(({ key, label }) => (
          <div key={key} className="flex flex-col items-end">
            <span className="text-xs text-text-muted">{label}</span>
            <span className="font-mono text-xs text-text">
              {formatUsageCell(
                totals.rollup.get(key) ?? 0,
                totals.partial.get(key) ?? 0,
              )}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

/**
 * F3.3c — end-of-month linear projection from the rolled-up window.
 * No $ amount: per-meter overage prices aren't configured yet (the
 * Stripe meter prices live in the dashboard, not in our env), so v0
 * surfaces unit projections only. Finance can multiply against the
 * invoice line items if they need a cost estimate.
 */
function UsageProjectionRow({
  projection,
  rangeDays,
}: {
  projection: Map<string, number>;
  rangeDays: number;
}) {
  const kinds: Array<{ key: string; label: string }> = [
    { key: "query.served", label: "Queries" },
    { key: "index.minutes", label: "Index min" },
    { key: "atlas.queries", label: "Atlas" },
  ];
  return (
    <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 flex items-center gap-4">
      <div className="flex items-center gap-1.5">
        <TrendingUp className="size-3.5 text-text-muted" />
        <span className="font-mono text-xs uppercase tracking-[0.08em] text-text-muted">
          30-day projection
        </span>
      </div>
      <div className="flex gap-4 ml-auto">
        {kinds.map(({ key, label }) => (
          <div key={key} className="flex flex-col items-end">
            <span className="text-xs text-text-muted">{label}</span>
            <span className="font-mono text-xs text-text">
              {(projection.get(key) ?? 0).toLocaleString()}
            </span>
          </div>
        ))}
      </div>
      <span className="font-mono text-[10px] text-text-muted">
        extrapolated from {rangeDays}d
      </span>
    </div>
  );
}

function UsageMembersTable({ members }: { members: MemberTotals[] }) {
  return (
    <div className="rounded-lg border border-border-soft bg-surface overflow-hidden">
      <table className="w-full text-sm">
        <thead className="bg-surface-overlay border-b border-border-soft">
          <tr className="text-left text-xs font-mono uppercase tracking-[0.08em] text-text-muted">
            <th className="px-3 py-2 font-semibold">Member</th>
            <th className="px-3 py-2 font-semibold text-right">Queries</th>
            <th className="px-3 py-2 font-semibold text-right">Index min</th>
            <th className="px-3 py-2 font-semibold text-right">Atlas</th>
          </tr>
        </thead>
        <tbody>
          {members.map((m) => (
            <tr key={m.user_id} className="border-b border-border-soft last:border-b-0">
              <td className="px-3 py-2 align-top">
                <div className="text-xs text-text">{m.email}</div>
                <div className="font-mono text-xs text-text-muted">
                  {m.user_id.slice(0, 8)}…
                </div>
              </td>
              <td className="px-3 py-2 align-top text-right">
                <span className="font-mono text-xs text-text">
                  {formatUsageCell(
                    m.rollup.get("query.served") ?? 0,
                    m.partial.get("query.served") ?? 0,
                  )}
                </span>
              </td>
              <td className="px-3 py-2 align-top text-right">
                <span className="font-mono text-xs text-text">
                  {formatUsageCell(
                    m.rollup.get("index.minutes") ?? 0,
                    m.partial.get("index.minutes") ?? 0,
                  )}
                </span>
              </td>
              <td className="px-3 py-2 align-top text-right">
                <span className="font-mono text-xs text-text">
                  {formatUsageCell(
                    m.rollup.get("atlas.queries") ?? 0,
                    m.partial.get("atlas.queries") ?? 0,
                  )}
                </span>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

interface LabeledInputProps {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  type?: string;
}

function LabeledInput({
  label,
  value,
  onChange,
  placeholder,
  type = "text",
}: LabeledInputProps) {
  return (
    <label className="flex flex-col gap-1.5">
      <span className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
        {label}
      </span>
      <input
        type={type}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className="h-9 px-3 rounded-md border border-border bg-surface font-mono text-sm text-text focus:outline-none focus:border-border-hover"
      />
    </label>
  );
}

interface DialogShellProps {
  title: string;
  hint?: string;
  onClose: () => void;
  children: React.ReactNode;
}

function DialogShell({ title, hint, onClose, children }: DialogShellProps) {
  const containerRef = useDialog(true, onClose);
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-sm p-6">
      <div
        ref={containerRef}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className="w-full max-w-md rounded-lg border border-border bg-surface shadow-xl flex flex-col gap-3 p-5"
      >
        <div className="flex items-start justify-between gap-3">
          <h4 className="font-mono text-sm font-semibold uppercase tracking-[0.08em] text-text">
            {title}
          </h4>
          <Button size="icon-sm" variant="ghost" onClick={onClose}>
            <X className="size-4" />
          </Button>
        </div>
        {hint && <p className="text-xs text-text-muted">{hint}</p>}
        {children}
      </div>
    </div>
  );
}

interface ProgressDrawerProps {
  corpusId: string;
  onClose: () => void;
}

function ProgressDrawer({ corpusId, onClose }: ProgressDrawerProps) {
  const [event, setEvent] = useState<CloudProgressEvent | null>(null);
  const subscribed = useRef(false);

  useEffect(() => {
    if (subscribed.current) return;
    subscribed.current = true;
    const channel = cloudClient.corpusProgress(corpusId);
    channel.onmessage = (msg) => setEvent(msg);
  }, [corpusId]);

  const terminal = event?.status === 2;
  const pct =
    event?.files_total && event.files_processed !== undefined
      ? Math.min(
          100,
          Math.round((event.files_processed / event.files_total) * 100),
        )
      : null;

  return (
    <DialogShell title={`Indexing: ${corpusId}`} onClose={onClose}>
      {!event && (
        <div className="text-sm text-text-muted flex items-center gap-2">
          <Loader2 className="size-3.5 animate-spin" />
          waiting for first event…
        </div>
      )}
      {event && (
        <div className="flex flex-col gap-3 text-sm">
          <div className="flex items-center gap-2">
            <span className="font-mono text-xs uppercase text-text-muted">
              Phase
            </span>
            <span className="font-mono text-text">{event.phase}</span>
            {terminal && (
              <span className="ml-auto text-accent text-xs font-mono">
                ✓ complete
              </span>
            )}
          </div>
          {event.files_total !== undefined && (
            <div className="flex flex-col gap-1">
              <div className="flex items-baseline justify-between font-mono text-xs text-text-muted">
                <span>
                  {event.files_processed ?? 0} / {event.files_total} files
                </span>
                {pct !== null && <span>{pct}%</span>}
              </div>
              <div className="h-1.5 w-full overflow-hidden rounded-full bg-surface-overlay">
                <div
                  className="h-full bg-accent transition-all duration-300"
                  style={{ width: `${pct ?? 0}%` }}
                />
              </div>
            </div>
          )}
          {event.current_file && (
            <div className="text-xs text-text-muted font-mono break-all">
              {event.current_file}
            </div>
          )}
        </div>
      )}
    </DialogShell>
  );
}

interface ConnectionStatusProps {
  status: CloudStatus | null;
  health: CloudHealth | null;
  healthError: string | null;
}

function ConnectionStatus({ status, health, healthError }: ConnectionStatusProps) {
  if (!status?.configured) {
    return (
      <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 text-sm text-text-muted">
        No endpoint configured.
      </div>
    );
  }
  if (healthError) {
    return <ErrorCallout title="Probe failed." message={healthError} />;
  }
  if (health) {
    return (
      <div className="rounded-md border border-border bg-surface px-3 py-2 text-sm flex items-center gap-3">
        <Check className="size-4 text-accent shrink-0" />
        <span className="text-text">{health.status}</span>
        <span className="text-text-muted">·</span>
        <LatencyChip ms={health.latency_ms} />
        <span className="text-text-muted">·</span>
        <span className="font-mono text-xs text-text-muted">v{health.version || "?"}</span>
        <span className="text-text-muted">·</span>
        <span className="text-xs text-text-muted">
          {health.corpus_count} {health.corpus_count === 1 ? "corpus" : "corpora"}
        </span>
      </div>
    );
  }
  return (
    <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 text-sm text-text-muted">
      Not yet probed. Click <span className="font-mono">Ping /healthz</span>.
    </div>
  );
}

function LatencyChip({ ms }: { ms: number }) {
  const tone =
    ms < 150 ? "text-accent" : ms < 500 ? "text-text" : "text-danger";
  return (
    <span className={cn("font-mono text-xs", tone)}>{ms} ms</span>
  );
}

/**
 * Cost/latency badges fed by the F1.4 metering pipeline. Renders
 * three compact chips: queries served today, index-minutes consumed
 * today, and last-probe round-trip latency. Hidden when no usage
 * data is available (self-hosted serve, or the user hasn't been
 * authenticated long enough for the cloud to have any events yet).
 */
function UsageBadges({
  usage,
  latencyMs,
}: {
  usage: CloudUsage | null;
  latencyMs: number | null;
}) {
  const todayTotal = (kind: string): number => {
    if (!usage) return 0;
    return usage.today_partial.find((p) => p.kind === kind)?.total ?? 0;
  };
  const queriesToday = todayTotal("query.served");
  const indexMinutesToday = todayTotal("index.minutes");
  // Render even when usage is null so the surface area is stable —
  // the chips just show "0" / "—" instead of disappearing.
  return (
    <div className="flex flex-wrap items-center gap-3 rounded-md border border-border-soft bg-surface-overlay px-3 py-2 text-xs">
      <UsageChip label="Queries today" value={String(queriesToday)} />
      <span className="text-text-muted">·</span>
      <UsageChip
        label="Index-min today"
        value={String(indexMinutesToday)}
      />
      <span className="text-text-muted">·</span>
      <UsageChip
        label="p50 latency"
        value={latencyMs == null ? "—" : `${latencyMs} ms`}
      />
    </div>
  );
}

/**
 * F2.4 — Plan-tier indicator rendered next to the "signed in" chip.
 * Pinned colour palette per tier so screenshots are visually distinct
 * across pricing-page card mocks.
 */
function PlanBadge({ plan }: { plan: "pro" | "team" | "enterprise" }) {
  const styles: Record<typeof plan, string> = {
    pro: "border-accent/40 bg-accent/10 text-accent",
    team: "border-accent/50 bg-accent/15 text-accent",
    enterprise: "border-accent/60 bg-accent/20 text-accent",
  };
  const label = plan.charAt(0).toUpperCase() + plan.slice(1);
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-md border px-2 py-0.5 text-[10px] uppercase tracking-[0.08em] font-semibold",
        styles[plan],
      )}
    >
      {label}
    </span>
  );
}

function UsageChip({ label, value }: { label: string; value: string }) {
  return (
    <span className="flex items-baseline gap-1.5">
      <span className="font-mono uppercase tracking-[0.08em] text-text-muted">
        {label}
      </span>
      <span className="font-mono text-text">{value}</span>
    </span>
  );
}

// ── F6.2-d — Session inspector ─────────────────────────────────────────────

interface SessionInspectorProps {
  authenticated: boolean;
}

/**
 * F6.2-d — fetch + render a session bundle for replay/audit. v0 takes
 * a manual session_id (no /sessions list endpoint yet) and surfaces
 * the parsed manifest + delivered + drops tables.
 *
 * UX choice: each render is fresh — there's no auto-refresh because
 * sessions are immutable post-export. The Fetch button is the only
 * trigger; results clear on auth flip.
 */
function SessionInspectorSection({ authenticated }: SessionInspectorProps) {
  const [sessionId, setSessionId] = useState("");
  const [bundle, setBundle] = useState<CloudSessionBundle | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // F6.2-e — populated by `listSessions` on auth flip / refresh.
  // Empty array = "list endpoint returned no sessions"; null = "haven't
  // loaded yet" (or load failed). UI falls back to a manual input when
  // either branch leaves us without a dropdown to render.
  const [sessions, setSessions] = useState<CloudSessionSummary[] | null>(null);

  useEffect(() => {
    if (!authenticated) {
      setBundle(null);
      setError(null);
      setSessionId("");
      setSessions(null);
      return;
    }
    let cancelled = false;
    void cloudClient
      .listSessions()
      .then((list) => {
        if (cancelled) return;
        setSessions(list);
        if (list.length > 0 && !sessionId) {
          setSessionId(list[0].session_id);
        }
      })
      .catch(() => {
        // List endpoint failed — fall back to manual input. Not an
        // error worth surfacing inline; the user can still type the id.
        if (!cancelled) setSessions([]);
      });
    return () => {
      cancelled = true;
    };
    // sessionId intentionally excluded — don't re-list on selection.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [authenticated]);

  const refreshSessions = useCallback(async () => {
    if (!authenticated) return;
    try {
      setSessions(await cloudClient.listSessions());
    } catch {
      setSessions([]);
    }
  }, [authenticated]);

  const onFetch = useCallback(async () => {
    const id = sessionId.trim();
    if (!id) return;
    setBusy(true);
    setError(null);
    try {
      setBundle(await cloudClient.fetchSessionBundle(id));
    } catch (e) {
      setError(String(e));
      setBundle(null);
    } finally {
      setBusy(false);
    }
  }, [sessionId]);

  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <h3 className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
          Session inspector
        </h3>
      </div>

      <p className="text-xs text-text-muted">
        Fetch + render a session bundle for replay or audit. The cloud
        builds a tar with{" "}
        <span className="font-mono">manifest.json</span> +{" "}
        <span className="font-mono">delivered.jsonl</span> (+{" "}
        <span className="font-mono">drops.jsonl</span> when a tenant scope
        + ledger are wired); the inspector parses it locally.
      </p>

      {!authenticated && (
        <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 text-sm text-text-muted">
          Sign in to inspect sessions.
        </div>
      )}

      {authenticated && (
        <div className="flex items-end gap-2">
          <label className="flex flex-col gap-1.5 flex-1">
            <span className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
              Session
            </span>
            {sessions && sessions.length > 0 ? (
              <select
                value={sessionId}
                onChange={(e) => setSessionId(e.target.value)}
                className="h-9 px-3 rounded-md border border-border bg-surface font-mono text-sm text-text focus:outline-none focus:border-border-hover"
              >
                {sessions.map((s) => (
                  <option key={s.session_id} value={s.session_id}>
                    {s.session_id} · {s.delivered_count} delivered · {s.budget_used.toLocaleString()} tokens
                  </option>
                ))}
              </select>
            ) : (
              // F6.2-e fallback — manual input when the list endpoint
              // returned empty (no live sessions) or failed.
              <input
                value={sessionId}
                onChange={(e) => setSessionId(e.target.value)}
                placeholder="agent-session-…"
                className="h-9 px-3 rounded-md border border-border bg-surface font-mono text-sm text-text focus:outline-none focus:border-border-hover"
              />
            )}
          </label>
          <Button
            size="sm"
            variant="outline"
            onClick={() => void refreshSessions()}
            disabled={busy}
            title="Re-list sessions on the contacted pod"
          >
            <RefreshCw className="size-3.5" />
            Refresh
          </Button>
          <Button
            size="sm"
            onClick={() => void onFetch()}
            disabled={!sessionId.trim() || busy}
          >
            {busy ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <Download className="size-3.5" />
            )}
            Fetch
          </Button>
        </div>
      )}

      {error && (
        <div className="rounded-md border border-danger/40 bg-danger/10 px-3 py-2 text-xs font-mono text-text">
          {error}
        </div>
      )}

      {bundle && <SessionBundleView bundle={bundle} />}
    </section>
  );
}

function SessionBundleView({ bundle }: { bundle: CloudSessionBundle }) {
  const { manifest, delivered, drops } = bundle;
  return (
    <div className="flex flex-col gap-3">
      <div className="rounded-md border border-border-soft bg-surface-overlay px-3 py-2 flex flex-col gap-1">
        <div className="flex items-baseline gap-1.5">
          <span className="font-mono text-xs uppercase tracking-[0.08em] text-text-muted">
            Session
          </span>
          <span className="font-mono text-xs text-text">{manifest.session_id}</span>
        </div>
        <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs">
          <UsageChip
            label="opened"
            value={manifest.opened_at}
          />
          <UsageChip
            label="exported"
            value={manifest.exported_at}
          />
          <UsageChip
            label="budget"
            value={manifest.budget_used.toLocaleString()}
          />
          <UsageChip
            label="delivered"
            value={manifest.delivered_count.toLocaleString()}
          />
          <UsageChip
            label="tokens"
            value={manifest.total_delivered_tokens.toLocaleString()}
          />
          <UsageChip
            label="pressure"
            value={manifest.pressure_level}
          />
        </div>
      </div>

      <div className="flex flex-col gap-1.5">
        <span className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
          Delivered ({delivered.length})
        </span>
        {delivered.length === 0 ? (
          <div className="text-xs text-text-muted">
            No deliveries recorded.
          </div>
        ) : (
          <div className="max-h-64 overflow-y-auto rounded-lg border border-border-soft">
            <table className="w-full text-xs font-mono">
              <thead className="bg-surface-overlay text-text-muted">
                <tr>
                  <th className="text-left px-2 py-1">Turn</th>
                  <th className="text-left px-2 py-1">Content ID</th>
                  <th className="text-right px-2 py-1">Tokens</th>
                  <th className="text-left px-2 py-1">Tier</th>
                </tr>
              </thead>
              <tbody>
                {delivered.map((d, i) => (
                  <tr key={`${d.content_id}-${i}`} className="border-t border-border-soft">
                    <td className="px-2 py-1 text-text-muted">{d.turn_delivered}</td>
                    <td className="px-2 py-1 text-text break-all">{d.content_id}</td>
                    <td className="px-2 py-1 text-right text-text">
                      {d.token_count.toLocaleString()}
                    </td>
                    <td className="px-2 py-1 text-text-muted">{d.compression_tier}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      <div className="flex flex-col gap-1.5">
        <span className="font-mono text-xs font-semibold uppercase tracking-[0.08em] text-text-muted">
          Drops{" "}
          {drops === undefined
            ? "(ledger not wired)"
            : `(${drops.length})`}
        </span>
        {drops === undefined ? (
          <div className="text-xs text-text-muted">
            Drops ledger not consulted — either self-hosted serve or no
            tenant scope on the server side.
          </div>
        ) : drops.length === 0 ? (
          <div className="text-xs text-text-muted">
            No evictions recorded for this session.
          </div>
        ) : (
          <div className="max-h-64 overflow-y-auto rounded-lg border border-border-soft">
            <table className="w-full text-xs font-mono">
              <thead className="bg-surface-overlay text-text-muted">
                <tr>
                  <th className="text-left px-2 py-1">Evicted</th>
                  <th className="text-left px-2 py-1">Claim ID</th>
                </tr>
              </thead>
              <tbody>
                {drops.map((d, i) => (
                  <tr key={`${d.claim_id}-${i}`} className="border-t border-border-soft">
                    <td className="px-2 py-1 text-text-muted">{d.evicted_at}</td>
                    <td className="px-2 py-1 text-text break-all">{d.claim_id}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}
