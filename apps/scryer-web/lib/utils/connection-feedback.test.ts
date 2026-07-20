import assert from "node:assert/strict";
import test from "node:test";

import {
  isReportedConnectionFeedbackError,
  runConnectionFeedback,
} from "./connection-feedback.ts";

test("connection feedback emits one terminal success message", async () => {
  const statuses: string[] = [];

  await runConnectionFeedback({
    setGlobalStatus: (status) => statuses.push(status),
    startMessage: "Testing",
    successMessage: "Connected",
    failureFallbackMessage: "Failed",
    run: async () => undefined,
  });

  assert.deepEqual(statuses, ["Testing", "Connected"]);
});

test("connection feedback emits one terminal failure message", async () => {
  const statuses: string[] = [];

  await assert.rejects(
    runConnectionFeedback({
      setGlobalStatus: (status) => statuses.push(status),
      successMessage: "Connected",
      failureFallbackMessage: "Failed",
      run: async () => {
        throw new Error("Unavailable");
      },
    }),
    isReportedConnectionFeedbackError,
  );
  assert.deepEqual(statuses, ["Unavailable"]);
});
