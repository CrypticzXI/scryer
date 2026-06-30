import * as React from "react";
import { ArrowLeft, type LucideIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/**
 * Accent-gradient treatment for a step's primary forward CTA (Next / Finish /
 * Import / Connect). Spread onto a DS <Button> via `className`. Secondary
 * actions (Back / Skip / Test) keep their default styling.
 */
export const SETUP_PRIMARY_CTA =
  "border-0 text-white [background-image:var(--scry-accent-grad)] shadow-[0_12px_28px_rgba(var(--scry-accent-rgb),0.35)] transition hover:brightness-110";

export function SetupBackButton({
  id,
  onClick,
  disabled,
  className,
  children,
}: {
  id?: string;
  onClick: () => void;
  disabled?: boolean;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <Button
      id={id}
      type="button"
      variant="ghost"
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "gap-2 px-2 text-[var(--scry-muted2)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)]",
        className,
      )}
    >
      <ArrowLeft className="h-4 w-4" />
      {children}
    </Button>
  );
}

/**
 * Glass card that wraps a setup step's content so steps read as surfaces on the
 * gradient rather than floating text. Forwards `id` for e2e hooks.
 */
export function SetupPanel({
  id,
  className,
  children,
}: {
  id?: string;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <div
      id={id}
      className={cn(
        "w-full rounded-[18px] border border-[var(--scry-border2)] bg-[var(--scry-surf)] p-6 shadow-[0_24px_56px_rgba(2,6,23,0.4)] [backdrop-filter:blur(14px)] sm:p-8",
        className,
      )}
    >
      {children}
    </div>
  );
}

/**
 * Centered icon-tile step header, unifying the wizard with the settings header
 * language (accent-tinted tile + title + subtitle).
 */
export function SetupStepHeader({
  icon: Icon,
  title,
  subtitle,
}: {
  icon: LucideIcon;
  title: string;
  subtitle?: string;
}) {
  return (
    <div className="flex flex-col items-center gap-3 text-center">
      <span className="flex h-12 w-12 items-center justify-center rounded-[14px] border border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.12)] text-[var(--scry-accent-text)]">
        <Icon className="h-6 w-6" />
      </span>
      <div className="space-y-1">
        <h2 className="font-[var(--font-space-grotesk)] text-xl font-bold tracking-tight text-[var(--scry-ink2)]">
          {title}
        </h2>
        {subtitle ? (
          <p className="text-sm text-[var(--scry-muted)]">{subtitle}</p>
        ) : null}
      </div>
    </div>
  );
}
