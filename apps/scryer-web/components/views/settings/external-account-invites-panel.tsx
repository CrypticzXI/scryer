import type * as React from "react";
import { Loader2, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  AuthProviderConnection,
  AuthProviderSettings,
  ExternalAccountProvider,
  LinkedAccount,
} from "@/lib/types/settings";

export type ExternalInviteDraft = {
  userId: string;
  provider: ExternalAccountProvider;
  connectionId: string;
  providerUserIdentifier: string;
};

export type ExternalInviteUser = {
  id: string;
  username: string;
};

type ExternalAccountInvitesPanelProps = {
  users: ExternalInviteUser[];
  invites: LinkedAccount[];
  authProviderSettings: AuthProviderSettings;
  loading: boolean;
  externalInviteDraft: ExternalInviteDraft;
  externalInviteSubmitting: boolean;
  updateExternalInviteDraft: (patch: Partial<ExternalInviteDraft>) => void;
  createExternalAccountInvite: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
};

function providerLabel(provider: ExternalAccountProvider): string {
  switch (provider) {
    case "plex":
      return "Plex";
    case "jellyfin":
      return "Jellyfin";
    default:
      return provider;
  }
}

function providerConnections(
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

  const ids =
    provider === "jellyfin"
      ? settings.allowedJellyfinConnectionIds
      : settings.allowedPlexConnectionIds;

  return ids.map((id) => ({
    id,
    displayName: provider === "jellyfin" ? "Jellyfin" : id,
    userVisibleUrl: null,
    baseUrl: null,
    machineId: null,
  }));
}

function providerConnectionLabel(connection: AuthProviderConnection): string {
  return connection.userVisibleUrl
    ? `${connection.displayName} (${connection.userVisibleUrl})`
    : connection.displayName;
}

function inviteConnectionLabel(
  settings: AuthProviderSettings,
  invite: LinkedAccount,
): string {
  const connection = providerConnections(settings, invite.provider).find(
    (candidate) => candidate.id === invite.connectionId,
  );
  if (connection) {
    return providerConnectionLabel(connection);
  }

  return invite.provider === "jellyfin"
    ? providerLabel(invite.provider)
    : invite.connectionId;
}

function providerIdentifierLabel(provider: ExternalAccountProvider, t: ReturnType<typeof useTranslate>): string {
  return provider === "jellyfin"
    ? t("settings.jellyfinUsername")
    : t("settings.plexUserId");
}

function inviteProviderOptions(settings: AuthProviderSettings): ExternalAccountProvider[] {
  return settings.allowedProviders.filter(
    (provider) =>
      settings.providerLoginEnabled.includes(provider) &&
      providerConnections(settings, provider).length > 0,
  );
}

function formatTimestamp(value: string | null | undefined): string {
  if (!value) {
    return "-";
  }

  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) {
    return value;
  }

  return parsed.toLocaleString();
}

function providerIdentityLabel(account: LinkedAccount): string {
  const name = account.displayName || account.username;
  return account.externalUserId ? `${name} (${account.externalUserId})` : name;
}

function inviteStatus(account: LinkedAccount, t: ReturnType<typeof useTranslate>) {
  if (account.status === "disabled") {
    return {
      label: t("settings.externalAccountInviteStatusDisabled"),
      className: "border-destructive/40 bg-destructive/10 text-destructive",
    };
  }

  if (account.lastLoginAt) {
    return {
      label: t("settings.externalAccountInviteStatusLoggedIn"),
      className: "border-emerald-500/40 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
    };
  }

  if (account.status === "pending_claim") {
    return {
      label: t("settings.externalAccountInviteStatusPending"),
      className: "border-amber-500/40 bg-amber-500/10 text-amber-700 dark:text-amber-300",
    };
  }

  return {
    label: t("settings.externalAccountInviteStatusActive"),
    className: "border-border bg-background text-muted-foreground",
  };
}

export function ExternalAccountInvitesPanel({
  users,
  invites,
  authProviderSettings,
  loading,
  externalInviteDraft,
  externalInviteSubmitting,
  updateExternalInviteDraft,
  createExternalAccountInvite,
}: ExternalAccountInvitesPanelProps) {
  const t = useTranslate();
  const inviteProviders = inviteProviderOptions(authProviderSettings);
  const inviteConnections = providerConnections(authProviderSettings, externalInviteDraft.provider);
  const providerIdentifierLabelText = providerIdentifierLabel(externalInviteDraft.provider, t);
  const inviteUnavailable = inviteProviders.length === 0 || users.length === 0;
  const usersById = new Map(users.map((user) => [user.id, user.username]));
  const sortedInvites = [...invites].sort((left, right) => {
    const rightTime = new Date(right.createdAt).getTime();
    const leftTime = new Date(left.createdAt).getTime();
    return (Number.isNaN(rightTime) ? 0 : rightTime) - (Number.isNaN(leftTime) ? 0 : leftTime);
  });

  return (
    <div className="space-y-4 rounded-lg border border-border bg-card/50 p-4">
      <div className="flex items-center gap-2">
        <h3 className="text-base font-medium">{t("settings.externalAccountInvites")}</h3>
        {loading ? <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" /> : null}
      </div>

      {inviteUnavailable ? (
        <p className="text-muted-foreground">
          {t("settings.externalAccountInvitesUnavailable")}
        </p>
      ) : (
        <form
          id="settings-external-account-invite-form"
          className="space-y-4"
          onSubmit={createExternalAccountInvite}
        >
          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-5">
            <div className="space-y-1.5">
              <Label htmlFor="settings-external-invite-user">
                {t("settings.user")}
              </Label>
              <select
                id="settings-external-invite-user"
                className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm"
                value={externalInviteDraft.userId}
                onChange={(event) =>
                  updateExternalInviteDraft({ userId: event.target.value })
                }
                disabled={externalInviteSubmitting}
                required
              >
                {users.map((user) => (
                  <option key={user.id} value={user.id}>
                    {user.username}
                  </option>
                ))}
              </select>
            </div>
            {inviteProviders.length > 1 ? (
              <div className="space-y-1.5">
                <Label htmlFor="settings-external-invite-provider">
                  {t("settings.provider")}
                </Label>
                <select
                  id="settings-external-invite-provider"
                  className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm"
                  value={externalInviteDraft.provider}
                  onChange={(event) => {
                    const provider = event.target.value as ExternalAccountProvider;
                    updateExternalInviteDraft({
                      provider,
                      connectionId: providerConnections(authProviderSettings, provider)[0]?.id ?? "",
                    });
                  }}
                  disabled={externalInviteSubmitting}
                  required
                >
                  {inviteProviders.map((provider) => (
                    <option key={provider} value={provider}>
                      {providerLabel(provider)}
                    </option>
                  ))}
                </select>
              </div>
            ) : null}
            {inviteConnections.length > 1 ? (
              <div className="space-y-1.5">
                <Label htmlFor="settings-external-invite-connection">
                  {t("profile.linkedAccountConnection")}
                </Label>
                <select
                  id="settings-external-invite-connection"
                  className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm"
                  value={externalInviteDraft.connectionId}
                  onChange={(event) =>
                    updateExternalInviteDraft({ connectionId: event.target.value })
                  }
                  disabled={externalInviteSubmitting}
                  required
                >
                  {inviteConnections.map((connection) => (
                    <option key={connection.id} value={connection.id}>
                      {providerConnectionLabel(connection)}
                    </option>
                  ))}
                </select>
              </div>
            ) : null}
            <div className="space-y-1.5">
              <Label htmlFor="settings-external-invite-provider-identifier">
                {providerIdentifierLabelText}
              </Label>
              <Input
                id="settings-external-invite-provider-identifier"
                value={externalInviteDraft.providerUserIdentifier}
                onChange={(event) =>
                  updateExternalInviteDraft({ providerUserIdentifier: event.target.value })
                }
                disabled={externalInviteSubmitting}
                placeholder={providerIdentifierLabelText}
                required
              />
            </div>
            <div className="flex items-end">
              <Button
                id="settings-external-account-invite-create"
                type="submit"
                className="min-w-40"
                disabled={externalInviteSubmitting}
              >
                {externalInviteSubmitting ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Plus className="h-4 w-4" />
                )}
                {externalInviteSubmitting ? t("label.saving") : t("settings.createInvite")}
              </Button>
            </div>
          </div>
        </form>
      )}

      <div className="space-y-3">
        <h4 className="text-sm font-medium">{t("settings.previousExternalAccountInvites")}</h4>
        <Table id="settings-external-account-invites-table">
          <TableHeader>
            <TableRow>
              <TableHead>{t("settings.user")}</TableHead>
              <TableHead>{t("settings.provider")}</TableHead>
              <TableHead>{t("profile.linkedAccountConnection")}</TableHead>
              <TableHead>{t("settings.externalAccountInviteProviderUser")}</TableHead>
              <TableHead>{t("profile.linkedAccountStatus")}</TableHead>
              <TableHead>{t("settings.externalAccountInviteCreatedAt")}</TableHead>
              <TableHead>{t("settings.externalAccountInviteLastLogin")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading ? (
              <TableRow>
                <TableCell colSpan={7} className="text-muted-foreground">
                  {t("label.loading")}
                </TableCell>
              </TableRow>
            ) : sortedInvites.length === 0 ? (
              <TableRow>
                <TableCell colSpan={7} className="text-muted-foreground">
                  {t("settings.noExternalAccountInvites")}
                </TableCell>
              </TableRow>
            ) : (
              sortedInvites.map((invite) => {
                const status = inviteStatus(invite, t);
                return (
                  <TableRow key={invite.id}>
                    <TableCell>{usersById.get(invite.userId) ?? invite.userId}</TableCell>
                    <TableCell>{providerLabel(invite.provider)}</TableCell>
                    <TableCell>{inviteConnectionLabel(authProviderSettings, invite)}</TableCell>
                    <TableCell>{providerIdentityLabel(invite)}</TableCell>
                    <TableCell>
                      <span
                        className={`inline-flex rounded-full border px-2 py-0.5 text-xs font-medium ${status.className}`}
                      >
                        {status.label}
                      </span>
                    </TableCell>
                    <TableCell>{formatTimestamp(invite.createdAt)}</TableCell>
                    <TableCell>{formatTimestamp(invite.lastLoginAt)}</TableCell>
                  </TableRow>
                );
              })
            )}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}
