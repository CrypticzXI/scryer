
import * as React from "react";
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
import {
  Archive,
  Bell,
  ChevronRight,
  Database,
  Download,
  FileText,
  Inbox,
  Monitor,
  Moon,
  Rainbow,
  Server,
  Shield,
  SlidersHorizontal,
  Sun,
  Users,
} from "lucide-react";
import { useTheme } from "next-themes";
import { getNextTheme } from "@/lib/theme";
import { cn } from "@/lib/utils";
import type { PendingImportCounts } from "@/lib/types";
import { pendingImportCountForView } from "@/lib/types";
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
  items: TopNavGroupItemDefinition[];
};

type TopNavGroup = {
  id: string;
  label: string;
  items: TopNavGroupItem[];
};

type TopNavGroupItemDefinition =
  | { kind: "view"; id: ViewId }
  | { kind: "requests"; icon: LucideIcon }
  | { kind: "system"; id: SystemSection; labelKey: string; icon: LucideIcon }
  | { kind: "settings"; id: SettingsSection; labelKey?: string; icon: LucideIcon };

type TopNavGroupItem =
  | (NavItem & { kind: "view" })
  | {
      kind: "requests";
      id: "requests";
      label: string;
      icon: LucideIcon;
    }
  | {
      kind: "system";
      id: SystemSection;
      label: string;
      icon: LucideIcon;
    }
  | {
      kind: "settings";
      id: SettingsSection;
      label: string;
      icon: LucideIcon;
    };

type HeaderWithMobileNavigationProps = {
  mobileNavigation?: React.ReactNode;
};

const TOP_NAV_GROUPS: TopNavGroupDefinition[] = [
  {
    id: "catalogs",
    labelKey: "nav.group.catalogs",
    items: [
      { kind: "view", id: "movies" },
      { kind: "view", id: "series" },
      { kind: "view", id: "anime" },
    ],
  },
  {
    id: "discover",
    labelKey: "nav.group.discover",
    items: [{ kind: "requests", icon: Inbox }],
  },
  {
    id: "automation",
    labelKey: "nav.group.automation",
    items: [
      { kind: "view", id: "wanted" },
      { kind: "view", id: "calendar" },
      { kind: "view", id: "activity" },
      { kind: "settings", id: "rules", icon: SlidersHorizontal },
    ],
  },
  {
    id: "integrations",
    labelKey: "nav.group.integrations",
    items: [
      { kind: "settings", id: "indexers", icon: Database },
      { kind: "settings", id: "downloadClients", icon: Download },
      { kind: "settings", id: "mediaServers", icon: Server },
      { kind: "settings", id: "notifications", icon: Bell },
    ],
  },
  {
    id: "system",
    labelKey: "nav.group.system",
    items: [
      { kind: "settings", id: "users", labelKey: "nav.usersAccess", icon: Users },
      { kind: "settings", id: "security", icon: Shield },
      { kind: "settings", id: "backups", icon: Archive },
      { kind: "system", id: "audit", labelKey: "nav.logs", icon: FileText },
      { kind: "view", id: "settings" },
    ],
  },
];

const PROMOTED_SETTINGS_SHORTCUT_IDS = new Set<SettingsSection>(
  TOP_NAV_GROUPS.flatMap((group) =>
    group.items.flatMap((item) => (item.kind === "settings" ? [item.id] : [])),
  ),
);
const DEFAULT_SETTINGS_SECTION_ORDER: SettingsSection[] = [
  "general",
  "qualityProfiles",
  "security",
  "users",
  "profile",
];
const MEDIA_NAV_VIEW_IDS: ViewId[] = ["movies", "series", "anime"];

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
  header?: React.ReactNode;
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
    id: "acquisition",
    label: (t) => t("settings.acquisition"),
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

const SETTINGS_NAV_GROUPS: Array<{
  id: string;
  labelKey: string;
  itemIds: SettingsSection[];
}> = [
  {
    id: "account",
    labelKey: "settings.navGroup.account",
    itemIds: ["profile", "general"],
  },
  {
    id: "catalogs",
    labelKey: "settings.navGroup.catalogs",
    itemIds: [
      "qualityProfiles",
      "delayProfiles",
      "rules",
      "acquisition",
      "post-processing",
      "subtitles",
      "recycleBin",
    ],
  },
  {
    id: "integrations",
    labelKey: "settings.navGroup.integrations",
    itemIds: [
      "mediaServers",
      "downloadClients",
      "indexers",
      "notifications",
      "plugins",
    ],
  },
  {
    id: "system",
    labelKey: "settings.navGroup.system",
    itemIds: ["users", "security", "backups"],
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
  { id: "audit", labelKey: "nav.logs" },
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

const TOP_NAV_BUTTON_CLASS =
  "h-9 rounded-[10px] px-2.5 text-[13px] font-medium transition-colors data-[active=true]:bg-[linear-gradient(90deg,rgba(var(--scry-accent-rgb),0.30),rgba(var(--scry-accent-rgb),0.10))] data-[active=true]:font-semibold data-[active=true]:text-foreground data-[active=true]:shadow-[inset_2px_0_0_rgb(var(--scry-accent-rgb)),0_8px_18px_rgba(var(--scry-accent-rgb),0.16)] data-[active=true]:[&>svg]:text-primary";

const SUB_NAV_BUTTON_CLASS =
  "rounded-[9px] text-[12px] transition-colors data-[active=true]:!bg-[rgba(var(--scry-accent-rgb),0.14)] data-[active=true]:!bg-none data-[active=true]:text-foreground data-[active=true]:shadow-[inset_2px_0_0_rgb(var(--scry-accent-rgb))]";

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
      return "bg-emerald-600 text-white dark:bg-emerald-500 dark:text-emerald-950";
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

function getThemeLabel(theme: string | undefined, t: Translate): string {
  switch (theme) {
    case "light":
      return t("theme.light");
    case "dark":
      return t("theme.dark");
    case "pride":
      return t("theme.pride");
    default:
      return t("theme.system");
  }
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
  header,
  children,
  onNavigate,
}: RootSidebarProps) {
  const t = useTranslate();
  const { isMobile, setOpenMobile } = useSidebar();
  const { theme, setTheme } = useTheme();
  const [themeMounted, setThemeMounted] = React.useState(false);
  React.useEffect(() => setThemeMounted(true), []);
  const cycleTheme = React.useCallback(() => {
    setTheme(getNextTheme(theme));
  }, [theme, setTheme]);
  const themeLabel = getThemeLabel(theme, t);
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
  const groupedSettingsEntries = React.useMemo(() => {
    const entriesById = new Map(visibleSettingsEntries.map((entry) => [entry.id, entry]));
    return SETTINGS_NAV_GROUPS.map((group) => ({
      ...group,
      entries: group.itemIds.flatMap((id) => {
        const entry = entriesById.get(id);
        return entry ? [entry] : [];
      }),
    })).filter((group) => group.entries.length > 0);
  }, [visibleSettingsEntries]);
  const canAccessMediaTopNav = canViewCatalog || canResolveImports || canManageLibrarySettings;
  const defaultMediaContentSection: ContentSettingsSection = canViewCatalog
    ? "overview"
    : canResolveImports
      ? "import"
      : "library";
  const visibleTopNav = React.useMemo(
    () =>
      topNav.filter(
        (item) =>
          (!MEDIA_NAV_VIEW_IDS.includes(item.id) || canAccessMediaTopNav) &&
          (item.id !== "calendar" || canViewCatalog) &&
          (item.id !== "wanted" || canViewCatalog) &&
          (item.id !== "system" || canManageSystemSettings) &&
          (item.id !== "activity" || canResolveImports || canManageTitle),
      ),
    [
      canAccessMediaTopNav,
      canManageSystemSettings,
      canManageTitle,
      canResolveImports,
      canViewCatalog,
      topNav,
    ],
  );
  const groupedTopNav = React.useMemo<TopNavGroup[]>(() => {
    const itemsById = new Map(visibleTopNav.map((item) => [item.id, item]));
    const settingsEntriesById = new Map(visibleSettingsEntries.map((entry) => [entry.id, entry]));
    const groupedIds = new Set<ViewId>();
    const groups = TOP_NAV_GROUPS.map((group) => {
      const items = group.items.flatMap<TopNavGroupItem>((definition) => {
        if (definition.kind === "settings") {
          const entry = settingsEntriesById.get(definition.id);
          if (!entry) {
            return [];
          }
          return [{
            kind: "settings",
            id: definition.id,
            label: definition.labelKey ? t(definition.labelKey) : entry.label(t),
            icon: definition.icon,
          }];
        }
        if (definition.kind === "requests") {
          if (!canManageTitle && !canRequestMedia) {
            return [];
          }
          return [{
            kind: "requests",
            id: "requests",
            label: t("nav.requests"),
            icon: definition.icon,
          }];
        }
        if (definition.kind === "system") {
          if (!canManageSystemSettings) {
            return [];
          }
          return [{
            kind: "system",
            id: definition.id,
            label: t(definition.labelKey),
            icon: definition.icon,
          }];
        }

        const item = itemsById.get(definition.id);
        if (!item) {
          return [];
        }
        groupedIds.add(definition.id);
        return [{ kind: "view", ...item }];
      });
      return { id: group.id, label: t(group.labelKey), items };
    }).filter((group) => group.items.length > 0);
    const ungroupedItems = visibleTopNav
      .filter((item) => !groupedIds.has(item.id))
      .map<TopNavGroupItem>((item) => ({ kind: "view", ...item }));

    return ungroupedItems.length > 0
      ? [...groups, { id: "more", label: t("nav.group.more"), items: ungroupedItems }]
      : groups;
  }, [canManageSystemSettings, canManageTitle, canRequestMedia, t, visibleSettingsEntries, visibleTopNav]);
  const defaultSettingsSectionForTopNav = React.useMemo<SettingsSection>(() => {
    const visibleIds = new Set(visibleSettingsEntries.map((entry) => entry.id));
    return DEFAULT_SETTINGS_SECTION_ORDER.find((section) => visibleIds.has(section)) ??
      visibleSettingsEntries[0]?.id ??
      "profile";
  }, [visibleSettingsEntries]);

  const pendingImportCountForNavView = React.useCallback(
    (viewId: ViewId) => pendingImportCountForView(pendingImportCounts, viewId),
    [pendingImportCounts],
  );
  const pendingMediaRequestCountForNavView = React.useCallback(
    (viewId: ViewId) => pendingImportCountForView(pendingMediaRequestCounts, viewId),
    [pendingMediaRequestCounts],
  );
  const pendingMediaRequestCount = MEDIA_NAV_VIEW_IDS.reduce(
    (total, viewId) => total + pendingMediaRequestCountForNavView(viewId),
    0,
  );
  const isRequestsSection =
    MEDIA_NAV_VIEW_IDS.includes(view) && contentSettingsSection === "requests";
  const requestNavTargetView = MEDIA_NAV_VIEW_IDS.includes(view)
    ? view
    : (visibleTopNav.find((item) => MEDIA_NAV_VIEW_IDS.includes(item.id))?.id ?? "movies");
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
    () => {
      if (isRequestsSection) {
        return t("nav.requests");
      }

      return visibleTopNav.find((item) => item.id === view)?.label ??
        topNav.find((item) => item.id === view)?.label ??
        t("nav.library");
    },
    [isRequestsSection, topNav, t, view, visibleTopNav],
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
        return null;
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
    visibleMediaSettingsSubPages,
    visibleActivitySubPages,
    visibleSettingsEntries,
    visibleWantedSubPages,
    wantedSection,
  ]);

  const mobileNavigationTrigger = (
    <SidebarTrigger
      id="root-sidebar-mobile-trigger"
      aria-label={t("nav.mobileTrigger")}
      className="size-10 rounded-xl border border-border bg-background/80 text-foreground shadow-none min-[981px]:hidden"
    />
  );
  const canInjectMobileNavigation =
    React.isValidElement<HeaderWithMobileNavigationProps>(header) &&
    typeof header.type !== "string";
  const headerWithMobileNavigation = canInjectMobileNavigation
    ? React.cloneElement(header, { mobileNavigation: mobileNavigationTrigger })
    : header;

  return (
    <>
      <Sidebar
        variant="sidebar"
        collapsible={isMobile ? "offcanvas" : "none"}
        mobileTitle={t("nav.mobileTitle")}
        mobileDescription={t("nav.mobileDescription")}
        className="overflow-hidden border-r border-sidebar-border/80 bg-sidebar/95 shadow-[12px_0_40px_rgba(2,6,23,0.22)] backdrop-blur min-[981px]:sticky min-[981px]:top-0 min-[981px]:h-[calc(100dvh-var(--root-shell-top-offset,0px))] min-[981px]:max-h-[calc(100dvh-var(--root-shell-top-offset,0px))] min-[981px]:self-start"
      >
        <SidebarHeader className="px-5 pb-3 pt-5">
          <div className="flex items-center gap-3">
            <ScryerLogo className="h-[38px]! w-[38px]! drop-shadow-[0_10px_18px_rgba(var(--scry-accent-rgb),0.28)]" />
            <span
              data-slot="brand-wordmark"
              className="text-[21px] font-bold leading-none text-foreground"
              style={{
                fontFamily:
                  "var(--font-space-grotesk), var(--font-inter), ui-sans-serif, system-ui, -apple-system, sans-serif",
              }}
            >
              Scryer
            </span>
          </div>
        </SidebarHeader>
        <SidebarContent className="overflow-y-auto px-3 pb-3">
          {groupedTopNav.map((group) => (
            <SidebarGroup key={group.id} className="px-0 py-1 first:pt-0">
              <SidebarGroupLabel className="h-6 px-2 text-[10px] font-semibold uppercase tracking-[0.16em] text-sidebar-foreground/45">
                {group.label}
              </SidebarGroupLabel>
              <SidebarMenu className="space-y-0.5">
                {group.items.map((item) => {
                  const Icon = item.icon;
                  if (item.kind === "requests") {
                    return (
                      <React.Fragment key="requests">
                        <SidebarMenuItem>
                          <SidebarMenuButton
                            id={selectorId("root-sidebar-nav", "requests")}
                            isActive={isRequestsSection}
                            className={TOP_NAV_BUTTON_CLASS}
                            onClick={(event) => {
                              handleNavigate(event, requestNavTargetView, undefined, "requests");
                            }}
                          >
                            <Icon className="h-4 w-4" />
                            {item.label}
                          </SidebarMenuButton>
                          {pendingMediaRequestCount > 0 ? (
                            <SidebarMenuBadge className={navBadgeToneClass("request")}>
                              {pendingMediaRequestCount}
                            </SidebarMenuBadge>
                          ) : null}
                        </SidebarMenuItem>
                      </React.Fragment>
                    );
                  }
                  if (item.kind === "settings") {
                    return (
                      <React.Fragment key={`settings-${item.id}`}>
                        <SidebarMenuItem>
                          <SidebarMenuButton
                            id={selectorId("root-sidebar-settings-shortcut", item.id)}
                            isActive={view === "settings" && settingsSection === item.id}
                            className={TOP_NAV_BUTTON_CLASS}
                            onClick={(event) => {
                              handleNavigate(event, "settings", item.id);
                            }}
                          >
                            <Icon className="h-4 w-4" />
                            {item.label}
                          </SidebarMenuButton>
                        </SidebarMenuItem>
                      </React.Fragment>
                    );
                  }
                  if (item.kind === "system") {
                    return (
                      <React.Fragment key={`system-${item.id}`}>
                        <SidebarMenuItem>
                          <SidebarMenuButton
                            id={selectorId("root-sidebar-system-shortcut", item.id)}
                            isActive={view === "system" && systemSection === item.id}
                            className={TOP_NAV_BUTTON_CLASS}
                            onClick={(event) => {
                              handleNavigate(event, "system", undefined, undefined, item.id);
                            }}
                          >
                            <Icon className="h-4 w-4" />
                            {item.label}
                          </SidebarMenuButton>
                        </SidebarMenuItem>
                      </React.Fragment>
                    );
                  }

                  const isPromotedSettingsSection =
                    view === "settings" && PROMOTED_SETTINGS_SHORTCUT_IDS.has(settingsSection);
                  const isMediaSection = ["movies", "series", "anime"].includes(item.id);
                  const isSettingsTop = item.id === "settings";
                  const isSystemTop = item.id === "system";
                  const isActivityTop = item.id === "activity";
                  const isWantedTop = item.id === "wanted";
                  const isActiveMediaSection = isMediaSection && view === item.id && !isRequestsSection;
                  const isActiveSettingsSection =
                    isSettingsTop && view === "settings" && !isPromotedSettingsSection;
                  const isActiveSystemSection =
                    isSystemTop && view === "system" && systemSection !== "audit";
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
                            className={TOP_NAV_BUTTON_CLASS}
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
                          isActive={
                            isSettingsTop
                              ? isActiveSettingsSection
                              : isSystemTop
                                ? isActiveSystemSection
                                : isMediaSection
                                  ? isActiveMediaSection
                                  : view === item.id
                          }
                          className={TOP_NAV_BUTTON_CLASS}
                          onClick={(event) => {
                            if (isSettingsTop) {
                              handleNavigate(
                                event,
                                "settings",
                                isPromotedSettingsSection ? defaultSettingsSectionForTopNav : settingsSection,
                              );
                              return;
                            }
                            if (isSystemTop) {
                              handleNavigate(event, "system", undefined, undefined, "overview");
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
                              defaultMediaContentSection,
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
                            ? groupedSettingsEntries.map((group) => (
                              <React.Fragment key={group.id}>
                                <SidebarMenuSubItem>
                                  <div className="px-2 pb-1 pt-2 text-[10px] font-semibold uppercase tracking-[0.13em] text-sidebar-foreground/40">
                                    {t(group.labelKey)}
                                  </div>
                                </SidebarMenuSubItem>
                                {group.entries.map((entry) => (
                                  <SidebarMenuSubItem key={entry.id}>
                                    <SidebarMenuSubButton
                                      id={selectorId("root-sidebar-settings", entry.id)}
                                      isActive={settingsSection === entry.id}
                                      className={SUB_NAV_BUTTON_CLASS}
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
                                ))}
                              </React.Fragment>
                            ))
                            : isSystemTop ? (
                              SYSTEM_SUB_PAGES.map((entry) => (
                                <SidebarMenuSubItem key={entry.id}>
                                  <SidebarMenuSubButton
                                    id={selectorId("root-sidebar-system", entry.id)}
                                    isActive={systemSection === entry.id}
                                    className={SUB_NAV_BUTTON_CLASS}
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
                                    className={SUB_NAV_BUTTON_CLASS}
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
                                    className={SUB_NAV_BUTTON_CLASS}
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
                                      className={SUB_NAV_BUTTON_CLASS}
                                      onClick={(event) => {
                                        handleNavigate(event, item.id, undefined, "overview");
                                      }}
                                    >
                                      {getMediaOverviewLabel(item.id, t)}
                                    </SidebarMenuSubButton>
                                  </SidebarMenuSubItem>
                                ) : null}
                                {canAccessFacetImport ? (
                                  <SidebarMenuSubItem>
                                    <SidebarMenuSubButton
                                      id={selectorId("root-sidebar-media", item.id, "import")}
                                      isActive={contentSettingsSection === "import"}
                                      className={SUB_NAV_BUTTON_CLASS}
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
                                {canAccessMediaSettings ? (
                                  <SidebarMenuSubItem>
                                    <Collapsible open={isSettingsSubPage(contentSettingsSection)}>
                                      <SidebarMenuSubButton
                                        id={selectorId("root-sidebar-media", item.id, "settings")}
                                        isActive={isSettingsSubPage(contentSettingsSection)}
                                        className={SUB_NAV_BUTTON_CLASS}
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
                                                className={SUB_NAV_BUTTON_CLASS}
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
        <SidebarFooter className="border-t border-sidebar-border/80 px-4 py-3">
          <div className="flex items-center justify-between gap-3">
            {scryerVersion ? (
              <span className="shrink-0 text-[11px] text-sidebar-foreground/45 group-data-[collapsible=icon]:hidden">
                Scryer v{scryerVersion}
              </span>
            ) : (
              <span className="shrink-0 text-[11px] text-sidebar-foreground/45 group-data-[collapsible=icon]:hidden">
                Scryer
              </span>
            )}
            {themeMounted ? (
              <button
                id="root-sidebar-theme-toggle"
                type="button"
                onClick={cycleTheme}
                aria-label={t("theme.switchLabel", { theme: themeLabel })}
                className={cn(
                  "flex min-w-0 shrink-0 items-center gap-1.5 rounded-lg border border-sidebar-border/70 bg-sidebar-accent/55 px-2 py-1.5 text-xs font-medium text-sidebar-foreground/70 shadow-[inset_0_1px_0_rgba(255,255,255,0.06)] transition hover:border-primary/35 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
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
                <span className="truncate">{themeLabel}</span>
              </button>
            ) : null}
          </div>
        </SidebarFooter>
      </Sidebar>
      <SidebarInset className="relative min-w-0 bg-transparent">
        {headerWithMobileNavigation}
        {canInjectMobileNavigation ? null : (
          <div className="mx-3 mb-3 mt-3 flex items-center gap-3 rounded-xl border border-border bg-card/80 px-3 py-2 min-[981px]:hidden">
            {mobileNavigationTrigger}
            <div className="min-w-0">
              <p className="truncate text-sm font-semibold text-foreground">{currentTopLevelLabel}</p>
              {currentSubsectionLabel && currentSubsectionLabel !== currentTopLevelLabel ? (
                <p className="truncate text-xs text-muted-foreground">{currentSubsectionLabel}</p>
              ) : null}
            </div>
          </div>
        )}
        {children}
      </SidebarInset>
    </>
  );
}

export const RootSidebar = React.memo(function RootSidebar(props: RootSidebarProps) {
  return (
    <SidebarProvider
      className="h-full bg-[radial-gradient(circle_at_14%_10%,rgba(var(--scry-accent-rgb),0.10),transparent_26rem),radial-gradient(circle_at_86%_14%,rgba(56,189,248,0.06),transparent_28rem),radial-gradient(circle_at_60%_92%,rgba(16,185,129,0.05),transparent_34rem),linear-gradient(180deg,var(--background)_0%,color-mix(in_srgb,var(--muted)_38%,transparent)_46%,var(--background)_100%)] bg-fixed"
      style={
        {
          "--sidebar-width": "236px",
        } as React.CSSProperties
      }
    >
      <RootSidebarContent {...props} />
    </SidebarProvider>
  );
});
