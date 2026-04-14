import { Rocket } from "lucide-react";
import { Link } from "react-router-dom";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import type { LocaleCode, LanguageOption } from "@/lib/i18n";

type SettingsOverviewSectionProps = {
  availableLanguages: LanguageOption[];
  selectedLanguage: LanguageOption | null;
  uiLanguage: LocaleCode;
  onSelectLanguage: (code: string) => void;
};

export function SettingsOverviewSection({
  availableLanguages,
  uiLanguage,
  onSelectLanguage,
}: SettingsOverviewSectionProps) {
  const t = useTranslate();
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
