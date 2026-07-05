import { AlertTriangle } from "lucide-react";
import { cn } from "@/lib/utils";

export function ErrorState({ message, className }: { message: string; className?: string }) {
  return (
    <div className={cn("flex items-center gap-2 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive", className)}>
      <AlertTriangle className="h-4 w-4 shrink-0" />
      <span className="break-words">{message}</span>
    </div>
  );
}
