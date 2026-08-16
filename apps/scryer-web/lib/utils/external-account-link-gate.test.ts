import assert from "node:assert/strict";
import test from "node:test";

import {
  canSubmitJellyfinLink,
  effectiveEmbyLinkMode,
} from "./external-account-link-gate.ts";

const ready = {
  connectionId: "jellyfin-main",
  username: "someone",
  busy: false,
};

test("Jellyfin linking requires a connection and username", () => {
  assert.equal(canSubmitJellyfinLink(ready), true);
  assert.equal(canSubmitJellyfinLink({ ...ready, connectionId: "" }), false);
  assert.equal(canSubmitJellyfinLink({ ...ready, connectionId: null }), false);
  assert.equal(canSubmitJellyfinLink({ ...ready, username: "" }), false);
  assert.equal(canSubmitJellyfinLink({ ...ready, username: "   " }), false);
  assert.equal(canSubmitJellyfinLink({ ...ready, busy: true }), false);
});

test("Jellyfin linking does not require a password", () => {
  // Regression guard for scryer-media/scryer#81: a passwordless Jellyfin
  // account could not be linked because the UI demanded a password.
  assert.equal(canSubmitJellyfinLink(ready), true);
});

test("Emby Connect link mode is coerced to local when the selected connection disables Connect", () => {
  assert.equal(effectiveEmbyLinkMode("CONNECT", false), "LOCAL");
  assert.equal(effectiveEmbyLinkMode("CONNECT", true), "CONNECT");
  assert.equal(effectiveEmbyLinkMode("LOCAL", true), "LOCAL");
});
