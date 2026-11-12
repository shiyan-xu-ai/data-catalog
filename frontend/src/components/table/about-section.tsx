import type { UseQueryResult } from "@tanstack/react-query";
import type { DescribeTableResponse } from "@/lib/schemas";
import { DESCRIPTION, OWNER, REGISTERED_AT, UPDATED_AT } from "@/lib/table-metadata";
import { Skeleton } from "@/components/ui/skeleton";
import { CopyButton } from "@/components/copy-button";
import { ErrorState } from "@/components/error-state";
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

function AboutBody({ cheap }: { cheap: UseQueryResult<DescribeTableResponse> }) {
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

/** Catalog-recorded facts from the cheap describe: description, location, owner, and the
 *  registration and update times. Place it inside an `@container`. */
export function AboutSection({ cheap }: { cheap: UseQueryResult<DescribeTableResponse> }) {
  return (
    <Section title="About">
      <AboutBody cheap={cheap} />
    </Section>
  );
}
