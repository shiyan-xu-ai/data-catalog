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

export const deregisterTableResponseSchema = z.object({
  id: z.array(z.string()).nullish(),
  location: z.string().nullish(),
  properties: propertiesSchema.nullish(),
});
export type DeregisterTableResponse = z.infer<typeof deregisterTableResponseSchema>;

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
});
export type DescribeTableResponse = z.infer<typeof describeTableResponseSchema>;
