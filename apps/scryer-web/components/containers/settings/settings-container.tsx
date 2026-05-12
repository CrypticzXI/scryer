
import { memo, useCallback, useState } from "react";
import { Link } from "react-router-dom";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { SettingsOverviewContainer } from "@/components/containers/settings/settings-overview-container";
import { SettingsSecurityContainer } from "@/components/containers/settings/settings-security-container";
import { SettingsUsersContainer } from "@/components/containers/settings/settings-users-container";
import { SettingsIndexersContainer } from "@/components/containers/settings/settings-indexers-container";
import { SettingsDownloadClientsContainer } from "@/components/containers/settings/settings-download-clients-container";
import { SettingsDelayProfilesContainer } from "@/components/containers/settings/settings-delay-profiles-container";
import { SettingsQualityProfilesContainer } from "@/components/containers/settings/settings-quality-profiles-container";
import { SettingsAcquisitionContainer } from "@/components/containers/settings/settings-acquisition-container";
import { SettingsProfileContainer } from "@/components/containers/settings/settings-profile-container";
import { SettingsRulesContainer } from "@/components/containers/settings/settings-rules-container";
import { SettingsPluginsContainer } from "@/components/containers/settings/settings-plugins-container";
import { SettingsNotificationsContainer } from "@/components/containers/settings/settings-notifications-container";
import { SettingsPostProcessingContainer } from "@/components/containers/settings/settings-post-processing-container";
import { SettingsSubtitlesContainer } from "@/components/containers/settings/settings-subtitles-container";
import { SettingsRecycleBinContainer } from "@/components/containers/settings/settings-recycle-bin-container";
import { SettingsBackupsContainer } from "@/components/containers/settings/settings-backups-container";
import type { SettingsSection } from "@/components/root/types";
import type { LocaleCode, LanguageOption } from "@/lib/i18n";
import { useTranslate } from "@/lib/context/translate-context";
import {
  type ProviderCatalogFamily,
  useProviderCatalogSubscription,
} from "@/lib/hooks/use-provider-catalog-subscription";

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
      </CardContent>
    </Card>
  );
});
