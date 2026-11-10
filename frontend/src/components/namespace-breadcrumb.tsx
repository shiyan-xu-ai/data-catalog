import { Link } from "@tanstack/react-router";
import { joinId } from "@/lib/ids";

/** `Namespaces / a / b` trail linking each namespace prefix. */
export function NamespaceBreadcrumb({ segments }: { segments: readonly string[] }) {
  return (
    <nav className="flex flex-wrap items-center gap-1 text-sm text-muted-foreground">
      <Link to="/namespaces" className="hover:text-foreground">Namespaces</Link>
      {segments.map((seg, i) => (
        <span key={i} className="flex items-center gap-1">
          <span>/</span>
          <Link
            to="/namespaces/$nsId"
            params={{ nsId: joinId(segments.slice(0, i + 1)) }}
            className="font-mono hover:text-foreground"
          >
            {seg}
          </Link>
        </span>
      ))}
    </nav>
  );
}
