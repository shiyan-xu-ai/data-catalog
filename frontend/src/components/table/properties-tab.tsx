import type { UseQueryResult } from "@tanstack/react-query";
import type { DescribeTableResponse } from "@/lib/schemas";
import { configEntries, extraProperties, tableMetadataEntries } from "@/lib/table-metadata";
import { PropertiesList } from "@/components/properties-list";
import { QueryBody } from "@/components/query-body";
import { Section } from "@/components/section";

/** Client properties (from the cheap describe) and the two Lance manifest maps (`table_metadata`,
 *  `config`). */
export function PropertiesTab({
  cheap,
  detail,
}: {
  cheap: UseQueryResult<DescribeTableResponse>;
  detail: UseQueryResult<DescribeTableResponse>;
}) {
  return (
    <div className="@container space-y-6">
      <Section
        title="Catalog properties"
        help="Properties set at registration, including owner and description, plus the catalog's registration and update times. The business and technical owners are on the Overview."
      >
        <QueryBody query={cheap} skeletonClassName="h-4 w-48">
          {(c) => <PropertiesList entries={extraProperties(c.properties)} empty="No client properties." />}
        </QueryBody>
      </Section>

      <Section title="Lance table metadata" help="Manifest table_metadata of the current version.">
        <QueryBody query={detail} skeletonClassName="h-4 w-48">
          {(d) => <PropertiesList entries={tableMetadataEntries(d.metadata)} empty="No Lance table metadata." />}
        </QueryBody>
      </Section>

      <Section title="Lance table configuration" help="Manifest config of the current version.">
        <QueryBody query={detail} skeletonClassName="h-4 w-48">
          {(d) => <PropertiesList entries={configEntries(d.metadata)} empty="No Lance table configuration." />}
        </QueryBody>
      </Section>
    </div>
  );
}
