import { useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";

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

export function PaneSection({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="space-y-1.5 border-t px-4 py-3">
      <h4 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">{title}</h4>
      {children}
    </section>
  );
}

/** Props spread on a selectable row of a list with a side pane. */
export interface PaneRowProps {
  ref: (el: HTMLElement | null) => void;
  tabIndex: number;
  onClick: () => void;
  onKeyDown: (e: React.KeyboardEvent) => void;
}

/** Keyboard-navigable selection over `items` for a list with a side pane. While an item is
 *  selected, Esc closes the pane and refocuses its row, and ArrowUp/Down move the selection;
 *  keys typed into editable targets (the SQL editor, inputs, dialogs) are left alone. `keyOf`
 *  must be a stable function. */
export function usePaneSelection<T, K>(items: readonly T[], keyOf: (item: T) => K) {
  const [active, setActive] = useState<K | null>(null);
  const rowRefs = useRef(new Map<K, HTMLElement>());
  const index = items.findIndex((item) => keyOf(item) === active);
  const current = index >= 0 ? items[index] : null;

  useEffect(() => {
    if (!current) return;
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (t && (t.isContentEditable || t.closest("input, textarea, select, [role=dialog]"))) return;
      if (e.key === "Escape") {
        e.preventDefault();
        setActive(null);
        rowRefs.current.get(keyOf(current))?.focus();
        return;
      }
      const step = e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : 0;
      const next = items[index + step];
      if (!step || !next) return;
      e.preventDefault();
      setActive(keyOf(next));
      const el = rowRefs.current.get(keyOf(next));
      el?.focus({ preventScroll: true });
      el?.scrollIntoView({ block: "nearest" });
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [current, index, items, keyOf]);

  const close = () => {
    if (current) rowRefs.current.get(keyOf(current))?.focus();
    setActive(null);
  };

  /** Props that make the row for `key` selectable by click, Enter, or Space. `selfOnly` ignores
   *  keys pressed on controls nested in the row. */
  const rowProps = (key: K, { selfOnly = false } = {}): PaneRowProps => ({
    ref: (el: HTMLElement | null) => {
      if (el) rowRefs.current.set(key, el);
      else rowRefs.current.delete(key);
    },
    tabIndex: 0,
    onClick: () => setActive(key),
    onKeyDown: (e: React.KeyboardEvent) => {
      if (selfOnly && e.target !== e.currentTarget) return;
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        setActive(key);
      }
    },
  });

  return { current, index, close, rowProps };
}

/** A list with its side pane beside it; `pane` is null while nothing is selected. */
export function SidePaneLayout({
  pane,
  className,
  children,
}: {
  pane: React.ReactNode;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-start gap-4">
      <div className={cn("min-w-0 flex-1", className)}>{children}</div>
      {pane}
    </div>
  );
}

/** Detail pane beside a list: sticky on wide screens (`top` px below the scroll area's top), an
 *  overlay on narrow ones. `toolbar` sits under the header; `wide` doubles as room for raw
 *  content. A new `bodyKey` remounts the body, resetting its scroll and state, without
 *  remounting the pane. */
export function SidePane({
  label,
  header,
  toolbar,
  wide = false,
  onClose,
  top = 0,
  bodyKey,
  className,
  children,
}: {
  label: string;
  header: React.ReactNode;
  toolbar?: React.ReactNode;
  wide?: boolean;
  onClose: () => void;
  top?: number;
  bodyKey: React.Key;
  className?: string;
  children: React.ReactNode;
}) {
  const ref = useRef<HTMLElement>(null);
  const [maxHeight, setMaxHeight] = useState<number>();
  // Fit the sticky pane to the page's scroll area, which changes with the window.
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
          className,
        )}
      >
        <div className="flex items-start gap-3 px-4 pt-3 pb-2">
          <div className="min-w-0 flex-1">{header}</div>
          <Button variant="ghost" size="icon" className="h-7 w-7" onClick={onClose} aria-label="Close details">
            <X />
          </Button>
        </div>
        {toolbar}
        <div key={bodyKey} className="min-h-0 flex-1 overflow-y-auto">
          {children}
        </div>
      </aside>
    </>
  );
}
