import { useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { getAuthToken } from "@/lib/hooks/use-auth";
import { getRuntimeBackendUrl, getRuntimeBasePath } from "@/lib/runtime-config";
import { selectorId } from "@/lib/utils/dom-ids";

const CLIENT_NAMES: Record<string, string> = {
  "generic-native": "Generic native integration",
  e2e: "Scryer E2E OAuth client",
};

function loginUrl() {
  const basePath = getRuntimeBasePath();
  const loginPath = basePath === "/" ? "/login" : `${basePath}/login`;
  const path =
    basePath !== "/" && window.location.pathname.startsWith(basePath)
      ? window.location.pathname.slice(basePath.length) || "/"
      : window.location.pathname;
  const redirect = `${path}${window.location.search}${window.location.hash}`;
  const params = new URLSearchParams({ redirect });
  return `${loginPath}?${params.toString()}`;
}

export default function OAuthAuthorizePage() {
  const params = useMemo(() => new URLSearchParams(window.location.search), []);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const clientId = params.get("client_id") ?? "";
  const redirectUri = params.get("redirect_uri") ?? "";
  const clientName = CLIENT_NAMES[clientId] ?? clientId;
  const token = getAuthToken();

  const decide = async (approved: boolean) => {
    setBusy(true);
    setError(null);
    try {
      if (!token) {
        window.location.assign(loginUrl());
        return;
      }
      const response = await fetch(getRuntimeBackendUrl("/oauth/authorize/decision"), {
        method: "POST",
        headers: {
          authorization: `Bearer ${token}`,
          "content-type": "application/json",
        },
        body: JSON.stringify({
          approved,
          responseType: params.get("response_type") ?? "",
          clientId,
          redirectUri,
          codeChallenge: params.get("code_challenge") ?? "",
          codeChallengeMethod: params.get("code_challenge_method") ?? "",
          scope: params.get("scope"),
          state: params.get("state"),
        }),
      });
      const body = (await response.json().catch(() => null)) as
        | { redirectUri?: string; errorDescription?: string; error_description?: string }
        | null;
      if (!response.ok || !body?.redirectUri) {
        setError(
          body?.error_description ??
            body?.errorDescription ??
            "Unable to authorize this integration.",
        );
        return;
      }
      window.location.assign(body.redirectUri);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Unable to authorize this integration.");
    } finally {
      setBusy(false);
    }
  };

  if (!token) {
    return (
      <main className="flex min-h-screen items-center justify-center bg-background p-6 text-foreground">
        <div className="grid w-full max-w-lg gap-4 rounded-md border border-border bg-card p-6 shadow-sm">
          <h1 id={selectorId("oauth-authorize-heading")} className="text-xl font-semibold">
            Authorize {clientName || "integration"}
          </h1>
          <p className="text-sm text-muted-foreground">Sign in to continue OAuth authorization.</p>
          <Button
            id={selectorId("oauth-authorize-sign-in")}
            onClick={() => window.location.assign(loginUrl())}
          >
            Sign in
          </Button>
        </div>
      </main>
    );
  }

  return (
    <main className="flex min-h-screen items-center justify-center bg-background p-6 text-foreground">
      <div className="grid w-full max-w-xl gap-5 rounded-md border border-border bg-card p-6 shadow-sm">
        <div className="space-y-1">
          <h1 id={selectorId("oauth-authorize-heading")} className="text-xl font-semibold">
            Authorize {clientName}
          </h1>
          <p className="break-all text-sm text-muted-foreground">{redirectUri}</p>
        </div>
        <div className="grid gap-2 text-sm">
          <p>Can access Scryer as you, limited to your library permissions.</p>
          <p>Cannot manage users, settings, backups, security, or app configuration.</p>
        </div>
        {error ? <p className="text-sm text-destructive">{error}</p> : null}
        <div className="flex flex-wrap gap-2">
          <Button
            id={selectorId("oauth-authorize-approve")}
            disabled={busy}
            onClick={() => decide(true)}
          >
            Authorize
          </Button>
          <Button
            id={selectorId("oauth-authorize-deny")}
            variant="outline"
            disabled={busy}
            onClick={() => decide(false)}
          >
            Deny
          </Button>
        </div>
      </div>
    </main>
  );
}
