import { cn } from "@/lib/utils";

export function CodeBlock({ children, className }: { children: string; className?: string }) {
  return (
    <pre
      className={cn(
        "max-h-96 overflow-auto rounded-md border bg-muted/40 p-3 font-mono text-xs leading-relaxed",
        className,
      )}
    >
      <code>{children}</code>
    </pre>
  );
}

export function prettyJson(s: string | null | undefined): string {
  if (!s) return "";
  try {
    return JSON.stringify(JSON.parse(s), null, 2);
  } catch {
    return s;
  }
}
