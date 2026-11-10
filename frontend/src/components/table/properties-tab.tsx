import type { UseQueryResult } from "@tanstack/react-query";
import { useTableBranches, useTableTags } from "@/lib/catalog-queries";
import { formatBytes } from "@/lib/format";
import type { DescribeTableResponse } from "@/lib/schemas";
import {
  DESCRIPTION,
  OWNER,
  REGISTERED_AT,
  UPDATED_AT,
  configEntries,
  extraProperties,
} from "@/lib/table-metadata";
import { Skeleton } from "@/components/ui/skeleton";
import { CopyButton } from "@/components/copy-button";
import { ErrorState } from "@/components/error-state";
import { PropertiesList } from "@/components/properties-list";
import { Section } from "@/components/section";
import { Timestamp } from "@/components/timestamp";

function Field({ label, children, className }: { label: string; children: React.ReactNode; className?: string }) {
  return (
    <div className={className}>
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="mt-0.5 text-sm break-words">{children}</dd>
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

/** Lance-derived section body: skeleton while loading, an error of its own on failure. */
function LanceBody({
  detail,
  children,
}: {
  detail: UseQueryResult<DescribeTableResponse>;
  children: (data: DescribeTableResponse) => React.ReactNode;
}) {
  if (detail.error) return <ErrorState message={detail.error.message} />;
  if (detail.isPending) return <Skeleton className="h-4 w-48" />;
  return <>{children(detail.data)}</>;
}

function About({
  segments,
  cheap,
}: {
  segments: readonly string[];
  cheap: UseQueryResult<DescribeTableResponse>;
}) {
  const tags = useTableTags(segments);
  const branches = useTableBranches(segments);
  const props = cheap.data?.properties ?? {};
  const location = cheap.data?.location ?? cheap.data?.table_uri ?? null;

  if (cheap.error) return <ErrorState message={cheap.error.message} />;
  if (cheap.isPending) {
    return (
      <div className="space-y-2">
        <Skeleton className="h-4 w-2/3" />
        <Skeleton className="h-4 w-1/2" />
      </div>
    );
  }
  return (
    <dl className="grid grid-cols-2 gap-x-6 gap-y-4 @3xl:grid-cols-4">
      <Field label="Description" className="col-span-2 @3xl:col-span-4">
        {props[DESCRIPTION] ? <p className="whitespace-pre-wrap">{props[DESCRIPTION]}</p> : <Unset>No description</Unset>}
      </Field>
      <Field label="Location" className="col-span-2 @3xl:col-span-4">
        {location ? (
          <div className="flex items-start gap-1">
            <code className="min-w-0 rounded bg-muted px-1.5 py-1 font-mono text-xs break-all">{location}</code>
            <CopyButton value={location} label="Copy location" />
          </div>
        ) : (
          <Unset />
        )}
      </Field>
      <Field label="Owner">{props[OWNER] ?? <Unset />}</Field>
      <Field label="Registered">
        <Timestamp value={props[REGISTERED_AT]} stacked />
      </Field>
      <Field label="Updated">
        <Timestamp value={props[UPDATED_AT]} stacked />
      </Field>
      <Field label="Tags">
        <Lance query={tags}>
          {(t) => {
            const entries = Object.entries(t.tags).sort(([a], [b]) => a.localeCompare(b));
            if (entries.length === 0) return <Unset>No tags</Unset>;
            return (
              <ul className="space-y-0.5">
                {entries.map(([name, tag]) => (
                  <li key={name} className="font-mono text-xs">
                    {name}{" "}
                    <span className="text-muted-foreground tabular-nums">
                      → {tag.branch ? `${tag.branch}@` : ""}v{tag.version}
                    </span>
                  </li>
                ))}
              </ul>
            );
          }}
        </Lance>
      </Field>
      <Field label="Branches">
        <Lance query={branches}>
          {(b) => {
            const entries = Object.entries(b.branches).sort(([x], [y]) => x.localeCompare(y));
            if (entries.length === 0) return <Unset>No branches</Unset>;
            return (
              <ul className="space-y-0.5">
                {entries.map(([name, br]) => (
                  <li key={name} className="font-mono text-xs">
                    {name}{" "}
                    <span className="text-muted-foreground tabular-nums">
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
      </Field>
    </dl>
  );
}

/** About facts first (catalog-recorded from the cheap describe, tags and branches from their
 *  routes), then client properties and Lance table configuration. Storage format and current
 *  version live in the page header; schema metadata lives on the Schema tab. Each
 *  Lance-derived part loads and fails on its own. */
export function PropertiesTab({
  segments,
  cheap,
  detail,
}: {
  segments: readonly string[];
  cheap: UseQueryResult<DescribeTableResponse>;
  detail: UseQueryResult<DescribeTableResponse>;
}) {
  return (
    <div className="@container space-y-6">
      <Section title="About">
        <About segments={segments} cheap={cheap} />
      </Section>

      <Section title="Catalog properties" description="Client properties recorded at registration">
        {cheap.error ? (
          <ErrorState message={cheap.error.message} />
        ) : cheap.isPending ? (
          <Skeleton className="h-4 w-48" />
        ) : (
          <PropertiesList entries={extraProperties(cheap.data.properties)} empty="No client properties." />
        )}
      </Section>

      <Section title="Lance table configuration" description="Manifest config entries of the current version">
        <LanceBody detail={detail}>
          {(d) => <PropertiesList entries={configEntries(d.metadata)} empty="No Lance table configuration." />}
        </LanceBody>
      </Section>
    </div>
  );
}
