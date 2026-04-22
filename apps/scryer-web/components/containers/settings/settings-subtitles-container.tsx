import * as React from "react";
import { useClient } from "urql";
import { SettingsSubtitlesSection } from "@/components/views/settings/settings-subtitles-section";
import { subtitleSettingsQuery } from "@/lib/graphql/queries";
import { updateSubtitleSettingsMutation } from "@/lib/graphql/mutations";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import type { SubtitleSettings } from "@/lib/types/settings";
import { toast } from "sonner";

const DEFAULTS: SubtitleSettings = {
  enabled: false,
  hasOpenSubtitlesApiKey: false,
  openSubtitlesUsername: "",
  hasOpenSubtitlesPassword: false,
  languages: [],
  autoDownloadOnImport: false,
  minimumScoreSeries: 240,
  minimumScoreMovie: 70,
  searchIntervalHours: 6,
  includeAiTranslated: false,
  includeMachineTranslated: false,
  syncEnabled: true,
  syncThresholdSeries: 90,
  syncThresholdMovie: 70,
  syncMaxOffsetSeconds: 60,
};

export function SettingsSubtitlesContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [settings, setSettings] = React.useState<SubtitleSettings>(DEFAULTS);
  const [saving, setSaving] = React.useState(false);
  const [loading, setLoading] = React.useState(true);
  const [passwordDraft, setPasswordDraft] = React.useState("");
  const [passwordTouched, setPasswordTouched] = React.useState(false);
  const loadedRef = React.useRef(false);
  const skipAutosaveRef = React.useRef(false);
  const savedUsernameRef = React.useRef("");

  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const { data, error } = await client.query(subtitleSettingsQuery, {}).toPromise();
        if (error) throw error;
        if (cancelled) return;
        const payload = data?.subtitleSettings;
        if (!payload) return;
        setSettings({
          ...DEFAULTS,
          ...payload,
        });
        savedUsernameRef.current = payload.openSubtitlesUsername ?? "";
        setPasswordDraft("");
        setPasswordTouched(false);
        loadedRef.current = true;
      } catch {
        // Use defaults on failure
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, [client]);

  // Auto-save on change (skip initial load)
  React.useEffect(() => {
    if (!loadedRef.current) return;
    if (skipAutosaveRef.current) {
      skipAutosaveRef.current = false;
      return;
    }
    const usernameChanged = settings.openSubtitlesUsername !== savedUsernameRef.current;
    const credentialSaveRequested = usernameChanged || passwordTouched;
    setSaving(true);
    client
      .mutation(updateSubtitleSettingsMutation, {
        input: {
          enabled: settings.enabled,
          openSubtitlesUsername: settings.openSubtitlesUsername,
          ...(passwordTouched ? { openSubtitlesPassword: passwordDraft } : {}),
          languages: settings.languages.map((language) => ({
            code: language.code,
            hearingImpaired: language.hearingImpaired,
            forced: language.forced,
          })),
          autoDownloadOnImport: settings.autoDownloadOnImport,
          minimumScoreSeries: settings.minimumScoreSeries,
          minimumScoreMovie: settings.minimumScoreMovie,
          searchIntervalHours: settings.searchIntervalHours,
          includeAiTranslated: settings.includeAiTranslated,
          includeMachineTranslated: settings.includeMachineTranslated,
          syncEnabled: settings.syncEnabled,
          syncThresholdSeries: settings.syncThresholdSeries,
          syncThresholdMovie: settings.syncThresholdMovie,
          syncMaxOffsetSeconds: settings.syncMaxOffsetSeconds,
        },
      })
      .toPromise()
      .then(({ data, error }) => {
        if (error) {
          const message = error.message || t("status.failedToUpdate");
          setGlobalStatus(message);
          toast.error(message);
          return;
        }

        const nextPayload = data?.updateSubtitleSettings;
        savedUsernameRef.current =
          nextPayload?.openSubtitlesUsername ?? settings.openSubtitlesUsername;

        if (passwordTouched) {
          skipAutosaveRef.current = true;
          setSettings((previous) => ({
            ...previous,
            hasOpenSubtitlesPassword:
              passwordTouched
                ? nextPayload?.hasOpenSubtitlesPassword ?? passwordDraft.trim().length > 0
                : previous.hasOpenSubtitlesPassword,
          }));
          setPasswordDraft("");
          setPasswordTouched(false);
        }

        if (
          credentialSaveRequested &&
          nextPayload?.hasOpenSubtitlesApiKey &&
          (nextPayload?.openSubtitlesUsername?.trim().length ?? 0) > 0 &&
          nextPayload?.hasOpenSubtitlesPassword
        ) {
          toast.success(t("settings.sub.credentialsVerified"));
        }
      })
      .catch((error: unknown) => {
        const message =
          error instanceof Error ? error.message : t("status.failedToUpdate");
        setGlobalStatus(message);
        toast.error(message);
      })
      .finally(() => setSaving(false));
  }, [
    client,
    passwordDraft,
    passwordTouched,
    setGlobalStatus,
    settings,
    t,
  ]);

  return (
    <SettingsSubtitlesSection
      settings={settings}
      setSettings={setSettings}
      passwordDraft={passwordDraft}
      onPasswordCommit={(value) => {
        setPasswordDraft(value);
        setPasswordTouched(true);
      }}
      saving={saving}
      loading={loading}
    />
  );
}
