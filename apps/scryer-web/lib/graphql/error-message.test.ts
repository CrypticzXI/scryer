import test from "node:test";
import assert from "node:assert/strict";

import { classifyStatusToastLevel } from "../utils/status-toast.ts";
import {
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

test("normalizeGraphQlErrorMessage strips nested GraphQL repository validation prefixes", () => {
  assert.equal(
    normalizeGraphQlErrorMessage(
      "[GraphQL] repository: validation: passkeys require a password-backed account",
    ),
    "passkeys require a password-backed account",
  );
});

test("userFacingGraphQlErrorMessage passes plain Error.message through", () => {
  assert.equal(userFacingGraphQlErrorMessage(new Error("queue failed"), "fallback"), "queue failed");
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
