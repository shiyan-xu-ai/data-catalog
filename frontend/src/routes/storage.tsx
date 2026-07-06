import { createFileRoute } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { getStorage } from "@/lib/api";
import type { StoragePrefixStat } from "@/lib/schemas";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ErrorState } from "@/components/error-state";
import { EmptyState } from "@/components/empty-state";
import { formatBytes, formatCount, formatTime } from "@/lib/format";
import { useTz } from "@/lib/timezone";

export const Route = createFileRoute("/storage")({
  component: StoragePage,
});

interface BucketGroup {
  bucket: string;
  region: string;
  registered: StoragePrefixStat[];
  unexplored: StoragePrefixStat[];
}

function groupByBucket(stats: StoragePrefixStat[]): BucketGroup[] {
  const groups = new Map<string, BucketGroup>();
  for (const s of stats) {
    const key = `${s.region}:${s.bucket}`;
    let g = groups.get(key);
    if (!g) {
      g = { bucket: s.bucket, region: s.region, registered: [], unexplored: [] };
      groups.set(key, g);
    }
    if (s.registered) g.registered.push(s);
    else g.unexplored.push(s);
  }
  return [...groups.values()].sort((a, b) => key(a).localeCompare(key(b)));
}

function key(g: BucketGroup): string {
  return `${g.bucket}:${g.region}`;
}

function StoragePage() {
  const { mode: tz } = useTz();
  const { data, isLoading, error } = useQuery({
    queryKey: ["storage"],
    queryFn: getStorage,
  });

  const groups = groupByBucket(data ?? []);

  return (
    <div className="space-y-4">
      <h1 className="text-2xl font-semibold">Storage</h1>

      {error ? (
        <ErrorState message={(error as Error).message} />
      ) : isLoading ? (
        <div className="py-8 text-center text-sm text-muted-foreground">Loading…</div>
      ) : groups.length === 0 ? (
        <EmptyState>No storage scan has run yet.</EmptyState>
      ) : (
        <div className="space-y-4">
          {groups.map((g) => (
            <Card key={key(g)}>
              <CardHeader>
                <CardTitle>
                  {g.bucket} ({g.region})
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                {g.registered.length === 0 ? (
                  <EmptyState>No registered namespaces in this bucket.</EmptyState>
                ) : (
                  <div className="overflow-x-auto rounded-lg border">
                    <table className="w-full text-sm">
                      <thead className="border-b bg-muted/40 text-left text-xs uppercase tracking-wide text-muted-foreground">
                        <tr>
                          <th className="px-3 py-2">Namespace</th>
                          <th className="px-3 py-2 text-right">Size</th>
                          <th className="px-3 py-2 text-right">Objects</th>
                          <th className="px-3 py-2 text-right">Tables</th>
                          <th className="px-3 py-2">Scanned</th>
                        </tr>
                      </thead>
                      <tbody>
                        {g.registered.map((s) => (
                          <tr key={s.prefix} className="border-b last:border-0 hover:bg-muted/40">
                            <td className="px-3 py-2 font-mono text-xs">{s.prefix}</td>
                            <td className="px-3 py-2 text-right tabular-nums text-muted-foreground">
                              {formatBytes(s.bytes)}
                            </td>
                            <td className="px-3 py-2 text-right tabular-nums text-muted-foreground">
                              {formatCount(s.objects)}
                            </td>
                            <td className="px-3 py-2 text-right tabular-nums text-muted-foreground">
                              {formatCount(s.table_count)}
                            </td>
                            <td className="px-3 py-2 text-muted-foreground">{formatTime(s.scanned_at, tz)}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                )}

                {g.unexplored.length > 0 && (
                  <div className="space-y-1">
                    <p className="text-xs uppercase tracking-wide text-muted-foreground">
                      Unexplored prefixes
                    </p>
                    <ul className="space-y-0.5 text-sm text-muted-foreground">
                      {g.unexplored.map((s) => (
                        <li key={s.prefix} className="font-mono text-xs">
                          {s.prefix}
                        </li>
                      ))}
                    </ul>
                  </div>
                )}
              </CardContent>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
