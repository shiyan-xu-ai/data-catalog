/** Key/value list for spec `properties`. */
export function PropertiesList({ properties }: { properties: Record<string, string> | null | undefined }) {
  const entries = Object.entries(properties ?? {}).sort(([a], [b]) => a.localeCompare(b));
  if (entries.length === 0) return <p className="text-sm text-muted-foreground">No properties.</p>;
  return (
    <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1">
      {entries.map(([k, v]) => (
        <div key={k} className="contents">
          <dt className="font-mono text-xs text-muted-foreground">{k}</dt>
          <dd className="font-mono text-xs break-all">{v}</dd>
        </div>
      ))}
    </dl>
  );
}
