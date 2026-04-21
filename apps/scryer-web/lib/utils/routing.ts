import type { LocaleCode } from "@/lib/i18n";
import type {
  ContentSettingsSection,
  SettingsSection,
  SystemSection,
  ViewId,
  WantedSection,
} from "@/components/root/types";
import { normalizeLocale } from "@/lib/i18n";
import { AVAILABLE_LANGUAGES } from "@/lib/i18n";
import { URL_PATH_SEGMENTS } from "@/lib/constants/settings";
import {
  SETTINGS_SECTION_PATH_TO_ID,
  CONTENT_SECTION_PATH_TO_ID,
  CONTENT_SETTINGS_SUB_PAGE_PATH_TO_ID,
  SYSTEM_SECTION_PATH_TO_ID,
  WANTED_SECTION_PATH_TO_ID,
} from "@/lib/constants/settings";
import { isMediaView } from "@/lib/facets/registry";

export const SETTINGS_SECTION_PATH: Record<SettingsSection, string> = {
  profile: "profile",
  general: "general",
  users: "users",
  indexers: "indexers",
  downloadClients: "download-clients",
  qualityProfiles: "quality-profiles",
  delayProfiles: "delay-profiles",
  acquisition: "acquisition",
  rules: "rules",
  plugins: "plugins",
  notifications: "notifications",
  "post-processing": "post-processing",
  subtitles: "subtitles",
  recycleBin: "recycle-bin",
};

export const CONTENT_SECTION_PATH: Record<ContentSettingsSection, string> = {
  overview: "overview",
  import: "import",
  settings: "settings",
  general: "settings/general",
  quality: "settings/quality",
  renaming: "settings/renaming",
  routing: "settings/routing",
};

export const WANTED_SECTION_PATH: Record<WantedSection, string> = {
  wanted: "wanted-items",
  cutoff: "cutoff-unmet",
  pending: "pending",
};

const MEDIA_RESERVED_OVERVIEW_SEGMENTS = new Set(["overview", "import", "settings"]);

export function buildViewPath(
  nextView: ViewId,
  nextSettingsSection?: SettingsSection,
  nextContentSection?: ContentSettingsSection,
  nextSystemSection?: SystemSection,
  nextWantedSection?: WantedSection,
) {
  const base = `/${nextView}`;
  if (nextView === "settings" && nextSettingsSection && nextSettingsSection !== "profile") {
    return `${base}/${SETTINGS_SECTION_PATH[nextSettingsSection]}`;
  }
  if (nextView === "system" && nextSystemSection && nextSystemSection !== "overview") {
    return `${base}/${nextSystemSection}`;
  }
  if (nextView === "wanted") {
    return `${base}/${WANTED_SECTION_PATH[nextWantedSection ?? "wanted"]}`;
  }
  if (isMediaView(nextView)) {
    if (nextContentSection && nextContentSection !== "overview") {
      return `${base}/${CONTENT_SECTION_PATH[nextContentSection]}`;
    }
  }
  return base;
}

export function buildOverviewDetailPath(view: ViewId, slug?: string | null) {
  const normalizedSlug = slug?.trim();
  if (!normalizedSlug) {
    return `/${view}/overview`;
  }
  return `/${view}/${encodeURIComponent(normalizedSlug)}`;
}

export function isLocaleSupported(code: string): code is LocaleCode {
  return AVAILABLE_LANGUAGES.some((language) => language.code === code);
}

export function parseViewFromPath(pathname: string | null | undefined): ViewId {
  const segment = (pathname ?? "").trim().toLowerCase();
  if (!segment) {
    return "movies";
  }

  return URL_PATH_SEGMENTS.includes(segment as ViewId) ? (segment as ViewId) : "movies";
}

export function parseSettingsSectionFromPath(value: string | null): SettingsSection {
  if (!value) {
    return "profile";
  }
  return SETTINGS_SECTION_PATH_TO_ID[value] ?? "profile";
}

export function parseContentSectionFromPath(value: string | null, subValue?: string | null): ContentSettingsSection {
  if (!value) {
    return "overview";
  }
  if (value === "settings" && subValue) {
    return CONTENT_SETTINGS_SUB_PAGE_PATH_TO_ID[subValue] ?? "general";
  }
  if (value === "settings") {
    return "general";
  }
  return CONTENT_SECTION_PATH_TO_ID[value] ?? "overview";
}

export function parseOverviewSlugFromPath(value: string | null, subValue?: string | null): string | null {
  const normalizedValue = value?.trim();
  if (!normalizedValue || subValue) {
    return null;
  }

  if (MEDIA_RESERVED_OVERVIEW_SEGMENTS.has(normalizedValue.toLowerCase())) {
    return null;
  }

  try {
    return decodeURIComponent(normalizedValue);
  } catch {
    return normalizedValue;
  }
}

export function parseSystemSectionFromPath(value: string | null): SystemSection {
  if (!value) {
    return "overview";
  }
  return SYSTEM_SECTION_PATH_TO_ID[value] ?? "overview";
}

export function parseWantedSectionFromPath(value: string | null): WantedSection {
  if (!value) {
    return "wanted";
  }
  return WANTED_SECTION_PATH_TO_ID[value] ?? "wanted";
}

export function parseLanguageFromParam(value: string | null): LocaleCode | null {
  if (!value) {
    return null;
  }

  const normalized = normalizeLocale(value);
  return isLocaleSupported(normalized) ? normalized : null;
}
