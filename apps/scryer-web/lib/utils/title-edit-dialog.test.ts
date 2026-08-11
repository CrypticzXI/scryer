import assert from "node:assert/strict";
import test from "node:test";

import {
  DISABLED_TITLE_EDIT_VALUE,
  ENABLED_TITLE_EDIT_VALUE,
  INHERIT_TITLE_EDIT_VALUE,
  UNCHANGED_TITLE_EDIT_VALUE,
  editDialogTargets,
  hasTitleEditChanges,
  initialTitleEditDraft,
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
      useSeasonFolders: false,
      monitorSpecials: true,
      interSeasonMovies: false,
      fillerPolicy: "SKIP_FILLER",
      recapPolicy: "DOWNLOAD_ALL",
    }),
    {
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
