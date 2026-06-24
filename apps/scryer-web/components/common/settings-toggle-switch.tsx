import { Switch } from "@/components/ui/switch";

type SettingsToggleSwitchProps = {
  id?: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (nextValue: boolean) => void;
  className?: string;
  size?: "default" | "lg";
  ariaLabel?: string;
};

/**
 * Thin wrapper over the shared `ui/switch` Radix primitive that preserves the
 * historical `SettingsToggleSwitch` API (`checked`/`onChange`/`ariaLabel`). New
 * code should prefer importing `Switch` from `@/components/ui/switch` directly.
 */
export function SettingsToggleSwitch({
  id,
  checked,
  disabled,
  onChange,
  className,
  size = "default",
  ariaLabel,
}: SettingsToggleSwitchProps) {
  return (
    <Switch
      id={id}
      checked={checked}
      disabled={disabled}
      onCheckedChange={onChange}
      aria-label={ariaLabel}
      size={size}
      className={className}
    />
  );
}
