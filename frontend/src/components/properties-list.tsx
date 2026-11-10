/** Key/value list; renders `empty` when there are no entries. */
export function PropertiesList({ entries, empty }: { entries: readonly [string, string][]; empty: string }) {
  if (entries.length === 0) return <p className="text-xs text-muted-foreground italic">{empty}</p>;
  return (
    <dl className="grid grid-cols-[minmax(0,max-content)_minmax(0,1fr)] gap-x-3 gap-y-1.5">
      {entries.map(([k, v]) => (
        <div key={k} className="contents">
          <dt className="truncate font-mono text-xs text-muted-foreground" title={k}>{k}</dt>
          <dd className="font-mono text-xs break-all">{v}</dd>
        </div>
      ))}
    </dl>
  );
}
