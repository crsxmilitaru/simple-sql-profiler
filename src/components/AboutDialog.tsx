import { open } from "@tauri-apps/plugin-shell";

interface Props {
  onClose: () => void;
}

export default function AboutDialog(props: Props) {
  const handleOpenRepo = async (e: MouseEvent) => {
    e.preventDefault();
    await open("https://github.com/crsmilitaru97/simple-sql-profiler");
  };

  return (
    <div class="absolute inset-0 z-[60] flex items-center justify-center bg-slate-900/80 backdrop-blur-sm">
      <div class="w-full max-w-sm bg-slate-900 border border-slate-800 rounded-lg shadow-2xl p-8 text-center">
        <div class="w-20 h-20 bg-blue-600/20 rounded-full flex items-center justify-center mx-auto mb-6">
          <i class="fa-solid fa-database text-3xl text-blue-500" />
        </div>

        <h2 class="text-2xl font-bold text-slate-100 mb-2">Simple SQL Profiler</h2>
        <p class="text-slate-400 text-sm mb-6">
          A lightweight, modern SQL Server Profiler alternative.
        </p>

        <div class="space-y-4 mb-8">
          <a
            href="https://github.com/crsmilitaru97/simple-sql-profiler"
            onClick={handleOpenRepo}
            class="flex items-center justify-center gap-2 text-blue-400 hover:text-blue-300 transition-colors text-sm font-medium"
          >
            <i class="fa-brands fa-github text-lg" />
            GitHub Repository
          </a>
        </div>

        <button
          onClick={props.onClose}
          class="w-full py-2 bg-slate-800 hover:bg-slate-700 text-slate-200 text-sm font-medium rounded transition-colors"
        >
          Close
        </button>
      </div>
    </div>
  );
}
