import assert from "node:assert/strict";
import test from "node:test";

import {
  canSubmitCatalogAdd,
  catalogAddDraftResetKey,
  catalogAddOptionsForSubmit,
  catalogQualityProfileSelectValue,
  draftForCatalogLibrary,
  inheritedCatalogQualityProfileLabel,
  INHERIT_CATALOG_QUALITY_PROFILE_VALUE,
} from "./catalog-add-quality-profile.ts";

const profiles = [
  { id: "profile-hd", name: "1080p" },
  { id: "profile-uhd", name: "4K" },
];

test("new catalog adds inherit the selected library quality profile by default", () => {
  assert.equal(
    catalogQualityProfileSelectValue(undefined),
    INHERIT_CATALOG_QUALITY_PROFILE_VALUE,
  );
  assert.equal(
    inheritedCatalogQualityProfileLabel(
      { qualityProfileId: "profile-hd" },
      "profile-uhd",
      profiles,
      "Inherit library",
    ),
    "Inherit library — 1080p",
  );
});

test("inherited catalog adds omit qualityProfileId from the mutation payload", () => {
  assert.deepEqual(
    catalogAddOptionsForSubmit({
      libraryId: "library-movies",
      rootFolderId: "root-hd",
      qualityProfileId: undefined,
    }),
    {
      libraryId: "library-movies",
      rootFolderId: "root-hd",
    },
  );
});

test("explicit catalog quality profiles are retained in the mutation payload", () => {
  assert.deepEqual(
    catalogAddOptionsForSubmit({
      libraryId: "library-movies",
      qualityProfileId: " profile-uhd ",
    }),
    {
      libraryId: "library-movies",
      qualityProfileId: "profile-uhd",
    },
  );
});

test("changing libraries resets the root folder without replacing an explicit profile", () => {
  assert.deepEqual(
    draftForCatalogLibrary(
      {
        libraryId: "library-uhd",
        qualityProfileId: "profile-uhd",
        rootFolderId: "root-uhd",
      },
      "library-hd",
      [
        { id: "root-hd-secondary", isDefault: false },
        { id: "root-hd", isDefault: true },
      ],
    ),
    {
      libraryId: "library-hd",
      qualityProfileId: "profile-uhd",
      rootFolderId: "root-hd",
    },
  );
});

test("catalog add reset identity is stable across equivalent config refetches", () => {
  const beforeRefetch = catalogAddDraftResetKey(
    "MOVIE",
    "tvdb-1",
    "library-hd",
    "root-hd",
  );
  const afterRefetch = catalogAddDraftResetKey(
    "MOVIE",
    "tvdb-1",
    "library-hd",
    "root-hd",
  );

  assert.equal(afterRefetch, beforeRefetch);
  assert.notEqual(
    catalogAddDraftResetKey("MOVIE", "tvdb-1", "library-uhd", "root-uhd"),
    beforeRefetch,
  );
});

test("catalog add cannot submit without a loaded quality-profile catalog", () => {
  assert.equal(
    canSubmitCatalogAdd({
      catalogConfigLoading: false,
      qualityProfileCount: 0,
      hasCatalogDestination: true,
      libraryRequired: true,
      hasSelectedLibrary: true,
    }),
    false,
  );
  assert.equal(
    canSubmitCatalogAdd({
      catalogConfigLoading: false,
      qualityProfileCount: 2,
      hasCatalogDestination: true,
      libraryRequired: true,
      hasSelectedLibrary: true,
    }),
    true,
  );
});
