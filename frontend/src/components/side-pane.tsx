import { useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";

/** One `label: value` fact in a pane section. */
export function Fact({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-1 text-sm">
      <span className="shrink-0 text-xs text-muted-foreground">{label}</span>
      <span className="min-w-0">{children}</span>
    </div>
  );
}

export function Mono({ children }: { children: React.ReactNode }) {
  return <code className="rounded bg-muted px-1 py-0.5 font-mono text-xs wrap-break-word">{children}</code>;
}

/** One labeled section of a side pane. */
export function PaneSection({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="space-y-1.5 border-t px-4 py-3">
      <h4 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">{title}</h4>
      {children}
    </section>
  );
}

/** Detail pane beside a list: sticky on wide screens (`top` px below the scroll area's top), an
 *  overlay on narrow ones. `toolbar` sits under the header; `wide` doubles as room for raw
 *  content. Place it as the last child of a `flex items-start` row. */
export function SidePane({
  label,
  header,
  toolbar,
  wide = false,
  onClose,
  top = 0,
  children,
}: {
  label: string;
  header: React.ReactNode;
  toolbar?: React.ReactNode;
  wide?: boolean;
  onClose: () => void;
  top?: number;
  children: React.ReactNode;
}) {
  const ref = useRef<HTMLElement>(null);
  const [maxHeight, setMaxHeight] = useState<number>();
  // Fit the sticky pane to the page's scroll area, which shrinks as the SQL dock grows.
  useEffect(() => {
    const main = ref.current?.closest("main");
    if (!main) return;
    const observer = new ResizeObserver(() => setMaxHeight(main.clientHeight - 32 - top));
    observer.observe(main);
    return () => observer.disconnect();
  }, [top]);

  return (
    <>
      <div aria-hidden className="fixed inset-0 z-40 bg-black/30 xl:hidden" onClick={onClose} />
      <aside
        ref={ref}
        aria-label={label}
        style={
          {
            "--pane-max": maxHeight ? `${maxHeight}px` : "100%",
            "--pane-top": `${top + 16}px`,
          } as React.CSSProperties
        }
        className={cn(
          "fixed inset-y-0 right-0 z-50 flex w-full flex-col overflow-hidden border-l bg-background shadow-xl xl:sticky xl:top-[calc(var(--tabs-h,0px)+var(--pane-top))] xl:z-10 xl:max-h-[calc(var(--pane-max)-var(--tabs-h,0px))] xl:shrink-0 xl:self-start xl:rounded-lg xl:border xl:shadow-sm",
          wide ? "max-w-[720px] xl:w-[720px]" : "max-w-[420px] xl:w-[420px]",
        )}
      >
        <div className="flex items-start gap-3 px-4 pt-3 pb-2">
          <div className="min-w-0 flex-1">{header}</div>
          <Button variant="ghost" size="icon" className="h-7 w-7" onClick={onClose} aria-label="Close details">
            <X />
          </Button>
        </div>
        {toolbar}
        <div className="min-h-0 flex-1 overflow-y-auto">{children}</div>
      </aside>
    </>
  );
}
