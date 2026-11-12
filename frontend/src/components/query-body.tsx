import type { UseQueryResult } from "@tanstack/react-query";
import { Skeleton } from "@/components/ui/skeleton";
import { ErrorState } from "@/components/error-state";

/** A query's error, a skeleton while it loads, then `children` with its data. */
export function QueryBody<T>({
  query,
  skeletonClassName,
  children,
}: {
  query: UseQueryResult<T>;
  skeletonClassName: string;
  children: (data: T) => React.ReactNode;
}) {
  if (query.error) return <ErrorState message={query.error.message} />;
  if (query.isPending) return <Skeleton className={skeletonClassName} />;
  return <>{children(query.data)}</>;
}
