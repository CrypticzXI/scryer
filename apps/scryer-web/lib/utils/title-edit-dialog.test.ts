import assert from "node:assert/strict";
import test from "node:test";

import {
  INHERIT_TITLE_EDIT_VALUE,
  buildTitleEditChanges,
  hasTitleEditChanges,
  initialTitleEditDraft,
  titleMatchesOptionUpdates,
} from "./title-edit-dialog.ts";

test("bulk title edit starts with every field unchanged", () => {
  const initialDraft = initialTitleEditDraft();

  assert.equal(hasTitleEditChanges(initialDraft, initialDraft), false);
  assert.equal(
    hasTitleEditChanges(
      { ...initialDraft, qualityProfileId: "profile-uhd" },
      initialDraft,
    ),
    true,
  );
});

test("bulk title edit sends null only for an explicitly selected inherited profile", () => {
  const initialDraft = {
    ...initialTitleEditDraft(),
    qualityProfileId: "profile-hd",
  };

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
