import * as React from "react";
import { ChevronDown } from "lucide-react";

import { Checkbox } from "@/components/ui/checkbox";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import {
  type SelectChrome,
  type SelectSize,
  selectContentClassName,
  selectTriggerClassName,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";

export type MultiSelectOption = {
  value: string;
  label: React.ReactNode;
  disabled?: boolean;
  description?: React.ReactNode;
  id?: string;
};

export type MultiSelectGroup = {
  label?: React.ReactNode;
  options: MultiSelectOption[];
};

export type MultiSelectAllOption = {
  label: React.ReactNode;
  selected: boolean;
  onSelect: () => void;
  id?: string;
};

type MultiSelectOptionListProps = {
  groups: MultiSelectGroup[];
  selectedValues: string[];
  onSelectedValuesChange: (values: string[]) => void;
  allOption?: MultiSelectAllOption;
  disabled?: boolean;
  optionIdPrefix?: string;
  className?: string;
  groupLabelClassName?: string;
  optionClassName?: string;
  optionLabelClassName?: string;
  maxHeightClassName?: string;
};

function flattenOptions(groups: MultiSelectGroup[]) {
  return groups.flatMap((group) => group.options);
}

function MultiSelectOptionList({
  groups,
  selectedValues,
  onSelectedValuesChange,
  allOption,
  disabled = false,
  optionIdPrefix,
  className,
  groupLabelClassName,
  optionClassName,
  optionLabelClassName,
  maxHeightClassName = "max-h-72",
}: MultiSelectOptionListProps) {
  const allOptions = React.useMemo(() => flattenOptions(groups), [groups]);
  const selectedSet = React.useMemo(
    () => new Set(selectedValues),
    [selectedValues],
  );

  const toggleOption = React.useCallback(
    (value: string) => {
      const nextSet = new Set(selectedSet);
      if (nextSet.has(value)) {
        nextSet.delete(value);
      } else {
        nextSet.add(value);
      }

      onSelectedValuesChange(
        allOptions
          .map((option) => option.value)
          .filter((optionValue) => nextSet.has(optionValue)),
      );
    },
    [allOptions, onSelectedValuesChange, selectedSet],
  );

  const renderOption = (option: MultiSelectOption) => {
    const checked = selectedSet.has(option.value);
    return (
      <button
        key={option.value}
        id={option.id ?? (optionIdPrefix ? `${optionIdPrefix}-${option.value}` : undefined)}
        type="button"
        onClick={() => toggleOption(option.value)}
        disabled={disabled || option.disabled}
        className={cn(
          "flex w-full items-center gap-2 rounded-[8px] px-2 py-1.5 text-left text-sm text-[var(--scry-ink2)] transition-colors",
          "hover:bg-[var(--scry-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[rgba(var(--scry-accent-rgb),0.32)]",
          "disabled:cursor-not-allowed disabled:opacity-55 disabled:hover:bg-transparent",
          optionClassName,
        )}
      >
        <Checkbox
          checked={checked}
          disabled={disabled || option.disabled}
          size="compact"
          className="pointer-events-none"
        />
        <span className={cn("min-w-0 flex-1 truncate", optionLabelClassName)}>
          {option.label}
        </span>
      </button>
    );
  };

  return (
    <div className={cn("flex flex-col gap-1 overflow-y-auto", maxHeightClassName, className)}>
      {allOption ? (
        <button
          id={allOption.id}
          type="button"
          onClick={allOption.onSelect}
          disabled={disabled}
          className={cn(
            "flex w-full items-center gap-2 rounded-[8px] px-2 py-1.5 text-left text-sm text-[var(--scry-ink2)] transition-colors",
            "hover:bg-[var(--scry-hover)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[rgba(var(--scry-accent-rgb),0.32)]",
            "disabled:cursor-not-allowed disabled:opacity-55 disabled:hover:bg-transparent",
            optionClassName,
          )}
        >
          <Checkbox
            checked={allOption.selected}
            disabled={disabled}
            size="compact"
            className="pointer-events-none"
          />
          <span className={cn("min-w-0 flex-1 truncate", optionLabelClassName)}>
            {allOption.label}
          </span>
        </button>
      ) : null}

      {groups.map((group, index) => (
        <div key={index} className="flex flex-col gap-1">
          {group.label ? (
            <div
              className={cn(
                "px-2 pt-1 text-xs font-semibold uppercase tracking-wide text-[var(--scry-faint)]",
                groupLabelClassName,
              )}
            >
              {group.label}
            </div>
          ) : null}
          {group.options.map(renderOption)}
        </div>
      ))}
    </div>
  );
}

type MultiSelectDropdownProps = {
  groups?: MultiSelectGroup[];
  options?: MultiSelectOption[];
  selectedValues: string[];
  onSelectedValuesChange: (values: string[]) => void;
  triggerLabel: React.ReactNode;
  placeholder?: React.ReactNode;
  allOption?: MultiSelectAllOption;
  disabled?: boolean;
  id?: string;
  ariaLabel?: string;
  size?: SelectSize;
  chrome?: SelectChrome;
  className?: string;
  triggerClassName?: string;
  contentClassName?: string;
  optionIdPrefix?: string;
  align?: React.ComponentProps<typeof PopoverContent>["align"];
};

function MultiSelectDropdown({
  groups,
  options,
  selectedValues,
  onSelectedValuesChange,
  triggerLabel,
  placeholder,
  allOption,
  disabled = false,
  id,
  ariaLabel,
  size = "default",
  chrome = "form",
  className,
  triggerClassName,
  contentClassName,
  optionIdPrefix,
  align = "start",
}: MultiSelectDropdownProps) {
  const normalizedGroups = React.useMemo<MultiSelectGroup[]>(
    () => groups ?? [{ options: options ?? [] }],
    [groups, options],
  );
  const hasSelection = selectedValues.length > 0 || allOption?.selected;

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          id={id}
          type="button"
          aria-label={ariaLabel}
          disabled={disabled}
          className={selectTriggerClassName({
            size,
            chrome,
            className: cn("w-full", className, triggerClassName),
          })}
        >
          <span
            className={cn(
              "min-w-0 truncate text-left",
              !hasSelection && "text-[var(--scry-faint)]",
            )}
          >
            {hasSelection ? triggerLabel : placeholder ?? triggerLabel}
          </span>
          <ChevronDown className="h-4 w-4 shrink-0 text-[var(--scry-faint)]" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align={align}
        className={cn(
          selectContentClassName("w-[var(--radix-popover-trigger-width)] p-2"),
          contentClassName,
        )}
      >
        <MultiSelectOptionList
          groups={normalizedGroups}
          selectedValues={selectedValues}
          onSelectedValuesChange={onSelectedValuesChange}
          allOption={allOption}
          disabled={disabled}
          optionIdPrefix={optionIdPrefix}
        />
      </PopoverContent>
    </Popover>
  );
}

export { MultiSelectDropdown, MultiSelectOptionList };
