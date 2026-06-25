import * as React from "react";
import { SettingsOverviewSection } from "@/components/views/settings/settings-overview-section";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { generalSettingsQuery } from "@/lib/graphql/queries";
import {
  rehydrateAllMetadataMutation,
  setMyUiSettingsMutation,
  updateGeneralSettingsMutation,
} from "@/lib/graphql/mutations";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import {
  uiSettingsInputFromSettings,
  useUiSettings,
} from "@/lib/context/ui-settings-context";
import type { LocaleCode, LanguageOption } from "@/lib/i18n";
import type { GeneralSettings, UiDateTimeFormat, UiSettings } from "@/lib/types/settings";

const DEFAULT_GENERAL_SETTINGS: GeneralSettings = {
  keepHistoryForever: false,
  historyRetentionDays: 180,
  pluginHttpCaBundlePem: "",
  pluginHttpTrustedCertificates: [],
};

type SettingsOverviewContainerProps = {
  availableLanguages: LanguageOption[];
  selectedLanguage: LanguageOption | null;
  uiLanguage: LocaleCode;
  onSelectLanguage: (code: string) => void;
};

export function SettingsOverviewContainer({
  availableLanguages,
  selectedLanguage,
  uiLanguage,
  onSelectLanguage,
}: SettingsOverviewContainerProps) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const { uiSettings, uiSettingsLoading, setUiSettings } = useUiSettings();
  const [pendingLanguage, setPendingLanguage] = React.useState<string | null>(null);
  const [rehydrating, setRehydrating] = React.useState(false);
  const [uiSettingsSaving, setUiSettingsSaving] = React.useState(false);
  const [generalSettings, setGeneralSettings] = React.useState<GeneralSettings>(
    DEFAULT_GENERAL_SETTINGS,
  );
  const [generalLoading, setGeneralLoading] = React.useState(true);
  const [generalSaving, setGeneralSaving] = React.useState(false);

  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const { data, error } = await client.query(generalSettingsQuery, {}).toPromise();
        if (error) throw error;
        if (cancelled) return;
        setGeneralSettings({
          ...DEFAULT_GENERAL_SETTINGS,
          ...data?.generalSettings,
        });
      } catch {
        if (!cancelled) {
          setGeneralSettings(DEFAULT_GENERAL_SETTINGS);
        }
      } finally {
        if (!cancelled) setGeneralLoading(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [client]);

  const handleLanguageSelect = React.useCallback((code: string) => {
    if (code === uiLanguage) return;
    setPendingLanguage(code);
  }, [uiLanguage]);

  const handleConfirmLanguageChange = React.useCallback(async () => {
    if (!pendingLanguage) return;
    setRehydrating(true);
    try {
      // Change UI language immediately
      onSelectLanguage(pendingLanguage);

      // Trigger backend metadata rehydration
      const { error } = await client.mutation(
        rehydrateAllMetadataMutation,
        { input: { language: pendingLanguage } },
      ).toPromise();

      if (error) {
        setGlobalStatus(error.message);
      } else {
        setGlobalStatus(t("settings.metadataRehydrationStarted"));
      }
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToUpdate"),
      );
    } finally {
      setRehydrating(false);
      setPendingLanguage(null);
    }
  }, [client, onSelectLanguage, pendingLanguage, setGlobalStatus, t]);

  const pendingLanguageLabel = pendingLanguage
    ? availableLanguages.find((l) => l.code === pendingLanguage)?.label ?? pendingLanguage
    : "";

  const handleDateTimeFormatChange = React.useCallback(
    async (dateTimeFormat: UiDateTimeFormat) => {
      if (dateTimeFormat === uiSettings.dateTimeFormat || uiSettingsSaving) return;

      const nextSettings: UiSettings = {
        ...uiSettings,
        dateTimeFormat,
      };
      setUiSettingsSaving(true);
      try {
        const { data, error } = await client
          .mutation<{ setMyUiSettings: UiSettings }>(setMyUiSettingsMutation, {
            input: uiSettingsInputFromSettings(nextSettings),
          })
          .toPromise();
        if (error) throw error;
        setUiSettings(data?.setMyUiSettings ?? nextSettings);
        setGlobalStatus(t("settings.uiSaved"));
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setUiSettingsSaving(false);
      }
    },
    [
      client,
      setGlobalStatus,
      setUiSettings,
      t,
      uiSettings,
      uiSettingsSaving,
    ],
  );

  const handleSaveGeneralSettings = React.useCallback(async () => {
    if (!generalSettings.keepHistoryForever && generalSettings.historyRetentionDays < 1) {
      setGlobalStatus(t("settings.historyRetentionValidation"));
      return;
    }

    setGeneralSaving(true);
    try {
      const { data, error } = await client
        .mutation(updateGeneralSettingsMutation, {
          input: {
            keepHistoryForever: generalSettings.keepHistoryForever,
            historyRetentionDays: generalSettings.historyRetentionDays,
            pluginHttpCaBundlePem: generalSettings.pluginHttpCaBundlePem,
          },
        })
        .toPromise();
      if (error) throw error;
      setGeneralSettings({
        ...DEFAULT_GENERAL_SETTINGS,
        ...data?.updateGeneralSettings,
      });
      setGlobalStatus(t("settings.generalSaved"));
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToUpdate"),
      );
    } finally {
      setGeneralSaving(false);
    }
  }, [client, generalSettings, setGlobalStatus, t]);

  return (
    <>
      <SettingsOverviewSection
        availableLanguages={availableLanguages}
        selectedLanguage={selectedLanguage}
        uiLanguage={uiLanguage}
        onSelectLanguage={handleLanguageSelect}
        dateTimeFormat={uiSettings.dateTimeFormat}
        dateTimeFormatLoading={uiSettingsLoading}
        dateTimeFormatSaving={uiSettingsSaving}
        onDateTimeFormatChange={handleDateTimeFormatChange}
        generalSettings={generalSettings}
        onGeneralSettingsChange={setGeneralSettings}
        generalLoading={generalLoading}
        generalSaving={generalSaving}
        onSaveGeneralSettings={handleSaveGeneralSettings}
      />
      <ConfirmDialog
        open={pendingLanguage !== null}
        title={t("settings.languageChangeTitle")}
        description={t("settings.languageChangeWarning", { language: pendingLanguageLabel })}
        confirmLabel={t("settings.languageChangeConfirm")}
        cancelLabel={t("label.cancel")}
        isBusy={rehydrating}
        onConfirm={handleConfirmLanguageChange}
        onCancel={() => setPendingLanguage(null)}
      />
    </>
  );
}
