import assert from "node:assert/strict";
import test from "node:test";

import { resolveCatalogSurfacePhase } from "./catalog-bootstrap-policy.ts";

test("configured roots never block loaded catalog content on reachability", () => {
  assert.equal(
    resolveCatalogSurfacePhase({
      canManageLibrarySettings: true,
      hasConfiguredRoots: true,
      loadedTitleCount: 3,
    }),
    "content",
  );
  assert.equal(
    resolveCatalogSurfacePhase({
      canManageLibrarySettings: true,
      hasConfiguredRoots: true,
      loadedTitleCount: 0,
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
    }),
    "rootsMissing",
  );
  assert.equal(
    resolveCatalogSurfacePhase({
      canManageLibrarySettings: false,
      hasConfiguredRoots: false,
      loadedTitleCount: 1,
    }),
    "content",
  );
});
