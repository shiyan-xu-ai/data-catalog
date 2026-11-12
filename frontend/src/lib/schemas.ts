import { z } from "zod";

// Zod schemas for the Lance REST Catalog spec models (`lance_namespace::models`, snake_case).
// Optional spec fields may arrive absent or null. Runtime-validate every response at the
// boundary; TS types are inferred from these.

const propertiesSchema = z.record(z.string(), z.string());

export const errorResponseSchema = z.object({
  error: z.string().nullish(),
  code: z.number(),
});
export type ErrorResponse = z.infer<typeof errorResponseSchema>;

/** `ErrorCode` values the UI branches on. */
export const ErrorCode = {
  Unsupported: 0,
  NamespaceNotFound: 1,
  NamespaceAlreadyExists: 2,
  NamespaceNotEmpty: 3,
  TableNotFound: 4,
  TableAlreadyExists: 5,
  TableVersionNotFound: 11,
  InvalidInput: 13,
  Internal: 18,
  Throttling: 21,
} as const;

export const listNamespacesResponseSchema = z.object({
  namespaces: z.array(z.string()),
  page_token: z.string().nullish(),
});
export type ListNamespacesResponse = z.infer<typeof listNamespacesResponseSchema>;

export const listTablesResponseSchema = z.object({
  tables: z.array(z.string()),
  page_token: z.string().nullish(),
});
export type ListTablesResponse = z.infer<typeof listTablesResponseSchema>;

export const describeNamespaceResponseSchema = z.object({
  properties: propertiesSchema.nullish(),
});
export type DescribeNamespaceResponse = z.infer<typeof describeNamespaceResponseSchema>;

/** Catalog route `POST /catalog/v1/table/{id}/sql` (not a spec model). Row values follow
 *  Arrow's JSON writer, so they stay `unknown` until formatted per column type. `version` is
 *  the table version the plain table name read. */
export const sqlResponseSchema = z.object({
  columns: z.array(z.object({ name: z.string(), type: z.string() })),
  rows: z.array(z.array(z.unknown())),
  row_count: z.number(),
  truncated: z.boolean(),
  elapsed_ms: z.number(),
  version: z.number().nullish(),
});
export type SqlResponse = z.infer<typeof sqlResponseSchema>;

export interface JsonArrowDataType {
  type: string;
  fields?: JsonArrowField[] | null;
  length?: number | null;
}

export interface JsonArrowField {
  name: string;
  type: JsonArrowDataType;
  nullable: boolean;
  metadata?: Record<string, string> | null;
}

export const jsonArrowDataTypeSchema: z.ZodType<JsonArrowDataType> = z.lazy(() =>
  z.object({
    type: z.string(),
    fields: z.array(jsonArrowFieldSchema).nullish(),
    length: z.number().nullish(),
  }),
);

export const jsonArrowFieldSchema: z.ZodType<JsonArrowField> = z.lazy(() =>
  z.object({
    name: z.string(),
    type: jsonArrowDataTypeSchema,
    nullable: z.boolean(),
    metadata: propertiesSchema.nullish(),
  }),
);

export const jsonArrowSchemaSchema = z.object({
  fields: z.array(jsonArrowFieldSchema),
  metadata: propertiesSchema.nullish(),
});
export type JsonArrowSchema = z.infer<typeof jsonArrowSchemaSchema>;

export const tableStatsSchema = z.object({
  num_deleted_rows: z.number(),
  num_fragments: z.number(),
});
export type TableStats = z.infer<typeof tableStatsSchema>;

export const describeTableResponseSchema = z.object({
  table: z.string().nullish(),
  namespace: z.array(z.string()).nullish(),
  location: z.string().nullish(),
  table_uri: z.string().nullish(),
  properties: propertiesSchema.nullish(),
  version: z.number().nullish(),
  schema: jsonArrowSchemaSchema.nullish(),
  stats: tableStatsSchema.nullish(),
  /** Detailed describe only: Lance version summary, `table_metadata.*`, `config.*`, `lance.data_storage_version`. */
  metadata: propertiesSchema.nullish(),
});
export type DescribeTableResponse = z.infer<typeof describeTableResponseSchema>;

export const tableVersionSchema = z.object({
  version: z.number(),
  manifest_path: z.string().nullish(),
  manifest_size: z.number().nullish(),
  e_tag: z.string().nullish(),
  timestamp_millis: z.number().nullish(),
  metadata: propertiesSchema.nullish(),
});
export type TableVersion = z.infer<typeof tableVersionSchema>;

export const listTableVersionsResponseSchema = z.object({
  versions: z.array(tableVersionSchema),
  page_token: z.string().nullish(),
});
export type ListTableVersionsResponse = z.infer<typeof listTableVersionsResponseSchema>;

export const indexContentSchema = z.object({
  index_name: z.string(),
  index_uuid: z.string().nullish(),
  columns: z.array(z.string()),
  status: z.string().nullish(),
  index_type: z.string().nullish(),
  num_indexed_rows: z.number().nullish(),
  num_unindexed_rows: z.number().nullish(),
  size_bytes: z.number().nullish(),
  created_at: z.string().nullish(),
  index_version: z.number().nullish(),
});
export type IndexContent = z.infer<typeof indexContentSchema>;

export const listTableIndicesResponseSchema = z.object({
  indexes: z.array(indexContentSchema),
  page_token: z.string().nullish(),
});
export type ListTableIndicesResponse = z.infer<typeof listTableIndicesResponseSchema>;

// Tag and branch contents use camelCase keys in the spec (`manifestSize`, `parentVersion`).
export const tagContentsSchema = z.object({
  version: z.number(),
  branch: z.string().nullish(),
  manifestSize: z.number().nullish(),
});
export type TagContents = z.infer<typeof tagContentsSchema>;

export const listTableTagsResponseSchema = z.object({
  tags: z.record(z.string(), tagContentsSchema),
  page_token: z.string().nullish(),
});
export type ListTableTagsResponse = z.infer<typeof listTableTagsResponseSchema>;

export const branchContentsSchema = z.object({
  parentBranch: z.string().nullish(),
  parentVersion: z.number().nullish(),
  createAt: z.number().nullish(),
  manifestSize: z.number().nullish(),
  metadata: propertiesSchema.nullish(),
});
export type BranchContents = z.infer<typeof branchContentsSchema>;

export const listTableBranchesResponseSchema = z.object({
  branches: z.record(z.string(), branchContentsSchema),
  page_token: z.string().nullish(),
});
export type ListTableBranchesResponse = z.infer<typeof listTableBranchesResponseSchema>;

/** Catalog route `GET /catalog/v1/table/{id}/column-storage` (not a spec model). */
export const columnStorageResponseSchema = z.object({
  version: z.number(),
  columns: z.array(z.object({ name: z.string(), bytes_on_disk: z.number() })),
});
export type ColumnStorageResponse = z.infer<typeof columnStorageResponseSchema>;

/** Objects under the table location, by category. */
const storageBreakdownSchema = z.object({
  total_bytes: z.number(),
  total_objects: z.number(),
  categories: z.array(z.object({ name: z.string(), bytes: z.number(), objects: z.number() })),
});
export type StorageBreakdown = z.infer<typeof storageBreakdownSchema>;

/** Catalog route `GET /catalog/v1/table/{id}/storage` (not a spec model). `current` breaks down
 *  the objects the current version references; `on_storage` every object under the location. */
export const tableStorageResponseSchema = z.object({
  version: z.number(),
  current: storageBreakdownSchema.extend({ data_bytes: z.number(), index_bytes: z.number() }),
  on_storage: storageBreakdownSchema,
  pricing: z.object({
    price_per_gb_month: z.number(),
    currency: z.string(),
    tier: z.string(),
    estimated_monthly_cost: z.number(),
  }),
});
export type TableStorageResponse = z.infer<typeof tableStorageResponseSchema>;

/** Min, percentiles, mean, and max of one per-fragment measure. */
const distributionSchema = z.object({
  min: z.number(),
  p25: z.number(),
  p50: z.number(),
  mean: z.number(),
  p75: z.number(),
  p99: z.number(),
  max: z.number(),
});
export type Distribution = z.infer<typeof distributionSchema>;

/** Catalog route `GET /catalog/v1/table/{id}/fragment-measures`: per-fragment measures of the
 *  current version from its manifest; a measure is null when some fragment does not record it. */
export const fragmentMeasuresResponseSchema = z.object({
  version: z.number(),
  fragments: z.number(),
  data_files: distributionSchema.nullable(),
  data_bytes: distributionSchema.nullable(),
  deleted_rows: distributionSchema.nullable(),
});
export type FragmentMeasuresResponse = z.infer<typeof fragmentMeasuresResponseSchema>;

/** Spec `GetTableStatsResponse` (`POST /v1/table/{id}/stats`). Fragment lengths are physical
 *  rows per fragment. */
export const tableStatsResponseSchema = z.object({
  total_bytes: z.number(),
  num_rows: z.number(),
  num_indices: z.number(),
  fragment_stats: z.object({
    num_fragments: z.number(),
    num_small_fragments: z.number(),
    lengths: z.object({
      min: z.number(),
      max: z.number(),
      mean: z.number(),
      p25: z.number(),
      p50: z.number(),
      p75: z.number(),
      p99: z.number(),
    }),
  }),
});
export type TableStatsResponse = z.infer<typeof tableStatsResponseSchema>;

/** Catalog `GET /catalog/v1/table/{id}/timeline`: one event per retained version, newest
 *  first. `operation` is the Lance transaction operation name; `details` depends on it. */
export const timelineEventSchema = z.object({
  version: z.number(),
  timestamp_millis: z.number().nullable(),
  operation: z.string(),
  summary: z.record(z.string(), z.string()),
  writer: z.object({ library: z.string(), version: z.string() }).nullable(),
  transaction: z
    .object({ uuid: z.string(), read_version: z.number(), properties: z.record(z.string(), z.string()) })
    .nullable(),
  tags: z.array(z.string()),
  details: z.record(z.string(), z.unknown()),
});
export type TimelineEvent = z.infer<typeof timelineEventSchema>;

export const timelineResponseSchema = z.object({
  events: z.array(timelineEventSchema),
  page_token: z.string().nullish(),
});
export type TimelineResponse = z.infer<typeof timelineResponseSchema>;

/** Catalog `GET /catalog/v1/table/{id}/version/{version}/files`: the files that record one
 *  version, each as its protobuf sections decoded to JSON. `path` is relative to the table
 *  location; a section that does not decode carries `error` instead of `content`. */
export const versionFileSectionSchema = z.object({
  name: z.string(),
  offset: z.number(),
  message: z.string().nullish(),
  content: z.unknown().optional(),
  error: z.string().nullish(),
});
export type VersionFileSection = z.infer<typeof versionFileSectionSchema>;

export const versionFilesResponseSchema = z.object({
  version: z.number(),
  files: z.array(
    z.object({
      path: z.string(),
      size_bytes: z.number(),
      sections: z.array(versionFileSectionSchema),
    }),
  ),
});
export type VersionFilesResponse = z.infer<typeof versionFilesResponseSchema>;
