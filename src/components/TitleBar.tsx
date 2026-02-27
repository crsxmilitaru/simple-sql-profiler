import { getCurrentWindow } from "@tauri-apps/api/window";

interface Props {
  onToggleConnection: () => void;
  onShowAbout: () => void;
  disabled?: boolean;
  aboutDisabled?: boolean;
}

export default function TitleBar(props: Props) {
  const appWindow = getCurrentWindow();

  return (
    <div class="titlebar">
      <div data-tauri-drag-region class="titlebar-drag">
        <span data-tauri-drag-region class="titlebar-title">
          <i data-tauri-drag-region class="fa-solid fa-database" />
          Simple SQL Profiler
        </span>
      </div>

      <div class="flex items-center h-full">
        <button
          class="titlebar-action"
          onClick={props.onToggleConnection}
          disabled={props.disabled}
        >
          <i class="fa-solid fa-plug text-[10px]" />
          Connection
        </button>
        <div class="w-px h-3.5 bg-slate-700 mx-1" />

        <button
          class="titlebar-action"
          onClick={props.onShowAbout}
          disabled={props.aboutDisabled}
        >
          <i class="fa-solid fa-circle-info text-[10px]" />
          About
        </button>
        <div class="w-px h-3.5 bg-slate-700 mx-1" />

        <button
          class="titlebar-btn"
          onClick={() => appWindow.minimize()}
        >
          <svg width="10" height="1" viewBox="0 0 10 1">
            <rect width="10" height="1" fill="currentColor" />
          </svg>
        </button>
        <button
          class="titlebar-btn"
          onClick={() => appWindow.toggleMaximize()}
        >
          <svg width="10" height="10" viewBox="0 0 10 10">
            <rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" stroke-width="1" />
          </svg>
        </button>
        <button
          class="titlebar-btn titlebar-btn--close"
          onClick={() => appWindow.close()}
        >
          <svg width="10" height="10" viewBox="0 0 10 10">
            <path d="M0.5,0.5 L9.5,9.5 M9.5,0.5 L0.5,9.5" fill="none" stroke="currentColor" stroke-width="1" />
          </svg>
        </button>
      </div>
    </div>
  );
}
