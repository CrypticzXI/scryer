import { useTranslate } from "@/lib/context/translate-context";
import type { ConvergenceState, RecencyLane } from "@/lib/types";

// RFC 119 §7.1: the UI shows convergence STATE, not a search cadence. One badge
// shared by the Missing/Upgrades tables and the title overviews.
export function ConvergenceBadge({
  state,
  indexersCovered,
  indexersRouted,
  recencyLane,
}: {
  state: ConvergenceState;
  indexersCovered: number;
  indexersRouted: number;
  recencyLane?: RecencyLane;
}) {
  const t = useTranslate();

  let className: string;
  let label: string;
  let hint: string | undefined;
  switch (state) {
    case "searching":
      className = "bg-[rgba(var(--scry-accent-rgb),0.2)] text-[var(--scry-accent-text)]";
      label = t("wanted.convergence.searching", {
        covered: indexersCovered,
        routed: indexersRouted,
      });
      break;
    case "converged":
      className = "bg-[var(--scry-success-bg-strong)] text-[var(--scry-success-text)]";
      label = t("wanted.convergence.converged");
      hint = t("wanted.convergence.convergedHint");
      break;
    case "deferred":
      className = "bg-[var(--scry-warning-bg-strong)] text-[var(--scry-warning-text)]";
      label = t("wanted.convergence.deferred");
      hint = t("wanted.convergence.deferredHint");
      break;
    default:
      className =
        recencyLane === "hot"
          ? "bg-[var(--scry-info-bg-strong)] text-[var(--scry-info-text)]"
          : "bg-muted text-muted-foreground";
      label =
        recencyLane === "hot"
          ? t("wanted.convergence.queuedHot")
          : t("wanted.convergence.queuedCold");
      break;
  }

  return (
    <span
      className={`inline-block rounded px-2 py-0.5 text-xs font-medium ${className}`}
      title={hint}
      data-ui="wanted-convergence-badge"
      data-convergence-state={state}
    >
      {label}
    </span>
  );
}
