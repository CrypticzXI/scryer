import * as React from "react";
import { useTranslate } from "@/lib/context/translate-context";
import type { Translate } from "@/components/root/types";
import { Button } from "@/components/ui/button";
import { SettingsToggleSwitch } from "@/components/common/settings-toggle-switch";
import { InfoHelp } from "@/components/common/info-help";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  applyRenameTemplatePreview,
  validateRenameTemplateSyntax,
  type RenameTemplateValidationIssue,
} from "@/lib/utils/rename-template";
import type { ViewCategoryId } from "./indexer-category-picker";

type ParsedQualityProfile = {
  id: string;
  name: string;
};

type QualityProfileOption = {
  value: string;
  label: string;
};

// --- Constants ---

const RENAME_COLLISION_POLICY_OPTIONS = [
  { value: "skip", label: "settings.renameCollisionPolicySkip" },
  { value: "error", label: "settings.renameCollisionPolicyError" },
  { value: "replace_if_better", label: "settings.renameCollisionPolicyReplaceIfBetter" },
];

const RENAME_MISSING_METADATA_POLICY_OPTIONS = [
  { value: "fallback_title", label: "settings.renameMissingMetadataPolicyFallbackTitle" },
  { value: "skip", label: "settings.renameMissingMetadataPolicySkip" },
];

const FILLER_POLICY_OPTIONS = [
  { value: "download_all", label: "settings.fillerPolicyDownloadAll" },
  { value: "skip_filler", label: "settings.fillerPolicySkipFiller" },
];

const RECAP_POLICY_OPTIONS = [
  { value: "download_all", label: "settings.recapPolicyDownloadAll" },
  { value: "skip_recap", label: "settings.recapPolicySkipRecap" },
];

const COMMON_RENAME_TOKENS = [
  "title", "year", "quality", "source",
  "video_codec", "audio_codec", "audio_channels", "group", "ext",
];
const EXTERNAL_ID_RENAME_TOKENS = [
  "imdb_id", "tmdb_id", "tvdb_id", "anidb_id", "mal_id", "anilist_id",
];
const EPISODE_RENAME_TOKENS = [
  "season", "season_order", "episode", "episode_title", "absolute_episode",
];
const VALID_MOVIE_RENAME_TOKENS = new Set([
  ...COMMON_RENAME_TOKENS,
  "edition",
  ...EXTERNAL_ID_RENAME_TOKENS,
]);
const VALID_EPISODE_RENAME_TOKENS = new Set([
  ...COMMON_RENAME_TOKENS,
  ...EPISODE_RENAME_TOKENS,
  ...EXTERNAL_ID_RENAME_TOKENS,
]);

const SHARED_RENAME_TOKEN_DESCRIPTIONS: { token: string; labelKey: string }[] = [
  { token: "title", labelKey: "settings.renameTokenTitle" },
  { token: "quality", labelKey: "settings.renameTokenQuality" },
  { token: "source", labelKey: "settings.renameTokenSource" },
  { token: "video_codec", labelKey: "settings.renameTokenVideoCodec" },
  { token: "audio_codec", labelKey: "settings.renameTokenAudioCodec" },
  { token: "audio_channels", labelKey: "settings.renameTokenAudioChannels" },
  { token: "group", labelKey: "settings.renameTokenGroup" },
  { token: "ext", labelKey: "settings.renameTokenExt" },
];

const MOVIE_RENAME_TOKEN_DESCRIPTIONS: { token: string; labelKey: string }[] = [
  { token: "year", labelKey: "settings.renameTokenYear" },
  { token: "edition", labelKey: "settings.renameTokenEdition" },
];

const EXTERNAL_ID_RENAME_TOKEN_DESCRIPTIONS: { token: string; labelKey: string }[] = [
  { token: "imdb_id", labelKey: "settings.renameTokenImdbId" },
  { token: "tmdb_id", labelKey: "settings.renameTokenTmdbId" },
  { token: "tvdb_id", labelKey: "settings.renameTokenTvdbId" },
  { token: "anidb_id", labelKey: "settings.renameTokenAnidbId" },
  { token: "mal_id", labelKey: "settings.renameTokenMalId" },
  { token: "anilist_id", labelKey: "settings.renameTokenAnilistId" },
];

const SERIES_RENAME_TOKEN_DESCRIPTIONS: { token: string; labelKey: string }[] = [
  { token: "season", labelKey: "settings.renameTokenSeason" },
  { token: "season_order", labelKey: "settings.renameTokenSeasonOrder" },
  { token: "episode", labelKey: "settings.renameTokenEpisode" },
  { token: "absolute_episode", labelKey: "settings.renameTokenAbsoluteEpisode" },
  { token: "episode_title", labelKey: "settings.renameTokenEpisodeTitle" },
];

const ANIME_RENAME_TOKEN_DESCRIPTIONS: { token: string; labelKey: string }[] = [
  { token: "season", labelKey: "settings.renameTokenSeason" },
  { token: "season_order", labelKey: "settings.renameTokenSeasonOrder" },
  { token: "episode", labelKey: "settings.renameTokenEpisode" },
  { token: "absolute_episode", labelKey: "settings.renameTokenAbsoluteEpisode" },
  { token: "episode_title", labelKey: "settings.renameTokenEpisodeTitle" },
];

function getRenameTokenDescriptions(scopeId: ViewCategoryId): { token: string; labelKey: string }[] {
  const scopeSpecific = scopeId === "MOVIE"
    ? MOVIE_RENAME_TOKEN_DESCRIPTIONS
    : scopeId === "ANIME"
      ? ANIME_RENAME_TOKEN_DESCRIPTIONS
      : SERIES_RENAME_TOKEN_DESCRIPTIONS;
  const shared = scopeId === "SERIES"
    ? SHARED_RENAME_TOKEN_DESCRIPTIONS.filter((token) => token.token !== "group")
    : SHARED_RENAME_TOKEN_DESCRIPTIONS;
  return [...scopeSpecific, ...EXTERNAL_ID_RENAME_TOKEN_DESCRIPTIONS, ...shared];
}

function getValidRenameTokens(scopeId: ViewCategoryId): ReadonlySet<string> {
  return scopeId === "MOVIE"
    ? VALID_MOVIE_RENAME_TOKENS
    : VALID_EPISODE_RENAME_TOKENS;
}

function validateRenameTemplate(
  template: string,
  scopeId: ViewCategoryId,
  t: Translate,
): string | null {
  return formatRenameValidationIssue(
    validateRenameTemplateSyntax(template, getValidRenameTokens(scopeId)),
    t,
  );
}

function formatRenameValidationIssue(
  issue: RenameTemplateValidationIssue | null,
  t: Translate,
): string | null {
  if (!issue) {
    return null;
  }

  switch (issue.kind) {
    case "empty":
      return t("settings.renameValidationEmpty");
    case "unmatchedOpen":
      return t("settings.renameValidationUnmatchedOpen");
    case "unmatchedClose":
      return t("settings.renameValidationUnmatchedClose");
    case "unknownToken":
      return t("settings.renameValidationUnknownToken", { token: issue.token });
    case "invalidFilter":
      return t("settings.renameValidationInvalidFilter", { filter: issue.filter });
  }

  return null;
}

const RENAME_PREVIEW_MOVIE_SAMPLE: Record<string, string> = {
  title: "The Dark Knight",
  year: "2008",
  quality: "2160p",
  edition: "IMAX",
  source: "BluRay",
  video_codec: "x265",
  audio_codec: "DTS-HD MA",
  audio_channels: "5.1",
  group: "FraMeSToR",
  ext: "mkv",
  imdb_id: "tt0468569",
  tmdb_id: "155",
  tvdb_id: "123456",
  anidb_id: "",
  mal_id: "",
  anilist_id: "",
  season: "1",
  episode: "5",
  episode_title: "Pilot",
};

const RENAME_PREVIEW_SERIES_SAMPLE: Record<string, string> = {
  title: "Friends",
  year: "1994",
  quality: "1080p",
  edition: "Director's Cut",
  source: "WEB-DL",
  video_codec: "x264",
  audio_codec: "AAC",
  audio_channels: "2.0",
  group: "NTb",
  ext: "mkv",
  imdb_id: "tt0108778",
  tmdb_id: "1668",
  tvdb_id: "79168",
  anidb_id: "",
  mal_id: "",
  anilist_id: "",
  season: "5",
  season_order: "5",
  episode: "12",
  absolute_episode: "97",
  episode_title: "The One with the Embryos",
};

const RENAME_PREVIEW_ANIME_SAMPLE: Record<string, string> = {
  title: "Tidebreaker",
  year: "1999",
  quality: "1080p",
  edition: "Director's Cut",
  source: "WEB-DL",
  video_codec: "x265",
  audio_codec: "AAC",
  audio_channels: "2.0",
  group: "SubsPlease",
  ext: "mkv",
  imdb_id: "tt0388629",
  tmdb_id: "37854",
  tvdb_id: "81797",
  anidb_id: "69",
  mal_id: "21",
  anilist_id: "21",
  season: "1",
  season_order: "1",
  episode: "1",
  absolute_episode: "1",
  episode_title: "Romance Dawn",
};

function applyRenameTemplate(template: string, scopeId: ViewCategoryId): string | null {
  const sampleValues =
    scopeId === "MOVIE"
      ? RENAME_PREVIEW_MOVIE_SAMPLE
      : scopeId === "ANIME"
        ? RENAME_PREVIEW_ANIME_SAMPLE
        : RENAME_PREVIEW_SERIES_SAMPLE;
  return applyRenameTemplatePreview(template, getValidRenameTokens(scopeId), sampleValues);
}

// --- Component ---

export function RenameSettingsForm({
  contentSettingsLabel,
  mediaSettingsLoading,
  qualityProfiles,
  qualityProfileParseError,
  categoryQualityProfileOverrides,
  activeQualityScopeId,
  qualityProfileInheritValue,
  toProfileOptions,
  handleQualityProfileOverrideChange,
  categoryRenameTemplates,
  handleRenameTemplateChange,
  categoryRenameCollisionPolicies,
  handleRenameCollisionPolicyChange,
  categoryRenameMissingMetadataPolicies,
  handleRenameMissingMetadataPolicyChange,
  categoryFillerPolicies,
  handleFillerPolicyChange,
  categoryRecapPolicies,
  handleRecapPolicyChange,
  categoryMonitorSpecials,
  handleMonitorSpecialsChange,
  categoryInterSeasonMovies,
  handleInterSeasonMoviesChange,
  nfoWriteOnImport,
  handleNfoWriteChange,
  plexmatchWriteOnImport,
  handlePlexmatchWriteChange,
  updateCategoryMediaProfileSettings,
  mediaSettingsSaving,
}: {
  contentSettingsLabel: string;

  mediaSettingsLoading: boolean;
  qualityProfiles: ParsedQualityProfile[];
  qualityProfileParseError: string;
  categoryQualityProfileOverrides: Record<ViewCategoryId, string>;
  activeQualityScopeId: ViewCategoryId;
  qualityProfileInheritValue: string;
  toProfileOptions: (profiles: ParsedQualityProfile[]) => QualityProfileOption[];
  handleQualityProfileOverrideChange: (value: string) => void;
  categoryRenameTemplates: Record<ViewCategoryId, string>;
  handleRenameTemplateChange: (event: React.ChangeEvent<HTMLInputElement>) => void;
  categoryRenameCollisionPolicies: Record<ViewCategoryId, string>;
  handleRenameCollisionPolicyChange: (value: string) => void;
  categoryRenameMissingMetadataPolicies: Record<ViewCategoryId, string>;
  handleRenameMissingMetadataPolicyChange: (value: string) => void;
  categoryFillerPolicies: Record<ViewCategoryId, string>;
  handleFillerPolicyChange: (value: string) => void;
  categoryRecapPolicies: Record<ViewCategoryId, string>;
  handleRecapPolicyChange: (value: string) => void;
  categoryMonitorSpecials: Record<ViewCategoryId, string>;
  handleMonitorSpecialsChange: (checked: boolean) => void;
  categoryInterSeasonMovies: Record<ViewCategoryId, string>;
  handleInterSeasonMoviesChange: (checked: boolean) => void;
  nfoWriteOnImport: Record<ViewCategoryId, string>;
  handleNfoWriteChange: (checked: boolean) => void;
  plexmatchWriteOnImport: Record<ViewCategoryId, string>;
  handlePlexmatchWriteChange: (checked: boolean) => void;
  updateCategoryMediaProfileSettings: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  mediaSettingsSaving: boolean;
}) {
  const t = useTranslate();
  const templateValue = categoryRenameTemplates[activeQualityScopeId];
  const renameValidationError = React.useMemo(
    () => validateRenameTemplate(templateValue, activeQualityScopeId, t),
    [activeQualityScopeId, templateValue, t],
  );

  const renamePreview = React.useMemo(
    () => applyRenameTemplate(templateValue, activeQualityScopeId),
    [activeQualityScopeId, templateValue],
  );

  const templateInputRef = React.useRef<HTMLInputElement>(null);

  const insertToken = React.useCallback(
    (token: string) => {
      const input = templateInputRef.current;
      if (!input) return;
      const insertion = `{${token}}`;
      const start = input.selectionStart ?? templateValue.length;
      const end = input.selectionEnd ?? start;
      const next = templateValue.slice(0, start) + insertion + templateValue.slice(end);

      const nativeInputValueSetter = Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value",
      )?.set;
      if (nativeInputValueSetter) {
        nativeInputValueSetter.call(input, next);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      }

      requestAnimationFrame(() => {
        const cursorPos = start + insertion.length;
        input.setSelectionRange(cursorPos, cursorPos);
        input.focus();
      });
    },
    [templateValue],
  );

  return (
    <form onSubmit={updateCategoryMediaProfileSettings} className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle>{t("settings.qualityProfileSection")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <label>
            <Label className="mb-2 inline-flex items-center gap-2">
              {t("settings.qualityProfileOverrideLabel", {
                category: contentSettingsLabel.toLowerCase(),
              })}
              <InfoHelp
                text={t("settings.qualityProfileOverrideHelp")}
                ariaLabel={t("settings.qualityProfileOverrideHelp")}
              />
            </Label>
            <Select value={categoryQualityProfileOverrides[activeQualityScopeId]} onValueChange={handleQualityProfileOverrideChange} disabled={mediaSettingsLoading}>
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={qualityProfileInheritValue}>{t("settings.qualityProfileInheritLabel")}</SelectItem>
                {toProfileOptions(qualityProfiles).map((opt) => (
                  <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
                ))}
              </SelectContent>
            </Select>
            {qualityProfileParseError ? (
              <p className="mt-2 rounded border border-[var(--scry-danger-border-strong)] bg-[var(--scry-danger-bg)] p-2 text-xs text-[var(--scry-danger-text)]">
                {qualityProfileParseError}
              </p>
            ) : null}
          </label>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t("settings.renameSectionTitle")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-6">
          <div className="grid gap-4 lg:grid-cols-[2fr_1fr]">
            <div className="space-y-2.5">
              <Label className="text-sm text-card-foreground">
                {t("settings.renameTemplateLabel")}
              </Label>
              <Input
                ref={templateInputRef}
                value={templateValue}
                onChange={handleRenameTemplateChange}
                placeholder={t("settings.renameTemplatePlaceholder")}
                disabled={mediaSettingsLoading}
                className={
                  templateValue.trim()
                    ? renameValidationError
                      ? "text-[var(--scry-danger-text-soft)] border-[var(--scry-danger-border-strong)]"
                      : "border-[var(--scry-accent)] text-[var(--scry-accent-text)]"
                    : undefined
                }
              />
              {renameValidationError ? (
                <p className="text-xs text-[var(--scry-danger-text-soft)]">{renameValidationError}</p>
              ) : null}
            </div>

            <div className="space-y-2">
              <Label className="text-xs uppercase tracking-wider text-muted-foreground/60">
                Example
              </Label>
              {renamePreview ? (
                <div className="rounded border border-border bg-muted px-3 py-1.5">
                  <p className="break-all font-[var(--font-code)] text-sm text-card-foreground">{renamePreview}</p>
                </div>
              ) : (
                <div className="rounded border border-dashed border-border bg-card/40 px-3 py-1.5">
                  <p className="text-sm text-muted-foreground/60">—</p>
                </div>
              )}
            </div>
          </div>

          <div className="space-y-2.5">
            <p className="text-sm font-medium text-card-foreground">
              {t("settings.renameAvailableTokens")}
            </p>
            <div className="flex flex-wrap gap-1.5">
              {getRenameTokenDescriptions(activeQualityScopeId).map((item) => (
                <button
                  key={item.token}
                  type="button"
                  className="inline-flex items-center gap-1 rounded-md border border-border bg-muted px-2.5 py-1 text-xs text-card-foreground transition-colors hover:border-[var(--scry-accent)] hover:bg-accent hover:text-foreground"
                  title={t(item.labelKey)}
                  onClick={() => insertToken(item.token)}
                >
                  <code className="text-[var(--scry-accent-text)]">{`{${item.token}}`}</code>
                  <span className="leading-none text-muted-foreground">{t(item.labelKey)}</span>
                </button>
              ))}
            </div>
            <div className="grid gap-2 text-xs text-muted-foreground sm:grid-cols-2">
              <p>
                <span className="font-medium text-card-foreground">{t("settings.renameLiteralBracesLabel")}:</span>{" "}
                <code className="text-[var(--scry-accent-text)]">{"{{edition-{edition}}}"}</code>
              </p>
              <p>
                <span className="font-medium text-card-foreground">{t("settings.renameSpaceFilterLabel")}:</span>{" "}
                <code className="text-[var(--scry-accent-text)]">{"{title|space:_}"}</code>
              </p>
            </div>
          </div>

          <div className="grid gap-4 md:grid-cols-2">
            <label className="space-y-2">
              <Label className="text-sm text-card-foreground">
                {t("settings.renameCollisionPolicyLabel")}
              </Label>
              <Select value={categoryRenameCollisionPolicies[activeQualityScopeId]} onValueChange={handleRenameCollisionPolicyChange} disabled={mediaSettingsLoading}>
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {RENAME_COLLISION_POLICY_OPTIONS.map((option) => (
                    <SelectItem key={option.value} value={option.value}>{t(option.label)}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
            <label className="space-y-2">
              <Label className="text-sm text-card-foreground">
                {t("settings.renameMissingMetadataPolicyLabel")}
              </Label>
              <Select value={categoryRenameMissingMetadataPolicies[activeQualityScopeId]} onValueChange={handleRenameMissingMetadataPolicyChange} disabled={mediaSettingsLoading}>
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {RENAME_MISSING_METADATA_POLICY_OPTIONS.map((option) => (
                    <SelectItem key={option.value} value={option.value}>{t(option.label)}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
          </div>

          {activeQualityScopeId === "ANIME" && (
            <div className="grid gap-4 md:grid-cols-2">
              <label className="space-y-2">
                <Label className="text-sm text-card-foreground">
                  {t("settings.fillerPolicyLabel")}
                </Label>
                <Select value={categoryFillerPolicies[activeQualityScopeId]} onValueChange={handleFillerPolicyChange} disabled={mediaSettingsLoading}>
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {FILLER_POLICY_OPTIONS.map((option) => (
                      <SelectItem key={option.value} value={option.value}>{t(option.label)}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
              <label className="space-y-2">
                <Label className="text-sm text-card-foreground">
                  {t("settings.recapPolicyLabel")}
                </Label>
                <Select value={categoryRecapPolicies[activeQualityScopeId]} onValueChange={handleRecapPolicyChange} disabled={mediaSettingsLoading}>
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {RECAP_POLICY_OPTIONS.map((option) => (
                      <SelectItem key={option.value} value={option.value}>{t(option.label)}</SelectItem>
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
                  <span className="text-xs text-muted-foreground">{t("settings.monitorSpecialsDescription")}</span>
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
                  <span className="text-xs text-muted-foreground">{t("settings.interSeasonMoviesDescription")}</span>
                </div>
              </div>
            </div>
          )}

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
                <span className="text-xs text-muted-foreground">{t("settings.nfoWriteOnImportDescription")}</span>
              </div>
            </div>
            {(activeQualityScopeId === "SERIES" || activeQualityScopeId === "ANIME") && (
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
                  <span className="text-xs text-muted-foreground">{t("settings.plexmatchWriteOnImportDescription")}</span>
                </div>
              </div>
            )}
          </div>

          <div className="flex justify-end">
            <Button type="submit" disabled={mediaSettingsSaving || renameValidationError !== null}>
              {mediaSettingsSaving ? t("label.saving") : t("label.save")}
            </Button>
          </div>
        </CardContent>
      </Card>
    </form>
  );
}
