
import * as React from "react";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import type { LucideIcon } from "lucide-react";
import type {
  ActivitySection,
  ContentSettingsSection,
  SettingsSection,
  SystemSection,
  Translate,
  ViewId,
  WantedSection,
} from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSub,
  SidebarMenuSubButton,
  SidebarMenuSubItem,
  SidebarInset,
  SidebarProvider,
  SidebarTrigger,
  useSidebar,
} from "@/components/ui/sidebar";
import { Collapsible, CollapsibleContent } from "@/components/ui/collapsible";
import { ChevronRight, Monitor, Moon, Rainbow, Sun } from "lucide-react";
import { useTheme } from "next-themes";
import { getNextTheme, getThemeLabel } from "@/lib/theme";
import { cn } from "@/lib/utils";
import type { PendingImportCounts } from "@/lib/types";
import { hasImportItemsForView, pendingImportCountForView } from "@/lib/types";
import type { AuthUser } from "@/lib/hooks/use-auth";
import { APP_PERMISSIONS, LIBRARY_PERMISSIONS, hasAnyAppPermission, hasAnyLibraryPermission } from "@/lib/utils/permissions";
import type { AppPermission, LibraryPermission } from "@/lib/utils/permissions";
import { selectorId } from "@/lib/utils/dom-ids";
import ScryerLogo from "@/components/scryer-logo";

type NavItem = {
  id: ViewId;
  label: string;
  icon: LucideIcon;
};

type TopNavGroupDefinition = {
  id: string;
  labelKey: string;
  itemIds: ViewId[];
};

type TopNavGroup = {
  id: string;
  label: string;
  items: NavItem[];
};

const TOP_NAV_GROUPS: TopNavGroupDefinition[] = [
  { id: "catalogs", labelKey: "nav.group.catalogs", itemIds: ["movies", "series", "anime"] },
  { id: "automation", labelKey: "nav.group.automation", itemIds: ["wanted", "calendar", "activity"] },
  { id: "configuration", labelKey: "nav.group.configuration", itemIds: ["settings", "system"] },
];

type RootSidebarProps = {
  topNav: NavItem[];
  view: ViewId;
  settingsSection: SettingsSection;
  contentSettingsSection: ContentSettingsSection;
  systemSection: SystemSection;
  activitySection: ActivitySection;
  wantedSection: WantedSection;
  user: AuthUser;
  pendingImportCounts: PendingImportCounts | null;
  pendingMediaRequestCounts: PendingImportCounts | null;
  manualImportRequiredCount: number;
  pluginUpdateCount: number;
  scryerVersion: string | null;
  children?: React.ReactNode;
  onNavigate: (
    nextView: ViewId,
    nextSettingsSection?: SettingsSection,
    nextContentSection?: ContentSettingsSection,
    nextSystemSection?: SystemSection,
    nextWantedSection?: WantedSection,
    nextActivitySection?: ActivitySection,
  ) => void;
};

const settingsEntries: Array<{
  id: SettingsSection;
  label: (t: Translate) => string;
  requiredAnyAppPermission?: AppPermission[];
  requiredAnyLibraryPermission?: LibraryPermission[];
}> = [
  {
    id: "profile",
    label: (t) => t("settings.profile"),
  },
  {
    id: "general",
    label: (t) => t("settings.general"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageSystemSettings],
  },
  {
    id: "backups",
    label: (t) => t("settings.backups"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageSystemSettings],
  },
  {
    id: "security",
    label: (t) => t("settings.security"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageUsers],
  },
  {
    id: "users",
    label: (t) => t("settings.users"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageUsers, APP_PERMISSIONS.managePermissions],
  },
  {
    id: "mediaServers",
    label: (t) => t("settings.mediaServers"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageSystemSettings],
  },
  {
    id: "qualityProfiles",
    label: (t) => t("settings.qualityProfiles"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageCatalogSettings],
  },
  {
    id: "delayProfiles",
    label: (t) => t("settings.delayProfiles"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageCatalogSettings],
  },
  {
    id: "downloadClients",
    label: (t) => t("settings.downloadClients"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageSystemSettings],
  },
  {
    id: "indexers",
    label: (t) => t("settings.indexers"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageSystemSettings],
  },
  {
    id: "rules",
    label: (t) => t("settings.rules"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageCatalogSettings],
  },
  {
    id: "plugins",
    label: (t) => t("settings.plugins"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageSystemSettings],
  },
  {
    id: "notifications",
    label: (t) => t("settings.notifications"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageSystemSettings],
  },
  {
    id: "post-processing",
    label: (t) => t("settings.postProcessing"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageCatalogSettings],
  },
  {
    id: "subtitles",
    label: (t) => t("settings.subtitles"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageCatalogSettings],
  },
  {
    id: "recycleBin",
    label: (t) => t("settings.recycleBin"),
    requiredAnyAppPermission: [APP_PERMISSIONS.manageSystemSettings],
    requiredAnyLibraryPermission: [LIBRARY_PERMISSIONS.manageTitles],
  },
];

const MEDIA_SETTINGS_SUB_PAGES: Array<{ id: ContentSettingsSection; labelKey: string }> = [
  { id: "library", labelKey: "nav.library" },
  { id: "general", labelKey: "facetSettings.general" },
  { id: "quality", labelKey: "facetSettings.quality" },
  { id: "renaming", labelKey: "facetSettings.renaming" },
  { id: "routing", labelKey: "facetSettings.routing" },
];

const SYSTEM_SUB_PAGES: Array<{ id: SystemSection; labelKey: string }> = [
  { id: "overview", labelKey: "system.title" },
  { id: "jobs", labelKey: "system.jobsTitle" },
  { id: "audit", labelKey: "system.auditTitle" },
];

const ACTIVITY_SUB_PAGES: Array<{ id: ActivitySection; labelKey: string }> = [
  { id: "import", labelKey: "activity.import" },
  { id: "activity", labelKey: "activity.activity" },
  { id: "history", labelKey: "activity.history" },
];

const WANTED_SUB_PAGES: Array<{ id: WantedSection; labelKey: string }> = [
  { id: "wanted", labelKey: "wanted.tabWanted" },
  { id: "cutoff", labelKey: "wanted.tabCutoff" },
  { id: "pending", labelKey: "wanted.tabPending" },
  { id: "history", labelKey: "history.title" },
];

const LEAF_NAV_BADGE_BASE_CLASS =
  "ml-auto inline-flex h-4 min-w-4 items-center justify-center rounded-md px-1 text-[10px] font-medium leading-none tabular-nums";

const TOP_NAV_BADGE_GROUP_CLASS =
  "pointer-events-none absolute right-1 flex items-center gap-1 select-none peer-data-[size=sm]/menu-button:top-1 peer-data-[size=default]/menu-button:top-1.5 peer-data-[size=lg]/menu-button:top-2.5 group-data-[collapsible=icon]:hidden";

const TOP_NAV_BADGE_BASE_CLASS =
  "inline-flex h-5 min-w-5 items-center justify-center rounded-md px-1 text-xs font-medium tabular-nums";

type NavBadgeTone = "cta" | "danger" | "request";

function navBadgeToneClass(tone: NavBadgeTone) {
  switch (tone) {
    case "danger":
      return "bg-red-600 text-white dark:bg-red-500 dark:text-white";
    case "request":
      return "bg-emerald-600 text-white dark:bg-emerald-500 dark:text-white";
    case "cta":
    default:
      return "bg-primary text-primary-foreground";
  }
}

function FacetNavBadges({
  importCount,
  requestCount,
}: {
  importCount: number;
  requestCount: number;
}) {
  if (importCount <= 0 && requestCount <= 0) {
    return null;
  }

  return (
    <div className={TOP_NAV_BADGE_GROUP_CLASS}>
      {importCount > 0 ? (
        <span className={cn(TOP_NAV_BADGE_BASE_CLASS, navBadgeToneClass("cta"))}>
          {importCount}
        </span>
      ) : null}
      {requestCount > 0 ? (
        <span
          className={cn(TOP_NAV_BADGE_BASE_CLASS, navBadgeToneClass("request"))}
        >
          {requestCount}
        </span>
      ) : null}
    </div>
  );
}

function LeafNavBadge({
  count,
  tone = "cta",
}: {
  count: number;
  tone?: NavBadgeTone;
}) {
  return (
    <span
      className={cn(
        LEAF_NAV_BADGE_BASE_CLASS,
        navBadgeToneClass(tone),
      )}
    >
      {count}
    </span>
  );
}

function isSettingsSubPage(section: ContentSettingsSection): boolean {
  return section === "library" || section === "general" || section === "quality" || section === "renaming" || section === "routing";
}

function getMediaOverviewLabel(_viewId: ViewId, t: Translate): string {
  return t("nav.library");
}

function getMediaSettingsLabel(_viewId: ViewId, t: Translate): string {
  return t("nav.settings");
}

function RootSidebarContent({
  topNav,
  view,
  settingsSection,
  contentSettingsSection,
  systemSection,
  activitySection,
  wantedSection,
  user,
  pendingImportCounts,
  pendingMediaRequestCounts,
  manualImportRequiredCount,
  pluginUpdateCount,
  scryerVersion,
  children,
  onNavigate,
}: RootSidebarProps) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const { setOpenMobile } = useSidebar();
  const { theme, setTheme } = useTheme();
  const [themeMounted, setThemeMounted] = React.useState(false);
  React.useEffect(() => setThemeMounted(true), []);
  const cycleTheme = React.useCallback(() => {
    setTheme(getNextTheme(theme));
  }, [theme, setTheme]);
  const canManageSystemSettings = hasAnyAppPermission(user, [APP_PERMISSIONS.manageSystemSettings]);
  const canManageCatalogSettings = hasAnyAppPermission(user, [APP_PERMISSIONS.manageCatalogSettings]);
  const canManageConfig = canManageSystemSettings || canManageCatalogSettings;
  const canManageLibrarySettings =
    canManageConfig || hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.manageLibrary);
  const canViewCatalog = hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.view);
  const canManageTitle = hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.manageTitles);
  const canRequestMedia = hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.request);
  const canResolveImports = hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.resolveImports);
  const canAccessFacetImport = canResolveImports;
  const visibleMediaSettingsSubPages = React.useMemo(
    () =>
      canManageConfig
        ? MEDIA_SETTINGS_SUB_PAGES
        : canManageLibrarySettings
          ? MEDIA_SETTINGS_SUB_PAGES.filter((subPage) => subPage.id === "library")
          : [],
    [canManageConfig, canManageLibrarySettings],
  );
  const canAccessMediaSettings = visibleMediaSettingsSubPages.length > 0;

  const visibleSettingsEntries = React.useMemo(
    () =>
      settingsEntries.filter(
        (entry) =>
          (!entry.requiredAnyAppPermission ||
            hasAnyAppPermission(user, entry.requiredAnyAppPermission)) ||
          entry.requiredAnyLibraryPermission?.some((permission) =>
            hasAnyLibraryPermission(user, permission),
          ),
      ),
    [user],
  );
  const visibleTopNav = React.useMemo(
    () =>
      topNav.filter(
        (item) =>
          (!isMobile || item.id !== "calendar") &&
          (item.id !== "system" || canManageSystemSettings) &&
          (item.id !== "activity" || canResolveImports || canManageTitle),
      ),
    [canManageSystemSettings, canManageTitle, canResolveImports, isMobile, topNav],
  );
  const groupedTopNav = React.useMemo<TopNavGroup[]>(() => {
    const itemsById = new Map(visibleTopNav.map((item) => [item.id, item]));
    const groupedIds = new Set<ViewId>();
    const groups = TOP_NAV_GROUPS.map((group) => {
      const items = group.itemIds.flatMap((id) => {
        const item = itemsById.get(id);
        if (!item) {
          return [];
        }
        groupedIds.add(id);
        return [item];
      });
      return { id: group.id, label: t(group.labelKey), items };
    }).filter((group) => group.items.length > 0);
    const ungroupedItems = visibleTopNav.filter((item) => !groupedIds.has(item.id));

    return ungroupedItems.length > 0
      ? [...groups, { id: "more", label: t("nav.group.more"), items: ungroupedItems }]
      : groups;
  }, [t, visibleTopNav]);

  const hasImportsForView = React.useCallback(
    (viewId: ViewId) => hasImportItemsForView(pendingImportCounts, viewId),
    [pendingImportCounts],
  );

  const pendingImportCountForNavView = React.useCallback(
    (viewId: ViewId) => pendingImportCountForView(pendingImportCounts, viewId),
    [pendingImportCounts],
  );
  const pendingMediaRequestCountForNavView = React.useCallback(
    (viewId: ViewId) => pendingImportCountForView(pendingMediaRequestCounts, viewId),
    [pendingMediaRequestCounts],
  );
  const activityImportBadgeCount = Math.max(0, manualImportRequiredCount);
  const hasActivityImportBadge = activityImportBadgeCount > 0;
  const visibleActivitySubPages = React.useMemo(
    () => ACTIVITY_SUB_PAGES.filter((entry) => entry.id !== "import" || hasActivityImportBadge),
    [hasActivityImportBadge],
  );
  const hasVisibleActivitySubnav = React.useMemo(
    () => visibleActivitySubPages.some((entry) => entry.id !== "activity"),
    [visibleActivitySubPages],
  );
  const visibleWantedSubPages = React.useMemo(
    () => WANTED_SUB_PAGES.filter((entry) => entry.id !== "history" || canManageTitle),
    [canManageTitle],
  );

  const handleNavigate = React.useCallback(
    (
      event: React.MouseEvent,
      nextView: ViewId,
      nextSettingsSection?: SettingsSection,
      nextContentSection?: ContentSettingsSection,
      nextSystemSection?: SystemSection,
      nextWantedSection?: WantedSection,
      nextActivitySection?: ActivitySection,
    ) => {
      event.preventDefault();
      onNavigate(
        nextView,
        nextSettingsSection,
        nextContentSection,
        nextSystemSection,
        nextWantedSection,
        nextActivitySection,
      );
      if (isMobile) {
        setOpenMobile(false);
      }
    },
    [isMobile, onNavigate, setOpenMobile],
  );

  const currentTopLevelLabel = React.useMemo(
    () =>
      visibleTopNav.find((item) => item.id === view)?.label ??
      topNav.find((item) => item.id === view)?.label ??
      t("nav.library"),
    [topNav, t, view, visibleTopNav],
  );

  const currentSubsectionLabel = React.useMemo(() => {
    if (view === "settings") {
      return visibleSettingsEntries.find((entry) => entry.id === settingsSection)?.label(t) ?? null;
    }

    if (view === "movies" || view === "series" || view === "anime") {
      if (contentSettingsSection === "overview") {
        return getMediaOverviewLabel(view, t);
      }

      if (contentSettingsSection === "import") {
        return canAccessFacetImport ? t("nav.import") : getMediaOverviewLabel(view, t);
      }

      if (contentSettingsSection === "requests") {
        return canManageTitle || canRequestMedia ? t("nav.requests") : getMediaOverviewLabel(view, t);
      }

      if (isSettingsSubPage(contentSettingsSection)) {
        if (!canAccessMediaSettings) {
          return getMediaOverviewLabel(view, t);
        }
        const mediaSettingsLabel = visibleMediaSettingsSubPages.find(
          (subPage) => subPage.id === contentSettingsSection,
        )?.labelKey;
        return mediaSettingsLabel ? t(mediaSettingsLabel) : getMediaOverviewLabel(view, t);
      }
    }

    if (view === "system") {
      return SYSTEM_SUB_PAGES.find((entry) => entry.id === systemSection)?.labelKey
        ? t(SYSTEM_SUB_PAGES.find((entry) => entry.id === systemSection)!.labelKey)
        : null;
    }

    if (view === "activity") {
      return visibleActivitySubPages.find((entry) => entry.id === activitySection)?.labelKey
        ? t(visibleActivitySubPages.find((entry) => entry.id === activitySection)!.labelKey)
        : null;
    }

    if (view === "wanted") {
      return visibleWantedSubPages.find((entry) => entry.id === wantedSection)?.labelKey
        ? t(visibleWantedSubPages.find((entry) => entry.id === wantedSection)!.labelKey)
        : null;
    }

    return null;
  }, [
    contentSettingsSection,
    settingsSection,
    systemSection,
    activitySection,
    t,
    view,
    canAccessFacetImport,
    canAccessMediaSettings,
    canManageTitle,
    canRequestMedia,
    visibleMediaSettingsSubPages,
    visibleActivitySubPages,
    visibleSettingsEntries,
    visibleWantedSubPages,
    wantedSection,
  ]);

  return (
    <>
      <Sidebar
        variant="floating"
        collapsible={isMobile ? "offcanvas" : "none"}
        className="overflow-hidden rounded-none border-0 md:-ml-4 md:sticky md:self-start md:top-[calc(var(--root-header-height,0px)+1rem)] md:max-h-[calc(100svh-var(--root-header-height,0px)-2rem)]"
      >
        <SidebarHeader className="px-3 pb-3 pt-3">
          <div className="flex items-center gap-3">
            <ScryerLogo className="h-10! w-10! drop-shadow-[0_10px_18px_rgba(var(--scry-accent-rgb),0.22)]" />
            <span data-slot="brand-wordmark" className="text-[21px] font-bold leading-none text-foreground">
              Scryer
            </span>
          </div>
        </SidebarHeader>
        <SidebarContent className="overflow-y-auto px-2 pb-2">
          {groupedTopNav.map((group) => (
            <SidebarGroup key={group.id} className="px-0 py-1 first:pt-0">
              <SidebarGroupLabel className="h-6 px-2 text-[10px] font-semibold uppercase tracking-[0.16em] text-sidebar-foreground/45">
                {group.label}
              </SidebarGroupLabel>
              <SidebarMenu className="space-y-0.5">
                {group.items.map((item) => {
                  const Icon = item.icon;
                  const isMediaSection = ["movies", "series", "anime"].includes(item.id);
                  const isSettingsTop = item.id === "settings";
                  const isSystemTop = item.id === "system";
                  const isActivityTop = item.id === "activity";
                  const isWantedTop = item.id === "wanted";
                  const isActiveMediaSection = isMediaSection && view === item.id;
                  const isActiveSettingsSection = isSettingsTop && view === "settings";
                  const isActiveSystemSection = isSystemTop && view === "system";
                  const isActiveActivitySection = isActivityTop && view === "activity";
                  const isActiveWantedSection = isWantedTop && view === "wanted";
                  const mediaFacetImportBadgeCount = isMediaSection
                    ? pendingImportCountForNavView(item.id)
                    : 0;
                  const mediaFacetRequestBadgeCount = isMediaSection
                    ? pendingMediaRequestCountForNavView(item.id)
                    : 0;
                  const shouldShowChildren =
                    isActiveMediaSection ||
                    isActiveSettingsSection ||
                    isActiveSystemSection ||
                    (isActiveActivitySection && hasVisibleActivitySubnav) ||
                    isActiveWantedSection;
                  if (!isMediaSection && !isSettingsTop && !isSystemTop && !isActivityTop && !isWantedTop) {
                    return (
                      <React.Fragment key={item.id}>
                        <SidebarMenuItem>
                          <SidebarMenuButton
                            id={selectorId("root-sidebar-nav", item.id)}
                            isActive={view === item.id}
                            className="h-9 rounded-[10px] px-2.5 text-[13px] font-medium"
                            onClick={(event) => {
                              handleNavigate(event, item.id);
                            }}
                          >
                            <Icon className="h-4 w-4" />
                            {item.label}
                          </SidebarMenuButton>
                          {item.id === "activity" && hasActivityImportBadge ? (
                            <SidebarMenuBadge className="bg-primary text-primary-foreground">
                              {activityImportBadgeCount}
                            </SidebarMenuBadge>
                          ) : null}
                          {item.id === "settings" && pluginUpdateCount > 0 ? (
                            <SidebarMenuBadge className="bg-red-600 text-white dark:bg-red-500 dark:text-white">
                              {pluginUpdateCount}
                            </SidebarMenuBadge>
                          ) : null}
                        </SidebarMenuItem>
                      </React.Fragment>
                    );
                  }

                return (
                    <React.Fragment key={item.id}>
                      <SidebarMenuItem>
                        <SidebarMenuButton
                          id={selectorId("root-sidebar-nav", item.id)}
                          isActive={view === item.id}
                          className="h-9 rounded-[10px] px-2.5 text-[13px] font-medium"
                          onClick={(event) => {
                            if (isSettingsTop) {
                              handleNavigate(event, "settings", settingsSection);
                              return;
                            }
                            if (isSystemTop) {
                              handleNavigate(event, "system", undefined, undefined, systemSection);
                              return;
                            }
                            if (isActivityTop) {
                              handleNavigate(
                                event,
                                "activity",
                                undefined,
                                undefined,
                                undefined,
                                undefined,
                                visibleActivitySubPages.some((entry) => entry.id === activitySection)
                                  ? activitySection
                                  : "activity",
                              );
                              return;
                            }
                            if (isWantedTop) {
                              handleNavigate(
                                event,
                                "wanted",
                                undefined,
                                undefined,
                                undefined,
                                canManageTitle || wantedSection !== "history" ? wantedSection : "wanted",
                              );
                              return;
                            }
                            handleNavigate(
                              event,
                              item.id,
                              undefined,
                              canViewCatalog
                                ? "overview"
                                : canAccessMediaSettings
                                  ? "library"
                                  : "requests",
                            );
                          }}
                        >
                          <Icon className="h-4 w-4" />
                          {item.label}
                        </SidebarMenuButton>
                        {item.id === "activity" && hasActivityImportBadge ? (
                          <SidebarMenuBadge className="bg-primary text-primary-foreground">
                            {activityImportBadgeCount}
                          </SidebarMenuBadge>
                        ) : null}
                        {isMediaSection ? (
                          <FacetNavBadges
                            importCount={mediaFacetImportBadgeCount}
                            requestCount={mediaFacetRequestBadgeCount}
                          />
                        ) : null}
                      </SidebarMenuItem>

                    {shouldShowChildren ? (
                      <SidebarGroupContent>
                        <SidebarMenuSub>
                          {isSettingsTop
                            ? visibleSettingsEntries.map((entry) => (
                              <SidebarMenuSubItem key={entry.id}>
                                  <SidebarMenuSubButton
                                    id={selectorId("root-sidebar-settings", entry.id)}
                                    isActive={settingsSection === entry.id}
                                    className="rounded-[9px] text-[12px]"
                                    onClick={(event) => {
                                      handleNavigate(event, "settings", entry.id);
                                    }}
                                  >
                                    {entry.label(t)}
                                    {entry.id === "plugins" && pluginUpdateCount > 0 ? (
                                      <LeafNavBadge count={pluginUpdateCount} tone="danger" />
                                    ) : null}
                                  </SidebarMenuSubButton>
                                </SidebarMenuSubItem>
                              ))
                            : isSystemTop ? (
                              SYSTEM_SUB_PAGES.map((entry) => (
                                <SidebarMenuSubItem key={entry.id}>
                                  <SidebarMenuSubButton
                                    id={selectorId("root-sidebar-system", entry.id)}
                                    isActive={systemSection === entry.id}
                                    className="rounded-[9px] text-[12px]"
                                    onClick={(event) => {
                                      handleNavigate(event, "system", undefined, undefined, entry.id);
                                    }}
                                  >
                                    {t(entry.labelKey)}
                                  </SidebarMenuSubButton>
                                </SidebarMenuSubItem>
                              ))
                            ) : isActivityTop ? (
                              visibleActivitySubPages.map((entry) => (
                                <SidebarMenuSubItem key={entry.id}>
                                  <SidebarMenuSubButton
                                    id={selectorId("root-sidebar-activity", entry.id)}
                                    isActive={activitySection === entry.id}
                                    className="rounded-[9px] text-[12px]"
                                    onClick={(event) => {
                                      handleNavigate(
                                        event,
                                        "activity",
                                        undefined,
                                        undefined,
                                        undefined,
                                        undefined,
                                        entry.id,
                                      );
                                    }}
                                  >
                                    {t(entry.labelKey)}
                                    {entry.id === "import" && hasActivityImportBadge ? (
                                      <LeafNavBadge count={activityImportBadgeCount} />
                                    ) : null}
                                  </SidebarMenuSubButton>
                                </SidebarMenuSubItem>
                              ))
                            ) : isWantedTop ? (
                              visibleWantedSubPages.map((entry) => (
                                <SidebarMenuSubItem key={entry.id}>
                                  <SidebarMenuSubButton
                                    id={selectorId("root-sidebar-wanted", entry.id)}
                                    isActive={wantedSection === entry.id}
                                    className="rounded-[9px] text-[12px]"
                                    onClick={(event) => {
                                      handleNavigate(
                                        event,
                                        "wanted",
                                        undefined,
                                        undefined,
                                        undefined,
                                        entry.id,
                                      );
                                    }}
                                  >
                                    {t(entry.labelKey)}
                                  </SidebarMenuSubButton>
                                </SidebarMenuSubItem>
                              ))
                            ) : (
                              <>
                                {canViewCatalog ? (
                                  <SidebarMenuSubItem>
                                    <SidebarMenuSubButton
                                      id={selectorId("root-sidebar-media", item.id, "overview")}
                                      isActive={contentSettingsSection === "overview"}
                                      className="rounded-[9px] text-[12px]"
                                      onClick={(event) => {
                                        handleNavigate(event, item.id, undefined, "overview");
                                      }}
                                    >
                                      {getMediaOverviewLabel(item.id, t)}
                                    </SidebarMenuSubButton>
                                  </SidebarMenuSubItem>
                                ) : null}
                                {canAccessFacetImport && hasImportsForView(item.id) ? (
                                  <SidebarMenuSubItem>
                                    <SidebarMenuSubButton
                                      id={selectorId("root-sidebar-media", item.id, "import")}
                                      isActive={contentSettingsSection === "import"}
                                      className="rounded-[9px] text-[12px]"
                                      onClick={(event) => {
                                        handleNavigate(event, item.id, undefined, "import");
                                      }}
                                    >
                                      {t("nav.import")}
                                      {pendingImportCountForNavView(item.id) > 0 ? (
                                        <LeafNavBadge count={pendingImportCountForNavView(item.id)} />
                                      ) : null}
                                    </SidebarMenuSubButton>
                                  </SidebarMenuSubItem>
                                ) : null}
                                {canManageTitle || canRequestMedia ? (
                                  <SidebarMenuSubItem>
                                    <SidebarMenuSubButton
                                      id={selectorId("root-sidebar-media", item.id, "requests")}
                                      isActive={contentSettingsSection === "requests"}
                                      className="rounded-[9px] text-[12px]"
                                      onClick={(event) => {
                                        handleNavigate(event, item.id, undefined, "requests");
                                      }}
                                    >
                                      {t("nav.requests")}
                                      {pendingMediaRequestCountForNavView(item.id) > 0 ? (
                                        <LeafNavBadge
                                          count={pendingMediaRequestCountForNavView(item.id)}
                                          tone="request"
                                        />
                                      ) : null}
                                    </SidebarMenuSubButton>
                                  </SidebarMenuSubItem>
                                ) : null}
                                {canAccessMediaSettings ? (
                                  <SidebarMenuSubItem>
                                    <Collapsible open={isSettingsSubPage(contentSettingsSection)}>
                                      <SidebarMenuSubButton
                                        id={selectorId("root-sidebar-media", item.id, "settings")}
                                        isActive={isSettingsSubPage(contentSettingsSection)}
                                        className="rounded-[9px] text-[12px]"
                                        onClick={(event) => {
                                          handleNavigate(event, item.id, undefined, "library");
                                        }}
                                      >
                                        {getMediaSettingsLabel(item.id, t)}
                                        <ChevronRight className={`ml-auto h-3 w-3 transition-transform ${isSettingsSubPage(contentSettingsSection) ? "rotate-90" : ""}`} />
                                      </SidebarMenuSubButton>
                                      <CollapsibleContent>
                                        <SidebarMenuSub>
                                          {visibleMediaSettingsSubPages.map((subPage) => (
                                            <SidebarMenuSubItem key={subPage.id}>
                                              <SidebarMenuSubButton
                                                id={selectorId("root-sidebar-media", item.id, subPage.id)}
                                                isActive={contentSettingsSection === subPage.id}
                                                className="rounded-[9px] text-[12px]"
                                                onClick={(event) => {
                                                  handleNavigate(event, item.id, undefined, subPage.id);
                                                }}
                                              >
                                                {t(subPage.labelKey)}
                                              </SidebarMenuSubButton>
                                            </SidebarMenuSubItem>
                                          ))}
                                        </SidebarMenuSub>
                                      </CollapsibleContent>
                                    </Collapsible>
                                  </SidebarMenuSubItem>
                                ) : null}
                              </>
                            )}
                        </SidebarMenuSub>
                      </SidebarGroupContent>
                    ) : null}
                  </React.Fragment>
                );
              })}
            </SidebarMenu>
          </SidebarGroup>
          ))}
        </SidebarContent>
        <SidebarFooter className="border-t border-border/70 px-3 py-2">
          <div className="flex items-center justify-between gap-2">
            {themeMounted ? (
              <button
                id="root-sidebar-theme-toggle"
                type="button"
                onClick={cycleTheme}
                aria-label={`Switch theme (current: ${getThemeLabel(theme)})`}
                className={cn(
                  "flex w-fit items-center gap-2 rounded-md px-2 py-1.5 text-sm text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                  theme === "pride" && "text-pink-200 hover:text-pink-100",
                )}
              >
                {theme === "light" ? (
                  <Sun className="h-4 w-4" />
                ) : theme === "dark" ? (
                  <Moon className="h-4 w-4" />
                ) : theme === "pride" ? (
                  <Rainbow className="h-4 w-4" />
                ) : (
                  <Monitor className="h-4 w-4" />
                )}
                Theme
              </button>
            ) : null}
            {scryerVersion ? (
              <span className="shrink-0 text-[11px] text-sidebar-foreground/45 group-data-[collapsible=icon]:hidden">
                v{scryerVersion}
              </span>
            ) : null}
          </div>
        </SidebarFooter>
      </Sidebar>
      <SidebarInset className="relative bg-background md:ml-4">
        <div className="mb-3 flex items-center gap-3 rounded-xl border border-border bg-card/80 px-3 py-2 md:hidden">
          <SidebarTrigger
            id="root-sidebar-mobile-trigger"
            className="size-9 rounded-lg border border-border bg-background text-foreground shadow-none"
          />
          <div className="min-w-0">
            <p className="truncate text-sm font-semibold text-foreground">{currentTopLevelLabel}</p>
            {currentSubsectionLabel && currentSubsectionLabel !== currentTopLevelLabel ? (
              <p className="truncate text-xs text-muted-foreground">{currentSubsectionLabel}</p>
            ) : null}
          </div>
        </div>
        {children}
      </SidebarInset>
    </>
  );
}

export const RootSidebar = React.memo(function RootSidebar(props: RootSidebarProps) {
  return (
    <SidebarProvider
      className="h-full"
      style={
        {
          "--sidebar-width": "clamp(14rem, 18vw, 16rem)",
        } as React.CSSProperties
      }
    >
      <RootSidebarContent {...props} />
    </SidebarProvider>
  );
});
