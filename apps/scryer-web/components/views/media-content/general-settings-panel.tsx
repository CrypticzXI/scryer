import { useTranslate } from "@/lib/context/translate-context";
import { SettingsToggleSwitch } from "@/components/common/settings-toggle-switch";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { ImportMode } from "@/lib/types/settings";
import type { ViewCategoryId } from "./indexer-category-picker";

const FILLER_POLICY_OPTIONS = [
  { value: "download_all", label: "settings.fillerPolicyDownloadAll" },
  { value: "skip_filler", label: "settings.fillerPolicySkipFiller" },
];

const RECAP_POLICY_OPTIONS = [
  { value: "download_all", label: "settings.recapPolicyDownloadAll" },
  { value: "skip_recap", label: "settings.recapPolicySkipRecap" },
];

const IMPORT_MODE_OPTIONS: { value: ImportMode; label: string }[] = [
  { value: "hardlink_or_copy", label: "settings.importModeHardlinkCopy" },
  { value: "move", label: "settings.importModeMove" },
];

export function GeneralSettingsPanel({
  activeQualityScopeId,
  mediaSettingsLoading,
  categoryFillerPolicies,
  handleFillerPolicyChange,
  categoryRecapPolicies,
  handleRecapPolicyChange,
  categoryMonitorSpecials,
  handleMonitorSpecialsChange,
  categoryInterSeasonMovies,
  handleInterSeasonMoviesChange,
  categoryMonitorFillerMovies,
  handleMonitorFillerMoviesChange,
  nfoWriteOnImport,
  handleNfoWriteChange,
  plexmatchWriteOnImport,
  handlePlexmatchWriteChange,
  importMode,
  handleImportModeChange,
}: {
  activeQualityScopeId: ViewCategoryId;
  mediaSettingsLoading: boolean;
  categoryFillerPolicies: Record<ViewCategoryId, string>;
  handleFillerPolicyChange: (value: string) => void;
  categoryRecapPolicies: Record<ViewCategoryId, string>;
  handleRecapPolicyChange: (value: string) => void;
  categoryMonitorSpecials: Record<ViewCategoryId, string>;
  handleMonitorSpecialsChange: (checked: boolean) => void;
  categoryInterSeasonMovies: Record<ViewCategoryId, string>;
  handleInterSeasonMoviesChange: (checked: boolean) => void;
  categoryMonitorFillerMovies: Record<ViewCategoryId, string>;
  handleMonitorFillerMoviesChange: (checked: boolean) => void;
  nfoWriteOnImport: Record<ViewCategoryId, string>;
  handleNfoWriteChange: (checked: boolean) => void;
  plexmatchWriteOnImport: Record<ViewCategoryId, string>;
  handlePlexmatchWriteChange: (checked: boolean) => void;
  importMode: Record<ViewCategoryId, ImportMode>;
  handleImportModeChange: (value: ImportMode) => void;
}) {
  const t = useTranslate();
  const showAnimePolicies = activeQualityScopeId === "anime";
  const showPlexmatch =
    activeQualityScopeId === "series" || activeQualityScopeId === "anime";

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle>{t("facetSettings.general")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="max-w-md space-y-2">
            <Label className="text-sm text-card-foreground">
              {t("settings.importModeLabel")}
            </Label>
            <Select
              value={importMode[activeQualityScopeId]}
              onValueChange={(value) => handleImportModeChange(value as ImportMode)}
              disabled={mediaSettingsLoading}
            >
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {IMPORT_MODE_OPTIONS.map((option) => (
                  <SelectItem key={option.value} value={option.value}>
                    {t(option.label)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">
              {t("settings.importModeDescription")}
            </p>
          </div>
          <div>
            <h3 className="text-sm font-semibold text-card-foreground">
              {t("facetSettings.sidecarFiles")}
            </h3>
          </div>
          <div className="grid gap-4 md:grid-cols-2">
            <div className="space-y-2">
              <Label className="text-sm text-card-foreground">
                {t("settings.nfoWriteOnImportLabel")}
              </Label>
              <div className="flex items-center gap-3">
                <SettingsToggleSwitch
                  checked={nfoWriteOnImport[activeQualityScopeId] === "true"}
                  ariaLabel={t("settings.nfoWriteOnImportLabel")}
                  disabled={mediaSettingsLoading}
                  onChange={(nextValue) => handleNfoWriteChange(nextValue)}
                />
                <span className="text-xs text-muted-foreground">
                  {t("settings.nfoWriteOnImportDescription")}
                </span>
              </div>
            </div>
            {showPlexmatch ? (
              <div className="space-y-2">
                <Label className="text-sm text-card-foreground">
                  {t("settings.plexmatchWriteOnImportLabel")}
                </Label>
                <div className="flex items-center gap-3">
                  <SettingsToggleSwitch
                    checked={plexmatchWriteOnImport[activeQualityScopeId] === "true"}
                    ariaLabel={t("settings.plexmatchWriteOnImportLabel")}
                    disabled={mediaSettingsLoading}
                    onChange={(nextValue) => handlePlexmatchWriteChange(nextValue)}
                  />
                  <span className="text-xs text-muted-foreground">
                    {t("settings.plexmatchWriteOnImportDescription")}
                  </span>
                </div>
              </div>
            ) : null}
          </div>
        </CardContent>
      </Card>

      {showAnimePolicies ? (
        <Card>
          <CardHeader>
            <CardTitle>{t("facetSettings.generalPolicies")}</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="grid gap-4 md:grid-cols-2">
              <label className="space-y-2">
                <Label className="text-sm text-card-foreground">
                  {t("settings.fillerPolicyLabel")}
                </Label>
                <Select
                  value={categoryFillerPolicies[activeQualityScopeId]}
                  onValueChange={handleFillerPolicyChange}
                  disabled={mediaSettingsLoading}
                >
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {FILLER_POLICY_OPTIONS.map((option) => (
                      <SelectItem key={option.value} value={option.value}>
                        {t(option.label)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
              <label className="space-y-2">
                <Label className="text-sm text-card-foreground">
                  {t("settings.recapPolicyLabel")}
                </Label>
                <Select
                  value={categoryRecapPolicies[activeQualityScopeId]}
                  onValueChange={handleRecapPolicyChange}
                  disabled={mediaSettingsLoading}
                >
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {RECAP_POLICY_OPTIONS.map((option) => (
                      <SelectItem key={option.value} value={option.value}>
                        {t(option.label)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
              <div className="space-y-2">
                <Label className="text-sm text-card-foreground">
                  {t("settings.monitorSpecialsLabel")}
                </Label>
                <div className="flex items-center gap-3">
                  <SettingsToggleSwitch
                    checked={categoryMonitorSpecials[activeQualityScopeId] !== "false"}
                    ariaLabel={t("settings.monitorSpecialsLabel")}
                    disabled={mediaSettingsLoading}
                    onChange={(nextValue) => handleMonitorSpecialsChange(nextValue)}
                  />
                  <span className="text-xs text-muted-foreground">
                    {t("settings.monitorSpecialsDescription")}
                  </span>
                </div>
              </div>
              <div className="space-y-2">
                <Label className="text-sm text-card-foreground">
                  {t("settings.interSeasonMoviesLabel")}
                </Label>
                <div className="flex items-center gap-3">
                  <SettingsToggleSwitch
                    checked={categoryInterSeasonMovies[activeQualityScopeId] !== "false"}
                    ariaLabel={t("settings.interSeasonMoviesLabel")}
                    disabled={mediaSettingsLoading}
                    onChange={(nextValue) => handleInterSeasonMoviesChange(nextValue)}
                  />
                  <span className="text-xs text-muted-foreground">
                    {t("settings.interSeasonMoviesDescription")}
                  </span>
                </div>
              </div>
              <div className="space-y-2">
                <Label className="text-sm text-card-foreground">
                  {t("settings.monitorFillerMoviesLabel")}
                </Label>
                <div className="flex items-center gap-3">
                  <SettingsToggleSwitch
                    checked={categoryMonitorFillerMovies[activeQualityScopeId] === "true"}
                    ariaLabel={t("settings.monitorFillerMoviesLabel")}
                    disabled={mediaSettingsLoading}
                    onChange={(nextValue) => handleMonitorFillerMoviesChange(nextValue)}
                  />
                  <span className="text-xs text-muted-foreground">
                    {t("settings.monitorFillerMoviesDescription")}
                  </span>
                </div>
              </div>
            </div>
          </CardContent>
        </Card>
      ) : null}
    </div>
  );
}
