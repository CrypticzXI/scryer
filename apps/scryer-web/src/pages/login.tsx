import { useCallback, useEffect, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { Loader2 } from "lucide-react";
import { useAuth } from "@/lib/hooks/use-auth";
import { Input } from "@/components/ui/input";
import { useBackendRestarting } from "@/lib/hooks/use-backend-restarting";
import { BackendRestartOverlay } from "@/components/common/backend-restart-overlay";

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
  const { login, user, loading: authLoading } = useAuth();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
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
        setError(err instanceof Error ? err.message : "Invalid username or password.");
      } finally {
        setSubmitting(false);
      }
    },
    [login, navigate, password, redirectTarget, username],
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
        <h1 className="text-center text-xl font-semibold tracking-tight">Sign in</h1>

        {error && (
          <div className="rounded-md bg-red-900/40 px-3 py-2 text-sm text-red-300">{error}</div>
        )}

        <div className="space-y-1.5">
          <label htmlFor="username" className="block text-sm font-medium text-muted-foreground">
            Username
          </label>
          <Input
            id="username"
            type="text"
            autoComplete="username"
            autoFocus
            required
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            placeholder="Username"
          />
        </div>

        <div className="space-y-1.5">
          <label htmlFor="password" className="block text-sm font-medium text-muted-foreground">
            Password
          </label>
          <Input
            id="password"
            type="password"
            autoComplete="current-password"
            required
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder="Password"
          />
        </div>

        <button
          id="login-submit"
          type="submit"
          disabled={submitting}
          className="flex w-full items-center justify-center gap-2 rounded-md bg-emerald-600 px-4 py-2 text-sm font-medium text-foreground hover:bg-emerald-500 disabled:opacity-50"
        >
          {submitting && <Loader2 className="h-4 w-4 animate-spin" />}
          {submitting ? "Signing in..." : "Sign in"}
        </button>
      </form>
    </div>
  );
}
