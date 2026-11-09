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

export function RegisterDialog({ open, onOpenChange }: RegisterDialogProps) {
  const qc = useQueryClient();
  const [id, setId] = useState("");
  const [owner, setOwner] = useState("");

  const mutation = useMutation({
    mutationFn: () => declareTable(id.trim(), { owner: owner.trim() || undefined }),
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
  }

  const hasId = id.trim().length > 0;

  return (
    <Dialog open={open} onOpenChange={(o) => { if (!o) reset(); onOpenChange(o); }}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Register table</DialogTitle>
          <DialogDescription>
            Track a table by its region:bucket:namespace:name id. Versions and sizes
            are filled by the next sync.
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-4 py-2">
          <div className="grid gap-2">
            <Label htmlFor="reg-id">Table id</Label>
            <Input
              id="reg-id"
              value={id}
              onChange={(e) => setId(e.target.value)}
              placeholder="region:bucket:namespace:table"
              autoFocus
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="reg-owner">Owner (optional)</Label>
            <Input id="reg-owner" value={owner} onChange={(e) => setOwner(e.target.value)} placeholder="team@example" />
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button>
          <Button disabled={!hasId || mutation.isPending} onClick={() => mutation.mutate()}>
            {mutation.isPending ? "Registering…" : "Register"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
