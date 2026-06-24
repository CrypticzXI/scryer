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
  return section === "import" || isMediaSettingsSection(section);
}

export function canAccessMediaSettingsSection(
  section: ContentSettingsSection,
  canManageConfig: boolean,
  canManageLibrarySettings: boolean,
): boolean {
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
  canManageUsers: boolean,
  canManageConfig: boolean,
  canAccessRecycleBin: boolean,
): boolean {
  if (section === "profile") {
    return true;
  }

  if (section === "security" || section === "users") {
    return canManageUsers;
  }

  if (section === "recycleBin") {
    return canAccessRecycleBin;
  }

  return canManageConfig;
}
