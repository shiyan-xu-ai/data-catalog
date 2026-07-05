import { QueryClient } from "@tanstack/react-query";

// Sweep freshness is bounded by the sweep interval; align staleTime so the UI never pretends to
// be more current than the catalog is. 30s matches a typical sweep cadence.
export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 30_000,
      refetchOnWindowFocus: false,
      retry: (failureCount, error) => {
        // Don't retry on 4xx — these are deterministic (404 not found, 400 bad id).
        if (error instanceof Error && "status" in error) {
          const status = (error as { status: number }).status;
          if (status >= 400 && status < 500) return false;
        }
        return failureCount < 2;
      },
    },
  },
});
