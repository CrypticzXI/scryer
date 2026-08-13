import assert from "node:assert/strict";
import test from "node:test";

import { providerConfigRecordToValuesWithDefaults } from "./provider-config.ts";

test("materializes absent provider defaults while preserving explicit blanks", () => {
  const fields = [
    { key: "security", fieldType: "SELECT", defaultValue: "starttls" },
    { key: "password", fieldType: "PASSWORD", defaultValue: null },
  ];

  assert.deepEqual(providerConfigRecordToValuesWithDefaults({}, fields), [
    { key: "security", stringValue: "starttls" },
  ]);
  assert.deepEqual(
    providerConfigRecordToValuesWithDefaults({ security: "" }, fields),
    [],
  );
});
