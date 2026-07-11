import { lazy, Suspense } from "react";
import { createBrowserRouter, Navigate, useLocation } from "react-router-dom";
import { PageShellFallback } from "@/components/root/page-shell-fallback";
import { getRuntimeBasePath } from "@/lib/runtime-config";
import { resolveAppRoute } from "@/lib/utils/routing";
import { RouteErrorBoundary } from "./error-boundary";

const RootPageShell = lazy(() => import("@/components/root/root-page-shell"));
const LoginPage = lazy(() => import("@/src/pages/login"));
const OAuthAuthorizePage = lazy(() => import("@/src/pages/oauth-authorize"));
const SetupPage = lazy(() => import("@/src/pages/setup"));

function ShellRoute() {
  const location = useLocation();
  const resolution = resolveAppRoute(
    location.pathname,
    location.search,
    location.hash,
  );

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
