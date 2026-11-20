import { quotePath } from "./sql";

type InsertHandler = (text: string) => void;

let handler: InsertHandler | null = null;

/** Registers where inserted table paths go: the Query page's active editor. Returns an
 *  unregister function that only clears this registration, so a stale cleanup cannot remove a
 *  newer handler. */
export function registerSqlInsert(next: InsertHandler): () => void {
  handler = next;
  return () => {
    if (handler === next) handler = null;
  };
}

/** Inserts a table's quoted full path into the active query editor. Does nothing while the
 *  Query page is not mounted. */
export function insertTablePath(segments: readonly string[]): void {
  handler?.(quotePath(segments));
}
