import { invoke } from "@tauri-apps/api/core";
import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import type { QueryEvent, QueryResultData } from "../lib/types.ts";

interface Props {
  query: QueryEvent;
  onClose: () => void;
}

type RunState =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "success"; data: QueryResultData }
  | { status: "error"; message: string };

function SqlBlock(props: { text: string; label?: string; class?: string }) {
  const [copied, setCopied] = createSignal(false);

  async function handleCopy() {
    await navigator.clipboard.writeText(props.text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }

  return (
    <div class={`relative group ${props.class || ""}`}>
      {props.label && (
        <div class="text-[10px] text-slate-500 uppercase tracking-wider mb-1.5 font-medium">
          {props.label}
        </div>
      )}
      <div class="relative bg-slate-900/50 rounded-lg p-4 border border-slate-700/50">
        <button
          onClick={handleCopy}
          class="absolute top-2 right-2 px-2 py-1 rounded bg-slate-800 border border-slate-700 text-[10px] text-slate-400 opacity-0 group-hover:opacity-100 transition-all hover:text-slate-200 hover:border-slate-600 active:scale-95 z-10"
        >
          {copied() ? (
            <span class="flex items-center gap-1 text-emerald-400">
              <i class="fa-solid fa-check" /> Copied
            </span>
          ) : (
            <span class="flex items-center gap-1">
              <i class="fa-solid fa-copy" /> Copy
            </span>
          )}
        </button>
        <pre class="text-xs font-mono text-slate-200 whitespace-pre-wrap break-all leading-relaxed selection:bg-blue-500/30">
          {props.text}
        </pre>
      </div>
    </div>
  );
}

function ResultsTable(props: { data: QueryResultData }) {
  return (
    <div>
      <div class="text-[10px] text-slate-500 uppercase tracking-wider mb-1.5 font-medium">
        Results ({props.data.rows.length} row{props.data.rows.length !== 1 ? "s" : ""})
      </div>
      <div class="bg-slate-900/50 rounded-lg border border-slate-700/50 overflow-auto max-h-[300px]">
        <Show
          when={props.data.columns.length > 0}
          fallback={
            <div class="px-4 py-3 text-xs text-slate-500">
              Query executed successfully. No result set returned.
            </div>
          }
        >
          <table class="w-full text-xs">
            <thead class="sticky top-0">
              <tr class="bg-slate-800 text-slate-400">
                <For each={props.data.columns}>
                  {(col) => (
                    <th class="px-3 py-1.5 text-left font-semibold text-[10px] uppercase tracking-wider border-b border-slate-700 whitespace-nowrap">
                      {col}
                    </th>
                  )}
                </For>
              </tr>
            </thead>
            <tbody>
              <For each={props.data.rows}>
                {(row, idx) => (
                  <tr class={idx() % 2 === 0 ? "bg-slate-900/30" : "bg-slate-900/60"}>
                    <For each={row}>
                      {(cell) => (
                        <td class="px-3 py-1 text-slate-300 font-mono text-[11px] whitespace-pre-wrap break-words border-b border-slate-800/50">
                          {cell === null ? <span class="text-slate-600 italic">NULL</span> : String(cell)}
                        </td>
                      )}
                    </For>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </Show>
      </div>
    </div>
  );
}

export default function QueryDetail(props: Props) {
  const savedHeight = parseInt(localStorage.getItem("detail-panel-height") || "300", 10);
  const [height, setHeight] = createSignal(savedHeight);
  const [mounted, setMounted] = createSignal(false);
  const [runState, setRunState] = createSignal<RunState>({ status: "idle" });
  const [showConfirm, setShowConfirm] = createSignal(false);

  let dragging = false;
  let startY = 0;
  let startH = 0;
  let contentRef: HTMLDivElement | undefined;

  function onPointerDown(e: PointerEvent) {
    dragging = true;
    startY = e.clientY;
    startH = height();
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
  }

  function onPointerMove(e: PointerEvent) {
    if (!dragging) return;
    const delta = startY - e.clientY;
    const next = Math.max(120, Math.min(startH + delta, window.innerHeight * 0.8));
    setHeight(next);
  }

  function onPointerUp() {
    dragging = false;
    localStorage.setItem("detail-panel-height", String(height()));
  }

  function formatDuration(ms: number): string {
    if (ms < 1000) return `${ms}ms`;
    if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
    return `${(ms / 60000).toFixed(1)}m`;
  }

  function formatStartTimeParts(isoStr: string): { time: string; date: string } {
    if (!isoStr) return { time: "-", date: "-" };

    const isoMatch = isoStr.match(
      /^(\d{4}-\d{2}-\d{2})[T\s](\d{2}:\d{2}:\d{2}(?:\.\d{1,3})?)/
    );
    if (isoMatch) {
      return { date: isoMatch[1], time: isoMatch[2] };
    }

    const d = new Date(isoStr);
    if (!Number.isNaN(d.getTime())) {
      const date = d.toLocaleDateString("en-CA");
      const time = d.toLocaleTimeString("en-US", { hour12: false, fractionalSecondDigits: 3 });
      return { date, time };
    }

    return { time: isoStr, date: "-" };
  }

  function handleRunClick() {
    if (runState().status === "loading") return;
    setShowConfirm(true);
  }

  async function executeQuery() {
    setShowConfirm(false);
    setRunState({ status: "loading" });
    try {
      const statement = props.query.current_statement || props.query.sql_text;
      const db = props.query.database_name;
      const sql = db ? `USE [${db}];\n${statement}` : statement;
      const data = await invoke<QueryResultData>("execute_query", { sql });
      setRunState({ status: "success", data });
      requestAnimationFrame(() => {
        if (contentRef) contentRef.scrollTo({ top: contentRef.scrollHeight, behavior: "smooth" });
      });
    } catch (e) {
      setRunState({ status: "error", message: String(e) });
      requestAnimationFrame(() => {
        if (contentRef) contentRef.scrollTo({ top: contentRef.scrollHeight, behavior: "smooth" });
      });
    }
  }

  // Reset run state when query changes
  createEffect(() => {
    void props.query.id;
    setRunState({ status: "idle" });
  });

  createEffect(() => {
    // Entrance animation
    setMounted(false);
    const raf = requestAnimationFrame(() => {
      setMounted(true);
    });
    onCleanup(() => cancelAnimationFrame(raf));
  });

  return (
    <div
      class={`relative border-t border-slate-700 bg-slate-800 flex flex-col shrink-0 select-text transition-all duration-200 ease-out ${!mounted() ? "translate-y-8 opacity-0" : "translate-y-0 opacity-100"
        }`}
      style={{
        height: `${height()}px`
      }}
    >
      <div
        class="absolute -top-1 left-0 right-0 h-2 cursor-ns-resize z-50 hover:bg-blue-500/30 transition-colors"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
      />

      {/* Confirm Dialog */}
      <Show when={showConfirm()}>
        <div class="absolute inset-0 z-[60] flex items-center justify-center bg-slate-900/80 backdrop-blur-sm">
          <div class="w-full max-w-lg bg-slate-900 border border-slate-800 rounded-xl shadow-2xl p-6">
            <div class="flex items-center gap-3 mb-3">
              <div class="w-10 h-10 rounded-full bg-amber-500/10 flex items-center justify-center shrink-0">
                <i class="fa-solid fa-play text-amber-400 text-sm" />
              </div>
              <div>
                <h3 class="text-sm font-semibold text-slate-100">Run query?</h3>
                <p class="text-xs text-slate-400 mt-0.5">
                  This will execute the query on <span class="text-slate-200 font-medium">{props.query.database_name || "the connected server"}</span>
                </p>
              </div>
            </div>
            <pre class="text-[11px] font-mono text-slate-300 bg-slate-800/80 rounded p-3 mb-4 max-h-[120px] overflow-auto whitespace-pre-wrap break-all border border-slate-700/50">
              {props.query.current_statement || props.query.sql_text}
            </pre>
            <div class="flex gap-2 justify-end">
              <button
                onClick={() => setShowConfirm(false)}
                class="px-4 py-1.5 bg-slate-800 hover:bg-slate-700 text-slate-300 text-xs font-medium rounded transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={() => void executeQuery()}
                class="px-4 py-1.5 bg-blue-600 hover:bg-blue-500 text-white text-xs font-medium rounded transition-colors"
              >
                Run
              </button>
            </div>
          </div>
        </div>
      </Show>

      {/* Header */}
      <div class="flex items-stretch border-b border-slate-700 bg-slate-800/50 shrink-0 h-[42px]">
        <div class="query-detail-header-scroll flex-1 min-w-0 overflow-x-auto overflow-y-hidden pr-2">
          <div class="flex items-stretch min-w-max h-full">
            {/* Event Type & Session */}
            <div class="flex items-center gap-3 px-4 border-r border-slate-700/50">
              <span class="px-1.5 py-0.5 rounded text-[10px] font-bold uppercase bg-slate-700 text-slate-400">
                {props.query.event_name.includes("login") ||
                  props.query.event_name.includes("logout") ||
                  props.query.event_name.includes("connection")
                  ? "AUDIT"
                  : props.query.event_name === "rpc_completed" ||
                    props.query.event_name === "rpc_starting"
                  ? "RPC"
                  : props.query.event_name === "sp_statement_completed" ||
                    props.query.event_name === "sql_statement_completed"
                    ? "STMT"
                    : props.query.event_name.includes("prepared") ||
                      props.query.event_name.includes("prepare")
                      ? "PREP"
                      : "BATCH"}
              </span>
              <div class="flex flex-col justify-center">
                <span class="text-[11px] font-semibold text-slate-100 tabular-nums">#{props.query.session_id}</span>
                <span class="text-[9px] text-slate-500 uppercase tracking-tighter">Session</span>
              </div>
            </div>

            {/* DB */}
            <div class="flex flex-col justify-center px-4 border-r border-slate-700/50 min-w-max">
              <span class="text-[11px] font-semibold text-slate-100">{props.query.database_name}</span>
              <span class="text-[9px] text-slate-500 uppercase tracking-tighter">Database</span>
            </div>

            {/* Environment */}
            <div class="flex flex-col justify-center px-4 border-r border-slate-700/50 min-w-max">
              <span class="text-[11px] font-medium text-slate-300 leading-tight">
                {props.query.login_name}<span class="text-slate-500">@</span>{props.query.host_name}
              </span>
              <span class="text-[9px] text-slate-500 truncate max-w-[120px]" title={props.query.program_name}>
                {props.query.program_name}
              </span>
            </div>

            {/* Timestamp */}
            <div class="flex flex-col justify-center px-4 border-r border-slate-700/50 min-w-max">
              <span class="text-[11px] font-medium text-slate-300 tabular-nums leading-tight">
                {formatStartTimeParts(props.query.start_time).time}
              </span>
              <span class="text-[9px] text-slate-500 tabular-nums leading-tight">
                {formatStartTimeParts(props.query.start_time).date}
              </span>
            </div>

            {/* Stats */}
            <div class="flex items-stretch divide-x divide-slate-700/50 border-l border-r border-slate-700/50">
              <div class="flex flex-col items-center justify-center px-4 min-w-[70px]">
                <span class="text-[11px] font-bold text-slate-100 tabular-nums">{formatDuration(props.query.elapsed_time)}</span>
                <span class="text-[9px] text-slate-500 uppercase tracking-wider">Duration</span>
              </div>
              <div class="flex flex-col items-center justify-center px-4 min-w-[60px]">
                <span class="text-[11px] font-bold text-slate-100 tabular-nums">{formatDuration(props.query.cpu_time)}</span>
                <span class="text-[9px] text-slate-500 uppercase tracking-wider">CPU</span>
              </div>
              <div class="flex flex-col items-center justify-center px-4 min-w-[70px]">
                <span class="text-[11px] font-bold text-slate-100 tabular-nums">{props.query.logical_reads.toLocaleString()}</span>
                <span class="text-[9px] text-slate-500 uppercase tracking-wider">Reads</span>
              </div>
              <div class="flex flex-col items-center justify-center px-4 min-w-[70px]">
                <span class="text-[11px] font-bold text-slate-100 tabular-nums">{props.query.physical_reads.toLocaleString()}</span>
                <span class="text-[9px] text-slate-500 uppercase tracking-wider">Physical</span>
              </div>
              <div class="flex flex-col items-center justify-center px-4 min-w-[60px]">
                <span class="text-[11px] font-bold text-slate-100 tabular-nums">{props.query.writes.toLocaleString()}</span>
                <span class="text-[9px] text-slate-500 uppercase tracking-wider">Writes</span>
              </div>
              <div class="flex flex-col items-center justify-center px-4 min-w-[60px]">
                <span class="text-[11px] font-bold text-slate-100 tabular-nums">{props.query.row_count.toLocaleString()}</span>
                <span class="text-[9px] text-slate-500 uppercase tracking-wider">Rows</span>
              </div>
            </div>
          </div>
        </div>

        {/* Run & Close buttons */}
        <button
          type="button"
          onClick={handleRunClick}
          disabled={runState().status === "loading"}
          class="text-slate-400 hover:text-emerald-400 w-12 h-full flex items-center justify-center hover:bg-slate-700/50 transition-all border-l border-slate-700/50 shrink-0 disabled:opacity-40 disabled:cursor-not-allowed"
          title="Run query"
        >
          {runState().status === "loading" ? (
            <i class="fa-solid fa-spinner fa-spin text-xs" />
          ) : (
            <i class="fa-solid fa-play text-xs" />
          )}
        </button>
        <button
          type="button"
          onClick={() => props.onClose()}
          class="text-slate-500 hover:text-slate-200 w-12 h-full flex items-center justify-center hover:bg-slate-700/50 transition-all border-l border-slate-700/50 shrink-0"
          title="Close details"
        >
          <i class="fa-solid fa-chevron-down text-xs" />
        </button>
      </div>

      {/* SQL Text & Results */}
      <div ref={contentRef} class="flex-1 overflow-auto p-4 flex flex-col gap-6">
        <SqlBlock
          text={props.query.current_statement || props.query.sql_text}
          label="Statement"
        />

        {props.query.current_statement &&
          props.query.sql_text !== props.query.current_statement && (
            <SqlBlock
              text={props.query.sql_text}
              label="Full Batch"
            />
          )}

        {/* Query Results */}
        {runState().status === "error" && (
          <div>
            <div class="text-[10px] text-slate-500 uppercase tracking-wider mb-1.5 font-medium">
              Error
            </div>
            <div class="bg-red-950/30 rounded-lg p-4 border border-red-900/50 text-xs text-red-400 font-mono whitespace-pre-wrap break-words">
              {(runState() as { status: "error"; message: string }).message}
            </div>
          </div>
        )}

        {runState().status === "success" && (
          <ResultsTable data={(runState() as { status: "success"; data: QueryResultData }).data} />
        )}
      </div>
    </div>
  );
}
