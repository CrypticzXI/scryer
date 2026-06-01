
import { lazy, memo, Suspense, useCallback, useState } from "react";
import { Link } from "react-router-dom";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import type { SettingsSection } from "@/components/root/types";
import type { LocaleCode, LanguageOption } from "@/lib/i18n";
import { useTranslate } from "@/lib/context/translate-context";
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
const SettingsAcquisitionContainer = lazy(async () => ({
  default: (await import("@/components/containers/settings/settings-acquisition-container")).SettingsAcquisitionContainer,
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
  availableLanguages: LanguageOption[];
  selectedLanguage: LanguageOption | null;
  uiLanguage: LocaleCode;
  onSelectLanguage: (code: string) => void;
};

export const SettingsContainer = memo(function SettingsContainer({
  settingsSection,
  userId,
  username,
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
    <Card className="bg-card border-border">
      <CardHeader className="flex items-center justify-between gap-3">
        <CardTitle>
          {t("settings.sectionTitle", {
            section:
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
                : settingsSection === "acquisition"
                  ? t("settings.acquisition")
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
                    : t("settings.qualityProfiles"),
          })}
        </CardTitle>
        {showPluginsLink ? (
          <Button asChild variant="primary" className="shrink-0">
            <Link to="/settings/plugins">{t("settings.plugins")}</Link>
          </Button>
        ) : null}
      </CardHeader>
      <CardContent>
        <Suspense fallback={<div className="py-6 text-sm text-muted-foreground">{t("label.loading")}</div>}>
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
          ) : settingsSection === "acquisition" ? (
            <SettingsAcquisitionContainer />
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
      </CardContent>
    </Card>
  );
});
