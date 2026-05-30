import { type ComponentProps, type FormEvent, useCallback, useEffect, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsMediaServersSection } from "@/components/views/settings/settings-media-servers-section";
import {
  createMediaServerConnectionMutation,
  deleteMediaServerConnectionMutation,
  testMediaServerConnectionMutation,
  updateMediaServerConnectionMutation,
} from "@/lib/graphql/mutations";
import {
  librariesQuery,
  mediaServerConnectionsQuery,
} from "@/lib/graphql/queries";
import { useClient } from "urql";
import { toast } from "sonner";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  isReportedConnectionFeedbackError,
  runConnectionFeedback,
} from "@/lib/utils/connection-feedback";
import { notifyExternalAccountInviteSourcesChanged } from "@/components/containers/settings/external-account-invites-container";
import type {
  LibraryRecord,
  MediaServerConnection,
  MediaServerConnectionDraft,
  MediaServerPathMapping,
} from "@/lib/types";

type SettingsMediaServersSectionProps = ComponentProps<typeof SettingsMediaServersSection>;

type PendingMediaServerEditorAction =
  | { type: "create" }
  | { type: "edit"; connection: MediaServerConnection }
  | { type: "close" }
  | null;

const DEFAULT_MEDIA_SERVER_DRAFT: MediaServerConnectionDraft = {
  provider: "jellyfin",
  displayName: "Jellyfin",
  baseUrl: "",
  enabled: true,
  loginEnabled: false,
  linkingEnabled: false,
  autoAddEnabled: false,
  defaultAppPermissions: [],
  defaultLibraryGrants: [],
  machineId: "",
  apiKey: "",
  clearApiKey: false,
  adminUsername: "",
  adminPassword: "",
  pathMappingsText: "",
};

function cloneDraft(draft: MediaServerConnectionDraft): MediaServerConnectionDraft {
  return {
    ...draft,
    defaultAppPermissions: [...draft.defaultAppPermissions],
    defaultLibraryGrants: draft.defaultLibraryGrants.map((grant) => ({
      libraryId: grant.libraryId,
      permissions: [...grant.permissions],
    })),
  };
}

function normalizeOptional(value: string): string | undefined {
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : undefined;
}

function serializePathMappings(mappings: MediaServerPathMapping[]): string {
  return mappings
    .map((mapping) => `${mapping.sourcePath.trim()} => ${mapping.destinationPath.trim()}`)
    .filter((line) => line !== "=>")
    .join("\n");
}

function parsePathMappings(value: string): MediaServerPathMapping[] {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const [sourcePath, destinationPath = ""] = line.split(/=>/, 2);
      return {
        sourcePath: sourcePath.trim(),
        destinationPath: destinationPath.trim(),
      };
    })
    .filter((mapping) => mapping.sourcePath.length > 0 && mapping.destinationPath.length > 0);
}

function draftFromConnection(connection: MediaServerConnection): MediaServerConnectionDraft {
  return {
    provider: connection.provider,
    displayName: connection.displayName,
    baseUrl: connection.baseUrl,
    enabled: connection.enabled,
    loginEnabled: connection.loginEnabled,
    linkingEnabled: connection.linkingEnabled,
    autoAddEnabled: connection.autoAddEnabled,
    defaultAppPermissions: [...connection.defaultAppPermissions],
    defaultLibraryGrants: connection.defaultLibraryGrants.map((grant) => ({
      libraryId: grant.libraryId,
      permissions: [...grant.permissions],
    })),
    machineId: connection.machineId ?? "",
    apiKey: "",
    clearApiKey: false,
    adminUsername: "",
    adminPassword: "",
    pathMappingsText: serializePathMappings(connection.pathMappings),
  };
}

function buildCreateInput(draft: MediaServerConnectionDraft) {
  const supportsAuth = draft.provider === "jellyfin" || draft.provider === "plex";
  const input: Record<string, unknown> = {
    provider: draft.provider,
    displayName: draft.displayName.trim(),
    baseUrl: draft.baseUrl.trim(),
    enabled: draft.enabled,
    loginEnabled: supportsAuth && draft.loginEnabled,
    linkingEnabled: supportsAuth && draft.linkingEnabled,
    autoAddEnabled: supportsAuth && draft.autoAddEnabled,
    defaultAppPermissions: supportsAuth ? draft.defaultAppPermissions : [],
    defaultLibraryGrants: supportsAuth ? draft.defaultLibraryGrants : [],
    pathMappings: parsePathMappings(draft.pathMappingsText),
  };

  const machineId = normalizeOptional(draft.machineId);
  const apiKey = normalizeOptional(draft.apiKey);
  const adminUsername = normalizeOptional(draft.adminUsername);
  const adminPassword = normalizeOptional(draft.adminPassword);
  if (draft.provider === "plex" && machineId) input.machineId = machineId;
  if ((draft.provider === "jellyfin" || draft.provider === "emby") && apiKey) input.apiKey = apiKey;
  if (draft.provider === "jellyfin" && adminUsername) input.adminUsername = adminUsername;
  if (draft.provider === "jellyfin" && adminPassword) input.adminPassword = adminPassword;
  return input;
}

function buildUpdateInput(id: string, draft: MediaServerConnectionDraft) {
  const input = buildCreateInput(draft);
  const apiKey = normalizeOptional(draft.apiKey);
  input.id = id;
  if (!apiKey) delete input.apiKey;
  if (draft.clearApiKey) input.clearApiKey = true;
  if (!normalizeOptional(draft.machineId) && draft.provider === "plex") {
    input.clearMachineId = true;
  }
  return input;
}

export function SettingsMediaServersContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [connections, setConnections] =
    useState<SettingsMediaServersSectionProps["connections"]>([]);
  const [libraries, setLibraries] = useState<LibraryRecord[]>([]);
  const [draft, setDraft] = useState<MediaServerConnectionDraft>(() =>
    cloneDraft(DEFAULT_MEDIA_SERVER_DRAFT),
  );
  const [draftBaseline, setDraftBaseline] = useState<MediaServerConnectionDraft>(() =>
    cloneDraft(DEFAULT_MEDIA_SERVER_DRAFT),
  );
  const [editingConnectionId, setEditingConnectionId] = useState<string | null>(null);
  const [mutatingConnectionId, setMutatingConnectionId] = useState<string | null>(null);
  const [testingConnectionId, setTestingConnectionId] = useState<string | null>(null);
  const [isEditorOpen, setIsEditorOpen] = useState(false);
  const [editorMode, setEditorMode] = useState<"create" | "edit">("create");
  const [pendingDeleteConnection, setPendingDeleteConnection] =
    useState<MediaServerConnection | null>(null);
  const [pendingEditorAction, setPendingEditorAction] =
    useState<PendingMediaServerEditorAction>(null);

  const isDraftDirty = JSON.stringify(draft) !== JSON.stringify(draftBaseline);

  const refreshConnections = useCallback(async () => {
    const { data, error } = await client
      .query(mediaServerConnectionsQuery, { provider: null }, { requestPolicy: "network-only" })
      .toPromise();
    if (error) throw error;
    setConnections((data?.mediaServerConnections ?? []) as MediaServerConnection[]);
  }, [client]);

  const refreshLibraries = useCallback(async () => {
    const { data, error } = await client
      .query(librariesQuery, { facet: null, permission: "view" }, { requestPolicy: "network-only" })
      .toPromise();
    if (error) throw error;
    setLibraries((data?.libraries ?? []) as LibraryRecord[]);
  }, [client]);

  useEffect(() => {
    let cancelled = false;
    void Promise.all([refreshConnections(), refreshLibraries()])
      .catch((error: unknown) => {
        if (!cancelled) {
          setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [refreshConnections, refreshLibraries, setGlobalStatus, t]);

  const resetDraft = useCallback(() => {
    setEditingConnectionId(null);
    const nextDraft = cloneDraft(DEFAULT_MEDIA_SERVER_DRAFT);
    setDraft(nextDraft);
    setDraftBaseline(cloneDraft(nextDraft));
  }, []);

  const openCreateEditor = useCallback(() => {
    const nextDraft = cloneDraft(DEFAULT_MEDIA_SERVER_DRAFT);
    setEditingConnectionId(null);
    setDraft(nextDraft);
    setDraftBaseline(cloneDraft(nextDraft));
    setEditorMode("create");
    setIsEditorOpen(true);
  }, []);

  const openEditEditor = useCallback((connection: MediaServerConnection) => {
    const nextDraft = draftFromConnection(connection);
    setEditingConnectionId(connection.id);
    setDraft(nextDraft);
    setDraftBaseline(cloneDraft(nextDraft));
    setEditorMode("edit");
    setIsEditorOpen(true);
    setGlobalStatus(t("status.editingMediaServer", { name: connection.displayName }));
  }, [setGlobalStatus, t]);

  const requestCreateEditor = useCallback(() => {
    if (!isEditorOpen || !isDraftDirty) {
      openCreateEditor();
      return;
    }
    setPendingEditorAction({ type: "create" });
  }, [isDraftDirty, isEditorOpen, openCreateEditor]);

  const requestEditConnection = useCallback((connection: MediaServerConnection) => {
    if (!isEditorOpen || !isDraftDirty) {
      openEditEditor(connection);
      return;
    }
    setPendingEditorAction({ type: "edit", connection });
  }, [isDraftDirty, isEditorOpen, openEditEditor]);

  const requestCloseEditor = useCallback(() => {
    if (!isEditorOpen) {
      return;
    }
    if (!isDraftDirty) {
      setIsEditorOpen(false);
      setEditorMode("create");
      resetDraft();
      return;
    }
    setPendingEditorAction({ type: "close" });
  }, [isDraftDirty, isEditorOpen, resetDraft]);

  const confirmPendingEditorAction = useCallback(() => {
    if (!pendingEditorAction) {
      return;
    }
    if (pendingEditorAction.type === "create") {
      openCreateEditor();
    } else if (pendingEditorAction.type === "edit") {
      openEditEditor(pendingEditorAction.connection);
    } else {
      setIsEditorOpen(false);
      setEditorMode("create");
      resetDraft();
    }
    setPendingEditorAction(null);
  }, [openCreateEditor, openEditEditor, pendingEditorAction, resetDraft]);

  const submitConnection = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const name = draft.displayName.trim();
    const baseUrl = draft.baseUrl.trim();
    if (!name || (draft.provider !== "plex" && !baseUrl)) {
      setGlobalStatus(t("settings.mediaServerValidation"));
      return;
    }

    setMutatingConnectionId(editingConnectionId ?? "new");
    try {
      if (editingConnectionId) {
        const { error } = await client.mutation(updateMediaServerConnectionMutation, {
          input: buildUpdateInput(editingConnectionId, draft),
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.mediaServerUpdated"));
      } else {
        const { error } = await client.mutation(createMediaServerConnectionMutation, {
          input: buildCreateInput(draft),
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.mediaServerCreated"));
      }

      resetDraft();
      setIsEditorOpen(false);
      setEditorMode("create");
      await refreshConnections();
      notifyExternalAccountInviteSourcesChanged();
    } catch (error) {
      if (!isReportedConnectionFeedbackError(error)) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
      }
    } finally {
      setMutatingConnectionId(null);
    }
  };

  const testConnection = useCallback(async (connection: MediaServerConnection) => {
    setTestingConnectionId(connection.id);
    try {
      await runConnectionFeedback({
        setGlobalStatus,
        startMessage: t("status.testingMediaServerConnection", {
          server: connection.displayName,
        }),
        successMessage: t("status.mediaServerConnectionTestPassed", {
          server: connection.displayName,
        }),
        failureFallbackMessage: t("status.mediaServerConnectionTestFailed", {
          server: connection.displayName,
        }),
        run: async () => {
          const { data, error } = await client
            .mutation(testMediaServerConnectionMutation, { id: connection.id })
            .toPromise();
          if (error) throw error;
          if (!data?.testMediaServerConnection) {
            throw new Error(t("status.mediaServerConnectionTestFailed", {
              server: connection.displayName,
            }));
          }
        },
      });
      toast.success(t("status.mediaServerConnectionTestPassed", {
        server: connection.displayName,
      }));
    } catch {
      // Shared connection feedback already surfaced the failure.
    } finally {
      setTestingConnectionId(null);
    }
  }, [client, setGlobalStatus, t]);

  const toggleConnectionEnabled = useCallback(async (connection: MediaServerConnection) => {
    const nextEnabled = !connection.enabled;
    setMutatingConnectionId(connection.id);
    try {
      const { error } = await client.mutation(updateMediaServerConnectionMutation, {
        input: {
          id: connection.id,
          enabled: nextEnabled,
        },
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.mediaServerUpdated"));
      await refreshConnections();
      notifyExternalAccountInviteSourcesChanged();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingConnectionId(null);
    }
  }, [client, refreshConnections, setGlobalStatus, t]);

  const deleteConnection = useCallback((connection: MediaServerConnection) => {
    setPendingDeleteConnection(connection);
  }, []);

  const confirmDeleteConnection = useCallback(async () => {
    if (!pendingDeleteConnection) {
      return;
    }
    const connection = pendingDeleteConnection;
    setMutatingConnectionId(connection.id);
    try {
      const { error } = await client
        .mutation(deleteMediaServerConnectionMutation, { id: connection.id })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.mediaServerDeleted", { name: connection.displayName }));
      await refreshConnections();
      notifyExternalAccountInviteSourcesChanged();
      if (editingConnectionId === connection.id) {
        resetDraft();
        setIsEditorOpen(false);
        setEditorMode("create");
      }
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingConnectionId(null);
      setPendingDeleteConnection(null);
    }
  }, [
    client,
    editingConnectionId,
    pendingDeleteConnection,
    refreshConnections,
    resetDraft,
    setGlobalStatus,
    t,
  ]);

  return (
    <>
      <SettingsMediaServersSection
        connections={connections}
        libraries={libraries}
        draft={draft}
        setDraft={setDraft}
        editingConnectionId={editingConnectionId}
        mutatingConnectionId={mutatingConnectionId}
        testingConnectionId={testingConnectionId}
        isEditorOpen={isEditorOpen}
        editorMode={editorMode}
        submitConnection={submitConnection}
        editConnection={requestEditConnection}
        testConnection={testConnection}
        toggleConnectionEnabled={toggleConnectionEnabled}
        deleteConnection={deleteConnection}
        resetDraft={requestCloseEditor}
        startCreateConnection={requestCreateEditor}
      />
      <ConfirmDialog
        open={pendingEditorAction !== null}
        title={t("settings.mediaServerConfirmDiscardTitle")}
        description={t("settings.mediaServerConfirmDiscardDescription")}
        confirmLabel={
          pendingEditorAction?.type === "create"
            ? t("settings.mediaServerCreateNew")
            : pendingEditorAction?.type === "edit"
              ? t("label.edit")
              : t("label.discard")
        }
        cancelLabel={t("label.cancel")}
        isBusy={mutatingConnectionId !== null}
        onConfirm={confirmPendingEditorAction}
        onCancel={() => setPendingEditorAction(null)}
      />
      <ConfirmDialog
        open={pendingDeleteConnection !== null}
        title={t("label.delete")}
        description={
          pendingDeleteConnection
            ? t("status.deletingMediaServer", { name: pendingDeleteConnection.displayName })
            : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={mutatingConnectionId !== null}
        onConfirm={confirmDeleteConnection}
        onCancel={() => setPendingDeleteConnection(null)}
      />
    </>
  );
}
