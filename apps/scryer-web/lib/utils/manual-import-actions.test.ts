import assert from "node:assert/strict";
import test from "node:test";

import { manualImportActions } from "./manual-import-actions.ts";

for (const facet of ["MOVIE", "SERIES", "ANIME"] as const) {
  test(`pending ${facet} import exposes no manual action`, () => {
    assert.deepEqual(
      manualImportActions({
        displayState: "IMPORT_PENDING",
        facet,
        hasTitle: true,
      }),
      { direct: false, interactive: false },
    );
  });
}

for (const displayState of ["IMPORT_BLOCKED", "IMPORT_FAILED"] as const) {
  test(`${displayState} series and anime imports use interactive mapping`, () => {
    for (const facet of ["SERIES", "ANIME"] as const) {
      assert.deepEqual(
        manualImportActions({ displayState, facet, hasTitle: true }),
        { direct: false, interactive: true },
      );
    }
  });

  test(`${displayState} movie imports use the direct action`, () => {
    assert.deepEqual(
      manualImportActions({
        displayState,
        facet: "MOVIE",
        hasTitle: true,
      }),
      { direct: true, interactive: false },
    );
  });
}

test("manual import actions require an assigned title", () => {
  assert.deepEqual(
    manualImportActions({
      displayState: "IMPORT_BLOCKED",
      facet: "series",
      hasTitle: false,
    }),
    { direct: false, interactive: false },
  );
});

test("manual import actions tolerate legacy lowercase facet values", () => {
  assert.deepEqual(
    manualImportActions({
      displayState: "IMPORT_BLOCKED",
      facet: "series",
      hasTitle: true,
    }),
    { direct: false, interactive: true },
  );
});
