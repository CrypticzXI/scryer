import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import {
  ActivitySquare,
  AlertOctagon,
  AlertTriangle,
  CalendarDays,
  Download,
  ListChecks,
  Loader2,
  Monitor,
  Settings,
  CircleFadingArrowUp,
  Sparkles,
  WifiOff,
  X,
} from "lucide-react";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";
import { useAuth, type AuthUser } from "@/lib/hooks/use-auth";
import { usePermissions } from "@/lib/hooks/use-permissions";
import { authorizationCacheSignature } from "@/lib/utils/permissions";
import { useSmgNotices } from "@/lib/hooks/use-smg-notices";
import { useNavigationBadges } from "@/lib/hooks/use-navigation-badges";
import { useAutoBackupNotice } from "@/lib/hooks/use-auto-backup-notice";
import { useConfigStepUp } from "@/lib/hooks/use-config-step-up";

import { TranslateContext } from "@/lib/context/translate-context";
import { GlobalStatusContext } from "@/lib/context/global-status-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { RootHeader } from "@/components/root/root-header";
import { buildRouteCommands } from "@/components/root/route-commands";
import { JobRunProvider } from "@/components/root/job-run-provider";
import { LibraryScanProgressProvider } from "@/components/root/library-scan-progress-provider";
import { ReactiveRefreshProvider } from "@/components/root/reactive-refresh-provider";
import { RootSidebar } from "@/components/root/root-sidebar";
import { ViewLoadingFallback } from "@/components/common/view-loading-fallback";
import { GlobalSearchProvider } from "@/components/root/global-search-provider";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";

import { useGlobalStatusToast } from "@/lib/hooks/use-global-status-toast";
import { useLanguage } from "@/lib/hooks/use-language";
import { ScryerGraphqlProvider } from "@/lib/graphql/urql-provider";
import { backendClient } from "@/lib/graphql/urql-client";
import { useOnlineStatus } from "@/lib/hooks/use-online-status";
import { useInstallPrompt } from "@/lib/hooks/use-install-prompt";
import { useBackendRestarting } from "@/lib/hooks/use-backend-restarting";
import { TotpCodeForm } from "@/components/auth/totp-code-form";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type {
  ActivitySection,
  ViewId,
  SettingsSection,
  ContentSettingsSection,
  LogsSection,
  OverviewTitleTarget,
  SmgScryerUpdateNotice,
  SmgVersionCompatibilityNotice,
  SystemSection,
  WantedSection,
} from "@/components/root/types";
import type { Facet } from "@/lib/types";
import type { UiDateTimeFormat } from "@/lib/types/settings";
import { formatUiDate } from "@/lib/utils/date-format";
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
  parseLogsSectionFromPath,
  parseOverviewTargetFromPath,
  parseSettingsSectionFromPath,
  parseSystemSectionFromPath,
  parseViewFromPath,
  parseWantedSectionFromPath,
} from "@/lib/utils/routing";
import {
  canAccessMediaSettingsSection,
  canAccessSettingsSection,
  isMediaSettingsSection,
  isProtectedSettingsRoute,
} from "@/lib/utils/routes";
import { cn } from "@/lib/utils";
import {
  FACET_REGISTRY,
  isMediaView,
  facetForView,
} from "@/lib/facets/registry";
import { BackendRestartOverlay } from "@/components/common/backend-restart-overlay";
import { resolveTitleOverviewTargetBySlug } from "@/lib/title-overview-loader";

const mediaContainers = () =>
  import("@/components/containers/media-containers");

const MediaContentContainer = lazy(() =>
  mediaContainers().then((m) => ({ default: m.MediaContentContainer })),
);

const RequestsContainer = lazy(() =>
  import("@/components/containers/requests-container").then((m) => ({
    default: m.RequestsContainer,
  })),
);

const SettingsContainer = lazy(() =>
  import("@/components/containers/settings/settings-container").then((m) => ({
    default: m.SettingsContainer,
  })),
);

const ActivityContainer = lazy(() =>
  import("@/components/containers/activity-container").then((m) => ({
    default: m.ActivityContainer,
  })),
);

const SystemContainer = lazy(() =>
  import("@/components/containers/system-container").then((m) => ({
    default: m.SystemContainer,
  })),
);

const WantedContainer = lazy(() =>
  import("@/components/containers/wanted-container").then((m) => ({
    default: m.WantedContainer,
  })),
);

const DiscoveryContainer = lazy(() =>
  import("@/components/containers/discovery-container").then((m) => ({
    default: m.DiscoveryContainer,
  })),
);

const CalendarContainer = lazy(() =>
  import("@/components/containers/calendar-container").then((m) => ({
    default: m.CalendarContainer,
  })),
);

const WantedHistoryContainer = lazy(() =>
  import("@/components/containers/title-history-container").then((m) => ({
    default: m.TitleHistoryContainer,
  })),
);

const PendingImportsContainer = lazy(() =>
  import("@/components/containers/pending-imports-container").then((m) => ({
    default: m.PendingImportsContainer,
  })),
);

const INSTALL_BANNER_DISMISSED_KEY = "scryer.pwa.installBannerDismissed";

type TranslateFn = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;

function normalizeSmgVersionCompatibilityStatus(
  status: string,
): "deprecated" | "blocked" {
  return status.trim().toLowerCase() === "deprecated"
    ? "deprecated"
    : "blocked";
}

function fallbackMediaContentSettingsSection(
  section: ContentSettingsSection,
  canManageConfig: boolean,
  canManageLibrarySettings: boolean,
  canResolveImports = false,
): ContentSettingsSection {
  if (
    canManageLibrarySettings &&
    !canManageConfig &&
    isMediaSettingsSection(section)
  ) {
    return "library";
  }
  if (canResolveImports) {
    return "import";
  }
  return "overview";
}

function defaultSettingsSection(
  canManageSystemSettings: boolean,
  canManageCatalogSettings: boolean,
  canManageUserAccounts: boolean,
  canManageUserAccess: boolean,
): SettingsSection {
  if (canManageSystemSettings) {
    return "general";
  }

  if (canManageCatalogSettings) {
    return "qualityProfiles";
  }

  if (canManageUserAccounts) {
    return "security";
  }

  if (canManageUserAccess) {
    return "users";
  }

  return "profile";
}

function defaultAccessibleRoute(
  canViewCatalog: boolean,
  canRequestMedia: boolean,
  canResolveImports: boolean,
  canManageUserAccounts: boolean,
  canManageUserAccess: boolean,
  canManageSystemSettings: boolean,
  canManageCatalogSettings: boolean,
  canManageLibrarySettings: boolean,
): {
  view: ViewId;
  settingsSection?: SettingsSection;
  contentSettingsSection?: ContentSettingsSection;
} {
  const canManageConfig = canManageSystemSettings || canManageCatalogSettings;

  if (canViewCatalog) {
    return {
      view: "movies",
      contentSettingsSection: "overview",
    };
  }

  if (canRequestMedia) {
    return {
      view: "requests",
    };
  }

  if (canResolveImports) {
    return {
      view: "movies",
      contentSettingsSection: "import",
    };
  }

  if (canManageLibrarySettings && !canManageConfig) {
    return {
      view: "movies",
      contentSettingsSection: "library",
    };
  }

  return {
    view: "settings",
    settingsSection: defaultSettingsSection(
      canManageSystemSettings,
      canManageCatalogSettings,
      canManageUserAccounts,
      canManageUserAccess,
    ),
  };
}

function formatSmgUpgradeDeadline(
  value: string | null,
  dateTimeFormat: UiDateTimeFormat,
): string | null {
  if (!value) {
    return null;
  }

  return formatUiDate(value, dateTimeFormat, { fallback: value });
}

function SmgUpgradeBanner({
  notice,
  t,
}: {
  notice: SmgVersionCompatibilityNotice;
  t: TranslateFn;
}) {
  const dateTimeFormat = useUiDateTimeFormat();
  const status = normalizeSmgVersionCompatibilityStatus(notice.status);
  const isDeprecated = status === "deprecated";
  const Icon = isDeprecated ? AlertTriangle : AlertOctagon;
  const deadline = formatSmgUpgradeDeadline(notice.upgradeDeadline, dateTimeFormat);
  const minimumVersion = notice.minimumVersion.trim();
  const serverMessage = notice.message.trim();
  const details = [
    minimumVersion
      ? t("smgUpgrade.minimumVersion", { version: minimumVersion })
      : null,
    deadline ? t("smgUpgrade.deadline", { date: deadline }) : null,
  ].filter((value): value is string => Boolean(value));

  return (
    <div
      data-slot="root-shell-notice"
      className="border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-surfA))] text-[var(--scry-body)] shadow-[0_8px_28px_rgba(2,6,23,0.14)] backdrop-blur"
    >
      <div className="mx-auto flex w-full max-w-[1480px] items-start gap-3 px-4 py-3">
        <span
          className={cn(
            "mt-0.5 flex h-8 w-8 flex-none items-center justify-center rounded-[10px] border shadow-[0_8px_20px_rgba(2,6,23,0.10)]",
            isDeprecated
              ? "border-[rgba(var(--scry-accent-rgb),0.28)] bg-[rgba(var(--scry-accent-rgb),0.14)] text-[var(--scry-accent-ring)]"
              : "border-destructive/30 bg-destructive/10 text-destructive",
          )}
        >
          <Icon className="h-4 w-4" aria-hidden="true" />
        </span>
        <div className="min-w-0">
          <div className="font-semibold text-[var(--scry-ink2)]">
            {isDeprecated
              ? t("smgUpgrade.deprecatedTitle")
              : t("smgUpgrade.blockedTitle")}
          </div>
          <div className="mt-0.5 text-sm text-[var(--scry-muted)]">
            {isDeprecated
              ? t("smgUpgrade.deprecatedBody")
              : t("smgUpgrade.blockedBody")}
          </div>
          {serverMessage ? (
            <div className="mt-1 text-sm text-[var(--scry-body)]">
              {serverMessage}
            </div>
          ) : null}
          {details.length > 0 ? (
            <div className="mt-1 text-xs font-medium uppercase tracking-wide text-[var(--scry-muted2)]">
              {details.join(" • ")}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function SmgScryerUpdateBanner({
  notice,
  t,
  onDismiss,
}: {
  notice: SmgScryerUpdateNotice;
  t: TranslateFn;
  onDismiss: () => void;
}) {
  const currentVersion = notice.currentVersion.trim();
  const latestVersion = notice.latestVersion.trim();
  const releaseUrl = notice.releaseUrl?.trim() || null;

  return (
    <div
      data-slot="root-shell-notice"
      className="border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-surfA))] text-[var(--scry-body)] shadow-[0_8px_28px_rgba(var(--scry-accent-rgb),0.10)] backdrop-blur"
    >
      <div className="mx-auto flex w-full max-w-[1480px] items-center gap-3 px-4 py-2 text-sm">
        <CircleFadingArrowUp
          className="h-4 w-4 flex-none text-[var(--scry-accent-ring)]"
          aria-hidden="true"
        />
        <div className="min-w-0 flex-1 truncate">
          <span className="font-medium text-[var(--scry-ink2)]">
            {t("smgUpdate.title")}
          </span>
          <span className="ml-2 text-[var(--scry-muted)]">
            {t("smgUpdate.body", {
              current: currentVersion || t("label.unknown"),
              latest: latestVersion || t("label.unknown"),
            })}
          </span>
        </div>
        {releaseUrl ? (
          <a
            href={releaseUrl}
            target="_blank"
            rel="noreferrer"
            className="flex-none rounded-[8px] border border-[var(--scry-border2)] bg-[rgba(var(--scry-accent-rgb),0.12)] px-2.5 py-1 text-xs font-medium text-[var(--scry-accent-text)] transition hover:border-[var(--scry-bhover2)] hover:bg-[rgba(var(--scry-accent-rgb),0.18)]"
          >
            {t("smgUpdate.releaseNotes")}
          </a>
        ) : null}
        <IconButton
          type="button"
          onClick={onDismiss}
          label={t("label.dismiss")}
          appearance="ghost"
          className="h-7 w-7 flex-none rounded-[8px]"
        >
          <X className="h-4 w-4" />
        </IconButton>
      </div>
    </div>
  );
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
  if (
    !parsedOverviewSlug ||
    !parsedOverviewLibrarySlug ||
    state == null ||
    typeof state !== "object"
  ) {
    return null;
  }

  const overviewState = state as OverviewNavigationState;
  const target = overviewState.scryerOverviewTarget;
  if (!target || target.view !== view) {
    return null;
  }

  const id = typeof target.id === "string" ? target.id.trim() : "";
  const slug = typeof target.slug === "string" ? target.slug.trim() : "";
  const libraryId =
    typeof target.libraryId === "string" ? target.libraryId.trim() : "";
  const librarySlug =
    typeof target.librarySlug === "string" ? target.librarySlug.trim() : "";
  const effectiveLibrarySlug =
    librarySlug ||
    (parsedOverviewLibrarySlug === view ? parsedOverviewLibrarySlug : "");
  if (
    !id ||
    !slug ||
    slug !== parsedOverviewSlug ||
    !effectiveLibrarySlug ||
    effectiveLibrarySlug !== parsedOverviewLibrarySlug
  ) {
    return null;
  }

  return {
    id,
    slug,
    libraryId: libraryId || null,
    librarySlug: effectiveLibrarySlug,
  };
}

/**
 * Renders the main content area.
 */
function MainContent({
  view,
  overviewTitleId,
  overviewTitleRoutePending,
  routeOverviewEpisodeId,
  handleBackToList,
  settingsSection,
  userId,
  username,
  selectedLanguage,
  uiLanguage,
  discoveryAuthorizationSignature,
  setLanguagePreferenceFromShell,
  contentSettingsSection,
  systemSection,
  logsSection,
  scryerVersion,
  activitySection,
  wantedSection,
  handleOpenOverview,
  handleImportRouteEmpty,
  canViewCatalog,
  canAccessActivity,
  canAccessRecycleBin,
  canResolveImports,
  canManageTitle,
  canRequestMedia,
  canManageUserAccounts,
  canManageUsers,
  canManageSystemSettings,
  canManageCatalogSettings,
  canManageConfig,
  canManageLibrarySettings,
}: {
  view: ViewId;
  overviewTitleId: string | null;
  overviewTitleRoutePending: boolean;
  routeOverviewEpisodeId: string | null;
  handleBackToList: () => void;
  settingsSection: SettingsSection;
  userId: string | undefined;
  username: string | undefined;
  selectedLanguage: LanguageOption;
  uiLanguage: LocaleCode;
  discoveryAuthorizationSignature: string;
  setLanguagePreferenceFromShell: (code: string) => void;
  contentSettingsSection: ContentSettingsSection;
  systemSection: SystemSection;
  logsSection: LogsSection;
  scryerVersion: string | null;
  activitySection: ActivitySection;
  wantedSection: WantedSection;
  handleOpenOverview: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
    episodeId?: string,
  ) => void;
  handleImportRouteEmpty: () => void;
  canViewCatalog: boolean;
  canAccessActivity: boolean;
  canAccessRecycleBin: boolean;
  canResolveImports: boolean;
  canManageTitle: boolean;
  canRequestMedia: boolean;
  canManageUserAccounts: boolean;
  canManageUsers: boolean;
  canManageSystemSettings: boolean;
  canManageCatalogSettings: boolean;
  canManageConfig: boolean;
  canManageLibrarySettings: boolean;
}) {
  if (view === "activity") {
    if (!canAccessActivity) {
      return <ViewLoadingFallback />;
    }
    return (
      <ActivityContainer key="activity" activitySection={activitySection} />
    );
  }
  if (view === "calendar") {
    return (
      <CalendarContainer key="calendar" onOpenOverview={handleOpenOverview} />
    );
  }
  if (view === "discovery") {
    return (
      <DiscoveryContainer
        key="discovery"
        userId={userId}
        uiLanguage={uiLanguage}
        authorizationSignature={discoveryAuthorizationSignature}
        canManageTitle={canManageTitle}
        canRequestMedia={canRequestMedia}
      />
    );
  }
  if (view === "requests") {
    if (!canManageTitle && !canRequestMedia) {
      return <ViewLoadingFallback />;
    }
    return <RequestsContainer key="requests" facet={null} />;
  }
  if (view === "wanted") {
    const resolvedWantedSection =
      wantedSection === "history" && !canManageTitle ? "wanted" : wantedSection;
    return (
      <WantedContainer
        key={`wanted-${resolvedWantedSection}`}
        wantedSection={resolvedWantedSection}
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
    if (!canManageSystemSettings) {
      return <ViewLoadingFallback />;
    }
    const effectiveSystemSection =
      systemSection === "logs" || systemSection === "audit"
        ? "overview"
        : systemSection;
    return (
      <SystemContainer
        key={`system-${effectiveSystemSection}`}
        systemSection={effectiveSystemSection}
        scryerVersion={scryerVersion}
      />
    );
  }
  if (view === "logs") {
    if (!canManageSystemSettings) {
      return <ViewLoadingFallback />;
    }
    return (
      <SystemContainer
        key={`logs-${logsSection}`}
        systemSection={logsSection}
        scryerVersion={scryerVersion}
      />
    );
  }
  if (
    isMediaView(view) &&
    contentSettingsSection === "import" &&
    canResolveImports
  ) {
    return (
      <PendingImportsContainer
        key={`${view}-imports`}
        view={view}
        onNavigateBackToOverview={handleImportRouteEmpty}
      />
    );
  }
  if (
    isMediaView(view) &&
    contentSettingsSection === "overview" &&
    !canViewCatalog
  ) {
    return <ViewLoadingFallback />;
  }
  if (view === "settings") {
    const resolvedSettingsSection = canAccessSettingsSection(
      settingsSection,
      canManageUserAccounts,
      canManageUsers,
      canManageSystemSettings,
      canManageCatalogSettings,
      canAccessRecycleBin,
    )
      ? settingsSection
      : defaultSettingsSection(
          canManageSystemSettings,
          canManageCatalogSettings,
          canManageUserAccounts,
          canManageUsers,
        );
    return (
      <SettingsContainer
        key="settings"
        settingsSection={resolvedSettingsSection}
        userId={userId}
        username={username}
        canManageSystemSettings={canManageSystemSettings}
        canManageCatalogSettings={canManageCatalogSettings}
        availableLanguages={AVAILABLE_LANGUAGES}
        selectedLanguage={selectedLanguage}
        uiLanguage={uiLanguage}
        onSelectLanguage={setLanguagePreferenceFromShell}
      />
    );
  }
  const effectiveContentSettingsSection = canAccessMediaSettingsSection(
    contentSettingsSection,
    canManageConfig,
    canManageLibrarySettings,
    canResolveImports,
  )
    ? contentSettingsSection
    : fallbackMediaContentSettingsSection(
        contentSettingsSection,
        canManageConfig,
        canManageLibrarySettings,
        canResolveImports,
      );
  return (
    <MediaContentContainer
      key={`${view}-${effectiveContentSettingsSection}`}
      view={view}
      contentSettingsSection={effectiveContentSettingsSection}
      canManageConfig={canManageConfig}
      canManageSystemSettings={canManageSystemSettings}
      canManageCatalogSettings={canManageCatalogSettings}
      canManageLibrarySettings={canManageLibrarySettings}
      canManageTitle={canManageTitle}
      canRequestMedia={canRequestMedia}
      onOpenOverview={handleOpenOverview}
      routeOverviewTitleId={overviewTitleId}
      routeOverviewPending={overviewTitleRoutePending}
      routeOverviewEpisodeId={routeOverviewEpisodeId}
      onCloseOverview={handleBackToList}
    />
  );
}

export default function HomePage() {
  const { serviceRestarting } = useBackendRestarting();
  const {
    token,
    user,
    loading: authLoading,
    effectiveFormLoginEnabled,
    mfaRequireConfigStepUp,
    adoptSession,
  } = useAuth();
  const navigate = useNavigate();
  const [setupChecked, setSetupChecked] = useState(false);

  useEffect(() => {
    if (
      !serviceRestarting &&
      !authLoading &&
      !user &&
      effectiveFormLoginEnabled === true
    ) {
      navigate("/login", { replace: true });
    }
  }, [
    authLoading,
    effectiveFormLoginEnabled,
    user,
    navigate,
    serviceRestarting,
  ]);

  // Check if setup wizard needs to run (first-run detection).
  useEffect(() => {
    if (serviceRestarting || authLoading || !user || setupChecked) return;
    (async () => {
      try {
        const { data } = await import("@/lib/graphql/urql-client").then((mod) =>
          mod.backendClient
            .query(
              `query SetupStatus { setupStatus { setupComplete } }`,
              {},
              { requestPolicy: "network-only" },
            )
            .toPromise(),
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
      <div
        data-slot="root-app-frame"
        className="flex min-h-dvh items-center justify-center text-[var(--scry-body)]"
      >
        <Loader2 className="h-6 w-6 animate-spin text-[var(--scry-accent-ring)]" />
      </div>
    );
  }

  if (!user) {
    if (effectiveFormLoginEnabled !== true) {
      return (
        <div
          data-slot="root-app-frame"
          className="flex min-h-dvh items-center justify-center text-[var(--scry-body)]"
        >
          <Loader2 className="h-6 w-6 animate-spin text-[var(--scry-accent-ring)]" />
        </div>
      );
    }
    return null;
  }

  return (
    <AuthenticatedHomePage
      authToken={token}
      authenticatedUser={user}
      mfaRequireConfigStepUp={mfaRequireConfigStepUp}
      adoptSession={adoptSession}
      serviceRestarting={serviceRestarting}
    />
  );
}

function AuthenticatedHomePage({
  authToken,
  authenticatedUser,
  mfaRequireConfigStepUp,
  adoptSession,
  serviceRestarting,
}: {
  authToken: string | null;
  authenticatedUser: AuthUser;
  mfaRequireConfigStepUp: boolean | null;
  adoptSession: (nextToken: string, nextUser: AuthUser | null) => void;
  serviceRestarting: boolean;
}) {
  const isOnline = useOnlineStatus();
  const { canPrompt, isInstalled, isIosSafari, promptInstall } =
    useInstallPrompt();

  const location = useLocation();
  const { pathname } = location;
  const [searchParams] = useSearchParams();
  const navigate = useNavigate();

  const {
    parsedView: view,
    parsedSettingsSection: settingsSection,
    parsedContentSection: contentSettingsSection,
    parsedSystemSection: systemSection,
    parsedLogsSection: logsSection,
    parsedActivitySection: activitySection,
    parsedWantedSection: wantedSection,
    parsedOverviewLibrarySlug,
    parsedOverviewSlug,
    parsedCanonicalRoutePath,
  } = useMemo(() => {
    const trimmed = pathname.replace(/^\/+|\/+$/g, "");
    const segments = trimmed ? trimmed.split("/") : [];
    const normalizedSegments = segments.map((segment) => segment.toLowerCase());
    const routeRoot = normalizedSegments[0] ?? null;
    const routeSection = normalizedSegments[1] ?? null;
    let settingsPathSegment = routeSection;
    let systemPathSegment = routeSection;
    let wantedPathSegment = routeSection;
    let parsedView = parseViewFromPath(routeRoot);
    let canonicalRoutePath: string | null = null;

    if (routeRoot === "automation") {
      if (routeSection === "wanted") {
        parsedView = "wanted";
        wantedPathSegment = normalizedSegments[2] ?? null;
        canonicalRoutePath = buildViewPath(
          "wanted",
          undefined,
          undefined,
          undefined,
          parseWantedSectionFromPath(wantedPathSegment),
        );
      } else {
        parsedView = "settings";
        settingsPathSegment = routeSection;
        if (["rules", "post-procesing", "post-processing"].includes(routeSection ?? "")) {
          canonicalRoutePath = buildViewPath(
            "settings",
            parseSettingsSectionFromPath(settingsPathSegment),
          );
        }
      }
    } else if (routeRoot === "integrations") {
      parsedView = "settings";
      settingsPathSegment = routeSection;
      if (["indexers", "download-clients", "media-servers", "notifications"].includes(routeSection ?? "")) {
        canonicalRoutePath = buildViewPath(
          "settings",
          parseSettingsSectionFromPath(settingsPathSegment),
        );
      }
    } else if (routeRoot === "system") {
      if (["users", "security", "backup", "backups"].includes(routeSection ?? "")) {
        parsedView = "settings";
        settingsPathSegment = routeSection;
        canonicalRoutePath = buildViewPath(
          "settings",
          parseSettingsSectionFromPath(settingsPathSegment),
        );
      } else {
        parsedView = "system";
        systemPathSegment = routeSection;
      }
    } else if (routeRoot === "wanted") {
      canonicalRoutePath = buildViewPath(
        "wanted",
        undefined,
        undefined,
        undefined,
        parseWantedSectionFromPath(wantedPathSegment),
      );
    } else if (routeRoot === "settings" && routeSection) {
      const settingsSection = parseSettingsSectionFromPath(settingsPathSegment);
      if (
        [
          "rules",
          "post-processing",
          "indexers",
          "downloadClients",
          "mediaServers",
          "notifications",
          "users",
          "security",
          "backups",
        ].includes(settingsSection)
      ) {
        canonicalRoutePath = buildViewPath("settings", settingsSection);
      }
    }

    const parsedSettingsSection: SettingsSection =
      parsedView === "settings"
        ? parseSettingsSectionFromPath(settingsPathSegment)
        : "general";
    const parsedContentSection: ContentSettingsSection = isMediaView(parsedView)
      ? parseContentSectionFromPath(
          normalizedSegments[1] ?? null,
          normalizedSegments[2] ?? null,
        )
      : "overview";
    const parsedSystemSection: SystemSection =
      parsedView === "system"
        ? parseSystemSectionFromPath(systemPathSegment)
        : "overview";
    const parsedLogsSection: LogsSection =
      parsedView === "logs"
        ? parseLogsSectionFromPath(normalizedSegments[1] ?? null)
        : "logs";
    const parsedActivitySection: ActivitySection =
      parsedView === "activity"
        ? parseActivitySectionFromPath(normalizedSegments[1] ?? null)
        : "activity";
    const parsedWantedSection: WantedSection =
      parsedView === "wanted"
        ? parseWantedSectionFromPath(wantedPathSegment)
        : "wanted";
    const parsedOverviewTarget =
      isMediaView(parsedView) && parsedContentSection === "overview"
        ? parseOverviewTargetFromPath(
            parsedView,
            segments[1] ?? null,
            segments[2] ?? null,
          )
        : { librarySlug: null, titleSlug: null };
    return {
      parsedView,
      parsedSettingsSection,
      parsedContentSection,
      parsedSystemSection,
      parsedLogsSection,
      parsedActivitySection,
      parsedWantedSection,
      parsedOverviewLibrarySlug: parsedOverviewTarget.librarySlug,
      parsedOverviewSlug: parsedOverviewTarget.titleSlug,
      parsedCanonicalRoutePath: canonicalRoutePath,
    };
  }, [pathname]);

  const legacyOverviewTitleId = useMemo(() => {
    if (
      !isMediaView(view) ||
      contentSettingsSection !== "overview" ||
      parsedOverviewSlug
    )
      return null;
    return searchParams.get("id")?.trim() || null;
  }, [view, contentSettingsSection, parsedOverviewSlug, searchParams]);

  useEffect(() => {
    if (!parsedCanonicalRoutePath) {
      return;
    }
    const nextQuery = searchParams.toString();
    const nextPathWithQuery = `${parsedCanonicalRoutePath}${nextQuery ? `?${nextQuery}` : ""}`;
    const currentPathWithQuery = `${pathname}${nextQuery ? `?${nextQuery}` : ""}`;

    if (nextPathWithQuery !== currentPathWithQuery) {
      navigate(nextPathWithQuery, { replace: true });
    }
  }, [navigate, parsedCanonicalRoutePath, pathname, searchParams]);

  useEffect(() => {
    if (view !== "history") {
      return;
    }

    const nextPath = buildViewPath(
      "wanted",
      undefined,
      undefined,
      undefined,
      "history",
    );
    const nextQuery = searchParams.toString();
    const nextPathWithQuery = `${nextPath}${nextQuery ? `?${nextQuery}` : ""}`;
    const currentPathWithQuery = `${pathname}${nextQuery ? `?${nextQuery}` : ""}`;

    if (nextPathWithQuery !== currentPathWithQuery) {
      navigate(nextPathWithQuery, { replace: true });
    }
  }, [navigate, pathname, searchParams, view]);

  useEffect(() => {
    if (view !== "system" || (systemSection !== "logs" && systemSection !== "audit")) {
      return;
    }

    const nextPath = buildViewPath(
      "logs",
      undefined,
      undefined,
      undefined,
      undefined,
      undefined,
      systemSection,
    );
    const nextQuery = searchParams.toString();
    const nextPathWithQuery = `${nextPath}${nextQuery ? `?${nextQuery}` : ""}`;
    const currentPathWithQuery = `${pathname}${nextQuery ? `?${nextQuery}` : ""}`;

    if (nextPathWithQuery !== currentPathWithQuery) {
      navigate(nextPathWithQuery, { replace: true });
    }
  }, [navigate, pathname, searchParams, systemSection, view]);

  const navigationOverviewTarget = useMemo(
    () =>
      readOverviewTargetFromLocationState(
        location.state,
        view,
        parsedOverviewSlug,
        parsedOverviewLibrarySlug,
      ),
    [location.state, parsedOverviewLibrarySlug, parsedOverviewSlug, view],
  );

  const overviewEpisodeId = useMemo(
    () => searchParams.get("episodeId")?.trim() || null,
    [searchParams],
  );

  const {
    uiLanguage,
    setLanguagePreference,
    selectedLanguage,
    t,
    getLanguageLabel,
  } = useLanguage(searchParams);

  const [, setGlobalStatusRaw] = useState("");
  const setGlobalStatus = useGlobalStatusToast(setGlobalStatusRaw);
  const shellFrameRef = useRef<HTMLDivElement>(null);
  const [shellTopOffset, setShellTopOffset] = useState(0);
  const {
    smgVersionCompatibilityNotice,
    smgScryerUpdateNotice,
    showSmgScryerUpdateReminder,
    dismissSmgScryerUpdateReminder,
  } = useSmgNotices();
  const [resolvedOverviewTarget, setResolvedOverviewTarget] =
    useState<OverviewTitleTarget | null>(null);
  const [, setOverviewSlugLoading] = useState(false);

  const setLanguagePreferenceFromShell = useCallback(
    (code: string) => {
      setLanguagePreference(code);
      setGlobalStatus(
        t("status.languageChanged", { language: getLanguageLabel(code) }),
      );
    },
    [getLanguageLabel, setLanguagePreference, t, setGlobalStatus],
  );

  const [installBannerDismissed, setInstallBannerDismissed] = useState(() => {
    if (typeof window === "undefined") {
      return false;
    }

    return window.localStorage.getItem(INSTALL_BANNER_DISMISSED_KEY) === "true";
  });
  const showInstallBanner =
    !isInstalled && !installBannerDismissed && (canPrompt || isIosSafari);
  useEffect(() => {
    const frame = shellFrameRef.current;
    if (!frame || typeof window === "undefined") {
      return;
    }

    let animationFrame: number | null = null;
    const updateShellOffset = () => {
      if (animationFrame !== null) {
        window.cancelAnimationFrame(animationFrame);
      }
      animationFrame = window.requestAnimationFrame(() => {
        const topOffset = Math.round(
          Math.max(0, frame.getBoundingClientRect().top),
        );
        setShellTopOffset((previousOffset) =>
          previousOffset === topOffset ? previousOffset : topOffset,
        );
        animationFrame = null;
      });
    };

    updateShellOffset();
    window.addEventListener("resize", updateShellOffset);
    const observer =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(updateShellOffset);
    observer?.observe(frame);
    if (frame.parentElement) {
      observer?.observe(frame.parentElement);
    }

    return () => {
      if (animationFrame !== null) {
        window.cancelAnimationFrame(animationFrame);
      }
      window.removeEventListener("resize", updateShellOffset);
      observer?.disconnect();
    };
  }, [
    isOnline,
    showInstallBanner,
    showSmgScryerUpdateReminder,
    smgScryerUpdateNotice,
    smgVersionCompatibilityNotice,
  ]);
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

  const activeFacet = useMemo<Facet>(
    () => facetForView(view)?.id ?? "movie",
    [view],
  );
  const queueFacet = activeFacet;

  const navigateTo = useCallback(
    (
      nextView: ViewId,
      nextSettingsSection?: SettingsSection,
      nextContentSection?: ContentSettingsSection,
      nextSystemSection?: SystemSection,
      nextWantedSection?: WantedSection,
      nextActivitySection?: ActivitySection,
      nextLogsSection?: LogsSection,
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
        nextView === "logs" ? nextLogsSection : undefined,
      );
      const normalizedContentSection = isMedia
        ? (nextContentSection ?? "overview")
        : "overview";
      const normalizedOverviewTitleId =
        (nextOverviewTitleId ?? "").trim().length > 0
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
      const targetPath = buildOverviewDetailPath(
        targetView,
        normalizedLibrarySlug,
        normalizedSlug,
      );
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
    const lookupLibrarySlug =
      parsedOverviewLibrarySlug === view ? null : parsedOverviewLibrarySlug;

    void resolveTitleOverviewTargetBySlug(
      backendClient,
      facet,
      lookupLibrarySlug,
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
          (target.slug !== parsedOverviewSlug ||
            target.librarySlug !== parsedOverviewLibrarySlug)
        ) {
          navigateToOverview(view, target, overviewEpisodeId, true);
        }
      })
      .catch((error: unknown) => {
        if (cancelled) {
          return;
        }

        setResolvedOverviewTarget(null);
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.apiError"),
        );
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
    ? (navigationOverviewTarget?.id ?? resolvedOverviewTarget?.id ?? null)
    : legacyOverviewTitleId;
  const overviewTitleRoutePending = Boolean(
    parsedOverviewSlug && isMediaView(view),
  );

  const handleOpenOverview = useCallback(
    (
      targetView: ViewId,
      overviewTarget: OverviewTitleTarget,
      episodeId?: string,
    ) => {
      if (!isMediaView(targetView)) {
        return;
      }

      navigateToOverview(targetView, overviewTarget, episodeId);
    },
    [navigateToOverview],
  );

  const topNav = useMemo(
    () => [
      ...FACET_REGISTRY.map((f) => ({
        id: f.viewId as ViewId,
        label: t(f.navLabelKey),
        icon: f.icon,
      })),
      {
        id: "discovery" as ViewId,
        label: t("nav.discovery"),
        icon: Sparkles,
      },
      {
        id: "activity" as ViewId,
        label: t("nav.activity"),
        icon: ActivitySquare,
      },
      {
        id: "calendar" as ViewId,
        label: t("nav.calendar"),
        icon: CalendarDays,
      },
      { id: "wanted" as ViewId, label: t("nav.wanted"), icon: ListChecks },
      { id: "system" as ViewId, label: t("system.title"), icon: Monitor },
      { id: "settings" as ViewId, label: t("nav.settings"), icon: Settings },
    ],
    [t],
  );
  const {
    canViewCatalog,
    canManageTitle,
    canRequestMedia,
    canResolveImports,
    canAccessActivity,
    canManageSystemSettings,
    canManageCatalogSettings,
    canManageUserAccounts,
    canManageUsers,
    canManageConfig,
    canManageLibrarySettings,
    canAccessRecycleBin,
  } = usePermissions(authenticatedUser);
  const discoveryAuthorizationSignature = useMemo(
    () => authorizationCacheSignature(authenticatedUser),
    [authenticatedUser],
  );
  const {
    pendingImportCounts,
    pendingMediaRequestCounts,
    manualImportRequiredCount,
    pluginUpdateCount,
    scryerVersion,
  } = useNavigationBadges({
    serviceRestarting,
    canManageTitle,
    canRequestMedia,
  });
  const viewingBackupsSettings =
    view === "settings" && settingsSection === "backups";
  const globalSearchRouteCommands = useMemo(
    () =>
      buildRouteCommands({
        t,
        user: authenticatedUser,
        activityImportCount: manualImportRequiredCount,
        onNavigate: navigateTo,
      }),
    [authenticatedUser, manualImportRequiredCount, navigateTo, t],
  );

  useAutoBackupNotice({
    canManageSystemSettings,
    serviceRestarting,
    viewingBackupsSettings,
    navigateTo,
    t,
  });

  useEffect(() => {
    if (
      !isMediaView(view) ||
      canAccessMediaSettingsSection(
        contentSettingsSection,
        canManageConfig,
        canManageLibrarySettings,
        canResolveImports,
      )
    ) {
      return;
    }

    navigateTo(
      view,
      undefined,
      fallbackMediaContentSettingsSection(
        contentSettingsSection,
        canManageConfig,
        canManageLibrarySettings,
        canResolveImports,
      ),
      undefined,
      undefined,
    );
  }, [
    canManageConfig,
    canManageLibrarySettings,
    canResolveImports,
    contentSettingsSection,
    navigateTo,
    view,
  ]);

  const navigateToAccessibleDefault = useCallback(() => {
    const fallback = defaultAccessibleRoute(
      canViewCatalog,
      canRequestMedia,
      canResolveImports,
      canManageUserAccounts,
      canManageUsers,
      canManageSystemSettings,
      canManageCatalogSettings,
      canManageLibrarySettings,
    );
    navigateTo(
      fallback.view,
      fallback.settingsSection,
      fallback.contentSettingsSection,
      undefined,
      undefined,
    );
  }, [
    canManageCatalogSettings,
    canManageLibrarySettings,
    canManageSystemSettings,
    canManageUserAccounts,
    canManageUsers,
    canResolveImports,
    canRequestMedia,
    canViewCatalog,
    navigateTo,
  ]);

  const routeCanAccessSettingsContent =
    view === "settings"
      ? canAccessSettingsSection(
          settingsSection,
          canManageUserAccounts,
          canManageUsers,
          canManageSystemSettings,
          canManageCatalogSettings,
          canAccessRecycleBin,
        )
      : !(
          isMediaView(view) &&
          !canAccessMediaSettingsSection(
            contentSettingsSection,
            canManageConfig,
            canManageLibrarySettings,
            canResolveImports,
          )
        );
  const protectedSettingsRoute =
    routeCanAccessSettingsContent &&
    isProtectedSettingsRoute(view, settingsSection, contentSettingsSection);
  const {
    refreshConfigStepUpPolicy,
    settingsStepUpCode,
    setSettingsStepUpCode,
    settingsStepUpBusy,
    settingsStepUpError,
    settingsStepUpOpen,
    settingsStepUpPolicyLoadFailed,
    settingsStepUpBlocksContent,
    handleCancelSettingsStepUp,
    handleSettingsStepUpSubmit,
  } = useConfigStepUp({
    authToken,
    initialMfaRequireConfigStepUp: mfaRequireConfigStepUp,
    protectedSettingsRoute,
    adoptSession,
    setGlobalStatus,
    navigateTo,
    t,
  });

  useEffect(() => {
    if (view !== "activity" || canAccessActivity) {
      return;
    }

    navigateToAccessibleDefault();
  }, [canAccessActivity, navigateToAccessibleDefault, view]);

  useEffect(() => {
    if ((view !== "calendar" && view !== "wanted") || canViewCatalog) {
      return;
    }

    navigateToAccessibleDefault();
  }, [canViewCatalog, navigateToAccessibleDefault, view]);

  useEffect(() => {
    if ((view !== "system" && view !== "logs") || canManageSystemSettings) {
      return;
    }

    navigateToAccessibleDefault();
  }, [canManageSystemSettings, navigateToAccessibleDefault, view]);

  useEffect(() => {
    if (
      !isMediaView(view) ||
      contentSettingsSection !== "overview" ||
      canViewCatalog
    ) {
      return;
    }

    if (canRequestMedia) {
      navigateTo("requests");
      return;
    }

    if (canResolveImports) {
      navigateTo(view, undefined, "import", undefined, undefined);
      return;
    }

    if (canManageLibrarySettings && !canManageConfig) {
      navigateTo(view, undefined, "library", undefined, undefined);
      return;
    }

    navigateToAccessibleDefault();
  }, [
    canManageConfig,
    canManageLibrarySettings,
    canRequestMedia,
    canResolveImports,
    canViewCatalog,
    contentSettingsSection,
    navigateTo,
    navigateToAccessibleDefault,
    view,
  ]);

  useEffect(() => {
    if (isMediaView(view) && contentSettingsSection === "requests") {
      if (canManageTitle || canRequestMedia) {
        navigateTo("requests");
        return;
      }
      navigateToAccessibleDefault();
      return;
    }

    if (view !== "requests" || canManageTitle || canRequestMedia) {
      return;
    }

    navigateToAccessibleDefault();
  }, [
    canManageTitle,
    canRequestMedia,
    contentSettingsSection,
    navigateTo,
    navigateToAccessibleDefault,
    view,
  ]);

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

    if (
      canAccessSettingsSection(
        settingsSection,
        canManageUserAccounts,
        canManageUsers,
        canManageSystemSettings,
        canManageCatalogSettings,
        canAccessRecycleBin,
      )
    ) {
      return;
    }

    navigateTo(
      "settings",
      defaultSettingsSection(
        canManageSystemSettings,
        canManageCatalogSettings,
        canManageUserAccounts,
        canManageUsers,
      ),
    );
  }, [
    canManageCatalogSettings,
    canManageSystemSettings,
    canManageUserAccounts,
    canManageUsers,
    canAccessRecycleBin,
    navigateTo,
    settingsSection,
    view,
  ]);

  const handleBackToList = useCallback(() => {
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
  }, [navigate, searchParams, view]);

  useEffect(() => {
    if (
      view !== "activity" ||
      activitySection !== "import" ||
      pendingImportCounts === null ||
      manualImportRequiredCount > 0
    ) {
      return;
    }

    navigateTo(
      "activity",
      undefined,
      undefined,
      undefined,
      undefined,
      "activity",
    );
  }, [
    activitySection,
    manualImportRequiredCount,
    navigateTo,
    pendingImportCounts,
    view,
  ]);

  return (
    <ScryerGraphqlProvider language={uiLanguage}>
      <TranslateContext.Provider value={t}>
        <GlobalStatusContext.Provider value={setGlobalStatus}>
          <div
            data-slot="root-app-frame"
            className="flex min-h-dvh flex-col overflow-x-hidden text-[var(--scry-body)]"
          >
            {serviceRestarting && <BackendRestartOverlay />}
            <Suspense fallback={<ViewLoadingFallback />}>
              <LibraryScanProgressProvider>
                <JobRunProvider>
                  <ReactiveRefreshProvider>
                    <GlobalSearchProvider
                      activeFacet={activeFacet}
                      authenticatedUser={authenticatedUser}
                      queueFacet={queueFacet}
                      uiLanguage={uiLanguage}
                    >
                      {smgVersionCompatibilityNotice ? (
                        <SmgUpgradeBanner
                          notice={smgVersionCompatibilityNotice}
                          t={t}
                        />
                      ) : null}

                      {showSmgScryerUpdateReminder && smgScryerUpdateNotice ? (
                        <SmgScryerUpdateBanner
                          notice={smgScryerUpdateNotice}
                          t={t}
                          onDismiss={dismissSmgScryerUpdateReminder}
                        />
                      ) : null}

                      {!isOnline ? (
                        <div
                          data-slot="root-shell-notice"
                          className="flex items-center justify-center gap-2 border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-surfA))] px-4 py-2 text-sm font-medium text-[var(--scry-body)] shadow-[0_8px_28px_rgba(2,6,23,0.14)] backdrop-blur"
                        >
                          <WifiOff className="h-4 w-4 flex-none text-[var(--scry-accent-ring)]" />
                          <span className="text-[var(--scry-ink2)]">
                            {t("pwa.offline")}
                          </span>
                        </div>
                      ) : null}

                      {showInstallBanner ? (
                        <div
                          data-slot="root-shell-notice"
                          className="flex items-center justify-center gap-3 border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-surfA))] px-4 py-2 text-sm text-[var(--scry-body)] shadow-[0_8px_28px_rgba(var(--scry-accent-rgb),0.10)] backdrop-blur"
                        >
                          <Download className="h-4 w-4 flex-none text-[var(--scry-accent-ring)]" />
                          <span className="text-[var(--scry-muted)]">
                            {isIosSafari
                              ? t("pwa.iosInstallHint")
                              : t("pwa.installApp")}
                          </span>
                          {canPrompt ? (
                            <Button
                              type="button"
                              onClick={() => void promptInstall()}
                              variant="outline"
                              size="sm"
                              className="h-7 rounded-[8px] px-3 text-xs font-semibold text-[var(--scry-accent-text)]"
                            >
                              {t("pwa.installApp")}
                            </Button>
                          ) : null}
                          <IconButton
                            type="button"
                            onClick={dismissInstallBanner}
                            label={t("label.dismiss")}
                            appearance="ghost"
                            className="ml-auto h-7 w-7 rounded-[8px]"
                          >
                            <X className="h-4 w-4" />
                          </IconButton>
                        </div>
                      ) : null}

                      <Dialog
                        open={settingsStepUpOpen}
                        onOpenChange={(open) => {
                          if (!open && settingsStepUpOpen) {
                            handleCancelSettingsStepUp();
                          }
                        }}
                      >
                        <DialogContent
                          id="settings-mfa-step-up-dialog"
                          className="sm:max-w-md"
                          onInteractOutside={(event) => event.preventDefault()}
                        >
                          <DialogHeader>
                            <DialogTitle>
                              {t("settings.mfaStepUpTitle")}
                            </DialogTitle>
                            <DialogDescription>
                              {t("settings.mfaStepUpDescription")}
                            </DialogDescription>
                          </DialogHeader>
                          <TotpCodeForm
                            id="settings-mfa-step-up-form"
                            inputId="settings-mfa-step-up-code"
                            submitId="settings-mfa-step-up-submit"
                            cancelId="settings-mfa-step-up-cancel"
                            code={settingsStepUpCode}
                            title={t("auth.totpCode")}
                            description={t("auth.totpCodeRequired")}
                            submitLabel={t("settings.mfaStepUpSubmit")}
                            busyLabel={t("settings.mfaStepUpVerifying")}
                            cancelLabel={t("label.cancel")}
                            busy={settingsStepUpBusy}
                            onCodeChange={setSettingsStepUpCode}
                            onSubmit={handleSettingsStepUpSubmit}
                            onCancel={handleCancelSettingsStepUp}
                          />
                          {settingsStepUpError ? (
                            <p
                              id="settings-mfa-step-up-error"
                              className="text-sm text-destructive"
                            >
                              {settingsStepUpError}
                            </p>
                          ) : null}
                        </DialogContent>
                      </Dialog>

                      <div
                        ref={shellFrameRef}
                        data-slot="root-shell-frame"
                        className="flex min-h-0 w-full flex-1 min-[981px]:h-[calc(100dvh-var(--root-shell-top-offset,0px))] min-[981px]:max-h-[calc(100dvh-var(--root-shell-top-offset,0px))] min-[981px]:overflow-hidden"
                        style={
                          {
                            "--root-shell-top-offset": `${shellTopOffset}px`,
                          } as CSSProperties
                        }
                      >
                        <RootSidebar
                          topNav={topNav}
                          view={view}
                          settingsSection={settingsSection}
                          contentSettingsSection={contentSettingsSection}
                          systemSection={systemSection}
                          logsSection={logsSection}
                          activitySection={activitySection}
                          wantedSection={wantedSection}
                          user={authenticatedUser}
                          pendingImportCounts={pendingImportCounts}
                          pendingMediaRequestCounts={pendingMediaRequestCounts}
                          manualImportRequiredCount={manualImportRequiredCount}
                          pluginUpdateCount={pluginUpdateCount}
                          header={
                            <RootHeader
                              onOpenOverview={handleOpenOverview}
                              routeCommandItems={globalSearchRouteCommands}
                            />
                          }
                          onNavigate={navigateTo}
                        >
                          <main
                            data-slot="root-main-scroll"
                            className="flex min-h-[70vh] flex-1 flex-col min-[981px]:min-h-0 min-[981px]:overflow-y-auto"
                          >
                            <Suspense fallback={<ViewLoadingFallback />}>
                              {settingsStepUpPolicyLoadFailed ? (
                                <div className="mx-auto flex min-h-[360px] w-full max-w-md flex-col items-center justify-center gap-3 px-6 py-12 text-center">
                                  <div className="flex h-12 w-12 items-center justify-center rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-chip)] text-amber-500 shadow-[0_12px_28px_rgba(2,6,23,0.12)]">
                                    <AlertTriangle
                                      className="h-6 w-6"
                                      aria-hidden="true"
                                    />
                                  </div>
                                  <h2 className="text-lg font-bold text-[var(--scry-ink2)]">
                                    {t("settings.mfaStepUpPolicyLoadFailed")}
                                  </h2>
                                  <p className="text-sm leading-6 text-[var(--scry-muted3)]">
                                    {t(
                                      "settings.mfaStepUpPolicyLoadFailedDescription",
                                    )}
                                  </p>
                                  <Button
                                    id="settings-mfa-step-up-policy-retry"
                                    type="button"
                                    onClick={() =>
                                      void refreshConfigStepUpPolicy()
                                    }
                                  >
                                    {t("settings.mfaStepUpPolicyRetry")}
                                  </Button>
                                </div>
                              ) : settingsStepUpBlocksContent ? (
                                <ViewLoadingFallback />
                              ) : (
                                <MainContent
                                  view={view}
                                  overviewTitleId={overviewTitleId}
                                  overviewTitleRoutePending={
                                    overviewTitleRoutePending
                                  }
                                  routeOverviewEpisodeId={overviewEpisodeId}
                                  handleBackToList={handleBackToList}
                                  settingsSection={settingsSection}
                                  userId={authenticatedUser.id}
                                  username={authenticatedUser.username}
                                  selectedLanguage={selectedLanguage}
                                  uiLanguage={uiLanguage}
                                  discoveryAuthorizationSignature={
                                    discoveryAuthorizationSignature
                                  }
                                  setLanguagePreferenceFromShell={
                                    setLanguagePreferenceFromShell
                                  }
                                  contentSettingsSection={
                                    contentSettingsSection
                                  }
                                  systemSection={systemSection}
                                  logsSection={logsSection}
                                  scryerVersion={scryerVersion}
                                  activitySection={activitySection}
                                  wantedSection={wantedSection}
                                  handleOpenOverview={handleOpenOverview}
                                  handleImportRouteEmpty={handleBackToList}
                                  canViewCatalog={canViewCatalog}
                                  canAccessActivity={canAccessActivity}
                                  canAccessRecycleBin={canAccessRecycleBin}
                                  canResolveImports={canResolveImports}
                                  canManageTitle={canManageTitle}
                                  canRequestMedia={canRequestMedia}
                                  canManageUserAccounts={canManageUserAccounts}
                                  canManageUsers={canManageUsers}
                                  canManageSystemSettings={
                                    canManageSystemSettings
                                  }
                                  canManageCatalogSettings={
                                    canManageCatalogSettings
                                  }
                                  canManageConfig={canManageConfig}
                                  canManageLibrarySettings={
                                    canManageLibrarySettings
                                  }
                                />
                              )}
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
