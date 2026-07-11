import assert from "node:assert/strict";
import test from "node:test";

import {
  canAccessRecycleBinPage,
  canAccessSystemSection,
} from "./routes.ts";

test("recycle-bin access follows the broader page-access permission", () => {
  assert.equal(canAccessSystemSection("recycleBin", false, false), false);
  assert.equal(canAccessSystemSection("recycleBin", false, true), true);
  assert.equal(canAccessSystemSection("recycleBin", true, false), true);
  assert.equal(canAccessSystemSection("recycleBin", true, true), true);
  assert.equal(canAccessRecycleBinPage(false, false), false);
  assert.equal(canAccessRecycleBinPage(false, true), true);
  assert.equal(canAccessRecycleBinPage(true, false), true);
});

test("other system sections still require system settings permission", () => {
  assert.equal(canAccessSystemSection("overview", false, true), false);
  assert.equal(canAccessSystemSection("jobs", false, true), false);
  assert.equal(canAccessSystemSection("overview", true, false), true);
  assert.equal(canAccessSystemSection("jobs", true, false), true);
});
