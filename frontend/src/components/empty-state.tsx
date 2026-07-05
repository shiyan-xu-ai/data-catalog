import { cn } from "@/lib/utils";

export function EmptyState({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <div className={cn("py-8 text-center text-sm text-muted-foreground", className)}>{children}</div>
  );
}
