import { useState } from "react";
import { useQueryClient, useMutation } from "@tanstack/react-query";
import { toast } from "sonner";
import { declareTable } from "@/lib/api";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

interface RegisterDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

const ID_RE = /^[A-Za-z0-9._-]+$/;

export function RegisterDialog({ open, onOpenChange }: RegisterDialogProps) {
  const qc = useQueryClient();
  const [id, setId] = useState("");
  const [owner, setOwner] = useState("");
  const [keepLastN, setKeepLastN] = useState("");
  const [maxAgeDays, setMaxAgeDays] = useState("");

  const mutation = useMutation({
    mutationFn: () => {
      const ttl = {
        keep_last_n: keepLastN.trim() ? Number(keepLastN) : null,
        max_age_days: maxAgeDays.trim() ? Number(maxAgeDays) : null,
      };
      const hasTtl = ttl.keep_last_n != null || ttl.max_age_days != null;
      return declareTable(id.trim(), {
        owner: owner.trim() || undefined,
        ttl_policy: hasTtl ? { keep_last_n: ttl.keep_last_n, max_age_days: ttl.max_age_days } : undefined,
      });
    },
    onSuccess: (table) => {
      toast.success(`Registered "${table.id}"`);
      qc.invalidateQueries({ queryKey: ["tables"] });
      qc.invalidateQueries({ queryKey: ["namespaces"] });
      reset();
      onOpenChange(false);
    },
    onError: (e: unknown) => toast.error((e as Error).message),
  });

  function reset() {
    setId("");
    setOwner("");
    setKeepLastN("");
    setMaxAgeDays("");
  }

  const idValid = id.trim().length > 0 && id.trim().length <= 255 && ID_RE.test(id.trim());

  return (
    <Dialog open={open} onOpenChange={(o) => { if (!o) reset(); onOpenChange(o); }}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Register table</DialogTitle>
          <DialogDescription>
            Registers the table by writing its authored overlay. Derived fields (versions, sizes) are
            filled by the next sweep.
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-4 py-2">
          <div className="grid gap-2">
            <Label htmlFor="reg-id">Table id</Label>
            <Input
              id="reg-id"
              value={id}
              onChange={(e) => setId(e.target.value)}
              placeholder="my_table.v2"
              autoFocus
            />
            {!idValid && id.length > 0 && (
              <p className="text-xs text-destructive">
                Must match [A-Za-z0-9._-], max 255 chars.
              </p>
            )}
          </div>
          <div className="grid gap-2">
            <Label htmlFor="reg-owner">Owner (optional)</Label>
            <Input id="reg-owner" value={owner} onChange={(e) => setOwner(e.target.value)} placeholder="team@example" />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-2">
              <Label htmlFor="reg-keep">Keep last N</Label>
              <Input id="reg-keep" type="number" min={0} value={keepLastN} onChange={(e) => setKeepLastN(e.target.value)} placeholder="10" />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="reg-age">Max age (days)</Label>
              <Input id="reg-age" type="number" min={0} value={maxAgeDays} onChange={(e) => setMaxAgeDays(e.target.value)} placeholder="30" />
            </div>
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button>
          <Button disabled={!idValid || mutation.isPending} onClick={() => mutation.mutate()}>
            {mutation.isPending ? "Registering…" : "Register"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
