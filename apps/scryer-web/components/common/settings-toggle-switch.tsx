import { cn } from "@/lib/utils";

type SettingsToggleSwitchProps = {
  id?: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (nextValue: boolean) => void;
  className?: string;
  size?: "default" | "lg";
  ariaLabel?: string;
};

const sizeClasses = {
  default: {
    track: "h-6 w-11 p-0.5",
    thumb: "h-5 w-5",
    checked: "translate-x-5",
    unchecked: "translate-x-0",
  },
  lg: {
    track: "h-8 w-14 p-1",
    thumb: "h-6 w-6",
    checked: "translate-x-6",
    unchecked: "translate-x-0",
  },
} as const;

export function SettingsToggleSwitch({
  id,
  checked,
  disabled,
  onChange,
  className,
  size = "default",
  ariaLabel,
}: SettingsToggleSwitchProps) {
  const classes = sizeClasses[size];

  return (
    <button
      id={id}
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      disabled={disabled}
      className={cn(
        "relative inline-flex shrink-0 items-center rounded-full border border-transparent transition-colors duration-200 outline-none",
        "focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/40",
        disabled ? "cursor-not-allowed opacity-50" : "cursor-pointer",
        checked
          ? "border-emerald-500/40 bg-emerald-500"
          : "border-red-500/40 bg-red-500/14",
        classes.track,
        className,
      )}
      onClick={() => {
        if (!disabled) {
          onChange(!checked);
        }
      }}
    >
      <span
        aria-hidden="true"
        className={cn(
          "pointer-events-none inline-block rounded-full bg-background shadow-sm transition-transform duration-200",
          classes.thumb,
          checked ? classes.checked : classes.unchecked,
        )}
      />
    </button>
  );
}
