import test from "node:test";
import assert from "node:assert/strict";

import {
  APP_PERMISSIONS,
  LIBRARY_PERMISSIONS,
  hasAppPermission,
  hasLibraryPermission,
  normalizeJwtPermissionClaims,
} from "./permissions.ts";

test("normalizeJwtPermissionClaims restores camelCase JWT permissions", () => {
  const user = normalizeJwtPermissionClaims(
    [
      "manageUsers",
      "MANAGE_PERMISSIONS",
      "manageSystemSettings",
      "manageCatalogSettings",
      "manageUsers",
      "futurePermission",
    ],
    [
      {
        libraryId: " library-primary ",
        permissions: ["view", "MANAGE_TITLES", "futureLibraryPermission"],
      },
      {
        libraryId: "library-primary",
        permissions: ["resolveImports", "manageLibrary"],
      },
      {
        libraryId: "library-secondary",
        permissions: ["request", "autoApproveRequests"],
      },
    ],
  );

  assert.deepEqual(user, {
    appPermissions: [
      APP_PERMISSIONS.manageUsers,
      APP_PERMISSIONS.managePermissions,
      APP_PERMISSIONS.manageSystemSettings,
      APP_PERMISSIONS.manageCatalogSettings,
    ],
    libraryPermissions: [
      {
        libraryId: "library-primary",
        permissions: [
          LIBRARY_PERMISSIONS.view,
          LIBRARY_PERMISSIONS.manageTitles,
          LIBRARY_PERMISSIONS.resolveImports,
          LIBRARY_PERMISSIONS.manageLibrary,
        ],
      },
      {
        libraryId: "library-secondary",
        permissions: [
          LIBRARY_PERMISSIONS.request,
          LIBRARY_PERMISSIONS.autoApproveRequests,
        ],
      },
    ],
  });
  assert.equal(hasAppPermission(user, APP_PERMISSIONS.manageUsers), true);
  assert.equal(
    hasLibraryPermission(
      user,
      "library-primary",
      LIBRARY_PERMISSIONS.manageTitles,
    ),
    true,
  );
});

test("normalizeJwtPermissionClaims discards malformed and unknown claims", () => {
  const user = normalizeJwtPermissionClaims(
    [null, "", "not-a-permission"],
    [
      null,
      { libraryId: "", permissions: ["view"] },
      { libraryId: 42, permissions: ["manageTitles"] },
      { libraryId: "library-primary", permissions: [null, "unknown"] },
    ],
  );

  assert.deepEqual(user, {
    appPermissions: [],
    libraryPermissions: [{ libraryId: "library-primary", permissions: [] }],
  });
});
