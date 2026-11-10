import { Plus, X } from "lucide-react";
import type { PropertyRow } from "@/lib/properties";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

interface PropertiesEditorProps {
  rows: PropertyRow[];
  onChange: (rows: PropertyRow[]) => void;
  error?: string;
}

/** Editable list of extra key/value properties. */
export function PropertiesEditor({ rows, onChange, error }: PropertiesEditorProps) {
  function update(i: number, patch: Partial<PropertyRow>) {
    onChange(rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  }

  return (
    <div className="grid gap-2">
      <div className="flex items-center justify-between">
        <Label>Properties (optional)</Label>
        <Button type="button" size="sm" variant="ghost" onClick={() => onChange([...rows, { key: "", value: "" }])}>
          <Plus /> Add
        </Button>
      </div>
      {rows.map((row, i) => (
        <div key={i} className="flex gap-2">
          <Input
            aria-label={`Property ${i + 1} key`}
            value={row.key}
            onChange={(e) => update(i, { key: e.target.value })}
            placeholder="key"
          />
          <Input
            aria-label={`Property ${i + 1} value`}
            value={row.value}
            onChange={(e) => update(i, { value: e.target.value })}
            placeholder="value"
          />
          <Button
            type="button"
            size="icon"
            variant="ghost"
            aria-label={`Remove property ${i + 1}`}
            onClick={() => onChange(rows.filter((_, j) => j !== i))}
          >
            <X />
          </Button>
        </div>
      ))}
      {error && <p className="text-xs text-destructive">{error}</p>}
    </div>
  );
}
