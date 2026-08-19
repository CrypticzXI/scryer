import assert from "node:assert/strict";
import test from "node:test";

import {
  canAccessRecycleBinPage,
  canAccessSystemSection,
  defaultAccessibleRoute,
} from "./routes.ts";

/**
 * `defaultAccessibleRoute` is what `/` resolves to once the signed-in user is
 * known, so these cases are the two user classes landing on the root path.
 */
function landingFor({
  canViewCatalog = false,
  canRequestMedia = false,
  canResolveImports = false,
  canManageUserAccounts = false,
  canManageUserAccess = false,
  canManageSystemSettings = false,
  canManageCatalogSettings = false,
  canManageLibrarySettings = false,
} = {}) {
  return defaultAccessibleRoute(
    canViewCatalog,
    canRequestMedia,
    canResolveImports,
    canManageUserAccounts,
    canManageUserAccess,
    canManageSystemSettings,
    canManageCatalogSettings,
    canManageLibrarySettings,
  );
}

test("system-settings managers land on the dashboard", () => {
  assert.deepEqual(landingFor({ canManageSystemSettings: true }), {
    view: "dashboard",
  });
  // The dashboard outranks the catalog even when both are reachable.
  assert.deepEqual(
    landingFor({ canManageSystemSettings: true, canViewCatalog: true }),
    { view: "dashboard" },
  );
});

test("everyone else lands on the route they can actually open", () => {
  assert.deepEqual(landingFor({ canViewCatalog: true }), {
    view: "movies",
    contentSettingsSection: "overview",
  });
  // A catalog manager without system settings is not an admin here.
  assert.deepEqual(
    landingFor({ canViewCatalog: true, canManageCatalogSettings: true }),
    { view: "movies", contentSettingsSection: "overview" },
  );
  assert.deepEqual(landingFor({ canRequestMedia: true }), { view: "requests" });
  assert.deepEqual(landingFor({ canResolveImports: true }), {
    view: "movies",
    contentSettingsSection: "import",
  });
  assert.deepEqual(landingFor(), {
    view: "settings",
    settingsSection: "profile",
  });
});

test("recycle-bin access follows the broader page-access permission", () => {
  assert.equal(canAccessSystemSection("recycleBin", false, false), false);
  assert.equal(canAccessSystemSection("recycleBin", false, true), true);
  assert.equal(canAccessSystemSection("recycleBin", true, false), true);
  assert.equal(canAccessSystemSection("recycleBin", true, true), true);
  assert.equal(canAccessRecycleBinPage(false, false), false);
  assert.equal(canAccessRecycleBinPage(false, true), true);
  assert.equal(canAccessRecycleBinPage(true, false), true);
});

test("other system sections still require system settings permission", () => {
  assert.equal(canAccessSystemSection("overview", false, true), false);
  assert.equal(canAccessSystemSection("jobs", false, true), false);
  assert.equal(canAccessSystemSection("overview", true, false), true);
  assert.equal(canAccessSystemSection("jobs", true, false), true);
});
