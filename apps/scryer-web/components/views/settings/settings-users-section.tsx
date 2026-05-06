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

type LibraryGrantDrafts = Record<string, string[]>;

type SettingsUsersSectionProps = {
  settingsUsers: UserRecord[];
  libraries: LibraryRecord[];
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
}: SettingsUsersSectionProps) {
  const t = useTranslate();
  return (
    <div className="space-y-4 text-sm">
      <CardTitle className="flex items-center gap-2 text-base">
        <User2 className="h-4 w-4" />
        {t("settings.knownUsers")}
      </CardTitle>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">{t("settings.createUser")}</CardTitle>
        </CardHeader>
        <CardContent>
          <form className="space-y-4" onSubmit={createUser}>
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
            <Button type="submit" className="min-w-40">
              {t("settings.createUser")}
            </Button>
          </form>
        </CardContent>
      </Card>

      <div className="rounded border border-border">
        <div className="border-b border-border px-3 py-2">
          <CardTitle className="text-base">{t("settings.knownUsers")}</CardTitle>
        </div>
        <Table>
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
                  <TableRow key={user.id}>
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
