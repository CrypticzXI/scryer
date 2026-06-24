import { providerConfigRecordToValues } from "@/lib/utils/provider-config";
import type { ConfigFieldDef } from "@/lib/types";

export function setupIndexerConfigFields(fields: ConfigFieldDef[]) {
  return fields.filter((field) => field.valueSource !== "host_binding");
}

export function buildSetupIndexerConfigValues(
  fields: ConfigFieldDef[],
): Record<string, string> {
  const values: Record<string, string> = {};
  for (const field of setupIndexerConfigFields(fields)) {
    values[field.key] =
      field.defaultValue ?? (field.fieldType === "bool" ? "false" : "");
  }
  return values;
}

export function serializeSetupIndexerConfigValues(
  fields: ConfigFieldDef[],
  values: Record<string, string>,
): ReturnType<typeof providerConfigRecordToValues> | undefined {
  const entries: Record<string, string> = {};
  const fieldKeySet = new Set(fields.map((field) => field.key));

  for (const [key, value] of Object.entries(values)) {
    if (!fieldKeySet.has(key) && value.trim() !== "") {
      entries[key] = value;
    }
  }

  for (const field of setupIndexerConfigFields(fields)) {
    let value =
      values[field.key] ??
      field.defaultValue ??
      (field.fieldType === "bool" ? "false" : "");
    if (field.fieldType === "bool") {
      entries[field.key] = value.trim() || field.defaultValue || "false";
      continue;
    }
    if (value.trim() === "" && field.defaultValue) {
      value = field.defaultValue;
    }
    if (value.trim() !== "") {
      entries[field.key] = value;
    }
  }

  const secretInputKeys = setupIndexerConfigFields(fields)
    .filter((field) => field.fieldType === "password")
    .map((field) => field.key);
  return Object.keys(entries).length > 0
    ? providerConfigRecordToValues(entries, secretInputKeys)
    : undefined;
}

export function findMissingSetupIndexerField(
  fields: ConfigFieldDef[],
  values: Record<string, string>,
): ConfigFieldDef | null {
  for (const field of setupIndexerConfigFields(fields)) {
    if (!field.required) {
      continue;
    }
    const value =
      values[field.key] ??
      field.defaultValue ??
      (field.fieldType === "bool" ? "false" : "");
    if (field.fieldType !== "bool" && value.trim() === "") {
      return field;
    }
  }
  return null;
}
