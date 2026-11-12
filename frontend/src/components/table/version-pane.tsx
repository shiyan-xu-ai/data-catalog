import {
  CircleDot,
  Columns3,
  Combine,
  Copy,
  FilePlus2,
  ListTree,
  type LucideIcon,
  PenLine,
  Replace,
  RotateCcw,
  Settings2,
  TableProperties,
  Trash2,
} from "lucide-react";
import type { UseQueryResult } from "@tanstack/react-query";
import { useTimelineEvent, useVersionFiles } from "@/lib/catalog-queries";
import { formatBytes, formatCount, formatIsoTimestamp } from "@/lib/format";
import { useTimeZonePref } from "@/lib/preferences";
import type { TimelineEvent } from "@/lib/schemas";
import {
  asRecord,
  asString,
  asStrings,
  operationFamily,
  operationLabel,
  type OperationFamily,
  type SummaryDelta,
} from "@/lib/timeline";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { CopyButton } from "@/components/copy-button";
import { ErrorState } from "@/components/error-state";
import { JsonTree } from "@/components/json-tree";
import { Fact, Mono, PaneSection, PaneTabs, SidePane } from "@/components/side-pane";

export type PaneView = "details" | "source";

const SUMMARY_FIELDS: [key: string, label: string, bytes?: boolean][] = [
  ["total_rows", "Rows"],
  ["total_files_size", "Data size", true],
  ["total_fragments", "Fragments"],
  ["total_data_files", "Data files"],
  ["total_deletion_files", "Deletion files"],
  ["total_deletion_file_rows", "Deleted rows"],
];

/** The manifest summary totals of one version. */
function SummaryTotals({ summary }: { summary: Record<string, string> }) {
  return (
    <dl className="grid grid-cols-2 gap-x-4 gap-y-1.5">
      {SUMMARY_FIELDS.map(([key, label, bytes]) => {
        const raw = summary[key];
        const n = raw == null || raw === "" ? null : Number(raw);
        return (
          <div key={key} className="min-w-0">
            <dt className="text-xs text-muted-foreground">{label}</dt>
            <dd className="text-sm tabular-nums">{bytes ? formatBytes(n) : formatCount(n)}</dd>
          </div>
        );
      })}
    </dl>
  );
}

/** The files that record `version`, each protobuf section as a JSON tree. */
function VersionSource({ segments, version }: { segments: readonly string[]; version: number }) {
  const files = useVersionFiles(segments, version);
  if (files.error) {
    return (
      <div className="p-4">
        <ErrorState message={files.error.message} />
      </div>
    );
  }
  if (files.isPending) {
    return (
      <div className="space-y-2 p-4">
        <Skeleton className="h-4 w-2/3" />
        <Skeleton className="h-40 w-full" />
      </div>
    );
  }
  return (
    <>
      <p className="px-4 pb-3 text-xs text-muted-foreground">
        Protobuf decoded to JSON. Byte fields over 64 bytes show only their length.
      </p>
      {files.data.files.map((f) => (
        <section key={f.path} className="space-y-2 border-t px-4 py-3">
          <div className="flex items-center gap-1">
            <code className="min-w-0 font-mono text-xs break-all">{f.path}</code>
            <CopyButton value={f.path} label="Copy path" />
            <span className="ml-auto shrink-0 text-xs text-muted-foreground tabular-nums">{formatBytes(f.size_bytes)}</span>
          </div>
          {f.sections.map((s) => (
            <div key={`${s.name}@${s.offset}`} className="rounded-md border">
              <div className="flex items-center gap-2 border-b bg-muted/50 py-1 pr-1 pl-2 text-xs">
                <span className="font-medium">{s.name}</span>
                {s.message && <span className="truncate font-mono text-muted-foreground">{s.message}</span>}
                <span className="ml-auto shrink-0 text-muted-foreground tabular-nums">@ {formatCount(s.offset)}</span>
                {s.content !== undefined && (
                  <CopyButton value={() => JSON.stringify(s.content, null, 2)} label="Copy JSON" />
                )}
              </div>
              <div className="overflow-x-auto p-2">
                {s.error ? (
                  <span className="text-xs text-destructive">{s.error}</span>
                ) : (
                  <JsonTree name={s.message?.split(".").pop() ?? s.name} value={s.content} />
                )}
              </div>
            </div>
          ))}
        </section>
      ))}
    </>
  );
}

const VIEWS = [
  ["details", "Details"],
  ["source", "Source files"],
] as const;

export const ICON: Record<string, LucideIcon> = {
  Append: FilePlus2,
  Overwrite: Replace,
  Merge: Columns3,
  DataReplacement: Replace,
  DataOverlay: Replace,
  Delete: Trash2,
  Update: PenLine,
  Rewrite: Combine,
  CreateIndex: ListTree,
  UpdateConfig: Settings2,
  Project: TableProperties,
  Restore: RotateCcw,
  Clone: Copy,
};

// Node colors per family: ring/background for the node, text for the icon and filter dot.
export const FAMILY_STYLE: Record<OperationFamily, { node: string; dot: string }> = {
  write: {
    node: "border-emerald-500/40 bg-emerald-50 text-emerald-600 dark:bg-emerald-950 dark:text-emerald-400",
    dot: "bg-emerald-500",
  },
  remove: { node: "border-rose-500/40 bg-rose-50 text-rose-600 dark:bg-rose-950 dark:text-rose-400", dot: "bg-rose-500" },
  update: {
    node: "border-amber-500/40 bg-amber-50 text-amber-600 dark:bg-amber-950 dark:text-amber-400",
    dot: "bg-amber-500",
  },
  maintenance: { node: "border-sky-500/40 bg-sky-50 text-sky-600 dark:bg-sky-950 dark:text-sky-400", dot: "bg-sky-500" },
  metadata: {
    node: "border-violet-500/40 bg-violet-50 text-violet-600 dark:bg-violet-950 dark:text-violet-400",
    dot: "bg-violet-500",
  },
  restore: {
    node: "border-orange-500/40 bg-orange-50 text-orange-600 dark:bg-orange-950 dark:text-orange-400",
    dot: "bg-orange-500",
  },
  other: { node: "border-border bg-muted text-muted-foreground", dot: "bg-muted-foreground" },
};

/** Green for growth, red for shrinkage; `inverted` flips that for counts where growth is a
 *  removal (deleted rows). */
function DeltaChip({
  label,
  value,
  format,
  inverted = false,
}: {
  label: string;
  value: number | null;
  format: (n: number) => string;
  inverted?: boolean;
}) {
  if (value == null || value === 0) return null;
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-md border px-1.5 py-0.5 text-xs tabular-nums",
        value > 0 !== inverted
          ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-400"
          : "border-rose-500/30 bg-rose-500/10 text-rose-700 dark:text-rose-400",
      )}
    >
      {value > 0 ? "+" : "−"}
      {format(Math.abs(value))}
      <span className="opacity-70">{label}</span>
    </span>
  );
}

function Deltas({ delta }: { delta: SummaryDelta | null }) {
  if (!delta) {
    return (
      <span className="text-xs text-muted-foreground" title="Load earlier versions to compare">
        —
      </span>
    );
  }
  const chips = [
    <DeltaChip key="r" label="rows" value={delta.rows} format={formatCount} />,
    <DeltaChip key="b" label="data" value={delta.bytes} format={formatBytes} />,
    <DeltaChip key="f" label="fragments" value={delta.fragments} format={formatCount} />,
    <DeltaChip key="d" label="deleted rows" value={delta.deletedRows} format={formatCount} inverted />,
  ];
  if (!Object.values(delta).some((v) => v != null && v !== 0)) {
    return <span className="text-xs text-muted-foreground">No data change</span>;
  }
  return <div className="flex flex-wrap gap-1.5">{chips}</div>;
}

function NameList({ names }: { names: string[] }) {
  return (
    <span className="inline-flex flex-wrap gap-1">
      {names.map((n) => (
        <Mono key={n}>{n}</Mono>
      ))}
    </span>
  );
}

function CountFact({ label, value }: { label: string; value: unknown }) {
  const n = typeof value === "number" ? value : null;
  if (n == null) return null;
  return (
    <Fact label={label}>
      <span className="tabular-nums">{formatCount(n)}</span>
    </Fact>
  );
}

/** Keys set and removed in one metadata map. `set` is a map, or a key list for schema metadata. */
function MapChange({ label, change }: { label: string; change: Record<string, unknown> | null }) {
  if (!change) return null;
  const setMap = asRecord(change.set);
  const setKeys = setMap ? Object.keys(setMap) : asStrings(change.set);
  const removed = asStrings(change.removed);
  if (setKeys.length === 0 && removed.length === 0) return null;
  return (
    <Fact label={label}>
      <span className="flex flex-col gap-1">
        {setKeys.map((k) => (
          <span key={`s-${k}`} className="flex min-w-0 flex-wrap items-baseline gap-1">
            <span className="text-xs text-emerald-600 dark:text-emerald-400">set</span>
            <Mono>{k}</Mono>
            {setMap && (
              <>
                <span className="text-muted-foreground">=</span>
                <Mono>{asString(setMap[k]) ?? JSON.stringify(setMap[k])}</Mono>
              </>
            )}
          </span>
        ))}
        {removed.map((k) => (
          <span key={`r-${k}`} className="flex items-baseline gap-1">
            <span className="text-xs text-rose-600 dark:text-rose-400">removed</span>
            <Mono>{k}</Mono>
          </span>
        ))}
      </span>
    </Fact>
  );
}

function Details({ op, d }: { op: string; d: Record<string, unknown> }) {
  const facts: React.ReactNode[] = [];
  switch (op) {
    case "Append":
      facts.push(
        <CountFact key="fa" label="Fragments added" value={d.fragments_added} />,
        <CountFact key="ra" label="Rows added" value={d.rows_added} />,
        <CountFact key="fi" label="Files added" value={d.files_added} />,
      );
      break;
    case "Delete":
      if (asString(d.predicate)) {
        facts.push(
          <Fact key="p" label="Predicate">
            <Mono>{asString(d.predicate)}</Mono>
          </Fact>,
        );
      }
      facts.push(
        <CountFact key="fu" label="Fragments updated" value={d.fragments_updated} />,
        <CountFact key="fr" label="Fragments removed" value={d.fragments_removed} />,
      );
      break;
    case "Update": {
      facts.push(
        <CountFact key="fu" label="Fragments updated" value={d.fragments_updated} />,
        <CountFact key="fr" label="Fragments removed" value={d.fragments_removed} />,
        <CountFact key="fa" label="Fragments added" value={d.fragments_added} />,
      );
      const fields = asStrings(d.fields_modified);
      if (fields.length) {
        facts.push(
          <Fact key="fm" label="Columns modified">
            <NameList names={fields} />
          </Fact>,
        );
      }
      break;
    }
    case "Overwrite":
      facts.push(
        <CountFact key="f" label="Fragments" value={d.fragments} />,
        <CountFact key="r" label="Rows" value={d.rows} />,
        <CountFact key="c" label="Columns" value={d.fields} />,
      );
      break;
    case "CreateIndex": {
      const added = Array.isArray(d.added) ? d.added.map(asRecord).filter((x) => x != null) : [];
      const removed = asStrings(d.removed);
      for (const [i, ix] of added.entries()) {
        const cols = asStrings(ix.columns);
        facts.push(
          <Fact key={`a${i}`} label="Index">
            <span className="inline-flex flex-wrap items-baseline gap-1.5">
              <Mono>{asString(ix.name) ?? "unnamed"}</Mono>
              {asString(ix.type) && <Badge variant="outline">{asString(ix.type)}</Badge>}
              {cols.length > 0 && (
                <>
                  <span className="text-xs text-muted-foreground">on</span>
                  <NameList names={cols} />
                </>
              )}
            </span>
          </Fact>,
        );
      }
      if (removed.length) {
        facts.push(
          <Fact key="rm" label="Replaced">
            <NameList names={removed} />
          </Fact>,
        );
      }
      break;
    }
    case "Rewrite":
      facts.push(
        <CountFact key="g" label="Groups" value={d.groups} />,
        <CountFact key="b" label="Fragments before" value={d.fragments_before} />,
        <CountFact key="a" label="Fragments after" value={d.fragments_after} />,
        <CountFact key="i" label="Indices rewritten" value={d.rewritten_indices} />,
      );
      break;
    case "Merge":
      facts.push(
        <CountFact key="f" label="Fragments" value={d.fragments} />,
        <CountFact key="c" label="Columns" value={d.fields} />,
      );
      break;
    case "Project": {
      const fields = asStrings(d.fields);
      if (fields.length) {
        facts.push(
          <Fact key="f" label="Columns">
            <NameList names={fields} />
          </Fact>,
        );
      }
      break;
    }
    case "UpdateConfig": {
      facts.push(
        <MapChange key="c" label="Config" change={asRecord(d.config)} />,
        <MapChange key="t" label="Table metadata" change={asRecord(d.table_metadata)} />,
        <MapChange key="s" label="Schema metadata" change={asRecord(d.schema_metadata)} />,
      );
      const fields = asStrings(asRecord(d.field_metadata)?.fields);
      if (fields.length) {
        facts.push(
          <Fact key="fm" label="Field metadata">
            <NameList names={fields} />
          </Fact>,
        );
      }
      break;
    }
    case "Restore":
      if (typeof d.version === "number") {
        facts.push(
          <Fact key="v" label="Restored to">
            <Badge variant="outline">v{d.version}</Badge>
          </Fact>,
        );
      }
      break;
    case "DataReplacement":
      facts.push(<CountFact key="r" label="Replacements" value={d.replacements} />);
      break;
  }
  if (facts.length === 0) return <span className="text-sm text-muted-foreground">None recorded</span>;
  return <div className="space-y-1.5">{facts}</div>;
}

function EventHeader({ version, event }: { version: number; event: TimelineEvent | null }) {
  const Icon = (event && ICON[event.operation]) ?? CircleDot;
  const style = FAMILY_STYLE[operationFamily(event?.operation ?? "")];
  return (
    <div className="flex items-start gap-3">
      <span className={cn("flex h-8 w-8 shrink-0 items-center justify-center rounded-full border", style.node)}>
        <Icon className="h-4 w-4" />
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          {event ? <span className="font-medium">{operationLabel(event.operation)}</span> : <Skeleton className="h-4 w-24" />}
          <Badge variant="outline" className="font-mono">
            v{version}
          </Badge>
        </div>
        {event && <div className="font-mono text-xs text-muted-foreground">{event.operation}</div>}
      </div>
    </div>
  );
}

function EventDetails({ event, delta }: { event: TimelineEvent; delta: SummaryDelta | null }) {
  const zone = useTimeZonePref();
  const date = event.timestamp_millis == null ? null : new Date(event.timestamp_millis);
  const props = Object.entries(event.transaction?.properties ?? {});
  return (
    <>
      <PaneSection title="Commit">
        <Fact label="Time">
          <span className="font-mono text-xs tabular-nums">{date ? formatIsoTimestamp(date, zone) : "Unknown"}</span>
        </Fact>
        <Fact label="Version">
          <span className="font-mono text-xs">v{event.version}</span>
        </Fact>
        {event.transaction && (
          <Fact label="Read version">
            <span className="font-mono text-xs">v{event.transaction.read_version}</span>
          </Fact>
        )}
        {event.tags.length > 0 && (
          <Fact label="Tags">
            <span className="inline-flex flex-wrap gap-1">
              {event.tags.map((t) => (
                <Badge key={t} className="font-mono">
                  {t}
                </Badge>
              ))}
            </span>
          </Fact>
        )}
      </PaneSection>
      <PaneSection title="Change from previous loaded version">
        <Deltas delta={delta} />
      </PaneSection>
      <PaneSection title="Operation details">
        <Details op={event.operation} d={event.details} />
      </PaneSection>
      <PaneSection title="Totals at this version">
        <SummaryTotals summary={event.summary} />
      </PaneSection>
      <PaneSection title="Writer and transaction">
        <Fact label="Writer">
          {event.writer ? `${event.writer.library} ${event.writer.version}` : <span className="text-muted-foreground">Unknown</span>}
        </Fact>
        {event.transaction ? (
          <>
            <Fact label="Transaction">
              <span className="inline-flex items-center gap-1">
                <Mono>{event.transaction.uuid}</Mono>
                <CopyButton value={event.transaction.uuid} label="Copy transaction id" />
              </span>
            </Fact>
            {props.length > 0 && (
              <Fact label="Properties">
                <span className="flex flex-col gap-1">
                  {props.map(([k, v]) => (
                    <span key={k} className="flex min-w-0 flex-wrap items-baseline gap-1">
                      <Mono>{k}</Mono>
                      <span className="text-muted-foreground">=</span>
                      <Mono>{v}</Mono>
                    </span>
                  ))}
                </span>
              </Fact>
            )}
          </>
        ) : (
          <Fact label="Transaction">
            <span className="text-muted-foreground">Not recorded</span>
          </Fact>
        )}
      </PaneSection>
    </>
  );
}

function EventBody({
  shown,
  fetched,
  delta,
}: {
  shown: TimelineEvent | null;
  fetched: UseQueryResult<TimelineEvent | null>;
  delta: SummaryDelta | null;
}) {
  if (shown) return <EventDetails event={shown} delta={delta} />;
  if (fetched.error) {
    return (
      <div className="p-4">
        <ErrorState message={fetched.error.message} />
      </div>
    );
  }
  if (fetched.isPending) {
    return (
      <div className="space-y-2 border-t p-4">
        <Skeleton className="h-4 w-2/3" />
        <Skeleton className="h-4 w-1/2" />
        <Skeleton className="h-4 w-3/5" />
      </div>
    );
  }
  return <p className="border-t p-4 text-sm text-muted-foreground">This version is no longer retained.</p>;
}

/** Everything recorded for one version (commit, change, operation details, totals, writer and
 *  transaction), or the decoded files behind it in the wider source view. Pass `event` when the
 *  caller already loaded the version's timeline event; otherwise it is fetched. `delta` is the
 *  change against the next older loaded version. */
export function VersionDetailPane({
  segments,
  version,
  event,
  delta,
  view,
  onView,
  onClose,
  top,
}: {
  segments: readonly string[];
  version: number;
  event?: TimelineEvent;
  delta: SummaryDelta | null;
  view: PaneView;
  onView: (view: PaneView) => void;
  onClose: () => void;
  top?: number;
}) {
  const fetched = useTimelineEvent(segments, version, event === undefined);
  const shown = event ?? fetched.data ?? null;
  return (
    <SidePane
      label={`Version ${version} details`}
      header={<EventHeader version={version} event={shown} />}
      onClose={onClose}
      top={top}
      bodyKey={version}
      toolbar={<PaneTabs views={VIEWS} value={view} onChange={onView} />}
    >
      {view === "details" ? (
        <EventBody shown={shown} fetched={fetched} delta={delta} />
      ) : (
        <VersionSource segments={segments} version={version} />
      )}
    </SidePane>
  );
}
