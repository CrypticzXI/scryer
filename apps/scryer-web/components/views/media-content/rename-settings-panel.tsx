import * as React from "react";
import { useTranslate } from "@/lib/context/translate-context";
import type { Translate } from "@/components/root/types";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { cn } from "@/lib/utils";
import type { ViewCategoryId } from "./indexer-category-picker";

const RENAME_COLLISION_POLICY_OPTIONS = [
  { value: "skip", label: "settings.renameCollisionPolicySkip" },
  { value: "error", label: "settings.renameCollisionPolicyError" },
  { value: "replace_if_better", label: "settings.renameCollisionPolicyReplaceIfBetter" },
];

const RENAME_MISSING_METADATA_POLICY_OPTIONS = [
  { value: "fallback_title", label: "settings.renameMissingMetadataPolicyFallbackTitle" },
  { value: "skip", label: "settings.renameMissingMetadataPolicySkip" },
];

const VALID_RENAME_TOKENS = new Set([
  "title", "year", "quality", "edition", "source",
  "video_codec", "audio_codec", "audio_channels", "group", "ext",
  "season", "season_order", "episode", "episode_title", "absolute_episode",
]);
const VALID_FOLDER_TOKENS = new Set(["title", "year"]);

const FOLDER_TOKEN_DESCRIPTIONS: { token: string; labelKey: string }[] = [
  { token: "title", labelKey: "settings.renameTokenTitle" },
  { token: "year", labelKey: "settings.renameTokenYear" },
];

const SHARED_RENAME_TOKEN_DESCRIPTIONS: { token: string; labelKey: string }[] = [
  { token: "title", labelKey: "settings.renameTokenTitle" },
  { token: "quality", labelKey: "settings.renameTokenQuality" },
  { token: "source", labelKey: "settings.renameTokenSource" },
  { token: "video_codec", labelKey: "settings.renameTokenVideoCodec" },
  { token: "audio_codec", labelKey: "settings.renameTokenAudioCodec" },
  { token: "audio_channels", labelKey: "settings.renameTokenAudioChannels" },
  { token: "group", labelKey: "settings.renameTokenGroup" },
  { token: "ext", labelKey: "settings.renameTokenExt" },
];

const MOVIE_RENAME_TOKEN_DESCRIPTIONS: { token: string; labelKey: string }[] = [
  { token: "year", labelKey: "settings.renameTokenYear" },
  { token: "edition", labelKey: "settings.renameTokenEdition" },
];

const SERIES_RENAME_TOKEN_DESCRIPTIONS: { token: string; labelKey: string }[] = [
  { token: "season", labelKey: "settings.renameTokenSeason" },
  { token: "episode", labelKey: "settings.renameTokenEpisode" },
  { token: "episode_title", labelKey: "settings.renameTokenEpisodeTitle" },
];

const ANIME_RENAME_TOKEN_DESCRIPTIONS: { token: string; labelKey: string }[] = [
  { token: "season", labelKey: "settings.renameTokenSeason" },
  { token: "season_order", labelKey: "settings.renameTokenSeasonOrder" },
  { token: "episode", labelKey: "settings.renameTokenEpisode" },
  { token: "absolute_episode", labelKey: "settings.renameTokenAbsoluteEpisode" },
  { token: "episode_title", labelKey: "settings.renameTokenEpisodeTitle" },
];

type TokenDescription = {
  token: string;
  labelKey: string;
};

function getRenameTokenDescriptions(scopeId: ViewCategoryId): { token: string; labelKey: string }[] {
  const scopeSpecific = scopeId === "movie"
    ? MOVIE_RENAME_TOKEN_DESCRIPTIONS
    : scopeId === "anime"
      ? ANIME_RENAME_TOKEN_DESCRIPTIONS
      : SERIES_RENAME_TOKEN_DESCRIPTIONS;
  const shared = scopeId === "series"
    ? SHARED_RENAME_TOKEN_DESCRIPTIONS.filter((token) => token.token !== "group")
    : SHARED_RENAME_TOKEN_DESCRIPTIONS;
  return [...scopeSpecific, ...shared];
}

function validateRenameTemplate(
  template: string,
  t: Translate,
): string | null {
  if (!template.trim()) {
    return t("settings.renameValidationEmpty");
  }

  let i = 0;
  while (i < template.length) {
    if (template[i] === "{") {
      const closeIndex = template.indexOf("}", i + 1);
      if (closeIndex === -1) {
        return t("settings.renameValidationUnmatchedOpen");
      }
      const inner = template.slice(i + 1, closeIndex);
      if (inner.includes("{")) {
        return t("settings.renameValidationUnmatchedOpen");
      }
      const tokenName = inner.includes(":") ? inner.split(":")[0] : inner;
      if (!VALID_RENAME_TOKENS.has(tokenName)) {
        return t("settings.renameValidationUnknownToken", { token: tokenName });
      }
      i = closeIndex + 1;
    } else if (template[i] === "}") {
      return t("settings.renameValidationUnmatchedClose");
    } else {
      i++;
    }
  }

  return null;
}

function validateFolderTemplate(
  template: string,
  t: Translate,
): string | null {
  if (!template.trim()) {
    return t("settings.folderValidationEmpty");
  }

  let i = 0;
  while (i < template.length) {
    if (template[i] === "{") {
      const closeIndex = template.indexOf("}", i + 1);
      if (closeIndex === -1) {
        return t("settings.renameValidationUnmatchedOpen");
      }
      const inner = template.slice(i + 1, closeIndex);
      if (inner.includes("{")) {
        return t("settings.renameValidationUnmatchedOpen");
      }
      const tokenName = inner.includes(":") ? inner.split(":")[0] : inner;
      if (!VALID_FOLDER_TOKENS.has(tokenName)) {
        return t("settings.folderValidationUnknownToken", { token: tokenName });
      }
      i = closeIndex + 1;
    } else if (template[i] === "}") {
      return t("settings.renameValidationUnmatchedClose");
    } else {
      i++;
    }
  }

  return null;
}

const RENAME_PREVIEW_MOVIE_SAMPLE: Record<string, string> = {
  title: "The Dark Knight", year: "2008", quality: "2160p", edition: "IMAX",
  source: "BluRay", video_codec: "x265", audio_codec: "DTS-HD MA",
  audio_channels: "5.1", group: "FraMeSToR", ext: "mkv",
  season: "1", episode: "5", episode_title: "Pilot",
};

const RENAME_PREVIEW_SERIES_SAMPLE: Record<string, string> = {
  title: "Friends", year: "1994", quality: "1080p", edition: "Director's Cut",
  source: "WEB-DL", video_codec: "x264", audio_codec: "AAC",
  audio_channels: "2.0", group: "NTb", ext: "mkv",
  season: "5", episode: "12", episode_title: "The One with the Embryos",
};

const RENAME_PREVIEW_ANIME_SAMPLE: Record<string, string> = {
  title: "Tidebreaker", year: "1999", quality: "1080p", edition: "Director's Cut",
  source: "WEB-DL", video_codec: "x265", audio_codec: "AAC",
  audio_channels: "2.0", group: "SubsPlease", ext: "mkv",
  season: "1", season_order: "1", episode: "1",
  absolute_episode: "1", episode_title: "Romance Dawn",
};

function applyRenameTemplate(template: string, scopeId: ViewCategoryId): string | null {
  if (!template.trim()) return null;
  let result = "";
  let i = 0;
  const sampleValues =
    scopeId === "movie"
      ? RENAME_PREVIEW_MOVIE_SAMPLE
      : scopeId === "anime"
        ? RENAME_PREVIEW_ANIME_SAMPLE
        : RENAME_PREVIEW_SERIES_SAMPLE;
  while (i < template.length) {
    if (template[i] === "{") {
      const closeIndex = template.indexOf("}", i + 1);
      if (closeIndex === -1) return null;
      const inner = template.slice(i + 1, closeIndex);
      if (inner.includes("{")) return null;
      const colonIdx = inner.indexOf(":");
      const tokenName = colonIdx >= 0 ? inner.slice(0, colonIdx) : inner;
      const padWidth = colonIdx >= 0 ? parseInt(inner.slice(colonIdx + 1), 10) : 0;
      if (!VALID_RENAME_TOKENS.has(tokenName)) return null;
      let value = sampleValues[tokenName] ?? tokenName;
      if (padWidth > 0 && /^\d+$/.test(value)) {
        value = value.padStart(padWidth, "0");
      }
      result += value;
      i = closeIndex + 1;
    } else if (template[i] === "}") {
      return null;
    } else {
      result += template[i];
      i++;
    }
  }
  return result;
}

function applyFolderTemplate(template: string, scopeId: ViewCategoryId): string | null {
  if (!template.trim()) return null;
  let result = "";
  let i = 0;
  const sampleValues =
    scopeId === "movie"
      ? RENAME_PREVIEW_MOVIE_SAMPLE
      : scopeId === "anime"
        ? RENAME_PREVIEW_ANIME_SAMPLE
        : RENAME_PREVIEW_SERIES_SAMPLE;
  while (i < template.length) {
    if (template[i] === "{") {
      const closeIndex = template.indexOf("}", i + 1);
      if (closeIndex === -1) return null;
      const inner = template.slice(i + 1, closeIndex);
      if (inner.includes("{")) return null;
      const tokenName = inner.includes(":") ? inner.split(":")[0] : inner;
      if (!VALID_FOLDER_TOKENS.has(tokenName)) return null;
      result += sampleValues[tokenName] ?? tokenName;
      i = closeIndex + 1;
    } else if (template[i] === "}") {
      return null;
    } else {
      result += template[i];
      i++;
    }
  }
  return result.trim() || null;
}

type TemplateSegment = {
  text: string;
  isToken: boolean;
};

function splitTemplateSegments(template: string): TemplateSegment[] {
  if (!template) {
    return [];
  }

  const segments: TemplateSegment[] = [];
  let cursor = 0;

  while (cursor < template.length) {
    if (template[cursor] === "{") {
      const closeIndex = template.indexOf("}", cursor + 1);
      if (closeIndex !== -1) {
        const inner = template.slice(cursor + 1, closeIndex);
        if (!inner.includes("{")) {
          segments.push({
            text: template.slice(cursor, closeIndex + 1),
            isToken: true,
          });
          cursor = closeIndex + 1;
          continue;
        }
      }
    }

    const nextTokenStart = template.indexOf("{", cursor);
    const plainEnd =
      nextTokenStart === -1
        ? template.length
        : nextTokenStart === cursor
          ? cursor + 1
          : nextTokenStart;
    segments.push({
      text: template.slice(cursor, plainEnd),
      isToken: false,
    });
    cursor = plainEnd;
  }

  return segments.filter((segment) => segment.text.length > 0);
}

type HighlightedTemplateInputProps = React.ComponentProps<typeof Input> & {
  value: string;
};

type TemplateTokenContext = {
  key: string;
  query: string;
  replaceStart: number;
  replaceEnd: number;
  shouldCloseBrace: boolean;
};

function updateInputValue(
  input: HTMLInputElement,
  nextValue: string,
  selectionStart: number,
  selectionEnd = selectionStart,
) {
  const nativeInputValueSetter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    "value",
  )?.set;
  if (!nativeInputValueSetter) {
    return;
  }
  nativeInputValueSetter.call(input, nextValue);
  input.dispatchEvent(new Event("input", { bubbles: true }));
  requestAnimationFrame(() => {
    input.setSelectionRange(selectionStart, selectionEnd);
    input.focus();
  });
}

function insertTemplateToken(
  input: HTMLInputElement,
  currentValue: string,
  token: string,
) {
  const insertion = `{${token}}`;
  const start = input.selectionStart ?? currentValue.length;
  const end = input.selectionEnd ?? start;
  const nextValue = currentValue.slice(0, start) + insertion + currentValue.slice(end);
  updateInputValue(input, nextValue, start + insertion.length);
}

function resolveTemplateTokenContext(
  value: string,
  cursor: number,
  tokenDescriptions: TokenDescription[],
): TemplateTokenContext | null {
  const lastOpen = value.lastIndexOf("{", cursor - 1);
  const lastClose = value.lastIndexOf("}", cursor - 1);
  if (lastOpen === -1 || lastOpen < lastClose) {
    return null;
  }

  const tokenBody = value.slice(lastOpen + 1, cursor);
  if (!tokenBody || tokenBody.includes("{") || tokenBody.includes("}")) {
    return null;
  }

  const colonIndex = tokenBody.indexOf(":");
  if (colonIndex !== -1) {
    return null;
  }

  const nextOpen = value.indexOf("{", lastOpen + 1);
  const nextClose = value.indexOf("}", lastOpen + 1);
  const shouldCloseBrace =
    nextClose === -1 || (nextOpen !== -1 && nextOpen < nextClose);
  const query = tokenBody.trim().toLowerCase();

  const matches = tokenDescriptions
    .filter(({ token }) => token.toLowerCase().includes(query))
    .sort((left, right) => {
      const leftToken = left.token.toLowerCase();
      const rightToken = right.token.toLowerCase();
      const leftStarts = leftToken.startsWith(query) ? 0 : 1;
      const rightStarts = rightToken.startsWith(query) ? 0 : 1;
      return leftStarts - rightStarts || leftToken.localeCompare(rightToken);
    });
  if (matches.length === 0) {
    return null;
  }

  return {
    key: `${lastOpen}:${query}`,
    query,
    replaceStart: lastOpen + 1,
    replaceEnd: lastOpen + 1 + tokenBody.length,
    shouldCloseBrace,
  };
}

function applyAutocompleteToken(
  input: HTMLInputElement,
  currentValue: string,
  context: TemplateTokenContext,
  token: string,
) {
  const suffix = context.shouldCloseBrace ? "}" : "";
  const nextValue =
    currentValue.slice(0, context.replaceStart) +
    token +
    suffix +
    currentValue.slice(context.replaceEnd);
  const cursor = context.replaceStart + token.length + suffix.length;
  updateInputValue(input, nextValue, cursor);
}

const HighlightedTemplateInput = React.forwardRef<HTMLInputElement, HighlightedTemplateInputProps>(
  ({ className, value, onScroll, ...props }, ref) => {
    const [scrollLeft, setScrollLeft] = React.useState(0);
    const segments = React.useMemo(() => splitTemplateSegments(value), [value]);
    const showOverlay = value.length > 0;

    return (
      <div className="relative">
        {showOverlay ? (
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-0 z-10 flex items-center overflow-hidden rounded-md px-3 py-1 text-base md:text-sm"
          >
            <div
              className="min-w-full whitespace-pre"
              style={{ transform: `translateX(-${scrollLeft}px)` }}
            >
              {segments.map((segment, index) => (
                <span
                  key={`${index}-${segment.text}`}
                  className={segment.isToken ? "text-emerald-600 dark:text-emerald-400" : "text-foreground"}
                >
                  {segment.text}
                </span>
              ))}
            </div>
          </div>
        ) : null}
        <Input
          {...props}
          ref={ref}
          value={value}
          onScroll={(event) => {
            setScrollLeft(event.currentTarget.scrollLeft);
            onScroll?.(event);
          }}
          className={cn(
            showOverlay && "text-transparent caret-foreground selection:text-transparent",
            className,
          )}
        />
      </div>
    );
  },
);

HighlightedTemplateInput.displayName = "HighlightedTemplateInput";

type TokenAutocompleteInputProps = Omit<HighlightedTemplateInputProps, "ref"> & {
  inputRef: React.RefObject<HTMLInputElement | null>;
  tokenDescriptions: TokenDescription[];
  onAutocompleteToken: (token: string) => void;
  translateLabel: Translate;
};

function TokenAutocompleteInput({
  inputRef,
  value,
  tokenDescriptions,
  onAutocompleteToken,
  translateLabel,
  onBlur,
  onChange,
  onClick,
  onFocus,
  onKeyDown,
  onSelect,
  ...props
}: TokenAutocompleteInputProps) {
  const [isFocused, setIsFocused] = React.useState(false);
  const [cursor, setCursor] = React.useState(0);
  const [highlightedIndex, setHighlightedIndex] = React.useState(0);
  const [dismissedKey, setDismissedKey] = React.useState<string | null>(null);

  const syncCursor = React.useCallback(() => {
    const input = inputRef.current;
    if (!input) {
      return;
    }
    setCursor(input.selectionStart ?? value.length);
  }, [inputRef, value.length]);

  const tokenContext = React.useMemo(
    () =>
      isFocused
        ? resolveTemplateTokenContext(value, cursor, tokenDescriptions)
        : null,
    [cursor, isFocused, tokenDescriptions, value],
  );

  const suggestions = React.useMemo(() => {
    if (!tokenContext || tokenContext.key === dismissedKey) {
      return [];
    }
    return tokenDescriptions
      .filter(({ token }) => token.toLowerCase().includes(tokenContext.query))
      .sort((left, right) => {
        const leftToken = left.token.toLowerCase();
        const rightToken = right.token.toLowerCase();
        const leftStarts = leftToken.startsWith(tokenContext.query) ? 0 : 1;
        const rightStarts = rightToken.startsWith(tokenContext.query) ? 0 : 1;
        return leftStarts - rightStarts || leftToken.localeCompare(rightToken);
      });
  }, [dismissedKey, tokenContext, tokenDescriptions]);

  React.useEffect(() => {
    setHighlightedIndex(0);
  }, [tokenContext?.key]);

  React.useEffect(() => {
    if (tokenContext && tokenContext.key !== dismissedKey) {
      return;
    }
    setDismissedKey(null);
  }, [dismissedKey, tokenContext]);

  return (
    <div className="relative">
      <HighlightedTemplateInput
        {...props}
        ref={inputRef}
        value={value}
        onChange={(event) => {
          onChange?.(event);
          requestAnimationFrame(syncCursor);
        }}
        onFocus={(event) => {
          setIsFocused(true);
          syncCursor();
          onFocus?.(event);
        }}
        onBlur={(event) => {
          setIsFocused(false);
          setDismissedKey(null);
          onBlur?.(event);
        }}
        onClick={(event) => {
          syncCursor();
          onClick?.(event);
        }}
        onSelect={(event) => {
          syncCursor();
          onSelect?.(event);
        }}
        onKeyDown={(event) => {
          if (suggestions.length > 0) {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              setHighlightedIndex((current) => (current + 1) % suggestions.length);
              return;
            }
            if (event.key === "ArrowUp") {
              event.preventDefault();
              setHighlightedIndex((current) => (current - 1 + suggestions.length) % suggestions.length);
              return;
            }
            if (event.key === "Enter" || event.key === "Tab") {
              event.preventDefault();
              onAutocompleteToken(suggestions[highlightedIndex]?.token ?? suggestions[0].token);
              setDismissedKey(null);
              return;
            }
            if (event.key === "Escape") {
              event.preventDefault();
              if (tokenContext) {
                setDismissedKey(tokenContext.key);
              }
              return;
            }
          }
          onKeyDown?.(event);
        }}
      />
      {isFocused && suggestions.length > 0 ? (
        <div className="absolute left-0 right-0 top-[calc(100%+0.375rem)] z-20 rounded-md border border-border/80 bg-popover shadow-lg">
          <div className="max-h-56 overflow-auto p-1">
            {suggestions.map((item, index) => {
              const isActive = index === highlightedIndex;
              return (
                <button
                  key={item.token}
                  type="button"
                  className={cn(
                    "flex w-full items-center justify-between gap-3 rounded-sm px-2 py-1.5 text-left text-sm transition-colors",
                    isActive ? "bg-accent text-accent-foreground" : "text-popover-foreground hover:bg-accent/70",
                  )}
                  onMouseDown={(event) => {
                    event.preventDefault();
                    onAutocompleteToken(item.token);
                    setDismissedKey(null);
                  }}
                >
                  <code className="font-mono text-emerald-600 dark:text-emerald-400">{`{${item.token}}`}</code>
                  <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
                    {translateLabel(item.labelKey)}
                  </span>
                </button>
              );
            })}
          </div>
        </div>
      ) : null}
    </div>
  );
}

export function RenameSettingsPanel({
  activeQualityScopeId,
  mediaSettingsLoading,
  mediaSettingsSaving,
  categoryFolderTemplates,
  handleFolderTemplateChange,
  categoryRenameTemplates,
  handleRenameTemplateChange,
  categoryRenameCollisionPolicies,
  handleRenameCollisionPolicyChange,
  categoryRenameMissingMetadataPolicies,
  handleRenameMissingMetadataPolicyChange,
  updateCategoryMediaProfileSettings,
}: {
  activeQualityScopeId: ViewCategoryId;
  mediaSettingsLoading: boolean;
  mediaSettingsSaving: boolean;
  categoryFolderTemplates: Record<ViewCategoryId, string>;
  handleFolderTemplateChange: (event: React.ChangeEvent<HTMLInputElement>) => void;
  categoryRenameTemplates: Record<ViewCategoryId, string>;
  handleRenameTemplateChange: (event: React.ChangeEvent<HTMLInputElement>) => void;
  categoryRenameCollisionPolicies: Record<ViewCategoryId, string>;
  handleRenameCollisionPolicyChange: (value: string) => void;
  categoryRenameMissingMetadataPolicies: Record<ViewCategoryId, string>;
  handleRenameMissingMetadataPolicyChange: (value: string) => void;
  updateCategoryMediaProfileSettings: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
}) {
  const t = useTranslate();
  const folderTemplateValue = categoryFolderTemplates[activeQualityScopeId];
  const templateValue = categoryRenameTemplates[activeQualityScopeId];
  const folderValidationError = React.useMemo(
    () => validateFolderTemplate(folderTemplateValue, t),
    [folderTemplateValue, t],
  );
  const renameValidationError = React.useMemo(
    () => validateRenameTemplate(templateValue, t),
    [templateValue, t],
  );

  const folderPreview = React.useMemo(
    () => applyFolderTemplate(folderTemplateValue, activeQualityScopeId),
    [activeQualityScopeId, folderTemplateValue],
  );

  const renamePreview = React.useMemo(
    () => applyRenameTemplate(templateValue, activeQualityScopeId),
    [activeQualityScopeId, templateValue],
  );

  const folderInputRef = React.useRef<HTMLInputElement>(null);
  const templateInputRef = React.useRef<HTMLInputElement>(null);
  const renameTokenDescriptions = React.useMemo(
    () => getRenameTokenDescriptions(activeQualityScopeId),
    [activeQualityScopeId],
  );

  const insertFolderToken = React.useCallback(
    (token: string) => {
      const input = folderInputRef.current;
      if (!input) return;
      insertTemplateToken(input, folderTemplateValue, token);
    },
    [folderTemplateValue],
  );

  const insertToken = React.useCallback(
    (token: string) => {
      const input = templateInputRef.current;
      if (!input) return;
      insertTemplateToken(input, templateValue, token);
    },
    [templateValue],
  );

  const autocompleteFolderToken = React.useCallback(
    (token: string) => {
      const input = folderInputRef.current;
      if (!input) {
        return;
      }
      const cursor = input.selectionStart ?? folderTemplateValue.length;
      const context = resolveTemplateTokenContext(folderTemplateValue, cursor, FOLDER_TOKEN_DESCRIPTIONS);
      if (!context) {
        insertTemplateToken(input, folderTemplateValue, token);
        return;
      }
      applyAutocompleteToken(input, folderTemplateValue, context, token);
    },
    [folderTemplateValue],
  );

  const autocompleteRenameToken = React.useCallback(
    (token: string) => {
      const input = templateInputRef.current;
      if (!input) {
        return;
      }
      const cursor = input.selectionStart ?? templateValue.length;
      const context = resolveTemplateTokenContext(templateValue, cursor, renameTokenDescriptions);
      if (!context) {
        insertTemplateToken(input, templateValue, token);
        return;
      }
      applyAutocompleteToken(input, templateValue, context, token);
    },
    [renameTokenDescriptions, templateValue],
  );

  return (
    <form onSubmit={updateCategoryMediaProfileSettings} className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle>{t("settings.renameSection")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <section className="space-y-5 rounded-lg border border-border/70 bg-card/40 p-4">
            <div className="space-y-1">
              <h3 className="text-sm font-semibold text-card-foreground">
                {t("settings.folderRenameSectionTitle")}
              </h3>
            </div>

            <div className="grid gap-4 lg:grid-cols-[2fr_1fr]">
              <div className="space-y-2.5">
                <Label className="text-sm text-card-foreground">
                  {t("settings.folderTemplateLabel")}
                </Label>
                <TokenAutocompleteInput
                  inputRef={folderInputRef}
                  value={folderTemplateValue}
                  onChange={handleFolderTemplateChange}
                  tokenDescriptions={FOLDER_TOKEN_DESCRIPTIONS}
                  onAutocompleteToken={autocompleteFolderToken}
                  translateLabel={t}
                  placeholder={t("settings.folderTemplatePlaceholder")}
                  disabled={mediaSettingsLoading}
                  className={
                    folderTemplateValue.trim()
                      ? folderValidationError
                        ? "border-rose-500/60"
                        : "border-emerald-500/60"
                      : undefined
                  }
                />
                {folderValidationError ? (
                  <p className="text-xs text-rose-400">{folderValidationError}</p>
                ) : null}
              </div>

              <div className="space-y-2">
                <Label className="text-xs uppercase tracking-wider text-muted-foreground/60">
                  Example
                </Label>
                {folderPreview ? (
                  <div className="rounded border border-border bg-muted px-3 py-1.5">
                    <p className="break-all font-mono text-sm text-card-foreground">{folderPreview}</p>
                  </div>
                ) : (
                  <div className="rounded border border-dashed border-border bg-card/40 px-3 py-1.5">
                    <p className="text-sm text-muted-foreground/60">&mdash;</p>
                  </div>
                )}
              </div>
            </div>

            <div className="space-y-2.5">
              <p className="text-sm font-medium text-card-foreground">
                {t("settings.folderAvailableTokens")}
              </p>
              <div className="flex flex-wrap gap-1.5">
                {FOLDER_TOKEN_DESCRIPTIONS.map((item) => (
                  <button
                    key={item.token}
                    type="button"
                    className="inline-flex items-center gap-1 rounded-md border border-border bg-muted px-2.5 py-1 text-xs text-card-foreground transition-colors hover:border-emerald-500 hover:bg-accent hover:text-foreground"
                    title={t(item.labelKey)}
                    onClick={() => insertFolderToken(item.token)}
                  >
                    <code className="text-emerald-600 dark:text-emerald-400">{`{${item.token}}`}</code>
                    <span className="leading-none text-muted-foreground">{t(item.labelKey)}</span>
                  </button>
                ))}
              </div>
            </div>
          </section>

          <section className="space-y-6 rounded-lg border border-border/70 bg-card/40 p-4">
            <div className="space-y-1">
              <h3 className="text-sm font-semibold text-card-foreground">
                {t("settings.renameSectionTitle")}
              </h3>
            </div>

            <div className="grid gap-4 lg:grid-cols-[2fr_1fr]">
              <div className="space-y-2.5">
                <Label className="text-sm text-card-foreground">
                  {t("settings.renameTemplateLabel")}
                </Label>
                <TokenAutocompleteInput
                  inputRef={templateInputRef}
                  value={templateValue}
                  onChange={handleRenameTemplateChange}
                  tokenDescriptions={renameTokenDescriptions}
                  onAutocompleteToken={autocompleteRenameToken}
                  translateLabel={t}
                  placeholder={t("settings.renameTemplatePlaceholder")}
                  disabled={mediaSettingsLoading}
                  className={
                    templateValue.trim()
                      ? renameValidationError
                        ? "border-rose-500/60"
                        : "border-emerald-500/60"
                      : undefined
                  }
                />
                {renameValidationError ? (
                  <p className="text-xs text-rose-400">{renameValidationError}</p>
                ) : null}
              </div>

              <div className="space-y-2">
                <Label className="text-xs uppercase tracking-wider text-muted-foreground/60">
                  Example
                </Label>
                {renamePreview ? (
                  <div className="rounded border border-border bg-muted px-3 py-1.5">
                    <p className="break-all font-mono text-sm text-card-foreground">{renamePreview}</p>
                  </div>
                ) : (
                  <div className="rounded border border-dashed border-border bg-card/40 px-3 py-1.5">
                    <p className="text-sm text-muted-foreground/60">&mdash;</p>
                  </div>
                )}
              </div>
            </div>

            <div className="space-y-2.5">
              <p className="text-sm font-medium text-card-foreground">
                {t("settings.renameAvailableTokens")}
              </p>
              <div className="flex flex-wrap gap-1.5">
                {renameTokenDescriptions.map((item) => (
                  <button
                    key={item.token}
                    type="button"
                    className="inline-flex items-center gap-1 rounded-md border border-border bg-muted px-2.5 py-1 text-xs text-card-foreground transition-colors hover:border-emerald-500 hover:bg-accent hover:text-foreground"
                    title={t(item.labelKey)}
                    onClick={() => insertToken(item.token)}
                  >
                    <code className="text-emerald-600 dark:text-emerald-400">{`{${item.token}}`}</code>
                    <span className="leading-none text-muted-foreground">{t(item.labelKey)}</span>
                  </button>
                ))}
              </div>
            </div>

            <div className="grid gap-4 md:grid-cols-2">
              <label className="space-y-2">
                <Label className="text-sm text-card-foreground">
                  {t("settings.renameCollisionPolicyLabel")}
                </Label>
                <Select value={categoryRenameCollisionPolicies[activeQualityScopeId]} onValueChange={handleRenameCollisionPolicyChange} disabled={mediaSettingsLoading}>
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {RENAME_COLLISION_POLICY_OPTIONS.map((option) => (
                      <SelectItem key={option.value} value={option.value}>{t(option.label)}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
              <label className="space-y-2">
                <Label className="text-sm text-card-foreground">
                  {t("settings.renameMissingMetadataPolicyLabel")}
                </Label>
                <Select value={categoryRenameMissingMetadataPolicies[activeQualityScopeId]} onValueChange={handleRenameMissingMetadataPolicyChange} disabled={mediaSettingsLoading}>
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {RENAME_MISSING_METADATA_POLICY_OPTIONS.map((option) => (
                      <SelectItem key={option.value} value={option.value}>{t(option.label)}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
            </div>
            <p className="text-xs text-muted-foreground">
              {t("settings.renamePolicyHelp")}
            </p>
          </section>

          <div className="flex justify-end">
            <Button
              type="submit"
              disabled={
                mediaSettingsSaving ||
                folderValidationError !== null ||
                renameValidationError !== null
              }
            >
              {mediaSettingsSaving ? t("label.saving") : t("label.save")}
            </Button>
          </div>
        </CardContent>
      </Card>
    </form>
  );
}
