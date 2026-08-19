import { lazy, Suspense, useCallback, useMemo } from "react";
import {
  createBrowserRouter,
  Navigate,
  Outlet,
  useLocation,
  useSearchParams,
} from "react-router";
import { PageShellFallback } from "@/components/root/page-shell-fallback";
import type { Translate } from "@/components/root/types";
import { URL_PARAM_LANGUAGE } from "@/lib/constants/settings";
import { TranslateContext } from "@/lib/context/translate-context";
import { DEFAULT_LANGUAGE, t as translate } from "@/lib/i18n";
import {
  isLocaleSupported,
  readStoredLanguageCode,
} from "@/lib/hooks/use-language";
import { getRuntimeBasePath } from "@/lib/runtime-config";
import { parseLanguageFromParam, resolveAppRoute } from "@/lib/utils/routing";
import { RouteErrorBoundary } from "./error-boundary";

const RootPageShell = lazy(() => import("@/components/root/root-page-shell"));
const LoginPage = lazy(() => import("@/src/pages/login"));
const OAuthAuthorizePage = lazy(() => import("@/src/pages/oauth-authorize"));
const SetupPage = lazy(() => import("@/src/pages/setup"));

// Root-level translate fallback. TranslateContext.Provider historically only
// mounted inside RootPageShell, but /login, /setup, and /oauth/authorize are
// SIBLING routes of the shell — any useTranslate() consumer reached from them
// (e.g. the setup wizard's folder browser) threw and blanked the page behind
// the error boundary. This provider derives the language exactly the way
// useLanguage() does pre-auth (?lang= param, then stored/browser locale, then
// default) and covers every route; the shell's own provider mounts deeper in
// the tree, so authenticated language switching is unchanged.
function RootTranslateBoundary() {
  const [searchParams] = useSearchParams();
  const uiLanguage = useMemo(() => {
    const fromQuery = parseLanguageFromParam(
      searchParams.get(URL_PARAM_LANGUAGE),
    );
    if (fromQuery) {
      return fromQuery;
    }
    const stored = readStoredLanguageCode();
    return isLocaleSupported(stored) ? stored : DEFAULT_LANGUAGE;
  }, [searchParams]);
  const t = useCallback<Translate>(
    (key, values) => translate(key, uiLanguage, values),
    [uiLanguage],
  );

  return (
    <TranslateContext.Provider value={t}>
      <Outlet />
    </TranslateContext.Provider>
  );
}

function ShellRoute() {
  const location = useLocation();
  const resolution = resolveAppRoute(
    location.pathname,
    location.search,
    location.hash,
  );

  // "landing" (`/`) deliberately falls through to the shell: the destination
  // depends on the signed-in user's permissions, which are not known until the
  // shell's auth bootstrap resolves.
  if (resolution.kind === "redirect") {
    return <Navigate to={resolution.to} replace />;
  }

  return (
    <Suspense fallback={<PageShellFallback />}>
      <RootPageShell />
    </Suspense>
  );
}

export const router = createBrowserRouter(
  [
    {
      element: <RootTranslateBoundary />,
      errorElement: <RouteErrorBoundary />,
      children: [
        {
          path: "/login",
          element: (
            <Suspense fallback={<PageShellFallback />}>
              <LoginPage />
            </Suspense>
          ),
        },
        {
          path: "/setup",
          element: (
            <Suspense fallback={<PageShellFallback />}>
              <SetupPage />
            </Suspense>
          ),
        },
        {
          path: "/oauth/authorize",
          element: (
            <Suspense fallback={<PageShellFallback />}>
              <OAuthAuthorizePage />
            </Suspense>
          ),
        },
        { path: "*", element: <ShellRoute /> },
      ],
    },
  ],
  { basename: getRuntimeBasePath() },
);
