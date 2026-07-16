import { useCallback, useEffect, useMemo, useState } from "react";
import type { Client } from "urql";

import {
  createDownloadClientMutation,
  testDownloadClientConnectionMutation,
} from "@/lib/graphql/mutations";
import {
  DEFAULT_DOWNLOAD_CLIENT_DRAFT,
  DEFAULT_PORT_FOR_CLIENT_TYPE,
} from "@/lib/constants/download-clients";
import {
  buildDownloadClientConfigValues,
  buildDownloadClientTypeOptions,
  defaultDownloadClientConfigValuesForFields,
  ensureDownloadClientTypeOption,
  normalizeDownloadClientType,
} from "@/lib/utils/download-clients";
import type { LocalPathStyle } from "@/lib/utils/local-path-style";
import type {
  DownloadClientDraft,
  DownloadClientTypeOption,
} from "@/lib/types/download-clients";

interface UseDownloadClientSetupArgs {
  client: Client;
}

export function useDownloadClientSetup({ client }: UseDownloadClientSetupArgs) {
  // ── Step 4 (fresh): Download Client ─────────────────────────────────
  const [dcDraft, setDcDraft] = useState<DownloadClientDraft>({
    ...DEFAULT_DOWNLOAD_CLIENT_DRAFT,
  });
  const [dcTypeOptions, setDcTypeOptions] = useState<
    DownloadClientTypeOption[]
  >(() => buildDownloadClientTypeOptions([]));
  const [dcLocalPathStyle, setDcLocalPathStyle] =
    useState<LocalPathStyle | undefined>(undefined);
  const [dcTesting, setDcTesting] = useState(false);
  const [dcTestResult, setDcTestResult] = useState<"success" | "failed" | null>(
    null,
  );
  const [dcSaving, setDcSaving] = useState(false);
  const [dcSaved, setDcSaved] = useState(false);
  const [dcError, setDcError] = useState<string | null>(null);

  useEffect(() => {
    setDcDraft((prev) => {
      const normalizedClientType = normalizeDownloadClientType(prev.clientType);
      if (
        dcTypeOptions.some((option) => option.value === normalizedClientType)
      ) {
        return prev;
      }

      return {
        ...prev,
        clientType:
          dcTypeOptions[0]?.value ?? DEFAULT_DOWNLOAD_CLIENT_DRAFT.clientType,
      };
    });
  }, [dcTypeOptions]);

  const availableDcTypeOptions = ensureDownloadClientTypeOption(
    dcTypeOptions,
    dcDraft.clientType,
  );
  const selectedDcConfigFields = useMemo(
    () =>
      dcTypeOptions.find(
        (option) =>
          option.value === normalizeDownloadClientType(dcDraft.clientType),
      )?.configFields ?? [],
    [dcDraft.clientType, dcTypeOptions],
  );

  useEffect(() => {
    if (selectedDcConfigFields.length === 0) {
      return;
    }
    setDcDraft((current) => {
      const defaults =
        defaultDownloadClientConfigValuesForFields(selectedDcConfigFields);
      const missingDefaults = Object.entries(defaults).filter(
        ([key]) => current.configValues[key] === undefined,
      );
      if (missingDefaults.length === 0) {
        return current;
      }
      return {
        ...current,
        configValues: {
          ...defaults,
          ...current.configValues,
        },
      };
    });
  }, [selectedDcConfigFields]);

  const handleDcDraftChange = useCallback(
    (updates: Partial<DownloadClientDraft>) => {
      const next = { ...dcDraft, ...updates };
      if (updates.clientType && updates.clientType !== dcDraft.clientType) {
        const nextClientType = updates.clientType;
        const prevDefault =
          DEFAULT_PORT_FOR_CLIENT_TYPE[dcDraft.clientType] ?? "8080";
        if (dcDraft.port === "" || dcDraft.port === prevDefault) {
          next.port =
            DEFAULT_PORT_FOR_CLIENT_TYPE[nextClientType] ?? "8080";
        }
        const nextFields =
          dcTypeOptions.find(
            (option) =>
              option.value === normalizeDownloadClientType(nextClientType),
          )?.configFields ?? [];
        next.configValues =
          defaultDownloadClientConfigValuesForFields(nextFields);
      }

      const hasChanged = (
        Object.keys(next) as Array<keyof DownloadClientDraft>
      ).some((key) => next[key] !== dcDraft[key]);

      if (!hasChanged) {
        return;
      }

      setDcDraft(next);
      setDcSaved(false);
      setDcTestResult(null);
      setDcError(null);
    },
    [dcDraft, dcTypeOptions],
  );

  // ── Download client test ────────────────────────────────────────────
  const testDownloadClient = useCallback(async () => {
    setDcTesting(true);
    setDcTestResult(null);
    setDcError(null);
    try {
      const { data, error } = await client
        .mutation(testDownloadClientConnectionMutation, {
          input: {
            clientType: dcDraft.clientType,
            config: buildDownloadClientConfigValues(
              dcDraft,
              selectedDcConfigFields,
            ),
          },
        })
        .toPromise();
      if (error) throw error;
      if (data?.testDownloadClientConnection?.status === "ok") {
        setDcTestResult("success");
      } else {
        setDcTestResult("failed");
      }
    } catch {
      setDcTestResult("failed");
    } finally {
      setDcTesting(false);
    }
  }, [client, dcDraft, selectedDcConfigFields]);

  // ── Download client save ────────────────────────────────────────────
  const saveDownloadClient = useCallback(async () => {
    setDcSaving(true);
    setDcError(null);
    try {
      const { error } = await client
        .mutation(createDownloadClientMutation, {
          input: {
            name: dcDraft.name.trim(),
            clientType: dcDraft.clientType,
            config: buildDownloadClientConfigValues(
              dcDraft,
              selectedDcConfigFields,
            ),
            isEnabled: true,
          },
        })
        .toPromise();
      if (error) throw error;
      setDcSaved(true);
    } catch (err) {
      setDcError(err instanceof Error ? err.message : "Failed to save");
    } finally {
      setDcSaving(false);
    }
  }, [client, dcDraft, selectedDcConfigFields]);

  const handleDcTestAndSave = useCallback(async () => {
    setDcTesting(true);
    setDcTestResult(null);
    setDcError(null);
    try {
      const { data, error } = await client
        .mutation(testDownloadClientConnectionMutation, {
          input: {
            clientType: dcDraft.clientType,
            config: buildDownloadClientConfigValues(
              dcDraft,
              selectedDcConfigFields,
            ),
          },
        })
        .toPromise();
      if (error) throw error;
      if (data?.testDownloadClientConnection?.status === "ok") {
        setDcTestResult("success");
        setDcTesting(false);
        await saveDownloadClient();
      } else {
        setDcTestResult("failed");
        setDcTesting(false);
      }
    } catch {
      setDcTestResult("failed");
      setDcTesting(false);
    }
  }, [client, dcDraft, saveDownloadClient, selectedDcConfigFields]);

  return {
    dcDraft,
    dcLocalPathStyle,
    setDcLocalPathStyle,
    setDcTypeOptions,
    availableDcTypeOptions,
    selectedDcConfigFields,
    dcTesting,
    dcTestResult,
    dcSaving,
    dcSaved,
    dcError,
    handleDcDraftChange,
    testDownloadClient,
    handleDcTestAndSave,
  };
}
