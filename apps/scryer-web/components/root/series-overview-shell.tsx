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
import { ScryerGraphqlProvider } from "@/lib/graphql/urql-provider";
import { TranslateContext } from "@/lib/context/translate-context";
import { GlobalStatusContext } from "@/lib/context/global-status-context";
import type {
  ActivitySection,
  ViewId,
  SettingsSection,
  ContentSettingsSection,
  SystemSection,
  WantedSection,
} from "@/components/root/types";
import { buildViewPath } from "@/lib/utils/routing";

const SeriesOverviewContainer = lazy(() =>
  import("@/components/containers/series-overview-container").then((m) => ({ default: m.SeriesOverviewContainer })),
);

const TOP_NAV_IDS: ViewId[] = [
  "movies",
  "series",
  "anime",
  "activity",
  "calendar",
  "wanted",
  "history",
  "settings",
  "system",
];

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

export function SeriesOverviewShell() {
  const [searchParams] = useSearchParams();
  const titleId = searchParams.get("id") ?? "";
  const navigate = useNavigate();

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
        nextView === "movies" ||
          nextView === "series" ||
          nextView === "anime"
          ? nextContentSection
          : undefined,
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
      pendingImportCounts: null,
      activityImportCount: 0,
      onNavigate: navigateTo,
    });
  }, [navigateTo, t]);

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
          <GlobalSearchProvider activeFacet="series" queueFacet="series" uiLanguage={uiLanguage}>
            <div className="min-h-screen bg-background text-foreground">
              <RootHeader routeCommandPalette={routeCommandPaletteConfig} />

              <div className="mx-auto w-full max-w-[1480px] px-3 pb-10 pt-4">
                <RootSidebar
                  topNav={topNav}
                  view="series"
                  settingsSection="profile"
                  contentSettingsSection="overview"
                  systemSection="overview"
                  activitySection="activity"
                  wantedSection="wanted"
                  entitlements={[]}
                  pendingImportCounts={null}
                  manualImportRequiredCount={0}
                  onNavigate={navigateTo}
                >
                  <main className="min-h-[70vh]">
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
