import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { cn } from "@/lib/utils";

/** Header cell of a `ScrollTable`. The top rule closes the card's border once its top scrolls
 *  away. */
export const HEAD_CELL =
  "h-10 bg-muted px-2 text-left align-middle text-sm font-medium whitespace-nowrap text-muted-foreground shadow-[inset_0_1px_0_hsl(var(--border)),inset_0_-1px_0_hsl(var(--border))]";

interface ColumnLayout {
  width: number;
  cols: number[];
}

function sameLayout(a: ColumnLayout | null, b: ColumnLayout): boolean {
  return a != null && a.width === b.width && a.cols.length === b.cols.length && a.cols.every((w, i) => w === b.cols[i]);
}

/** A table that scrolls sideways within its container while its header row stays pinned below the
 *  tab bar and `--sticky-top` (the height of any pinned element above it) as the page scrolls. A
 *  scroll wrapper would scope `sticky` to itself, so the header renders twice: collapsed inside
 *  the scrolled table, where it still sizes the columns, and as a pinned copy that takes the
 *  measured column widths and follows the horizontal scroll. `head` is one header `<tr>` (cells
 *  styled with `HEAD_CELL`); `children` is the `<tbody>`. */
export function ScrollTable({ head, className, children }: { head: ReactNode; className?: string; children: ReactNode }) {
  const pinned = useRef<HTMLDivElement>(null);
  const table = useRef<HTMLTableElement>(null);
  const [layout, setLayout] = useState<ColumnLayout | null>(null);

  // Column widths follow content and the container, so measure after every render and resize.
  useLayoutEffect(() => {
    const el = table.current;
    const cells = el?.tHead?.rows[0]?.cells;
    if (!el || !cells) return;
    const next = { width: el.getBoundingClientRect().width, cols: Array.from(cells, (c) => c.getBoundingClientRect().width) };
    setLayout((prev) => (sameLayout(prev, next) ? prev : next));
  });
  const [, setResized] = useState(0);
  useLayoutEffect(() => {
    const el = table.current;
    if (!el) return;
    const observer = new ResizeObserver(() => setResized((n) => n + 1));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  return (
    <div>
      {/* The 1px overlap hides sub-pixel seams at fractional zoom. */}
      <div ref={pinned} className="sticky top-[calc(var(--tabs-h,0px)+var(--sticky-top,0px)-1px)] z-10 overflow-hidden">
        <table className={cn(className, "table-fixed")} style={layout ? { width: layout.width } : undefined}>
          {layout && (
            <colgroup>
              {layout.cols.map((w, i) => (
                <col key={i} style={{ width: w }} />
              ))}
            </colgroup>
          )}
          <thead>{head}</thead>
        </table>
      </div>
      <div
        className="overflow-x-auto"
        onScroll={(e) => {
          if (pinned.current) pinned.current.scrollLeft = e.currentTarget.scrollLeft;
        }}
      >
        <table ref={table} className={className}>
          <thead aria-hidden className="[visibility:collapse]">
            {head}
          </thead>
          {children}
        </table>
      </div>
    </div>
  );
}
