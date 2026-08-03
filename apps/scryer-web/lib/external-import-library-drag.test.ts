import assert from "node:assert/strict";
import test from "node:test";

import {
  importLibraryDropState,
  shouldEnableNativeImportDrag,
} from "./external-import-library-drag.ts";

test("native library drag is limited to wide fine-pointer layouts", () => {
  assert.equal(shouldEnableNativeImportDrag(false, false), true);
  assert.equal(shouldEnableNativeImportDrag(true, false), false);
  assert.equal(shouldEnableNativeImportDrag(false, true), false);
});

test("library drop feedback distinguishes valid and rejected targets", () => {
  assert.equal(importLibraryDropState(false, true), "idle");
  assert.equal(importLibraryDropState(true, true), "compatible");
  assert.equal(importLibraryDropState(true, false), "incompatible");
});
