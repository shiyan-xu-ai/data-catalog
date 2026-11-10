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
