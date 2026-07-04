import "./style.css";
import { describeTable, listTables, ttlApply, ttlAudit, ttlDryRun } from "./api";
import { formatBytes, formatTime, nsToString, ttlPolicyString } from "./format";
import type { TableEntry, TtlAuditRecord, TtlDryRunResponse } from "./types";

const app = document.getElementById("app")!;

// ---- hash-based router -----------------------------------------------------

interface Route {
  view: "list" | "table";
  tableId?: string;
}

function parseHash(): Route {
  const h = window.location.hash.replace(/^#/, "");
  if (h.startsWith("/table/")) {
    return { view: "table", tableId: decodeURIComponent(h.slice("/table/".length)) };
  }
  return { view: "list" };
}

window.addEventListener("hashchange", render);

// ---- error handling --------------------------------------------------------

function isNotLeader(msg: string): boolean {
  return msg.startsWith("503");
}

function errorMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  return String(e);
}

// ---- list view --------------------------------------------------------------

async function renderList(): Promise<void> {
  app.innerHTML = `<h1>Data Catalog</h1><p>Loading tables…</p>`;
  let tables: TableEntry[];
  try {
    tables = await listTables();
  } catch (e) {
    app.innerHTML = `<h1>Data Catalog</h1><p class="error">Failed to load: ${errorMessage(e)}</p>`;
    return;
  }
  tables.sort((a, b) => a.id.localeCompare(b.id));

  const rows = tables
    .map((t) => {
      const totalBytes = t.versions.reduce((s, v) => s + v.storage_bytes_total, 0);
      const ns = nsToString(t.namespace);
      const owner = t.owner ?? "—";
      const lastSwept = formatTime(t.last_swept);
      const policy = ttlPolicyString(t.ttl_policy);
      return `<tr>
        <td><a href="#/table/${encodeURIComponent(t.id)}">${esc(t.id)}</a></td>
        <td>${esc(ns)}</td>
        <td>${esc(owner)}</td>
        <td class="num">${t.versions.length}</td>
        <td class="num">${formatBytes(totalBytes)}</td>
        <td>${esc(lastSwept)}</td>
        <td>${esc(policy)}</td>
      </tr>`;
    })
    .join("");

  app.innerHTML = `<h1>Data Catalog</h1>
    <table>
      <thead><tr>
        <th>Name</th><th>Namespace</th><th>Owner</th>
        <th>#Versions</th><th>Total bytes</th><th>Last swept</th><th>TTL policy</th>
      </tr></thead>
      <tbody>${rows}</tbody>
    </table>`;
}

// ---- table drill-down view -------------------------------------------------

async function renderTable(id: string): Promise<void> {
  app.innerHTML = `<h1>${esc(id)}</h1><p>Loading…</p><p><a href="#/">← Back to list</a></p>`;
  let table: TableEntry;
  try {
    table = await describeTable(id);
  } catch (e) {
    app.innerHTML = `<h1>${esc(id)}</h1>
      <p class="error">Failed to load: ${errorMessage(e)}</p>
      <p><a href="#/">← Back to list</a></p>`;
    return;
  }

  const totalBytes = table.versions.reduce((s, v) => s + v.storage_bytes_total, 0);
  const header = `<h1>${esc(table.id)}</h1>
    <p><a href="#/">← Back to list</a></p>
    <dl class="meta">
      <dt>Namespace</dt><dd>${esc(nsToString(table.namespace))}</dd>
      <dt>Root location</dt><dd><code>${esc(table.root_location)}</code></dd>
      <dt>Owner</dt><dd>${esc(table.owner ?? "—")}</dd>
      <dt>Last swept</dt><dd>${esc(formatTime(table.last_swept))}</dd>
      <dt>TTL policy</dt><dd>${esc(ttlPolicyString(table.ttl_policy))}</dd>
      <dt>Versions</dt><dd>${table.versions.length}</dd>
      <dt>Total bytes</dt><dd>${formatBytes(totalBytes)} (logical/deduped — physical S3 footprint may be larger)</dd>
    </dl>`;

  const versionsRows = table.versions
    .slice()
    .sort((a, b) => b.timestamp.localeCompare(a.timestamp))
    .map((v) => {
      const breakdown = [
        `core ${formatBytes(v.lance_core_bytes)}`,
        `sidecar ${formatBytes(v.sidecar_bytes)}`,
        `segments ${formatBytes(v.segments_bytes)}`,
        `other_aux ${formatBytes(v.other_aux_bytes)}`,
      ].join(" · ");
      const flags = [v.partial ? "partial" : "", v.protected ? "protected" : ""]
        .filter(Boolean)
        .join(" ");
      return `<tr>
        <td><code>${esc(v.version_id)}</code></td>
        <td>${esc(v.shape)}${flags ? ` <span class="flag">${esc(flags)}</span>` : ""}</td>
        <td class="num">${formatBytes(v.storage_bytes_total)}<div class="sub">${esc(breakdown)}</div></td>
        <td class="num">${v.row_count ?? "—"}</td>
        <td class="num">${v.num_fragments ?? "—"}</td>
        <td class="num">${v.num_indices ?? "—"}</td>
        <td>${esc(formatTime(v.swept_at))}</td>
      </tr>`;
    })
    .join("");

  const auxRows = table.aux_latest
    .map(
      (a) =>
        `<tr>
          <td>${esc(a.name)}</td>
          <td>${esc(a.format)}</td>
          <td class="num">${formatBytes(a.storage_bytes)}</td>
          <td>${esc(a.role)}</td>
        </tr>`,
    )
    .join("");

  app.innerHTML =
    header +
    `<h2>Versions</h2>
      <table>
        <thead><tr>
          <th>Version ID</th><th>Shape</th><th>Storage</th>
          <th>Rows</th><th>Fragments</th><th>Indices</th><th>Swept at</th>
        </tr></thead>
        <tbody>${versionsRows || emptyRow(7)}</tbody>
      </table>
      <h2>Aux (latest version)</h2>
      <table>
        <thead><tr><th>Name</th><th>Format</th><th>Size</th><th>Role</th></tr></thead>
        <tbody>${auxRows || emptyRow(4)}</tbody>
      </table>
      <h2>TTL</h2>
      <div id="ttl">${ttlSection(table)}</div>`;

  wireTtlActions(table.id);
}

function ttlSection(table: TableEntry): string {
  const policy = ttlPolicyString(table.ttl_policy);
  return `
    <dl class="meta">
      <dt>Policy</dt><dd>${esc(policy)}</dd>
    </dl>
    <div class="ttl-actions">
      <button id="ttl-dryrun">Dry run</button>
      <button id="ttl-apply" disabled>Apply (hard delete)</button>
      <button id="ttl-audit">Load audit log</button>
    </div>
    <div id="ttl-result"></div>
    <div id="ttl-audit-result"></div>`;
}

function wireTtlActions(tableId: string): void {
  const result = document.getElementById("ttl-result")!;
  const auditResult = document.getElementById("ttl-audit-result")!;
  const dryRunBtn = document.getElementById("ttl-dryrun") as HTMLButtonElement | null;
  const applyBtn = document.getElementById("ttl-apply") as HTMLButtonElement | null;
  const auditBtn = document.getElementById("ttl-audit") as HTMLButtonElement | null;

  let lastDryRun: TtlDryRunResponse | null = null;

  dryRunBtn?.addEventListener("click", async () => {
    result.innerHTML = "<p>Running dry-run…</p>";
    try {
      lastDryRun = await ttlDryRun(tableId);
      const cands = lastDryRun.candidates.length
        ? lastDryRun.candidates.map((c) => `<li><code>${esc(c)}</code></li>`).join("")
        : "<li>No eligible versions.</li>";
      result.innerHTML = `
        <p><strong>Dry-run result.</strong> Reclaimable (logical): ${formatBytes(lastDryRun.reclaimable_bytes)}.</p>
        <p>Candidate versions (${lastDryRun.candidates.length}):</p>
        <ul>${cands}</ul>`;
      if (applyBtn) applyBtn.disabled = lastDryRun.candidates.length === 0;
    } catch (e) {
      result.innerHTML = `<p class="error">Dry-run failed: ${errorMessage(e)}</p>`;
    }
  });

  applyBtn?.addEventListener("click", () => {
    if (lastDryRun === null) {
      result.innerHTML = `<p class="error">Run a dry-run first.</p>`;
      return;
    }
    if (lastDryRun.candidates.length === 0) {
      result.innerHTML = `<p>No eligible versions to delete.</p>`;
      return;
    }
    promptConfirm(
      `This will HARD-DELETE ${lastDryRun.candidates.length} version(s) from S3. This is irreversible.\n\nType the table name to confirm:\n${tableId}`,
      (typed) => typed === tableId,
      async () => {
        result.innerHTML = "<p>Applying (deleting versions)…</p>";
        try {
          const r = await ttlApply(tableId);
          result.innerHTML = `
            <p><strong>Applied.</strong> Deleted ${r.deleted.length} version(s), reclaimed ${formatBytes(r.reclaimed_bytes)} (logical).</p>
            <ul>${r.deleted.map((d) => `<li><code>${esc(d)}</code></li>`).join("")}</ul>`;
          if (applyBtn) applyBtn.disabled = true;
        } catch (e) {
          const msg = errorMessage(e);
          if (isNotLeader(msg)) {
            result.innerHTML = `<p class="error">Not the leader pod — retry (a leader pod will accept the write).</p>`;
          } else {
            result.innerHTML = `<p class="error">Apply failed: ${msg}</p>`;
          }
        }
      },
    );
  });

  auditBtn?.addEventListener("click", async () => {
    auditResult.innerHTML = "<p>Loading audit log…</p>";
    try {
      const records: TtlAuditRecord[] = await ttlAudit(tableId);
      if (records.length === 0) {
        auditResult.innerHTML = "<p>No audit records for this table.</p>";
        return;
      }
      const rows = records
        .map(
          (r) =>
            `<tr>
              <td><code>${esc(r.version_id)}</code></td>
              <td>${esc(formatTime(r.deleted_at))}</td>
              <td class="num">${formatBytes(r.reclaimed_bytes)}</td>
              <td>${esc(ttlPolicyString(r.policy_snapshot))}</td>
              <td>${esc(r.actor)}</td>
            </tr>`,
        )
        .join("");
      auditResult.innerHTML = `<table>
        <thead><tr><th>Version</th><th>Deleted at</th><th>Reclaimed</th><th>Policy snapshot</th><th>Actor</th></tr></thead>
        <tbody>${rows}</tbody>
      </table>`;
    } catch (e) {
      auditResult.innerHTML = `<p class="error">Audit load failed: ${errorMessage(e)}</p>`;
    }
  });
}

// Type-to-confirm gate for irreversible ops.
function promptConfirm(
  message: string,
  check: (typed: string) => boolean,
  onConfirm: () => void,
): void {
  const typed = window.prompt(message);
  if (typed === null) return;
  if (!check(typed)) {
    resultEl().innerHTML = `<p class="error">Confirmation text did not match; apply aborted.</p>`;
    return;
  }
  onConfirm();
}

function resultEl(): HTMLElement {
  return document.getElementById("ttl-result")!;
}

// ---- helpers ---------------------------------------------------------------

function emptyRow(cols: number): string {
  return `<tr><td colspan="${cols}" class="empty">— no entries —</td></tr>`;
}

function esc(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

// ---- render dispatch --------------------------------------------------------

async function render(): Promise<void> {
  const route = parseHash();
  if (route.view === "table" && route.tableId) {
    await renderTable(route.tableId);
  } else {
    await renderList();
  }
}

render();
