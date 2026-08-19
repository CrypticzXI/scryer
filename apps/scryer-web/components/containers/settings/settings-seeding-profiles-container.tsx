import * as React from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsSeedingProfilesSection } from "@/components/views/settings/settings-seeding-profiles-section";
import { useTranslate } from "@/lib/context/translate-context";
import { useSeedingProfilesManager } from "@/lib/hooks/use-seeding-profiles-manager";
import type { SeedingProfileDraft } from "@/lib/types/seeding-profiles";

type PendingSeedingProfileEditorAction =
  | { type: "create" }
  | { type: "edit"; profileId: string }
  | { type: "close" }
  | null;

export function SettingsSeedingProfilesContainer() {
  const manager = useSeedingProfilesManager();
  const t = useTranslate();
  const [isEditorOpen, setIsEditorOpen] = React.useState(false);
  const [editorMode, setEditorMode] = React.useState<"create" | "edit">("create");
  const [pendingEditorAction, setPendingEditorAction] =
    React.useState<PendingSeedingProfileEditorAction>(null);
  const [pendingDeleteProfileId, setPendingDeleteProfileId] = React.useState<
    string | null
  >(null);
  const [draftBaseline, setDraftBaseline] = React.useState<SeedingProfileDraft>(
    () => ({ ...manager.draft }),
  );
  const [awaitingBaselineSync, setAwaitingBaselineSync] = React.useState(false);

  React.useEffect(() => {
    if (!awaitingBaselineSync) {
      return;
    }
    setDraftBaseline({ ...manager.draft });
    setAwaitingBaselineSync(false);
  }, [awaitingBaselineSync, manager.draft]);

  const isDraftDirty =
    JSON.stringify(manager.draft) !== JSON.stringify(draftBaseline);

  const openCreateEditor = React.useCallback(() => {
    manager.resetDraft();
    setEditorMode("create");
    setIsEditorOpen(true);
    setAwaitingBaselineSync(true);
  }, [manager]);

  const openEditEditor = React.useCallback(
    (profileId: string) => {
      manager.loadProfileById(profileId);
      setEditorMode("edit");
      setIsEditorOpen(true);
      setAwaitingBaselineSync(true);
    },
    [manager],
  );

  const requestCreateEditor = React.useCallback(() => {
    if (!isEditorOpen || !isDraftDirty) {
      openCreateEditor();
      return;
    }
    setPendingEditorAction({ type: "create" });
  }, [isDraftDirty, isEditorOpen, openCreateEditor]);

  const requestEditProfile = React.useCallback(
    (profileId: string) => {
      if (!isEditorOpen || !isDraftDirty) {
        openEditEditor(profileId);
        return;
      }
      setPendingEditorAction({ type: "edit", profileId });
    },
    [isDraftDirty, isEditorOpen, openEditEditor],
  );

  const requestCloseEditor = React.useCallback(() => {
    if (!isEditorOpen) return;
    if (!isDraftDirty) {
      setIsEditorOpen(false);
      setEditorMode("create");
      manager.resetDraft();
      setAwaitingBaselineSync(true);
      return;
    }
    setPendingEditorAction({ type: "close" });
  }, [isDraftDirty, isEditorOpen, manager]);

  const confirmPendingEditorAction = React.useCallback(() => {
    if (!pendingEditorAction) return;
    if (pendingEditorAction.type === "create") {
      openCreateEditor();
    } else if (pendingEditorAction.type === "edit") {
      openEditEditor(pendingEditorAction.profileId);
    } else {
      setIsEditorOpen(false);
      setEditorMode("create");
      manager.resetDraft();
      setAwaitingBaselineSync(true);
    }
    setPendingEditorAction(null);
  }, [manager, openCreateEditor, openEditEditor, pendingEditorAction]);

  const handleSaveProfile = React.useCallback(
    async (event?: React.FormEvent<HTMLFormElement>) => {
      const saved = await manager.saveProfile(event);
      if (saved) {
        setIsEditorOpen(false);
        setEditorMode("create");
        setAwaitingBaselineSync(true);
      }
    },
    [manager],
  );

  const confirmDeleteProfile = React.useCallback(async () => {
    if (!pendingDeleteProfileId) return;
    const deleted = await manager.deleteProfile(pendingDeleteProfileId);
    if (
      deleted &&
      editorMode === "edit" &&
      manager.draft.id === pendingDeleteProfileId
    ) {
      setIsEditorOpen(false);
      setEditorMode("create");
      manager.resetDraft();
      setAwaitingBaselineSync(true);
    }
    // Closes on failure too: the blocked-delete reason stays on screen in the
    // section's error banner rather than being trapped behind the dialog.
    setPendingDeleteProfileId(null);
  }, [editorMode, manager, pendingDeleteProfileId]);

  return (
    <>
      <SettingsSeedingProfilesSection
        loading={manager.loading}
        saving={manager.saving}
        profiles={manager.profiles}
        defaultProfileId={manager.defaultProfileId}
        errorMessage={manager.errorMessage}
        clearErrorMessage={manager.clearErrorMessage}
        draft={manager.draft}
        setDraft={manager.setDraft}
        saveProfile={handleSaveProfile}
        deleteProfile={(profileId) => setPendingDeleteProfileId(profileId)}
        loadProfileById={requestEditProfile}
        resetDraft={requestCloseEditor}
        setDefaultProfile={(profileId) => void manager.setDefaultProfile(profileId)}
        isEditorOpen={isEditorOpen}
        editorMode={editorMode}
        startCreateProfile={requestCreateEditor}
      />
      <ConfirmDialog
        open={pendingEditorAction !== null}
        title={t("settings.seedingProfileConfirmDiscardTitle")}
        description={t("settings.seedingProfileConfirmDiscardDescription")}
        confirmLabel={
          pendingEditorAction?.type === "create"
            ? t("settings.seedingProfileCreateNew")
            : pendingEditorAction?.type === "edit"
              ? t("label.edit")
              : t("label.discard")
        }
        cancelLabel={t("label.cancel")}
        isBusy={manager.saving}
        onConfirm={confirmPendingEditorAction}
        onCancel={() => setPendingEditorAction(null)}
      />
      <ConfirmDialog
        open={pendingDeleteProfileId !== null}
        title={t("label.delete")}
        description={t("settings.seedingProfileDeleteConfirm")}
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={manager.saving}
        onConfirm={confirmDeleteProfile}
        onCancel={() => setPendingDeleteProfileId(null)}
      />
    </>
  );
}
