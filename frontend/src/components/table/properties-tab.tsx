import type { UseQueryResult } from "@tanstack/react-query";
import type { DescribeTableResponse } from "@/lib/schemas";
import { configEntries, extraProperties, tableMetadataEntries } from "@/lib/table-metadata";
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

/** Client properties (from the cheap describe) and the two Lance manifest maps (`table_metadata`,
 *  `config`). The About facts live on the Overview tab, storage format and served version in the
 *  page header, tags and branches on the Versions tab, schema metadata on the Schema tab. */
export function PropertiesTab({
  cheap,
  detail,
}: {
  cheap: UseQueryResult<DescribeTableResponse>;
  detail: UseQueryResult<DescribeTableResponse>;
}) {
  return (
    <div className="@container space-y-6">
      <Section title="Catalog properties" help="Client properties set at registration.">
        {cheap.error ? (
          <ErrorState message={cheap.error.message} />
        ) : cheap.isPending ? (
          <Skeleton className="h-4 w-48" />
        ) : (
          <PropertiesList entries={extraProperties(cheap.data.properties)} empty="No client properties." />
        )}
      </Section>

      <Section title="Lance table metadata" help="Manifest table_metadata of the served version.">
        <LanceBody detail={detail}>
          {(d) => <PropertiesList entries={tableMetadataEntries(d.metadata)} empty="No Lance table metadata." />}
        </LanceBody>
      </Section>

      <Section title="Lance table configuration" help="Manifest config of the served version.">
        <LanceBody detail={detail}>
          {(d) => <PropertiesList entries={configEntries(d.metadata)} empty="No Lance table configuration." />}
        </LanceBody>
      </Section>
    </div>
  );
}
