import { Loader2, Rocket } from "lucide-react";
import { Link } from "react-router-dom";
import { Button } from "@/components/ui/button";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import type { LocaleCode, LanguageOption } from "@/lib/i18n";
import type { GeneralSettings } from "@/lib/types/settings";

type SettingsOverviewSectionProps = {
  availableLanguages: LanguageOption[];
  selectedLanguage: LanguageOption | null;
  uiLanguage: LocaleCode;
  onSelectLanguage: (code: string) => void;
  generalSettings: GeneralSettings;
  onGeneralSettingsChange: (settings: GeneralSettings) => void;
  generalLoading: boolean;
  generalSaving: boolean;
  onSaveGeneralSettings: () => void;
};

export function SettingsOverviewSection({
  availableLanguages,
  uiLanguage,
  onSelectLanguage,
  generalSettings,
  onGeneralSettingsChange,
  generalLoading,
  generalSaving,
  onSaveGeneralSettings,
}: SettingsOverviewSectionProps) {
  const t = useTranslate();
  const updateGeneralSettings = (patch: Partial<GeneralSettings>) =>
    onGeneralSettingsChange({ ...generalSettings, ...patch });

  return (
    <div className="space-y-6 text-sm">
      <div>
        <p>{t("settings.generalText")}</p>
        <p>{t("settings.generalPlaceholder")}</p>
      </div>

      <div>
        <label className="mb-2 block text-xs font-medium uppercase tracking-wide text-muted-foreground">
          {t("label.language")}
        </label>
        <Select value={uiLanguage} onValueChange={onSelectLanguage}>
          <SelectTrigger className="w-56">
            <SelectValue placeholder={t("label.language")} />
          </SelectTrigger>
          <SelectContent>
            {availableLanguages.map((language) => (
              <SelectItem key={language.code} value={language.code}>
                {language.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div className="space-y-4 border-t border-border pt-6">
        <div className="space-y-1">
          <h3 className="text-sm font-semibold">{t("settings.historyRetentionTitle")}</h3>
          <p className="text-muted-foreground">
            {t("settings.historyRetentionHelp")}
          </p>
          <p className="text-muted-foreground">
            {t("settings.historyRetentionExternalHelp")}
          </p>
        </div>

        {generalLoading ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            {t("label.loading")}
          </div>
        ) : (
          <>
            <div className="flex items-center gap-3">
              <Label>{t("settings.keepHistoryForever")}</Label>
              <button
                type="button"
                role="switch"
                aria-checked={generalSettings.keepHistoryForever}
                className={`relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors ${generalSettings.keepHistoryForever ? "bg-primary" : "bg-muted"}`}
                onClick={() =>
                  updateGeneralSettings({
                    keepHistoryForever: !generalSettings.keepHistoryForever,
                  })}
              >
                <span
                  className={`pointer-events-none inline-block h-5 w-5 rounded-full bg-background shadow-lg transition-transform ${generalSettings.keepHistoryForever ? "translate-x-5" : "translate-x-0"}`}
                />
              </button>
            </div>

            <div className="space-y-1 max-w-xs">
              <Label>{t("settings.historyRetentionDaysLabel")}</Label>
              <Input
                {...integerInputProps}
                disabled={generalSettings.keepHistoryForever}
                value={generalSettings.historyRetentionDays}
                onChange={(event) => {
                  const nextValue = sanitizeDigits(event.target.value);
                  updateGeneralSettings({
                    historyRetentionDays: nextValue === "" ? 0 : Number(nextValue),
                  });
                }}
              />
            </div>

            <Button onClick={onSaveGeneralSettings} disabled={generalSaving}>
              {generalSaving ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  {t("label.saving")}
                </>
              ) : (
                t("settings.save")
              )}
            </Button>
          </>
        )}
      </div>

      <div className="border-t border-border pt-6">
        <Button asChild variant="primary" className="gap-2">
          <Link to={{ pathname: "/setup", search: "?reentry=1" }}>
            <Rocket className="h-4 w-4" />
            {t("settings.runSetupWizard")}
          </Link>
        </Button>
      </div>
    </div>
  );
}
