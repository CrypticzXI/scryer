
import { type ComponentProps, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsDownloadClientsSection } from "@/components/views/settings/settings-download-clients-section";
import {
  createDownloadClientMutation,
  deleteDownloadClientMutation,
  reorderDownloadClientsMutation,
  testDownloadClientConnectionMutation,
  updateDownloadClientMutation,
} from "@/lib/graphql/mutations";
import {
  downloadClientProviderTypesQuery,
  downloadClientsInitQuery,
  downloadClientsQuery,
} from "@/lib/graphql/queries";
import { DEFAULT_DOWNLOAD_CLIENT_DRAFT } from "@/lib/constants/download-clients";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import {
  isReportedConnectionFeedbackError,
  runConnectionFeedback,
} from "@/lib/utils/connection-feedback";
import {
  buildDownloadClientBaseUrl,
  buildDownloadClientConfigJson,
  buildDownloadClientDraftFromRecord,
  buildDownloadClientTypeOptions,
  ensureDownloadClientTypeOption,
  isBuiltInDownloadClientType,
  normalizeDownloadClientType,
} from "@/lib/utils/download-clients";
import type {
  DownloadClientRecord,
  DownloadClientDraft,
  DownloadClientTypeOption,
  ProviderTypeInfo,
} from "@/lib/types";

type SettingsDownloadClientsSectionProps = ComponentProps<typeof SettingsDownloadClientsSection>;

type SettingsDownloadClientsContainerProps = {
  providerCatalogVersion?: number;
};

export function SettingsDownloadClientsContainer({
  providerCatalogVersion = 0,
}: SettingsDownloadClientsContainerProps) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [settingsDownloadClients, setSettingsDownloadClients] = useState<SettingsDownloadClientsSectionProps["settingsDownloadClients"]>(
    [],
  );
  const [downloadClientTypeOptions, setDownloadClientTypeOptions] = useState<DownloadClientTypeOption[]>(
    () => buildDownloadClientTypeOptions([]),
  );
  const [downloadClientDraft, setDownloadClientDraft] = useState<DownloadClientDraft>(() => ({
    ...DEFAULT_DOWNLOAD_CLIENT_DRAFT,
  }));
  const [editingDownloadClientId, setEditingDownloadClientId] = useState<string | null>(null);
  const [mutatingDownloadClientId, setMutatingDownloadClientId] = useState<string | null>(null);
  const [isTestingDownloadClientConnection, setIsTestingDownloadClientConnection] = useState(false);
  const [pendingDeleteDownloadClient, setPendingDeleteDownloadClient] = useState<DownloadClientRecord | null>(null);
  const [downloadClientOrder, setDownloadClientOrder] = useState<string[]>([]);
  const [isSavingOrder, setIsSavingOrder] = useState(false);
  const providerCatalogVersionRef = useRef(providerCatalogVersion);

  const getDownloadClientErrorMessage = useCallback(
    (error: unknown, fallback: string) => (error instanceof Error ? error.message : fallback),
    [],
  );

  const resetDownloadClientDraft = useCallback(() => {
    setEditingDownloadClientId(null);
    setDownloadClientDraft({
      ...DEFAULT_DOWNLOAD_CLIENT_DRAFT,
      isEnabled: true,
    });
  }, []);

  const refreshDownloadClients = useCallback(async () => {
    try {
      const { data, error } = await client
        .query(downloadClientsQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (error) throw error;
      const clients: DownloadClientRecord[] = data.downloadClientConfigs || [];
      setSettingsDownloadClients(clients);
      setDownloadClientOrder(clients.map((c) => c.id));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    }
  }, [client, setGlobalStatus, t]);

  const refreshProviderTypes = useCallback(async () => {
    const { data, error } = await client
      .query(downloadClientProviderTypesQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (error) throw error;
    setDownloadClientTypeOptions(
      buildDownloadClientTypeOptions(
        (data?.downloadClientProviderTypes as ProviderTypeInfo[] | undefined) ?? [],
      ),
    );
  }, [client]);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const { data, error } = await client
          .query(downloadClientsInitQuery, {}, { requestPolicy: "network-only" })
          .toPromise();
        if (error && !data?.downloadClientConfigs) throw error;
        if (cancelled) return;
        const clients: DownloadClientRecord[] = data?.downloadClientConfigs || [];
        setSettingsDownloadClients(clients);
        setDownloadClientOrder(clients.map((clientRecord) => clientRecord.id));
        setDownloadClientTypeOptions(
          buildDownloadClientTypeOptions(
            (data?.downloadClientProviderTypes as ProviderTypeInfo[] | undefined) ?? [],
          ),
        );
      } catch (error) {
        setDownloadClientTypeOptions(buildDownloadClientTypeOptions([]));
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
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
    void refreshProviderTypes().catch((error: unknown) => {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    });
  }, [providerCatalogVersion, refreshProviderTypes, setGlobalStatus, t]);

  useEffect(() => {
    if (editingDownloadClientId) {
      return;
    }

    setDownloadClientDraft((prev) => {
      const normalizedClientType = normalizeDownloadClientType(prev.clientType);
      const configuredOption =
        downloadClientTypeOptions.find(
          (option) => option.value === normalizedClientType,
        ) ?? null;
      const nextOption = configuredOption ?? downloadClientTypeOptions[0] ?? null;

      if (!nextOption) {
        return prev;
      }

      const previousLabel = configuredOption?.label ?? prev.clientType.trim();
      const shouldAutofillName =
        prev.name.trim().length === 0 || prev.name === previousLabel;
      const nextClientType = configuredOption ? prev.clientType : nextOption.value;
      const nextName = shouldAutofillName ? nextOption.label : prev.name;

      if (nextClientType === prev.clientType && nextName === prev.name) {
        return prev;
      }

      return {
        ...prev,
        clientType: nextClientType,
        name: nextName,
      };
    });
  }, [downloadClientTypeOptions, editingDownloadClientId]);

  const availableDownloadClientTypeOptions = useMemo(
    () => ensureDownloadClientTypeOption(downloadClientTypeOptions, downloadClientDraft.clientType),
    [downloadClientDraft.clientType, downloadClientTypeOptions],
  );

  const selectedDownloadClientLabel = useMemo(() => {
    const normalizedClientType = normalizeDownloadClientType(downloadClientDraft.clientType, "");
    const configuredClientLabel = downloadClientDraft.clientType.trim();
    return (
      availableDownloadClientTypeOptions.find((option) => option.value === normalizedClientType)?.label ??
      (configuredClientLabel || "Download client")
    );
  }, [availableDownloadClientTypeOptions, downloadClientDraft.clientType]);

  const submitDownloadClient = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const payload = {
      name: downloadClientDraft.name.trim(),
      clientType: normalizeDownloadClientType(downloadClientDraft.clientType),
      host: downloadClientDraft.host.trim(),
      port: downloadClientDraft.port.trim(),
      configJson: buildDownloadClientConfigJson(downloadClientDraft),
      isEnabled: downloadClientDraft.isEnabled,
    };

    if (!payload.name || !payload.host) {
      const message = t("settings.downloadClientValidation");
      setGlobalStatus(message);
      return;
    }

    setMutatingDownloadClientId(editingDownloadClientId || "new");
    try {
      if (isBuiltInDownloadClientType(payload.clientType)) {
        await runConnectionFeedback({
          setGlobalStatus,
          startMessage: t("status.testingDownloadClient", {
            client: selectedDownloadClientLabel,
          }),
          successMessage: t("status.downloadClientConnectionTestPassed", {
            client: selectedDownloadClientLabel,
          }),
          failureFallbackMessage: t("status.downloadClientConnectionTestFailed", {
            client: selectedDownloadClientLabel,
          }),
          announceSuccess: false,
          run: async () => {
            const { data: testData, error: testError } = await client
              .mutation(testDownloadClientConnectionMutation, {
                input: {
                  clientType: payload.clientType,
                  configJson: payload.configJson,
                },
              })
              .toPromise();
            if (testError) throw testError;
            if (!testData.testDownloadClientConnection) {
              throw new Error(
                t("status.downloadClientConnectionTestFailed", {
                  client: selectedDownloadClientLabel,
                }),
              );
            }
          },
        });
      }

      if (editingDownloadClientId) {
        const { error } = await client.mutation(updateDownloadClientMutation, {
          input: {
            id: editingDownloadClientId,
            name: payload.name,
            clientType: payload.clientType,
            configJson: payload.configJson,
            isEnabled: payload.isEnabled,
          },
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.downloadClientUpdated"));
      } else {
        const { error } = await client.mutation(
          createDownloadClientMutation,
          {
            input: {
              name: payload.name,
              clientType: payload.clientType,
              configJson: payload.configJson,
              isEnabled: payload.isEnabled,
            },
          },
        ).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.downloadClientCreated"));
      }
      resetDownloadClientDraft();
      await refreshDownloadClients();
    } catch (error) {
      if (!isReportedConnectionFeedbackError(error)) {
        const message = getDownloadClientErrorMessage(
          error,
          t("status.failedToUpdate"),
        );
        setGlobalStatus(message);
      }
    } finally {
      setMutatingDownloadClientId(null);
    }
  };

  const testDownloadClientConnection = async () => {
    const payload = {
      name: downloadClientDraft.name.trim(),
      clientType: normalizeDownloadClientType(downloadClientDraft.clientType),
      host: downloadClientDraft.host.trim(),
      baseUrl: buildDownloadClientBaseUrl(downloadClientDraft),
      configJson: buildDownloadClientConfigJson(downloadClientDraft),
    };

    if (!payload.name || !payload.host) {
      const message = t("settings.downloadClientValidation");
      setGlobalStatus(message);
      return;
    }

    if (!payload.baseUrl) {
      const message = t("settings.downloadClientBaseUrlRequired");
      setGlobalStatus(message);
      return;
    }

    setIsTestingDownloadClientConnection(true);
    try {
      await runConnectionFeedback({
        setGlobalStatus,
        startMessage: t("status.testingDownloadClient", {
          client: selectedDownloadClientLabel,
        }),
        successMessage: t("status.downloadClientConnectionTestPassed", {
          client: selectedDownloadClientLabel,
        }),
        failureFallbackMessage: t("status.downloadClientConnectionTestFailed", {
          client: selectedDownloadClientLabel,
        }),
        run: async () => {
          const { data: testData, error: testError } = await client
            .mutation(testDownloadClientConnectionMutation, {
              input: {
                clientType: payload.clientType,
                configJson: payload.configJson,
              },
            })
            .toPromise();
          if (testError) throw testError;
          if (!testData.testDownloadClientConnection) {
            throw new Error(
              t("status.downloadClientConnectionTestFailed", {
                client: selectedDownloadClientLabel,
              }),
            );
          }
        },
      });
    } catch {
      // Connection feedback is already surfaced through the shared helper.
    } finally {
      setIsTestingDownloadClientConnection(false);
    }
  };

  const moveDownloadClient = useCallback(async (clientId: string, direction: "up" | "down") => {
    if (isSavingOrder) {
      return;
    }

    const currentOrder =
      downloadClientOrder.length > 0
        ? downloadClientOrder
        : settingsDownloadClients.map((downloadClient) => downloadClient.id);
    const index = currentOrder.indexOf(clientId);
    if (index < 0) {
      return;
    }

    const nextIndex = direction === "up" ? index - 1 : index + 1;
    if (nextIndex < 0 || nextIndex >= currentOrder.length) {
      return;
    }

    const nextOrder = [...currentOrder];
    [nextOrder[index], nextOrder[nextIndex]] = [nextOrder[nextIndex], nextOrder[index]];
    setDownloadClientOrder(nextOrder);
    setIsSavingOrder(true);

    try {
      const { error } = await client.mutation(reorderDownloadClientsMutation, {
        input: { ids: nextOrder },
      }).toPromise();
      if (error) {
        throw error;
      }
      await refreshDownloadClients();
    } catch (error) {
      setDownloadClientOrder(currentOrder);
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setIsSavingOrder(false);
    }
  }, [client, downloadClientOrder, isSavingOrder, refreshDownloadClients, setGlobalStatus, settingsDownloadClients, t]);

  const editDownloadClient = useCallback((downloadClient: DownloadClientRecord) => {
    setEditingDownloadClientId(downloadClient.id);
    setDownloadClientDraft(buildDownloadClientDraftFromRecord(downloadClient));
    setGlobalStatus(t("status.editingDownloadClient", { name: downloadClient.name }));
  }, [setGlobalStatus, t]);

  const toggleDownloadClientEnabled = useCallback(async (downloadClient: DownloadClientRecord) => {
    const nextIsEnabled = !downloadClient.isEnabled;
    setMutatingDownloadClientId(downloadClient.id);
    try {
      const { error } = await client.mutation(updateDownloadClientMutation, {
        input: {
          id: downloadClient.id,
          isEnabled: nextIsEnabled,
        },
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.downloadClientUpdated"));
      await refreshDownloadClients();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingDownloadClientId(null);
    }
  }, [client, refreshDownloadClients, setGlobalStatus, t]);

  const deleteDownloadClient = useCallback(async (downloadClient: DownloadClientRecord) => {
    setPendingDeleteDownloadClient(downloadClient);
  }, []);

  const confirmDeleteDownloadClient = useCallback(async () => {
    if (!pendingDeleteDownloadClient) {
      return;
    }
    const downloadClient = pendingDeleteDownloadClient;
    setMutatingDownloadClientId(downloadClient.id);
    try {
      const { error } = await client.mutation(deleteDownloadClientMutation, {
        input: { id: downloadClient.id },
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.downloadClientDeleted", { name: downloadClient.name }));
      await refreshDownloadClients();
      if (editingDownloadClientId === downloadClient.id) {
        resetDownloadClientDraft();
      }
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingDownloadClientId(null);
      setPendingDeleteDownloadClient(null);
    }
  }, [editingDownloadClientId, pendingDeleteDownloadClient, refreshDownloadClients, resetDownloadClientDraft, client, setGlobalStatus, t]);

  return (
    <>
      <SettingsDownloadClientsSection
        editingDownloadClientId={editingDownloadClientId}
        downloadClientTypeOptions={availableDownloadClientTypeOptions}
        downloadClientDraft={downloadClientDraft}
        setDownloadClientDraft={setDownloadClientDraft}
        submitDownloadClient={submitDownloadClient}
        testDownloadClientConnection={testDownloadClientConnection}
        isTestingDownloadClientConnection={isTestingDownloadClientConnection}
        mutatingDownloadClientId={mutatingDownloadClientId}
        resetDownloadClientDraft={resetDownloadClientDraft}
        settingsDownloadClients={settingsDownloadClients}
        editDownloadClient={editDownloadClient}
        toggleDownloadClientEnabled={toggleDownloadClientEnabled}
        deleteDownloadClient={deleteDownloadClient}
        downloadClientOrder={downloadClientOrder}
        moveDownloadClient={moveDownloadClient}
        isSavingOrder={isSavingOrder}
      />
      <ConfirmDialog
        open={pendingDeleteDownloadClient !== null}
        title={t("label.delete")}
        description={
          pendingDeleteDownloadClient
            ? t("status.deletingDownloadClient", { name: pendingDeleteDownloadClient.name })
            : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={mutatingDownloadClientId !== null}
        onConfirm={confirmDeleteDownloadClient}
        onCancel={() => setPendingDeleteDownloadClient(null)}
      />
    </>
  );
}
