<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

  interface CorpusInfo {
    id: string;
    paths: string[];
    status: { state: string; files_done?: number; files_total?: number; message?: string };
    files_indexed: number;
    sections_count: number;
    embeddings_count: number;
  }

  interface DaemonStatus {
    version: string;
    uptime_secs: number;
    memory_mb: number;
    model: string;
    model_dimension: number;
    corpora: CorpusInfo[];
  }

  let status: DaemonStatus | null = $state(null);
  let error: string | null = $state(null);

  async function refresh() {
    try {
      status = await invoke<DaemonStatus>("daemon_status");
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  onMount(() => {
    refresh();
    const interval = setInterval(refresh, 2000);
    return () => clearInterval(interval);
  });

  function formatUptime(secs: number): string {
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const s = secs % 60;
    if (h > 0) return `${h}h ${m}m`;
    if (m > 0) return `${m}m ${s}s`;
    return `${s}s`;
  }

  function statusColor(state: string): string {
    if (state === "indexing") return "#f59e0b";
    if (state === "error") return "#ef4444";
    return "#22c55e";
  }
</script>

<main>
  <header>
    <h1>iris</h1>
    {#if status}
      <div class="status-bar">
        <span>v{status.version}</span>
        <span>{status.model} ({status.model_dimension}d)</span>
        <span>{status.memory_mb.toFixed(0)} MB</span>
        <span>up {formatUptime(status.uptime_secs)}</span>
      </div>
    {/if}
  </header>

  {#if error}
    <div class="error">{error}</div>
  {/if}

  {#if status}
    <section class="corpora">
      <h2>Corpora ({status.corpora.length})</h2>
      {#if status.corpora.length === 0}
        <p class="empty">No corpora registered. The MCP server will register corpora automatically.</p>
      {:else}
        <div class="corpus-list">
          {#each status.corpora as corpus}
            <div class="corpus-card">
              <div class="corpus-header">
                <span class="status-dot" style="background: {statusColor(corpus.status.state)}"></span>
                <code class="corpus-id">{corpus.id}</code>
              </div>
              <div class="corpus-paths">
                {#each corpus.paths as path}
                  <div class="path">{path}</div>
                {/each}
              </div>
              <div class="corpus-stats">
                <span>{corpus.files_indexed} files</span>
                <span>{corpus.sections_count} sections</span>
                <span>{corpus.embeddings_count} embeddings</span>
              </div>
              {#if corpus.status.state === "indexing"}
                <div class="progress">
                  Indexing: {corpus.status.files_done}/{corpus.status.files_total} files
                </div>
              {/if}
              {#if corpus.status.state === "error"}
                <div class="error">{corpus.status.message}</div>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </section>
  {:else if !error}
    <p>Loading...</p>
  {/if}
</main>

<style>
  :global(body) {
    margin: 0;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
    background: #0f0f0f;
    color: #e0e0e0;
  }

  main {
    padding: 20px;
    max-width: 800px;
    margin: 0 auto;
  }

  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 24px;
    border-bottom: 1px solid #333;
    padding-bottom: 12px;
  }

  h1 {
    font-size: 1.5rem;
    font-weight: 600;
    color: #fff;
    margin: 0;
  }

  h2 {
    font-size: 1.1rem;
    font-weight: 500;
    color: #ccc;
    margin: 0 0 12px 0;
  }

  .status-bar {
    display: flex;
    gap: 16px;
    font-size: 0.8rem;
    color: #888;
  }

  .error {
    background: #2d1b1b;
    border: 1px solid #ef4444;
    border-radius: 6px;
    padding: 10px 14px;
    font-size: 0.85rem;
    color: #fca5a5;
    margin-bottom: 16px;
  }

  .empty {
    color: #666;
    font-style: italic;
  }

  .corpus-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .corpus-card {
    background: #1a1a1a;
    border: 1px solid #333;
    border-radius: 8px;
    padding: 14px;
  }

  .corpus-header {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
  }

  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .corpus-id {
    font-size: 0.85rem;
    color: #aaa;
  }

  .corpus-paths {
    margin-bottom: 8px;
  }

  .path {
    font-size: 0.8rem;
    color: #888;
    font-family: "SF Mono", "Fira Code", monospace;
    padding: 2px 0;
  }

  .corpus-stats {
    display: flex;
    gap: 16px;
    font-size: 0.8rem;
    color: #666;
  }

  .progress {
    margin-top: 8px;
    font-size: 0.8rem;
    color: #f59e0b;
  }
</style>
