import { createSignal, createEffect, onMount, onCleanup } from "solid-js";
import { createStore, produce } from "solid-js/store";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import type { QueryEvent, ProfilerStatus, ConnectionConfig } from "./lib/types.ts";
import ConnectionForm from "./components/ConnectionForm.tsx";
import Toolbar from "./components/Toolbar.tsx";
import QueryFeed from "./components/QueryFeed.tsx";
import QueryDetail from "./components/QueryDetail.tsx";

export default function App() {
  const [status, setStatus] = createSignal<ProfilerStatus>({
    connected: false,
    capturing: false,
    error: null,
  });
  const [queries, setQueries] = createStore<QueryEvent[]>([]);
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [filterText, setFilterText] = createSignal("");
  const [showConnection, setShowConnection] = createSignal(true);
  const [autoScroll, setAutoScroll] = createSignal(true);

  const selectedQuery = () => queries.find((q) => q.id === selectedId()) ?? null;

  const filteredQueries = () => {
    const filter = filterText().toLowerCase();
    if (!filter) return queries;
    return queries.filter(
      (q) =>
        q.sql_text.toLowerCase().includes(filter) ||
        q.current_statement.toLowerCase().includes(filter) ||
        q.database_name.toLowerCase().includes(filter) ||
        q.login_name.toLowerCase().includes(filter) ||
        q.program_name.toLowerCase().includes(filter)
    );
  };

  onMount(async () => {
    const unlistenQuery = await listen<QueryEvent>("query-event", (event) => {
      const query = event.payload;
      const existingIdx = queries.findIndex((q) => q.id === query.id);
      if (existingIdx >= 0) {
        setQueries(existingIdx, query);
      } else {
        setQueries(produce((draft) => draft.push(query)));
      }
    });

    const unlistenStatus = await listen<ProfilerStatus>(
      "profiler-status",
      (event) => {
        setStatus(event.payload);
        if (event.payload.connected) {
          setShowConnection(false);
        }
      }
    );

    onCleanup(() => {
      unlistenQuery();
      unlistenStatus();
    });
  });

  async function handleConnect(config: ConnectionConfig, rememberPassword: boolean) {
    try {
      setStatus((s) => ({ ...s, error: null }));
      await invoke("connect_to_server", { config, rememberPassword });
    } catch (e) {
      setStatus((s) => ({ ...s, error: String(e) }));
    }
  }

  async function handleDisconnect() {
    try {
      await invoke("disconnect_from_server");
      setStatus({ connected: false, capturing: false, error: null });
      setShowConnection(true);
    } catch (e) {
      setStatus((s) => ({ ...s, error: String(e) }));
    }
  }

  async function handleStartCapture() {
    try {
      await invoke("start_capture");
    } catch (e) {
      setStatus((s) => ({ ...s, error: String(e) }));
    }
  }

  async function handleStopCapture() {
    try {
      await invoke("stop_capture");
    } catch (e) {
      setStatus((s) => ({ ...s, error: String(e) }));
    }
  }

  function handleClear() {
    setQueries([]);
    setSelectedId(null);
  }

  return (
    <div class="h-screen flex flex-col bg-slate-950">
      {/* Title Bar */}
      <header class="flex items-center justify-between px-4 py-2 bg-slate-900 border-b border-slate-800" data-tauri-drag-region>
        <h1 class="text-sm font-semibold text-slate-200 tracking-wide">
          Simple SQL Profiler
        </h1>
        <div class="flex items-center gap-2 text-xs text-slate-500">
          <span>MSSQL</span>
          {status().connected && (
            <span class="flex items-center gap-1">
              <span class="w-1.5 h-1.5 rounded-full bg-emerald-500" />
              Connected
            </span>
          )}
        </div>
      </header>

      {/* Connection Form (overlay) */}
      {showConnection() && (
        <ConnectionForm
          onConnect={handleConnect}
          onClose={() => status().connected && setShowConnection(false)}
          error={status().error}
          connected={status().connected}
        />
      )}

      {/* Toolbar */}
      <Toolbar
        connected={status().connected}
        capturing={status().capturing}
        queryCount={queries.length}
        filterText={filterText()}
        autoScroll={autoScroll()}
        onStartCapture={handleStartCapture}
        onStopCapture={handleStopCapture}
        onClear={handleClear}
        onDisconnect={handleDisconnect}
        onFilterChange={setFilterText}
        onToggleConnection={() => setShowConnection((s) => !s)}
        onToggleAutoScroll={() => setAutoScroll((s) => !s)}
      />

      {/* Main Content */}
      <div class="flex-1 flex flex-col min-h-0">
        <QueryFeed
          queries={filteredQueries()}
          selectedId={selectedId()}
          autoScroll={autoScroll()}
          onSelect={(id) => setSelectedId(id === selectedId() ? null : id)}
        />

        {selectedQuery() && (
          <QueryDetail
            query={selectedQuery()!}
            onClose={() => setSelectedId(null)}
          />
        )}
      </div>

      {/* Status Bar */}
      <footer class="flex items-center justify-between px-3 py-1 bg-slate-900 border-t border-slate-800 text-xs text-slate-500">
        <div class="flex items-center gap-3">
          {status().capturing ? (
            <span class="flex items-center gap-1.5">
              <span class="w-1.5 h-1.5 rounded-full bg-red-500 animate-pulse" />
              Capturing
            </span>
          ) : (
            <span>Idle</span>
          )}
          <span>{queries.length} events</span>
          {filterText() && (
            <span>{filteredQueries().length} shown</span>
          )}
        </div>
        {status().error && (
          <span class="text-red-400 truncate max-w-md">{status().error}</span>
        )}
      </footer>
    </div>
  );
}
