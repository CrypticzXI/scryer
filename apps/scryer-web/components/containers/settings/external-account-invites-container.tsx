import type * as React from "react";
import { useCallback, useEffect, useState } from "react";
import {
  ExternalAccountInvitesPanel,
  type ExternalInviteDraft,
  type ExternalInviteProviderUserOption,
  type ExternalInviteUser,
} from "@/components/views/settings/external-account-invites-panel";
import { createExternalAccountInviteMutation } from "@/lib/graphql/mutations";
import {
  externalAuthRuntimeSettingsQuery,
  externalAccountInvitesQuery,
  jellyfinServerUsersQuery,
  usersQuery,
} from "@/lib/graphql/queries";
import { isVisibleExternalAccountProvider } from "@/lib/constants/integration-providers";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useClient } from "urql";
import type {
  ExternalAccountProvider,
  ExternalAuthRuntimeConnection,
  ExternalAuthRuntimeSettings,
  LinkedAccount,
} from "@/lib/types/settings";

const DEFAULT_EXTERNAL_AUTH_RUNTIME_SETTINGS: ExternalAuthRuntimeSettings = {
  loginProviders: [],
  linkingProviders: [],
  connections: [],
};

const DEFAULT_EXTERNAL_INVITE_DRAFT: ExternalInviteDraft = {
  userId: "",
  provider: "jellyfin",
  connectionId: "",
  providerUserIdentifier: "",
  providerUserId: "",
};

const EXTERNAL_ACCOUNT_INVITE_SOURCES_CHANGED_EVENT =
  "scryer:external-account-invite-sources-changed";

export function notifyExternalAccountInviteSourcesChanged() {
  if (typeof window !== "undefined") {
    window.dispatchEvent(new Event(EXTERNAL_ACCOUNT_INVITE_SOURCES_CHANGED_EVENT));
  }
}

function connectionDescriptorsForProvider(
  settings: ExternalAuthRuntimeSettings,
  provider: ExternalAccountProvider,
): ExternalAuthRuntimeConnection[] {
  return settings.connections.filter(
    (connection) => connection.provider === provider && connection.loginEnabled,
  );
}

function inviteProviders(settings: ExternalAuthRuntimeSettings): ExternalAccountProvider[] {
  const providers: ExternalAccountProvider[] = [];
  for (const connection of settings.connections) {
    if (
      !connection.loginEnabled ||
      !settings.loginProviders.includes(connection.provider) ||
      !isVisibleExternalAccountProvider(connection.provider) ||
      providers.includes(connection.provider)
    ) {
      continue;
    }
    providers.push(connection.provider);
  }
  return providers;
}

function visibleExternalAccountInvites(invites: LinkedAccount[]): LinkedAccount[] {
  return invites.filter((invite) => isVisibleExternalAccountProvider(invite.provider));
}

export function ExternalAccountInvitesContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [users, setUsers] = useState<ExternalInviteUser[]>([]);
  const [invites, setInvites] = useState<LinkedAccount[]>([]);
  const [providerUserOptions, setProviderUserOptions] =
    useState<ExternalInviteProviderUserOption[]>([]);
  const [providerUserSearchLoading, setProviderUserSearchLoading] = useState(false);
  const [providerUserLookupError, setProviderUserLookupError] = useState<string | null>(null);
  const [externalAuthSettings, setExternalAuthSettings] =
    useState<ExternalAuthRuntimeSettings>(DEFAULT_EXTERNAL_AUTH_RUNTIME_SETTINGS);
  const [externalInviteDraft, setExternalInviteDraft] =
    useState<ExternalInviteDraft>(DEFAULT_EXTERNAL_INVITE_DRAFT);
  const [loading, setLoading] = useState(true);
  const [externalInviteSubmitting, setExternalInviteSubmitting] = useState(false);

  const updateExternalInviteDraft = useCallback((patch: Partial<ExternalInviteDraft>) => {
    setProviderUserLookupError(null);
    setExternalInviteDraft((previous) => ({ ...previous, ...patch }));
  }, []);

  const refreshExternalInvites = useCallback(async () => {
    const { data, error } = await client
      .query(externalAccountInvitesQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (error) throw error;
    setInvites(
      visibleExternalAccountInvites((data?.externalAccountInvites ?? []) as LinkedAccount[]),
    );
  }, [client]);

  const refreshInviteData = useCallback(async () => {
    setLoading(true);
    try {
      const [usersResult, externalAuthResult, invitesResult] = await Promise.all([
        client.query(usersQuery, {}, { requestPolicy: "network-only" }).toPromise(),
        client
          .query<{ externalAuthRuntimeSettings?: ExternalAuthRuntimeSettings }>(
            externalAuthRuntimeSettingsQuery,
            {},
            { requestPolicy: "network-only" },
          )
          .toPromise(),
        client
          .query(externalAccountInvitesQuery, {}, { requestPolicy: "network-only" })
          .toPromise(),
      ]);
      if (usersResult.error) throw usersResult.error;
      if (externalAuthResult.error) throw externalAuthResult.error;
      if (invitesResult.error) throw invitesResult.error;

      setUsers(
        ((usersResult.data?.users ?? []) as ExternalInviteUser[]).map((user) => ({
          id: user.id,
          username: user.username,
        })),
      );
      setExternalAuthSettings(
        externalAuthResult.data?.externalAuthRuntimeSettings ??
          DEFAULT_EXTERNAL_AUTH_RUNTIME_SETTINGS,
      );
      setInvites(
        visibleExternalAccountInvites(
          (invitesResult.data?.externalAccountInvites ?? []) as LinkedAccount[],
        ),
      );
    } catch (error) {
      setExternalAuthSettings(DEFAULT_EXTERNAL_AUTH_RUNTIME_SETTINGS);
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setLoading(false);
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    void refreshInviteData();
  }, [refreshInviteData]);

  useEffect(() => {
    if (typeof window === "undefined") {
      return;
    }

    const handleInviteSourcesChanged = () => {
      void refreshInviteData();
    };

    window.addEventListener(
      EXTERNAL_ACCOUNT_INVITE_SOURCES_CHANGED_EVENT,
      handleInviteSourcesChanged,
    );
    return () => {
      window.removeEventListener(
        EXTERNAL_ACCOUNT_INVITE_SOURCES_CHANGED_EVENT,
        handleInviteSourcesChanged,
      );
    };
  }, [refreshInviteData]);

  useEffect(() => {
    setExternalInviteDraft((previous) => {
      const providerOptions = inviteProviders(externalAuthSettings);
      const provider = providerOptions.includes(previous.provider)
        ? previous.provider
        : providerOptions[0] ?? "jellyfin";
      const connections = connectionDescriptorsForProvider(externalAuthSettings, provider);
      const connectionId = connections.some((connection) => connection.id === previous.connectionId)
        ? previous.connectionId
        : connections[0]?.id ?? "";

      const providerChanged = provider !== previous.provider;
      const connectionChanged = connectionId !== previous.connectionId;

      return {
        ...previous,
        userId: users.some((user) => user.id === previous.userId)
          ? previous.userId
          : users[0]?.id ?? "",
        provider,
        connectionId,
        providerUserIdentifier: providerChanged || connectionChanged
          ? ""
          : previous.providerUserIdentifier,
        providerUserId: providerChanged || connectionChanged ? "" : previous.providerUserId,
      };
    });
  }, [externalAuthSettings, users]);

  useEffect(() => {
    if (
      externalInviteDraft.provider !== "jellyfin" ||
      externalInviteDraft.connectionId.trim().length === 0
    ) {
      setProviderUserOptions([]);
      setProviderUserSearchLoading(false);
      setProviderUserLookupError(null);
      return;
    }

    let cancelled = false;
    const search = externalInviteDraft.providerUserIdentifier.trim();
    setProviderUserSearchLoading(true);
    setProviderUserLookupError(null);

    const timeoutId = window.setTimeout(() => {
      void client
        .query(
          jellyfinServerUsersQuery,
          {
            connectionId: externalInviteDraft.connectionId,
            search: search.length > 0 ? search : null,
          },
          { requestPolicy: "network-only" },
        )
        .toPromise()
        .then(({ data, error }) => {
          if (cancelled) return;
          if (error) {
            setProviderUserOptions([]);
            setProviderUserLookupError(error.message);
            return;
          }
          setProviderUserOptions(
            ((data?.jellyfinServerUsers ?? []) as ExternalInviteProviderUserOption[]).map(
              (user) => ({
                id: user.id,
                username: user.username,
                displayName: user.displayName,
                avatarUrl: user.avatarUrl,
              }),
            ),
          );
        })
        .catch((error: unknown) => {
          if (cancelled) return;
          setProviderUserOptions([]);
          setProviderUserLookupError(
            error instanceof Error ? error.message : t("settings.jellyfinUserSearchFailed"),
          );
        })
        .finally(() => {
          if (!cancelled) {
            setProviderUserSearchLoading(false);
          }
        });
    }, search.length > 0 ? 250 : 0);

    return () => {
      cancelled = true;
      window.clearTimeout(timeoutId);
    };
  }, [
    client,
    externalInviteDraft.connectionId,
    externalInviteDraft.provider,
    externalInviteDraft.providerUserIdentifier,
    t,
  ]);

  const createExternalAccountInvite = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const userId = externalInviteDraft.userId.trim();
    const connectionId = externalInviteDraft.connectionId.trim();
    const providerUserIdentifier = externalInviteDraft.providerUserIdentifier.trim();
    const providerUserId =
      externalInviteDraft.provider === "jellyfin"
        ? externalInviteDraft.providerUserId.trim()
        : null;

    if (
      !userId ||
      !connectionId ||
      (externalInviteDraft.provider === "jellyfin" ? !providerUserId : !providerUserIdentifier)
    ) {
      setGlobalStatus(t("settings.externalAccountInviteRequired"));
      return;
    }

    setExternalInviteSubmitting(true);
    try {
      const { error } = await client
        .mutation(createExternalAccountInviteMutation, {
          input: {
            userId,
            provider: externalInviteDraft.provider,
            connectionId,
            providerUserIdentifier,
            providerUserId,
          },
        })
        .toPromise();
      if (error) throw error;
      setExternalInviteDraft((previous) => ({
        ...previous,
        providerUserIdentifier: "",
        providerUserId: "",
      }));
      setGlobalStatus(t("settings.externalAccountInviteCreated"));
      await refreshExternalInvites();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("settings.externalAccountInviteFailed"));
    } finally {
      setExternalInviteSubmitting(false);
    }
  };

  return (
    <ExternalAccountInvitesPanel
      users={users}
      invites={invites}
      providerUserOptions={providerUserOptions}
      providerUserSearchLoading={providerUserSearchLoading}
      providerUserLookupError={providerUserLookupError}
      externalAuthSettings={externalAuthSettings}
      loading={loading}
      externalInviteDraft={externalInviteDraft}
      externalInviteSubmitting={externalInviteSubmitting}
      updateExternalInviteDraft={updateExternalInviteDraft}
      createExternalAccountInvite={createExternalAccountInvite}
    />
  );
}
