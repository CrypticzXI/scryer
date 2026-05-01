
import { type FormEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsNotificationsSection } from "@/components/views/settings/settings-notifications-section";
import { useClient } from "urql";
import { toast } from "sonner";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import type {
  NotificationChannel,
  NotificationChannelDraft,
  NotificationProviderType,
  NotificationSubscription,
  NotificationSubscriptionDraft,
  NotificationSubscriptionRow,
  TitleRecord,
} from "@/lib/types";
import {
  notificationChannelsQuery,
  notificationProviderTypesQuery,
  notificationSubscriptionsQuery,
  notificationsInitQuery,
  titleListEntryQuery,
} from "@/lib/graphql/queries";
import {
  createNotificationChannelMutation,
  updateNotificationChannelMutation,
  deleteNotificationChannelMutation,
  testNotificationChannelMutation,
  createNotificationSubscriptionMutation,
  updateNotificationSubscriptionMutation,
  deleteNotificationSubscriptionMutation,
} from "@/lib/graphql/mutations";

const CHANNEL_INITIAL_DRAFT: NotificationChannelDraft = {
  name: "",
  channelType: "",
  isEnabled: true,
  configValues: {},
};

const SUBSCRIPTION_INITIAL_DRAFT: NotificationSubscriptionDraft = {
  channelId: "",
  eventTypes: [],
  scope: "global",
  facetScopeIds: [],
  titleScopeId: "",
  titleScopeTitle: null,
  isEnabled: true,
};

const FACET_SCOPE_ORDER = ["movie", "series", "anime"] as const;

function isFacetScopeId(value: string): value is (typeof FACET_SCOPE_ORDER)[number] {
  return FACET_SCOPE_ORDER.includes(value as (typeof FACET_SCOPE_ORDER)[number]);
}

function parseFacetScopeIds(scopeId: string | null | undefined): string[] {
  if (!scopeId) {
    return [];
  }

  const selected = new Set<string>();
  for (const token of scopeId.split(",")) {
    const normalized = token.trim().toLowerCase();
    if (isFacetScopeId(normalized)) {
      selected.add(normalized);
    }
  }

  return FACET_SCOPE_ORDER.filter((facet) => selected.has(facet));
}

function serializeFacetScopeIds(scopeIds: Iterable<string>): string {
  const selected = new Set(scopeIds);
  return FACET_SCOPE_ORDER.filter((facet) => selected.has(facet)).join(",");
}

type NotificationSubscriptionSpec = {
  channelId: string;
  eventType: string;
  scope: string;
  scopeId?: string;
  isEnabled: boolean;
};

function subscriptionSpecKey(spec: Pick<NotificationSubscriptionSpec, "channelId" | "eventType" | "scope" | "scopeId">): string {
  return [spec.channelId, spec.eventType, spec.scope, spec.scopeId ?? ""].join("::");
}

function subscriptionRowKey(subscription: NotificationSubscription): string {
  return [
    subscription.channelId,
    subscription.scope,
    subscription.scopeId ?? "",
    subscription.isEnabled ? "1" : "0",
  ].join("::");
}

function buildNotificationSubscriptionRows(
  subscriptions: NotificationSubscription[],
): NotificationSubscriptionRow[] {
  const groups = new Map<
    string,
    NotificationSubscriptionRow & { eventTypeSet: Set<string> }
  >();

  for (const subscription of subscriptions) {
    const key = subscriptionRowKey(subscription);
    const existing = groups.get(key);
    if (existing) {
      existing.eventTypeSet.add(subscription.eventType);
      existing.subscriptionIds.push(subscription.id);
      continue;
    }

    groups.set(key, {
      id: key,
      channelId: subscription.channelId,
      eventTypes: [],
      scope: subscription.scope,
      scopeId: subscription.scopeId,
      isEnabled: subscription.isEnabled,
      subscriptionIds: [subscription.id],
      eventTypeSet: new Set([subscription.eventType]),
    });
  }

  return Array.from(groups.values()).map(({ eventTypeSet, ...row }) => ({
    ...row,
    eventTypes: Array.from(eventTypeSet).sort(),
  }));
}

function buildNotificationSubscriptionSpecs(
  draft: NotificationSubscriptionDraft,
): NotificationSubscriptionSpec[] {
  const normalizedEventTypes = Array.from(
    new Set(draft.eventTypes.map((eventType) => eventType.trim()).filter(Boolean)),
  );
  const normalizedScope = draft.scope.trim();
  const normalizedFacetScopeId = serializeFacetScopeIds(draft.facetScopeIds);
  const normalizedTitleScopeId = draft.titleScopeId.trim();
  const scopeSpec =
    normalizedScope === "global"
      ? { scope: normalizedScope, scopeId: undefined }
      : normalizedScope === "facet"
        ? normalizedFacetScopeId
          ? { scope: normalizedScope, scopeId: normalizedFacetScopeId }
          : null
        : normalizedScope === "title"
          ? normalizedTitleScopeId
            ? { scope: normalizedScope, scopeId: normalizedTitleScopeId }
            : null
          : null;

  if (!scopeSpec) {
    return [];
  }

  return normalizedEventTypes.map((eventType) => ({
    channelId: draft.channelId,
    eventType,
    scope: scopeSpec.scope,
    scopeId: scopeSpec.scopeId,
    isEnabled: draft.isEnabled,
  }));
}

function serializeConfigJson(configValues: Record<string, string>): string | undefined {
  const nonEmpty = Object.fromEntries(
    Object.entries(configValues).filter(([, v]) => v !== ""),
  );
  return Object.keys(nonEmpty).length > 0 ? JSON.stringify(nonEmpty) : undefined;
}

function parseConfigJson(configJson: string | null): Record<string, string> {
  if (!configJson) return {};
  try {
    return JSON.parse(configJson) as Record<string, string>;
  } catch {
    return {};
  }
}

type SettingsNotificationsContainerProps = {
  providerCatalogVersion?: number;
};

export function SettingsNotificationsContainer({
  providerCatalogVersion = 0,
}: SettingsNotificationsContainerProps) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();

  // --- Channel state ---
  const [channels, setChannels] = useState<NotificationChannel[]>([]);
  const [editingChannelId, setEditingChannelId] = useState<string | null>(null);
  const [mutatingChannelId, setMutatingChannelId] = useState<string | null>(null);
  const [pendingDeleteChannel, setPendingDeleteChannel] = useState<NotificationChannel | null>(null);
  const [channelDraft, setChannelDraft] = useState<NotificationChannelDraft>(() => ({ ...CHANNEL_INITIAL_DRAFT }));
  const [providerTypes, setProviderTypes] = useState<NotificationProviderType[]>([]);
  const [testingChannelId, setTestingChannelId] = useState<string | null>(null);

  // --- Subscription state ---
  const [subscriptions, setSubscriptions] = useState<NotificationSubscription[]>([]);
  const [editingSubscriptionId, setEditingSubscriptionId] = useState<string | null>(null);
  const [mutatingSubscriptionId, setMutatingSubscriptionId] = useState<string | null>(null);
  const [pendingDeleteSubscription, setPendingDeleteSubscription] = useState<NotificationSubscriptionRow | null>(null);
  const [subscriptionDraft, setSubscriptionDraft] = useState<NotificationSubscriptionDraft>(() => ({ ...SUBSCRIPTION_INITIAL_DRAFT }));
  const [eventTypes, setEventTypes] = useState<string[]>([]);
  const [subscriptionTitlesById, setSubscriptionTitlesById] = useState<Record<string, TitleRecord | null>>({});
  const providerCatalogVersionRef = useRef(providerCatalogVersion);
  const subscriptionRows = useMemo(
    () => buildNotificationSubscriptionRows(subscriptions),
    [subscriptions],
  );
  const titleScopedSubscriptionIds = useMemo(
    () =>
      Array.from(
        new Set(
          subscriptions
            .filter((subscription) => subscription.scope === "title" && !!subscription.scopeId)
            .map((subscription) => subscription.scopeId as string),
        ),
      ),
    [subscriptions],
  );

  // --- Fetch data ---
  const refreshChannels = useCallback(async () => {
    try {
      const { data, error } = await client
        .query(notificationChannelsQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (error) throw error;
      setChannels(data.notificationChannels || []);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    }
  }, [client, setGlobalStatus, t]);

  const refreshSubscriptions = useCallback(async () => {
    try {
      const { data, error } = await client
        .query(notificationSubscriptionsQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (error) throw error;
      setSubscriptions(data.notificationSubscriptions || []);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    }
  }, [client, setGlobalStatus, t]);

  const refreshProviderTypes = useCallback(async () => {
    const { data, error } = await client
      .query(notificationProviderTypesQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (error) throw error;
    setProviderTypes(data?.notificationProviderTypes || []);
  }, [client]);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const { data, error } = await client
          .query(notificationsInitQuery, {}, { requestPolicy: "network-only" })
          .toPromise();
        if (error && !data?.notificationChannels && !data?.notificationSubscriptions) throw error;
        if (cancelled) return;
        setChannels(data?.notificationChannels || []);
        setSubscriptions(data?.notificationSubscriptions || []);
        setProviderTypes(data?.notificationProviderTypes || []);
        setEventTypes(data?.notificationEventTypes || []);
      } catch (error) {
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
    setSubscriptionTitlesById((current) => {
      const next: Record<string, TitleRecord | null> = {};
      for (const titleId of titleScopedSubscriptionIds) {
        if (Object.prototype.hasOwnProperty.call(current, titleId)) {
          next[titleId] = current[titleId];
        }
      }
      return next;
    });
  }, [titleScopedSubscriptionIds]);

  useEffect(() => {
    const unresolvedIds = titleScopedSubscriptionIds.filter(
      (titleId) => !Object.prototype.hasOwnProperty.call(subscriptionTitlesById, titleId),
    );

    if (unresolvedIds.length === 0) {
      return;
    }

    let cancelled = false;

    void Promise.all(
      unresolvedIds.map(async (titleId) => {
        const { data, error } = await client
          .query<{ title?: TitleRecord | null }>(titleListEntryQuery, { id: titleId })
          .toPromise();
        if (error) {
          return [titleId, null] as const;
        }
        return [titleId, (data?.title as TitleRecord | null) ?? null] as const;
      }),
    ).then((entries) => {
      if (cancelled) {
        return;
      }

      setSubscriptionTitlesById((current) => {
        const next = { ...current };
        for (const [titleId, title] of entries) {
          next[titleId] = title;
        }
        return next;
      });
    });

    return () => {
      cancelled = true;
    };
  }, [client, subscriptionTitlesById, titleScopedSubscriptionIds]);

  // --- Channel CRUD ---
  const resetChannelDraft = useCallback(() => {
    setEditingChannelId(null);
    setChannelDraft(() => ({ ...CHANNEL_INITIAL_DRAFT }));
  }, []);

  const submitChannel = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const payload = {
      name: channelDraft.name.trim(),
      channelType: channelDraft.channelType.trim(),
      configJson: serializeConfigJson(channelDraft.configValues),
      isEnabled: channelDraft.isEnabled,
    };

    if (!payload.name || !payload.channelType) {
      setGlobalStatus(t("status.failedToCreate"));
      return;
    }

    setMutatingChannelId(editingChannelId || "new");
    try {
      if (editingChannelId) {
        const { error } = await client.mutation(updateNotificationChannelMutation, {
          input: {
            id: editingChannelId,
            name: payload.name,
            configJson: payload.configJson,
            isEnabled: payload.isEnabled,
          },
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.notificationChannelUpdated"));
      } else {
        const { error } = await client.mutation(createNotificationChannelMutation, {
          input: {
            name: payload.name,
            channelType: payload.channelType,
            configJson: payload.configJson,
            isEnabled: payload.isEnabled,
          },
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.notificationChannelCreated"));
      }
      resetChannelDraft();
      await refreshChannels();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingChannelId(null);
    }
  };

   const editChannel = (channel: NotificationChannel) => {
     setEditingChannelId(channel.id);
     const provider = providerTypes.find((pt) => pt.providerType === channel.channelType);
     setChannelDraft({
       name: provider?.name ?? channel.name,
       channelType: channel.channelType,
       isEnabled: channel.isEnabled,
       configValues: parseConfigJson(channel.configJson),
     });
     setGlobalStatus(t("status.editingNotificationChannel", { name: channel.name }));
   };

  const deleteChannel = (channel: NotificationChannel) => {
    setPendingDeleteChannel(channel);
  };

  const confirmDeleteChannel = async () => {
    if (!pendingDeleteChannel) return;
    const channel = pendingDeleteChannel;
    setMutatingChannelId(channel.id);
    try {
      const { error } = await client.mutation(deleteNotificationChannelMutation, {
        id: channel.id,
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.notificationChannelDeleted", { name: channel.name }));
      await refreshChannels();
      await refreshSubscriptions();
      if (editingChannelId === channel.id) {
        resetChannelDraft();
      }
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingChannelId(null);
      setPendingDeleteChannel(null);
    }
  };

  const toggleChannelEnabled = useCallback(async (channel: NotificationChannel) => {
    const nextIsEnabled = !channel.isEnabled;
    setMutatingChannelId(channel.id);
    setChannels((current) => current.map((existing) => (
      existing.id === channel.id ? { ...existing, isEnabled: nextIsEnabled } : existing
    )));
    if (editingChannelId === channel.id) {
      setChannelDraft((current) => ({
        ...current,
        isEnabled: nextIsEnabled,
      }));
    }
    try {
      const { error } = await client.mutation(updateNotificationChannelMutation, {
        input: {
          id: channel.id,
          isEnabled: nextIsEnabled,
        },
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.notificationChannelUpdated"));
      await refreshChannels();
    } catch (error) {
      setChannels((current) => current.map((existing) => (
        existing.id === channel.id ? { ...existing, isEnabled: channel.isEnabled } : existing
      )));
      if (editingChannelId === channel.id) {
        setChannelDraft((current) => ({
          ...current,
          isEnabled: channel.isEnabled,
        }));
      }
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingChannelId(null);
    }
  }, [client, editingChannelId, refreshChannels, setGlobalStatus, t]);

  const testChannel = useCallback(async (channel: NotificationChannel) => {
    setTestingChannelId(channel.id);
    try {
      const { data, error } = await client.mutation(testNotificationChannelMutation, {
        id: channel.id,
      }).toPromise();
      if (error) throw error;
      if (data?.testNotificationChannel) {
        const message = t("settings.notificationTestSuccess");
        setGlobalStatus(message);
        toast.success(message);
      } else {
        setGlobalStatus(t("settings.notificationTestFailed"));
      }
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("settings.notificationTestFailed"));
    } finally {
      setTestingChannelId(null);
    }
  }, [client, setGlobalStatus, t]);

  // --- Subscription CRUD ---
  const resetSubscriptionDraft = useCallback(() => {
    setEditingSubscriptionId(null);
    setSubscriptionDraft(() => ({ ...SUBSCRIPTION_INITIAL_DRAFT }));
  }, []);

  const submitSubscription = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const desiredSpecs = buildNotificationSubscriptionSpecs(subscriptionDraft);

    if (!subscriptionDraft.channelId || desiredSpecs.length === 0) {
      setGlobalStatus(t("status.failedToCreate"));
      return;
    }

    setMutatingSubscriptionId(editingSubscriptionId || "new");
    try {
      if (editingSubscriptionId) {
        const editingRow = subscriptionRows.find((row) => row.id === editingSubscriptionId);
        if (!editingRow) {
          throw new Error(t("status.failedToUpdate"));
        }

        const existingSubscriptions = subscriptions.filter((subscription) =>
          editingRow.subscriptionIds.includes(subscription.id),
        );
        const existingByKey = new Map(
          existingSubscriptions.map((subscription) => [
            subscriptionSpecKey({
              channelId: subscription.channelId,
              eventType: subscription.eventType,
              scope: subscription.scope,
              scopeId: subscription.scopeId ?? undefined,
            }),
            subscription,
          ]),
        );
        const desiredByKey = new Map(
          desiredSpecs.map((spec) => [subscriptionSpecKey(spec), spec]),
        );

        for (const spec of desiredSpecs) {
          const key = subscriptionSpecKey(spec);
          const existing = existingByKey.get(key);
          if (!existing) {
            const { error } = await client.mutation(createNotificationSubscriptionMutation, {
              input: {
                channelId: spec.channelId,
                eventType: spec.eventType,
                scope: spec.scope,
                scopeId: spec.scopeId,
                isEnabled: spec.isEnabled,
              },
            }).toPromise();
            if (error) throw error;
            continue;
          }

          if (existing.isEnabled !== spec.isEnabled) {
            const { error } = await client.mutation(updateNotificationSubscriptionMutation, {
              input: {
                id: existing.id,
                isEnabled: spec.isEnabled,
              },
            }).toPromise();
            if (error) throw error;
          }
        }

        for (const subscription of existingSubscriptions) {
          const key = subscriptionSpecKey({
            channelId: subscription.channelId,
            eventType: subscription.eventType,
            scope: subscription.scope,
            scopeId: subscription.scopeId ?? undefined,
          });
          if (desiredByKey.has(key)) {
            continue;
          }

          const { error } = await client.mutation(deleteNotificationSubscriptionMutation, {
            id: subscription.id,
          }).toPromise();
          if (error) throw error;
        }

        setGlobalStatus(t("status.notificationSubscriptionUpdated"));
      } else {
        for (const spec of desiredSpecs) {
          const { error } = await client.mutation(createNotificationSubscriptionMutation, {
            input: {
              channelId: spec.channelId,
              eventType: spec.eventType,
              scope: spec.scope,
              scopeId: spec.scopeId,
              isEnabled: spec.isEnabled,
            },
          }).toPromise();
          if (error) throw error;
        }
        setGlobalStatus(t("status.notificationSubscriptionCreated"));
      }
      resetSubscriptionDraft();
      await refreshSubscriptions();
    } catch (error) {
      await refreshSubscriptions().catch(() => undefined);
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingSubscriptionId(null);
    }
  };

  const editSubscription = (sub: NotificationSubscriptionRow) => {
    setEditingSubscriptionId(sub.id);
    setSubscriptionDraft({
      channelId: sub.channelId,
      eventTypes: [...sub.eventTypes],
      scope: sub.scope,
      facetScopeIds: sub.scope === "facet" ? parseFacetScopeIds(sub.scopeId) : [],
      titleScopeId: sub.scope === "title" ? sub.scopeId || "" : "",
      titleScopeTitle:
        sub.scope === "title" && sub.scopeId
          ? subscriptionTitlesById[sub.scopeId] ?? null
          : null,
      isEnabled: sub.isEnabled,
    });
    setGlobalStatus(t("status.editingNotificationSubscription"));
  };

  const deleteSubscription = (sub: NotificationSubscriptionRow) => {
    setPendingDeleteSubscription(sub);
  };

  const confirmDeleteSubscription = async () => {
    if (!pendingDeleteSubscription) return;
    const sub = pendingDeleteSubscription;
    setMutatingSubscriptionId(sub.id);
    try {
      for (const subscriptionId of sub.subscriptionIds) {
        const { error } = await client.mutation(deleteNotificationSubscriptionMutation, {
          id: subscriptionId,
        }).toPromise();
        if (error) throw error;
      }
      setGlobalStatus(t("status.notificationSubscriptionDeleted"));
      await refreshSubscriptions();
      if (editingSubscriptionId === sub.id) {
        resetSubscriptionDraft();
      }
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingSubscriptionId(null);
      setPendingDeleteSubscription(null);
    }
  };

  const toggleSubscriptionEnabled = useCallback(async (sub: NotificationSubscriptionRow) => {
    setMutatingSubscriptionId(sub.id);
    try {
      for (const subscriptionId of sub.subscriptionIds) {
        const { error } = await client.mutation(updateNotificationSubscriptionMutation, {
          input: {
            id: subscriptionId,
            isEnabled: !sub.isEnabled,
          },
        }).toPromise();
        if (error) throw error;
      }
      setGlobalStatus(t("status.notificationSubscriptionUpdated"));
      await refreshSubscriptions();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingSubscriptionId(null);
    }
  }, [client, refreshSubscriptions, setGlobalStatus, t]);

  return (
    <>
      <SettingsNotificationsSection
        channels={channels}
        editingChannelId={editingChannelId}
        channelDraft={channelDraft}
        setChannelDraft={setChannelDraft}
        submitChannel={submitChannel}
        mutatingChannelId={mutatingChannelId}
        resetChannelDraft={resetChannelDraft}
        editChannel={editChannel}
        toggleChannelEnabled={toggleChannelEnabled}
        deleteChannel={deleteChannel}
        testChannel={testChannel}
        testingChannelId={testingChannelId}
        providerTypes={providerTypes}
        subscriptions={subscriptionRows}
        subscriptionTitlesById={subscriptionTitlesById}
        editingSubscriptionId={editingSubscriptionId}
        subscriptionDraft={subscriptionDraft}
        setSubscriptionDraft={setSubscriptionDraft}
        submitSubscription={submitSubscription}
        mutatingSubscriptionId={mutatingSubscriptionId}
        resetSubscriptionDraft={resetSubscriptionDraft}
        editSubscription={editSubscription}
        toggleSubscriptionEnabled={toggleSubscriptionEnabled}
        deleteSubscription={deleteSubscription}
        eventTypes={eventTypes}
      />
      <ConfirmDialog
        open={pendingDeleteChannel !== null}
        title={t("label.delete")}
        description={
          pendingDeleteChannel ? t("status.deletingNotificationChannel", { name: pendingDeleteChannel.name }) : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={mutatingChannelId !== null}
        onConfirm={confirmDeleteChannel}
        onCancel={() => setPendingDeleteChannel(null)}
      />
      <ConfirmDialog
        open={pendingDeleteSubscription !== null}
        title={t("label.delete")}
        description={t("status.deletingNotificationSubscription")}
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={mutatingSubscriptionId !== null}
        onConfirm={confirmDeleteSubscription}
        onCancel={() => setPendingDeleteSubscription(null)}
      />
    </>
  );
}
