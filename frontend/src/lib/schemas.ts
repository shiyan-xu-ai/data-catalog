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

export const createNamespaceResponseSchema = z.object({
  properties: propertiesSchema.nullish(),
});
export type CreateNamespaceResponse = z.infer<typeof createNamespaceResponseSchema>;

export const dropNamespaceResponseSchema = z.object({
  properties: propertiesSchema.nullish(),
});
export type DropNamespaceResponse = z.infer<typeof dropNamespaceResponseSchema>;

export const registerTableResponseSchema = z.object({
  location: z.string().nullish(),
  properties: propertiesSchema.nullish(),
});
export type RegisterTableResponse = z.infer<typeof registerTableResponseSchema>;

/** Catalog route `POST /catalog/v1/table/{id}/sql` (not a spec model). Row values follow
 *  Arrow's JSON writer, so they stay `unknown` until formatted per column type. */
export const sqlResponseSchema = z.object({
  columns: z.array(z.object({ name: z.string(), type: z.string() })),
  rows: z.array(z.array(z.unknown())),
  row_count: z.number(),
  truncated: z.boolean(),
  elapsed_ms: z.number(),
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
  /** Detailed describe only: Lance version summary, `config.*`, `lance.data_storage_version`. */
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

/** Catalog route `GET /catalog/v1/table/{id}/storage` (not a spec model). */
export const tableStorageResponseSchema = z.object({
  version: z.number(),
  current: z.object({ data_bytes: z.number(), index_bytes: z.number() }),
  on_storage: z.object({
    total_bytes: z.number(),
    total_objects: z.number(),
    categories: z.array(z.object({ name: z.string(), bytes: z.number(), objects: z.number() })),
  }),
  pricing: z.object({
    price_per_gb_month: z.number(),
    currency: z.string(),
    tier: z.string(),
    estimated_monthly_cost: z.number(),
  }),
});
export type TableStorageResponse = z.infer<typeof tableStorageResponseSchema>;
