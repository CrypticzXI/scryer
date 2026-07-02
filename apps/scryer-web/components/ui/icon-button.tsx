import * as React from "react"

import { Button } from "@/components/ui/button"
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
  type BoxedActionButtonTone,
} from "@/lib/utils/action-button-styles"
import { cn } from "@/lib/utils"

type IconButtonProps = Omit<
  React.ComponentProps<typeof Button>,
  "size" | "variant"
> & {
  label: string
  appearance?: "boxed" | "ghost"
  tone?: BoxedActionButtonTone
  showTitleAttribute?: boolean
}

function IconButton({
  label,
  appearance = "boxed",
  tone = "neutral",
  showTitleAttribute = true,
  className,
  title,
  children,
  ...props
}: IconButtonProps) {
  const ariaLabel = props["aria-label"] ?? label

  return (
    <Button
      type="button"
      size="icon-sm"
      variant={appearance === "boxed" ? "secondary" : "ghost"}
      title={showTitleAttribute ? title ?? label : undefined}
      aria-label={ariaLabel}
      className={cn(
        appearance === "boxed"
          ? [boxedActionButtonBaseClass, boxedActionButtonToneClass[tone]]
          : "text-[var(--scry-muted2)] hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)]",
        className,
      )}
      {...props}
    >
      {children}
    </Button>
  )
}

export { IconButton }
export type { IconButtonProps }
