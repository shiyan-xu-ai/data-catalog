import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import { registerTable } from "@/lib/api";
import { keys } from "@/lib/catalog-queries";
import { displayName, joinId, segmentError } from "@/lib/ids";
import { buildProperties, type PropertyRow } from "@/lib/properties";
import { CATALOG_PREFIX, DESCRIPTION, OWNER } from "@/lib/table-metadata";
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
import { Textarea } from "@/components/ui/textarea";
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
  const [description, setDescription] = useState("");
  const [rows, setRows] = useState<PropertyRow[]>([]);

  const nameError = name ? segmentError(name) : null;
  // `catalog.*` keys are server-managed; registration would reject them.
  const props = buildProperties({ [OWNER]: owner, [DESCRIPTION]: description }, rows, CATALOG_PREFIX);
  const canSubmit = name.length > 0 && !nameError && location.trim().length > 0 && props.ok;

  const mutation = useMutation({
    mutationFn: () => {
      if (!props.ok) throw new Error(props.error);
      return registerTable([...namespace, name], location.trim(), props.properties);
    },
    onSuccess: () => {
      const table = [...namespace, name];
      const tableId = joinId(table);
      toast.success(`Registered "${displayName(table)}"`);
      qc.invalidateQueries({ queryKey: keys.tables(joinId(namespace)) });
      qc.invalidateQueries({ queryKey: keys.allTables });
      qc.removeQueries({ queryKey: keys.table(tableId) });
      close();
      navigate({ to: "/tables/$tableId", params: { tableId } });
    },
    onError: (e: unknown) => toast.error((e as Error).message),
  });

  function close() {
    setName("");
    setLocation("");
    setOwner("");
    setDescription("");
    setRows([]);
    onOpenChange(false);
  }

  return (
    <Dialog open={open} onOpenChange={(o) => (o ? onOpenChange(true) : close())}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>Register table</DialogTitle>
          <DialogDescription>
            Register an existing Lance dataset in{" "}
            <span className="font-mono text-foreground">{displayName(namespace)}</span>. Registration
            records catalog metadata only; it never copies or modifies the data.
          </DialogDescription>
        </DialogHeader>

        <form
          className="grid gap-4 py-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (canSubmit && !mutation.isPending) mutation.mutate();
          }}
        >
          <div className="grid gap-1.5">
            <Label htmlFor="reg-name">Table name</Label>
            <Input id="reg-name" value={name} onChange={(e) => setName(e.target.value)} autoFocus />
            {nameError && <p className="text-xs text-destructive">{nameError}</p>}
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="reg-location">Location</Label>
            <Input
              id="reg-location"
              value={location}
              onChange={(e) => setLocation(e.target.value)}
              placeholder="s3://bucket/path/table.lance"
              className="font-mono text-xs"
            />
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="reg-owner">Owner <span className="font-normal text-muted-foreground">(optional)</span></Label>
            <Input id="reg-owner" value={owner} onChange={(e) => setOwner(e.target.value)} placeholder="team@example" />
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="reg-description">
              Description <span className="font-normal text-muted-foreground">(optional)</span>
            </Label>
            <Textarea
              id="reg-description"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="What the table holds and who uses it"
              rows={3}
            />
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
