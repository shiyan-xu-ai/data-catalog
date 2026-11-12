import { cn } from "@/lib/utils";

/** The Lance logo, marking a Lance table; size it with `className` like a lucide icon. */
export function LanceIcon({ className }: { className?: string }) {
  return <img src="/lance-logo.png" alt="" aria-hidden="true" className={cn("rounded-[3px]", className)} />;
}
