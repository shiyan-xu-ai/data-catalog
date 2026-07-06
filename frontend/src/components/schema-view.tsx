import { useState } from "react";
import { Braces, TableProperties } from "lucide-react";
import { Button } from "@/components/ui/button";
import { CodeBlock, prettyJson } from "@/components/code-block";
import { EmptyState } from "@/components/empty-state";

interface SchemaField {
  name: string;
  data_type: string;
  nullable: boolean;
}

/** Parse the swept schema JSON (`{ fields: [{ name, data_type, nullable }] }`).
 *  Returns null when the payload isn't the expected shape so the caller can fall
 *  back to the raw view. */
function parseFields(json: string): SchemaField[] | null {
  try {
    const parsed = JSON.parse(json) as unknown;
    const fields = (parsed as { fields?: unknown })?.fields;
    if (!Array.isArray(fields)) return null;
    return fields.map((f) => {
      const field = f as Record<string, unknown>;
      return {
        name: String(field.name ?? ""),
        data_type: String(field.data_type ?? ""),
        nullable: Boolean(field.nullable),
      };
    });
  } catch {
    return null;
  }
}

/** Schema viewer with a Table / Raw toggle. Defaults to the table view when the
 *  JSON parses into fields, otherwise shows the raw JSON only. */
export function SchemaView({ schemaJson }: { schemaJson: string }) {
  const fields = parseFields(schemaJson);
  const [mode, setMode] = useState<"table" | "raw">(fields ? "table" : "raw");

  return (
    <div className="space-y-3">
      {fields && (
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
      )}

      {mode === "table" && fields ? (
        fields.length === 0 ? (
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
                {fields.map((f) => (
                  <tr key={f.name} className="border-b last:border-0 align-top">
                    <td className="px-3 py-2 font-mono text-xs break-all">{f.name}</td>
                    <td className="px-3 py-2 font-mono text-xs break-all text-muted-foreground">
                      {f.data_type}
                    </td>
                    <td className="px-3 py-2 text-muted-foreground">{f.nullable ? "yes" : "no"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )
      ) : (
        <CodeBlock>{prettyJson(schemaJson)}</CodeBlock>
      )}
    </div>
  );
}
