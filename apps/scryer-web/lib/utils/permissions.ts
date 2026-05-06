export const APP_PERMISSIONS = {
  manageUsers: "manageUsers",
  managePermissions: "managePermissions",
  manageSystemSettings: "manageSystemSettings",
  manageCatalogSettings: "manageCatalogSettings",
} as const;

export const LIBRARY_PERMISSIONS = {
  view: "view",
  manageTitles: "manageTitles",
  resolveImports: "resolveImports",
  manageLibrary: "manageLibrary",
  request: "request",
  autoApproveRequests: "autoApproveRequests",
} as const;

export type AppPermission = (typeof APP_PERMISSIONS)[keyof typeof APP_PERMISSIONS];
export type LibraryPermission = (typeof LIBRARY_PERMISSIONS)[keyof typeof LIBRARY_PERMISSIONS];

export type LibraryPermissionGrant = {
  libraryId: string;
  permissions: LibraryPermission[];
};

export type PermissionUser = {
  appPermissions: AppPermission[];
  libraryPermissions: LibraryPermissionGrant[];
};

export function hasAppPermission(user: PermissionUser | null | undefined, permission: AppPermission): boolean {
  return user?.appPermissions.includes(permission) === true;
}

export function hasAnyAppPermission(
  user: PermissionUser | null | undefined,
  permissions: AppPermission[],
): boolean {
  return permissions.some((permission) => hasAppPermission(user, permission));
}

export function hasLibraryPermission(
  user: PermissionUser | null | undefined,
  libraryId: string | null | undefined,
  permission: LibraryPermission,
): boolean {
  if (!user || !libraryId) {
    return false;
  }
  return user.libraryPermissions.some(
    (grant) => grant.libraryId === libraryId && grant.permissions.includes(permission),
  );
}

export function hasAnyLibraryPermission(
  user: PermissionUser | null | undefined,
  permission: LibraryPermission,
): boolean {
  return user?.libraryPermissions.some((grant) => grant.permissions.includes(permission)) === true;
}
