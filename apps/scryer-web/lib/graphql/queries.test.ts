import assert from "node:assert/strict";
import test from "node:test";

import {
  buildReactiveRefreshQuery,
  titleOverviewNativeQuery,
} from "./queries.ts";

test("reactive title overview native refresh omits acquisition diagnostics", () => {
  const result = buildReactiveRefreshQuery([
    {
      key: "titleOverviewNative:title-1:300",
      kind: "titleOverviewNative",
      titleId: "title-1",
      blocklistLimit: 300,
    },
  ]);

  assert.equal(result.query.includes("titleAcquisitionDiagnostics"), false);
  assert.equal(result.query.includes("title(id:"), true);
  assert.equal(result.query.includes("titleHistory("), true);
  assert.equal(result.query.includes("titleReleaseBlocklist("), true);
  assert.equal(result.query.includes("externalSubtitles("), true);
  assert.equal(result.query.includes("setupStatus"), true);
  assert.equal(
    Object.hasOwn(result.actionPlans[0] ?? {}, "titleAcquisitionDiagnosticsAlias"),
    false,
  );
});

test("full title overview native loader still includes acquisition diagnostics", () => {
  assert.equal(titleOverviewNativeQuery.includes("titleAcquisitionDiagnostics"), true);
});
