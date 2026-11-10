import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import { registerTable } from "@/lib/api";
import { displayName, joinId, segmentError } from "@/lib/ids";
import { buildProperties, type PropertyRow } from "@/lib/properties";
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
import { PropertiesEditor } from "@/components/properties-editor";

interface RegisterDialogProps {
  namespace: readonly string[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

/** Register an existing Lance dataset URI as a table in `namespace`. */
export function RegisterDialog({ namespace, open, onOpenChange }: RegisterDialogProps) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [name, setName] = useState("");
  const [location, setLocation] = useState("");
  const [owner, setOwner] = useState("");
  const [rows, setRows] = useState<PropertyRow[]>([]);

  const nameError = name ? segmentError(name) : null;
  const props = buildProperties(owner, rows);
  const canSubmit = name.length > 0 && !nameError && location.trim().length > 0 && props.ok;

  const mutation = useMutation({
    mutationFn: () => {
      if (!props.ok) throw new Error(props.error);
      return registerTable([...namespace, name], location.trim(), props.properties);
    },
    onSuccess: () => {
      const table = [...namespace, name];
      toast.success(`Registered "${displayName(table)}"`);
      qc.invalidateQueries({ queryKey: ["tables", joinId(namespace)] });
      close();
      navigate({ to: "/tables/$tableId", params: { tableId: joinId(table) } });
    },
    onError: (e: unknown) => toast.error((e as Error).message),
  });

  function close() {
    setName("");
    setLocation("");
    setOwner("");
    setRows([]);
    onOpenChange(false);
  }

  return (
    <Dialog open={open} onOpenChange={(o) => (o ? onOpenChange(true) : close())}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Register table</DialogTitle>
          <DialogDescription>
            Register an existing Lance dataset in{" "}
            <span className="font-mono">{displayName(namespace)}</span>. Registration records
            metadata only; it does not copy or modify data.
          </DialogDescription>
        </DialogHeader>

        <form
          className="grid gap-4 py-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (canSubmit && !mutation.isPending) mutation.mutate();
          }}
        >
          <div className="grid gap-2">
            <Label htmlFor="reg-name">Table name</Label>
            <Input id="reg-name" value={name} onChange={(e) => setName(e.target.value)} autoFocus />
            {nameError && <p className="text-xs text-destructive">{nameError}</p>}
          </div>
          <div className="grid gap-2">
            <Label htmlFor="reg-location">Location</Label>
            <Input
              id="reg-location"
              value={location}
              onChange={(e) => setLocation(e.target.value)}
              placeholder="s3://bucket/path/table.lance"
            />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="reg-owner">Owner (optional)</Label>
            <Input id="reg-owner" value={owner} onChange={(e) => setOwner(e.target.value)} placeholder="team@example" />
          </div>
          <PropertiesEditor rows={rows} onChange={setRows} error={props.ok ? undefined : props.error} />

          <DialogFooter>
            <Button type="button" variant="outline" onClick={close}>Cancel</Button>
            <Button type="submit" disabled={!canSubmit || mutation.isPending}>
              {mutation.isPending ? "Registering…" : "Register"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
