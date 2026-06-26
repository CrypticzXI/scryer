import { lazy, Suspense, useState, useCallback, useMemo } from "react";
import { ActivitySquare, CalendarDays, Clapperboard, Compass, FileText, Film, History, Inbox, ListChecks, Monitor, MonitorCog, Settings } from "lucide-react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { RootHeader } from "@/components/root/root-header";
import { RootSidebar } from "@/components/root/root-sidebar";
import { GlobalSearchProvider } from "@/components/root/global-search-provider";
import { ViewLoadingFallback } from "@/components/common/view-loading-fallback";
import { useLanguage } from "@/lib/hooks/use-language";
import { useGlobalStatusToast } from "@/lib/hooks/use-global-status-toast";
import { useAuth, type AuthUser } from "@/lib/hooks/use-auth";
import { ScryerGraphqlProvider } from "@/lib/graphql/urql-provider";
import { TranslateContext } from "@/lib/context/translate-context";
import { GlobalStatusContext } from "@/lib/context/global-status-context";
import type {
  ActivitySection,
  OverviewTitleTarget,
  ViewId,
  SettingsSection,
  ContentSettingsSection,
  LogsSection,
  SystemSection,
  WantedSection,
} from "@/components/root/types";
import { buildOverviewDetailPath, buildViewPath } from "@/lib/utils/routing";

const SeriesOverviewContainer = lazy(() =>
  import("@/components/containers/series-overview-container").then((m) => ({ default: m.SeriesOverviewContainer })),
);

const TOP_NAV_IDS: ViewId[] = [
  "movies",
  "series",
  "anime",
  "discovery",
  "requests",
  "activity",
  "calendar",
  "wanted",
  "settings",
  "system",
];

const TOP_NAV_ICONS: Record<ViewId, typeof Film> = {
  movies: Film,
  series: Monitor,
  anime: Clapperboard,
  discovery: Compass,
  requests: Inbox,
  activity: ActivitySquare,
  calendar: CalendarDays,
  wanted: ListChecks,
  history: History,
  settings: Settings,
  logs: FileText,
  system: MonitorCog,
};

const EMPTY_AUTH_USER: AuthUser = {
  id: "",
  username: "",
  appPermissions: [],
  libraryPermissions: [],
};

export function SeriesOverviewShell() {
  const [searchParams] = useSearchParams();
  const titleId = searchParams.get("id") ?? "";
  const navigate = useNavigate();
  const { user } = useAuth();
  const permissionUser = user ?? EMPTY_AUTH_USER;

  const { uiLanguage, t } = useLanguage(searchParams);

  const [, setGlobalStatusRaw] = useState("");
  const setGlobalStatus = useGlobalStatusToast(setGlobalStatusRaw);

  const topNav = useMemo(
    () =>
      TOP_NAV_IDS.map((id) => ({
        id,
        label: t(`nav.${id}`),
        icon: TOP_NAV_ICONS[id],
      })),
    [t],
  );

  const handleTitleNotFound = useCallback(() => {
    navigate("/series", { replace: true });
  }, [navigate]);

  const handleOpenOverview = useCallback(
    (targetView: ViewId, overviewTarget: OverviewTitleTarget) => {
      const normalizedTitleId = overviewTarget.id.trim();
      if (!normalizedTitleId) {
        return;
      }

      const normalizedSlug = overviewTarget.slug?.trim() || null;
      const normalizedLibrarySlug = overviewTarget.librarySlug?.trim() || null;
      const targetPath = buildOverviewDetailPath(targetView, normalizedLibrarySlug, normalizedSlug);
      const nextParams = new URLSearchParams();
      if (!normalizedSlug || !normalizedLibrarySlug) {
        nextParams.set("id", normalizedTitleId);
      }

      const nextQuery = nextParams.toString();
      navigate(`${targetPath}${nextQuery ? `?${nextQuery}` : ""}`);
    },
    [navigate],
  );

  const navigateTo = useCallback(
    (
      nextView: ViewId,
      nextSettingsSection?: SettingsSection,
      nextContentSection?: ContentSettingsSection,
      nextSystemSection?: SystemSection,
      nextWantedSection?: WantedSection,
      nextActivitySection?: ActivitySection,
      nextLogsSection?: LogsSection,
    ) => {
      const targetPath = buildViewPath(
        nextView,
        nextView === "settings" ? nextSettingsSection : undefined,
        nextView === "movies" ||
          nextView === "series" ||
          nextView === "anime"
          ? nextContentSection
          : undefined,
        nextView === "system" ? nextSystemSection : undefined,
        nextView === "wanted" ? nextWantedSection : undefined,
        nextView === "activity" ? nextActivitySection : undefined,
        nextView === "logs" ? nextLogsSection : undefined,
      );
      navigate(targetPath);
    },
    [navigate],
  );

  return (
    <ScryerGraphqlProvider language={uiLanguage}>
      <TranslateContext.Provider value={t}>
        <GlobalStatusContext.Provider value={setGlobalStatus}>
          <GlobalSearchProvider
            activeFacet="series"
            authenticatedUser={permissionUser}
            queueFacet="series"
            uiLanguage={uiLanguage}
          >
            <div
              data-slot="root-app-frame"
              className="flex min-h-dvh flex-col overflow-x-hidden text-[var(--scry-body)]"
            >
              <div
                data-slot="root-shell-frame"
                className="flex min-h-0 w-full flex-1 min-[981px]:h-[calc(100dvh-var(--root-shell-top-offset,0px))] min-[981px]:max-h-[calc(100dvh-var(--root-shell-top-offset,0px))] min-[981px]:overflow-hidden"
              >
                <RootSidebar
                  topNav={topNav}
                  view="series"
                  settingsSection="profile"
                  contentSettingsSection="overview"
                  systemSection="overview"
                  logsSection="logs"
                  activitySection="activity"
                  wantedSection="wanted"
                  user={permissionUser}
                  pendingImportCounts={null}
                  pendingMediaRequestCounts={null}
                  manualImportRequiredCount={0}
                  pluginUpdateCount={0}
                  scryerVersion={null}
                  header={
                    <RootHeader
                      onOpenOverview={handleOpenOverview}
                    />
                  }
                  onNavigate={navigateTo}
                >
                  <main
                    data-slot="root-main-scroll"
                    className="flex min-h-[70vh] flex-1 flex-col min-[981px]:min-h-0 min-[981px]:overflow-y-auto"
                  >
                    <Suspense fallback={<ViewLoadingFallback />}>
                      <SeriesOverviewContainer
                        titleId={titleId}
                        onTitleNotFound={handleTitleNotFound}
                      />
                    </Suspense>
                  </main>
                </RootSidebar>
              </div>
            </div>
          </GlobalSearchProvider>
        </GlobalStatusContext.Provider>
      </TranslateContext.Provider>
    </ScryerGraphqlProvider>
  );
}
