import { type FormEvent, useCallback, useEffect, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsPostProcessingSection } from "@/components/views/settings/settings-post-processing-section";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import {
  postProcessingScriptsQuery,
  postProcessingScriptRunsQuery,
} from "@/lib/graphql/queries";
import {
  createPostProcessingScriptMutation,
  updatePostProcessingScriptMutation,
  deletePostProcessingScriptMutation,
  togglePostProcessingScriptMutation,
} from "@/lib/graphql/mutations";

export type PPScript = {
  id: string;
  name: string;
  description: string;
  scriptType: string;
  scriptContent: string;
  appliedFacets: string[];
  executionMode: string;
  timeoutSecs: number;
  priority: number;
  enabled: boolean;
  debug: boolean;
  createdAt: string;
  updatedAt: string;
};

export type PPScriptRun = {
  id: string;
  scriptId: string;
  scriptName: string;
  titleId: string | null;
  titleName: string | null;
  facet: string | null;
  filePath: string | null;
  status: string;
  exitCode: number | null;
  stdoutTail: string | null;
  stderrTail: string | null;
  durationMs: number | null;
  startedAt: string;
  completedAt: string | null;
};

export type PPScriptDraft = {
  name: string;
  description: string;
  scriptType: string;
  scriptContent: string;
  appliedFacets: string[];
  executionMode: string;
  timeoutSecs: number;
  priority: number;
  debug: boolean;
};

const INITIAL_DRAFT: PPScriptDraft = {
  name: "",
  description: "",
  scriptType: "inline",
  scriptContent: "",
  appliedFacets: [],
  executionMode: "blocking",
  timeoutSecs: 300,
  priority: 0,
  debug: true,
};

function cloneScriptDraft(draft: PPScriptDraft): PPScriptDraft {
  return {
    ...draft,
    appliedFacets: [...draft.appliedFacets],
  };
}

type PendingScriptEditorAction =
  | { type: "create" }
  | { type: "edit"; record: PPScript }
  | { type: "close" }
  | null;

type PendingInlineShellAction =
  | { type: "save" }
  | { type: "toggle"; record: PPScript }
  | null;

export function SettingsPostProcessingContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [scripts, setScripts] = useState<PPScript[]>([]);
  const [editingScriptId, setEditingScriptId] = useState<string | null>(null);
  const [isEditorOpen, setIsEditorOpen] = useState(false);
  const [pendingDeleteScript, setPendingDeleteScript] = useState<PPScript | null>(null);
  const [pendingEditorAction, setPendingEditorAction] =
    useState<PendingScriptEditorAction>(null);
  const [pendingInlineShellAction, setPendingInlineShellAction] =
    useState<PendingInlineShellAction>(null);
  const [mutatingScriptId, setMutatingScriptId] = useState<string | null>(null);
  const [scriptDraft, setScriptDraft] = useState<PPScriptDraft>(() =>
    cloneScriptDraft(INITIAL_DRAFT),
  );
  const [scriptDraftBaseline, setScriptDraftBaseline] = useState<PPScriptDraft>(() =>
    cloneScriptDraft(INITIAL_DRAFT),
  );
  const [expandedScriptId, setExpandedScriptId] = useState<string | null>(null);
  const [scriptRuns, setScriptRuns] = useState<Record<string, PPScriptRun[]>>({});

  const closeEditor = useCallback(() => {
    setIsEditorOpen(false);
    setEditingScriptId(null);
    setScriptDraft(() => cloneScriptDraft(INITIAL_DRAFT));
    setScriptDraftBaseline(() => cloneScriptDraft(INITIAL_DRAFT));
  }, []);

  const isDraftDirty =
    JSON.stringify(scriptDraft) !== JSON.stringify(scriptDraftBaseline);

  const scriptDraftRequiresInlineShellAcknowledgement =
    scriptDraft.scriptType === "inline" &&
    (!editingScriptId ||
      scriptDraftBaseline.scriptType !== "inline" ||
      scriptDraft.scriptContent !== scriptDraftBaseline.scriptContent);

  const openCreateEditor = useCallback(() => {
    const nextDraft = cloneScriptDraft(INITIAL_DRAFT);
    setEditingScriptId(null);
    setScriptDraft(nextDraft);
    setScriptDraftBaseline(cloneScriptDraft(nextDraft));
    setIsEditorOpen(true);
  }, []);

  const openEditEditor = useCallback((record: PPScript) => {
    const nextDraft = {
      name: record.name,
      description: record.description,
      scriptType: record.scriptType,
      scriptContent: record.scriptContent,
      appliedFacets: [...record.appliedFacets],
      executionMode: record.executionMode,
      timeoutSecs: record.timeoutSecs,
      priority: record.priority,
      debug: record.debug,
    };
    setEditingScriptId(record.id);
    setScriptDraft(nextDraft);
    setScriptDraftBaseline(cloneScriptDraft(nextDraft));
    setIsEditorOpen(true);
    setGlobalStatus(t("status.editingRule", { name: record.name }));
  }, [setGlobalStatus, t]);

  const requestCreateEditor = useCallback(() => {
    if (!isEditorOpen || !isDraftDirty) {
      openCreateEditor();
      return;
    }
    setPendingEditorAction({ type: "create" });
  }, [isDraftDirty, isEditorOpen, openCreateEditor]);

  const requestEditScript = useCallback((record: PPScript) => {
    if (!isEditorOpen || !isDraftDirty) {
      openEditEditor(record);
      return;
    }
    setPendingEditorAction({ type: "edit", record });
  }, [isDraftDirty, isEditorOpen, openEditEditor]);

  const requestCloseEditor = useCallback(() => {
    if (!isEditorOpen) return;
    if (!isDraftDirty) {
      closeEditor();
      return;
    }
    setPendingEditorAction({ type: "close" });
  }, [closeEditor, isDraftDirty, isEditorOpen]);

  const refreshScripts = useCallback(async () => {
    try {
      const { data, error } = await client.query(postProcessingScriptsQuery, {}).toPromise();
      if (error) throw error;
      setScripts(data.postProcessingScripts || []);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    void refreshScripts();
  }, [refreshScripts]);

  const loadRunsForScript = useCallback(
    async (scriptId: string) => {
      try {
        const { data, error } = await client
          .query(postProcessingScriptRunsQuery, { scriptId, limit: 20 })
          .toPromise();
        if (error) throw error;
        setScriptRuns((prev) => ({
          ...prev,
          [scriptId]: data.postProcessingScriptRuns || [],
        }));
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
      }
    },
    [client, setGlobalStatus, t],
  );

  const saveScript = useCallback(
    async (inlineShellAcknowledged = false) => {
    const payload = {
      name: scriptDraft.name.trim(),
      description: scriptDraft.description.trim(),
      scriptType: scriptDraft.scriptType,
      scriptContent: scriptDraft.scriptContent,
      appliedFacets: scriptDraft.appliedFacets,
      executionMode: scriptDraft.executionMode,
      timeoutSecs: scriptDraft.timeoutSecs,
      priority: scriptDraft.priority,
      debug: scriptDraft.debug,
      ...(inlineShellAcknowledged ? { inlineShellAcknowledged: true } : {}),
    };

    if (!payload.name || !payload.scriptContent.trim()) {
      setGlobalStatus(t("settings.ruleValidationRequired"));
      return;
    }

    setMutatingScriptId(editingScriptId || "new");
    try {
      if (editingScriptId) {
        const { error } = await client
          .mutation(updatePostProcessingScriptMutation, {
            input: { id: editingScriptId, ...payload },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("settings.pp.updated"));
      } else {
        const { error } = await client
          .mutation(createPostProcessingScriptMutation, { input: payload })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("settings.pp.created"));
      }
      closeEditor();
      await refreshScripts();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingScriptId(null);
    }
    },
    [
      client,
      closeEditor,
      editingScriptId,
      refreshScripts,
      scriptDraft,
      setGlobalStatus,
      t,
    ],
  );

  const submitScript = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!scriptDraft.name.trim() || !scriptDraft.scriptContent.trim()) {
      setGlobalStatus(t("settings.ruleValidationRequired"));
      return;
    }
    if (scriptDraftRequiresInlineShellAcknowledgement) {
      setPendingInlineShellAction({ type: "save" });
      return;
    }
    void saveScript(false);
  };

  const deleteScript = (record: PPScript) => {
    setPendingDeleteScript(record);
  };

  const executeToggleScript = useCallback(
    async (record: PPScript, inlineShellAcknowledged = false) => {
      setMutatingScriptId(record.id);
      try {
        const { error } = await client
          .mutation(togglePostProcessingScriptMutation, {
            id: record.id,
            ...(inlineShellAcknowledged ? { inlineShellAcknowledged: true } : {}),
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(
          t("settings.pp.toggled", {
            state: record.enabled ? t("label.disabled") : t("label.enabled"),
          }),
        );
        await refreshScripts();
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
      } finally {
        setMutatingScriptId(null);
      }
    },
    [client, refreshScripts, setGlobalStatus, t],
  );

  const toggleScript = useCallback(
    async (record: PPScript) => {
      if (record.scriptType === "inline" && !record.enabled) {
        setPendingInlineShellAction({ type: "toggle", record });
        return;
      }
      await executeToggleScript(record, false);
    },
    [executeToggleScript],
  );

  const confirmDeleteScript = async () => {
    if (!pendingDeleteScript) return;
    const record = pendingDeleteScript;
    setMutatingScriptId(record.id);
    try {
      const { error } = await client
        .mutation(deletePostProcessingScriptMutation, { id: record.id })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("settings.pp.deleted"));
      await refreshScripts();
      if (editingScriptId === record.id) {
        closeEditor();
      }
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingScriptId(null);
      setPendingDeleteScript(null);
    }
  };

  const confirmPendingEditorAction = useCallback(() => {
    if (!pendingEditorAction) return;
    if (pendingEditorAction.type === "create") {
      openCreateEditor();
    } else if (pendingEditorAction.type === "edit") {
      openEditEditor(pendingEditorAction.record);
    } else {
      closeEditor();
    }
    setPendingEditorAction(null);
  }, [closeEditor, openCreateEditor, openEditEditor, pendingEditorAction]);

  const confirmPendingInlineShellAction = useCallback(() => {
    const action = pendingInlineShellAction;
    if (!action) return;
    setPendingInlineShellAction(null);
    if (action.type === "save") {
      void saveScript(true);
    } else {
      void executeToggleScript(action.record, true);
    }
  }, [executeToggleScript, pendingInlineShellAction, saveScript]);

  return (
    <>
      <SettingsPostProcessingSection
        scripts={scripts}
        isEditorOpen={isEditorOpen}
        editorMode={editingScriptId ? "edit" : "create"}
        editingScriptId={editingScriptId}
        scriptDraft={scriptDraft}
        setScriptDraft={setScriptDraft}
        submitScript={submitScript}
        mutatingScriptId={mutatingScriptId}
        resetDraft={requestCloseEditor}
        startCreateScript={requestCreateEditor}
        editScript={requestEditScript}
        toggleScript={toggleScript}
        deleteScript={deleteScript}
        expandedScriptId={expandedScriptId}
        setExpandedScriptId={setExpandedScriptId}
        scriptRuns={scriptRuns}
        loadRunsForScript={loadRunsForScript}
      />
      <ConfirmDialog
        open={pendingDeleteScript !== null}
        title={t("label.delete")}
        description={
          pendingDeleteScript
            ? t("status.deletingRule", { name: pendingDeleteScript.name })
            : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={mutatingScriptId !== null}
        onConfirm={confirmDeleteScript}
        onCancel={() => setPendingDeleteScript(null)}
      />
      <ConfirmDialog
        open={pendingInlineShellAction !== null}
        title={t("settings.pp.inlineShellConfirmTitle")}
        description={t("settings.pp.inlineShellConfirmDescription")}
        confirmLabel={t("settings.pp.inlineShellConfirm")}
        cancelLabel={t("label.cancel")}
        contentId="settings-post-processing-inline-shell-confirm"
        confirmButtonId="settings-post-processing-inline-shell-confirm-accept"
        cancelButtonId="settings-post-processing-inline-shell-confirm-cancel"
        isBusy={mutatingScriptId !== null}
        onConfirm={confirmPendingInlineShellAction}
        onCancel={() => setPendingInlineShellAction(null)}
      />
      <ConfirmDialog
        open={pendingEditorAction !== null}
        title={t("settings.pp.confirmDiscardTitle")}
        description={t("settings.pp.confirmDiscardDescription")}
        confirmLabel={
          pendingEditorAction?.type === "create"
            ? t("settings.pp.createNewScript")
            : pendingEditorAction?.type === "edit"
              ? t("label.edit")
              : t("label.discard")
        }
        cancelLabel={t("label.cancel")}
        isBusy={mutatingScriptId !== null}
        onConfirm={confirmPendingEditorAction}
        onCancel={() => setPendingEditorAction(null)}
      />
    </>
  );
}
