import type { LucideIcon } from "lucide-react";
import {
  ActivitySquare,
  Bell,
  CalendarDays,
  Captions,
  FolderCog,
  MonitorCog,
  Puzzle,
  Search,
  Settings,
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
import { hasImportItemsForView, type PendingImportCounts } from "@/lib/types";

export type RouteCommand = {
  id: string;
  label: string;
  description: string;
  keywords: string[];
  icon: LucideIcon;
  onSelect: () => void;
};

type BuildRouteCommandsArgs = {
  t: Translate;
  pendingImportCounts: PendingImportCounts | null;
  ignoredImportCounts?: PendingImportCounts | null;
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
  pendingImportCounts,
  ignoredImportCounts,
  activityImportCount = 0,
  onNavigate,
}: BuildRouteCommandsArgs): RouteCommand[] {
  const mediaCommands = FACET_REGISTRY.flatMap((f) => {
    const commands: RouteCommand[] = [
      {
        id: `${f.viewId}-overview`,
        label: t(f.overviewLabelKey),
        description: t(f.navLabelKey),
        keywords: [f.viewId, f.id, "manage", "catalog", "overview", "library"],
        icon: f.icon,
        onSelect: buildNavigate(onNavigate, f.viewId as ViewId),
      },
    ];

    if (hasImportItemsForView(pendingImportCounts, ignoredImportCounts, f.viewId)) {
      commands.push({
        id: `${f.viewId}-import`,
        label: `${t(f.navLabelKey)} / ${t("nav.import")}`,
        description: t("nav.import"),
        keywords: [f.viewId, f.id, "import", "pending", "ignored", "unmatched", "match"],
        icon: f.icon,
        onSelect: buildNavigate(onNavigate, f.viewId as ViewId, undefined, "import"),
      });
    }

    commands.push({
      id: `${f.viewId}-settings`,
      label: t(f.settingsLabelKey),
      description: t(f.settingsLabelKey),
      keywords: [f.viewId, f.id, "settings", "media", "paths", "folder"],
      icon: Settings,
      onSelect: buildNavigate(onNavigate, f.viewId as ViewId, undefined, "general"),
    });

    const facetSubSections: Array<{
      section: ContentSettingsSection;
      labelKey: string;
      extraKeywords: string[];
    }> = [
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
    for (const sub of facetSubSections) {
      commands.push({
        id: `${f.viewId}-settings-${sub.section}`,
        label: `${t(f.settingsLabelKey)} / ${t(sub.labelKey)}`,
        description: t(sub.labelKey),
        keywords: [f.viewId, f.id, "settings", ...sub.extraKeywords],
        icon: Settings,
        onSelect: buildNavigate(onNavigate, f.viewId as ViewId, undefined, sub.section),
      });
    }

    return commands;
  });

  return [
    ...mediaCommands,
    {
      id: "wanted-items",
      label: "Wanted / Wanted Items",
      description: t("wanted.tabWanted"),
      keywords: ["wanted", "missing", "wanted items", "acquisition", "search"],
      icon: ActivitySquare,
      onSelect: buildNavigate(onNavigate, "wanted", undefined, undefined, undefined, "wanted"),
    },
    {
      id: "wanted-cutoff",
      label: "Wanted / Cutoff Unmet",
      description: t("wanted.tabCutoff"),
      keywords: ["wanted", "cutoff", "upgrade", "quality", "unmet"],
      icon: ActivitySquare,
      onSelect: buildNavigate(onNavigate, "wanted", undefined, undefined, undefined, "cutoff"),
    },
    {
      id: "wanted-pending",
      label: "Wanted / Pending",
      description: t("wanted.tabPending"),
      keywords: ["wanted", "pending", "delayed", "releases"],
      icon: ActivitySquare,
      onSelect: buildNavigate(onNavigate, "wanted", undefined, undefined, undefined, "pending"),
    },
    {
      id: "wanted-history",
      label: `${t("nav.wanted")} / ${t("history.title")}`,
      description: t("history.title"),
      keywords: ["wanted", "history", "imports", "downloads", "blocklist", "failures"],
      icon: ActivitySquare,
      onSelect: buildNavigate(onNavigate, "wanted", undefined, undefined, undefined, "history"),
    },
    {
      id: "activity-overview",
      label: `${t("nav.activity")} / ${t("activity.activity")}`,
      description: t("activity.activity"),
      keywords: ["activity", "events", "log", "audit", "system", "queue"],
      icon: ActivitySquare,
      onSelect: buildNavigate(onNavigate, "activity", undefined, undefined, undefined, undefined, "activity"),
    },
    ...(activityImportCount > 0
      ? [{
          id: "activity-import",
          label: `${t("nav.activity")} / ${t("activity.import")}`,
          description: t("activity.import"),
          keywords: ["activity", "import", "queue", "manual", "blocked"],
          icon: ActivitySquare,
          onSelect: buildNavigate(onNavigate, "activity", undefined, undefined, undefined, undefined, "import"),
        } satisfies RouteCommand]
      : []),
    {
      id: "activity-history",
      label: `${t("nav.activity")} / ${t("activity.history")}`,
      description: t("activity.history"),
      keywords: ["activity", "history", "completed", "failed", "downloads"],
      icon: ActivitySquare,
      onSelect: buildNavigate(onNavigate, "activity", undefined, undefined, undefined, undefined, "history"),
    },
    {
      id: "calendar",
      label: t("nav.calendar"),
      description: t("nav.calendar"),
      keywords: ["calendar", "episodes", "airing", "schedule", "upcoming"],
      icon: CalendarDays,
      onSelect: buildNavigate(onNavigate, "calendar"),
    },
    {
      id: "settings-general",
      label: `${t("nav.settings")} / ${t("settings.general")}`,
      description: t("nav.settings"),
      keywords: ["settings", "general", "preferences", "configuration", "system"],
      icon: Settings,
      onSelect: buildNavigate(onNavigate, "settings", "general"),
    },
    {
      id: "settings-profile",
      label: `${t("nav.settings")} / ${t("settings.profile")}`,
      description: t("settings.profile"),
      keywords: ["settings", "profile", "account", "me"],
      icon: User,
      onSelect: buildNavigate(onNavigate, "settings", "profile"),
    },
    {
      id: "settings-users",
      label: t("settings.users"),
      description: t("settings.users"),
      keywords: ["settings", "users", "accounts", "management"],
      icon: Users,
      onSelect: buildNavigate(onNavigate, "settings", "users"),
    },
    {
      id: "settings-quality-profiles",
      label: t("settings.qualityProfiles"),
      description: t("settings.qualityProfiles"),
      keywords: ["settings", "quality", "profiles", "metadata", "rules"],
      icon: Settings,
      onSelect: buildNavigate(onNavigate, "settings", "qualityProfiles"),
    },
    {
      id: "settings-delay-profiles",
      label: t("settings.delayProfiles"),
      description: t("settings.delayProfiles"),
      keywords: ["settings", "delay", "profiles", "pending", "wait"],
      icon: Settings,
      onSelect: buildNavigate(onNavigate, "settings", "delayProfiles"),
    },
    {
      id: "settings-download-clients",
      label: t("settings.downloadClients"),
      description: t("settings.downloadClients"),
      keywords: ["settings", "download", "clients", "indexers"],
      icon: Settings,
      onSelect: buildNavigate(onNavigate, "settings", "downloadClients"),
    },
    {
      id: "settings-indexers",
      label: t("settings.indexers"),
      description: t("settings.indexers"),
      keywords: ["settings", "indexers", "feeds", "search", "sources"],
      icon: Settings,
      onSelect: buildNavigate(onNavigate, "settings", "indexers"),
    },
    {
      id: "settings-rules",
      label: t("settings.rules"),
      description: t("settings.rules"),
      keywords: ["settings", "rules", "rego", "opa", "scoring", "custom"],
      icon: Settings,
      onSelect: buildNavigate(onNavigate, "settings", "rules"),
    },
    {
      id: "settings-acquisition",
      label: t("settings.acquisition"),
      description: t("settings.acquisition"),
      keywords: ["settings", "acquisition", "search", "grab", "release"],
      icon: Search,
      onSelect: buildNavigate(onNavigate, "settings", "acquisition"),
    },
    {
      id: "settings-post-processing",
      label: t("settings.postProcessing"),
      description: t("settings.postProcessing"),
      keywords: ["settings", "post", "processing", "import", "rename", "move"],
      icon: FolderCog,
      onSelect: buildNavigate(onNavigate, "settings", "post-processing"),
    },
    {
      id: "settings-subtitles",
      label: t("settings.subtitles"),
      description: t("settings.subtitles"),
      keywords: ["settings", "subtitles", "captions", "srt", "opensubtitles"],
      icon: Captions,
      onSelect: buildNavigate(onNavigate, "settings", "subtitles"),
    },
    {
      id: "settings-notifications",
      label: t("settings.notifications"),
      description: t("settings.notifications"),
      keywords: ["settings", "notifications", "alerts", "discord", "webhook"],
      icon: Bell,
      onSelect: buildNavigate(onNavigate, "settings", "notifications"),
    },
    {
      id: "settings-plugins",
      label: t("settings.plugins"),
      description: t("settings.plugins"),
      keywords: ["settings", "plugins", "wasm", "extensions"],
      icon: Puzzle,
      onSelect: buildNavigate(onNavigate, "settings", "plugins"),
    },
    {
      id: "settings-recycle-bin",
      label: t("settings.recycleBin"),
      description: t("settings.recycleBin"),
      keywords: ["settings", "recycle", "bin", "trash", "deleted"],
      icon: Trash2,
      onSelect: buildNavigate(onNavigate, "settings", "recycleBin"),
    },
    {
      id: "system",
      label: t("nav.system"),
      description: t("nav.system"),
      keywords: ["system", "health", "status", "database", "worker"],
      icon: MonitorCog,
      onSelect: buildNavigate(onNavigate, "system"),
    },
    {
      id: "system-jobs",
      label: t("system.jobsTitle"),
      description: t("system.jobsTitle"),
      keywords: ["system", "jobs", "scheduler", "background", "rss", "library"],
      icon: MonitorCog,
      onSelect: buildNavigate(onNavigate, "system", undefined, undefined, "jobs"),
    },
  ];
}
