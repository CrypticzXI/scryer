import assert from "node:assert/strict";
import test from "node:test";
import {
  removePathMappingRow,
  serializePathMappings,
} from "./path-mappings.ts";

test("removing the final path mapping leaves a blank editor and empty value", () => {
  const rows = removePathMappingRow(
    [{ localPath: "/downloads", remotePath: "/remote" }],
    0,
  );

  assert.deepEqual(rows, [{ localPath: "", remotePath: "" }]);
  assert.equal(serializePathMappings(rows, "local-to-remote"), "");
});
