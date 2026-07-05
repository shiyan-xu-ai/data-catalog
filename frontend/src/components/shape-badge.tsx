import { Badge } from "@/components/ui/badge";
import { shapeLabel } from "@/lib/format";
import type { VersionShape } from "@/lib/schemas";

const variantByShape: Record<VersionShape, "success" | "secondary" | "warning" | "destructive" | "outline"> = {
  full: "success",
  lance_only: "secondary",
  seg_only: "warning",
  lance_only_partial: "warning",
  empty: "destructive",
};

export function ShapeBadge({ shape }: { shape: VersionShape }) {
  return <Badge variant={variantByShape[shape]}>{shapeLabel(shape)}</Badge>;
}
