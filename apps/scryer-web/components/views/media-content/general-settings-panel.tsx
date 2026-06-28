import { useEffect, useState, type ComponentType, type ReactNode } from "react";
import { FileText, Import as ImportIcon, SlidersVertical } from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import { SettingsToggleSwitch } from "@/components/common/settings-toggle-switch";
import { Input } from "@/components/ui/input";
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

function chmodDraftIsValid(value: string): boolean {
  const trimmed = value.trim();
  return trimmed === "" || /^[0-7]{3,4}$/.test(trimmed);
}

type SectionIcon = ComponentType<{ className?: string }>;

function GeneralSettingsSection({
  icon: Icon,
  title,
  description,
  children,
}: {
  icon: SectionIcon;
  title: string;
  description?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="rounded-[16px] border border-[var(--scry-border)] bg-[var(--scry-surf)] p-5 sm:p-6">
      <div className="flex items-center gap-2.5">
        <Icon className="h-[17px] w-[17px] text-[var(--scry-accent-text)]" />
        <h2 className="text-[16px] font-bold text-[var(--scry-ink2)]">{title}</h2>
      </div>
      {description ? (
        <p className="mt-1.5 text-[12.5px] leading-relaxed text-[var(--scry-muted3)]">
          {description}
        </p>
      ) : null}
      <div className="mt-5 space-y-5">{children}</div>
    </section>
  );
}

function ToggleSettingRow({
  label,
  description,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  description: string;
  checked: boolean;
  disabled: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-4 rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)] p-4">
      <div className="min-w-0">
        <Label className="text-[13.5px] font-semibold text-[var(--scry-body)]">
          {label}
        </Label>
        <p className="mt-1 text-xs leading-relaxed text-[var(--scry-muted3)]">
          {description}
        </p>
      </div>
      <SettingsToggleSwitch
        checked={checked}
        ariaLabel={label}
        disabled={disabled}
        onChange={onChange}
      />
    </div>
  );
}

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
  setPermissionsLinux,
  handleSetPermissionsLinuxChange,
  fileChmod,
  handleFileChmodChange,
  folderChmod,
  handleFolderChmodChange,
  chownGroup,
  handleChownGroupChange,
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
  setPermissionsLinux: Record<ViewCategoryId, string>;
  handleSetPermissionsLinuxChange: (checked: boolean) => void;
  fileChmod: Record<ViewCategoryId, string>;
  handleFileChmodChange: (value: string) => void;
  folderChmod: Record<ViewCategoryId, string>;
  handleFolderChmodChange: (value: string) => void;
  chownGroup: Record<ViewCategoryId, string>;
  handleChownGroupChange: (value: string) => void;
}) {
  const t = useTranslate();
  const showAnimePolicies = activeQualityScopeId === "anime";
  const showPlexmatch =
    activeQualityScopeId === "series" || activeQualityScopeId === "anime";
  const [fileChmodDraft, setFileChmodDraft] = useState(
    fileChmod[activeQualityScopeId] ?? "",
  );
  const [folderChmodDraft, setFolderChmodDraft] = useState(
    folderChmod[activeQualityScopeId] ?? "",
  );
  const [chownGroupDraft, setChownGroupDraft] = useState(
    chownGroup[activeQualityScopeId] ?? "",
  );
  const [fileChmodError, setFileChmodError] = useState<string | null>(null);
  const [folderChmodError, setFolderChmodError] = useState<string | null>(null);

  useEffect(() => {
    setFileChmodDraft(fileChmod[activeQualityScopeId] ?? "");
    setFileChmodError(null);
  }, [activeQualityScopeId, fileChmod]);

  useEffect(() => {
    setFolderChmodDraft(folderChmod[activeQualityScopeId] ?? "");
    setFolderChmodError(null);
  }, [activeQualityScopeId, folderChmod]);

  useEffect(() => {
    setChownGroupDraft(chownGroup[activeQualityScopeId] ?? "");
  }, [activeQualityScopeId, chownGroup]);

  const commitFileChmod = () => {
    if (!chmodDraftIsValid(fileChmodDraft)) {
      setFileChmodError(t("settings.chmodValidation"));
      return;
    }
    const normalized = fileChmodDraft.trim();
    setFileChmodError(null);
    if (normalized !== (fileChmod[activeQualityScopeId] ?? "")) {
      handleFileChmodChange(normalized);
    }
  };

  const commitFolderChmod = () => {
    if (!chmodDraftIsValid(folderChmodDraft)) {
      setFolderChmodError(t("settings.chmodValidation"));
      return;
    }
    const normalized = folderChmodDraft.trim();
    setFolderChmodError(null);
    if (normalized !== (folderChmod[activeQualityScopeId] ?? "")) {
      handleFolderChmodChange(normalized);
    }
  };

  const commitChownGroup = () => {
    const normalized = chownGroupDraft.trim();
    if (normalized !== (chownGroup[activeQualityScopeId] ?? "")) {
      handleChownGroupChange(normalized);
    }
  };

  return (
    <div className="space-y-[18px]">
      <GeneralSettingsSection
        icon={ImportIcon}
        title={t("settings.importBehaviorTitle")}
        description={t("settings.importModeDescription")}
      >
        <div className="max-w-xl space-y-2">
          <Label>{t("settings.importModeLabel")}</Label>
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
        </div>
        <div className="grid max-w-xl gap-4 sm:grid-cols-2">
          <ToggleSettingRow
            label={t("settings.setPermissionsLinuxLabel")}
            description={t("settings.setPermissionsLinuxDescription")}
            checked={setPermissionsLinux[activeQualityScopeId] === "true"}
            disabled={mediaSettingsLoading}
            onChange={handleSetPermissionsLinuxChange}
          />
          <div className="space-y-2">
            <Label>{t("settings.fileChmodLabel")}</Label>
            <Input
              value={fileChmodDraft}
              onChange={(event) => {
                setFileChmodDraft(event.target.value);
                if (fileChmodError) {
                  setFileChmodError(null);
                }
              }}
              onBlur={commitFileChmod}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  commitFileChmod();
                }
              }}
              disabled={mediaSettingsLoading}
              placeholder={t("label.none")}
              aria-invalid={fileChmodError ? true : undefined}
            />
            {fileChmodError ? (
              <p className="text-xs text-destructive">{fileChmodError}</p>
            ) : null}
          </div>
          <div className="space-y-2">
            <Label>{t("settings.folderChmodLabel")}</Label>
            <Input
              value={folderChmodDraft}
              onChange={(event) => {
                setFolderChmodDraft(event.target.value);
                if (folderChmodError) {
                  setFolderChmodError(null);
                }
              }}
              onBlur={commitFolderChmod}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  commitFolderChmod();
                }
              }}
              disabled={mediaSettingsLoading}
              placeholder="755"
              aria-invalid={folderChmodError ? true : undefined}
            />
            {folderChmodError ? (
              <p className="text-xs text-destructive">{folderChmodError}</p>
            ) : null}
          </div>
          <div className="space-y-2">
            <Label>{t("settings.chownGroupLabel")}</Label>
            <Input
              value={chownGroupDraft}
              onChange={(event) => setChownGroupDraft(event.target.value)}
              onBlur={commitChownGroup}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  commitChownGroup();
                }
              }}
              disabled={mediaSettingsLoading}
              placeholder={t("label.none")}
            />
          </div>
        </div>
      </GeneralSettingsSection>

      <GeneralSettingsSection
        icon={FileText}
        title={t("settings.sidecarFilesTitle")}
      >
        <div
          className={`grid gap-3 ${showPlexmatch ? "md:grid-cols-2" : "md:grid-cols-1"}`}
        >
          <ToggleSettingRow
            label={t("settings.nfoWriteOnImportLabel")}
            description={t("settings.nfoWriteOnImportDescription")}
            checked={nfoWriteOnImport[activeQualityScopeId] === "true"}
            disabled={mediaSettingsLoading}
            onChange={handleNfoWriteChange}
          />
          {showPlexmatch ? (
            <ToggleSettingRow
              label={t("settings.plexmatchWriteOnImportLabel")}
              description={t("settings.plexmatchWriteOnImportDescription")}
              checked={plexmatchWriteOnImport[activeQualityScopeId] === "true"}
              disabled={mediaSettingsLoading}
              onChange={handlePlexmatchWriteChange}
            />
          ) : null}
        </div>
      </GeneralSettingsSection>

      {showAnimePolicies ? (
        <GeneralSettingsSection
          icon={SlidersVertical}
          title={t("settings.animeSettings")}
        >
          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
            <div className="space-y-2">
              <Label>
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
            </div>

            <div className="space-y-2">
              <Label>
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
            </div>
          </div>

          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
            <ToggleSettingRow
              label={t("settings.monitorSpecialsLabel")}
              description={t("settings.monitorSpecialsDescription")}
              checked={categoryMonitorSpecials[activeQualityScopeId] !== "false"}
              disabled={mediaSettingsLoading}
              onChange={handleMonitorSpecialsChange}
            />
            <ToggleSettingRow
              label={t("settings.interSeasonMoviesLabel")}
              description={t("settings.interSeasonMoviesDescription")}
              checked={categoryInterSeasonMovies[activeQualityScopeId] !== "false"}
              disabled={mediaSettingsLoading}
              onChange={handleInterSeasonMoviesChange}
            />
            <ToggleSettingRow
              label={t("settings.monitorFillerMoviesLabel")}
              description={t("settings.monitorFillerMoviesDescription")}
              checked={categoryMonitorFillerMovies[activeQualityScopeId] === "true"}
              disabled={mediaSettingsLoading}
              onChange={handleMonitorFillerMoviesChange}
            />
          </div>
        </GeneralSettingsSection>
      ) : null}
    </div>
  );
}
