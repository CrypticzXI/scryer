import assert from "node:assert/strict";
import test from "node:test";

import {
  DISABLED_TITLE_EDIT_VALUE,
  ENABLED_TITLE_EDIT_VALUE,
  INHERIT_TITLE_EDIT_VALUE,
  UNCHANGED_TITLE_EDIT_VALUE,
  buildTitleEditChanges,
  editDialogTargets,
  hasTitleEditChanges,
  initialTitleEditDraft,
  titleMatchesOptionUpdates,
} from "./title-edit-dialog.ts";

test("movie edit targets only the title opened from its overview", () => {
  const selectedTitles = ["movie-1", "movie-2"];

  assert.deepEqual(editDialogTargets("movie-3", selectedTitles), ["movie-3"]);
});

test("bulk edit preserves the current multi-title selection", () => {
  const selectedTitles = ["movie-1", "movie-2"];

  assert.deepEqual(editDialogTargets(null, selectedTitles), selectedTitles);
});

test("direct movie edit starts with the movie's linked profile and options", () => {
  assert.deepEqual(
    initialTitleEditDraft({
      qualityProfileId: "profile-uhd",
      rootFolderId: "root-movies",
      monitorType: "MONITORED",
      useSeasonFoldersOverride: false,
      monitorSpecials: true,
      interSeasonMovies: false,
      fillerPolicy: "SKIP_FILLER",
      recapPolicy: "DOWNLOAD_ALL",
    }),
    {
      metadataLanguage: INHERIT_TITLE_EDIT_VALUE,
      qualityProfileId: "profile-uhd",
      rootFolderId: "root-movies",
      monitorType: "MONITORED",
      useSeasonFolders: DISABLED_TITLE_EDIT_VALUE,
      monitorSpecials: ENABLED_TITLE_EDIT_VALUE,
      interSeasonMovies: DISABLED_TITLE_EDIT_VALUE,
      fillerPolicy: "SKIP_FILLER",
      recapPolicy: "DOWNLOAD_ALL",
    },
  );
});

test("direct movie edit displays inherited profile values for empty overrides", () => {
  const draft = initialTitleEditDraft({ qualityProfileId: "" });

  assert.equal(draft.qualityProfileId, INHERIT_TITLE_EDIT_VALUE);
  assert.equal(draft.rootFolderId, UNCHANGED_TITLE_EDIT_VALUE);
});

test("direct movie edit does not submit unchanged displayed values", () => {
  const initialDraft = initialTitleEditDraft({
    qualityProfileId: "profile-hd",
    rootFolderId: "root-movies",
    monitorType: "MONITORED",
  });

  assert.equal(hasTitleEditChanges(initialDraft, initialDraft), false);
  assert.equal(
    hasTitleEditChanges(
      { ...initialDraft, qualityProfileId: "profile-uhd" },
      initialDraft,
    ),
    true,
  );
});

test("title edit sends null only for an explicitly selected inherited profile", () => {
  const initialDraft = initialTitleEditDraft({ qualityProfileId: "profile-hd" });

  assert.deepEqual(buildTitleEditChanges(initialDraft, initialDraft), {});
  assert.deepEqual(
    buildTitleEditChanges(
      { ...initialDraft, qualityProfileId: INHERIT_TITLE_EDIT_VALUE },
      initialDraft,
    ),
    { qualityProfileId: null },
  );
});

test("title edit outcome accepts an explicit profile override returned by reload", () => {
  assert.equal(
    titleMatchesOptionUpdates(
      { qualityProfileId: "profile-hd" },
      { qualityProfileId: "profile-hd" },
    ),
    true,
  );
  assert.equal(
    titleMatchesOptionUpdates(
      { qualityProfileId: "profile-uhd" },
      { qualityProfileId: "profile-hd" },
    ),
    false,
  );
});

test("title edit outcome treats null and empty inherited overrides equivalently", () => {
  assert.equal(
    titleMatchesOptionUpdates(
      { qualityProfileId: null, fillerPolicy: "", recapPolicy: undefined },
      { qualityProfileId: null, fillerPolicy: null, recapPolicy: null },
    ),
    true,
  );
});

test("title edit can clear metadata-language and season-folder overrides", () => {
  const initialDraft = initialTitleEditDraft({
    metadataLanguageOverride: "fra",
    useSeasonFoldersOverride: false,
  });

  assert.deepEqual(
    buildTitleEditChanges(
      {
        ...initialDraft,
        metadataLanguage: INHERIT_TITLE_EDIT_VALUE,
        useSeasonFolders: INHERIT_TITLE_EDIT_VALUE,
      },
      initialDraft,
    ),
    { metadataLanguage: null, useSeasonFolders: null },
  );
});
