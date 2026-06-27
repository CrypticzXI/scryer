
import { lazy, memo, Suspense, useCallback, useState } from "react";
import { Link } from "react-router-dom";
import {
  Captions,
  ChevronRight,
  Puzzle,
  Settings2,
  SlidersHorizontal,
  Timer,
  User,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import type { SettingsSection } from "@/components/root/types";
import type { LocaleCode, LanguageOption } from "@/lib/i18n";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";
import { buildViewPath } from "@/lib/utils/routing";
import {
  type ProviderCatalogFamily,
  useProviderCatalogSubscription,
} from "@/lib/hooks/use-provider-catalog-subscription";

const SettingsOverviewContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-overview-container")).SettingsOverviewContainer,
}));
const SettingsSecurityContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-security-container")).SettingsSecurityContainer,
}));
const SettingsUsersContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-users-container")).SettingsUsersContainer,
}));
const SettingsIndexersContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-indexers-container")).SettingsIndexersContainer,
}));
const SettingsMediaServersContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-media-servers-container")).SettingsMediaServersContainer,
}));
const SettingsDownloadClientsContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-download-clients-container")).SettingsDownloadClientsContainer,
}));
const SettingsDelayProfilesContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-delay-profiles-container")).SettingsDelayProfilesContainer,
}));
const SettingsQualityProfilesContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-quality-profiles-container")).SettingsQualityProfilesContainer,
}));
const SettingsProfileContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-profile-container")).SettingsProfileContainer,
}));
const SettingsRulesContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-rules-container")).SettingsRulesContainer,
}));
const SettingsPluginsContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-plugins-container")).SettingsPluginsContainer,
}));
const SettingsNotificationsContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-notifications-container")).SettingsNotificationsContainer,
}));
const SettingsPostProcessingContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-post-processing-container")).SettingsPostProcessingContainer,
}));
const SettingsSubtitlesContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-subtitles-container")).SettingsSubtitlesContainer,
}));
const SettingsRecycleBinContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-recycle-bin-container")).SettingsRecycleBinContainer,
}));
const SettingsBackupsContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-backups-container")).SettingsBackupsContainer,
}));

type SettingsContainerProps = {
  settingsSection: SettingsSection;
  userId?: string;
  username?: string;
  canManageSystemSettings: boolean;
  canManageCatalogSettings: boolean;
  availableLanguages: LanguageOption[];
  selectedLanguage: LanguageOption | null;
  uiLanguage: LocaleCode;
  onSelectLanguage: (code: string) => void;
};

export const SettingsContainer = memo(function SettingsContainer({
  settingsSection,
  userId,
  username,
  canManageSystemSettings,
  canManageCatalogSettings,
  availableLanguages,
  selectedLanguage,
  uiLanguage,
  onSelectLanguage,
}: SettingsContainerProps) {
  const t = useTranslate();
  const [providerCatalogVersions, setProviderCatalogVersions] = useState<
    Record<ProviderCatalogFamily, number>
  >({
    subtitle: 0,
    notification: 0,
    indexer: 0,
    download_client: 0,
  });
  const showPluginsLink =
    settingsSection === "downloadClients" ||
    settingsSection === "indexers" ||
    settingsSection === "notifications" ||
    settingsSection === "subtitles";
  const subscribeToProviderCatalog = showPluginsLink;
  // Surfaces that embed the FilteredPluginList manage plugins inline, so they
  // don't need the shortcut to the standalone Plugins page.
  const showPluginsShortcut =
    showPluginsLink && settingsSection !== "indexers";
  const settingsSectionLabel =
    settingsSection === "profile"
      ? t("settings.profile")
      : settingsSection === "general"
        ? t("settings.general")
        : settingsSection === "backups"
          ? t("settings.backups")
          : settingsSection === "security"
            ? t("settings.security")
            : settingsSection === "users"
              ? t("settings.users")
              : settingsSection === "mediaServers"
                ? t("settings.mediaServers")
                : settingsSection === "indexers"
                  ? t("settings.indexers")
                  : settingsSection === "downloadClients"
                    ? t("settings.downloadClients")
                    : settingsSection === "rules"
                      ? t("settings.rules")
                      : settingsSection === "plugins"
                        ? t("settings.plugins")
                        : settingsSection === "notifications"
                          ? t("settings.notifications")
                          : settingsSection === "post-processing"
                            ? t("settings.postProcessing")
                            : settingsSection === "subtitles"
                              ? t("settings.subtitles")
                              : settingsSection === "recycleBin"
                                ? t("settings.recycleBin")
                                : settingsSection === "delayProfiles"
                                  ? t("settings.delayProfiles")
                                  : t("settings.qualityProfiles");
  const primarySettingsNav = [
    {
      section: "profile" as const,
      label: t("settings.profile"),
      icon: User,
      visible: true,
    },
    {
      section: "general" as const,
      label: t("settings.general"),
      icon: Settings2,
      visible: canManageSystemSettings,
    },
    {
      section: "qualityProfiles" as const,
      label: t("settings.qualityProfiles"),
      icon: SlidersHorizontal,
      visible: canManageCatalogSettings,
    },
    {
      section: "delayProfiles" as const,
      label: t("settings.delayProfiles"),
      icon: Timer,
      visible: canManageCatalogSettings,
    },
    {
      section: "plugins" as const,
      label: t("settings.plugins"),
      icon: Puzzle,
      visible: canManageSystemSettings,
    },
    {
      section: "subtitles" as const,
      label: t("settings.subtitles"),
      icon: Captions,
      visible: canManageCatalogSettings,
    },
  ].filter((item) => item.visible);
  const showPrimarySettingsSubnav = primarySettingsNav.some(
    (item) => item.section === settingsSection,
  );
  const SettingsSectionIcon =
    primarySettingsNav.find((item) => item.section === settingsSection)?.icon ??
    Settings2;

  useProviderCatalogSubscription(
    useCallback((families: ProviderCatalogFamily[]) => {
      setProviderCatalogVersions((previous) => {
        const uniqueFamilies = [...new Set(families)];
        if (uniqueFamilies.length === 0) {
          return previous;
        }

        const next = { ...previous };
        for (const family of uniqueFamilies) {
          next[family] += 1;
        }
        return next;
      });
    }, []),
    subscribeToProviderCatalog,
  );

  return (
    <div className="flex min-h-0 w-full flex-1 flex-col overflow-hidden bg-transparent md:flex-row">
      {showPrimarySettingsSubnav ? (
        <aside
          data-slot="settings-subnav-scroll"
          className="w-full shrink-0 border-b border-[var(--scry-border3)] bg-[var(--scry-surfF)] p-3 md:h-full md:w-[218px] md:overflow-y-auto md:border-b-0 md:border-r md:p-[22px_14px]"
        >
          <div className="mb-3 flex items-center gap-2 px-2 text-[var(--scry-ink2)] md:mb-4">
            <Settings2 className="h-[18px] w-[18px] text-[var(--scry-accent-text)]" />
            <span className="text-[16px] font-bold">{t("nav.settings")}</span>
          </div>
          <nav className="flex gap-2 overflow-x-auto pb-1 md:flex-col md:overflow-visible md:pb-0">
            {primarySettingsNav.map((item) => {
              const Icon = item.icon;
              const active = settingsSection === item.section;
              return (
                <Link
                  key={item.section}
                  to={buildViewPath("settings", item.section)}
                  className={cn(
                    "flex h-9 shrink-0 items-center gap-2 rounded-[9px] px-3 text-[13px] font-medium text-[var(--scry-muted)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] md:w-full",
                    active &&
                      "bg-[linear-gradient(90deg,rgba(var(--scry-accent-rgb),0.26),rgba(var(--scry-accent-rgb),0.08))] text-[var(--scry-ink2)] shadow-[inset_2px_0_0_var(--scry-accent-ring)]",
                  )}
                >
                  <Icon
                    className={cn(
                      "h-[17px] w-[17px] text-[var(--scry-muted2)]",
                      active && "text-[var(--scry-accent-text)]",
                    )}
                  />
                  <span className="whitespace-nowrap">{item.label}</span>
                </Link>
              );
            })}
          </nav>
        </aside>
      ) : null}
      <main
        data-slot="settings-main-scroll"
        className="min-w-0 flex-1 overflow-y-auto bg-transparent"
      >
        <div
          className={cn(
            "mx-auto w-full px-4 py-5 sm:px-6 md:px-[30px] md:py-[26px] md:pb-[60px]",
            settingsSection === "rules" ||
            settingsSection === "indexers" ||
            settingsSection === "post-processing"
              ? "max-w-none"
              : "max-w-[1280px]",
          )}
        >
          <div className="mb-4 flex items-center gap-1.5 text-[12.5px] text-[var(--scry-faint)]">
            <span>{t("nav.settings")}</span>
            <ChevronRight className="h-3.5 w-3.5" />
            <span className="font-semibold text-[var(--scry-accent-text)]">{settingsSectionLabel}</span>
          </div>
          <div className="mb-6 flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <div className="flex min-w-0 items-start gap-4">
              <div className="flex h-[46px] w-[46px] shrink-0 items-center justify-center rounded-[13px] border border-[var(--scry-baccent)] bg-[linear-gradient(135deg,rgba(var(--scry-accent-rgb),0.35),rgba(123,91,255,0.22))] text-[var(--scry-accent-text)]">
                <SettingsSectionIcon className="h-[23px] w-[23px]" />
              </div>
              <div className="min-w-0">
                <h1 className="text-[25px] font-bold tracking-normal text-[var(--scry-ink2)]">
                  {settingsSectionLabel}
                </h1>
                <p className="mt-1 max-w-[640px] text-[13.5px] text-[var(--scry-muted)]">
                  {t("settings.sectionTitle", { section: settingsSectionLabel })}
                </p>
              </div>
            </div>
            {showPluginsShortcut ? (
              <Button asChild variant="primary" className="h-10 shrink-0 rounded-[10px] px-3 text-[13px]">
                <Link to="/settings/plugins">{t("settings.plugins")}</Link>
              </Button>
            ) : null}
          </div>
          <Suspense fallback={<div className="py-6 text-sm text-[var(--scry-muted3)]">{t("label.loading")}</div>}>
          {settingsSection === "profile" ? (
            <SettingsProfileContainer
              userId={userId}
              username={username}
            />
          ) : settingsSection === "general" ? (
            <SettingsOverviewContainer
              availableLanguages={availableLanguages}
              selectedLanguage={selectedLanguage}
              uiLanguage={uiLanguage}
              onSelectLanguage={onSelectLanguage}
            />
          ) : settingsSection === "backups" ? (
            <SettingsBackupsContainer />
          ) : settingsSection === "security" ? (
            <SettingsSecurityContainer />
          ) : settingsSection === "users" ? (
            <SettingsUsersContainer />
          ) : settingsSection === "mediaServers" ? (
            <SettingsMediaServersContainer />
          ) : settingsSection === "indexers" ? (
            <SettingsIndexersContainer
              providerCatalogVersion={providerCatalogVersions.indexer}
            />
          ) : settingsSection === "downloadClients" ? (
            <SettingsDownloadClientsContainer
              providerCatalogVersion={providerCatalogVersions.download_client}
            />
          ) : settingsSection === "rules" ? (
            <SettingsRulesContainer />
          ) : settingsSection === "plugins" ? (
            <SettingsPluginsContainer />
          ) : settingsSection === "notifications" ? (
            <SettingsNotificationsContainer
              providerCatalogVersion={providerCatalogVersions.notification}
            />
          ) : settingsSection === "post-processing" ? (
            <SettingsPostProcessingContainer />
          ) : settingsSection === "subtitles" ? (
            <SettingsSubtitlesContainer
              providerCatalogVersion={providerCatalogVersions.subtitle}
            />
          ) : settingsSection === "recycleBin" ? (
            <SettingsRecycleBinContainer />
          ) : settingsSection === "delayProfiles" ? (
            <SettingsDelayProfilesContainer />
          ) : (
            <SettingsQualityProfilesContainer />
          )}
          </Suspense>
        </div>
      </main>
    </div>
  );
});
