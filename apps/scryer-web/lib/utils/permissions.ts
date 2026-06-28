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

export function authorizationCacheSignature(
  user: PermissionUser | null | undefined,
): string {
  const appPermissions = Array.from(new Set(user?.appPermissions ?? []))
    .sort()
    .join(",");
  const libraryPermissions = (user?.libraryPermissions ?? [])
    .map((grant) => {
      const libraryId = grant.libraryId.trim();
      const permissions = Array.from(new Set(grant.permissions)).sort().join(",");
      return `${libraryId}:${permissions}`;
    })
    .sort()
    .join("|");
  return `app=${appPermissions};libraries=${libraryPermissions}`;
}

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
  return user.libraryPermissions.some((grant) => {
    if (grant.libraryId !== libraryId) {
      return false;
    }
    return libraryPermissionMatches(grant.permissions, permission);
  });
}

export function hasAnyLibraryPermission(
  user: PermissionUser | null | undefined,
  permission: LibraryPermission,
): boolean {
  return user?.libraryPermissions.some((grant) =>
    libraryPermissionMatches(grant.permissions, permission),
  ) === true;
}

export function libraryPermissionsWithRequestShadowing(values: string[]): string[] {
  const next = new Set(values);
  if (next.has(LIBRARY_PERMISSIONS.manageTitles)) {
    next.add(LIBRARY_PERMISSIONS.autoApproveRequests);
    next.add(LIBRARY_PERMISSIONS.request);
  } else if (next.has(LIBRARY_PERMISSIONS.autoApproveRequests)) {
    next.add(LIBRARY_PERMISSIONS.request);
  }
  return Array.from(next);
}

export function normalizeLibraryPermissionsForStorage(values: string[]): string[] {
  const next = new Set(values);
  if (next.has(LIBRARY_PERMISSIONS.manageTitles)) {
    next.delete(LIBRARY_PERMISSIONS.autoApproveRequests);
    next.delete(LIBRARY_PERMISSIONS.request);
  } else if (next.has(LIBRARY_PERMISSIONS.autoApproveRequests)) {
    next.delete(LIBRARY_PERMISSIONS.request);
  }
  return Array.from(next);
}

export function libraryPermissionShadowSource(
  explicitValues: string[],
  permission: string,
): string | null {
  const explicit = new Set(explicitValues);
  if (
    (permission === LIBRARY_PERMISSIONS.request ||
      permission === LIBRARY_PERMISSIONS.autoApproveRequests) &&
    explicit.has(LIBRARY_PERMISSIONS.manageTitles)
  ) {
    return "Manage Titles";
  }
  if (
    permission === LIBRARY_PERMISSIONS.request &&
    explicit.has(LIBRARY_PERMISSIONS.autoApproveRequests)
  ) {
    return "Auto-Approve Requests";
  }
  return null;
}

function libraryPermissionMatches(
  values: LibraryPermission[],
  permission: LibraryPermission,
): boolean {
  const explicit = new Set<string>(values);
  switch (permission) {
    case LIBRARY_PERMISSIONS.request:
      return (
        !explicit.has(LIBRARY_PERMISSIONS.manageTitles) &&
        libraryPermissionsWithRequestShadowing(values).includes(permission)
      );
    case LIBRARY_PERMISSIONS.autoApproveRequests:
      return (
        !explicit.has(LIBRARY_PERMISSIONS.manageTitles) &&
        libraryPermissionsWithRequestShadowing(values).includes(permission)
      );
    default:
      return explicit.has(permission);
  }
}
