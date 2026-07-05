import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Play, Save, Trash2 } from "lucide-react";
import { ttlApply, ttlDryRun } from "@/lib/api";
import type { TtlAuditRecord, TtlPolicy } from "@/lib/schemas";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { EmptyState } from "@/components/empty-state";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { formatBytes, formatTime, ttlPolicyString } from "@/lib/format";
import { declareTable } from "@/lib/api";

interface Props {
  tableId: string;
  policy: TtlPolicy | null;
  audit: TtlAuditRecord[];
}

export function TtlPanel({ tableId, policy, audit }: Props) {
  const qc = useQueryClient();
  const [keepLastN, setKeepLastN] = useState(policy?.keep_last_n?.toString() ?? "");
  const [maxAgeDays, setMaxAgeDays] = useState(policy?.max_age_days?.toString() ?? "");

  const dryRun = useQuery({
    queryKey: ["ttl-dryrun", tableId],
    queryFn: () => ttlDryRun(tableId),
  });

  const saveMut = useMutation({
    mutationFn: () => {
      const ttl: { keep_last_n: number | null; max_age_days: number | null } = {
        keep_last_n: keepLastN.trim() ? Number(keepLastN) : null,
        max_age_days: maxAgeDays.trim() ? Number(maxAgeDays) : null,
      };
      return declareTable(tableId, { ttl_policy: ttl });
    },
    onSuccess: () => {
      toast.success("TTL policy saved");
      qc.invalidateQueries({ queryKey: ["table", tableId] });
      qc.invalidateQueries({ queryKey: ["tables"] });
    },
    onError: (e: unknown) => toast.error((e as Error).message),
  });

  const applyMut = useMutation({
    mutationFn: () => ttlApply(tableId),
    onSuccess: (res) => {
      toast.success(`Deleted ${res.deleted.length} version(s), reclaimed ${formatBytes(res.reclaimed_bytes)}`);
      qc.invalidateQueries({ queryKey: ["table", tableId] });
      qc.invalidateQueries({ queryKey: ["tables"] });
      qc.invalidateQueries({ queryKey: ["ttl-dryrun", tableId] });
      qc.invalidateQueries({ queryKey: ["ttl-audit", tableId] });
    },
    onError: (e: unknown) => toast.error((e as Error).message),
  });

  return (
    <Card>
      <CardHeader>
        <CardTitle>TTL</CardTitle>
      </CardHeader>
      <CardContent className="space-y-6">
        {/* Policy editor */}
        <div className="space-y-3">
          <div className="grid grid-cols-2 gap-3 max-w-md">
            <div className="grid gap-2">
              <Label htmlFor="keep-last-n">Keep last N</Label>
              <Input id="keep-last-n" type="number" min={0} value={keepLastN} onChange={(e) => setKeepLastN(e.target.value)} placeholder="10" />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="max-age-days">Max age (days)</Label>
              <Input id="max-age-days" type="number" min={0} value={maxAgeDays} onChange={(e) => setMaxAgeDays(e.target.value)} placeholder="30" />
            </div>
          </div>
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <span>Current:</span>
            <Badge variant="secondary">{policy ? ttlPolicyString(policy) : "none"}</Badge>
          </div>
          <Button size="sm" variant="outline" disabled={saveMut.isPending} onClick={() => saveMut.mutate()}>
            <Save className="h-4 w-4" /> {saveMut.isPending ? "Saving…" : "Save policy"}
          </Button>
        </div>

        {/* Dry-run */}
        <div className="space-y-2 border-t pt-4">
          <div className="flex items-center justify-between">
            <h4 className="text-sm font-medium">Dry-run</h4>
            <Button size="sm" variant="outline" onClick={() => dryRun.refetch()} disabled={dryRun.isFetching}>
              <Play className="h-4 w-4" /> {dryRun.isFetching ? "Running…" : "Refresh"}
            </Button>
          </div>
          {dryRun.isError ? (
            <p className="text-sm text-destructive">{(dryRun.error as Error).message}</p>
          ) : dryRun.data ? (
            dryRun.data.candidates.length === 0 ? (
              <EmptyState>No versions eligible under the current policy.</EmptyState>
            ) : (
              <div className="space-y-1">
                <p className="text-sm">
                  <span className="font-medium">{dryRun.data.candidates.length}</span> eligible ·{" "}
                  <span className="font-medium">{formatBytes(dryRun.data.reclaimable_bytes)}</span> reclaimable
                </p>
                <ul className="ml-4 list-disc text-xs text-muted-foreground">
                  {dryRun.data.candidates.map((c) => <li key={c} className="font-mono">{c}</li>)}
                </ul>
              </div>
            )
          ) : null}
        </div>

        {/* Apply */}
        <div className="space-y-2 border-t pt-4">
          <div className="flex items-center justify-between">
            <h4 className="text-sm font-medium">Apply</h4>
            <ConfirmDialog
              trigger={
                <Button size="sm" variant="destructive" disabled={!dryRun.data?.candidates.length || applyMut.isPending}>
                  <Trash2 className="h-4 w-4" /> Apply
                </Button>
              }
              title="Apply TTL — irreversible hard delete"
              description={
                dryRun.data
                  ? `This will hard-delete ${dryRun.data.candidates.length} version(s) and reclaim ${formatBytes(dryRun.data.reclaimable_bytes)}. This cannot be undone.`
                  : "This will hard-delete all TTL-eligible versions. This cannot be undone."
              }
              confirmLabel="Delete versions"
              destructive
              onConfirm={async () => { await applyMut.mutateAsync(); }}
            />
          </div>
          <p className="text-xs text-muted-foreground">
            Apply deletes eligible versions from S3 and appends audit records. Protected versions are exempt.
          </p>
        </div>

        {/* Audit */}
        <div className="space-y-2 border-t pt-4">
          <h4 className="text-sm font-medium">Audit log</h4>
          {audit.length === 0 ? (
            <EmptyState>No TTL deletions recorded.</EmptyState>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead className="border-b bg-muted/40 text-left text-xs uppercase tracking-wide text-muted-foreground">
                  <tr>
                    <th className="px-3 py-2">Version</th>
                    <th className="px-3 py-2 text-right">Reclaimed</th>
                    <th className="px-3 py-2">Policy</th>
                    <th className="px-3 py-2">Deleted at</th>
                    <th className="px-3 py-2">Actor</th>
                  </tr>
                </thead>
                <tbody>
                  {audit.map((r) => (
                    <tr key={r.version_id + r.deleted_at} className="border-b last:border-0">
                      <td className="px-3 py-2 font-mono text-xs">{r.version_id}</td>
                      <td className="px-3 py-2 text-right tabular-nums">{formatBytes(r.reclaimed_bytes)}</td>
                      <td className="px-3 py-2"><Badge variant="outline">{ttlPolicyString(r.policy_snapshot)}</Badge></td>
                      <td className="px-3 py-2 text-muted-foreground">{formatTime(r.deleted_at)}</td>
                      <td className="px-3 py-2 text-muted-foreground">{r.actor}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
