import type * as React from "react";
import { ChevronRight, KeyRound, Trash2, User2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
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
import type { LibraryRecord, UserRecord } from "@/lib/types";
import type {
  AuthProviderConnection,
  AuthProviderSettings,
  ExternalAccountProvider,
} from "@/lib/types/settings";
import { selectorId } from "@/lib/utils/dom-ids";

type LibraryGrantDrafts = Record<string, string[]>;

export type ExternalInviteDraft = {
  userId: string;
  provider: ExternalAccountProvider;
  connectionId: string;
  externalUserId: string;
  username: string;
};

type SettingsUsersSectionProps = {
  settingsUsers: UserRecord[];
  libraries: LibraryRecord[];
  authProviderSettings: AuthProviderSettings;
  currentUserId?: string | null;
  appPermissions: string[];
  libraryPermissions: string[];
  newUsername: string;
  setNewUsername: (value: string) => void;
  newPassword: string;
  setNewPassword: (value: string) => void;
  newAppPermissions: string[];
  newLibraryPermissionDrafts: LibraryGrantDrafts;
  toggleNewAppPermission: (value: string) => void;
  toggleNewLibraryPermission: (libraryId: string, value: string) => void;
  createUser: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  userPasswordDrafts: Record<string, string>;
  userAppPermissionDrafts: Record<string, string[]>;
  userLibraryPermissionDrafts: Record<string, LibraryGrantDrafts>;
  updateUserPasswordDraft: (userId: string, value: string) => void;
  toggleUserAppPermission: (userId: string, permission: string) => void;
  toggleUserLibraryPermission: (userId: string, libraryId: string, permission: string) => void;
  mutatingUserId: string | null;
  setUserPassword: (userId: string) => Promise<void> | void;
  setUserAppPermissions: (userId: string, permissions?: string[]) => Promise<void> | void;
  setUserLibraryPermissions: (
    userId: string,
    libraryId: string,
    permissions?: string[],
  ) => Promise<void> | void;
  deleteUser: (user: UserRecord) => Promise<void> | void;
  externalInviteDraft: ExternalInviteDraft;
  externalInviteSubmitting: boolean;
  updateExternalInviteDraft: (patch: Partial<ExternalInviteDraft>) => void;
  createExternalAccountInvite: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
};

function appPermissionLabel(permission: string): string {
  switch (permission) {
    case "manageUsers":
      return "Manage Users";
    case "managePermissions":
      return "Manage Permissions";
    case "manageSystemSettings":
      return "Manage System Settings";
    case "manageCatalogSettings":
      return "Manage Catalog Settings";
    default:
      return permission;
  }
}

function libraryPermissionLabel(permission: string): string {
  switch (permission) {
    case "view":
      return "View";
    case "manageTitles":
      return "Manage Titles";
    case "resolveImports":
      return "Resolve Imports";
    case "manageLibrary":
      return "Manage Library";
    case "request":
      return "Request";
    case "autoApproveRequests":
      return "Auto-Approve Requests";
    default:
      return permission;
  }
}

function facetLabel(facet: LibraryRecord["facet"]): string {
  switch (facet) {
    case "movie":
      return "Movies";
    case "series":
      return "Series";
    case "anime":
      return "Anime";
    default:
      return String(facet);
  }
}

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
    displayName: id,
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

function inviteProviderOptions(settings: AuthProviderSettings): ExternalAccountProvider[] {
  return settings.allowedProviders.filter(
    (provider) =>
      settings.providerLoginEnabled.includes(provider) &&
      providerConnections(settings, provider).length > 0,
  );
}

function toggleNext(current: string[], permission: string): string[] {
  const next = new Set(current);
  if (next.has(permission)) {
    next.delete(permission);
  } else {
    next.add(permission);
  }
  return Array.from(next);
}

function PermissionChips({
  permissions,
  selected,
  disabled,
  onToggle,
}: {
  permissions: string[];
  selected: string[];
  disabled?: boolean;
  onToggle: (permission: string, next: string[]) => void;
}) {
  return (
    <div className="grid gap-1 sm:grid-cols-2">
      {permissions.map((permission) => (
        <label
          key={permission}
          className="flex min-h-8 items-center gap-2 rounded-md px-2 py-1 hover:bg-card/70"
        >
          <Checkbox
            checked={selected.includes(permission)}
            onCheckedChange={() => onToggle(permission, toggleNext(selected, permission))}
            disabled={disabled}
          />
          <span className="text-xs">{appPermissionLabel(permission)}</span>
        </label>
      ))}
    </div>
  );
}

function LibraryPermissionRow({
  library,
  permissions,
  selected,
  disabled,
  onToggle,
}: {
  library: LibraryRecord;
  permissions: string[];
  selected: string[];
  disabled?: boolean;
  onToggle: (permission: string, next: string[]) => void;
}) {
  return (
    <div className="grid gap-2 border-t border-border py-2 first:border-t-0 md:grid-cols-[12rem_minmax(0,1fr)]">
      <div className="min-w-0">
        <div className="truncate text-sm font-medium text-card-foreground">{library.name}</div>
        <div className="text-xs text-muted-foreground">{facetLabel(library.facet)}</div>
      </div>
      <div className="grid gap-1 sm:grid-cols-2 xl:grid-cols-3">
        {permissions.map((permission) => (
          <label
            key={`${library.id}-${permission}`}
            className="flex min-h-8 items-center gap-2 rounded-md px-2 py-1 hover:bg-card/70"
          >
            <Checkbox
              checked={selected.includes(permission)}
              onCheckedChange={() => onToggle(permission, toggleNext(selected, permission))}
              disabled={disabled}
            />
            <span className="text-xs">{libraryPermissionLabel(permission)}</span>
          </label>
        ))}
      </div>
    </div>
  );
}

function CollapsiblePermissionSection({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <details className="group rounded border border-border bg-background/40 p-2">
      <summary className="flex cursor-pointer list-none items-center gap-2 text-sm font-medium text-card-foreground [&::-webkit-details-marker]:hidden">
        <ChevronRight className="h-4 w-4 text-muted-foreground transition-transform group-open:rotate-90" />
        <span>{title}</span>
      </summary>
      <div className="mt-2">{children}</div>
    </details>
  );
}

export function SettingsUsersSection({
  settingsUsers,
  libraries,
  authProviderSettings,
  currentUserId,
  appPermissions,
  libraryPermissions,
  newUsername,
  setNewUsername,
  newPassword,
  setNewPassword,
  newAppPermissions,
  newLibraryPermissionDrafts,
  toggleNewAppPermission,
  toggleNewLibraryPermission,
  createUser,
  userPasswordDrafts,
  userAppPermissionDrafts,
  userLibraryPermissionDrafts,
  updateUserPasswordDraft,
  toggleUserAppPermission,
  toggleUserLibraryPermission,
  mutatingUserId,
  setUserPassword,
  setUserAppPermissions,
  setUserLibraryPermissions,
  deleteUser,
  externalInviteDraft,
  externalInviteSubmitting,
  updateExternalInviteDraft,
  createExternalAccountInvite,
}: SettingsUsersSectionProps) {
  const t = useTranslate();
  const inviteProviders = inviteProviderOptions(authProviderSettings);
  const inviteConnections = providerConnections(authProviderSettings, externalInviteDraft.provider);
  const inviteUnavailable = inviteProviders.length === 0 || settingsUsers.length === 0;
  return (
    <div id="settings-users-section" className="space-y-4 text-sm">
      <CardTitle className="flex items-center gap-2 text-base">
        <User2 className="h-4 w-4" />
        {t("settings.knownUsers")}
      </CardTitle>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">{t("settings.createUser")}</CardTitle>
        </CardHeader>
        <CardContent>
          <form id="settings-user-create-form" className="space-y-4" onSubmit={createUser}>
            <div className="grid gap-3 md:grid-cols-2">
              <div>
                <Label htmlFor="settings-user-username" className="mb-2 block">
                  {t("settings.username")}
                </Label>
                <Input
                  id="settings-user-username"
                  value={newUsername}
                  onChange={(event) => setNewUsername(event.target.value)}
                  placeholder={t("form.usernamePlaceholder")}
                  required
                />
              </div>
              <div>
                <Label htmlFor="settings-user-password" className="mb-2 block">
                  {t("settings.password")}
                </Label>
                <Input
                  id="settings-user-password"
                  value={newPassword}
                  onChange={(event) => setNewPassword(event.target.value)}
                  placeholder={t("form.passwordPlaceholder")}
                  type="password"
                  required
                />
              </div>
            </div>
            <div className="grid gap-4 lg:grid-cols-[minmax(0,22rem)_minmax(0,1fr)]">
              <div className="rounded border border-border bg-background/40 p-3">
                <Label className="mb-2 block">App Permissions</Label>
                <PermissionChips
                  permissions={appPermissions}
                  selected={newAppPermissions}
                  onToggle={(permission) => toggleNewAppPermission(permission)}
                />
              </div>
              <div className="rounded border border-border bg-background/40 p-3">
                <Label className="mb-2 block">Library Permissions</Label>
                {libraries.map((library) => (
                  <LibraryPermissionRow
                    key={`new-${library.id}`}
                    library={library}
                    permissions={libraryPermissions}
                    selected={newLibraryPermissionDrafts[library.id] ?? []}
                    onToggle={(permission) => toggleNewLibraryPermission(library.id, permission)}
                  />
                ))}
              </div>
            </div>
            <Button id="settings-user-create" type="submit" className="min-w-40">
              {t("settings.createUser")}
            </Button>
          </form>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">{t("settings.externalAccountInvites")}</CardTitle>
        </CardHeader>
        <CardContent>
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
              <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
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
                    {settingsUsers.map((user) => (
                      <option key={user.id} value={user.id}>
                        {user.username}
                      </option>
                    ))}
                  </select>
                </div>
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
                <div className="space-y-1.5">
                  <Label htmlFor="settings-external-invite-username">
                    {t("settings.providerUsername")}
                  </Label>
                  <Input
                    id="settings-external-invite-username"
                    value={externalInviteDraft.username}
                    onChange={(event) =>
                      updateExternalInviteDraft({ username: event.target.value })
                    }
                    disabled={externalInviteSubmitting}
                    placeholder={t("settings.providerUsername")}
                    required
                  />
                </div>
              </div>
              <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_auto]">
                <div className="space-y-1.5">
                  <Label htmlFor="settings-external-invite-external-id">
                    {t("settings.providerExternalUserId")}
                  </Label>
                  <Input
                    id="settings-external-invite-external-id"
                    value={externalInviteDraft.externalUserId}
                    onChange={(event) =>
                      updateExternalInviteDraft({ externalUserId: event.target.value })
                    }
                    disabled={externalInviteSubmitting}
                    placeholder="provider-user-id"
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
                    {externalInviteSubmitting ? t("label.saving") : t("settings.createInvite")}
                  </Button>
                </div>
              </div>
            </form>
          )}
        </CardContent>
      </Card>

      <div className="rounded border border-border">
        <div className="border-b border-border px-3 py-2">
          <CardTitle className="text-base">{t("settings.knownUsers")}</CardTitle>
        </div>
        <Table id="settings-users-table">
          <TableHeader>
            <TableRow>
              <TableHead className="min-w-40">{t("settings.username")}</TableHead>
              <TableHead className="min-w-[520px]">Permissions</TableHead>
              <TableHead className="min-w-72">{t("settings.newPassword")}</TableHead>
              <TableHead className="w-44 text-right">{t("label.actions")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {settingsUsers.length === 0 ? (
              <TableRow>
                <TableCell colSpan={4} className="text-muted-foreground">
                  {t("settings.noUsers")}
                </TableCell>
              </TableRow>
            ) : (
              settingsUsers.map((user) => {
                const isOwnUser = currentUserId === user.id;
                const appSelected = userAppPermissionDrafts[user.id] ?? user.appPermissions;
                const libraryDrafts =
                  userLibraryPermissionDrafts[user.id] ??
                  Object.fromEntries(
                    user.libraryPermissions.map((grant) => [
                      grant.libraryId,
                      grant.permissions,
                    ]),
                  );
                return (
                  <TableRow key={user.id} id={selectorId("settings-user-row", user.username)}>
                    <TableCell className="align-top">
                      <div className="text-lg font-semibold text-foreground">{user.username}</div>
                    </TableCell>
                    <TableCell className="align-top">
                      <div className="space-y-3">
                        <CollapsiblePermissionSection title="App Permissions">
                          <PermissionChips
                            permissions={appPermissions}
                            selected={appSelected}
                            disabled={mutatingUserId === user.id || isOwnUser}
                            onToggle={(permission, nextPermissions) => {
                              if (isOwnUser) return;
                              toggleUserAppPermission(user.id, permission);
                              void setUserAppPermissions(user.id, nextPermissions);
                            }}
                          />
                        </CollapsiblePermissionSection>
                        <CollapsiblePermissionSection title="Library Permissions">
                          {libraries.map((library) => (
                            <LibraryPermissionRow
                              key={`${user.id}-${library.id}`}
                              library={library}
                              permissions={libraryPermissions}
                              selected={libraryDrafts[library.id] ?? []}
                              disabled={mutatingUserId === user.id || isOwnUser}
                              onToggle={(permission, nextPermissions) => {
                                if (isOwnUser) return;
                                toggleUserLibraryPermission(user.id, library.id, permission);
                                void setUserLibraryPermissions(
                                  user.id,
                                  library.id,
                                  nextPermissions,
                                );
                              }}
                            />
                          ))}
                        </CollapsiblePermissionSection>
                      </div>
                    </TableCell>
                    <TableCell className="align-middle">
                      <div className="flex items-center gap-2">
                        <label className="sr-only" htmlFor={`new-password-${user.id}`}>
                          {t("settings.newPassword")}
                        </label>
                        <Input
                          id={`new-password-${user.id}`}
                          value={userPasswordDrafts[user.id] ?? ""}
                          onChange={(event) => updateUserPasswordDraft(user.id, event.target.value)}
                          placeholder={t("form.newPasswordPlaceholder")}
                          type="password"
                          aria-label={t("settings.newPassword")}
                        />
                        <Button
                          id={selectorId("settings-user-update-password", user.username)}
                          variant="primary"
                          size="sm"
                          className="min-w-44"
                          onClick={() => void setUserPassword(user.id)}
                          disabled={mutatingUserId === user.id}
                        >
                          <KeyRound className="mr-1 h-3.5 w-3.5" />
                          {mutatingUserId === user.id ? t("label.saving") : t("settings.updatePassword")}
                        </Button>
                      </div>
                    </TableCell>
                    <TableCell className="align-middle text-right">
                      <div className="flex justify-end gap-2">
                        <Button
                          id={selectorId("settings-user-delete", user.username)}
                          variant="destructive"
                          size="sm"
                          onClick={() => void deleteUser(user)}
                          disabled={mutatingUserId === user.id || isOwnUser}
                        >
                          <Trash2 className="mr-1 h-3.5 w-3.5" />
                          {t("label.delete")}
                        </Button>
                      </div>
                    </TableCell>
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
