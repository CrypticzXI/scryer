import { Check, X } from "lucide-react";

import { ActionTooltip } from "@/components/ui/tooltip";

type RenderBooleanIconProps = {
  value: boolean;
  label: string;
};

export function RenderBooleanIcon({ value, label }: RenderBooleanIconProps) {
  return (
    <ActionTooltip content={label}>
      <span
        className="inline-flex h-5 w-5 shrink-0 items-center justify-center"
        aria-label={label}
      >
        {value ? (
          <Check className="h-4 w-4 text-[var(--scry-success-text-soft)]" />
        ) : (
          <X className="h-4 w-4 text-[var(--scry-danger-text)]" />
        )}
      </span>
    </ActionTooltip>
  );
}
