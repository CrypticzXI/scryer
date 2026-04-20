import type { LucideIcon } from "lucide-react";
import {
  ActivitySquare,
  CalendarDays,
  History,
  MonitorCog,
  Settings,
  Users,
} from "lucide-react";
import type {
  ContentSettingsSection,
  SettingsSection,
  SystemSection,
  Translate,
  ViewId,
  WantedSection,
} from "@/components/root/types";
import { FACET_REGISTRY } from "@/lib/facets/registry";
import type { PendingImportCounts } from "@/lib/types";
import { pendingImportCountForView } from "@/lib/types";

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
  onNavigate: (
    nextView: ViewId,
    nextSettingsSection?: SettingsSection,
    nextContentSection?: ContentSettingsSection,
    nextSystemSection?: SystemSection,
    nextWantedSection?: WantedSection,
  ) => void;
};

function buildNavigate(
  onNavigate: BuildRouteCommandsArgs["onNavigate"],
  view: ViewId,
  settingsSection?: SettingsSection,
  contentSection?: ContentSettingsSection,
  systemSection?: SystemSection,
  wantedSection?: WantedSection,
): () => void {
  return () => {
    onNavigate(view, settingsSection, contentSection, systemSection, wantedSection);
  };
}

export function buildRouteCommands({
  t,
  pendingImportCounts,
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

    if (pendingImportCountForView(pendingImportCounts, f.viewId) > 0) {
      commands.push({
        id: `${f.viewId}-import`,
        label: `${t(f.navLabelKey)} / ${t("nav.import")}`,
        description: t("nav.import"),
        keywords: [f.viewId, f.id, "import", "pending", "unmatched", "match"],
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
      id: "activity",
      label: t("nav.activity"),
      description: t("nav.activity"),
      keywords: ["activity", "events", "log", "audit", "system"],
      icon: ActivitySquare,
      onSelect: buildNavigate(onNavigate, "activity"),
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
      id: "history",
      label: t("nav.history"),
      description: t("nav.history"),
      keywords: ["history", "imports", "import", "log", "records"],
      icon: History,
      onSelect: buildNavigate(onNavigate, "history"),
    },
    {
      id: "settings-general",
      label: `${t("nav.settings")} / ${t("settings.general")}`,
      description: t("nav.settings"),
      keywords: ["settings", "general", "preferences", "configuration", "system"],
      icon: Users,
      onSelect: buildNavigate(onNavigate, "settings", "general"),
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
