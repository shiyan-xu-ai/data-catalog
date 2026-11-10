import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { createNamespace } from "@/lib/api";
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

interface CreateNamespaceDialogProps {
  parent: readonly string[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

/** Create a child namespace of `parent` (the root when empty). */
export function CreateNamespaceDialog({ parent, open, onOpenChange }: CreateNamespaceDialogProps) {
  const qc = useQueryClient();
  const [name, setName] = useState("");
  const [rows, setRows] = useState<PropertyRow[]>([]);

  const nameError = name ? segmentError(name) : null;
  const props = buildProperties("", rows);
  const canSubmit = name.length > 0 && !nameError && props.ok;

  const mutation = useMutation({
    mutationFn: () => {
      if (!props.ok) throw new Error(props.error);
      return createNamespace([...parent, name], props.properties);
    },
    onSuccess: () => {
      toast.success(`Created namespace "${displayName([...parent, name])}"`);
      qc.invalidateQueries({ queryKey: ["namespaces", joinId(parent)] });
      close();
    },
    onError: (e: unknown) => toast.error((e as Error).message),
  });

  function close() {
    setName("");
    setRows([]);
    onOpenChange(false);
  }

  return (
    <Dialog open={open} onOpenChange={(o) => (o ? onOpenChange(true) : close())}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Create namespace</DialogTitle>
          <DialogDescription>
            {parent.length ? (
              <>
                New child of <span className="font-mono">{displayName(parent)}</span>.
              </>
            ) : (
              "New top-level namespace."
            )}
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
            <Label htmlFor="ns-name">Name</Label>
            <Input id="ns-name" value={name} onChange={(e) => setName(e.target.value)} autoFocus />
            {nameError && <p className="text-xs text-destructive">{nameError}</p>}
          </div>
          <PropertiesEditor rows={rows} onChange={setRows} error={props.ok ? undefined : props.error} />

          <DialogFooter>
            <Button type="button" variant="outline" onClick={close}>Cancel</Button>
            <Button type="submit" disabled={!canSubmit || mutation.isPending}>
              {mutation.isPending ? "Creating…" : "Create"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
