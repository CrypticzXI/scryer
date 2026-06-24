import { lazy, Suspense, useState, useCallback, useMemo } from "react";
import { ActivitySquare, CalendarDays, Clapperboard, Film, History, ListChecks, Monitor, MonitorCog, Settings } from "lucide-react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { RootHeader } from "@/components/root/root-header";
import { RootSidebar } from "@/components/root/root-sidebar";
import { GlobalSearchProvider } from "@/components/root/global-search-provider";
import { ViewLoadingFallback } from "@/components/common/view-loading-fallback";
import { buildRouteCommands } from "@/components/root/route-commands";
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
  SystemSection,
  WantedSection,
} from "@/components/root/types";
import { buildOverviewDetailPath, buildViewPath } from "@/lib/utils/routing";

const MovieOverviewContainer = lazy(() =>
  import("@/components/containers/movie-overview-container").then((m) => ({ default: m.MovieOverviewContainer })),
);

// Minimal nav items — same sidebar as the main shell so navigation feels consistent.
const TOP_NAV_IDS: ViewId[] = ["movies", "series", "anime", "activity", "calendar", "wanted", "settings", "system"];

const TOP_NAV_ICONS: Record<ViewId, typeof Film> = {
  movies: Film,
  series: Monitor,
  anime: Clapperboard,
  activity: ActivitySquare,
  calendar: CalendarDays,
  wanted: ListChecks,
  history: History,
  settings: Settings,
  system: MonitorCog,
};

const EMPTY_AUTH_USER: AuthUser = {
  id: "",
  username: "",
  appPermissions: [],
  libraryPermissions: [],
};

export function MovieOverviewShell() {
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
    navigate("/movies", { replace: true });
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
    ) => {
      const targetPath = buildViewPath(
        nextView,
        nextView === "settings" ? nextSettingsSection : undefined,
        nextView === "movies" || nextView === "series" || nextView === "anime" ? nextContentSection : undefined,
        nextView === "system" ? nextSystemSection : undefined,
        nextView === "wanted" ? nextWantedSection : undefined,
        nextView === "activity" ? nextActivitySection : undefined,
      );
      navigate(targetPath);
    },
    [navigate],
  );

  const routeCommandPalette = useMemo(() => {
    return buildRouteCommands({
      t,
      user: permissionUser,
      activeFacet: "movie",
      activityImportCount: 0,
      onNavigate: navigateTo,
    });
  }, [navigateTo, permissionUser, t]);

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

  return (
    <ScryerGraphqlProvider language={uiLanguage}>
      <TranslateContext.Provider value={t}>
        <GlobalStatusContext.Provider value={setGlobalStatus}>
          <GlobalSearchProvider
            activeFacet="movie"
            authenticatedUser={permissionUser}
            queueFacet="movie"
            uiLanguage={uiLanguage}
          >
            <div className="flex min-h-screen flex-col bg-background text-foreground">
              <div className="flex min-h-0 w-full flex-1">
                <RootSidebar
                  topNav={topNav}
                  view="movies"
                  settingsSection="profile"
                  contentSettingsSection="overview"
                  systemSection="overview"
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
                      routeCommandPalette={routeCommandPaletteConfig}
                      onOpenOverview={handleOpenOverview}
                    />
                  }
                  onNavigate={navigateTo}
                >
                  <main className="min-h-[70vh]">
                    <Suspense fallback={<ViewLoadingFallback />}>
                      <MovieOverviewContainer
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
