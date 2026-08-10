import assert from "node:assert/strict";
import test from "node:test";

import { subscribableProviderNotificationEvents } from "./notification-capabilities.ts";

test("notification event choices are limited to host and provider capabilities", () => {
  assert.deepEqual(
    subscribableProviderNotificationEvents(
      ["grab", "import_complete", "media_request_submitted"],
      ["grab", "health_issue", "test"],
    ),
    ["grab"],
  );
});

test("an empty provider capability list retains legacy support for all host events", () => {
  assert.deepEqual(
    subscribableProviderNotificationEvents(["grab", "import_complete"], []),
    ["grab", "import_complete"],
  );
});
