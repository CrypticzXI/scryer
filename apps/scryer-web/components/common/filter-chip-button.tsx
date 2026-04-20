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
      variant={selected ? "default" : "secondary"}
      onClick={onClick}
      title={title}
      className={cn("gap-1.5 text-xs", className)}
    >
      {icon}
      {children}
    </Button>
  );
}
