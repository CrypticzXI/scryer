import type { LucideIcon } from "lucide-react";
import {
  ActivitySquare,
  Archive,
  Bell,
  CalendarDays,
  Captions,
  Database,
  Download,
  FileText,
  FolderCog,
  Inbox,
  ListChecks,
  Puzzle,
  Search,
  Server,
  Settings,
  Shield,
  SlidersHorizontal,
  Trash2,
  User,
  Users,
} from "lucide-react";
import type {
  ActivitySection,
  ContentSettingsSection,
  SettingsSection,
  SystemSection,
  Translate,
  ViewId,
  WantedSection,
} from "@/components/root/types";
import { FACET_REGISTRY } from "@/lib/facets/registry";
import type { Facet } from "@/lib/types";
import type { AuthUser } from "@/lib/hooks/use-auth";
import { APP_PERMISSIONS, LIBRARY_PERMISSIONS, hasAnyAppPermission, hasAnyLibraryPermission } from "@/lib/utils/permissions";

export type RouteCommand = {
  id: string;
  label: string;
  description: string;
  groupLabel: string;
  keywords: string[];
  icon: LucideIcon;
  onSelect: () => void;
};

type BuildRouteCommandsArgs = {
  t: Translate;
  user: AuthUser;
  activeFacet: Facet;
  activityImportCount?: number;
  onNavigate: (
    nextView: ViewId,
    nextSettingsSection?: SettingsSection,
    nextContentSection?: ContentSettingsSection,
    nextSystemSection?: SystemSection,
    nextWantedSection?: WantedSection,
    nextActivitySection?: ActivitySection,
  ) => void;
};

function buildNavigate(
  onNavigate: BuildRouteCommandsArgs["onNavigate"],
  view: ViewId,
  settingsSection?: SettingsSection,
  contentSection?: ContentSettingsSection,
  systemSection?: SystemSection,
  wantedSection?: WantedSection,
  activitySection?: ActivitySection,
): () => void {
  return () => {
    onNavigate(view, settingsSection, contentSection, systemSection, wantedSection, activitySection);
  };
}

export function buildRouteCommands({
  t,
  user,
  activeFacet,
  activityImportCount = 0,
  onNavigate,
}: BuildRouteCommandsArgs): RouteCommand[] {
  const canViewCatalog = hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.view);
  const canManageTitle = hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.manageTitles);
  const canRequestMedia = hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.request);
  const canResolveImports = hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.resolveImports);
  const canAccessActivity = canResolveImports || canManageTitle;
  const canManageUserAccounts = hasAnyAppPermission(user, [APP_PERMISSIONS.manageUsers]);
  const canManageUserAccess = hasAnyAppPermission(user, [
    APP_PERMISSIONS.manageUsers,
    APP_PERMISSIONS.managePermissions,
  ]);
  const canManageSystemSettings = hasAnyAppPermission(user, [APP_PERMISSIONS.manageSystemSettings]);
  const canManageCatalogSettings = hasAnyAppPermission(user, [APP_PERMISSIONS.manageCatalogSettings]);
  const canManageConfig = canManageSystemSettings || canManageCatalogSettings;
  const canManageLibrarySettings =
    canManageConfig || hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.manageLibrary);
  const automationGroupLabel = t("nav.group.automation");
  const catalogsGroupLabel = t("nav.group.catalogs");
  const integrationsGroupLabel = t("nav.group.integrations");
  const requestsGroupLabel = t("nav.group.requests");
  const settingsGroupLabel = t("nav.settings");
  const systemGroupLabel = t("nav.group.system");
  const requestFacet = FACET_REGISTRY.find((f) => f.id === activeFacet) ?? FACET_REGISTRY[0];
  const requestView = requestFacet.viewId as ViewId;
  const mediaOverviewCommands = canViewCatalog ? FACET_REGISTRY.map((f) => ({
    id: `${f.viewId}-overview`,
    label: t(f.overviewLabelKey),
    description: t(f.navLabelKey),
    groupLabel: catalogsGroupLabel,
    keywords: [f.viewId, f.id, "manage", "catalog", "overview", "library"],
    icon: f.icon,
    onSelect: buildNavigate(onNavigate, f.viewId as ViewId, undefined, "overview"),
  }) satisfies RouteCommand) : [];
  const mediaDetailCommands = canResolveImports || canManageConfig || canManageLibrarySettings
    ? FACET_REGISTRY.flatMap((f) => {
        const commands: RouteCommand[] = [];

        if (canResolveImports) {
          commands.push({
            id: `${f.viewId}-import`,
            label: `${t(f.navLabelKey)} / ${t("nav.import")}`,
            description: t("nav.import"),
            groupLabel: catalogsGroupLabel,
            keywords: [f.viewId, f.id, "import", "pending", "unmatched", "match"],
            icon: f.icon,
            onSelect: buildNavigate(onNavigate, f.viewId as ViewId, undefined, "import"),
          });
        }

        if (canManageLibrarySettings) {
          commands.push({
            id: `${f.viewId}-settings`,
            label: t(f.settingsLabelKey),
            description: t(f.settingsLabelKey),
            groupLabel: catalogsGroupLabel,
            keywords: [f.viewId, f.id, "settings", "media", "paths", "folder"],
            icon: Settings,
            onSelect: buildNavigate(onNavigate, f.viewId as ViewId, undefined, "library"),
          });

          const facetSubSections: Array<{
            section: ContentSettingsSection;
            labelKey: string;
            extraKeywords: string[];
          }> = [
            {
              section: "library",
              labelKey: "nav.library",
              extraKeywords: ["library", "roots", "folders", "scan", "overrides"],
            },
            {
              section: "general",
              labelKey: "facetSettings.general",
              extraKeywords: ["general", "sidecar", "nfo", "plexmatch"],
            },
            {
              section: "quality",
              labelKey: "facetSettings.quality",
              extraKeywords: ["quality", "profiles"],
            },
            {
              section: "renaming",
              labelKey: "facetSettings.renaming",
              extraKeywords: ["renaming", "naming", "format"],
            },
            {
              section: "routing",
              labelKey: "facetSettings.routing",
              extraKeywords: ["routing", "paths", "folders", "root"],
            },
          ];
          for (const sub of canManageConfig
            ? facetSubSections
            : facetSubSections.filter((entry) => entry.section === "library")) {
            commands.push({
              id: `${f.viewId}-settings-${sub.section}`,
              label: `${t(f.settingsLabelKey)} / ${t(sub.labelKey)}`,
              description: t(sub.labelKey),
              groupLabel: catalogsGroupLabel,
              keywords: [f.viewId, f.id, "settings", ...sub.extraKeywords],
              icon: Settings,
              onSelect: buildNavigate(onNavigate, f.viewId as ViewId, undefined, sub.section),
            });
          }
        }

        return commands;
      })
    : [];

  return [
    ...mediaOverviewCommands,
    ...(canManageTitle || canRequestMedia
      ? [{
          id: "requests",
          label: t("nav.requests"),
          description: FACET_REGISTRY.map((f) => t(f.navLabelKey)).join(" / "),
          groupLabel: requestsGroupLabel,
          keywords: [
            "requests",
            "request queue",
            "media requests",
            "movies",
            "series",
            "anime",
            "library",
          ],
          icon: Inbox,
          onSelect: buildNavigate(onNavigate, requestView, undefined, "requests"),
        } satisfies RouteCommand]
      : []),
    ...(canViewCatalog
      ? [{
          id: "wanted-items",
          label: `${t("nav.wanted")} / ${t("wanted.tabWanted")}`,
          description: t("wanted.tabWanted"),
          groupLabel: automationGroupLabel,
          keywords: ["wanted", "missing", "wanted items", "acquisition", "search"],
          icon: ListChecks,
          onSelect: buildNavigate(onNavigate, "wanted", undefined, undefined, undefined, "wanted"),
        } satisfies RouteCommand, {
          id: "calendar",
          label: t("nav.calendar"),
          description: t("nav.calendar"),
          groupLabel: automationGroupLabel,
          keywords: ["calendar", "episodes", "airing", "schedule", "upcoming"],
          icon: CalendarDays,
          onSelect: buildNavigate(onNavigate, "calendar"),
        } satisfies RouteCommand]
      : []),
    ...(canAccessActivity
      ? [{
          id: "activity-overview",
          label: `${t("nav.activity")} / ${t("activity.activity")}`,
          description: t("activity.activity"),
          groupLabel: automationGroupLabel,
          keywords: ["activity", "events", "log", "audit", "system", "queue"],
          icon: ActivitySquare,
          onSelect: buildNavigate(onNavigate, "activity", undefined, undefined, undefined, undefined, "activity"),
        } satisfies RouteCommand]
      : []),
    ...mediaDetailCommands,
    ...(canViewCatalog
      ? [{
          id: "wanted-cutoff",
          label: `${t("nav.wanted")} / ${t("wanted.tabCutoff")}`,
          description: t("wanted.tabCutoff"),
          groupLabel: automationGroupLabel,
          keywords: ["wanted", "cutoff", "upgrade", "quality", "unmet"],
          icon: ListChecks,
          onSelect: buildNavigate(onNavigate, "wanted", undefined, undefined, undefined, "cutoff"),
        } satisfies RouteCommand, {
          id: "wanted-pending",
          label: `${t("nav.wanted")} / ${t("wanted.tabPending")}`,
          description: t("wanted.tabPending"),
          groupLabel: automationGroupLabel,
          keywords: ["wanted", "pending", "delayed", "releases"],
          icon: ListChecks,
          onSelect: buildNavigate(onNavigate, "wanted", undefined, undefined, undefined, "pending"),
        } satisfies RouteCommand]
      : []),
    ...(canViewCatalog && canManageTitle
      ? [{
          id: "wanted-history",
          label: `${t("nav.wanted")} / ${t("history.title")}`,
          description: t("history.title"),
          groupLabel: automationGroupLabel,
          keywords: ["wanted", "history", "imports", "downloads", "blocklist", "failures"],
          icon: ListChecks,
          onSelect: buildNavigate(onNavigate, "wanted", undefined, undefined, undefined, "history"),
        } satisfies RouteCommand]
      : []),
    ...(canAccessActivity && activityImportCount > 0
      ? [{
          id: "activity-import",
          label: `${t("nav.activity")} / ${t("activity.import")}`,
          description: t("activity.import"),
          groupLabel: automationGroupLabel,
          keywords: ["activity", "import", "queue", "manual", "blocked"],
          icon: ActivitySquare,
          onSelect: buildNavigate(onNavigate, "activity", undefined, undefined, undefined, undefined, "import"),
        } satisfies RouteCommand]
      : []),
    ...(canAccessActivity
      ? [{
          id: "activity-history",
          label: `${t("nav.activity")} / ${t("activity.history")}`,
          description: t("activity.history"),
          groupLabel: automationGroupLabel,
          keywords: ["activity", "history", "completed", "failed", "downloads"],
          icon: ActivitySquare,
          onSelect: buildNavigate(onNavigate, "activity", undefined, undefined, undefined, undefined, "history"),
        } satisfies RouteCommand]
      : []),
    ...(canManageSystemSettings
      ? [{
          id: "settings-general",
          label: `${settingsGroupLabel} / ${t("settings.general")}`,
          description: t("nav.settings"),
          groupLabel: settingsGroupLabel,
          keywords: ["settings", "general", "preferences", "configuration", "system"],
          icon: Settings,
          onSelect: buildNavigate(onNavigate, "settings", "general"),
        } satisfies RouteCommand, {
          id: "settings-backups",
          label: `${systemGroupLabel} / ${t("settings.backups")}`,
          description: t("settings.backups"),
          groupLabel: systemGroupLabel,
          keywords: ["settings", "backups", "backup", "restore", "bundle", "download"],
          icon: Archive,
          onSelect: buildNavigate(onNavigate, "settings", "backups"),
        } satisfies RouteCommand]
      : []),
    {
      id: "settings-profile",
      label: `${settingsGroupLabel} / ${t("settings.profile")}`,
      description: t("settings.profile"),
      groupLabel: settingsGroupLabel,
      keywords: ["settings", "profile", "account", "me"],
      icon: User,
      onSelect: buildNavigate(onNavigate, "settings", "profile"),
    },
    ...(canManageUserAccounts
      ? [{
          id: "settings-security",
          label: `${systemGroupLabel} / ${t("settings.security")}`,
          description: t("settings.security"),
          groupLabel: systemGroupLabel,
          keywords: ["settings", "security", "auth", "login", "password"],
          icon: Shield,
          onSelect: buildNavigate(onNavigate, "settings", "security"),
        } satisfies RouteCommand]
      : []),
    ...(canManageUserAccess
      ? [{
          id: "settings-users",
          label: `${systemGroupLabel} / ${t("nav.usersAccess")}`,
          description: t("settings.users"),
          groupLabel: systemGroupLabel,
          keywords: ["settings", "users", "access", "accounts", "management"],
          icon: Users,
          onSelect: buildNavigate(onNavigate, "settings", "users"),
        } satisfies RouteCommand]
      : []),
    ...(canManageCatalogSettings
      ? [{
          id: "settings-quality-profiles",
          label: `${settingsGroupLabel} / ${t("settings.qualityProfiles")}`,
          description: t("settings.qualityProfiles"),
          groupLabel: settingsGroupLabel,
          keywords: ["settings", "quality", "profiles", "metadata", "rules"],
          icon: Settings,
          onSelect: buildNavigate(onNavigate, "settings", "qualityProfiles"),
        } satisfies RouteCommand, {
          id: "settings-delay-profiles",
          label: `${settingsGroupLabel} / ${t("settings.delayProfiles")}`,
          description: t("settings.delayProfiles"),
          groupLabel: settingsGroupLabel,
          keywords: ["settings", "delay", "profiles", "pending", "wait"],
          icon: Settings,
          onSelect: buildNavigate(onNavigate, "settings", "delayProfiles"),
        } satisfies RouteCommand, {
          id: "settings-rules",
          label: `${automationGroupLabel} / ${t("settings.rules")}`,
          description: t("settings.rules"),
          groupLabel: automationGroupLabel,
          keywords: ["settings", "rules", "rego", "opa", "scoring", "custom"],
          icon: SlidersHorizontal,
          onSelect: buildNavigate(onNavigate, "settings", "rules"),
        } satisfies RouteCommand, {
          id: "settings-acquisition",
          label: `${catalogsGroupLabel} / ${t("settings.acquisition")}`,
          description: t("settings.acquisition"),
          groupLabel: catalogsGroupLabel,
          keywords: ["settings", "acquisition", "search", "grab", "release"],
          icon: Search,
          onSelect: buildNavigate(onNavigate, "settings", "acquisition"),
        } satisfies RouteCommand, {
          id: "settings-post-processing",
          label: `${catalogsGroupLabel} / ${t("settings.postProcessing")}`,
          description: t("settings.postProcessing"),
          groupLabel: catalogsGroupLabel,
          keywords: ["settings", "post", "processing", "import", "rename", "move"],
          icon: FolderCog,
          onSelect: buildNavigate(onNavigate, "settings", "post-processing"),
        } satisfies RouteCommand, {
          id: "settings-subtitles",
          label: `${settingsGroupLabel} / ${t("settings.subtitles")}`,
          description: t("settings.subtitles"),
          groupLabel: settingsGroupLabel,
          keywords: ["settings", "subtitles", "captions", "srt", "opensubtitles"],
          icon: Captions,
          onSelect: buildNavigate(onNavigate, "settings", "subtitles"),
        } satisfies RouteCommand]
      : []),
    ...(canManageSystemSettings
      ? [{
          id: "settings-download-clients",
          label: `${integrationsGroupLabel} / ${t("settings.downloadClients")}`,
          description: t("settings.downloadClients"),
          groupLabel: integrationsGroupLabel,
          keywords: ["settings", "download", "clients", "indexers"],
          icon: Download,
          onSelect: buildNavigate(onNavigate, "settings", "downloadClients"),
        } satisfies RouteCommand, {
          id: "settings-indexers",
          label: `${integrationsGroupLabel} / ${t("settings.indexers")}`,
          description: t("settings.indexers"),
          groupLabel: integrationsGroupLabel,
          keywords: ["settings", "indexers", "feeds", "search", "sources"],
          icon: Database,
          onSelect: buildNavigate(onNavigate, "settings", "indexers"),
        } satisfies RouteCommand, {
          id: "settings-media-servers",
          label: `${integrationsGroupLabel} / ${t("settings.mediaServers")}`,
          description: t("settings.mediaServers"),
          groupLabel: integrationsGroupLabel,
          keywords: ["settings", "media", "servers", "plex", "jellyfin", "integrations"],
          icon: Server,
          onSelect: buildNavigate(onNavigate, "settings", "mediaServers"),
        } satisfies RouteCommand, {
          id: "settings-notifications",
          label: `${integrationsGroupLabel} / ${t("settings.notifications")}`,
          description: t("settings.notifications"),
          groupLabel: integrationsGroupLabel,
          keywords: ["settings", "notifications", "alerts", "discord", "webhook"],
          icon: Bell,
          onSelect: buildNavigate(onNavigate, "settings", "notifications"),
        } satisfies RouteCommand, {
          id: "settings-plugins",
          label: `${settingsGroupLabel} / ${t("settings.plugins")}`,
          description: t("settings.plugins"),
          groupLabel: settingsGroupLabel,
          keywords: ["settings", "plugins", "wasm", "extensions"],
          icon: Puzzle,
          onSelect: buildNavigate(onNavigate, "settings", "plugins"),
        } satisfies RouteCommand]
      : []),
    ...(canManageSystemSettings || canManageTitle
      ? [{
          id: "settings-recycle-bin",
          label: `${catalogsGroupLabel} / ${t("settings.recycleBin")}`,
          description: t("settings.recycleBin"),
          groupLabel: catalogsGroupLabel,
          keywords: ["settings", "recycle", "bin", "trash", "deleted"],
          icon: Trash2,
          onSelect: buildNavigate(onNavigate, "settings", "recycleBin"),
        } satisfies RouteCommand]
      : []),
    ...(canManageSystemSettings
      ? [{
          id: "system-audit",
          label: t("nav.logs"),
          description: t("nav.logs"),
          groupLabel: systemGroupLabel,
          keywords: ["audit", "events", "logs", "log", "history", "delete"],
          icon: FileText,
          onSelect: buildNavigate(onNavigate, "system", undefined, undefined, "audit"),
        } satisfies RouteCommand]
      : []),
  ];
}
