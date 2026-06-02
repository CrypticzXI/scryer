import type * as React from "react";
import { ChevronRight, KeyRound, Plus, ShieldOff, Trash2, User2 } from "lucide-react";
import {
  PermissionDropdowns,
  type LibraryPermissionDrafts,
} from "@/components/common/permission-checkboxes";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
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
import { cn } from "@/lib/utils";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
} from "@/lib/utils/action-button-styles";
import { selectorId } from "@/lib/utils/dom-ids";

type SettingsUsersSectionProps = {
  settingsUsers: UserRecord[];
  libraries: LibraryRecord[];
  externalAccountInvitesPanel: React.ReactNode;
  currentUserId?: string | null;
  appPermissions: string[];
  libraryPermissions: string[];
  newUsername: string;
  setNewUsername: (value: string) => void;
  newPassword: string;
  setNewPassword: (value: string) => void;
  newAppPermissions: string[];
  newLibraryPermissionDrafts: LibraryPermissionDrafts;
  canManagePermissions: boolean;
  toggleNewAppPermission: (value: string) => void;
  toggleNewLibraryPermission: (libraryId: string, value: string) => void;
  createUser: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  userPasswordDrafts: Record<string, string>;
  userAppPermissionDrafts: Record<string, string[]>;
  userLibraryPermissionDrafts: Record<string, LibraryPermissionDrafts>;
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
  resetUserMfa: (user: UserRecord) => Promise<void> | void;
};

function CollapsiblePermissionSection({
  id,
  title,
  children,
}: {
  id?: string;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <details id={id} className="group rounded border border-border bg-background/40 p-2">
      <summary className="flex cursor-pointer list-none items-center gap-2 text-sm font-medium text-card-foreground [&::-webkit-details-marker]:hidden">
        <ChevronRight className="h-4 w-4 text-muted-foreground transition-transform group-open:rotate-90" />
        <span>{title}</span>
      </summary>
      <div className="mt-2">{children}</div>
    </details>
  );
}

function AuthFactorStatusBadge({ enabled }: { enabled: boolean }) {
  const t = useTranslate();
  return (
    <span
      className={cn(
        "inline-flex min-w-24 items-center justify-center rounded border px-2 py-1 text-xs font-medium",
        enabled
          ? "border-emerald-500/35 bg-emerald-500/10 text-emerald-700 dark:text-emerald-200"
          : "border-border bg-background text-muted-foreground",
      )}
    >
      {enabled ? t("settings.setUp") : t("settings.notSetUp")}
    </span>
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
  canManagePermissions,
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
  resetUserMfa,
  externalAccountInvitesPanel,
}: SettingsUsersSectionProps) {
  const t = useTranslate();
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
            {canManagePermissions ? (
              <div className="rounded border border-border bg-background/40 p-3">
                <Label className="mb-2 block">{t("settings.permissions")}</Label>
                <PermissionDropdowns
                  libraries={libraries}
                  appPermissions={appPermissions}
                  libraryPermissions={libraryPermissions}
                  selectedAppPermissions={newAppPermissions}
                  selectedLibraryPermissions={newLibraryPermissionDrafts}
                  onAppChange={(_nextPermissions, permission) =>
                    toggleNewAppPermission(permission)
                  }
                  onLibraryChange={(libraryId, _nextPermissions, permission) =>
                    toggleNewLibraryPermission(libraryId, permission)
                  }
                />
              </div>
            ) : null}
            <Button id="settings-user-create" type="submit" className="min-w-40">
              <Plus className="h-4 w-4" />
              {t("settings.createUser")}
            </Button>
          </form>
        </CardContent>
      </Card>

      {externalAccountInvitesPanel}

      <div className="rounded border border-border">
        <div className="border-b border-border px-3 py-2">
          <CardTitle className="text-base">{t("settings.knownUsers")}</CardTitle>
        </div>
        <Table id="settings-users-table">
          <TableHeader>
            <TableRow>
              <TableHead className="min-w-40">{t("settings.username")}</TableHead>
              <TableHead className="min-w-[520px]">Permissions</TableHead>
              <TableHead className="w-32">{t("settings.mfa")}</TableHead>
              <TableHead className="w-32">{t("settings.passkey")}</TableHead>
              <TableHead className="min-w-72">{t("settings.newPassword")}</TableHead>
              <TableHead className="w-44 text-right">{t("label.actions")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {settingsUsers.length === 0 ? (
              <TableRow>
                <TableCell colSpan={6} className="text-muted-foreground">
                  {t("settings.noUsers")}
                </TableCell>
              </TableRow>
            ) : (
              settingsUsers.map((user) => {
                const isOwnUser = currentUserId === user.id;
                const canSetPassword = user.accountKind !== "external_auto_provisioned";
                const permissionControlsDisabled =
                  mutatingUserId === user.id || isOwnUser || !canManagePermissions;
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
                      <CollapsiblePermissionSection
                        id={selectorId("settings-user-permissions", user.username, "section")}
                        title="Permissions"
                      >
                        <PermissionDropdowns
                          libraries={libraries}
                          appPermissions={appPermissions}
                          libraryPermissions={libraryPermissions}
                          idPrefix={selectorId("settings-user-permissions", user.username)}
                          selectedAppPermissions={appSelected}
                          selectedLibraryPermissions={libraryDrafts}
                          disabled={permissionControlsDisabled}
                          onAppChange={(nextPermissions, permission) => {
                            if (isOwnUser || !canManagePermissions) return;
                            toggleUserAppPermission(user.id, permission);
                            void setUserAppPermissions(user.id, nextPermissions);
                          }}
                          onLibraryChange={(libraryId, nextPermissions, permission) => {
                            if (isOwnUser || !canManagePermissions) return;
                            toggleUserLibraryPermission(user.id, libraryId, permission);
                            void setUserLibraryPermissions(
                              user.id,
                              libraryId,
                              nextPermissions,
                            );
                          }}
                        />
                      </CollapsiblePermissionSection>
                    </TableCell>
                    <TableCell className="align-middle">
                      <AuthFactorStatusBadge enabled={user.hasMfa} />
                    </TableCell>
                    <TableCell className="align-middle">
                      <AuthFactorStatusBadge enabled={user.hasPasskey} />
                    </TableCell>
                    <TableCell className="align-middle">
                      {canSetPassword ? (
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
                            disabled={isOwnUser}
                          />
                          <Button
                            id={selectorId("settings-user-update-password", user.username)}
                            variant="primary"
                            size="sm"
                            className="min-w-24 px-2"
                            onClick={() => void setUserPassword(user.id)}
                            disabled={mutatingUserId === user.id || isOwnUser}
                          >
                            <KeyRound className="h-3.5 w-3.5" />
                            {mutatingUserId === user.id ? t("label.saving") : t("settings.updatePassword")}
                          </Button>
                        </div>
                      ) : (
                        <span className="text-sm text-muted-foreground">
                          {t("settings.passwordManagedExternally")}
                        </span>
                      )}
                    </TableCell>
                    <TableCell className="align-middle text-right">
                      <div className="flex justify-end gap-2">
                        {!isOwnUser && user.hasMfa ? (
                          <Button
                            id={selectorId("settings-user-reset-mfa", user.username)}
                            type="button"
                            variant="secondary"
                            size="icon-sm"
                            title={t("settings.resetMfa")}
                            aria-label={t("settings.resetMfa")}
                            className={cn(
                              boxedActionButtonBaseClass,
                              boxedActionButtonToneClass.neutral,
                            )}
                            onClick={() => void resetUserMfa(user)}
                            disabled={mutatingUserId === user.id}
                          >
                            <ShieldOff className="h-4 w-4" />
                          </Button>
                        ) : null}
                        <Button
                          id={selectorId("settings-user-delete", user.username)}
                          type="button"
                          variant="secondary"
                          size="icon-sm"
                          title={t("label.delete")}
                          aria-label={t("label.delete")}
                          className={cn(
                            boxedActionButtonBaseClass,
                            boxedActionButtonToneClass.delete,
                          )}
                          onClick={() => void deleteUser(user)}
                          disabled={mutatingUserId === user.id || isOwnUser}
                        >
                          <Trash2 className="h-4 w-4" />
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
