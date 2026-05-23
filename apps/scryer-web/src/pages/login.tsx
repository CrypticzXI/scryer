import { useCallback, useEffect, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { Loader2 } from "lucide-react";
import { useAuth } from "@/lib/hooks/use-auth";
import { useLanguage } from "@/lib/hooks/use-language";
import { Input } from "@/components/ui/input";
import { useBackendRestarting } from "@/lib/hooks/use-backend-restarting";
import { BackendRestartOverlay } from "@/components/common/backend-restart-overlay";
import { authenticateWithPasskey, PasskeyClientError } from "@/lib/utils/passkeys";

function resolveRedirectTarget(value: string | null): string {
  if (!value || !value.startsWith("/") || value.startsWith("//")) {
    return "/";
  }

  return value;
}

export default function LoginPage() {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const { serviceRestarting } = useBackendRestarting();
  const { t } = useLanguage(searchParams);
  const { login, adoptSession, user, loading: authLoading, passkeyEnabled } = useAuth();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [passkeySubmitting, setPasskeySubmitting] = useState(false);
  const redirectTarget = resolveRedirectTarget(searchParams.get("redirect"));

  // Redirect to home if already authenticated
  useEffect(() => {
    if (!serviceRestarting && !authLoading && user) {
      navigate(redirectTarget, { replace: true });
    }
  }, [authLoading, user, navigate, redirectTarget, serviceRestarting]);

  const handleSubmit = useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      setError(null);
      setSubmitting(true);
      try {
        await login(username, password);
        navigate(redirectTarget, { replace: true });
      } catch (err) {
        setError(err instanceof Error ? err.message : t("auth.invalidCredentials"));
      } finally {
        setSubmitting(false);
      }
    },
    [login, navigate, password, redirectTarget, t, username],
  );

  const handlePasskeySignIn = useCallback(
    async () => {
      setError(null);
      setPasskeySubmitting(true);
      try {
        const result = await authenticateWithPasskey();
        adoptSession(result.token, result.user);
        navigate(redirectTarget, { replace: true });
      } catch (err) {
        if (err instanceof PasskeyClientError) {
          if (err.code === "unsupported") {
            setError(t("auth.passkeyUnsupported"));
          } else if (err.code === "cancelled") {
            setError(t("auth.passkeyCancelled"));
          } else {
            setError(err.message || t("auth.passkeyFailed"));
          }
          return;
        }

        setError(err instanceof Error ? err.message : t("auth.passkeyFailed"));
      } finally {
        setPasskeySubmitting(false);
      }
    },
    [adoptSession, navigate, redirectTarget, t],
  );

  if (serviceRestarting) {
    return <BackendRestartOverlay />;
  }

  if (authLoading) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-background text-card-foreground">
        <Loader2 className="h-6 w-6 animate-spin text-emerald-700 dark:text-emerald-300" />
      </div>
    );
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-background p-4 text-foreground">
      <form
        id="login-form"
        onSubmit={handleSubmit}
        className="w-full max-w-sm space-y-5 rounded-lg border border-border bg-card/70 p-8"
      >
        <h1 className="text-center text-xl font-semibold tracking-tight">{t("auth.signIn")}</h1>

        {error && (
          <div className="rounded-md bg-red-900/40 px-3 py-2 text-sm text-red-300">{error}</div>
        )}

        <div className="space-y-1.5">
          <label htmlFor="username" className="block text-sm font-medium text-muted-foreground">
            {t("auth.username")}
          </label>
          <Input
            id="username"
            type="text"
            autoComplete="username"
            autoFocus
            required
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            placeholder={t("auth.username")}
          />
        </div>

        <div className="space-y-1.5">
          <label htmlFor="password" className="block text-sm font-medium text-muted-foreground">
            {t("auth.password")}
          </label>
          <Input
            id="password"
            type="password"
            autoComplete="current-password"
            required
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder={t("auth.password")}
          />
        </div>

        <button
          id="login-submit"
          type="submit"
          disabled={submitting || passkeySubmitting}
          className="flex w-full items-center justify-center gap-2 rounded-md bg-emerald-600 px-4 py-2 text-sm font-medium text-foreground hover:bg-emerald-500 disabled:opacity-50"
        >
          {submitting && <Loader2 className="h-4 w-4 animate-spin" />}
          {submitting ? t("auth.signingIn") : t("auth.signIn")}
        </button>

        {passkeyEnabled ? (
          <>
            <div className="flex items-center gap-3 text-xs uppercase tracking-[0.2em] text-muted-foreground">
              <div className="h-px flex-1 bg-border" />
              <span>{t("label.or")}</span>
              <div className="h-px flex-1 bg-border" />
            </div>
            <button
              type="button"
              onClick={handlePasskeySignIn}
              disabled={submitting || passkeySubmitting}
              className="flex w-full items-center justify-center gap-2 rounded-md border border-border bg-background px-4 py-2 text-sm font-medium text-foreground hover:bg-muted disabled:opacity-50"
            >
              {passkeySubmitting ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
              {passkeySubmitting ? t("auth.passkeySigningIn") : t("auth.signInWithPasskey")}
            </button>
          </>
        ) : null}
      </form>
    </div>
  );
}
