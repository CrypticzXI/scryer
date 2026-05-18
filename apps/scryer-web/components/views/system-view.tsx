import { startTransition, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { useClient } from "urql";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import "@fontsource-variable/jetbrains-mono";
import { serviceLogsQuery, serviceLogLinesSubscription } from "@/lib/graphql/queries";
import { CODE_FONT } from "@/lib/fonts";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";
import { useIsMobile } from "@/lib/hooks/use-mobile";

type SystemViewState = {
  systemHealth: SystemHealth | null;
  systemLoading: boolean;
  refreshSystem: () => Promise<void>;
};

type IndexerQueryStats = {
  indexerId: string;
  indexerName: string;
  queriesLast24H: number;
  successfulLast24H: number;
  failedLast24H: number;
  lastQueryAt: string | null;
  apiCurrent: number | null;
  apiMax: number | null;
  grabCurrent: number | null;
  grabMax: number | null;
};

type SystemHealth = {
  serviceReady: boolean;
  dbPath: string;
  datastoreEngine: string;
  datastoreMigrationKey: string | null;
  totalTitles: number;
  monitoredTitles: number;
  totalUsers: number;
  titlesMovie: number;
  titlesSeries: number;
  titlesAnime: number;
  titlesOther: number;
  recentEvents: number;
  recentEventPreview: string[];
  dbMigrationVersion: string | null;
  indexerStats: IndexerQueryStats[];
};

type DataSource = {
  nameKey: string;
  href: string;
};

const DATA_SOURCES: DataSource[] = [
  { nameKey: "system.sourceTvdbName", href: "https://www.thetvdb.com/" },
  { nameKey: "system.sourceTmdbName", href: "https://www.themoviedb.org/" },
  { nameKey: "system.sourceMalName", href: "https://myanimelist.net/" },
  { nameKey: "system.sourceAniBridgeName", href: "https://github.com/anibridge/anibridge" },
];

function detectLogLevel(line: string): string {
  const match = String(line ?? "").match(/\b(ERROR|WARN|WARNING|INFO|DEBUG|TRACE)\b/i);
  if (!match) return "info";
  if (match[1].toLowerCase() === "warning") return "warn";
  return match[1].toLowerCase();
}

function quotaBadgeClass(current: number | null, max: number | null): string {
  if (current === null || max === null || max === 0) return "";
  const pct = current / max;
  if (pct >= 1) return "text-red-500 font-semibold";
  if (pct >= 0.9) return "text-red-400";
  if (pct >= 0.75) return "text-yellow-400";
  return "text-green-400";
}

const LOG_LEVEL_COLORS: Record<string, string> = {
  error: "text-red-600 dark:text-red-400",
  warn: "text-yellow-600 dark:text-yellow-400",
  info: "text-blue-600 dark:text-blue-400",
  debug: "text-emerald-600 dark:text-emerald-400",
  trace: "text-zinc-400 dark:text-zinc-500",
};

// Tracing default format: {timestamp} {LEVEL} {target}: {message} {key=value ...}
const TRACING_LINE_RE =
  /^(\d{4}-\d{2}-\d{2}T[\d:.]+Z)\s+(ERROR|WARN|INFO|DEBUG|TRACE)\s+([\w:]+):\s+(.*)/;
const KV_RE = /(\w+)=("(?:[^"\\]|\\.)*"|\S+)/g;

type ParsedLine = {
  timestamp: string;
  level: string;
  target: string;
  message: string;
  kvPairs: { key: string; value: string; start: number; end: number }[];
};

type RawLogLineEntry = {
  id: number;
  raw: string;
  lower: string;
  level: string;
  parsed?: ParsedLine | null;
};

type LogLineEntry = {
  id: number;
  raw: string;
  lower: string;
  level: string;
  parsed: ParsedLine | null;
};

type LogViewerSnapshot = {
  lines: LogLineEntry[];
  bufferedCount: number;
  matchedCount: number;
  liveTailing: boolean;
};

function parseLine(raw: string): ParsedLine | null {
  const m = TRACING_LINE_RE.exec(raw);
  if (!m) return null;

  const body = m[4];
  const kvPairs: ParsedLine["kvPairs"] = [];
  let kv: RegExpExecArray | null;
  KV_RE.lastIndex = 0;
  while ((kv = KV_RE.exec(body)) !== null) {
    kvPairs.push({
      key: kv[1],
      value: kv[2],
      start: kv.index,
      end: kv.index + kv[0].length,
    });
  }

  return { timestamp: m[1], level: m[2], target: m[3], message: body, kvPairs };
}

function buildRawLogLineEntry(id: number, raw: string): RawLogLineEntry {
  return {
    id,
    raw,
    lower: raw.toLowerCase(),
    level: detectLogLevel(raw),
  };
}

function materializeLogLineEntry(entry: RawLogLineEntry): LogLineEntry {
  if (entry.parsed === undefined) {
    entry.parsed = parseLine(entry.raw);
  }

  return {
    id: entry.id,
    raw: entry.raw,
    lower: entry.lower,
    level: entry.level,
    parsed: entry.parsed,
  };
}

function HighlightedLine({ entry }: { entry: LogLineEntry }) {
  const parsed = entry.parsed;
  if (!parsed) {
    return (
      <span className="text-foreground/80" style={{ fontFamily: CODE_FONT }}>
        {entry.raw}
      </span>
    );
  }

  const lvl = parsed.level.toLowerCase();
  const levelColor = LOG_LEVEL_COLORS[lvl] ?? "text-foreground/80";

  const fragments: React.ReactNode[] = [];
  let cursor = 0;
  for (const kv of parsed.kvPairs) {
    if (kv.start > cursor) {
      fragments.push(
        <span key={`t${cursor}`} className="text-foreground/70">
          {parsed.message.slice(cursor, kv.start)}
        </span>,
      );
    }
    fragments.push(
      <span key={`k${kv.start}`}>
        <span className="text-cyan-600 dark:text-cyan-400">{kv.key}</span>
        <span className="text-muted-foreground">=</span>
        <span className="text-foreground/90">{kv.value}</span>
      </span>,
    );
    cursor = kv.end;
  }
  if (cursor < parsed.message.length) {
    fragments.push(
      <span key={`t${cursor}`} className="text-foreground/70">
        {parsed.message.slice(cursor)}
      </span>,
    );
  }

  return (
    <span style={{ fontFamily: CODE_FONT }}>
      <span className="text-muted-foreground/60">{parsed.timestamp}</span>
      {" "}
      <span className={levelColor}>{parsed.level.padStart(5)}</span>
      {" "}
      <span className="text-muted-foreground">{parsed.target}</span>
      <span className="text-muted-foreground/60">:</span>
      {" "}
      {fragments}
    </span>
  );
}

const RAW_BUFFER_MAX = 2000;
const LIVE_TAIL_LINES = 300;
const MAX_RENDERED_LINES = 2000;
const LOG_INGEST_BATCH_MS = 50;
const LOG_RENDER_BATCH_MS = 150;
const EMPTY_LOG_SNAPSHOT: LogViewerSnapshot = {
  lines: [],
  bufferedCount: 0,
  matchedCount: 0,
  liveTailing: false,
};

function buildLogViewerSnapshot(
  source: RawLogLineEntry[],
  query: string,
  level: string,
  paused: boolean,
): LogViewerSnapshot {
  const normalizedQuery = query.trim().toLowerCase();
  const hasFilters = normalizedQuery.length > 0 || level !== "all";

  const matching = source.filter((line) => {
    if (normalizedQuery && !line.lower.includes(normalizedQuery)) {
      return false;
    }
    if (level !== "all" && line.level !== level) {
      return false;
    }
    return true;
  });

  const liveTailing = !paused && !hasFilters && matching.length > LIVE_TAIL_LINES;
  const visible = liveTailing
    ? matching.slice(-LIVE_TAIL_LINES)
    : matching.slice(-MAX_RENDERED_LINES);

  return {
    lines: visible.map(materializeLogLineEntry),
    bufferedCount: source.length,
    matchedCount: matching.length,
    liveTailing,
  };
}

function LogViewer() {
  const client = useClient();
  const isMobile = useIsMobile();
  const [search, setSearch] = useState("");
  const [level, setLevel] = useState("all");
  const [paused, setPaused] = useState(false);
  const [snapshot, setSnapshot] = useState<LogViewerSnapshot>(EMPTY_LOG_SNAPSHOT);
  const [connected, setConnected] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const autoScrollRef = useRef(true);
  const pausedRef = useRef(paused);
  const searchRef = useRef(search);
  const levelRef = useRef(level);
  const nextLineIdRef = useRef(0);
  const rawBufferRef = useRef<RawLogLineEntry[]>([]);
  const pendingLinesRef = useRef<string[]>([]);
  const ingestTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const snapshotTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const commitSnapshot = useCallback(() => {
    const nextSnapshot = buildLogViewerSnapshot(
      rawBufferRef.current,
      searchRef.current,
      levelRef.current,
      pausedRef.current,
    );

    startTransition(() => {
      setSnapshot(nextSnapshot);
    });
  }, []);

  const scheduleSnapshot = useCallback((immediate = false) => {
    if (snapshotTimerRef.current) {
      if (!immediate) {
        return;
      }
      clearTimeout(snapshotTimerRef.current);
      snapshotTimerRef.current = null;
    }

    snapshotTimerRef.current = setTimeout(() => {
      snapshotTimerRef.current = null;
      commitSnapshot();
    }, immediate ? 0 : LOG_RENDER_BATCH_MS);
  }, [commitSnapshot]);

  const flushPendingLines = useCallback(() => {
    ingestTimerRef.current = null;
    if (pendingLinesRef.current.length === 0) {
      return;
    }

    const pending = pendingLinesRef.current.splice(0, pendingLinesRef.current.length);
    const buffer = rawBufferRef.current;

    for (const line of pending) {
      const id = nextLineIdRef.current;
      nextLineIdRef.current += 1;
      buffer.push(buildRawLogLineEntry(id, line));
    }

    if (buffer.length > RAW_BUFFER_MAX) {
      buffer.splice(0, buffer.length - RAW_BUFFER_MAX);
    }

    scheduleSnapshot();
  }, [scheduleSnapshot]);

  const enqueueLine = useCallback((line: string) => {
    pendingLinesRef.current.push(line);
    if (ingestTimerRef.current) {
      return;
    }

    ingestTimerRef.current = setTimeout(flushPendingLines, LOG_INGEST_BATCH_MS);
  }, [flushPendingLines]);

  useEffect(() => {
    pausedRef.current = paused;
    if (paused && ingestTimerRef.current) {
      clearTimeout(ingestTimerRef.current);
      ingestTimerRef.current = null;
      flushPendingLines();
    }
    scheduleSnapshot(true);
  }, [flushPendingLines, paused, scheduleSnapshot]);

  useEffect(() => {
    searchRef.current = search;
    scheduleSnapshot(true);
  }, [scheduleSnapshot, search]);

  useEffect(() => {
    levelRef.current = level;
    scheduleSnapshot(true);
  }, [level, scheduleSnapshot]);

  // Initial load via query
  useEffect(() => {
    client.query(serviceLogsQuery, { limit: RAW_BUFFER_MAX }).toPromise().then(({ data }) => {
      const initial: string[] = Array.isArray(data?.serviceLogs?.lines) ? data.serviceLogs.lines : [];
      rawBufferRef.current = initial.map((line) => {
        const id = nextLineIdRef.current;
        nextLineIdRef.current += 1;
        return buildRawLogLineEntry(id, line);
      });
      scheduleSnapshot(true);
    });
  }, [client, scheduleSnapshot]);

  useDeferredWsSubscription<{ data?: { serviceLogLines?: string } }>({
    requestKey: "serviceLogLines",
    request: { query: serviceLogLinesSubscription },
    onStart() {
      setConnected(true);
    },
    onNext(result) {
      const line = result.data?.serviceLogLines;
      if (line && !pausedRef.current) {
        enqueueLine(line);
      }
    },
    onError(err) {
      console.error("[service-logs] subscription error:", err);
      setConnected(false);
    },
    onComplete() {
      setConnected(false);
    },
  });

  useEffect(
    () => () => {
      if (ingestTimerRef.current) {
        clearTimeout(ingestTimerRef.current);
      }
      if (snapshotTimerRef.current) {
        clearTimeout(snapshotTimerRef.current);
      }
      pendingLinesRef.current = [];
    },
    [],
  );

  // Auto-scroll when new lines arrive
  useEffect(() => {
    if (autoScrollRef.current && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [snapshot.lines]);

  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    autoScrollRef.current = atBottom;
  }, []);

  const liveTailNotice = useMemo(() => {
    if (!snapshot.liveTailing) {
      return null;
    }

    return `Live mode is showing the latest ${snapshot.lines.length} lines from ${snapshot.bufferedCount} buffered entries. Pause or filter to inspect more history.`;
  }, [snapshot.bufferedCount, snapshot.liveTailing, snapshot.lines.length]);

  return (
    <div className="space-y-3">
      <div className="grid gap-3 sm:flex sm:flex-wrap sm:items-end">
        <div className="space-y-1">
          <Label className="text-xs text-muted-foreground">Level</Label>
          <Select value={level} onValueChange={setLevel}>
            <SelectTrigger size="sm" className="w-full sm:w-[100px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All</SelectItem>
              <SelectItem value="error">Error</SelectItem>
              <SelectItem value="warn">Warn</SelectItem>
              <SelectItem value="info">Info</SelectItem>
              <SelectItem value="debug">Debug</SelectItem>
              <SelectItem value="trace">Trace</SelectItem>
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1">
          <Label className="text-xs text-muted-foreground">Search</Label>
          <Input
            type="search"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="filter..."
            className="h-8 w-full text-sm sm:w-48"
          />
        </div>
        <div className="flex flex-col gap-2 sm:flex-row sm:items-end">
          <Button
            size="sm"
            variant="secondary"
            className="w-full sm:w-auto"
            onClick={() => setPaused((p) => !p)}
          >
            {paused ? "Resume" : "Pause"}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            className="w-full sm:w-auto"
            onClick={() => {
              if (ingestTimerRef.current) {
                clearTimeout(ingestTimerRef.current);
                ingestTimerRef.current = null;
              }
              if (snapshotTimerRef.current) {
                clearTimeout(snapshotTimerRef.current);
                snapshotTimerRef.current = null;
              }
              pendingLinesRef.current = [];
              rawBufferRef.current = [];
              startTransition(() => {
                setSnapshot(EMPTY_LOG_SNAPSHOT);
              });
              autoScrollRef.current = true;
            }}
          >
            Clear
          </Button>
        </div>
        <div className="flex items-center gap-1.5 text-xs text-muted-foreground sm:ml-auto">
          <span
            className={`inline-block size-2 rounded-full ${connected ? "bg-green-400" : "bg-red-400"}`}
          />
          {connected ? "Live" : "Disconnected"}
          {paused && <span className="text-yellow-400">(paused)</span>}
        </div>
      </div>
      {liveTailNotice ? (
        <p className="text-xs text-muted-foreground">{liveTailNotice}</p>
      ) : null}
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        data-code-font
        className={`overflow-y-auto rounded-lg border border-border bg-card text-xs leading-5 ${isMobile ? "h-[55vh] min-h-[280px]" : "h-[calc(100vh-320px)] min-h-[400px]"}`}
        style={{ fontFamily: CODE_FONT }}
      >
        {snapshot.lines.length === 0 ? (
          <p className="p-4 text-muted-foreground">No logs available yet.</p>
        ) : (
          <div className="space-y-0.5 p-2">
            {snapshot.lines.map((line, index) => (
              <div
                key={line.id}
                className="flex items-start gap-3 rounded-sm px-1 hover:bg-accent/50"
              >
                <span
                  className="shrink-0 select-none text-right tabular-nums text-muted-foreground/50"
                  style={{ minWidth: "4ch" }}
                >
                  {index + 1}
                </span>
                <div
                  className="min-w-0 flex-1 whitespace-pre-wrap break-all"
                  style={{ fontFamily: CODE_FONT }}
                >
                  <HighlightedLine entry={line} />
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
      <p className="text-xs text-muted-foreground">
        {snapshot.lines.length} shown
        {` · ${snapshot.matchedCount} matching`}
        {` · ${snapshot.bufferedCount} buffered`}
        {snapshot.liveTailing ? " · live tail" : ""}
      </p>
    </div>
  );
}

export function SystemView({
  state,
}: {
  state: SystemViewState;
}) {
  const t = useTranslate();
  const { systemHealth, systemLoading, refreshSystem } = state;

  return (
    <div className="space-y-4">
      {/* Service Health */}
      <Card>
        <CardHeader>
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <CardTitle>{t("system.title")}</CardTitle>
            <Button
              size="sm"
              variant="secondary"
              className="w-full sm:w-auto"
              onClick={() => void refreshSystem()}
              disabled={systemLoading}
            >
              {systemLoading ? t("system.refreshing") : t("label.refresh")}
            </Button>
          </div>
        </CardHeader>
        <CardContent>
          {!systemHealth ? (
            <p className="text-sm text-muted-foreground">{t("system.notLoaded")}</p>
          ) : (
            <div className="space-y-2">
              <p className="text-sm">
                <span className="text-muted-foreground">{t("system.serviceReady")}:</span> {systemHealth.serviceReady ? t("label.yes") : t("label.no")}
              </p>
              <p className="text-sm">
                <span className="text-muted-foreground">{t("system.dbPathLabel")}:</span> <span className="break-all">{systemHealth.dbPath}</span>
              </p>
              <p className="text-sm">
                <span className="text-muted-foreground">Migration:</span>{" "}
                <code className="rounded bg-muted px-1 py-0.5 text-xs">
                  {systemHealth.dbMigrationVersion ?? "unknown"}
                </code>
              </p>
              <p className="text-sm">
                <span className="text-muted-foreground">{t("system.totalTitlesLabel")}:</span> {systemHealth.totalTitles}
              </p>
              <p className="text-sm">
                <span className="text-muted-foreground">{t("system.monitoredTitlesLabel")}:</span> {systemHealth.monitoredTitles}
              </p>
              <p className="text-sm">
                <span className="text-muted-foreground">{t("system.usersLabel")}:</span> {systemHealth.totalUsers}
              </p>
              <p className="text-sm">
                <span className="text-muted-foreground">{t("system.facetLabel")}:</span> movie={systemHealth.titlesMovie}, series={systemHealth.titlesSeries}, anime=
                {systemHealth.titlesAnime}, other={systemHealth.titlesOther}
              </p>
            </div>
          )}
        </CardContent>
      </Card>


      {/* Indexer Stats */}
      {systemHealth && systemHealth.indexerStats.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle className="text-base">Indexer Stats (Last 24h)</CardTitle>
          </CardHeader>
          <CardContent>
            <div className={`grid gap-3 ${systemHealth.indexerStats.length === 1 ? "grid-cols-1" : systemHealth.indexerStats.length === 2 ? "grid-cols-1 sm:grid-cols-2" : "grid-cols-1 sm:grid-cols-2 lg:grid-cols-3"}`}>
              {systemHealth.indexerStats.map((stat) => (
                <div
                  key={stat.indexerId}
                  className="rounded-xl border border-border bg-card p-3 text-sm"
                >
                  <p className="font-medium">{stat.indexerName}</p>
                  <div className="mt-1 space-y-1 text-xs">
                    <p>
                      <span className="text-muted-foreground">Queries:</span>{" "}
                      {stat.queriesLast24H}
                      {stat.failedLast24H > 0 && (
                        <span className="text-red-400"> ({stat.failedLast24H} failed)</span>
                      )}
                    </p>
                    {stat.apiMax !== null && (
                      <p>
                        <span className="text-muted-foreground">API usage:</span>{" "}
                        <span className={quotaBadgeClass(stat.apiCurrent, stat.apiMax)}>
                          {stat.apiCurrent ?? 0}/{stat.apiMax}
                        </span>
                      </p>
                    )}
                    {stat.grabMax !== null && (
                      <p>
                        <span className="text-muted-foreground">Grabs:</span>{" "}
                        <span className={quotaBadgeClass(stat.grabCurrent, stat.grabMax)}>
                          {stat.grabCurrent ?? 0}/{stat.grabMax}
                        </span>
                      </p>
                    )}
                    {stat.lastQueryAt && (
                      <p>
                        <span className="text-muted-foreground">Last query:</span>{" "}
                        {new Date(stat.lastQueryAt).toLocaleString()}
                      </p>
                    )}
                  </div>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      )}

      {/* Data Sources */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base">{t("system.sourcesTitle")}</CardTitle>
        </CardHeader>
        <CardContent>
          <p className="mb-2 text-sm text-muted-foreground">{t("system.sourcesSupport")}</p>
          <div className="grid grid-cols-2 gap-2 text-sm">
            {DATA_SOURCES.map((source) => (
              <div key={source.href} className="rounded-xl border border-border bg-card p-3">
                <a
                  href={source.href}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="font-medium text-primary hover:underline"
                >
                  {t(source.nameKey)}
                </a>
              </div>
            ))}
          </div>
        </CardContent>
      </Card>

      {/* Log Viewer */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Service Logs</CardTitle>
        </CardHeader>
        <CardContent>
          <LogViewer />
        </CardContent>
      </Card>
    </div>
  );
}
