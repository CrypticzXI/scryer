import type * as React from "react";
import { useCallback, useEffect, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import {
  ExternalAccountInvitesContainer,
  notifyExternalAccountInviteSourcesChanged,
} from "@/components/containers/settings/external-account-invites-container";
import { SettingsUsersSection } from "@/components/views/settings/settings-users-section";
import {
  createUserMutation,
  deleteUserMutation,
  resetUserMfaMutation,
  setUserAppPermissionsMutation,
  setUserLibraryPermissionsMutation,
  setUserPasswordMutation,
} from "@/lib/graphql/mutations";
import { librariesQuery, usersQuery } from "@/lib/graphql/queries";
import { useAuth } from "@/lib/hooks/use-auth";
import { useClient } from "urql";
import type { LibraryRecord, UserRecord } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import {
  APP_PERMISSIONS,
  LIBRARY_PERMISSIONS,
  normalizeLibraryPermissionsForStorage,
} from "@/lib/utils/permissions";

type LibraryGrantDrafts = Record<string, string[]>;

function normalizePermissions(values: string[] | null | undefined): string[] {
  return Array.from(new Set((values ?? []).map((value) => value.trim()).filter(Boolean)));
}

function grantsToDrafts(
  grants: UserRecord["libraryPermissions"] | null | undefined,
): LibraryGrantDrafts {
  return Object.fromEntries(
    (grants ?? []).map((grant) => [
      grant.libraryId,
      normalizeLibraryPermissionsForStorage(normalizePermissions(grant.permissions)),
    ]),
  );
}

function togglePermission(current: string[], value: string): string[] {
  const existing = new Set(current);
  if (existing.has(value)) {
    existing.delete(value);
  } else {
    existing.add(value);
  }
  return Array.from(existing);
}

export function SettingsUsersContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const { user: currentUser } = useAuth();
  const [settingsUsers, setSettingsUsers] = useState<UserRecord[]>([]);
  const [libraries, setLibraries] = useState<LibraryRecord[]>([]);
  const [newUsername, setNewUsername] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [newAppPermissions, setNewAppPermissions] = useState<string[]>([]);
  const [newLibraryPermissionDrafts, setNewLibraryPermissionDrafts] = useState<LibraryGrantDrafts>({});
  const [userPasswordDrafts, setUserPasswordDrafts] = useState<Record<string, string>>({});
  const [userAppPermissionDrafts, setUserAppPermissionDrafts] = useState<Record<string, string[]>>({});
  const [userLibraryPermissionDrafts, setUserLibraryPermissionDrafts] = useState<Record<string, LibraryGrantDrafts>>({});
  const [mutatingUserId, setMutatingUserId] = useState<string | null>(null);
  const [pendingDeleteUser, setPendingDeleteUser] = useState<UserRecord | null>(null);
  const [pendingResetMfaUser, setPendingResetMfaUser] = useState<UserRecord | null>(null);
  const canManagePermissions =
    currentUser?.appPermissions?.includes(APP_PERMISSIONS.managePermissions) ?? false;

  const updateUserPasswordDraft = useCallback((userId: string, value: string) => {
    setUserPasswordDrafts((previous) => ({ ...previous, [userId]: value }));
  }, []);

  const toggleNewAppPermission = useCallback((value: string) => {
    setNewAppPermissions((previous) => togglePermission(previous, value));
  }, []);

  const toggleNewLibraryPermission = useCallback((libraryId: string, value: string) => {
    setNewLibraryPermissionDrafts((previous) => ({
      ...previous,
      [libraryId]: togglePermission(previous[libraryId] ?? [], value),
    }));
  }, []);

  const toggleUserAppPermission = useCallback((userId: string, value: string) => {
    setUserAppPermissionDrafts((previous) => ({
      ...previous,
      [userId]: togglePermission(previous[userId] ?? [], value),
    }));
  }, []);

  const toggleUserLibraryPermission = useCallback((userId: string, libraryId: string, value: string) => {
    setUserLibraryPermissionDrafts((previous) => {
      const grants = previous[userId] ?? {};
      return {
        ...previous,
        [userId]: {
          ...grants,
          [libraryId]: togglePermission(grants[libraryId] ?? [], value),
        },
      };
    });
  }, []);

  const refreshUsers = useCallback(async () => {
    try {
      const { data, error } = await client.query(usersQuery, {}).toPromise();
      if (error) throw error;
      const users = (data.users || []).map((user: UserRecord) => ({
        ...user,
        appPermissions: normalizePermissions(user.appPermissions),
        libraryPermissions: (user.libraryPermissions ?? []).map((grant) => ({
          libraryId: grant.libraryId,
          permissions: normalizeLibraryPermissionsForStorage(
            normalizePermissions(grant.permissions),
          ),
        })),
      }));
      setSettingsUsers(users);
      setUserAppPermissionDrafts(
        Object.fromEntries(users.map((user: UserRecord) => [user.id, [...user.appPermissions]])),
      );
      setUserLibraryPermissionDrafts(
        Object.fromEntries(users.map((user: UserRecord) => [user.id, grantsToDrafts(user.libraryPermissions)])),
      );
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    }
  }, [client, setGlobalStatus, t]);

  const refreshLibraries = useCallback(async () => {
    try {
      const { data, error } = await client
        .query(librariesQuery, { facet: null, permission: "view" })
        .toPromise();
      if (error) throw error;
      setLibraries((data?.libraries ?? []) as LibraryRecord[]);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    void refreshUsers();
    void refreshLibraries();
  }, [refreshLibraries, refreshUsers]);

  const createUser = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!newUsername.trim() || !newPassword.trim()) {
      setGlobalStatus(t("status.userRequired"));
      return;
    }
    try {
      const { error } = await client.mutation(createUserMutation, {
        input: {
          username: newUsername.trim(),
          password: newPassword,
          appPermissions: canManagePermissions ? newAppPermissions : [],
          libraryPermissions: canManagePermissions
            ? Object.entries(newLibraryPermissionDrafts)
                .map(([libraryId, permissions]) => ({
                  libraryId,
                  permissions: normalizeLibraryPermissionsForStorage(permissions),
                }))
                .filter((grant) => grant.permissions.length > 0)
            : [],
        },
      }).toPromise();
      if (error) throw error;
      setNewUsername("");
      setNewPassword("");
      setNewAppPermissions([]);
      setNewLibraryPermissionDrafts({});
      setGlobalStatus(t("user.created"));
      await refreshUsers();
      notifyExternalAccountInviteSourcesChanged();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToCreate"));
    }
  };

  const setUserPassword = async (userId: string) => {
    const password = userPasswordDrafts[userId]?.trim();
    if (!password) {
      setGlobalStatus(t("status.passwordRequired"));
      return;
    }
    setMutatingUserId(userId);
    try {
      const { error } = await client.mutation(setUserPasswordMutation, {
        input: {
          userId,
          password,
        },
      }).toPromise();
      if (error) throw error;
      setUserPasswordDrafts((previous) => ({
        ...previous,
        [userId]: "",
      }));
      setGlobalStatus(t("user.passwordUpdated"));
      await refreshUsers();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingUserId(null);
    }
  };

  const setUserAppPermissions = async (userId: string, permissions?: string[]) => {
    if (!canManagePermissions) {
      setGlobalStatus(t("status.managePermissionsRequired"));
      return;
    }
    const resolvedPermissions = normalizePermissions(permissions ?? userAppPermissionDrafts[userId]);
    const updatedUserName =
      settingsUsers.find((candidate) => candidate.id === userId)?.username ?? userId;
    setMutatingUserId(userId);
    try {
      const { error } = await client.mutation(setUserAppPermissionsMutation, {
        input: {
          userId,
          permissions: resolvedPermissions,
        },
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("user.permissionsUpdated", { name: updatedUserName }));
      await refreshUsers();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingUserId(null);
    }
  };

  const setUserLibraryPermissions = async (
    userId: string,
    libraryId: string,
    permissions?: string[],
  ) => {
    if (!canManagePermissions) {
      setGlobalStatus(t("status.managePermissionsRequired"));
      return;
    }
    const user = settingsUsers.find((candidate) => candidate.id === userId);
    const currentDrafts = userLibraryPermissionDrafts[userId] ?? {};
    const nextDrafts = {
      ...currentDrafts,
      [libraryId]: normalizePermissions(permissions ?? currentDrafts[libraryId]),
    };
    const grants = Object.entries(nextDrafts)
      .map(([grantLibraryId, grantPermissions]) => ({
        libraryId: grantLibraryId,
        permissions: normalizeLibraryPermissionsForStorage(grantPermissions),
      }))
      .filter((grant) => grant.permissions.length > 0);
    setMutatingUserId(userId);
    try {
      const { error } = await client.mutation(setUserLibraryPermissionsMutation, {
        input: { userId, grants },
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("user.permissionsUpdated", { name: user?.username ?? userId }));
      await refreshUsers();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingUserId(null);
    }
  };

  const deleteUser = async (user: UserRecord) => {
    setPendingDeleteUser(user);
  };

  const resetUserMfa = async (user: UserRecord) => {
    setPendingResetMfaUser(user);
  };

  const confirmDeleteUser = async () => {
    if (!pendingDeleteUser) {
      return;
    }
    const user = pendingDeleteUser;
    setMutatingUserId(user.id);
    try {
      const { error } = await client.mutation(deleteUserMutation, {
        id: user.id,
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.deletingUser", { name: user.username }));
      await refreshUsers();
      setUserPasswordDrafts((previous) => {
        const next = { ...previous };
        delete next[user.id];
        return next;
      });
      setUserAppPermissionDrafts((previous) => {
        const next = { ...previous };
        delete next[user.id];
        return next;
      });
      setUserLibraryPermissionDrafts((previous) => {
        const next = { ...previous };
        delete next[user.id];
        return next;
      });
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingUserId(null);
      setPendingDeleteUser(null);
    }
  };

  const confirmResetUserMfa = async () => {
    if (!pendingResetMfaUser) {
      return;
    }
    const user = pendingResetMfaUser;
    setMutatingUserId(user.id);
    try {
      const { error } = await client.mutation(resetUserMfaMutation, {
        id: user.id,
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("user.mfaReset", { name: user.username }));
      await refreshUsers();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingUserId(null);
      setPendingResetMfaUser(null);
    }
  };

  return (
    <>
      <SettingsUsersSection
        settingsUsers={settingsUsers}
        libraries={libraries}
        externalAccountInvitesPanel={<ExternalAccountInvitesContainer />}
        newUsername={newUsername}
        setNewUsername={setNewUsername}
        newPassword={newPassword}
        setNewPassword={setNewPassword}
        appPermissions={Object.values(APP_PERMISSIONS)}
        libraryPermissions={Object.values(LIBRARY_PERMISSIONS)}
        newAppPermissions={newAppPermissions}
        newLibraryPermissionDrafts={newLibraryPermissionDrafts}
        canManagePermissions={canManagePermissions}
        toggleNewAppPermission={toggleNewAppPermission}
        toggleNewLibraryPermission={toggleNewLibraryPermission}
        createUser={createUser}
        userPasswordDrafts={userPasswordDrafts}
        userAppPermissionDrafts={userAppPermissionDrafts}
        userLibraryPermissionDrafts={userLibraryPermissionDrafts}
        updateUserPasswordDraft={updateUserPasswordDraft}
        toggleUserAppPermission={toggleUserAppPermission}
        toggleUserLibraryPermission={toggleUserLibraryPermission}
        mutatingUserId={mutatingUserId}
        setUserPassword={setUserPassword}
        setUserAppPermissions={setUserAppPermissions}
        setUserLibraryPermissions={setUserLibraryPermissions}
        deleteUser={deleteUser}
        resetUserMfa={resetUserMfa}
        currentUserId={currentUser?.id ?? null}
      />
      <ConfirmDialog
        open={pendingDeleteUser !== null}
        contentId="settings-user-delete-dialog"
        title={t("label.delete")}
        description={pendingDeleteUser ? t("status.deletingUser", { name: pendingDeleteUser.username }) : ""}
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        confirmButtonId="settings-user-delete-confirm"
        cancelButtonId="settings-user-delete-cancel"
        isBusy={mutatingUserId !== null}
        onConfirm={confirmDeleteUser}
        onCancel={() => setPendingDeleteUser(null)}
      />
      <ConfirmDialog
        open={pendingResetMfaUser !== null}
        title={t("settings.resetMfa")}
        description={
          pendingResetMfaUser
            ? t("settings.resetMfaConfirm", { name: pendingResetMfaUser.username })
            : ""
        }
        confirmLabel={t("settings.resetMfa")}
        cancelLabel={t("label.cancel")}
        isBusy={mutatingUserId !== null}
        onConfirm={confirmResetUserMfa}
        onCancel={() => setPendingResetMfaUser(null)}
      />
    </>
  );
}
