import { useState } from "react";
import { Braces, TableProperties } from "lucide-react";
import { Button } from "@/components/ui/button";
import { CodeBlock } from "@/components/code-block";
import { EmptyState } from "@/components/empty-state";
import { formatArrowType } from "@/lib/arrow";
import type { JsonArrowSchema } from "@/lib/schemas";

/** Spec `JsonArrowSchema` viewer with a Table / Raw toggle. */
export function SchemaView({ schema }: { schema: JsonArrowSchema }) {
  const [mode, setMode] = useState<"table" | "raw">("table");

  return (
    <div className="space-y-3">
      <div className="inline-flex rounded-md border p-0.5">
        <Button
          size="sm"
          variant={mode === "table" ? "secondary" : "ghost"}
          onClick={() => setMode("table")}
        >
          <TableProperties className="h-4 w-4" /> Table
        </Button>
        <Button
          size="sm"
          variant={mode === "raw" ? "secondary" : "ghost"}
          onClick={() => setMode("raw")}
        >
          <Braces className="h-4 w-4" /> Raw
        </Button>
      </div>

      {mode === "raw" ? (
        <CodeBlock>{JSON.stringify(schema, null, 2)}</CodeBlock>
      ) : schema.fields.length === 0 ? (
        <EmptyState>No fields.</EmptyState>
      ) : (
        <div className="overflow-x-auto rounded-md border">
          <table className="w-full text-sm">
            <thead className="border-b bg-muted/40 text-left text-xs uppercase tracking-wide text-muted-foreground">
              <tr>
                <th className="px-3 py-2">Field</th>
                <th className="px-3 py-2">Type</th>
                <th className="px-3 py-2">Nullable</th>
              </tr>
            </thead>
            <tbody>
              {schema.fields.map((f) => (
                <tr key={f.name} className="border-b last:border-0 align-top">
                  <td className="px-3 py-2 font-mono text-xs break-all">{f.name}</td>
                  <td className="px-3 py-2 font-mono text-xs break-all text-muted-foreground">
                    {formatArrowType(f.type)}
                  </td>
                  <td className="px-3 py-2 text-muted-foreground">{f.nullable ? "yes" : "no"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
