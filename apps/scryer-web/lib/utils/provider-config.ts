export type ProviderConfigFieldValue =
  | { __typename: "StringConfigValuePayload"; stringValue: string }
  | { __typename: "BoolConfigValuePayload"; boolValue: boolean }
  | { __typename: "IntConfigValuePayload"; intValue: number }
  | { __typename: "FloatConfigValuePayload"; floatValue: number }
  | { __typename: "SecretConfigValuePayload"; stored: boolean };

export type ProviderConfigValue = {
  key: string;
  label?: string | null;
  fieldType?: string | null;
  required?: boolean | null;
  defaultValue?: string | null;
  valueSource?: string | null;
  role?: string | null;
  hostBinding?: string | null;
  options?: Array<{ value: string; label: string }> | null;
  helpText?: string | null;
  value?: ProviderConfigFieldValue | null;
};

export type ProviderConfigValueInput = {
  key: string;
  stringValue?: string;
  boolValue?: boolean;
  intValue?: number;
  floatValue?: number;
  secretValue?: string;
  clearSecret?: boolean;
};

export function providerConfigValuesToRecord(
  values: ProviderConfigValue[] | null | undefined,
): Record<string, string> {
  const record: Record<string, string> = {};
  for (const entry of values ?? []) {
    const value = entry.value;
    if (!value) {
      continue;
    }
    switch (value.__typename) {
      case "StringConfigValuePayload":
        record[entry.key] = value.stringValue;
        break;
      case "BoolConfigValuePayload":
        record[entry.key] = value.boolValue ? "true" : "false";
        break;
      case "IntConfigValuePayload":
        record[entry.key] = String(value.intValue);
        break;
      case "FloatConfigValuePayload":
        record[entry.key] = String(value.floatValue);
        break;
      case "SecretConfigValuePayload":
        break;
    }
  }
  return record;
}

export function providerConfigRecordToValues(
  record: Record<string, string> | undefined,
  secretKeys: Iterable<string> = [],
): ProviderConfigValueInput[] {
  const secretKeySet = new Set(secretKeys);
  return Object.entries(record ?? {})
    .filter(([key, value]) => key.trim() !== "" && value !== "")
    .map(([key, value]) =>
      secretKeySet.has(key)
        ? { key, secretValue: value }
        : { key, stringValue: value },
    );
}

/**
 * Materialize descriptor defaults that the form displays without placing in
 * draft state. Explicit blank values remain blank so validation can report a
 * deliberately cleared required field.
 */
export function providerConfigRecordToValuesWithDefaults(
  record: Record<string, string> | undefined,
  fields: ProviderConfigValue[] | null | undefined,
): ProviderConfigValueInput[] {
  const materialized = { ...(record ?? {}) };
  for (const field of fields ?? []) {
    if (
      !(field.key in materialized) &&
      field.defaultValue !== null &&
      field.defaultValue !== undefined
    ) {
      materialized[field.key] = field.defaultValue;
    }
  }
  const secretKeys = (fields ?? [])
    .filter((field) => field.fieldType === "PASSWORD")
    .map((field) => field.key);
  return providerConfigRecordToValues(materialized, secretKeys);
}
