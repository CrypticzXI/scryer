import {
  type ComponentProps,
  type FormEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsIndexersSection } from "@/components/views/settings/settings-indexers-section";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import type { ConfigFieldDef, IndexerRecord, ProviderTypeInfo } from "@/lib/types";
import { useIndexersSubscription } from "@/lib/hooks/use-indexers-subscription";
import { runConnectionFeedback } from "@/lib/utils/connection-feedback";
import {
  indexerProviderTypesQuery,
  indexersInitQuery,
  indexersQuery,
} from "@/lib/graphql/queries";
import {
  createIndexerMutation,
  deleteIndexerMutation,
  syncIndexerConfigMutation,
  testIndexerConnectionMutation,
  updateIndexerMutation,
} from "@/lib/graphql/mutations";
import {
  providerConfigRecordToValues,
  providerConfigValuesToRecord,
} from "@/lib/utils/provider-config";

type SettingsIndexersSectionProps = ComponentProps<
  typeof SettingsIndexersSection
>;

const INDEXER_INITIAL_DRAFT = {
  name: "",
  providerType: "",
  storedSecretKeys: [] as string[],
  isEnabled: true,
  enableInteractiveSearch: true,
  enableAutoSearch: true,
  configValues: {} as Record<string, string>,
};

function serializeConfigValues(
  fields: ConfigFieldDef[],
  configValues: Record<string, string>,
  storedSecretKeys: string[] = [],
): ReturnType<typeof providerConfigRecordToValues> | undefined {
  const entries: Record<string, string> = {};
  const storedSecretKeySet = new Set(storedSecretKeys);

  if (fields.length === 0) {
    for (const [key, value] of Object.entries(configValues)) {
      if (value.trim() !== "") {
        entries[key] = value;
      }
    }
    return Object.keys(entries).length > 0
      ? providerConfigRecordToValues(entries)
      : undefined;
  }

  const fieldKeySet = new Set(fields.map((field) => field.key));
  const secretInputKeys = fields
    .filter((field) => field.fieldType === "password")
    .map((field) => field.key);
  for (const [key, value] of Object.entries(configValues)) {
    if (!fieldKeySet.has(key) && value.trim() !== "") {
      entries[key] = value;
    }
  }

  for (const field of fields) {
    if (field.valueSource === "host_binding") {
      continue;
    }

    const isStoredSecret =
      field.fieldType === "password" && storedSecretKeySet.has(field.key);
    let nextValue =
      configValues[field.key] ??
      field.defaultValue ??
      (field.fieldType === "bool" ? "false" : "");

    if (isStoredSecret && nextValue.trim() === "") {
      continue;
    }

    if (field.fieldType === "bool") {
      entries[field.key] = nextValue.trim() || field.defaultValue || "false";
      continue;
    }

    if (nextValue.trim() === "" && field.defaultValue) {
      nextValue = field.defaultValue;
    }

    if (nextValue.trim() !== "") {
      entries[field.key] = nextValue;
    }
  }

  return Object.keys(entries).length > 0
    ? providerConfigRecordToValues(entries, secretInputKeys)
    : undefined;
}

function buildDraftConfigValues(
  fields: ConfigFieldDef[],
  parsedConfigValues: Record<string, string>,
  storedSecretKeys: string[] = [],
): Record<string, string> {
  if (fields.length === 0) {
    return { ...parsedConfigValues };
  }

  const nextValues = { ...parsedConfigValues };
  const storedSecretKeySet = new Set(storedSecretKeys);
  for (const field of fields) {
    if (field.valueSource === "host_binding") {
      continue;
    }

    if (field.fieldType === "password" && storedSecretKeySet.has(field.key)) {
      nextValues[field.key] = "";
      continue;
    }

    nextValues[field.key] =
      parsedConfigValues[field.key] ??
      field.defaultValue ??
      (field.fieldType === "bool" ? "false" : "");
  }

  return nextValues;
}

function findMissingRequiredConfigField(
  fields: ConfigFieldDef[],
  configValues: Record<string, string>,
  storedSecretKeys: string[] = [],
): ConfigFieldDef | null {
  const storedSecretKeySet = new Set(storedSecretKeys);
  for (const field of fields) {
    if (!field.required || field.valueSource === "host_binding") {
      continue;
    }

    const nextValue =
      configValues[field.key] ??
      field.defaultValue ??
      (field.fieldType === "bool" ? "false" : "");

    if (
      field.fieldType === "password" &&
      storedSecretKeySet.has(field.key) &&
      nextValue.trim() === ""
    ) {
      continue;
    }

    if (field.fieldType !== "bool" && nextValue.trim() === "") {
      return field;
    }
  }

  return null;
}

type SettingsIndexersContainerProps = {
  providerCatalogVersion?: number;
};

type PendingIndexerEditorAction =
  | { type: "create" }
  | { type: "edit"; indexer: IndexerRecord }
  | { type: "close" }
  | null;

function cloneIndexerDraft(
  draft: SettingsIndexersSectionProps["indexerDraft"],
): SettingsIndexersSectionProps["indexerDraft"] {
  return {
    ...draft,
    storedSecretKeys: [...draft.storedSecretKeys],
    configValues: { ...draft.configValues },
  };
}

export function SettingsIndexersContainer({
  providerCatalogVersion = 0,
}: SettingsIndexersContainerProps) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [settingsIndexers, setSettingsIndexers] = useState<IndexerRecord[]>([]);
  const [settingsIndexerFilter, setSettingsIndexerFilter] = useState("");
  const [mutatingIndexerId, setMutatingIndexerId] = useState<string | null>(
    null,
  );
  const [editingIndexerId, setEditingIndexerId] = useState<string | null>(null);
  const [pendingDeleteIndexer, setPendingDeleteIndexer] =
    useState<IndexerRecord | null>(null);
  const [isTestingConnection, setIsTestingConnection] = useState(false);
  const [providerTypes, setProviderTypes] = useState<ProviderTypeInfo[]>([]);
  const [indexerDraft, setIndexerDraft] = useState<
    SettingsIndexersSectionProps["indexerDraft"]
  >(() => cloneIndexerDraft(INDEXER_INITIAL_DRAFT));
  const [isEditorOpen, setIsEditorOpen] = useState(false);
  const [editorMode, setEditorMode] = useState<"create" | "edit">("create");
  const [pendingEditorAction, setPendingEditorAction] =
    useState<PendingIndexerEditorAction>(null);
  const [draftBaseline, setDraftBaseline] = useState<
    SettingsIndexersSectionProps["indexerDraft"]
  >(() => cloneIndexerDraft(INDEXER_INITIAL_DRAFT));
  const [awaitingBaselineSync, setAwaitingBaselineSync] = useState(false);
  const didMountRef = useRef(false);
  const providerCatalogVersionRef = useRef(providerCatalogVersion);

  const resetIndexerDraft = useCallback(() => {
    setEditingIndexerId(null);
    setIndexerDraft(() => cloneIndexerDraft(INDEXER_INITIAL_DRAFT));
  }, []);

  useEffect(() => {
    if (!awaitingBaselineSync) {
      return;
    }

    setDraftBaseline(cloneIndexerDraft(indexerDraft));
    setAwaitingBaselineSync(false);
  }, [awaitingBaselineSync, indexerDraft]);

  const isDraftDirty =
    JSON.stringify(indexerDraft) !== JSON.stringify(draftBaseline);

  const refreshIndexers = useCallback(async () => {
    try {
      const { data, error } = await client
        .query(indexersQuery, {
          providerType: settingsIndexerFilter || undefined,
        }, {
          requestPolicy: "network-only",
        })
        .toPromise();
      if (error) throw error;
      setSettingsIndexers(data.indexers || []);
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
    }
  }, [client, settingsIndexerFilter, setGlobalStatus, t]);

  const refreshProviderTypes = useCallback(async () => {
    const { data, error } = await client
      .query(indexerProviderTypesQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (error) throw error;
    setProviderTypes(data?.indexerProviderTypes || []);
  }, [client]);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const { data, error } = await client
          .query(indexersInitQuery, {}, { requestPolicy: "network-only" })
          .toPromise();
        if (error && !data?.indexers) throw error;
        if (cancelled) return;
        setSettingsIndexers(data?.indexers || []);
        setProviderTypes(data?.indexerProviderTypes || []);
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToLoad"),
        );
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    if (providerCatalogVersion === providerCatalogVersionRef.current) {
      return;
    }

    providerCatalogVersionRef.current = providerCatalogVersion;
    void Promise.all([refreshProviderTypes(), refreshIndexers()]).catch((error: unknown) => {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
    });
  }, [
    providerCatalogVersion,
    refreshIndexers,
    refreshProviderTypes,
    setGlobalStatus,
    t,
  ]);

  useEffect(() => {
    if (editingIndexerId || providerTypes.length === 0) {
      return;
    }

    setIndexerDraft((prev) => {
      const configuredProvider =
        providerTypes.find(
          (providerType) => providerType.providerType === prev.providerType,
        ) ?? null;
      const nextProvider = configuredProvider ?? providerTypes[0] ?? null;
      if (!nextProvider) {
        return prev;
      }

      const shouldAutofillName =
        prev.name.trim().length === 0 ||
        prev.name === (configuredProvider?.name ?? prev.providerType);
      const nextProviderType = configuredProvider
        ? prev.providerType
        : nextProvider.providerType;
      const nextName = shouldAutofillName ? nextProvider.name : prev.name;

      if (nextProviderType === prev.providerType && nextName === prev.name) {
        return prev;
      }

      return {
        ...prev,
        providerType: nextProviderType,
        name: nextName,
        configValues:
          nextProviderType === prev.providerType
            ? prev.configValues
            : buildDraftConfigValues(nextProvider.configFields, {}),
      };
    });
  }, [editingIndexerId, providerTypes]);

  useEffect(() => {
    if (!didMountRef.current) {
      didMountRef.current = true;
      return;
    }
    void refreshIndexers();
  }, [refreshIndexers]);

  useIndexersSubscription(
    useCallback(() => {
      void refreshIndexers();
    }, [refreshIndexers]),
  );

  const openCreateEditor = useCallback(() => {
    resetIndexerDraft();
    setEditorMode("create");
    setIsEditorOpen(true);
    setAwaitingBaselineSync(true);
  }, [resetIndexerDraft]);

  const submitIndexer = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const normalizedProviderType = indexerDraft.providerType.trim().toLowerCase();
    const selectedProvider =
      providerTypes.find((pt) => pt.providerType === normalizedProviderType) ?? null;
    const missingRequiredConfigField = findMissingRequiredConfigField(
      selectedProvider?.configFields ?? [],
      indexerDraft.configValues,
      indexerDraft.storedSecretKeys,
    );
    const payload = {
      name: indexerDraft.name.trim(),
      providerType: normalizedProviderType,
      isEnabled: indexerDraft.isEnabled,
      enableInteractiveSearch: indexerDraft.enableInteractiveSearch,
      enableAutoSearch: indexerDraft.enableAutoSearch,
      config: serializeConfigValues(
        selectedProvider?.configFields ?? [],
        indexerDraft.configValues,
        indexerDraft.storedSecretKeys,
      ),
    };

    if (!payload.name || !payload.providerType) {
      setGlobalStatus(t("form.indexerValidation"));
      return;
    }

    if (missingRequiredConfigField) {
      setGlobalStatus(`${missingRequiredConfigField.label}: ${t("setup.required")}`);
      return;
    }

    setMutatingIndexerId(editingIndexerId || "new");
    try {
      if (editingIndexerId) {
        const { error } = await client
          .mutation(updateIndexerMutation, {
            input: {
              id: editingIndexerId,
              name: payload.name,
              providerType: payload.providerType,
              isEnabled: payload.isEnabled,
              enableInteractiveSearch: payload.enableInteractiveSearch,
              enableAutoSearch: payload.enableAutoSearch,
              config: payload.config,
            },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.indexerUpdated"));
      } else {
        const { error } = await client
          .mutation(createIndexerMutation, {
            input: {
              name: payload.name,
              providerType: payload.providerType,
              isEnabled: payload.isEnabled,
              enableInteractiveSearch: payload.enableInteractiveSearch,
              enableAutoSearch: payload.enableAutoSearch,
              config: payload.config,
            },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.indexerCreated"));
      }
      resetIndexerDraft();
      setIsEditorOpen(false);
      setEditorMode("create");
      setAwaitingBaselineSync(true);
      await refreshIndexers();
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToUpdate"),
      );
    } finally {
      setMutatingIndexerId(null);
    }
  };

  const editIndexer = useCallback((indexer: IndexerRecord) => {
    if (indexer.isManaged) {
      setGlobalStatus(t("settings.managedIndexerReadOnly"));
      return;
    }
    const selectedProvider =
      providerTypes.find(
        (providerType) =>
          providerType.providerType === indexer.providerType.trim().toLowerCase(),
      ) ?? null;
    const parsedConfigValues = providerConfigValuesToRecord(indexer.config);
    setEditingIndexerId(indexer.id);
    setIndexerDraft({
      name: indexer.name,
      providerType: indexer.providerType,
      storedSecretKeys: indexer.storedSecretKeys,
      isEnabled: indexer.isEnabled,
      enableInteractiveSearch: indexer.enableInteractiveSearch,
      enableAutoSearch: indexer.enableAutoSearch,
      configValues: buildDraftConfigValues(
        selectedProvider?.configFields ?? [],
        parsedConfigValues,
        indexer.storedSecretKeys,
      ),
    });
    setGlobalStatus(t("status.editingIndexer", { name: indexer.name }));
  }, [providerTypes, setGlobalStatus, t]);

  const openEditEditor = useCallback((indexer: IndexerRecord) => {
    editIndexer(indexer);
    setEditorMode("edit");
    setIsEditorOpen(true);
    setAwaitingBaselineSync(true);
  }, [editIndexer]);

  const requestCreateEditor = useCallback(() => {
    if (!isEditorOpen || !isDraftDirty) {
      openCreateEditor();
      return;
    }

    setPendingEditorAction({ type: "create" });
  }, [isDraftDirty, isEditorOpen, openCreateEditor]);

  const requestEditIndexer = useCallback((indexer: IndexerRecord) => {
    if (!isEditorOpen || !isDraftDirty) {
      openEditEditor(indexer);
      return;
    }

    setPendingEditorAction({ type: "edit", indexer });
  }, [isDraftDirty, isEditorOpen, openEditEditor]);

  const requestCloseEditor = useCallback(() => {
    if (!isEditorOpen) {
      return;
    }

    if (!isDraftDirty) {
      setIsEditorOpen(false);
      setEditorMode("create");
      resetIndexerDraft();
      setAwaitingBaselineSync(true);
      return;
    }

    setPendingEditorAction({ type: "close" });
  }, [isDraftDirty, isEditorOpen, resetIndexerDraft]);

  const confirmPendingEditorAction = useCallback(() => {
    if (!pendingEditorAction) {
      return;
    }

    if (pendingEditorAction.type === "create") {
      openCreateEditor();
    } else if (pendingEditorAction.type === "edit") {
      openEditEditor(pendingEditorAction.indexer);
    } else {
      setIsEditorOpen(false);
      setEditorMode("create");
      resetIndexerDraft();
      setAwaitingBaselineSync(true);
    }

    setPendingEditorAction(null);
  }, [openCreateEditor, openEditEditor, pendingEditorAction, resetIndexerDraft]);

  const deleteIndexer = async (indexer: IndexerRecord) => {
    if (indexer.isManaged) {
      setGlobalStatus(t("settings.managedIndexerReadOnly"));
      return;
    }
    setPendingDeleteIndexer(indexer);
  };

  const toggleIndexerEnabled = useCallback(
    async (indexer: IndexerRecord) => {
      if (indexer.isManaged) {
        setGlobalStatus(t("settings.managedIndexerReadOnly"));
        return;
      }
      const nextIsEnabled = !indexer.isEnabled;
      setMutatingIndexerId(indexer.id);
      try {
        const { error } = await client
          .mutation(updateIndexerMutation, {
            input: {
              id: indexer.id,
              isEnabled: nextIsEnabled,
            },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.indexerUpdated"));
        await refreshIndexers();
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setMutatingIndexerId(null);
      }
    },
    [client, refreshIndexers, setGlobalStatus, t],
  );

  const syncIndexer = useCallback(
    async (indexer: IndexerRecord) => {
      if (!indexer.supportsManagedChildrenSync || indexer.isManaged) {
        return;
      }
      setMutatingIndexerId(indexer.id);
      try {
        const { error } = await client
          .mutation(syncIndexerConfigMutation, {
            id: indexer.id,
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.indexerSynced", { name: indexer.name }));
        await refreshIndexers();
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setMutatingIndexerId(null);
      }
    },
    [client, refreshIndexers, setGlobalStatus, t],
  );

  const confirmDeleteIndexer = async () => {
    if (!pendingDeleteIndexer) {
      return;
    }
    const indexer = pendingDeleteIndexer;
    setMutatingIndexerId(indexer.id);
    try {
      const { error } = await client
        .mutation(deleteIndexerMutation, {
          id: indexer.id,
        })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.indexerDeleted", { name: indexer.name }));
      await refreshIndexers();
      if (editingIndexerId === indexer.id) {
        resetIndexerDraft();
        setIsEditorOpen(false);
        setEditorMode("create");
        setAwaitingBaselineSync(true);
      }
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToDelete"),
      );
    } finally {
      setMutatingIndexerId(null);
      setPendingDeleteIndexer(null);
    }
  };

  const testIndexerConnection = async () => {
    const normalizedProviderType = indexerDraft.providerType.trim().toLowerCase();
    const selectedProvider =
      providerTypes.find((pt) => pt.providerType === normalizedProviderType) ?? null;
    const missingRequiredConfigField = findMissingRequiredConfigField(
      selectedProvider?.configFields ?? [],
      indexerDraft.configValues,
      indexerDraft.storedSecretKeys,
    );
    const payload = {
      providerType: normalizedProviderType,
      config: serializeConfigValues(
        selectedProvider?.configFields ?? [],
        indexerDraft.configValues,
        indexerDraft.storedSecretKeys,
      ),
      indexerId: editingIndexerId ?? undefined,
    };

    if (!payload.providerType) {
      setGlobalStatus(t("form.indexerValidation"));
      return;
    }
    if (missingRequiredConfigField) {
      setGlobalStatus(`${missingRequiredConfigField.label}: ${t("setup.required")}`);
      return;
    }
    setIsTestingConnection(true);
    try {
      await runConnectionFeedback({
        setGlobalStatus,
        startMessage: t("status.testingIndexerConnection"),
        successMessage: t("status.indexerConnectionTestPassed"),
        failureFallbackMessage: t("status.indexerConnectionTestFailed"),
        run: async () => {
          const { data: testData, error: testError } = await client
            .mutation(testIndexerConnectionMutation, { input: payload })
            .toPromise();
          if (testError) throw testError;
          const validation = testData?.testIndexerConnection;
          if (validation?.status !== "ok") {
            throw new Error(
              validation?.message ?? t("status.indexerConnectionTestFailed"),
            );
          }
        },
      });
    } catch {
      // Connection feedback is already surfaced through the shared helper.
    } finally {
      setIsTestingConnection(false);
    }
  };

  return (
    <>
      <SettingsIndexersSection
        editingIndexerId={editingIndexerId}
        indexerDraft={indexerDraft}
        setIndexerDraft={setIndexerDraft}
        submitIndexer={submitIndexer}
        mutatingIndexerId={mutatingIndexerId}
        resetIndexerDraft={requestCloseEditor}
        settingsIndexerFilter={settingsIndexerFilter}
        setSettingsIndexerFilter={setSettingsIndexerFilter}
        settingsIndexers={settingsIndexers}
        editIndexer={requestEditIndexer}
        toggleIndexerEnabled={toggleIndexerEnabled}
        deleteIndexer={deleteIndexer}
        syncIndexer={syncIndexer}
        providerTypes={providerTypes}
        testIndexerConnection={testIndexerConnection}
        isTestingConnection={isTestingConnection}
        isEditorOpen={isEditorOpen}
        editorMode={editorMode}
        startCreateIndexer={requestCreateEditor}
      />
      <ConfirmDialog
        open={pendingEditorAction !== null}
        title={t("settings.indexerConfirmDiscardTitle")}
        description={t("settings.indexerConfirmDiscardDescription")}
        confirmLabel={
          pendingEditorAction?.type === "create"
            ? t("settings.indexerCreateNew")
            : pendingEditorAction?.type === "edit"
              ? t("label.edit")
              : t("label.discard")
        }
        cancelLabel={t("label.cancel")}
        isBusy={mutatingIndexerId !== null}
        onConfirm={confirmPendingEditorAction}
        onCancel={() => setPendingEditorAction(null)}
      />
      <ConfirmDialog
        open={pendingDeleteIndexer !== null}
        contentId="settings-indexer-delete-dialog"
        title={t("label.delete")}
        description={
          pendingDeleteIndexer
            ? t("status.deletingIndexer", { name: pendingDeleteIndexer.name })
            : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        confirmButtonId="settings-indexer-delete-confirm"
        cancelButtonId="settings-indexer-delete-cancel"
        isBusy={mutatingIndexerId !== null}
        onConfirm={confirmDeleteIndexer}
        onCancel={() => setPendingDeleteIndexer(null)}
      />
    </>
  );
}
