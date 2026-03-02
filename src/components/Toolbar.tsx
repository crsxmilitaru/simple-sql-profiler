interface Props {
  connected: boolean;
  capturing: boolean;
  starting: boolean;
  stopping: boolean;
  queryCount: number;
  filterText: string;
  advancedFilterCount: number;
  autoScroll: "on" | "off" | "smart";
  deduplicateRepeats: boolean;
  error: string | null;
  onStartCapture: () => void;
  onStopCapture: () => void;
  onClear: () => void;
  onFilterChange: (value: string) => void;
  onOpenAdvancedFilter: () => void;
  onToggleAutoScroll: () => void;
  onToggleDeduplicateRepeats: () => void;
}

export default function Toolbar(props: Props) {
  const btnBase =
    "flex items-center justify-center gap-1.5 px-3 min-w-[96px] py-1.5 text-xs font-medium rounded transition-colors disabled:opacity-40 disabled:cursor-not-allowed border border-transparent";

  return (
    <div class="flex flex-col bg-slate-800/60 border-b border-slate-700">
      <div class="flex items-center gap-2 px-3 py-2">
        <div class="flex items-center gap-1.5">
          {!props.capturing ? (
            <button
              class={`${btnBase} bg-emerald-600 enabled:hover:bg-emerald-500 text-white`}
              disabled={!props.connected || props.starting}
              onClick={props.onStartCapture}
            >
              <i class={`fa-solid ${props.starting ? "fa-spinner fa-spin" : "fa-play"} text-[10px]`} />
              {props.starting ? "Starting..." : "Start"}
            </button>
          ) : (
            <button
              class={`${btnBase} bg-red-600 enabled:hover:bg-red-500 text-white`}
              disabled={props.stopping}
              onClick={props.onStopCapture}
            >
              <i class={`fa-solid ${props.stopping ? "fa-spinner fa-spin" : "fa-stop"} text-[10px]`} />
              {props.stopping ? "Stopping..." : "Stop"}
            </button>
          )}

          <button
            class={`${btnBase} bg-slate-700 enabled:hover:bg-slate-600 text-slate-200`}
            disabled={props.queryCount === 0}
            onClick={props.onClear}
          >
            <i class="fa-solid fa-trash-can text-[10px]" />
            Clear
          </button>
        </div>

        <div class="flex-1 mx-2 relative flex items-center gap-2">
          <div class="relative flex-1">
            <i class="fa-solid fa-magnifying-glass absolute left-2.5 top-1/2 -translate-y-1/2 text-[10px] text-slate-500" />
            <input
              type="text"
              value={props.filterText}
              onInput={(e) => props.onFilterChange(e.currentTarget.value)}
              placeholder="Filter queries..."
              class="w-full pl-7 pr-3 py-1.5 bg-slate-800 border border-slate-700 rounded text-xs text-slate-200 placeholder-slate-500 focus:outline-none focus:border-blue-500 transition-colors"
            />
          </div>

          <button
            onClick={props.onOpenAdvancedFilter}
            class={`px-3 py-1.5 text-xs font-medium rounded border transition-all flex items-center gap-2 h-[30px] ${props.advancedFilterCount > 0
              ? "bg-blue-600/20 text-blue-400 border-blue-500/40"
              : "bg-slate-800 text-slate-400 border-slate-700 hover:border-slate-600 hover:text-slate-300"
              }`}
          >
            <i class="fa-solid fa-sliders text-[10px]" />
            Advanced
            {props.advancedFilterCount > 0 && (
              <span class="flex items-center justify-center bg-blue-500 text-white text-[9px] font-bold rounded-full w-4 h-4">
                {props.advancedFilterCount}
              </span>
            )}
          </button>
        </div>

        <button
          class={`${btnBase} ${props.deduplicateRepeats
            ? "bg-blue-600/20 text-blue-400 border-blue-500/40"
            : "bg-slate-700 text-slate-400"
            }`}
          onClick={props.onToggleDeduplicateRepeats}
          title="Hide consecutive repeated queries"
        >
          <i class="fa-solid fa-filter text-[10px]" />
          Deduplicate
        </button>

        <button
          class={`${btnBase} ${props.autoScroll !== "off"
            ? "bg-blue-600/20 text-blue-400 border-blue-500/40"
            : "bg-slate-700 text-slate-400"
            }`}
          onClick={props.onToggleAutoScroll}
          title={
            props.autoScroll === "smart" ? "Smart auto-scroll (stops when viewing details)" :
              props.autoScroll === "on" ? "Auto-scroll: On" :
                "Auto-scroll: Off"
          }
        >
          {props.autoScroll === "smart" && <i class="fa-solid fa-arrow-down-short-wide text-[10px]" />}
          {props.autoScroll === "on" && <i class="fa-solid fa-arrow-down text-[10px]" />}
          {props.autoScroll === "off" && <i class="fa-solid fa-arrow-down text-[10px] opacity-50" />}
          {props.autoScroll === "smart" ? "Smart Scroll" : "Auto-scroll"}
        </button>
      </div>

      {props.error && (
        <div class="mx-3 mb-2 p-2.5 bg-red-500/10 border border-red-500/20 rounded text-xs text-red-400 select-text flex items-start gap-2 animate-in fade-in slide-in-from-top-1 duration-200">
          <i class="fa-solid fa-circle-exclamation mt-0.5" />
          <div class="flex-1 leading-relaxed">
            {props.error}
          </div>
        </div>
      )}
    </div>
  );
}
