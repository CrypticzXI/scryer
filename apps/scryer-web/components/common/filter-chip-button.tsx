import type { ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

export function FilterChipButton({
  selected,
  onClick,
  title,
  icon,
  children,
  className,
}: {
  selected: boolean;
  onClick: () => void;
  title?: string;
  icon?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <Button
      type="button"
      size="sm"
      variant={selected ? "default" : "outline"}
      onClick={onClick}
      title={title}
      className={cn(
        "gap-1.5 text-xs font-semibold transition",
        selected
          ? "!border-transparent !bg-[var(--scry-accent-grad)] !text-primary-foreground shadow-[0_8px_18px_rgba(var(--scry-accent-rgb),0.22)]"
          : "!border-[var(--scry-border2)] !bg-[var(--scry-inset)] !text-[var(--scry-muted)] hover:!bg-[var(--scry-hover)] hover:!text-[var(--scry-ink2)]",
        className,
      )}
    >
      {icon}
      {children}
    </Button>
  );
}
