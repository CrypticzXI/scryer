import * as React from "react";

import { cn } from "@/lib/utils";

type AddNewButtonProps = {
  /** Leading icon (e.g. FolderPlus, Plus). */
  icon: React.ComponentType<{ className?: string }>;
  /** Button label. */
  label: React.ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  id?: string;
  type?: "button" | "submit";
  className?: string;
};

/**
 * Dashed, accent-tinted "add a new thing" affordance — a slightly oversized
 * button used to create a new item (root folder, library, channel, etc.).
 * Renders a bespoke button (not the shared Button primitive) so its bright
 * scryer-primary dashed border isn't overridden by a variant border color.
 */
export function AddNewButton({
  icon: Icon,
  label,
  onClick,
  disabled,
  id,
  type = "button",
  className,
}: AddNewButtonProps) {
  return (
    <button
      id={id}
      type={type}
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "inline-flex h-11 items-center justify-center gap-2 whitespace-nowrap rounded-[11px] border-[1.5px] border-dashed border-[var(--scry-accent)]! bg-[rgba(var(--scry-accent-rgb),0.08)] px-4 text-[13px] font-semibold text-[var(--scry-accent)] transition-colors hover:bg-[rgba(var(--scry-accent-rgb),0.16)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-accent-ring)] disabled:pointer-events-none disabled:opacity-50 [&_svg]:shrink-0",
        className,
      )}
    >
      <Icon className="size-[18px]" />
      {label}
    </button>
  );
}
