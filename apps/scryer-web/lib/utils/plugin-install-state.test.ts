import assert from "node:assert/strict";
import test from "node:test";

import {
  applySuccessfulPluginOperationState,
  claimPluginTerminalOperation,
} from "./plugin-install-state.ts";

test("successful upgrades advance installed state", () => {
  const result = applySuccessfulPluginOperationState(
    {
      installInProgress: true,
      isInstalled: true,
      updateAvailable: true,
      installedVersion: "1.0.0",
      latestVersion: "1.1.0",
      version: "1.1.0",
    },
    "UPGRADE",
  );

  assert.equal(result.installInProgress, false);
  assert.equal(result.updateAvailable, false);
  assert.equal(result.installedVersion, "1.1.0");
});

test("terminal operations can be claimed only once", () => {
  const claimed = new Set<string>();

  assert.equal(claimPluginTerminalOperation(claimed, "plugin-1"), true);
  assert.equal(claimPluginTerminalOperation(claimed, "plugin-1"), false);
});
