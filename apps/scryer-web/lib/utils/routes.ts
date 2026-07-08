import type {
  ContentSettingsSection,
  SettingsSection,
  ViewId,
} from "@/components/root/types";
import { isMediaView } from "@/lib/facets/registry";

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
  canAccessRecycleBin: boolean,
): boolean {
  switch (section) {
    case "profile":
      return true;
    case "security":
      return canManageUserAccounts;
    case "users":
      return canManageUserAccess;
    case "recycleBin":
      return canAccessRecycleBin;
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
