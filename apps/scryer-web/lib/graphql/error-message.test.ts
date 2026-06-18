import test from "node:test";
import assert from "node:assert/strict";

import { classifyStatusToastLevel } from "../utils/status-toast.ts";
import {
  isMfaStepUpRequiredError,
  normalizeGraphQlErrorMessage,
  userFacingGraphQlErrorMessage,
} from "./error-message.ts";

test("normalizeGraphQlErrorMessage strips GraphQL validation prefixes", () => {
  assert.equal(
    normalizeGraphQlErrorMessage(
      "[GraphQL] validation: no download client enabled for library movie_default_library",
    ),
    "no download client enabled for library movie_default_library",
  );
});

test("normalizeGraphQlErrorMessage masks legacy GraphQL repository prefixes", () => {
  assert.equal(
    normalizeGraphQlErrorMessage(
      "[GraphQL] repository: validation: passkeys require a password-backed account",
    ),
    "Internal server error",
  );
});

test("normalizeGraphQlErrorMessage strips plain GraphQL prefixes", () => {
  assert.equal(
    normalizeGraphQlErrorMessage(
      "[GraphQL] enable TOTP for your account before requiring TOTP for system configuration",
    ),
    "enable TOTP for your account before requiring TOTP for system configuration",
  );
});

test("normalizeGraphQlErrorMessage rewrites config step-up errors", () => {
  assert.equal(
    normalizeGraphQlErrorMessage(
      "[GraphQL] MFA verification is required before changing system configuration",
    ),
    "Settings verification expired. Enter an authenticator code to continue.",
  );
});

test("isMfaStepUpRequiredError detects GraphQL extension codes", () => {
  assert.equal(
    isMfaStepUpRequiredError({
      graphQLErrors: [
        {
          message: "MFA verification is required before changing system configuration",
          extensions: { code: "MFA_STEP_UP_REQUIRED" },
        },
      ],
    }),
    true,
  );
});

test("userFacingGraphQlErrorMessage passes plain Error.message through", () => {
  assert.equal(userFacingGraphQlErrorMessage(new Error("queue failed"), "fallback"), "queue failed");
});

test("userFacingGraphQlErrorMessage normalizes prefixed Error.message values", () => {
  assert.equal(
    userFacingGraphQlErrorMessage(
      new Error(
        "[GraphQL] enable TOTP for your account before requiring TOTP for system configuration",
      ),
      "fallback",
    ),
    "enable TOTP for your account before requiring TOTP for system configuration",
  );
});

test("userFacingGraphQlErrorMessage uses fallback when no message exists", () => {
  assert.equal(userFacingGraphQlErrorMessage({ graphQLErrors: [] }, "queue failed"), "queue failed");
});

test("queue validation failure normalizes into global status text that toasts as error", () => {
  const message = userFacingGraphQlErrorMessage(
    {
      graphQLErrors: [
        {
          message:
            "[GraphQL] validation: no download client enabled for library movie_default_library",
        },
      ],
    },
    "queue failed",
  );

  assert.equal(message, "no download client enabled for library movie_default_library");
  assert.equal(classifyStatusToastLevel(message), "error");
});

test("GraphQL internal errors mask repository details and include the reference id", () => {
  const message = userFacingGraphQlErrorMessage(
    {
      graphQLErrors: [
        {
          message:
            "[GraphQL] repository: metadata gateway request failed (502 Bad Gateway): <!DOCTYPE html><html><body>Bad gateway</body></html>",
          extensions: { code: "INTERNAL_ERROR", errorId: "err-123" },
        },
      ],
    },
    "metadata search failed",
  );

  assert.equal(
    message,
    "Internal server error. Reference ID: err-123",
  );
  assert.equal(classifyStatusToastLevel(message), "error");
});
