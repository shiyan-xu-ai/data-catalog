/** Identifier (column, index, table name) that wraps only after `_`, never mid-word. The full
 *  name is the title, for names too long to fit even when wrapped. */
export function Identifier({ name, className }: { name: string; className?: string }) {
  const parts = name.split("_");
  return (
    <span className={className} style={{ wordBreak: "keep-all", overflowWrap: "normal" }} title={name}>
      {parts.map((part, i) => (
        <span key={i}>
          {part}
          {i < parts.length - 1 && (
            <>
              _<wbr />
            </>
          )}
        </span>
      ))}
    </span>
  );
}
