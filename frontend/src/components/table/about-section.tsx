import type { UseQueryResult } from "@tanstack/react-query";
import type { PagedVersions } from "@/lib/catalog-queries";
import { formatRelative, parseTime } from "@/lib/format";
import type { DescribeTableResponse } from "@/lib/schemas";
import { DESCRIPTION, OWNER, REGISTERED_AT, UPDATED_AT, tableFormat } from "@/lib/table-metadata";
import { Skeleton } from "@/components/ui/skeleton";
import { CopyButton } from "@/components/copy-button";
import { ErrorState } from "@/components/error-state";
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

/** A Lance-derived field value: a short failure note on error, a skeleton while loading. */
function Pending({ error, loading, children }: { error: Error | null; loading: boolean; children: React.ReactNode }) {
  if (error) {
    return (
      <span className="text-xs text-muted-foreground" title={error.message}>
        Unavailable
      </span>
    );
  }
  if (loading) return <Skeleton className="h-4 w-16" />;
  return <>{children}</>;
}

/** The newest committed version (first of the newest-first list) and, when it differs, the
 *  version the catalog considers current. */
function LatestVersion({ versions, current }: { versions: PagedVersions; current: number | null | undefined }) {
  const latest = versions.versions[0];
  if (!latest) {
    return (
      <Pending error={versions.error} loading={versions.isPending}>
        <Unset />
      </Pending>
    );
  }
  return (
    <>
      <div className="font-mono">v{latest.version}</div>
      <div className="text-xs text-muted-foreground">
        {formatRelative(parseTime(latest.timestamp_millis))}
        {current != null && current !== latest.version && ` · current v${current}`}
      </div>
    </>
  );
}

interface AboutProps {
  cheap: UseQueryResult<DescribeTableResponse>;
  detail: UseQueryResult<DescribeTableResponse>;
  versions: PagedVersions;
}

function AboutBody({ cheap, detail, versions }: AboutProps) {
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
    <dl className="grid grid-cols-2 gap-x-6 gap-y-4 @3xl:grid-cols-5">
      <Field label="Description" className="col-span-2 @3xl:col-span-5">
        {props[DESCRIPTION] ? <p className="whitespace-pre-wrap">{props[DESCRIPTION]}</p> : <Unset>No description</Unset>}
      </Field>
      <Field label="Location" className="col-span-2 @3xl:col-span-5">
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
      <Field label="Table format">
        <Pending error={detail.error} loading={!detail.data}>
          {tableFormat(detail.data?.metadata) ?? <Unset />}
        </Pending>
      </Field>
      <Field label="Latest version">
        <LatestVersion versions={versions} current={detail.data?.version} />
      </Field>
    </dl>
  );
}

/** Untitled card of catalog-recorded facts from the cheap describe (description, location,
 *  owner, registration and update times), the table format of the current version, and the
 *  newest committed version. Place it inside an `@container`. */
export function AboutSection({ cheap, detail, versions }: AboutProps) {
  return (
    <section aria-label="About" className="rounded-lg border bg-card p-4 text-card-foreground shadow-xs">
      <AboutBody cheap={cheap} detail={detail} versions={versions} />
    </section>
  );
}
