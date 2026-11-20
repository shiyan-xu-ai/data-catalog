import { useMemo, useRef, useState } from "react";
import { Download, Loader2 } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { ApiError, presignMedia } from "@/lib/api";
import { formatBytes, formatCount } from "@/lib/format";
import type { ByteRange } from "@/lib/hevc-player";
import { MAX_CLIP_BYTES, clipFileName, fetchClip } from "@/lib/media";
import { ErrorCode, type SqlResponse } from "@/lib/schemas";
import { formatCell, isBlobColumn, shortTypeName, videoSource, type CellKind, type VideoSource } from "@/lib/sql";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { CopyButton } from "@/components/copy-button";
import { ClipVideo } from "@/components/clip-video";
import { HevcPlayer } from "@/components/hevc-player";
import { JsonTree } from "@/components/json-tree";
import { PaneTabs, SidePane, SidePaneLayout, usePaneSelection } from "@/components/side-pane";

const cellClass: Record<CellKind, string> = {
  null: "italic text-muted-foreground/60",
  number: "text-right tabular-nums",
  bool: "text-[hsl(var(--chart-3))]",
  text: "",
  nested: "text-muted-foreground",
  binary: "text-[hsl(var(--chart-4))]",
};

export function errorTitle(e: Error): string {
  if (!(e instanceof ApiError)) return "Request failed";
  if (e.code === ErrorCode.Throttling) return "The server is busy running other queries; try again shortly";
  if (e.code === ErrorCode.TableNotFound) return "Table not found";
  if (e.code === ErrorCode.TableVersionNotFound) return "Version not found";
  if (e.code === ErrorCode.InvalidInput) return "Query rejected";
  return "Query failed";
}

export function formatElapsed(ms: number): string {
  return ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(ms < 10_000 ? 2 : 1)} s`;
}

const rowIndex = (i: number) => i;

function copyText(value: unknown): string {
  if (value == null) return "";
  return typeof value === "object" ? JSON.stringify(value, null, 2) : String(value);
}

type RowView = "values" | "playback";

/** Saves the clip's byte range to a file. */
function SaveClip({ url, source }: { url: string; source: VideoSource & { range: ByteRange } }) {
  const [state, setState] = useState<"idle" | "saving" | "failed">("idle");
  const tooBig = source.range.length != null && source.range.length > MAX_CLIP_BYTES;
  async function save() {
    setState("saving");
    try {
      const href = URL.createObjectURL(await fetchClip(url, source.range));
      const link = document.createElement("a");
      link.href = href;
      link.download = clipFileName(source.url, source.range);
      link.click();
      URL.revokeObjectURL(href);
      setState("idle");
    } catch {
      setState("failed");
    }
  }
  return (
    <Button
      size="sm"
      variant="outline"
      className="ml-auto h-7 shrink-0"
      disabled={tooBig || state === "saving"}
      onClick={save}
      title={tooBig ? "Too large to save from the browser" : state === "failed" ? "Saving failed; try again" : undefined}
    >
      {state === "saving" ? <Loader2 className="animate-spin" /> : <Download />}
      {state === "failed" ? "Retry download" : "Download"}
    </Button>
  );
}

/** One playable column. `s3://` sources are presigned by the catalog and then read from object
 *  storage directly; raw H.265 streams are decoded in the browser, other files use `<video>`.
 *  A source with a byte range (a Lance blob) plays and downloads only those bytes. */
function VideoClip({ name, source }: { name: string; source: VideoSource }) {
  const [failed, setFailed] = useState(false);
  const signed = useQuery({
    queryKey: ["media-url", source.url],
    queryFn: () => presignMedia(source.url),
    enabled: source.presign,
    staleTime: 10 * 60_000,
  });
  const url = source.presign ? signed.data?.url : source.url;
  const downloadUrl = source.presign ? signed.data?.download_url : source.url;
  const { range } = source;
  return (
    <section className="space-y-1.5 border-t px-4 py-3">
      <div className="flex min-w-0 items-center gap-2">
        <span className="truncate font-mono text-xs font-medium" title={source.url}>
          {name}
        </span>
        {url && range ? (
          <SaveClip url={url} source={{ ...source, range }} />
        ) : (
          downloadUrl && (
            <Button asChild size="sm" variant="outline" className="ml-auto h-7 shrink-0">
              <a href={downloadUrl} download>
                <Download />
                Download
              </a>
            </Button>
          )
        )}
      </div>
      {range && (
        <p className="text-[11px] text-muted-foreground tabular-nums">
          {range.length != null
            ? `Blob bytes ${formatCount(range.start)}–${formatCount(range.start + range.length)} (${formatBytes(range.length)})`
            : `Blob bytes from ${formatCount(range.start)} to the end of the object`}
        </p>
      )}
      {source.presign && signed.error ? (
        <p className="text-xs text-destructive">Could not get a URL for this object: {signed.error.message}</p>
      ) : !url ? (
        <Skeleton className="aspect-video w-full" />
      ) : source.rawHevc ? (
        <HevcPlayer key={`${url}:${range?.start}:${range?.length}`} url={url} range={range} />
      ) : failed ? (
        <p className="text-xs text-muted-foreground">This browser cannot play this file. Download it to watch.</p>
      ) : (
        <ClipVideo url={url} range={range} onFailed={() => setFailed(true)} />
      )}
    </section>
  );
}

/** Every column of one result row with its type and full value, nested values as a JSON tree.
 *  When some value is a playable video (see `videoSource`: a Lance blob column or a video URL),
 *  a Playback view shows a player per such column. */
function RowPane({
  result,
  row,
  inline,
  view,
  onView,
  onClose,
}: {
  result: SqlResponse;
  row: number;
  inline: boolean;
  view: RowView;
  onView: (view: RowView) => void;
  onClose: () => void;
}) {
  const values = result.rows[row];
  const videos = result.columns.flatMap((c, i) => {
    const source = videoSource(values[i], isBlobColumn(c));
    return source ? [{ name: c.name, source }] : [];
  });
  const shown = videos.length > 0 ? view : "values";
  return (
    <SidePane
      label={`Row ${row + 1} details`}
      inline={inline}
      onClose={onClose}
      bodyKey={`${row}:${shown}`}
      className={inline ? "mt-3 mr-3" : undefined}
      header={
        <>
          <div className="font-medium">Row {formatCount(row + 1)}</div>
          <div className="text-xs text-muted-foreground">of {formatCount(result.rows.length)} returned</div>
        </>
      }
      toolbar={
        videos.length > 0 && (
          <PaneTabs
            views={[
              ["values", "Values"],
              ["playback", `Playback (${videos.length})`],
            ]}
            value={shown}
            onChange={onView}
          />
        )
      }
    >
      {shown === "playback"
        ? videos.map((v, i) => <VideoClip key={i} name={v.name} source={v.source} />)
        : result.columns.map((c, i) => {
            const cell = formatCell(values[i], c.type);
            return (
              <section key={i} className="space-y-1 border-t px-4 py-2.5">
                <div className="flex min-w-0 items-center gap-2">
                  <span className="truncate font-mono text-xs font-medium" title={c.name}>
                    {c.name}
                  </span>
                  <span className="truncate text-[10px] text-muted-foreground" title={c.type}>
                    {shortTypeName(c.type)}
                  </span>
                  <span className="ml-auto">
                    <CopyButton value={() => copyText(values[i])} label={`Copy ${c.name}`} />
                  </span>
                </div>
                {cell.kind === "nested" ? (
                  <div className="overflow-x-auto">
                    <JsonTree name={c.name} value={values[i]} />
                  </div>
                ) : (
                  <div className={cn("font-mono text-xs break-all whitespace-pre-wrap", cellClass[cell.kind], "text-left")}>
                    {cell.text}
                  </div>
                )}
              </section>
            );
          })}
    </SidePane>
  );
}

/** Results with a header that stays pinned to the top of the surrounding scroll container
 *  (`data-pane-scroll`) while the rows scroll. Wide results scroll sideways in the body; the
 *  header, a separate table with the same column widths, follows that scroll. Selecting a row
 *  opens its values beside the grid when `inlinePane`, and over the page otherwise. */
export function ResultGrid({ result, inlinePane }: { result: SqlResponse; inlinePane: boolean }) {
  const [widths, setWidths] = useState<number[]>([]);
  const head = useRef<HTMLDivElement>(null);
  const indices = useMemo(() => result.rows.map((_, i) => i), [result]);
  const { current, close, rowProps } = usePaneSelection(indices, rowIndex);
  const [view, setView] = useState<RowView>("values");
  const widthAt = (index: number) => widths[index] ?? 220;

  function resizeColumn(index: number, width: number) {
    setWidths((current) => {
      const next = [...current];
      next[index] = Math.max(80, Math.min(10000, width));
      return next;
    });
  }

  function startColumnResize(event: React.PointerEvent<HTMLDivElement>, index: number) {
    event.preventDefault();
    const handle = event.currentTarget;
    const startX = event.clientX;
    const startWidth = widthAt(index);
    handle.setPointerCapture(event.pointerId);
    const onMove = (e: PointerEvent) => resizeColumn(index, startWidth + e.clientX - startX);
    const onEnd = () => {
      handle.removeEventListener("pointermove", onMove);
      handle.removeEventListener("pointerup", onEnd);
      handle.removeEventListener("pointercancel", onEnd);
    };
    handle.addEventListener("pointermove", onMove);
    handle.addEventListener("pointerup", onEnd);
    handle.addEventListener("pointercancel", onEnd);
  }

  if (result.columns.length === 0) {
    return <p className="p-4 text-xs text-muted-foreground">The query returned no columns.</p>;
  }
  const width = 48 + result.columns.reduce((sum, _, i) => sum + widthAt(i), 0);
  const columns = (
    <colgroup>
      <col style={{ width: 48 }} />
      {result.columns.map((_, i) => <col key={i} style={{ width: widthAt(i) }} />)}
    </colgroup>
  );
  const tableClass = "table-fixed border-separate border-spacing-0 font-mono text-xs";
  return (
    <SidePaneLayout
      pane={
        current != null && (
          <RowPane
            result={result}
            row={current}
            inline={inlinePane}
            view={view}
            onView={setView}
            onClose={close}
          />
        )
      }
    >
      <div ref={head} className="sticky -top-px z-10 overflow-hidden">
        <table className={tableClass} style={{ width, minWidth: "100%" }}>
          {columns}
          <thead>
            <tr>
              <th className="border-b bg-muted px-2 py-1.5 text-right font-normal text-muted-foreground">#</th>
              {result.columns.map((c, i) => (
                <th key={i} className="relative overflow-hidden border-b border-l bg-muted px-2 py-1.5 text-left align-bottom" title={`${c.name}: ${c.type}`}>
                  <div className="truncate font-sans text-xs font-semibold text-foreground">{c.name}</div>
                  <div className="truncate text-[10px] font-normal text-muted-foreground">{shortTypeName(c.type)}</div>
                  <div
                    role="separator"
                    aria-label={`Resize column ${c.name}`}
                    aria-orientation="vertical"
                    aria-valuenow={widthAt(i)}
                    aria-valuemin={80}
                    aria-valuemax={10000}
                    aria-valuetext={`${widthAt(i)} pixels`}
                    tabIndex={0}
                    onPointerDown={(e) => startColumnResize(e, i)}
                    onKeyDown={(e) => {
                      if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
                        e.preventDefault();
                        resizeColumn(i, widthAt(i) + (e.key === "ArrowLeft" ? -20 : 20));
                      }
                    }}
                    className="absolute inset-y-0 right-0 w-2 cursor-col-resize touch-none hover:bg-primary/20 focus-visible:bg-primary/20"
                  />
                </th>
              ))}
            </tr>
          </thead>
        </table>
      </div>
      <div
        className="overflow-x-auto"
        onScroll={(e) => {
          if (head.current) head.current.scrollLeft = e.currentTarget.scrollLeft;
        }}
      >
        <table className={tableClass} style={{ width, minWidth: "100%" }}>
          {columns}
          <tbody>
            {result.rows.map((row, r) => (
              <tr
                key={r}
                {...rowProps(r)}
                aria-selected={r === current}
                className={cn(
                  "cursor-pointer outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset",
                  r === current ? "bg-accent" : "hover:bg-muted/40",
                )}
              >
                <td className="border-b px-2 py-1 text-right text-muted-foreground/70 tabular-nums">{r + 1}</td>
                {result.columns.map((c, i) => {
                  const cell = formatCell(row[i], c.type);
                  return (
                    <td
                      key={i}
                      title={cell.title ?? cell.text}
                      className={cn("truncate border-b border-l px-2 py-1 whitespace-nowrap", cellClass[cell.kind])}
                    >
                      {cell.text}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </SidePaneLayout>
  );
}
