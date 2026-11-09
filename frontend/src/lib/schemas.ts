import { z } from "zod";

// Zod schemas mirroring catalog-core/src/types.rs serde shapes (snake_case).
// Runtime-validate every API response at the boundary; TS types inferred from these.

export const auxFormatSchema = z.enum([
  "parquet",
  "lance",
  "csv",
  "mixed",
  "unknown",
]);
export type AuxFormat = z.infer<typeof auxFormatSchema>;

export const versionShapeSchema = z.enum([
  "full",
  "lance_only",
  "seg_only",
  "lance_only_partial",
  "empty",
]);
export type VersionShape = z.infer<typeof versionShapeSchema>;

export const auxEntrySchema = z.object({
  name: z.string(),
  path: z.string().default(""),
  format: auxFormatSchema,
  role: z.string().default(""),
  storage_bytes: z.number().default(0),
  fingerprint: z.string().nullable().default(null),
  category: z.string().nullable().default(null),
  dataset_path: z.string().nullable().default(null),
  row_count: z.number().nullable().default(null),
  schema_json: z.string().nullable().default(null),
  lance_version: z.number().nullable().default(null),
  writer_version: z.string().nullable().default(null),
});
export type AuxEntry = z.infer<typeof auxEntrySchema>;

export const auxSampleResponseSchema = z.object({
  table_id: z.string(),
  version_id: z.string(),
  aux_name: z.string(),
  format: auxFormatSchema,
  schema: z.array(z.record(z.string(), z.unknown())).default([]),
  rows: z.array(z.record(z.string(), z.unknown())).default([]),
});
export type AuxSampleResponse = z.infer<typeof auxSampleResponseSchema>;

export const tableVersionSchema = z.object({
  version_id: z.string(),
  timestamp: z.string(),
  snapshot_path: z.string(),
  shape: versionShapeSchema,
  partial: z.boolean().default(false),
  storage_bytes_total: z.number().default(0),
  lance_core_bytes: z.number().default(0),
  sidecar_bytes: z.number().default(0),
  segments_bytes: z.number().default(0),
  other_aux_bytes: z.number().default(0),
  row_count: z.number().nullable().default(null),
  num_fragments: z.number().nullable().default(null),
  schema_json: z.string().nullable().default(null),
  num_indices: z.number().nullable().default(null),
  lance_version: z.number().nullable().default(null),
  writer_version: z.string().nullable().default(null),
  aux: z.array(auxEntrySchema).default([]),
  synced_at: z.string(),
});
export type TableVersion = z.infer<typeof tableVersionSchema>;

export const tableEntrySchema = z.object({
  id: z.string(),
  name: z.string(),
  region: z.string().default(""),
  bucket: z.string().default(""),
  namespace: z.array(z.string()),
  root_location: z.string(),
  owner: z.string().nullable(),
  last_synced: z.string().nullable(),
  versions: z.array(tableVersionSchema),
  aux_latest: z.array(auxEntrySchema),
});
export type TableEntry = z.infer<typeof tableEntrySchema>;

export const errorResponseSchema = z.object({
  error_code: z.number(),
  message: z.string(),
});
export type ErrorResponse = z.infer<typeof errorResponseSchema>;

// Namespaces
export const namespaceRefSchema = z.object({
  bucket: z.string(),
  namespace: z.array(z.string()),
});
export type NamespaceRef = z.infer<typeof namespaceRefSchema>;

export const listNamespacesResponseSchema = z.object({
  namespaces: z.array(namespaceRefSchema),
});
export type ListNamespacesResponse = z.infer<typeof listNamespacesResponseSchema>;

export const describeNamespaceResponseSchema = z.object({
  namespace: z.array(z.string()),
  table_count: z.number(),
});
export type DescribeNamespaceResponse = z.infer<typeof describeNamespaceResponseSchema>;
