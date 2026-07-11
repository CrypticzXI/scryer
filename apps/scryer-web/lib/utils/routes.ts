import type {
  ContentSettingsSection,
  SettingsSection,
  SystemSection,
  ViewId,
} from "@/components/root/types";
import { isMediaView } from "../facets/registry.ts";

export function isMediaSettingsSection(section: ContentSettingsSection): boolean {
  return (
    section === "library" ||
    section === "general" ||
    section === "quality" ||
    section === "renaming" ||
    section === "routing"
  );
}

export function isProtectedSettingsRoute(
  view: ViewId,
  settingsSection: SettingsSection,
  contentSettingsSection: ContentSettingsSection,
): boolean {
  if (view === "settings") {
    return settingsSection !== "profile";
  }

  return isMediaView(view) && isMediaSettingsSection(contentSettingsSection);
}

export function isManageConfigMediaSection(section: ContentSettingsSection): boolean {
  return isMediaSettingsSection(section);
}

export function canAccessMediaSettingsSection(
  section: ContentSettingsSection,
  canManageConfig: boolean,
  canManageLibrarySettings: boolean,
  canResolveImports = false,
): boolean {
  if (section === "import") {
    return canResolveImports;
  }

  if (!isManageConfigMediaSection(section)) {
    return true;
  }

  if (section === "library") {
    return canManageConfig || canManageLibrarySettings;
  }

  return canManageConfig;
}

export function canAccessSettingsSection(
  section: SettingsSection,
  canManageUserAccounts: boolean,
  canManageUserAccess: boolean,
  canManageSystemSettings: boolean,
  canManageCatalogSettings: boolean,
): boolean {
  switch (section) {
    case "profile":
      return true;
    case "security":
      return canManageUserAccounts;
    case "users":
      return canManageUserAccess;
    case "general":
    case "backups":
    case "mediaServers":
    case "indexers":
    case "downloadClients":
    case "acquisition":
    case "plugins":
    case "notifications":
      return canManageSystemSettings;
    case "qualityProfiles":
    case "delayProfiles":
    case "rules":
    case "post-processing":
    case "subtitles":
      return canManageCatalogSettings;
    default:
      return false;
  }
}

export function canAccessRecycleBinPage(
  canManageSystemSettings: boolean,
  canManageTitle: boolean,
): boolean {
  return canManageSystemSettings || canManageTitle;
}

export function canAccessSystemSection(
  section: SystemSection,
  canManageSystemSettings: boolean,
  canManageTitle: boolean,
): boolean {
  return section === "recycleBin"
    ? canAccessRecycleBinPage(canManageSystemSettings, canManageTitle)
    : canManageSystemSettings;
}
