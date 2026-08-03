import assert from "node:assert/strict";
import test from "node:test";

import {
  canRetryProwlarrDiscovery,
  continueExternalImportFromConnect,
  isProwlarrDiscoveryReady,
} from "./external-import-wizard-orchestration.ts";

test("an unresolved preview cannot delay Connect navigation", () => {
  const events: string[] = [];
  const neverResolves = new Promise<void>(() => {});

  continueExternalImportFromConnect(
    () => {
      events.push("preview-started");
      return neverResolves;
    },
    () => events.push("navigated"),
  );

  assert.deepEqual(events, ["preview-started", "navigated"]);
});

test("Prowlarr discovery gates Sources only until completion", () => {
  assert.equal(isProwlarrDiscoveryReady(false, null, null), true);
  assert.equal(isProwlarrDiscoveryReady(true, null, null), false);
  assert.equal(isProwlarrDiscoveryReady(true, "session", "RUNNING"), false);
  assert.equal(isProwlarrDiscoveryReady(true, "session", "FAILED"), false);
  assert.equal(isProwlarrDiscoveryReady(true, "session", "COMPLETED"), true);
});

test("failed or canceled Prowlarr discovery exposes retry", () => {
  assert.equal(canRetryProwlarrDiscovery("RUNNING"), false);
  assert.equal(canRetryProwlarrDiscovery("COMPLETED"), false);
  assert.equal(canRetryProwlarrDiscovery("FAILED"), true);
  assert.equal(canRetryProwlarrDiscovery("CANCELED"), true);
});
