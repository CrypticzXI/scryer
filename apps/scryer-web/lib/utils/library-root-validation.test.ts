import assert from "node:assert/strict";
import test from "node:test";

import {
  normalizeComparableLibraryRootPath,
  trimLibraryRootPath,
  validateLibraryRootPaths,
} from "./library-root-validation.ts";

test("library root normalization preserves filesystem roots", () => {
  assert.equal(trimLibraryRootPath(" / "), "/");
  assert.equal(trimLibraryRootPath("C:\\"), "C:\\");
  assert.equal(trimLibraryRootPath("C:/"), "C:/");
  assert.equal(trimLibraryRootPath("\\\\server\\share\\"), "\\\\server\\share\\");
  assert.equal(trimLibraryRootPath("/media/movies/"), "/media/movies");
  assert.equal(normalizeComparableLibraryRootPath("C:\\"), "c:/");
  assert.equal(
    normalizeComparableLibraryRootPath("\\\\SERVER\\Share\\"),
    "//server/share",
  );
});

test("root validation distinguishes invalid paths from unavailable validation", async () => {
  const result = await validateLibraryRootPaths(
    ["/valid", "/missing", "/unreadable", "/unknown"],
    async (path) => {
      if (path === "/missing" || path === "/unreadable") {
        return { graphQLErrors: [{ extensions: { code: "VALIDATION_ERROR" } }] };
      }
      if (path === "/unknown") {
        return { graphQLErrors: [{ extensions: { code: "RATE_LIMITED" } }] };
      }
      return null;
    },
  );

  assert.deepEqual(result.invalidPaths.sort(), ["/missing", "/unreadable"]);
  assert.equal(result.unavailable, true);
});

test("root validation limits concurrent requests", async () => {
  let active = 0;
  let peak = 0;
  await validateLibraryRootPaths(
    Array.from({ length: 12 }, (_, index) => `/path-${index}`),
    async () => {
      active += 1;
      peak = Math.max(peak, active);
      await new Promise((resolve) => setTimeout(resolve, 1));
      active -= 1;
      return null;
    },
    4,
  );
  assert.equal(peak, 4);
});
