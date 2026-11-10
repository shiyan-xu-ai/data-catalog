import type { UseQueryResult } from "@tanstack/react-query";
import { useTableBranches, useTableTags } from "@/lib/catalog-queries";
import { formatBytes } from "@/lib/format";
import type { DescribeTableResponse } from "@/lib/schemas";
import {
  DESCRIPTION,
  OWNER,
  REGISTERED_AT,
  UPDATED_AT,
  storageFormatVersion,
} from "@/lib/table-metadata";
import { Skeleton } from "@/components/ui/skeleton";
import { CopyButton } from "@/components/copy-button";
import { Timestamp } from "@/components/timestamp";

function Group({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="space-y-2.5 border-t px-4 py-4 first:border-t-0">
      <h3 className="text-[11px] font-semibold tracking-wider text-muted-foreground uppercase">{title}</h3>
      {children}
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="space-y-0.5">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="text-sm break-words">{children}</div>
    </div>
  );
}

const Unset = ({ children = "Not set" }: { children?: string }) => (
  <span className="text-muted-foreground/70 italic">{children}</span>
);

/** Lance-derived value: skeleton while loading, a short failure note on error (R19). */
function Lance<T>({ query, children }: { query: UseQueryResult<T>; children: (data: T) => React.ReactNode }) {
  if (query.error) return <span className="text-xs text-destructive" title={query.error.message}>Unavailable</span>;
  if (query.isPending) return <Skeleton className="h-4 w-20" />;
  return <>{children(query.data)}</>;
}

/** Always-rendered table facts. Catalog-recorded fields come from the cheap describe; Lance
 *  fields come from the detailed describe and fail independently. */
export function DetailsSidebar({
  segments,
  cheap,
  detail,
}: {
  segments: readonly string[];
  cheap: DescribeTableResponse | undefined;
  detail: UseQueryResult<DescribeTableResponse>;
}) {
  const tags = useTableTags(segments);
  const branches = useTableBranches(segments);
  const props = cheap?.properties ?? {};
  const location = cheap?.location ?? cheap?.table_uri ?? null;

  return (
    <aside className="rounded-lg border bg-card text-card-foreground shadow-xs">
      <Group title="About">
        <Field label="Description">
          {props[DESCRIPTION] ? <p className="whitespace-pre-wrap">{props[DESCRIPTION]}</p> : <Unset>No description</Unset>}
        </Field>
        <Field label="Owner">{props[OWNER] ?? <Unset />}</Field>
        <Field label="Location">
          {location ? (
            <div className="flex items-start gap-1">
              <code className="min-w-0 flex-1 rounded bg-muted px-1.5 py-1 font-mono text-xs break-all">{location}</code>
              <CopyButton value={location} label="Copy location" />
            </div>
          ) : (
            <Unset />
          )}
        </Field>
      </Group>

      <Group title="Format">
        <div className="grid grid-cols-2 gap-3">
          <Field label="Storage format">
            <Lance query={detail}>
              {(d) => {
                const v = storageFormatVersion(d.metadata);
                return v ? <span className="font-mono">Lance {v}</span> : <Unset>Unknown</Unset>;
              }}
            </Lance>
          </Field>
          <Field label="Current version">
            <Lance query={detail}>
              {(d) => (d.version != null ? <span className="font-mono tabular-nums">v{d.version}</span> : <Unset>Unknown</Unset>)}
            </Lance>
          </Field>
        </div>
      </Group>

      <Group title="Catalog">
        <div className="grid grid-cols-2 gap-3">
          <Field label="Registered">
            <Timestamp value={props[REGISTERED_AT]} stacked />
          </Field>
          <Field label="Updated">
            <Timestamp value={props[UPDATED_AT]} stacked />
          </Field>
        </div>
      </Group>

      <Group title="Tags">
        <Lance query={tags}>
          {(t) => {
            const entries = Object.entries(t.tags).sort(([a], [b]) => a.localeCompare(b));
            if (entries.length === 0) return <Unset>No tags</Unset>;
            return (
              <ul className="space-y-1">
                {entries.map(([name, tag]) => (
                  <li key={name} className="flex items-baseline justify-between gap-3 text-sm">
                    <span className="truncate font-mono text-xs">{name}</span>
                    <span className="shrink-0 font-mono text-xs text-muted-foreground tabular-nums">
                      {tag.branch ? `${tag.branch}@` : ""}v{tag.version}
                    </span>
                  </li>
                ))}
              </ul>
            );
          }}
        </Lance>
      </Group>

      <Group title="Branches">
        <Lance query={branches}>
          {(b) => {
            const entries = Object.entries(b.branches).sort(([x], [y]) => x.localeCompare(y));
            if (entries.length === 0) return <Unset>No branches</Unset>;
            return (
              <ul className="space-y-1">
                {entries.map(([name, br]) => (
                  <li key={name} className="flex items-baseline justify-between gap-3 text-sm">
                    <span className="truncate font-mono text-xs">{name}</span>
                    <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
                      from {br.parentBranch ?? "main"}
                      {br.parentVersion != null ? ` v${br.parentVersion}` : ""}
                      {br.manifestSize != null ? ` · ${formatBytes(br.manifestSize)}` : ""}
                    </span>
                  </li>
                ))}
              </ul>
            );
          }}
        </Lance>
      </Group>
    </aside>
  );
}
