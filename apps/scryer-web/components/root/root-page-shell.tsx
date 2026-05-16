import { lazy, Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { ActivitySquare, AlertOctagon, AlertTriangle, CalendarDays, Download, ListChecks, Loader2, MonitorCog, Settings, WifiOff, X } from "lucide-react";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";
import { useAuth, type AuthUser } from "@/lib/hooks/use-auth";
import { APP_PERMISSIONS, LIBRARY_PERMISSIONS, hasAnyLibraryPermission, hasAppPermission } from "@/lib/utils/permissions";

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
import { useSettingsSubscription } from "@/lib/hooks/use-settings-subscription";
import type {
  ActivitySection,
  ViewId,
  SettingsSection,
  ContentSettingsSection,
  OverviewTitleTarget,
  SmgVersionCompatibilityNotice,
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
  parseOverviewTargetFromPath,
  parseSettingsSectionFromPath,
  parseSystemSectionFromPath,
  parseViewFromPath,
  parseWantedSectionFromPath,
} from "@/lib/utils/routing";
import { FACET_REGISTRY, isMediaView, facetForView } from "@/lib/facets/registry";
import { BackendRestartOverlay } from "@/components/common/backend-restart-overlay";
import {
  navigationBadgeCountsQuery,
  scryerVersionQuery,
  smgVersionCompatibilityNoticeQuery,
} from "@/lib/graphql/queries";
import type { PendingImportCounts } from "@/lib/types";
import { resolveTitleOverviewTargetBySlug } from "@/lib/title-overview-loader";
import {
  NAVIGATION_BADGES_REFRESH_EVENT,
  type NavigationBadgesRefreshDetail,
} from "@/lib/events/navigation-badges";
const SMG_VERSION_COMPATIBILITY_NOTICE_KEY = "smg.version_compatibility_notice";

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

const WantedHistoryContainer = lazy(() =>
  import("@/components/containers/title-history-container").then((m) => ({ default: m.TitleHistoryContainer })),
);

const PendingImportsContainer = lazy(() =>
  import("@/components/containers/pending-imports-container").then((m) => ({ default: m.PendingImportsContainer })),
);

const INSTALL_BANNER_DISMISSED_KEY = "scryer.pwa.installBannerDismissed";

type TranslateFn = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;

function normalizeSmgVersionCompatibilityStatus(
  status: string,
): "deprecated" | "blocked" {
  return status.trim().toLowerCase() === "deprecated" ? "deprecated" : "blocked";
}

function isMediaSettingsSection(section: ContentSettingsSection): boolean {
  return (
    section === "library" ||
    section === "general" ||
    section === "quality" ||
    section === "renaming" ||
    section === "routing"
  );
}

function isManageConfigMediaSection(section: ContentSettingsSection): boolean {
  return section === "import" || isMediaSettingsSection(section);
}

function canAccessSettingsSection(
  section: SettingsSection,
  canManageUsers: boolean,
  canManageConfig: boolean,
): boolean {
  if (section === "profile") {
    return true;
  }

  if (section === "security" || section === "users") {
    return canManageUsers;
  }

  return canManageConfig;
}

function defaultSettingsSection(
  canManageUsers: boolean,
  canManageConfig: boolean,
): SettingsSection {
  if (canManageConfig) {
    return "general";
  }

  if (canManageUsers) {
    return "security";
  }

  return "profile";
}

function defaultAccessibleRoute(
  canViewCatalog: boolean,
  canManageUsers: boolean,
  canManageConfig: boolean,
): {
  view: ViewId;
  settingsSection?: SettingsSection;
  contentSettingsSection?: ContentSettingsSection;
} {
  if (canViewCatalog) {
    return {
      view: "movies",
      contentSettingsSection: "overview",
    };
  }

  return {
    view: "settings",
    settingsSection: defaultSettingsSection(canManageUsers, canManageConfig),
  };
}

function formatSmgUpgradeDeadline(value: string | null): string | null {
  if (!value) {
    return null;
  }

  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }

  return date.toLocaleDateString();
}

function SmgUpgradeBanner({
  notice,
  t,
}: {
  notice: SmgVersionCompatibilityNotice;
  t: TranslateFn;
}) {
  const status = normalizeSmgVersionCompatibilityStatus(notice.status);
  const isDeprecated = status === "deprecated";
  const Icon = isDeprecated ? AlertTriangle : AlertOctagon;
  const deadline = formatSmgUpgradeDeadline(notice.upgradeDeadline);
  const minimumVersion = notice.minimumVersion.trim();
  const serverMessage = notice.message.trim();
  const details = [
    minimumVersion ? t("smgUpgrade.minimumVersion", { version: minimumVersion }) : null,
    deadline ? t("smgUpgrade.deadline", { date: deadline }) : null,
  ].filter((value): value is string => Boolean(value));

  return (
    <div
      className={isDeprecated
        ? "border-b border-amber-300 bg-amber-100 text-amber-950 dark:border-amber-900 dark:bg-amber-950/70 dark:text-amber-100"
        : "border-b border-red-300 bg-red-100 text-red-950 dark:border-red-900 dark:bg-red-950/70 dark:text-red-100"}
    >
      <div className="mx-auto flex w-full max-w-[1480px] items-start gap-3 px-4 py-3">
        <Icon className="mt-0.5 h-5 w-5 flex-none" aria-hidden="true" />
        <div className="min-w-0">
          <div className="font-semibold">
            {isDeprecated ? t("smgUpgrade.deprecatedTitle") : t("smgUpgrade.blockedTitle")}
          </div>
          <div className="mt-0.5 text-sm">
            {isDeprecated ? t("smgUpgrade.deprecatedBody") : t("smgUpgrade.blockedBody")}
          </div>
          {serverMessage ? (
            <div className="mt-1 text-sm opacity-90">{serverMessage}</div>
          ) : null}
          {details.length > 0 ? (
            <div className="mt-1 text-xs font-medium uppercase tracking-wide opacity-80">
              {details.join(" • ")}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

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
    libraryId?: unknown;
    librarySlug?: unknown;
  };
};

function readOverviewTargetFromLocationState(
  state: unknown,
  view: ViewId,
  parsedOverviewSlug: string | null,
  parsedOverviewLibrarySlug: string | null,
): OverviewTitleTarget | null {
  if (!parsedOverviewSlug || !parsedOverviewLibrarySlug || state == null || typeof state !== "object") {
    return null;
  }

  const overviewState = state as OverviewNavigationState;
  const target = overviewState.scryerOverviewTarget;
  if (!target || target.view !== view) {
    return null;
  }

  const id = typeof target.id === "string" ? target.id.trim() : "";
  const slug = typeof target.slug === "string" ? target.slug.trim() : "";
  const libraryId = typeof target.libraryId === "string" ? target.libraryId.trim() : "";
  const librarySlug = typeof target.librarySlug === "string" ? target.librarySlug.trim() : "";
  if (!id || !slug || !librarySlug || slug !== parsedOverviewSlug || librarySlug !== parsedOverviewLibrarySlug) {
    return null;
  }

  return { id, slug, libraryId: libraryId || null, librarySlug };
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
  handleImportRouteEmpty,
  canAccessActivity,
  canManageTitle,
  canManageUsers,
  canManageConfig,
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
  handleImportRouteEmpty: () => void;
  canAccessActivity: boolean;
  canManageTitle: boolean;
  canManageUsers: boolean;
  canManageConfig: boolean;
}) {
  if (view === "activity") {
    if (!canAccessActivity) {
      return <ViewLoadingFallback />;
    }
    return <ActivityContainer key="activity" activitySection={activitySection} />;
  }
  if (view === "calendar") {
    return <CalendarContainer key="calendar" onOpenOverview={handleOpenOverview} />;
  }
  if (view === "wanted") {
    if (wantedSection === "history") {
      if (!canManageTitle) {
        return (
          <WantedContainer
            key="wanted-wanted"
            wantedSection="wanted"
            onOpenOverview={handleOpenOverview}
          />
        );
      }
      return <WantedHistoryContainer key="wanted-history" />;
    }
    return (
      <WantedContainer
        key={`wanted-${wantedSection}`}
        wantedSection={wantedSection}
        onOpenOverview={handleOpenOverview}
      />
    );
  }
  if (view === "history") {
    if (!canManageTitle) {
      return (
        <WantedContainer
          key="wanted-wanted"
          wantedSection="wanted"
          onOpenOverview={handleOpenOverview}
        />
      );
    }
    return <WantedHistoryContainer key="history" />;
  }
  if (view === "system") {
    if (!canManageConfig) {
      return <ViewLoadingFallback />;
    }
    return <SystemContainer key={`system-${systemSection}`} systemSection={systemSection} />;
  }
  if (isMediaView(view) && contentSettingsSection === "import" && canManageConfig) {
    return (
      <PendingImportsContainer
        key={`${view}-imports`}
        view={view}
        onNavigateBackToOverview={handleImportRouteEmpty}
      />
    );
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
    const resolvedSettingsSection = canAccessSettingsSection(
      settingsSection,
      canManageUsers,
      canManageConfig,
    )
      ? settingsSection
      : defaultSettingsSection(canManageUsers, canManageConfig);
    return (
      <SettingsContainer
        key="settings"
        settingsSection={resolvedSettingsSection}
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
      key={`${view}-${!canManageConfig && isManageConfigMediaSection(contentSettingsSection) ? "overview" : contentSettingsSection}`}
      view={view}
      contentSettingsSection={
        !canManageConfig && isManageConfigMediaSection(contentSettingsSection)
          ? "overview"
          : contentSettingsSection
      }
      canManageConfig={canManageConfig}
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
            { requestPolicy: "network-only" },
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
      authenticatedUser={user}
      serviceRestarting={serviceRestarting}
      setServiceRestarting={setServiceRestarting}
    />
  );
}

function AuthenticatedHomePage({
  authenticatedUser,
  serviceRestarting,
  setServiceRestarting,
}: {
  authenticatedUser: AuthUser;
  serviceRestarting: boolean;
  setServiceRestarting: (value: boolean) => void;
}) {
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
    parsedOverviewLibrarySlug,
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
      const parsedOverviewTarget = isMediaView(parsedView) && parsedContentSection === "overview"
        ? parseOverviewTargetFromPath(parsedView, segments[1] ?? null, segments[2] ?? null)
        : { librarySlug: null, titleSlug: null };
      return {
        parsedView,
        parsedSettingsSection,
        parsedContentSection,
        parsedSystemSection,
        parsedActivitySection,
        parsedWantedSection,
        parsedOverviewLibrarySlug: parsedOverviewTarget.librarySlug,
        parsedOverviewSlug: parsedOverviewTarget.titleSlug,
      };
    }, [pathname]);

  const legacyOverviewTitleId = useMemo(() => {
    if (!isMediaView(view) || contentSettingsSection !== "overview" || parsedOverviewSlug) return null;
    return searchParams.get("id")?.trim() || null;
  }, [view, contentSettingsSection, parsedOverviewSlug, searchParams]);

  useEffect(() => {
    if (view !== "history") {
      return;
    }

    const nextPath = buildViewPath("wanted", undefined, undefined, undefined, "history");
    const nextQuery = searchParams.toString();
    const nextPathWithQuery = `${nextPath}${nextQuery ? `?${nextQuery}` : ""}`;
    const currentPathWithQuery = `${pathname}${nextQuery ? `?${nextQuery}` : ""}`;

    if (nextPathWithQuery !== currentPathWithQuery) {
      navigate(nextPathWithQuery, { replace: true });
    }
  }, [navigate, pathname, searchParams, view]);

  const navigationOverviewTarget = useMemo(
    () => readOverviewTargetFromLocationState(
      location.state,
      view,
      parsedOverviewSlug,
      parsedOverviewLibrarySlug,
    ),
    [location.state, parsedOverviewLibrarySlug, parsedOverviewSlug, view],
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
  type NavigationBadgeCountsPayload = {
    pendingImportCounts?: PendingImportCounts | null;
    activityImportCount?: number | null;
    pluginUpdateCount?: number | null;
  };
  const [pendingImportCounts, setPendingImportCounts] = useState<PendingImportCounts | null>(null);
  const [manualImportRequiredCount, setManualImportRequiredCount] = useState(0);
  const [pluginUpdateCount, setPluginUpdateCount] = useState(0);
  const [scryerVersion, setScryerVersion] = useState<string | null>(null);
  const [smgVersionCompatibilityNotice, setSmgVersionCompatibilityNotice] =
    useState<SmgVersionCompatibilityNotice | null>(null);
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

  const refreshScryerVersion = useCallback(async () => {
    try {
      const { data, error } = await backendClient
        .query<{ scryerVersion?: string | null }>(scryerVersionQuery, {})
        .toPromise();
      if (error) {
        throw error;
      }
      setScryerVersion(data?.scryerVersion ?? null);
    } catch (error) {
      console.warn("Failed to refresh Scryer version", error);
    }
  }, []);

  useEffect(() => {
    if (!serviceRestarting) {
      void refreshScryerVersion();
    }
  }, [refreshScryerVersion, serviceRestarting]);

  const refreshSmgVersionCompatibilityNotice = useCallback(async () => {
    try {
      const { data, error } = await backendClient
        .query<{ smgVersionCompatibilityNotice?: SmgVersionCompatibilityNotice | null }>(
          smgVersionCompatibilityNoticeQuery,
          {},
        )
        .toPromise();
      if (error) {
        throw error;
      }
      setSmgVersionCompatibilityNotice(data?.smgVersionCompatibilityNotice ?? null);
    } catch (error) {
      console.warn("Failed to refresh SMG version compatibility notice", error);
    }
  }, []);

  useEffect(() => {
    void refreshSmgVersionCompatibilityNotice();
  }, [refreshSmgVersionCompatibilityNotice]);

  useSettingsSubscription(useCallback((changedKeys) => {
    if (changedKeys.includes(SMG_VERSION_COMPATIBILITY_NOTICE_KEY)) {
      void refreshSmgVersionCompatibilityNotice();
    }
  }, [refreshSmgVersionCompatibilityNotice]));

  useEffect(() => {
    if (typeof window === "undefined" || typeof document === "undefined") {
      return;
    }

    const handleFocus = () => {
      void refreshSmgVersionCompatibilityNotice();
    };
    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        void refreshSmgVersionCompatibilityNotice();
      }
    };

    window.addEventListener("focus", handleFocus);
    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      window.removeEventListener("focus", handleFocus);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [refreshSmgVersionCompatibilityNotice]);

  const refreshNavigationBadges = useCallback(async () => {
    try {
      const badgeCountsResult = await backendClient
        .query(navigationBadgeCountsQuery, {})
        .toPromise();

      if (badgeCountsResult.error) {
        throw badgeCountsResult.error;
      }

      const badgeCounts = badgeCountsResult.data?.navigationBadgeCounts as NavigationBadgeCountsPayload | undefined;
      setPendingImportCounts(
        badgeCounts?.pendingImportCounts ?? { movie: 0, series: 0, anime: 0 },
      );
      setManualImportRequiredCount(Number(badgeCounts?.activityImportCount ?? 0));
      setPluginUpdateCount(Number(badgeCounts?.pluginUpdateCount ?? 0));
    } catch (error) {
      console.warn("Failed to refresh navigation badges", error);
    }
  }, []);

  useEffect(() => {
    void refreshNavigationBadges();
  }, [refreshNavigationBadges]);

  useEffect(() => {
    const handleNavigationBadgeRefresh = (event: Event) => {
      const delta =
        event instanceof CustomEvent &&
          typeof (event as CustomEvent<NavigationBadgesRefreshDetail>).detail?.delta === "number"
          ? Number((event as CustomEvent<NavigationBadgesRefreshDetail>).detail?.delta)
          : 0;
      if (delta !== 0) {
        setManualImportRequiredCount((current) => Math.max(0, current + delta));
        window.setTimeout(() => {
          void refreshNavigationBadges();
        }, 2_000);
        return;
      }
      void refreshNavigationBadges();
    };
    window.addEventListener(
      NAVIGATION_BADGES_REFRESH_EVENT,
      handleNavigationBadgeRefresh,
    );
    const intervalId = window.setInterval(() => {
      void refreshNavigationBadges();
    }, 30_000);
    return () => {
      window.removeEventListener(
        NAVIGATION_BADGES_REFRESH_EVENT,
        handleNavigationBadgeRefresh,
      );
      window.clearInterval(intervalId);
    };
  }, [refreshNavigationBadges]);

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
      const normalizedLibrarySlug = overviewTarget.librarySlug?.trim() || null;
      const normalizedLibraryId = overviewTarget.libraryId?.trim() || null;
      const hasSlugRoute = Boolean(normalizedSlug && normalizedLibrarySlug);
      const targetPath = buildOverviewDetailPath(targetView, normalizedLibrarySlug, normalizedSlug);
      const nextParams = new URLSearchParams(searchParams.toString());
      nextParams.delete(URL_PARAM_VIEW_DEPRECATED);
      nextParams.delete(URL_PARAM_SETTINGS_SECTION_DEPRECATED);
      nextParams.delete(URL_PARAM_CONTENT_SECTION_DEPRECATED);
      nextParams.delete(URL_PARAM_LANGUAGE);
      nextParams.delete("tab");
      if (!hasSlugRoute) {
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
      const state = hasSlugRoute
        ? {
            scryerOverviewTarget: {
              view: targetView,
              id: normalizedTitleId,
              slug: normalizedSlug,
              libraryId: normalizedLibraryId,
              librarySlug: normalizedLibrarySlug,
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

    if (
      !isMediaView(view) ||
      contentSettingsSection !== "overview" ||
      !parsedOverviewLibrarySlug ||
      !parsedOverviewSlug
    ) {
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

    void resolveTitleOverviewTargetBySlug(
      backendClient,
      facet,
      parsedOverviewLibrarySlug,
      parsedOverviewSlug,
    )
      .then((target) => {
        if (cancelled) {
          return;
        }

        setResolvedOverviewTarget(target);
        if (!target) {
          navigateTo(view, undefined, "overview", undefined, undefined);
          return;
        }

        if (
          target.slug &&
          target.librarySlug &&
          (target.slug !== parsedOverviewSlug || target.librarySlug !== parsedOverviewLibrarySlug)
        ) {
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
    parsedOverviewLibrarySlug,
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
      const normalizedLibrarySlug = overviewTarget.librarySlug?.trim() || null;
      if (!normalizedSlug || !normalizedLibrarySlug) {
        return;
      }

      if (parsedOverviewSlug && parsedOverviewLibrarySlug) {
        if (
          normalizedSlug !== parsedOverviewSlug ||
          normalizedLibrarySlug !== parsedOverviewLibrarySlug
        ) {
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
    parsedOverviewLibrarySlug,
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
      { id: "settings" as ViewId, label: t("nav.settings"), icon: Settings },
      { id: "system" as ViewId, label: t("nav.system"), icon: MonitorCog },
    ],
    [t],
  );
  const canViewCatalog = hasAnyLibraryPermission(authenticatedUser, LIBRARY_PERMISSIONS.view);
  const canManageTitle = hasAnyLibraryPermission(authenticatedUser, LIBRARY_PERMISSIONS.manageTitles);
  const canResolveImports = hasAnyLibraryPermission(authenticatedUser, LIBRARY_PERMISSIONS.resolveImports);
  const canAccessActivity = canResolveImports || canManageTitle;
  const canManageUserAccounts = hasAppPermission(authenticatedUser, APP_PERMISSIONS.manageUsers);
  const canManagePermissions = hasAppPermission(authenticatedUser, APP_PERMISSIONS.managePermissions);
  const canManageSystemSettings = hasAppPermission(authenticatedUser, APP_PERMISSIONS.manageSystemSettings);
  const canManageCatalogSettings = hasAppPermission(authenticatedUser, APP_PERMISSIONS.manageCatalogSettings);
  const canManageUsers = canManageUserAccounts || canManagePermissions;
  const canManageConfig = canManageSystemSettings || canManageCatalogSettings;

  const routeCommandPalette = useMemo(
    () => buildRouteCommands({
      t,
      pendingImportCounts,
      user: authenticatedUser,
      activityImportCount: manualImportRequiredCount,
      onNavigate: navigateTo,
    }),
    [authenticatedUser, manualImportRequiredCount, navigateTo, pendingImportCounts, t],
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

  useEffect(() => {
    if (!isMediaView(view) || canManageConfig || !isManageConfigMediaSection(contentSettingsSection)) {
      return;
    }

    navigateTo(view, undefined, "overview", undefined, undefined);
  }, [canManageConfig, contentSettingsSection, navigateTo, view]);

  const navigateToAccessibleDefault = useCallback(() => {
    const fallback = defaultAccessibleRoute(
      canViewCatalog,
      canManageUsers,
      canManageConfig,
    );
    navigateTo(
      fallback.view,
      fallback.settingsSection,
      fallback.contentSettingsSection,
      undefined,
      undefined,
    );
  }, [canManageConfig, canManageUsers, canViewCatalog, navigateTo]);

  useEffect(() => {
    if (view !== "activity" || canAccessActivity) {
      return;
    }

    navigateToAccessibleDefault();
  }, [canAccessActivity, navigateToAccessibleDefault, view]);

  useEffect(() => {
    if (view !== "system" || canManageConfig) {
      return;
    }

    navigateToAccessibleDefault();
  }, [canManageConfig, navigateToAccessibleDefault, view]);

  useEffect(() => {
    if (view !== "wanted" || wantedSection !== "history" || canManageTitle) {
      return;
    }

    if (!canViewCatalog) {
      navigateToAccessibleDefault();
      return;
    }

    navigateTo("wanted", undefined, undefined, undefined, "wanted");
  }, [
    canManageTitle,
    canViewCatalog,
    navigateTo,
    navigateToAccessibleDefault,
    view,
    wantedSection,
  ]);

  useEffect(() => {
    if (view !== "settings") {
      return;
    }

    if (canAccessSettingsSection(settingsSection, canManageUsers, canManageConfig)) {
      return;
    }

    navigateTo(
      "settings",
      defaultSettingsSection(canManageUsers, canManageConfig),
    );
  }, [
    canManageConfig,
    canManageUsers,
    navigateTo,
    settingsSection,
    view,
  ]);

  const handleBackToList = useCallback(
    () => {
      const targetPath = buildViewPath(view, undefined, "overview");
      const nextParams = new URLSearchParams(searchParams.toString());
      nextParams.delete(URL_PARAM_VIEW_DEPRECATED);
      nextParams.delete(URL_PARAM_SETTINGS_SECTION_DEPRECATED);
      nextParams.delete(URL_PARAM_CONTENT_SECTION_DEPRECATED);
      nextParams.delete(URL_PARAM_LANGUAGE);
      nextParams.delete("tab");
      nextParams.delete("id");
      nextParams.delete("episodeId");

      const nextQuery = nextParams.toString();
      const nextPathWithQuery = `${targetPath}${nextQuery ? `?${nextQuery}` : ""}`;
      navigate(nextPathWithQuery, {
        state: { restoreOverviewScroll: true },
      });
    },
    [navigate, searchParams, view],
  );

  const handleTitleNotFound = useCallback(
    () => navigateTo(view, undefined, "overview", undefined, undefined),
    [navigateTo, view],
  );

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

                {smgVersionCompatibilityNotice ? (
                  <SmgUpgradeBanner
                    notice={smgVersionCompatibilityNotice}
                    t={t}
                  />
                ) : null}

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
                    user={authenticatedUser}
                    pendingImportCounts={pendingImportCounts}
                    manualImportRequiredCount={manualImportRequiredCount}
                    pluginUpdateCount={pluginUpdateCount}
                    scryerVersion={scryerVersion}
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
                          userId={authenticatedUser.id}
                          username={authenticatedUser.username}
                          selectedLanguage={selectedLanguage}
                          uiLanguage={uiLanguage}
                          setLanguagePreferenceFromShell={setLanguagePreferenceFromShell}
                          contentSettingsSection={contentSettingsSection}
                          systemSection={systemSection}
                          activitySection={activitySection}
                          wantedSection={wantedSection}
                          handleOpenOverview={handleOpenOverview}
                          handleImportRouteEmpty={handleBackToList}
                          canAccessActivity={canAccessActivity}
                          canManageTitle={canManageTitle}
                          canManageUsers={canManageUsers}
                          canManageConfig={canManageConfig}
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
