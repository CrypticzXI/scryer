
import { useCallback, useEffect, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsUsersSection } from "@/components/views/settings/settings-users-section";
import {
  createUserMutation,
  deleteUserMutation,
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
import { APP_PERMISSIONS, LIBRARY_PERMISSIONS } from "@/lib/utils/permissions";

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
      normalizePermissions(grant.permissions),
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
          permissions: normalizePermissions(grant.permissions),
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
          appPermissions: newAppPermissions,
          libraryPermissions: Object.entries(newLibraryPermissionDrafts)
            .filter(([, permissions]) => permissions.length > 0)
            .map(([libraryId, permissions]) => ({ libraryId, permissions })),
        },
      }).toPromise();
      if (error) throw error;
      setNewUsername("");
      setNewPassword("");
      setNewAppPermissions([]);
      setNewLibraryPermissionDrafts({});
      setGlobalStatus(t("user.created"));
      await refreshUsers();
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
    const user = settingsUsers.find((candidate) => candidate.id === userId);
    const currentDrafts = userLibraryPermissionDrafts[userId] ?? {};
    const nextDrafts = {
      ...currentDrafts,
      [libraryId]: normalizePermissions(permissions ?? currentDrafts[libraryId]),
    };
    const grants = Object.entries(nextDrafts)
      .filter(([, grantPermissions]) => grantPermissions.length > 0)
      .map(([grantLibraryId, grantPermissions]) => ({
        libraryId: grantLibraryId,
        permissions: grantPermissions,
      }));
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

  const confirmDeleteUser = async () => {
    if (!pendingDeleteUser) {
      return;
    }
    const user = pendingDeleteUser;
    setMutatingUserId(user.id);
    try {
      const { error } = await client.mutation(deleteUserMutation, {
        input: { userId: user.id },
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

  return (
    <>
      <SettingsUsersSection
        settingsUsers={settingsUsers}
        libraries={libraries}
        newUsername={newUsername}
        setNewUsername={setNewUsername}
        newPassword={newPassword}
        setNewPassword={setNewPassword}
        appPermissions={Object.values(APP_PERMISSIONS)}
        libraryPermissions={Object.values(LIBRARY_PERMISSIONS)}
        newAppPermissions={newAppPermissions}
        newLibraryPermissionDrafts={newLibraryPermissionDrafts}
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
        currentUserId={currentUser?.id ?? null}
      />
      <ConfirmDialog
        open={pendingDeleteUser !== null}
        title={t("label.delete")}
        description={pendingDeleteUser ? t("status.deletingUser", { name: pendingDeleteUser.username }) : ""}
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={mutatingUserId !== null}
        onConfirm={confirmDeleteUser}
        onCancel={() => setPendingDeleteUser(null)}
      />
    </>
  );
}
