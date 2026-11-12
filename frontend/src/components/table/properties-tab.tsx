import type { UseQueryResult } from "@tanstack/react-query";
import type { DescribeTableResponse } from "@/lib/schemas";
import {
  DESCRIPTION,
  OWNER,
  REGISTERED_AT,
  UPDATED_AT,
  configEntries,
  extraProperties,
  tableMetadataEntries,
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

function About({ cheap }: { cheap: UseQueryResult<DescribeTableResponse> }) {
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
    </dl>
  );
}

/** About facts first (catalog-recorded, from the cheap describe), then client properties and the
 *  two Lance manifest maps (`table_metadata`, `config`). Storage format and served version live
 *  in the page header, tags and branches on the Versions tab, schema metadata on the Schema tab. */
export function PropertiesTab({
  cheap,
  detail,
}: {
  cheap: UseQueryResult<DescribeTableResponse>;
  detail: UseQueryResult<DescribeTableResponse>;
}) {
  return (
    <div className="@container space-y-6">
      <Section title="About">
        <About cheap={cheap} />
      </Section>

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
