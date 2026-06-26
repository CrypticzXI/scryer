import type { ReactNode } from "react";
import { Link } from "react-router-dom";
import {
  ChevronRight,
  FileType,
  Library,
  Route,
  Settings2,
  SlidersHorizontal,
  type LucideIcon,
} from "lucide-react";
import type { ViewId } from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";
import { buildViewPath } from "@/lib/utils/routing";

export type FacetSettingsSectionId =
  | "library"
  | "general"
  | "quality"
  | "renaming"
  | "routing";

const SECTION_ORDER: FacetSettingsSectionId[] = [
  "library",
  "general",
  "quality",
  "renaming",
  "routing",
];

const SECTION_META: Record<
  FacetSettingsSectionId,
  { labelKey: string; icon: LucideIcon }
> = {
  library: { labelKey: "nav.library", icon: Library },
  general: { labelKey: "facetSettings.general", icon: Settings2 },
  quality: { labelKey: "facetSettings.quality", icon: SlidersHorizontal },
  renaming: { labelKey: "facetSettings.renaming", icon: FileType },
  routing: { labelKey: "facetSettings.routing", icon: Route },
};

type FacetSettingsSectionProps = {
  /** The active facet view (movies/series/anime), used to build sub-page links. */
  view: ViewId;
  /** The active facet settings sub-page. */
  section: FacetSettingsSectionId;
  /** The facet label shown in the breadcrumb and subnav header, e.g. "Movies". */
  facetLabel: string;
  /** Whether the user can manage full catalog config (all sub-pages). */
  canManageConfig: boolean;
  /** Whether the user can manage library settings (library sub-page only). */
  canManageLibrarySettings: boolean;
  /** The existing settings form/panel for this section. */
  children: ReactNode;
};

/**
 * Wraps the existing facet settings forms in the same page framing as the
 * global Settings area (Settings > Quality Profiles): an optional left subnav
 * for switching sub-pages, a breadcrumb, an icon-tile page header, and a
 * centered, scrollable content column. The forms themselves are passed through
 * unchanged as `children`.
 */
export function FacetSettingsSection({
  view,
  section,
  facetLabel,
  canManageConfig,
  canManageLibrarySettings,
  children,
}: FacetSettingsSectionProps) {
  const t = useTranslate();
  const meta = SECTION_META[section];
  const sectionLabel = t(meta.labelKey);
  const Icon = meta.icon;

  // Mirror the root sidebar's visibleMediaSettingsSubPages permission rule.
  const availableSections = canManageConfig
    ? SECTION_ORDER
    : canManageLibrarySettings
      ? SECTION_ORDER.filter((id) => id === "library")
      : [];
  const showSubnav = availableSections.length > 1;

  return (
    <div className="flex min-h-0 w-full flex-1 flex-col overflow-hidden bg-transparent md:flex-row">
      {showSubnav ? (
        <aside
          data-slot="facet-settings-subnav"
          className="w-full shrink-0 border-b border-[var(--scry-border3)] bg-[var(--scry-surfF)] p-3 md:h-full md:w-[218px] md:overflow-y-auto md:border-b-0 md:border-r md:p-[22px_14px]"
        >
          <div className="mb-3 flex items-center gap-2 px-2 text-[var(--scry-ink2)] md:mb-4">
            <Settings2 className="h-[18px] w-[18px] text-[var(--scry-accent-text)]" />
            <span className="text-[16px] font-bold">{facetLabel}</span>
          </div>
          <nav className="flex gap-2 overflow-x-auto pb-1 md:flex-col md:overflow-visible md:pb-0">
            {availableSections.map((id) => {
              const ItemIcon = SECTION_META[id].icon;
              const active = section === id;
              return (
                <Link
                  key={id}
                  to={buildViewPath(view, undefined, id)}
                  className={cn(
                    "flex h-9 shrink-0 items-center gap-2 rounded-[9px] px-3 text-[13px] font-medium text-[var(--scry-muted)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] md:w-full",
                    active &&
                      "bg-[linear-gradient(90deg,rgba(var(--scry-accent-rgb),0.26),rgba(var(--scry-accent-rgb),0.08))] text-[var(--scry-ink2)] shadow-[inset_2px_0_0_var(--scry-accent-ring)]",
                  )}
                >
                  <ItemIcon
                    className={cn(
                      "h-[17px] w-[17px] text-[var(--scry-muted2)]",
                      active && "text-[var(--scry-accent-text)]",
                    )}
                  />
                  <span className="whitespace-nowrap">
                    {t(SECTION_META[id].labelKey)}
                  </span>
                </Link>
              );
            })}
          </nav>
        </aside>
      ) : null}
      <div
        data-slot="facet-settings-scroll"
        className="min-h-0 min-w-0 flex-1 overflow-y-auto bg-transparent"
      >
        <div className="mx-auto w-full max-w-[1280px] px-4 py-5 sm:px-6 md:px-[30px] md:py-[26px] md:pb-[60px]">
          <div className="mb-4 flex items-center gap-1.5 text-[12.5px] text-[var(--scry-faint)]">
            <span>{facetLabel}</span>
            <ChevronRight className="h-3.5 w-3.5" />
            <span>{t("nav.settings")}</span>
            <ChevronRight className="h-3.5 w-3.5" />
            <span className="font-semibold text-[var(--scry-accent-text)]">
              {sectionLabel}
            </span>
          </div>
          <div className="mb-6 flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <div className="flex min-w-0 items-start gap-4">
              <div className="flex h-[46px] w-[46px] shrink-0 items-center justify-center rounded-[13px] border border-[var(--scry-baccent)] bg-[linear-gradient(135deg,rgba(var(--scry-accent-rgb),0.35),rgba(123,91,255,0.22))] text-[var(--scry-accent-text)]">
                <Icon className="h-[23px] w-[23px]" />
              </div>
              <div className="min-w-0">
                <h1 className="text-[25px] font-bold tracking-normal text-[var(--scry-ink2)]">
                  {sectionLabel}
                </h1>
                <p className="mt-1 max-w-[640px] text-[13.5px] text-[var(--scry-muted)]">
                  {t("settings.sectionTitle", { section: sectionLabel })}
                </p>
              </div>
            </div>
          </div>
          {children}
        </div>
      </div>
    </div>
  );
}
