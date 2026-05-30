import type * as React from "react";
import { useCallback, useEffect, useState } from "react";
import {
  ExternalAccountInvitesPanel,
  type ExternalInviteDraft,
  type ExternalInviteUser,
} from "@/components/views/settings/external-account-invites-panel";
import { createExternalAccountInviteMutation } from "@/lib/graphql/mutations";
import {
  authProviderRuntimeSettingsQuery,
  externalAccountInvitesQuery,
  usersQuery,
} from "@/lib/graphql/queries";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useClient } from "urql";
import type {
  AuthProviderConnection,
  AuthProviderSettings,
  ExternalAccountProvider,
  LinkedAccount,
} from "@/lib/types/settings";

const DEFAULT_AUTH_PROVIDER_SETTINGS: AuthProviderSettings = {
  allowedProviders: [],
  providerLoginEnabled: [],
  providerLinkingEnabled: [],
  allowedJellyfinConnectionIds: [],
  allowedPlexConnectionIds: [],
  allowedJellyfinConnections: [],
  allowedPlexConnections: [],
};

const DEFAULT_EXTERNAL_INVITE_DRAFT: ExternalInviteDraft = {
  userId: "",
  provider: "jellyfin",
  connectionId: "",
  providerUserIdentifier: "",
};

function connectionIdsForProvider(
  settings: AuthProviderSettings,
  provider: ExternalAccountProvider,
): string[] {
  return provider === "jellyfin"
    ? settings.allowedJellyfinConnectionIds
    : settings.allowedPlexConnectionIds;
}

function connectionDescriptorsForProvider(
  settings: AuthProviderSettings,
  provider: ExternalAccountProvider,
): AuthProviderConnection[] {
  const descriptors =
    provider === "jellyfin"
      ? settings.allowedJellyfinConnections
      : settings.allowedPlexConnections;

  if (descriptors.length > 0) {
    return descriptors;
  }

  return connectionIdsForProvider(settings, provider).map((id) => ({
    id,
    displayName: id,
    userVisibleUrl: null,
    baseUrl: null,
    machineId: null,
  }));
}

function inviteProviders(settings: AuthProviderSettings): ExternalAccountProvider[] {
  return settings.allowedProviders.filter(
    (provider) =>
      settings.providerLoginEnabled.includes(provider) &&
      connectionDescriptorsForProvider(settings, provider).length > 0,
  );
}

export function ExternalAccountInvitesContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [users, setUsers] = useState<ExternalInviteUser[]>([]);
  const [invites, setInvites] = useState<LinkedAccount[]>([]);
  const [authProviderSettings, setAuthProviderSettings] =
    useState<AuthProviderSettings>(DEFAULT_AUTH_PROVIDER_SETTINGS);
  const [externalInviteDraft, setExternalInviteDraft] =
    useState<ExternalInviteDraft>(DEFAULT_EXTERNAL_INVITE_DRAFT);
  const [loading, setLoading] = useState(true);
  const [externalInviteSubmitting, setExternalInviteSubmitting] = useState(false);

  const updateExternalInviteDraft = useCallback((patch: Partial<ExternalInviteDraft>) => {
    setExternalInviteDraft((previous) => ({ ...previous, ...patch }));
  }, []);

  const refreshExternalInvites = useCallback(async () => {
    const { data, error } = await client.query(externalAccountInvitesQuery, {}).toPromise();
    if (error) throw error;
    setInvites((data?.externalAccountInvites ?? []) as LinkedAccount[]);
  }, [client]);

  const refreshInviteData = useCallback(async () => {
    setLoading(true);
    try {
      const [usersResult, authProviderResult, invitesResult] = await Promise.all([
        client.query(usersQuery, {}).toPromise(),
        client.query(authProviderRuntimeSettingsQuery, {}).toPromise(),
        client.query(externalAccountInvitesQuery, {}).toPromise(),
      ]);
      if (usersResult.error) throw usersResult.error;
      if (authProviderResult.error) throw authProviderResult.error;
      if (invitesResult.error) throw invitesResult.error;

      setUsers(
        ((usersResult.data?.users ?? []) as ExternalInviteUser[]).map((user) => ({
          id: user.id,
          username: user.username,
        })),
      );
      setAuthProviderSettings({
        ...DEFAULT_AUTH_PROVIDER_SETTINGS,
        ...authProviderResult.data?.authProviderRuntimeSettings,
      });
      setInvites((invitesResult.data?.externalAccountInvites ?? []) as LinkedAccount[]);
    } catch (error) {
      setAuthProviderSettings(DEFAULT_AUTH_PROVIDER_SETTINGS);
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setLoading(false);
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    void refreshInviteData();
  }, [refreshInviteData]);

  useEffect(() => {
    setExternalInviteDraft((previous) => {
      const providerOptions = inviteProviders(authProviderSettings);
      const provider = providerOptions.includes(previous.provider)
        ? previous.provider
        : providerOptions[0] ?? "jellyfin";
      const connections = connectionDescriptorsForProvider(authProviderSettings, provider);
      const connectionId = connections.some((connection) => connection.id === previous.connectionId)
        ? previous.connectionId
        : connections[0]?.id ?? "";

      return {
        ...previous,
        userId: users.some((user) => user.id === previous.userId)
          ? previous.userId
          : users[0]?.id ?? "",
        provider,
        connectionId,
      };
    });
  }, [authProviderSettings, users]);

  const createExternalAccountInvite = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const userId = externalInviteDraft.userId.trim();
    const connectionId = externalInviteDraft.connectionId.trim();
    const providerUserIdentifier = externalInviteDraft.providerUserIdentifier.trim();

    if (!userId || !connectionId || !providerUserIdentifier) {
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
          },
        })
        .toPromise();
      if (error) throw error;
      setExternalInviteDraft((previous) => ({
        ...previous,
        providerUserIdentifier: "",
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
      authProviderSettings={authProviderSettings}
      loading={loading}
      externalInviteDraft={externalInviteDraft}
      externalInviteSubmitting={externalInviteSubmitting}
      updateExternalInviteDraft={updateExternalInviteDraft}
      createExternalAccountInvite={createExternalAccountInvite}
    />
  );
}
