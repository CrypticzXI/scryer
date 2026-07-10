import { Check, Loader2, Search, X } from "lucide-react";
import { PluginVisualLabel } from "@/components/common/plugin-visual";
import { Button } from "@/components/ui/button";
import {
  SetupBackButton,
  SetupPanel,
  SetupPrimaryButton,
  SetupStepHeader,
} from "./setup-chrome";
import { Checkbox } from "@/components/ui/checkbox";
import { Input, signedIntegerInputProps } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { selectorId } from "@/lib/utils/dom-ids";
import { type ConfigFieldDef, visibleIndexerConfigFields } from "@/lib/types";

interface ProviderOption {
  value: string;
  label: string;
  defaultBaseUrl?: string;
  configFields: ConfigFieldDef[];
}

interface SetupIndexerViewProps {
  t: (key: string) => string;
  name: string;
  providerType: string;
  configValues: Record<string, string>;
  providerOptions: ProviderOption[];
  onNameChange: (value: string) => void;
  onProviderTypeChange: (value: string) => void;
  onConfigValueChange: (key: string, value: string) => void;
  onTestConnection: () => void;
  onNext: () => void;
  onBack: () => void;
  onSkip?: () => void;
  testing: boolean;
  testResult: "success" | "failed" | null;
  saving: boolean;
  saved: boolean;
  error: string | null;
}

function isMissingRequiredField(
  field: ConfigFieldDef,
  configValues: Record<string, string>,
) {
  if (!field.required || field.valueSource === "HOST_BINDING") {
    return false;
  }

  const value =
    configValues[field.key] ??
    field.defaultValue ??
    (field.fieldType === "BOOL" ? "false" : "");
  return field.fieldType !== "BOOL" && value.trim() === "";
}

function DynamicConfigField({
  t,
  field,
  value,
  onChange,
}: {
  t: SetupIndexerViewProps["t"];
  field: ConfigFieldDef;
  value: string;
  onChange: (key: string, value: string) => void;
}) {
  const fieldId = selectorId("setup-indexer-field", field.key);
  const requiredMarker = field.required ? (
    <span aria-hidden="true" className="text-destructive">
      *
    </span>
  ) : null;

  if (field.fieldType === "BOOL") {
    return (
      <label className="flex items-center gap-2">
        <Checkbox
          id={fieldId}
          checked={value === "true"}
          onCheckedChange={(checkedValue) =>
            onChange(field.key, checkedValue === true ? "true" : "false")
          }
        />
        <span className="inline-flex items-center gap-2 text-sm">
          {field.label}
          {requiredMarker}
        </span>
        {field.helpText ? (
          <span className="text-xs text-muted-foreground">
            {field.helpText}
          </span>
        ) : null}
      </label>
    );
  }

  if (field.fieldType === "SELECT" && field.options.length > 0) {
    return (
      <label className="space-y-2">
        <Label className="inline-flex items-center gap-2" htmlFor={fieldId}>
          {field.label}
          {requiredMarker}
        </Label>
        <Select
          value={value || field.defaultValue || ""}
          onValueChange={(nextValue) => onChange(field.key, nextValue)}
        >
          <SelectTrigger id={fieldId} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {field.options.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {option.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {field.helpText ? (
          <p className="text-xs text-muted-foreground">{field.helpText}</p>
        ) : null}
      </label>
    );
  }

  if (field.fieldType === "MULTILINE") {
    return (
      <label className="space-y-2">
        <Label className="inline-flex items-center gap-2" htmlFor={fieldId}>
          {field.label}
          {requiredMarker}
        </Label>
        <Textarea
          id={fieldId}
          value={value}
          onChange={(event) => onChange(field.key, event.target.value)}
          required={field.required}
          placeholder={field.defaultValue ?? ""}
          rows={5}
        />
        {field.helpText ? (
          <p className="text-xs text-muted-foreground">{field.helpText}</p>
        ) : null}
      </label>
    );
  }

  return (
    <label className="space-y-2">
      <Label className="inline-flex items-center gap-2" htmlFor={fieldId}>
        {field.label}
        {requiredMarker}
      </Label>
      <Input
        id={fieldId}
        value={value}
        onChange={(event) => onChange(field.key, event.target.value)}
        {...(field.fieldType === "NUMBER" ? signedIntegerInputProps : {})}
        type={
          field.fieldType === "PASSWORD"
            ? "password"
            : field.fieldType === "NUMBER"
              ? "number"
              : "text"
        }
        required={field.required}
        placeholder={
          field.fieldType === "PASSWORD"
            ? t("form.apiKeyInputPlaceholder")
            : field.defaultValue ?? ""
        }
      />
      {field.helpText ? (
        <p className="text-xs text-muted-foreground">{field.helpText}</p>
      ) : null}
    </label>
  );
}

export function SetupIndexerView({
  t,
  name,
  providerType,
  configValues,
  providerOptions,
  onNameChange,
  onProviderTypeChange,
  onConfigValueChange,
  onTestConnection,
  onNext,
  onBack,
  onSkip,
  testing,
  testResult,
  saving,
  saved,
  error,
}: SetupIndexerViewProps) {
  const selectedProvider = providerOptions.find((p) => p.value === providerType);
  const selectedProviderFields = visibleIndexerConfigFields(
    providerType,
    (selectedProvider?.configFields ?? []).filter(
      (field) => field.valueSource !== "HOST_BINDING",
    ),
  );
  const hasMissingRequiredField = selectedProviderFields.some((field) =>
    isMissingRequiredField(field, configValues),
  );
  const canTest =
    name.trim().length > 0 && providerType.length > 0 && !hasMissingRequiredField;
  const canProceed = saved;

  return (
    <SetupPanel id="setup-indexer-view" className="flex flex-col gap-6">
      <SetupStepHeader
        icon={Search}
        title={t("setup.indexerTitle")}
        subtitle={t("setup.indexerDescription")}
      />
      <div className="mx-auto flex w-full max-w-md flex-col gap-4">
        <div className="space-y-2">
          <Label htmlFor="setup-indexer-name">{t("label.name")}</Label>
          <Input
            id="setup-indexer-name"
            value={name}
            onChange={(e) => onNameChange(e.target.value)}
            placeholder="My Indexer"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="setup-indexer-provider">{t("settings.indexerProvider")}</Label>
          <Select value={providerType} onValueChange={onProviderTypeChange}>
            <SelectTrigger id="setup-indexer-provider">
              <SelectValue placeholder="Select provider">
                {selectedProvider ? (
                  <PluginVisualLabel
                    providerType={selectedProvider.value}
                    pluginType="indexer"
                    label={selectedProvider.label}
                  />
                ) : null}
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              {providerOptions.map((opt) => (
                <SelectItem key={opt.value} value={opt.value}>
                  <PluginVisualLabel
                    providerType={opt.value}
                    pluginType="indexer"
                    label={opt.label}
                  />
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        {selectedProviderFields
          .filter((field) => field.fieldType !== "BOOL")
          .map((field) => (
            <DynamicConfigField
              key={field.key}
              t={t}
              field={field}
              value={
                configValues[field.key] ??
                field.defaultValue ??
                ""
              }
              onChange={onConfigValueChange}
            />
          ))}
        {selectedProviderFields.some((field) => field.fieldType === "BOOL") ? (
          <div className="flex flex-wrap items-center gap-4">
            {selectedProviderFields
              .filter((field) => field.fieldType === "BOOL")
              .map((field) => (
                <DynamicConfigField
                  key={field.key}
                  t={t}
                  field={field}
                  value={
                    configValues[field.key] ??
                    field.defaultValue ??
                    "false"
                  }
                  onChange={onConfigValueChange}
                />
              ))}
          </div>
        ) : null}
        <div className="flex items-center gap-3">
          <Button
            id="setup-indexer-test-connection"
            variant="outline"
            onClick={onTestConnection}
            disabled={!canTest || testing || saving}
          >
            {testing ? (
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            ) : null}
            {t("label.testConnection")}
          </Button>
          {testResult === "success" && (
            <span
              id="setup-indexer-test-result-success"
              className="flex items-center gap-1 text-sm text-[var(--scry-success-text-soft)]"
            >
              <Check className="h-4 w-4" /> {t("setup.connectionSuccess")}
            </span>
          )}
          {testResult === "failed" && (
            <span
              id="setup-indexer-test-result-failed"
              className="flex items-center gap-1 text-sm text-destructive"
            >
              <X className="h-4 w-4" /> {t("setup.connectionFailed")}
            </span>
          )}
        </div>
        {error && <p id="setup-indexer-error" className="text-sm text-destructive">{error}</p>}
        {saved && (
          <p id="setup-indexer-saved" className="text-sm text-[var(--scry-success-text-soft)]">{t("setup.saved")}</p>
        )}
      </div>
      <div className="flex items-center justify-between pt-2">
        <SetupBackButton id="setup-indexer-back" onClick={onBack}>
          {t("setup.back")}
        </SetupBackButton>
        <div className="flex items-center gap-3">
          {onSkip && (
            <Button id="setup-indexer-skip" type="button" variant="link" onClick={onSkip}>
              {t("setup.skip")}
            </Button>
          )}
          <SetupPrimaryButton id="setup-indexer-next" onClick={onNext} disabled={!canProceed || saving}>
            {saving ? t("label.saving") : t("setup.next")}
          </SetupPrimaryButton>
        </div>
      </div>
    </SetupPanel>
  );
}
