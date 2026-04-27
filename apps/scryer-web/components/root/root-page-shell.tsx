import { lazy, Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { ActivitySquare, CalendarDays, Download, History, ListChecks, Loader2, MonitorCog, Settings, WifiOff, X } from "lucide-react";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";
import { useAuth } from "@/lib/hooks/use-auth";

import { TranslateContext } from "@/lib/context/translate-context";
import { GlobalStatusContext } from "@/lib/context/global-status-context";
import { RootHeader } from "@/components/root/root-header";
import { JobRunProvider } from "@/components/root/job-run-provider";
import { LibraryScanProgressProvider } from "@/components/root/library-scan-progress-provider";
import { ReactiveRefreshProvider } from "@/components/root/reactive-refresh-provider";
import { RootSidebar } from "@/components/root/root-sidebar";
import { ViewLoadingFallback } from "@/components/common/view-loading-fallback";
import { buildRouteCommands } from "@/components/root/route-commands";
import { GlobalSearchProvider } from "@/components/root/global-search-provider";

import { useGlobalStatusToast } from "@/lib/hooks/use-global-status-toast";
import { useLanguage } from "@/lib/hooks/use-language";
import { ScryerGraphqlProvider } from "@/lib/graphql/urql-provider";
import { backendClient } from "@/lib/graphql/urql-client";
import { useOnlineStatus } from "@/lib/hooks/use-online-status";
import { useInstallPrompt } from "@/lib/hooks/use-install-prompt";
import { useBackendRestarting } from "@/lib/hooks/use-backend-restarting";
import type {
  ActivitySection,
  ViewId,
  SettingsSection,
  ContentSettingsSection,
  OverviewTitleTarget,
  SystemSection,
  WantedSection,
} from "@/components/root/types";
import type { Facet } from "@/lib/types";
import {
  URL_PARAM_CONTENT_SECTION_DEPRECATED,
  URL_PARAM_LANGUAGE,
  URL_PARAM_SETTINGS_SECTION_DEPRECATED,
  URL_PARAM_VIEW_DEPRECATED,
} from "@/lib/constants/settings";
import { AVAILABLE_LANGUAGES } from "@/lib/i18n";
import type { LocaleCode, LanguageOption } from "@/lib/i18n";

import {
  buildOverviewDetailPath,
  buildViewPath,
  parseActivitySectionFromPath,
  parseContentSectionFromPath,
  parseOverviewSlugFromPath,
  parseSettingsSectionFromPath,
  parseSystemSectionFromPath,
  parseViewFromPath,
  parseWantedSectionFromPath,
} from "@/lib/utils/routing";
import { FACET_REGISTRY, isMediaView, facetForView } from "@/lib/facets/registry";
import { BackendRestartOverlay } from "@/components/common/backend-restart-overlay";
import {
  importQueueCountQuery,
  pendingImportsQuery,
  pendingImportCountsQuery,
} from "@/lib/graphql/queries";
import { hasImportItemsForView, type PendingImportCounts } from "@/lib/types";
import { resolveTitleOverviewTargetBySlug } from "@/lib/title-overview-loader";

const IMPORT_COUNT_FACETS = ["movie", "series", "anime"] as const;

const mediaContainers = () => import("@/components/containers/media-containers");

const MediaContentContainer = lazy(() =>
  mediaContainers().then((m) => ({ default: m.MediaContentContainer })),
);

const MovieOverviewContainer = lazy(() =>
  mediaContainers().then((m) => ({ default: m.MovieOverviewContainer })),
);

const SeriesOverviewContainer = lazy(() =>
  mediaContainers().then((m) => ({ default: m.SeriesOverviewContainer })),
);

const SettingsContainer = lazy(() =>
  import("@/components/containers/settings/settings-container").then((m) => ({ default: m.SettingsContainer })),
);

const ActivityContainer = lazy(() =>
  import("@/components/containers/activity-container").then((m) => ({ default: m.ActivityContainer })),
);

const SystemContainer = lazy(() =>
  import("@/components/containers/system-container").then((m) => ({ default: m.SystemContainer })),
);

const WantedContainer = lazy(() =>
  import("@/components/containers/wanted-container").then((m) => ({ default: m.WantedContainer })),
);

const CalendarContainer = lazy(() =>
  import("@/components/containers/calendar-container").then((m) => ({ default: m.CalendarContainer })),
);

const ImportHistoryContainer = lazy(() =>
  import("@/components/containers/import-history-container").then((m) => ({ default: m.ImportHistoryContainer })),
);

const PendingImportsContainer = lazy(() =>
  import("@/components/containers/pending-imports-container").then((m) => ({ default: m.PendingImportsContainer })),
);

const INSTALL_BANNER_DISMISSED_KEY = "scryer.pwa.installBannerDismissed";

function OverviewContainerForView({
  view,
  initialEpisodeId,
  onTitleResolved,
  ...props
}: {
  view: ViewId;
  titleId: string;
  onBackToList: () => void;
  onTitleNotFound: () => void;
  onTitleResolved?: (title: OverviewTitleTarget) => void;
  initialEpisodeId?: string | null;
}) {
  const facet = facetForView(view);
  if (facet?.hasEpisodes) {
    return (
      <SeriesOverviewContainer
        {...props}
        initialEpisodeId={initialEpisodeId}
        onTitleResolved={onTitleResolved}
      />
    );
  }
  return <MovieOverviewContainer {...props} onTitleResolved={onTitleResolved} />;
}

type OverviewNavigationState = {
  scryerOverviewTarget?: {
    view?: unknown;
    id?: unknown;
    slug?: unknown;
  };
};

function readOverviewTargetFromLocationState(
  state: unknown,
  view: ViewId,
  parsedOverviewSlug: string | null,
): OverviewTitleTarget | null {
  if (!parsedOverviewSlug || state == null || typeof state !== "object") {
    return null;
  }

  const overviewState = state as OverviewNavigationState;
  const target = overviewState.scryerOverviewTarget;
  if (!target || target.view !== view) {
    return null;
  }

  const id = typeof target.id === "string" ? target.id.trim() : "";
  const slug = typeof target.slug === "string" ? target.slug.trim() : "";
  if (!id || !slug || slug !== parsedOverviewSlug) {
    return null;
  }

  return { id, slug };
}

/**
 * Renders the main content area.
 */
function MainContent({
  view,
  overviewTitleId,
  overviewLoading,
  overviewEpisodeId,
  handleBackToList,
  handleTitleNotFound,
  handleOverviewTitleResolved,
  settingsSection,
  userId,
  username,
  selectedLanguage,
  uiLanguage,
  setLanguagePreferenceFromShell,
  contentSettingsSection,
  systemSection,
  activitySection,
  wantedSection,
  handleOpenOverview,
}: {
  view: ViewId;
  overviewTitleId: string | null;
  overviewLoading: boolean;
  overviewEpisodeId: string | null;
  handleBackToList: () => void;
  handleTitleNotFound: () => void;
  handleOverviewTitleResolved: (title: OverviewTitleTarget) => void;
  settingsSection: SettingsSection;
  userId: string | undefined;
  username: string | undefined;
  selectedLanguage: LanguageOption;
  uiLanguage: LocaleCode;
  setLanguagePreferenceFromShell: (code: string) => void;
  contentSettingsSection: ContentSettingsSection;
  systemSection: SystemSection;
  activitySection: ActivitySection;
  wantedSection: WantedSection;
  handleOpenOverview: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
    episodeId?: string,
  ) => void;
}) {
  if (view === "activity") {
    return <ActivityContainer key="activity" activitySection={activitySection} />;
  }
  if (view === "calendar") {
    return <CalendarContainer key="calendar" onOpenOverview={handleOpenOverview} />;
  }
  if (view === "wanted") {
    return <WantedContainer key={`wanted-${wantedSection}`} wantedSection={wantedSection} />;
  }
  if (view === "history") {
    return <ImportHistoryContainer key="history" />;
  }
  if (view === "system") {
    return <SystemContainer key={`system-${systemSection}`} systemSection={systemSection} />;
  }
  if (isMediaView(view) && contentSettingsSection === "import") {
    return <PendingImportsContainer key={`${view}-imports`} view={view} />;
  }
  if (isMediaView(view) && contentSettingsSection === "overview" && overviewLoading) {
    return <ViewLoadingFallback />;
  }
  if (isMediaView(view) && overviewTitleId) {
    return (
      <OverviewContainerForView
        key={`${view}-overview-${overviewTitleId}`}
        view={view}
        titleId={overviewTitleId}
        initialEpisodeId={overviewEpisodeId}
        onBackToList={handleBackToList}
        onTitleNotFound={handleTitleNotFound}
        onTitleResolved={handleOverviewTitleResolved}
      />
    );
  }
  if (view === "settings") {
    return (
      <SettingsContainer
        key="settings"
        settingsSection={settingsSection}
        userId={userId}
        username={username}
        availableLanguages={AVAILABLE_LANGUAGES}
        selectedLanguage={selectedLanguage}
        uiLanguage={uiLanguage}
        onSelectLanguage={setLanguagePreferenceFromShell}
      />
    );
  }
  return (
    <MediaContentContainer
      key={`${view}-${contentSettingsSection}`}
      view={view}
      contentSettingsSection={contentSettingsSection}
      onOpenOverview={handleOpenOverview}
    />
  );
}

export default function HomePage() {
  const { serviceRestarting, setServiceRestarting } = useBackendRestarting();
  const { user, loading: authLoading } = useAuth();
  const navigate = useNavigate();
  const [setupChecked, setSetupChecked] = useState(false);

  useEffect(() => {
    if (!serviceRestarting && !authLoading && !user) {
      navigate("/login", { replace: true });
    }
  }, [authLoading, user, navigate, serviceRestarting]);

  // Check if setup wizard needs to run (first-run detection).
  useEffect(() => {
    if (serviceRestarting || authLoading || !user || setupChecked) return;
    (async () => {
      try {
        const { data } = await import("@/lib/graphql/urql-client").then(
          (mod) => mod.backendClient.query(
            `query SetupStatus { setupStatus { setupComplete } }`,
            {},
          ).toPromise(),
        );
        if (data?.setupStatus?.setupComplete === false) {
          navigate("/setup", { replace: true });
          return;
        }
      } catch {
        // If the query fails (e.g., old backend), skip the check
      }
      setSetupChecked(true);
    })();
  }, [authLoading, user, setupChecked, navigate, serviceRestarting]);

  if (serviceRestarting) {
    return <BackendRestartOverlay />;
  }

  if (authLoading || (!setupChecked && user)) {
    return (
        <div className="flex min-h-screen items-center justify-center bg-background text-foreground">
        <Loader2 className="h-6 w-6 animate-spin text-emerald-700 dark:text-emerald-300" />
      </div>
    );
  }

  if (!user) {
    return null;
  }

  return (
    <AuthenticatedHomePage
      serviceRestarting={serviceRestarting}
      setServiceRestarting={setServiceRestarting}
    />
  );
}

function AuthenticatedHomePage({
  serviceRestarting,
  setServiceRestarting,
}: {
  serviceRestarting: boolean;
  setServiceRestarting: (value: boolean) => void;
}) {
  const { user } = useAuth();
  const isOnline = useOnlineStatus();
  const { canPrompt, isInstalled, isIosSafari, promptInstall } = useInstallPrompt();

  const location = useLocation();
  const { pathname } = location;
  const [searchParams] = useSearchParams();
  const navigate = useNavigate();

  const {
    parsedView: view,
    parsedSettingsSection: settingsSection,
    parsedContentSection: contentSettingsSection,
    parsedSystemSection: systemSection,
    parsedActivitySection: activitySection,
    parsedWantedSection: wantedSection,
    parsedOverviewSlug,
  } =
    useMemo(() => {
      const trimmed = pathname.replace(/^\/+|\/+$/g, "");
      const segments = trimmed ? trimmed.split("/") : [];
      const normalizedSegments = segments.map((segment) => segment.toLowerCase());
      const parsedView = parseViewFromPath(normalizedSegments[0]);
      const parsedSettingsSection: SettingsSection = parsedView === "settings"
        ? parseSettingsSectionFromPath(normalizedSegments[1] ?? null)
        : "general";
      const parsedContentSection: ContentSettingsSection = isMediaView(parsedView)
        ? parseContentSectionFromPath(normalizedSegments[1] ?? null, normalizedSegments[2] ?? null)
        : "overview";
      const parsedSystemSection: SystemSection = parsedView === "system"
        ? parseSystemSectionFromPath(normalizedSegments[1] ?? null)
        : "overview";
      const parsedActivitySection: ActivitySection = parsedView === "activity"
        ? parseActivitySectionFromPath(normalizedSegments[1] ?? null)
        : "activity";
      const parsedWantedSection: WantedSection = parsedView === "wanted"
        ? parseWantedSectionFromPath(normalizedSegments[1] ?? null)
        : "wanted";
      const parsedOverviewSlug = isMediaView(parsedView) && parsedContentSection === "overview"
        ? parseOverviewSlugFromPath(segments[1] ?? null, segments[2] ?? null)
        : null;
      return {
        parsedView,
        parsedSettingsSection,
        parsedContentSection,
        parsedSystemSection,
        parsedActivitySection,
        parsedWantedSection,
        parsedOverviewSlug,
      };
    }, [pathname]);

  const legacyOverviewTitleId = useMemo(() => {
    if (!isMediaView(view) || contentSettingsSection !== "overview" || parsedOverviewSlug) return null;
    return searchParams.get("id")?.trim() || null;
  }, [view, contentSettingsSection, parsedOverviewSlug, searchParams]);

  const navigationOverviewTarget = useMemo(
    () => readOverviewTargetFromLocationState(location.state, view, parsedOverviewSlug),
    [location.state, parsedOverviewSlug, view],
  );

  const overviewEpisodeId = useMemo(() =>
    searchParams.get("episodeId")?.trim() || null, [searchParams]);

  const {
    uiLanguage,
    setLanguagePreference,
    selectedLanguage,
    t,
    getLanguageLabel,
  } = useLanguage(searchParams);

  const [, setGlobalStatusRaw] = useState("");
  const setGlobalStatus = useGlobalStatusToast(setGlobalStatusRaw, {
    onServiceRestarting: useCallback(() => setServiceRestarting(true), [setServiceRestarting]),
  });
  const [pendingImportCounts, setPendingImportCounts] = useState<PendingImportCounts | null>(null);
  const [ignoredImportCounts, setIgnoredImportCounts] = useState<PendingImportCounts | null>(null);
  const [manualImportRequiredCount, setManualImportRequiredCount] = useState(0);
  const [resolvedOverviewTarget, setResolvedOverviewTarget] = useState<OverviewTitleTarget | null>(null);
  const [overviewSlugLoading, setOverviewSlugLoading] = useState(false);

  const setLanguagePreferenceFromShell = useCallback(
    (code: string) => {
      setLanguagePreference(code);
      setGlobalStatus(t("status.languageChanged", { language: getLanguageLabel(code) }));
    },
    [getLanguageLabel, setLanguagePreference, t, setGlobalStatus],
  );

  const [installBannerDismissed, setInstallBannerDismissed] = useState(() => {
    if (typeof window === "undefined") {
      return false;
    }

    return window.localStorage.getItem(INSTALL_BANNER_DISMISSED_KEY) === "true";
  });
  const showInstallBanner = !isInstalled && !installBannerDismissed && (canPrompt || isIosSafari);

  useEffect(() => {
    if (!isInstalled || typeof window === "undefined") {
      return;
    }

    window.localStorage.removeItem(INSTALL_BANNER_DISMISSED_KEY);
    setInstallBannerDismissed(false);
  }, [isInstalled]);

  const dismissInstallBanner = useCallback(() => {
    setInstallBannerDismissed(true);
    if (typeof window !== "undefined") {
      window.localStorage.setItem(INSTALL_BANNER_DISMISSED_KEY, "true");
    }
  }, []);

  const refreshSidebarCounts = useCallback(async () => {
    try {
      const [pendingImportsResult, manualImportCountResult, ignoredImportResults] = await Promise.all([
        backendClient.query(pendingImportCountsQuery, {}).toPromise(),
        backendClient.query(importQueueCountQuery, {}).toPromise(),
        Promise.all(
          IMPORT_COUNT_FACETS.map((facet) =>
            backendClient
              .query(pendingImportsQuery, {
                facet,
                status: "ignored",
                limit: 1,
                offset: 0,
              })
              .toPromise(),
          ),
        ),
      ]);

      if (pendingImportsResult.error) {
        throw pendingImportsResult.error;
      }

      if (manualImportCountResult.error) {
        throw manualImportCountResult.error;
      }

      for (const result of ignoredImportResults) {
        if (result.error) {
          throw result.error;
        }
      }

      if (pendingImportsResult.data?.pendingImportCounts) {
        setPendingImportCounts(pendingImportsResult.data.pendingImportCounts as PendingImportCounts);
      } else {
        setPendingImportCounts({ movie: 0, series: 0, anime: 0 });
      }

      setIgnoredImportCounts({
        movie: Number(ignoredImportResults[0]?.data?.pendingImports?.total ?? 0),
        series: Number(ignoredImportResults[1]?.data?.pendingImports?.total ?? 0),
        anime: Number(ignoredImportResults[2]?.data?.pendingImports?.total ?? 0),
      });

      setManualImportRequiredCount(
        Number(manualImportCountResult.data?.downloadImport?.totalCount ?? 0),
      );
    } catch {
      setPendingImportCounts({ movie: 0, series: 0, anime: 0 });
      setIgnoredImportCounts({ movie: 0, series: 0, anime: 0 });
      setManualImportRequiredCount(0);
    }
  }, []);

  useEffect(() => {
    void refreshSidebarCounts();
  }, [refreshSidebarCounts]);

  useEffect(() => {
    const handleSidebarCountsRefresh = (event: Event) => {
      const delta =
        event instanceof CustomEvent && typeof event.detail?.delta === "number"
          ? event.detail.delta
          : 0;
      if (delta !== 0) {
        setManualImportRequiredCount((current) => Math.max(0, current + delta));
        window.setTimeout(() => {
          void refreshSidebarCounts();
        }, 2_000);
        return;
      }
      void refreshSidebarCounts();
    };
    window.addEventListener("scryer:pendingImportsRefresh", handleSidebarCountsRefresh);
    const intervalId = window.setInterval(() => {
      void refreshSidebarCounts();
    }, 30_000);
    return () => {
      window.removeEventListener("scryer:pendingImportsRefresh", handleSidebarCountsRefresh);
      window.clearInterval(intervalId);
    };
  }, [refreshSidebarCounts]);

  const activeFacet = useMemo<Facet>(() => facetForView(view)?.id ?? "movie", [view]);
  const queueFacet = activeFacet;

  const navigateTo = useCallback(
    (
      nextView: ViewId,
      nextSettingsSection?: SettingsSection,
      nextContentSection?: ContentSettingsSection,
      nextSystemSection?: SystemSection,
      nextWantedSection?: WantedSection,
      nextActivitySection?: ActivitySection,
      nextOverviewTitleId?: string | null,
      nextEpisodeId?: string | null,
    ) => {
      const isMedia = isMediaView(nextView);
      const targetPath = buildViewPath(
        nextView,
        nextView === "settings" ? nextSettingsSection : undefined,
        isMedia ? nextContentSection : undefined,
        nextView === "system" ? nextSystemSection : undefined,
        nextView === "wanted" ? nextWantedSection : undefined,
        nextView === "activity" ? nextActivitySection : undefined,
      );
      const normalizedContentSection = isMedia
        ? (nextContentSection ?? "overview")
        : "overview";
      const normalizedOverviewTitleId = (nextOverviewTitleId ?? "").trim().length > 0
        ? (nextOverviewTitleId as string).trim()
        : null;

      const nextParams = new URLSearchParams(searchParams.toString());
      nextParams.delete(URL_PARAM_VIEW_DEPRECATED);
      nextParams.delete(URL_PARAM_SETTINGS_SECTION_DEPRECATED);
      nextParams.delete(URL_PARAM_CONTENT_SECTION_DEPRECATED);
      nextParams.delete(URL_PARAM_LANGUAGE);
      nextParams.delete("tab");
      if (
        normalizedOverviewTitleId &&
        isMedia &&
        normalizedContentSection === "overview"
      ) {
        nextParams.set("id", normalizedOverviewTitleId);
      } else {
        nextParams.delete("id");
      }
      if (nextEpisodeId) {
        nextParams.set("episodeId", nextEpisodeId);
      } else {
        nextParams.delete("episodeId");
      }

      const nextQuery = nextParams.toString();
      const nextPathWithQuery = `${targetPath}${nextQuery ? `?${nextQuery}` : ""}`;
      const currentPathWithQuery = `${pathname}${searchParams.toString() ? `?${searchParams.toString()}` : ""}`;

      if (nextPathWithQuery !== currentPathWithQuery) {
        navigate(nextPathWithQuery);
      }
    },
    [navigate, searchParams, pathname],
  );

  const navigateToOverview = useCallback(
    (
      targetView: ViewId,
      overviewTarget: OverviewTitleTarget,
      episodeId?: string | null,
      replace = false,
    ) => {
      if (!isMediaView(targetView)) {
        return;
      }

      const normalizedTitleId = overviewTarget.id.trim();
      if (!normalizedTitleId) {
        return;
      }

      const normalizedSlug = overviewTarget.slug?.trim() || null;
      const targetPath = buildOverviewDetailPath(targetView, normalizedSlug);
      const nextParams = new URLSearchParams(searchParams.toString());
      nextParams.delete(URL_PARAM_VIEW_DEPRECATED);
      nextParams.delete(URL_PARAM_SETTINGS_SECTION_DEPRECATED);
      nextParams.delete(URL_PARAM_CONTENT_SECTION_DEPRECATED);
      nextParams.delete(URL_PARAM_LANGUAGE);
      nextParams.delete("tab");
      if (!normalizedSlug) {
        nextParams.set("id", normalizedTitleId);
      } else {
        nextParams.delete("id");
      }
      if (episodeId) {
        nextParams.set("episodeId", episodeId);
      } else {
        nextParams.delete("episodeId");
      }

      const nextQuery = nextParams.toString();
      const nextPathWithQuery = `${targetPath}${nextQuery ? `?${nextQuery}` : ""}`;
      const currentPathWithQuery = `${pathname}${searchParams.toString() ? `?${searchParams.toString()}` : ""}`;
      const state = normalizedSlug
        ? {
            scryerOverviewTarget: {
              view: targetView,
              id: normalizedTitleId,
              slug: normalizedSlug,
            },
          }
        : undefined;

      if (nextPathWithQuery !== currentPathWithQuery) {
        navigate(nextPathWithQuery, { replace, state });
      }
    },
    [navigate, pathname, searchParams],
  );

  useEffect(() => {
    let cancelled = false;

    if (navigationOverviewTarget) {
      setResolvedOverviewTarget(navigationOverviewTarget);
      setOverviewSlugLoading(false);
      return () => {
        cancelled = true;
      };
    }

    if (!isMediaView(view) || contentSettingsSection !== "overview" || !parsedOverviewSlug) {
      setResolvedOverviewTarget(null);
      setOverviewSlugLoading(false);
      return () => {
        cancelled = true;
      };
    }

    const facet = facetForView(view)?.id;
    if (!facet) {
      setResolvedOverviewTarget(null);
      setOverviewSlugLoading(false);
      return () => {
        cancelled = true;
      };
    }

    setResolvedOverviewTarget(null);
    setOverviewSlugLoading(true);

    void resolveTitleOverviewTargetBySlug(backendClient, facet, parsedOverviewSlug)
      .then((target) => {
        if (cancelled) {
          return;
        }

        setResolvedOverviewTarget(target);
        if (!target) {
          navigateTo(view, undefined, "overview", undefined, undefined);
          return;
        }

        if (target.slug && target.slug !== parsedOverviewSlug) {
          navigateToOverview(view, target, overviewEpisodeId, true);
        }
      })
      .catch((error: unknown) => {
        if (cancelled) {
          return;
        }

        setResolvedOverviewTarget(null);
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
        navigateTo(view, undefined, "overview", undefined, undefined);
      })
      .finally(() => {
        if (!cancelled) {
          setOverviewSlugLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [
    contentSettingsSection,
    navigateTo,
    navigateToOverview,
    navigationOverviewTarget,
    overviewEpisodeId,
    parsedOverviewSlug,
    setGlobalStatus,
    t,
    view,
  ]);

  const overviewTitleId = parsedOverviewSlug
    ? navigationOverviewTarget?.id ?? resolvedOverviewTarget?.id ?? null
    : legacyOverviewTitleId;
  const overviewLoading =
    Boolean(parsedOverviewSlug) &&
    !navigationOverviewTarget &&
    (overviewSlugLoading || overviewTitleId === null);

  const handleOpenOverview = useCallback(
    (targetView: ViewId, overviewTarget: OverviewTitleTarget, episodeId?: string) => {
      if (!isMediaView(targetView)) {
        return;
      }

      navigateToOverview(targetView, overviewTarget, episodeId);
    },
    [navigateToOverview],
  );

  const handleOverviewTitleResolved = useCallback(
    (overviewTarget: OverviewTitleTarget) => {
      if (!isMediaView(view) || contentSettingsSection !== "overview") {
        return;
      }

      const normalizedSlug = overviewTarget.slug?.trim() || null;
      if (!normalizedSlug) {
        return;
      }

      if (parsedOverviewSlug) {
        if (normalizedSlug !== parsedOverviewSlug) {
          navigateToOverview(view, overviewTarget, overviewEpisodeId, true);
        }
        return;
      }

      if (legacyOverviewTitleId) {
        navigateToOverview(view, overviewTarget, overviewEpisodeId, true);
      }
    },
    [
      contentSettingsSection,
      legacyOverviewTitleId,
      navigateToOverview,
      overviewEpisodeId,
      parsedOverviewSlug,
      view,
    ],
  );

  const topNav = useMemo(
    () => [
      ...FACET_REGISTRY.map((f) => ({ id: f.viewId as ViewId, label: t(f.navLabelKey), icon: f.icon })),
      { id: "activity" as ViewId, label: t("nav.activity"), icon: ActivitySquare },
      { id: "calendar" as ViewId, label: t("nav.calendar"), icon: CalendarDays },
      { id: "wanted" as ViewId, label: t("nav.wanted"), icon: ListChecks },
      { id: "history" as ViewId, label: t("nav.history"), icon: History },
      { id: "settings" as ViewId, label: t("nav.settings"), icon: Settings },
      { id: "system" as ViewId, label: t("nav.system"), icon: MonitorCog },
    ],
    [t],
  );

  const routeCommandPalette = useMemo(
    () => buildRouteCommands({
      t,
      pendingImportCounts,
      ignoredImportCounts,
      activityImportCount: manualImportRequiredCount,
      onNavigate: navigateTo,
    }),
    [ignoredImportCounts, manualImportRequiredCount, navigateTo, pendingImportCounts, t],
  );

  const routeCommandPaletteConfig = useMemo(
    () => ({
      title: t("command.paletteTitle"),
      description: t("command.paletteDescription"),
      placeholder: t("command.palettePlaceholder"),
      noResultsText: t("command.paletteNoResults"),
      groupLabel: t("command.paletteGroup"),
      items: routeCommandPalette,
    }),
    [routeCommandPalette, t],
  );

  const entitlements = useMemo(() => user?.entitlements ?? [], [user?.entitlements]);

  const handleBackToList = useCallback(
    () => navigateTo(view, undefined, "overview", undefined, undefined),
    [navigateTo, view],
  );

  const handleTitleNotFound = useCallback(
    () => navigateTo(view, undefined, "overview", undefined, undefined),
    [navigateTo, view],
  );

  useEffect(() => {
    if (
      !isMediaView(view) ||
      contentSettingsSection !== "import" ||
      pendingImportCounts === null ||
      ignoredImportCounts === null
    ) {
      return;
    }

    if (hasImportItemsForView(pendingImportCounts, ignoredImportCounts, view)) {
      return;
    }

    navigateTo(view, undefined, "overview", undefined, undefined);
  }, [contentSettingsSection, ignoredImportCounts, navigateTo, pendingImportCounts, view]);

  useEffect(() => {
    if (
      view !== "activity" ||
      activitySection !== "import" ||
      pendingImportCounts === null ||
      manualImportRequiredCount > 0
    ) {
      return;
    }

    navigateTo("activity", undefined, undefined, undefined, undefined, "activity");
  }, [activitySection, manualImportRequiredCount, navigateTo, pendingImportCounts, view]);

  return (
    <ScryerGraphqlProvider language={uiLanguage}>
    <TranslateContext.Provider value={t}>
    <GlobalStatusContext.Provider value={setGlobalStatus}>
    <div className="flex min-h-screen flex-col bg-background text-foreground">
      {serviceRestarting && (
        <BackendRestartOverlay />
      )}
      <Suspense fallback={<ViewLoadingFallback />}>
        <LibraryScanProgressProvider>
          <JobRunProvider>
            <ReactiveRefreshProvider>
              <GlobalSearchProvider
                activeFacet={activeFacet}
                queueFacet={queueFacet}
                uiLanguage={uiLanguage}
              >
                <RootHeader
                  onOpenOverview={handleOpenOverview}
                  routeCommandPalette={routeCommandPaletteConfig}
                />

                {!isOnline ? (
                  <div className="flex items-center justify-center gap-2 bg-amber-900/80 px-4 py-2 text-sm text-amber-100">
                    <WifiOff className="h-4 w-4 flex-none" />
                    <span>{t("pwa.offline")}</span>
                  </div>
                ) : null}

                {showInstallBanner ? (
                  <div className="flex items-center justify-center gap-3 bg-emerald-100 dark:bg-emerald-900/60 px-4 py-2 text-sm text-emerald-800 dark:text-emerald-100">
                    <Download className="h-4 w-4 flex-none" />
                    <span>{isIosSafari ? t("pwa.iosInstallHint") : t("pwa.installApp")}</span>
                    {canPrompt ? (
                      <button
                        type="button"
                        onClick={() => void promptInstall()}
                        className="rounded-md bg-emerald-600 px-3 py-1 text-xs font-medium text-foreground hover:bg-emerald-500"
                      >
                        {t("pwa.installApp")}
                      </button>
                    ) : null}
                    <button
                      type="button"
                      onClick={dismissInstallBanner}
                      className="ml-auto text-emerald-700 dark:text-emerald-300 hover:text-foreground"
                      aria-label={t("label.dismiss")}
                    >
                      <X className="h-4 w-4" />
                    </button>
                  </div>
                ) : null}

                <div className="mx-auto flex w-full max-w-[1480px] flex-1 min-h-0 px-3 pb-10 pt-4">
                  <RootSidebar
                    topNav={topNav}
                    view={view}
                    settingsSection={settingsSection}
                    contentSettingsSection={contentSettingsSection}
                    systemSection={systemSection}
                    activitySection={activitySection}
                    wantedSection={wantedSection}
                    entitlements={entitlements}
                    pendingImportCounts={pendingImportCounts}
                    ignoredImportCounts={ignoredImportCounts}
                    manualImportRequiredCount={manualImportRequiredCount}
                    onNavigate={navigateTo}
                  >
                    <main className={view === "wanted" || view === "calendar" ? "flex min-h-0 flex-1 flex-col" : "min-h-[70vh]"}>
                      <Suspense fallback={<ViewLoadingFallback />}>
                        <MainContent
                          view={view}
                          overviewTitleId={overviewTitleId}
                          overviewLoading={overviewLoading}
                          overviewEpisodeId={overviewEpisodeId}
                          handleBackToList={handleBackToList}
                          handleTitleNotFound={handleTitleNotFound}
                          handleOverviewTitleResolved={handleOverviewTitleResolved}
                          settingsSection={settingsSection}
                          userId={user?.id}
                          username={user?.username}
                          selectedLanguage={selectedLanguage}
                          uiLanguage={uiLanguage}
                          setLanguagePreferenceFromShell={setLanguagePreferenceFromShell}
                          contentSettingsSection={contentSettingsSection}
                          systemSection={systemSection}
                          activitySection={activitySection}
                          wantedSection={wantedSection}
                          handleOpenOverview={handleOpenOverview}
                        />
                      </Suspense>
                    </main>
                  </RootSidebar>
                </div>
              </GlobalSearchProvider>
            </ReactiveRefreshProvider>
          </JobRunProvider>
        </LibraryScanProgressProvider>
      </Suspense>
    </div>
    </GlobalStatusContext.Provider>
    </TranslateContext.Provider>
    </ScryerGraphqlProvider>
  );
}
