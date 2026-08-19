import * as React from "react";
import { Edit, Plus, Trash2 } from "lucide-react";
import { AddNewButton } from "@/components/common/add-new-button";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import { CheckboxField } from "@/components/ui/checkbox";
import { Input, integerInputProps } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableActionsCell,
  TableActionsHead,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  PostImportTracking,
  SeasonPackSeedMode,
  SeedGoalMetAction,
  SeedingProfileDraft,
  SeedingProfileRecord,
} from "@/lib/types/seeding-profiles";
import {
  formatSeasonPackSummary,
  formatSeedingProfileRatio,
  formatSeedingProfileSeedTime,
  handsOffAfterImport,
  POST_IMPORT_TRACKING_MODES,
  SEASON_PACK_SEED_MODES,
  SEED_GOAL_MET_ACTIONS,
  SEEDING_PROFILE_INHERIT_VALUE,
  seedingProfileSelectValue,
  seedingProfileSelectValueToId,
} from "@/lib/utils/seeding-profiles";
import { selectorId } from "@/lib/utils/dom-ids";

type SettingsSeedingProfilesSectionProps = {
  loading: boolean;
  saving: boolean;
  profiles: SeedingProfileRecord[];
  defaultProfileId: string | null;
  errorMessage: string;
  clearErrorMessage: () => void;
  draft: SeedingProfileDraft;
  setDraft: React.Dispatch<React.SetStateAction<SeedingProfileDraft>>;
  saveProfile: (event?: React.FormEvent<HTMLFormElement>) => void;
  deleteProfile: (profileId: string) => void;
  loadProfileById: (profileId: string) => void;
  resetDraft: () => void;
  setDefaultProfile: (profileId: string | null) => void;
  isEditorOpen: boolean;
  editorMode: "create" | "edit";
  startCreateProfile: () => void;
};

const SEEDING_PANEL_CLASS =
  "overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]";
const SEEDING_PANEL_HEADER_CLASS =
  "border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3";
const SEEDING_PANEL_TITLE_CLASS =
  "text-[15px] font-semibold text-[var(--scry-ink2)]";
const SEEDING_PANEL_BODY_CLASS = "p-4 sm:p-5";
const SEEDING_MUTED_TEXT_CLASS = "text-[var(--scry-muted3)]";
const SEEDING_TABLE_HEADER_CELL_CLASS =
  "text-center font-semibold text-[var(--scry-muted3)]";

const SEASON_PACK_MODE_LABEL_KEY: Record<SeasonPackSeedMode, string> = {
  INHERIT: "settings.seedingProfileSeasonPackInherit",
  OVERRIDE: "settings.seedingProfileSeasonPackOverride",
};

const GOAL_MET_ACTION_LABEL_KEY: Record<SeedGoalMetAction, string> = {
  REMOVE_ENTRY: "settings.seedingProfileGoalMetRemoveEntry",
  STOP_SEEDING: "settings.seedingProfileGoalMetStopSeeding",
  KEEP: "settings.seedingProfileGoalMetKeep",
};

const POST_IMPORT_TRACKING_LABEL_KEY: Record<PostImportTracking, string> = {
  PARK: "settings.seedingProfilePostImportTrackingPark",
  HAND_OFF: "settings.seedingProfilePostImportTrackingHandOff",
};

export function SettingsSeedingProfilesSection({
  loading,
  saving,
  profiles,
  defaultProfileId,
  errorMessage,
  clearErrorMessage,
  draft,
  setDraft,
  saveProfile,
  deleteProfile,
  loadProfileById,
  resetDraft,
  setDefaultProfile,
  isEditorOpen,
  editorMode,
  startCreateProfile,
}: SettingsSeedingProfilesSectionProps) {
  const t = useTranslate();

  const isEditing = editorMode === "edit";
  const isSeasonPackOverride = draft.seasonPackMode === "OVERRIDE";
  // Handing a torrent off after import means Scryer never acts on the goal,
  // so the goal-met action and the never-remove flag stop applying.
  const isHandOff = handsOffAfterImport(draft.postImportTracking);

  function updateField<K extends keyof SeedingProfileDraft>(
    field: K,
    value: SeedingProfileDraft[K],
  ) {
    setDraft((prev) => ({ ...prev, [field]: value }));
  }

  // Season-pack goals are an advanced block: collapsed unless the profile being
  // edited actually overrides, matching the download-client editor's advanced
  // filesystem-path-mapping block.
  const [isSeasonPackOpen, setIsSeasonPackOpen] = React.useState(
    () => draft.seasonPackMode === "OVERRIDE",
  );
  const editorIdentity = `${isEditorOpen ? "open" : "closed"}:${editorMode}:${draft.id || "new"}`;
  const previousEditorIdentity = React.useRef(editorIdentity);

  React.useEffect(() => {
    if (previousEditorIdentity.current === editorIdentity) {
      return;
    }
    previousEditorIdentity.current = editorIdentity;
    setIsSeasonPackOpen(draft.seasonPackMode === "OVERRIDE");
  }, [draft.seasonPackMode, editorIdentity]);

  const defaultProfileMissing =
    defaultProfileId !== null &&
    !profiles.some((profile) => profile.id === defaultProfileId);

  return (
    <div id="settings-seeding-profiles-section" className="space-y-4 text-sm">
      {errorMessage ? (
        <div
          id="settings-seeding-profile-error"
          role="alert"
          className="flex flex-wrap items-start justify-between gap-3 rounded-[12px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] p-3 text-sm text-[var(--scry-danger-text)]"
        >
          <span className="min-w-0 whitespace-pre-wrap break-words">
            {errorMessage}
          </span>
          <Button
            id="settings-seeding-profile-error-dismiss"
            type="button"
            variant="ghost"
            size="sm"
            className="h-auto shrink-0 px-2 py-0.5 text-xs"
            onClick={clearErrorMessage}
          >
            {t("label.dismiss")}
          </Button>
        </div>
      ) : null}

      <section className={SEEDING_PANEL_CLASS}>
        <div className={SEEDING_PANEL_HEADER_CLASS}>
          <h2 className={SEEDING_PANEL_TITLE_CLASS}>
            {t("settings.seedingProfileDefaultTitle")}
          </h2>
        </div>
        <div className={`${SEEDING_PANEL_BODY_CLASS} space-y-1.5`}>
          <Label
            className="text-[var(--scry-ink2)]"
            htmlFor="settings-seeding-profile-default"
          >
            {t("settings.seedingProfileDefaultLabel")}
          </Label>
          <Select
            value={seedingProfileSelectValue(defaultProfileId)}
            onValueChange={(value) =>
              setDefaultProfile(seedingProfileSelectValueToId(value))
            }
          >
            <SelectTrigger
              id="settings-seeding-profile-default"
              className="w-full max-w-[320px]"
              disabled={loading || saving}
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={SEEDING_PROFILE_INHERIT_VALUE}>
                {t("settings.seedingProfileDefaultNone")}
              </SelectItem>
              {defaultProfileMissing && defaultProfileId ? (
                <SelectItem value={defaultProfileId}>
                  {t("settings.seedingProfileMissing", {
                    id: defaultProfileId,
                  })}
                </SelectItem>
              ) : null}
              {profiles.map((profile) => (
                <SelectItem key={profile.id} value={profile.id}>
                  {profile.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <p className={`text-xs ${SEEDING_MUTED_TEXT_CLASS}`}>
            {t("settings.seedingProfileDefaultHelp")}
          </p>
        </div>
      </section>

      <section className={SEEDING_PANEL_CLASS}>
        <div className={SEEDING_PANEL_HEADER_CLASS}>
          <h2 className={SEEDING_PANEL_TITLE_CLASS}>
            {t("settings.seedingProfileExisting")}
          </h2>
        </div>
        <div>
          {loading ? (
            <p
              className={`${SEEDING_PANEL_BODY_CLASS} text-sm ${SEEDING_MUTED_TEXT_CLASS}`}
            >
              {t("label.loading")}
            </p>
          ) : profiles.length === 0 ? (
            <p
              id="settings-seeding-profiles-empty"
              className={`${SEEDING_PANEL_BODY_CLASS} text-sm ${SEEDING_MUTED_TEXT_CLASS}`}
            >
              {t("settings.seedingProfileNone")}
            </p>
          ) : (
            <div className="overflow-hidden">
              <Table overflow="clip" layout="fixed" density="dense">
                <TableHeader>
                  <TableRow className="border-[var(--scry-border3)] bg-[var(--scry-inset)] hover:bg-[var(--scry-inset)]">
                    <TableHead
                      className={`w-[22%] font-semibold ${SEEDING_MUTED_TEXT_CLASS}`}
                    >
                      {t("settings.seedingProfileNameLabel")}
                    </TableHead>
                    <TableHead className={`w-24 ${SEEDING_TABLE_HEADER_CELL_CLASS}`}>
                      {t("settings.seedingProfileRatioLabel")}
                    </TableHead>
                    <TableHead className={`w-28 ${SEEDING_TABLE_HEADER_CELL_CLASS}`}>
                      {t("settings.seedingProfileSeedTimeLabel")}
                    </TableHead>
                    <TableHead className={`w-32 ${SEEDING_TABLE_HEADER_CELL_CLASS}`}>
                      {t("settings.seedingProfileSeasonPacksColumn")}
                    </TableHead>
                    <TableHead className={`w-36 ${SEEDING_TABLE_HEADER_CELL_CLASS}`}>
                      {t("settings.seedingProfileGoalMetActionLabel")}
                    </TableHead>
                    <TableHead className={`w-36 ${SEEDING_TABLE_HEADER_CELL_CLASS}`}>
                      {t("settings.seedingProfilePostImportTrackingLabel")}
                    </TableHead>
                    <TableHead className={`w-32 ${SEEDING_TABLE_HEADER_CELL_CLASS}`}>
                      {t("settings.seedingProfileHonorTrackerMinimumsLabel")}
                    </TableHead>
                    <TableHead className={`w-32 ${SEEDING_TABLE_HEADER_CELL_CLASS}`}>
                      {t("settings.seedingProfileNeverRemoveLabel")}
                    </TableHead>
                    <TableActionsHead className="w-24">
                      {t("label.actions")}
                    </TableActionsHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {profiles.map((profile) => (
                    <TableRow
                      key={profile.id}
                      id={selectorId("settings-seeding-profile-row", profile.id)}
                      className="border-[var(--scry-border3)] hover:bg-[var(--scry-rowHover)]"
                    >
                      <TableCell className="truncate font-medium text-[var(--scry-ink2)]">
                        {profile.name}
                        {profile.id === defaultProfileId ? (
                          <span className="ml-2 text-xs font-normal text-[var(--scry-accent-text)]">
                            {t("settings.seedingProfileDefaultBadge")}
                          </span>
                        ) : null}
                      </TableCell>
                      <TableCell className={`text-center ${SEEDING_MUTED_TEXT_CLASS}`}>
                        {formatSeedingProfileRatio(profile.ratio)}
                      </TableCell>
                      <TableCell className={`text-center ${SEEDING_MUTED_TEXT_CLASS}`}>
                        {formatSeedingProfileSeedTime(profile.seedTimeMinutes)}
                      </TableCell>
                      <TableCell className={`truncate text-center ${SEEDING_MUTED_TEXT_CLASS}`}>
                        {formatSeasonPackSummary(
                          profile,
                          t("settings.seedingProfileSeasonPackInherit"),
                        )}
                      </TableCell>
                      <TableCell className="text-center text-[var(--scry-ink2)]">
                        {handsOffAfterImport(profile.postImportTracking)
                          ? "—"
                          : t(GOAL_MET_ACTION_LABEL_KEY[profile.goalMetAction])}
                      </TableCell>
                      <TableCell className="text-center">
                        {handsOffAfterImport(profile.postImportTracking) ? (
                          <span className="inline-flex items-center gap-1 rounded-full border border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] px-2 py-0.5 text-xs font-medium text-[var(--scry-info-text)]">
                            {t("settings.seedingProfilePostImportTrackingHandOffBadge")}
                          </span>
                        ) : (
                          <span className={`text-xs ${SEEDING_MUTED_TEXT_CLASS}`}>
                            {t(
                              POST_IMPORT_TRACKING_LABEL_KEY[
                                profile.postImportTracking
                              ],
                            )}
                          </span>
                        )}
                      </TableCell>
                      <TableCell className="text-center">
                        <RenderBooleanIcon
                          value={profile.honorTrackerMinimums}
                          label={`${t("settings.seedingProfileHonorTrackerMinimumsLabel")}: ${profile.name}`}
                        />
                      </TableCell>
                      <TableCell className="text-center">
                        {profile.neverRemove ? (
                          <span className="inline-flex items-center gap-1 rounded-full border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-2 py-0.5 text-xs font-medium text-[var(--scry-warning-text)]">
                            {t("settings.seedingProfileNeverRemoveBadge")}
                          </span>
                        ) : (
                          <RenderBooleanIcon
                            value={false}
                            label={`${t("settings.seedingProfileNeverRemoveLabel")}: ${profile.name}`}
                          />
                        )}
                      </TableCell>
                      <TableActionsCell className="w-24">
                        <div className="flex items-center justify-center gap-1">
                          <IconButton
                            id={selectorId(
                              "settings-seeding-profile-edit",
                              profile.id,
                            )}
                            label={t("label.edit")}
                            tone="edit"
                            onClick={() => loadProfileById(profile.id)}
                            title={t("label.load")}
                          >
                            <Edit className="h-4 w-4" />
                          </IconButton>
                          <IconButton
                            id={selectorId(
                              "settings-seeding-profile-delete",
                              profile.id,
                            )}
                            label={t("label.delete")}
                            tone="delete"
                            onClick={() => deleteProfile(profile.id)}
                            disabled={saving}
                          >
                            <Trash2 className="h-4 w-4" />
                          </IconButton>
                        </div>
                      </TableActionsCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </div>
      </section>

      {isEditorOpen ? (
        <>
          <section className={SEEDING_PANEL_CLASS}>
            <div className={SEEDING_PANEL_HEADER_CLASS}>
              <h2 className={SEEDING_PANEL_TITLE_CLASS}>
                {isEditing
                  ? t("settings.seedingProfileEdit")
                  : t("settings.seedingProfileCreate")}
              </h2>
            </div>
            <div className={SEEDING_PANEL_BODY_CLASS}>
              <form
                id="settings-seeding-profile-form"
                onSubmit={saveProfile}
                className="space-y-4"
              >
                {/* Name */}
                <div className="space-y-1.5">
                  <Label
                    className="text-[var(--scry-ink2)]"
                    htmlFor="settings-seeding-profile-name"
                  >
                    {t("settings.seedingProfileNameLabel")}
                  </Label>
                  <Input
                    id="settings-seeding-profile-name"
                    value={draft.name}
                    onChange={(event) => updateField("name", event.target.value)}
                    placeholder={t("settings.seedingProfileNamePlaceholder")}
                  />
                </div>

                {/* Goals */}
                <div className="grid gap-4 sm:grid-cols-2">
                  <div className="space-y-1.5">
                    <Label
                      className="text-[var(--scry-ink2)]"
                      htmlFor="settings-seeding-profile-ratio"
                    >
                      {t("settings.seedingProfileRatioLabel")}
                    </Label>
                    <Input
                      id="settings-seeding-profile-ratio"
                      inputMode="decimal"
                      value={draft.ratio}
                      onChange={(event) =>
                        updateField("ratio", event.target.value)
                      }
                      placeholder={t("settings.seedingProfileGoalPlaceholder")}
                    />
                    <p className={`text-xs ${SEEDING_MUTED_TEXT_CLASS}`}>
                      {t("settings.seedingProfileRatioHelp")}
                    </p>
                  </div>
                  <div className="space-y-1.5">
                    <Label
                      className="text-[var(--scry-ink2)]"
                      htmlFor="settings-seeding-profile-seed-time"
                    >
                      {t("settings.seedingProfileSeedTimeLabel")}
                    </Label>
                    <Input
                      id="settings-seeding-profile-seed-time"
                      {...integerInputProps}
                      value={draft.seedTimeMinutes}
                      onChange={(event) =>
                        updateField("seedTimeMinutes", event.target.value)
                      }
                      placeholder={t("settings.seedingProfileGoalPlaceholder")}
                    />
                    <p className={`text-xs ${SEEDING_MUTED_TEXT_CLASS}`}>
                      {t("settings.seedingProfileSeedTimeHelp")}
                    </p>
                    <p className={`text-xs ${SEEDING_MUTED_TEXT_CLASS}`}>
                      {t("settings.seedingProfileSeedTimeTransmissionHelp")}
                    </p>
                  </div>
                </div>

                {/* Post-import tracking */}
                <div className="space-y-1.5">
                  <Label
                    className="text-[var(--scry-ink2)]"
                    htmlFor="settings-seeding-profile-post-import-tracking"
                  >
                    {t("settings.seedingProfilePostImportTrackingLabel")}
                  </Label>
                  <Select
                    value={draft.postImportTracking}
                    onValueChange={(value) =>
                      updateField(
                        "postImportTracking",
                        value as PostImportTracking,
                      )
                    }
                  >
                    <SelectTrigger
                      id="settings-seeding-profile-post-import-tracking"
                      className="w-full max-w-[320px]"
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {POST_IMPORT_TRACKING_MODES.map((mode) => (
                        <SelectItem
                          key={mode}
                          id={selectorId(
                            "settings-seeding-profile-post-import-tracking-option",
                            mode,
                          )}
                          value={mode}
                        >
                          {t(POST_IMPORT_TRACKING_LABEL_KEY[mode])}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <p className={`text-xs ${SEEDING_MUTED_TEXT_CLASS}`}>
                    {t("settings.seedingProfilePostImportTrackingHelp")}
                  </p>
                  {isHandOff ? (
                    <p className="text-xs text-[var(--scry-warning-text)]">
                      {t("settings.seedingProfilePostImportTrackingHandOffHelp")}
                    </p>
                  ) : null}
                </div>

                {/* Goal-met action */}
                <div className="space-y-1.5">
                  <Label
                    className="text-[var(--scry-ink2)]"
                    htmlFor="settings-seeding-profile-goal-met-action"
                  >
                    {t("settings.seedingProfileGoalMetActionLabel")}
                  </Label>
                  <Select
                    value={draft.goalMetAction}
                    disabled={isHandOff}
                    onValueChange={(value) =>
                      updateField("goalMetAction", value as SeedGoalMetAction)
                    }
                  >
                    <SelectTrigger
                      id="settings-seeding-profile-goal-met-action"
                      className="w-full max-w-[320px]"
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {SEED_GOAL_MET_ACTIONS.map((action) => (
                        <SelectItem
                          key={action}
                          id={selectorId(
                            "settings-seeding-profile-goal-met-action-option",
                            action,
                          )}
                          value={action}
                        >
                          {t(GOAL_MET_ACTION_LABEL_KEY[action])}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <p className={`text-xs ${SEEDING_MUTED_TEXT_CLASS}`}>
                    {t("settings.seedingProfileGoalMetActionHelp")}
                  </p>
                </div>

                {/* Tracker minimums */}
                <CheckboxField
                  id="settings-seeding-profile-honor-tracker-minimums"
                  checked={draft.honorTrackerMinimums}
                  onCheckedChange={(checked) =>
                    updateField("honorTrackerMinimums", checked === true)
                  }
                  label={t("settings.seedingProfileHonorTrackerMinimumsLabel")}
                  description={t(
                    "settings.seedingProfileHonorTrackerMinimumsHelp",
                  )}
                  className="text-[var(--scry-ink2)]"
                />

                {/* Never remove */}
                <CheckboxField
                  id="settings-seeding-profile-never-remove"
                  checked={draft.neverRemove}
                  disabled={isHandOff}
                  onCheckedChange={(checked) =>
                    updateField("neverRemove", checked === true)
                  }
                  label={t("settings.seedingProfileNeverRemoveLabel")}
                  description={t("settings.seedingProfileNeverRemoveHelp")}
                  className="text-[var(--scry-ink2)]"
                  descriptionClassName="text-[var(--scry-warning-text)]"
                />

                {/* Season-pack overrides (advanced) */}
                <details
                  id="settings-seeding-profile-season-pack"
                  className="rounded-xl border border-border bg-card p-3"
                  open={isSeasonPackOpen}
                  onToggle={(event) =>
                    setIsSeasonPackOpen(event.currentTarget.open)
                  }
                >
                  <summary
                    id="settings-seeding-profile-season-pack-toggle"
                    className="cursor-pointer select-none text-sm font-medium text-card-foreground"
                  >
                    {t("settings.seedingProfileSeasonPackAdvanced")}
                  </summary>
                  <div className="mt-3 space-y-3">
                    <div className="space-y-1.5">
                      <Label
                        className="text-[var(--scry-ink2)]"
                        htmlFor="settings-seeding-profile-season-pack-mode"
                      >
                        {t("settings.seedingProfileSeasonPackModeLabel")}
                      </Label>
                      <Select
                        value={draft.seasonPackMode}
                        onValueChange={(value) =>
                          updateField(
                            "seasonPackMode",
                            value as SeasonPackSeedMode,
                          )
                        }
                      >
                        <SelectTrigger
                          id="settings-seeding-profile-season-pack-mode"
                          className="w-full max-w-[320px]"
                        >
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {SEASON_PACK_SEED_MODES.map((mode) => (
                            <SelectItem
                              key={mode}
                              id={selectorId(
                                "settings-seeding-profile-season-pack-mode-option",
                                mode,
                              )}
                              value={mode}
                            >
                              {t(SEASON_PACK_MODE_LABEL_KEY[mode])}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      <p className={`text-xs ${SEEDING_MUTED_TEXT_CLASS}`}>
                        {t("settings.seedingProfileSeasonPackModeHelp")}
                      </p>
                    </div>
                    <div className="grid gap-4 sm:grid-cols-2">
                      <div className="space-y-1.5">
                        <Label
                          className="text-[var(--scry-ink2)]"
                          htmlFor="settings-seeding-profile-season-pack-ratio"
                        >
                          {t("settings.seedingProfileSeasonPackRatioLabel")}
                        </Label>
                        <Input
                          id="settings-seeding-profile-season-pack-ratio"
                          inputMode="decimal"
                          value={draft.seasonPackRatio}
                          disabled={!isSeasonPackOverride}
                          onChange={(event) =>
                            updateField("seasonPackRatio", event.target.value)
                          }
                          placeholder={t(
                            "settings.seedingProfileGoalPlaceholder",
                          )}
                        />
                      </div>
                      <div className="space-y-1.5">
                        <Label
                          className="text-[var(--scry-ink2)]"
                          htmlFor="settings-seeding-profile-season-pack-seed-time"
                        >
                          {t("settings.seedingProfileSeasonPackSeedTimeLabel")}
                        </Label>
                        <Input
                          id="settings-seeding-profile-season-pack-seed-time"
                          {...integerInputProps}
                          value={draft.seasonPackSeedTimeMinutes}
                          disabled={!isSeasonPackOverride}
                          onChange={(event) =>
                            updateField(
                              "seasonPackSeedTimeMinutes",
                              event.target.value,
                            )
                          }
                          placeholder={t(
                            "settings.seedingProfileGoalPlaceholder",
                          )}
                        />
                      </div>
                    </div>
                    <p className={`text-xs ${SEEDING_MUTED_TEXT_CLASS}`}>
                      {t("settings.seedingProfileSeasonPackGoalsHelp")}
                    </p>
                  </div>
                </details>

                {/* Actions */}
                <div className="flex flex-wrap gap-2 pt-2">
                  <Button
                    id="settings-seeding-profile-save"
                    type="submit"
                    disabled={saving}
                  >
                    {saving
                      ? t("label.saving")
                      : isEditing
                        ? t("label.save")
                        : t("settings.seedingProfileCreate")}
                  </Button>
                  <Button
                    id="settings-seeding-profile-cancel"
                    type="button"
                    variant="outline"
                    onClick={resetDraft}
                  >
                    {t("label.cancel")}
                  </Button>
                </div>
              </form>
            </div>
          </section>
          {isEditing ? (
            <div className="flex justify-center">
              <AddNewButton
                id="settings-seeding-profile-create-new"
                icon={Plus}
                label={t("settings.seedingProfileCreateNew")}
                onClick={startCreateProfile}
                disabled={saving}
              />
            </div>
          ) : null}
        </>
      ) : (
        <div className="flex justify-center">
          <AddNewButton
            id="settings-seeding-profile-create"
            icon={Plus}
            label={t("settings.seedingProfileCreateNew")}
            onClick={startCreateProfile}
            disabled={saving}
          />
        </div>
      )}
    </div>
  );
}
