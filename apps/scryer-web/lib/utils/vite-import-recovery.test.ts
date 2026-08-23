import assert from "node:assert/strict";
import test from "node:test";
import {
  shouldRetryStaleViteImport,
  VITE_IMPORT_RECOVERY_WINDOW_MS,
} from "./vite-import-recovery.ts";

const NOW = 1_000_000;
const dynamicImportFailure = new TypeError(
  "Failed to fetch dynamically imported module: http://localhost:3000/src/pages/login.tsx",
);

test("retries a failed dynamic import when no recent recovery was attempted", () => {
  assert.equal(shouldRetryStaleViteImport(dynamicImportFailure, null, NOW), true);
});

test("does not loop when the recovery reload also fails", () => {
  assert.equal(
    shouldRetryStaleViteImport(dynamicImportFailure, NOW, NOW),
    false,
  );
});

test("allows a new recovery attempt after the retry window", () => {
  assert.equal(
    shouldRetryStaleViteImport(
      dynamicImportFailure,
      NOW,
      NOW + VITE_IMPORT_RECOVERY_WINDOW_MS + 1,
    ),
    true,
  );
});

test("does not retry unrelated route errors", () => {
  assert.equal(
    shouldRetryStaleViteImport(new Error("route loader failed"), null, NOW),
    false,
  );
});
