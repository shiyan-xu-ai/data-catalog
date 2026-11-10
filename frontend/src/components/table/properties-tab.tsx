import type { UseQueryResult } from "@tanstack/react-query";
import type { DescribeTableResponse } from "@/lib/schemas";
import { DESCRIPTION, OWNER, configEntries, extraProperties } from "@/lib/table-metadata";
import { Skeleton } from "@/components/ui/skeleton";
import { ErrorState } from "@/components/error-state";
import { PropertiesList } from "@/components/properties-list";
import { Section } from "@/components/section";

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

/** Catalog properties (from the cheap describe), Lance table configuration, and schema-level
 *  metadata (both from the detailed describe), each with its own empty and error state. */
export function PropertiesTab({
  cheap,
  detail,
}: {
  cheap: UseQueryResult<DescribeTableResponse>;
  detail: UseQueryResult<DescribeTableResponse>;
}) {
  const props = cheap.data?.properties ?? {};
  const wellKnown: [string, string][] = [
    [OWNER, props[OWNER] ?? ""],
    [DESCRIPTION, props[DESCRIPTION] ?? ""],
  ];
  return (
    <div className="space-y-6">
      <Section title="Catalog properties" description="Recorded at registration; client properties are free-form">
        {cheap.error ? (
          <ErrorState message={cheap.error.message} />
        ) : cheap.isPending ? (
          <Skeleton className="h-4 w-48" />
        ) : (
          <div className="space-y-4">
            <dl className="grid grid-cols-[minmax(0,max-content)_minmax(0,1fr)] gap-x-3 gap-y-1.5">
              {wellKnown.map(([k, v]) => (
                <div key={k} className="contents">
                  <dt className="font-mono text-xs text-muted-foreground">{k}</dt>
                  <dd className="text-sm break-words whitespace-pre-wrap">
                    {v || <span className="text-muted-foreground/70 italic">Not set</span>}
                  </dd>
                </div>
              ))}
            </dl>
            <div className="space-y-2 border-t pt-3">
              <h3 className="text-[11px] font-semibold tracking-wider text-muted-foreground uppercase">Client properties</h3>
              <PropertiesList entries={extraProperties(props)} empty="No client properties." />
            </div>
          </div>
        )}
      </Section>

      <Section title="Lance table configuration" description="Manifest config entries of the current version">
        <LanceBody detail={detail}>
          {(d) => <PropertiesList entries={configEntries(d.metadata)} empty="No Lance table configuration." />}
        </LanceBody>
      </Section>

      <Section title="Schema metadata" description="Key/value metadata on the Arrow schema">
        <LanceBody detail={detail}>
          {(d) => (
            <PropertiesList
              entries={Object.entries(d.schema?.metadata ?? {}).sort(([a], [b]) => a.localeCompare(b))}
              empty="No schema-level metadata."
            />
          )}
        </LanceBody>
      </Section>
    </div>
  );
}
