
import { lazy, memo, Suspense, useCallback, useEffect, useState } from "react";
import { Link } from "react-router-dom";
import {
  Archive,
  Bell,
  Captions,
  ChevronRight,
  Database,
  Download,
  FolderCog,
  Puzzle,
  Server,
  Settings2,
  ShieldCheck,
  SlidersHorizontal,
  Timer,
  User,
  Users,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import type { SettingsSection } from "@/components/root/types";
import type { LocaleCode, LanguageOption } from "@/lib/i18n";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";
import { selectorId } from "@/lib/utils/dom-ids";
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

/** DOM id of the right reference rail. Inline plugin managers are portaled here
 * so they anchor to the top of the pane. */
export const SETTINGS_REFERENCE_SLOT_ID = "settings-content-reference";
export const SETTINGS_HEADER_ACTIONS_SLOT_ID = "settings-header-actions";

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
    showPluginsLink &&
    settingsSection !== "indexers" &&
    settingsSection !== "downloadClients" &&
    settingsSection !== "notifications" &&
    settingsSection !== "subtitles";
  // Pages that render an inline plugins rail anchored to the top of the content pane.
  const showReferenceRail =
    settingsSection === "indexers" ||
    settingsSection === "downloadClients" ||
    settingsSection === "notifications" ||
    settingsSection === "subtitles";
  const isSubtitlesSection = settingsSection === "subtitles";
  const [referenceRailOpen, setReferenceRailOpen] = useState(false);
  const [referenceRailDocked, setReferenceRailDocked] = useState(false);

  useEffect(() => {
    setReferenceRailOpen(false);
  }, [referenceRailDocked, settingsSection]);

  useEffect(() => {
    if (!showReferenceRail) {
      setReferenceRailDocked(false);
      return;
    }

    const mediaQuery = window.matchMedia(
      isSubtitlesSection ? "(min-width: 1440px)" : "(min-width: 1960px)",
    );
    const syncReferenceRailMode = () => {
      setReferenceRailDocked(mediaQuery.matches);
    };

    syncReferenceRailMode();
    mediaQuery.addEventListener("change", syncReferenceRailMode);
    return () =>
      mediaQuery.removeEventListener("change", syncReferenceRailMode);
  }, [isSubtitlesSection, showReferenceRail]);

  useEffect(() => {
    if (!referenceRailOpen || referenceRailDocked) {
      return;
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setReferenceRailOpen(false);
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [referenceRailDocked, referenceRailOpen]);

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
  const usesAutomationHeader =
    settingsSection === "rules" || settingsSection === "post-processing";
  const usesIntegrationsHeader =
    settingsSection === "downloadClients" ||
    settingsSection === "indexers" ||
    settingsSection === "mediaServers" ||
    settingsSection === "notifications";
  const usesAccessHeader = settingsSection === "security" || settingsSection === "users";
  const usesSystemHeader = usesAccessHeader || settingsSection === "backups";
  const SettingsSectionIcon = (() => {
    switch (settingsSection) {
      case "rules":
        return SlidersHorizontal;
      case "post-processing":
        return FolderCog;
      case "indexers":
        return Database;
      case "downloadClients":
        return Download;
      case "mediaServers":
        return Server;
      case "notifications":
        return Bell;
      case "security":
        return ShieldCheck;
      case "users":
        return Users;
      case "backups":
        return Archive;
      default:
        return (
          primarySettingsNav.find((item) => item.section === settingsSection)?.icon ??
          Settings2
        );
    }
  })();
  const breadcrumbRootLabel =
    usesAutomationHeader
      ? t("nav.group.automation")
      : usesIntegrationsHeader
        ? t("nav.group.integrations")
        : usesSystemHeader
          ? t("nav.group.system")
      : t("nav.settings");

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
          <nav className="flex gap-2 overflow-x-auto pb-1 md:flex-col md:overflow-visible md:pb-0">
            {primarySettingsNav.map((item) => {
              const Icon = item.icon;
              const active = settingsSection === item.section;
              return (
                <Link
                  key={item.section}
                  id={selectorId("root-sidebar-settings", item.section)}
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
            settingsSection === "indexers" ||
            settingsSection === "downloadClients" ||
            settingsSection === "notifications" ||
            settingsSection === "subtitles"
              ? referenceRailDocked
                ? isSubtitlesSection
                  ? "max-w-[1340px]"
                  : "max-w-[1700px]"
                : "max-w-[1280px]"
              : settingsSection === "rules" ||
                  settingsSection === "post-processing"
                ? "max-w-none"
                : "max-w-[1280px]",
          )}
        >
          <div
            className={cn(
              showReferenceRail && referenceRailDocked
                ? "flex items-start gap-5"
                : "contents",
            )}
          >
            <div
              className={cn(
                showReferenceRail && referenceRailDocked
                  ? isSubtitlesSection
                    ? "min-w-0 flex-[1_1_auto]"
                    : "min-w-0 w-[1280px] shrink-0"
                  : showReferenceRail
                    ? "min-w-0"
                  : "contents",
              )}
            >
          <div className="mb-4 flex items-center gap-1.5 text-[12.5px] text-[var(--scry-faint)]">
            <span>{breadcrumbRootLabel}</span>
            <ChevronRight className="h-3.5 w-3.5" />
            <span className="font-semibold text-[var(--scry-accent-text)]">{settingsSectionLabel}</span>
          </div>
          <div
            className={cn(
              "mb-6 flex gap-3",
              showReferenceRail && !referenceRailDocked
                ? "flex-row items-center justify-between"
                : "flex-col sm:flex-row sm:items-center sm:justify-between",
            )}
          >
            <div className="flex min-w-0 items-center gap-4">
              <div className="flex h-[46px] w-[46px] shrink-0 items-center justify-center rounded-[13px] border border-[var(--scry-baccent)] bg-[linear-gradient(135deg,rgba(var(--scry-accent-rgb),0.35),rgba(123,91,255,0.22))] text-[var(--scry-accent-text)]">
                <SettingsSectionIcon className="h-[23px] w-[23px]" />
              </div>
              <div className="min-w-0">
                <h1 className="text-[25px] font-bold tracking-normal text-[var(--scry-ink2)]">
                  {settingsSectionLabel}
                </h1>
                {!usesAutomationHeader &&
                !usesIntegrationsHeader &&
                !usesSystemHeader &&
                settingsSection !== "profile" &&
                settingsSection !== "general" &&
                settingsSection !== "qualityProfiles" &&
                settingsSection !== "delayProfiles" &&
                settingsSection !== "plugins" &&
                settingsSection !== "subtitles" ? (
                  <p className="mt-1 max-w-[640px] text-[13.5px] text-[var(--scry-muted)]">
                    {t("settings.sectionTitle", { section: settingsSectionLabel })}
                  </p>
                ) : null}
              </div>
            </div>
            {showReferenceRail && !referenceRailDocked ? (
              <Button
                type="button"
                variant="primary"
                className="h-10 w-auto shrink-0 self-start rounded-[10px] px-3 text-[13px]"
                onClick={() => setReferenceRailOpen(true)}
                aria-expanded={referenceRailOpen}
                aria-controls={SETTINGS_REFERENCE_SLOT_ID}
              >
                <Puzzle className="h-4 w-4" />
                {t("settings.plugins")}
              </Button>
            ) : settingsSection === "plugins" ? (
              <div
                id={SETTINGS_HEADER_ACTIONS_SLOT_ID}
                className="flex min-h-10 shrink-0 flex-wrap items-center justify-end gap-2 sm:min-w-[29rem]"
              />
            ) : showPluginsShortcut ? (
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
            {showReferenceRail ? (
              <>
                {!referenceRailDocked ? (
                  <button
                    type="button"
                    aria-label={t("label.close")}
                    className={cn(
                      "fixed inset-0 z-40 bg-black/45 backdrop-blur-[2px] transition-opacity",
                      referenceRailOpen
                        ? "opacity-100"
                        : "pointer-events-none opacity-0",
                    )}
                    onClick={() => setReferenceRailOpen(false)}
                  />
                ) : null}
                <aside
                  aria-label={t("settings.plugins")}
                  className={cn(
                    referenceRailDocked
                      ? isSubtitlesSection
                        ? "sticky top-[26px] z-auto w-[320px] shrink-0"
                        : "sticky top-[26px] z-auto min-w-[320px] max-w-[400px] flex-[1_1_400px]"
                      : "fixed bottom-4 right-4 top-[118px] z-50 flex w-[min(420px,calc(100vw-2rem))] min-w-0 flex-col gap-3 overflow-y-auto rounded-[16px] border border-[var(--scry-border)] bg-[var(--scry-bg)] p-3 shadow-[0_20px_50px_rgba(0,0,0,0.38)] transition duration-200",
                    !referenceRailDocked &&
                      (referenceRailOpen
                        ? "translate-x-0 opacity-100"
                        : "pointer-events-none translate-x-[calc(100%+1rem)] opacity-0"),
                  )}
                >
                  {!referenceRailDocked ? (
                    <div className="flex items-center justify-between gap-3 rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-bg)] px-3 py-2 shadow-[0_10px_24px_rgba(0,0,0,0.16)]">
                      <div className="flex min-w-0 items-center gap-2 text-[15px] font-semibold text-[var(--scry-ink2)]">
                        <Puzzle className="h-4 w-4 shrink-0" />
                        <span className="truncate">{t("settings.plugins")}</span>
                      </div>
                      <IconButton
                        label={t("label.close")}
                        tone="neutral"
                        onClick={() => setReferenceRailOpen(false)}
                      >
                        <X className="h-4 w-4" />
                      </IconButton>
                    </div>
                  ) : null}
                  <div
                    id={SETTINGS_REFERENCE_SLOT_ID}
                    data-slot="settings-reference"
                    className="min-w-0"
                  />
                </aside>
              </>
            ) : null}
          </div>
        </div>
      </main>
    </div>
  );
});
