import assert from "node:assert/strict";
import test from "node:test";

import { resolveCatalogSurfacePhase } from "./catalog-bootstrap-policy.ts";

test("existing catalog content takes precedence over invalid roots", () => {
  assert.equal(
    resolveCatalogSurfacePhase({
      canManageLibrarySettings: true,
      hasConfiguredRoots: true,
      loadedTitleCount: 3,
      rootValidationState: "invalid",
    }),
    "content",
  );
});

test("an empty catalog distinguishes invalid, valid, and unavailable roots", () => {
  assert.equal(
    resolveCatalogSurfacePhase({
      canManageLibrarySettings: true,
      hasConfiguredRoots: true,
      loadedTitleCount: 0,
      rootValidationState: "invalid",
    }),
    "rootsInvalid",
  );
  assert.equal(
    resolveCatalogSurfacePhase({
      canManageLibrarySettings: true,
      hasConfiguredRoots: true,
      loadedTitleCount: 0,
      rootValidationState: "valid",
    }),
    "empty",
  );
  assert.equal(
    resolveCatalogSurfacePhase({
      canManageLibrarySettings: true,
      hasConfiguredRoots: true,
      loadedTitleCount: 0,
      rootValidationState: "unavailable",
    }),
    "empty",
  );
});

test("only a missing configured root blocks catalog bootstrap", () => {
  assert.equal(
    resolveCatalogSurfacePhase({
      canManageLibrarySettings: true,
      hasConfiguredRoots: false,
      loadedTitleCount: null,
      rootValidationState: "notRun",
    }),
    "rootsMissing",
  );
  assert.equal(
    resolveCatalogSurfacePhase({
      canManageLibrarySettings: false,
      hasConfiguredRoots: false,
      loadedTitleCount: 1,
      rootValidationState: "notRun",
    }),
    "content",
  );
});
