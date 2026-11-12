import { useMutation, useQueryClient, type UseQueryResult } from "@tanstack/react-query";
import { Loader2, RefreshCw } from "lucide-react";
import { refreshTable } from "@/lib/api";
import { keys, type PagedVersions } from "@/lib/catalog-queries";
import { formatRelative, parseTime } from "@/lib/format";
import { joinId } from "@/lib/ids";
import type { DescribeTableResponse } from "@/lib/schemas";
import { BUSINESS_OWNER, TECHNICAL_OWNER, tableFormat } from "@/lib/table-metadata";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { CopyButton } from "@/components/copy-button";
import { ErrorState } from "@/components/error-state";

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

/** The version the catalog considers current, with its commit age. When a later version is
 *  committed, names it and offers a refresh that makes it current; every cached read of the
 *  table is then refetched. */
function CurrentVersion({
  segments,
  versions,
  current,
}: {
  segments: readonly string[];
  versions: PagedVersions;
  current: number | null | undefined;
}) {
  const queryClient = useQueryClient();
  const refresh = useMutation({
    mutationFn: () => refreshTable(segments),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: keys.table(joinId(segments)) }),
  });
  if (current == null) {
    return (
      <Pending error={versions.error} loading={versions.isPending}>
        <Unset />
      </Pending>
    );
  }
  const latest = versions.versions[0];
  const behind = latest != null && latest.version > current;
  const committed = versions.versions.find((v) => v.version === current);
  return (
    <>
      <div className="flex items-center gap-2">
        <span className="font-mono">v{current}</span>
        {behind && (
          <Button
            size="sm"
            variant="outline"
            className="h-6 gap-1 px-2 text-xs"
            disabled={refresh.isPending}
            onClick={() => refresh.mutate()}
            title={`Make the newest committed version (v${latest.version}) current`}
          >
            {refresh.isPending ? <Loader2 className="animate-spin" /> : <RefreshCw />}
            Refresh
          </Button>
        )}
      </div>
      <div className="text-xs text-muted-foreground">
        {committed && formatRelative(parseTime(committed.timestamp_millis))}
        {committed && behind && " · "}
        {behind && `latest v${latest.version}`}
      </div>
      {refresh.error && <div className="text-xs text-destructive">Refresh failed: {refresh.error.message}</div>}
    </>
  );
}

interface AboutProps {
  segments: readonly string[];
  cheap: UseQueryResult<DescribeTableResponse>;
  detail: UseQueryResult<DescribeTableResponse>;
  versions: PagedVersions;
}

function AboutBody({ segments, cheap, detail, versions }: AboutProps) {
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
      <Field label="Table format">
        <Pending error={detail.error} loading={!detail.data}>
          {tableFormat(detail.data?.metadata) ?? <Unset />}
        </Pending>
      </Field>
      <Field label="Current version">
        <Pending error={detail.error} loading={!detail.data}>
          <CurrentVersion segments={segments} versions={versions} current={detail.data?.version} />
        </Pending>
      </Field>
      <Field label="Business owner">{props[BUSINESS_OWNER] ?? <Unset />}</Field>
      <Field label="Technical owner">{props[TECHNICAL_OWNER] ?? <Unset />}</Field>
      <Field label="Location" className="col-span-full">
        {location ? (
          <div className="flex items-start gap-1">
            <code className="min-w-0 rounded bg-muted px-1.5 py-1 font-mono text-xs break-all">{location}</code>
            <CopyButton value={location} label="Copy location" />
          </div>
        ) : (
          <Unset />
        )}
      </Field>
    </dl>
  );
}

/** Untitled card: the current version's table format, the current version, the business and
 *  technical owners (`business_owner` / `technical_owner` table properties), then the location.
 *  Place it inside an `@container`. */
export function AboutSection({ segments, cheap, detail, versions }: AboutProps) {
  return (
    <section aria-label="About" className="rounded-lg border bg-card p-4 text-card-foreground shadow-xs">
      <AboutBody segments={segments} cheap={cheap} detail={detail} versions={versions} />
    </section>
  );
}
