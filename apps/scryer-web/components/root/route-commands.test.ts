import assert from "node:assert/strict";
import test from "node:test";

import type { AuthUser } from "@/lib/hooks/use-auth";
import {
  APP_PERMISSIONS,
  LIBRARY_PERMISSIONS,
} from "../../lib/utils/permissions.ts";
import { buildRouteCommands } from "./route-commands.ts";

function user(overrides: Partial<AuthUser> = {}): AuthUser {
  return {
    id: "user-id",
    username: "user",
    appPermissions: [],
    libraryPermissions: [],
    ...overrides,
  };
}

const t = (key: string) => key;

test("recycle-bin command is visible to either backend-authorized role", () => {
  const systemUser = user({
    appPermissions: [APP_PERMISSIONS.manageSystemSettings],
  });
  const titleManager = user({
    libraryPermissions: [{
      libraryId: "library-id",
      permissions: [LIBRARY_PERMISSIONS.manageTitles],
    }],
  });
  const ordinaryUser = user();

  for (const authorizedUser of [systemUser, titleManager]) {
    assert.ok(buildRouteCommands({
      t,
      user: authorizedUser,
      onNavigate: () => {},
    }).some((command) => command.id === "system-recycle-bin"));
  }
  assert.equal(buildRouteCommands({
    t,
    user: ordinaryUser,
    onNavigate: () => {},
  }).some((command) => command.id === "system-recycle-bin"), false);
});

test("recycle-bin command targets the canonical system section", () => {
  const calls: unknown[][] = [];
  const command = buildRouteCommands({
    t,
    user: user({
      libraryPermissions: [{
        libraryId: "library-id",
        permissions: [LIBRARY_PERMISSIONS.manageTitles],
      }],
    }),
    onNavigate: (...args) => calls.push(args),
  }).find((candidate) => candidate.id === "system-recycle-bin");

  assert.ok(command);
  command.onSelect();
  assert.equal(calls[0]?.[0], "system");
  assert.equal(calls[0]?.[3], "recycleBin");
});

test("post-processing is grouped with Automation", () => {
  const command = buildRouteCommands({
    t,
    user: user({
      appPermissions: [APP_PERMISSIONS.manageCatalogSettings],
    }),
    onNavigate: () => {},
  }).find((candidate) => candidate.id === "settings-post-processing");

  assert.equal(command?.groupLabel, "nav.group.automation");
});
