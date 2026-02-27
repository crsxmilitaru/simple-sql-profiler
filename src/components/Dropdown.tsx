import { createSignal, For, onCleanup, onMount } from "solid-js";

interface Option {
  value: string;
  label: string;
}

interface Props {
  value: string;
  options: Option[];
  onChange: (value: string) => void;
}

export default function Dropdown(props: Props) {
  const [open, setOpen] = createSignal(false);
  let containerRef!: HTMLDivElement;

  const selectedLabel = () =>
    props.options.find((o) => o.value === props.value)?.label ?? "";

  function handleClickOutside(e: MouseEvent) {
    if (!containerRef.contains(e.target as Node)) {
      setOpen(false);
    }
  }

  onMount(() => document.addEventListener("mousedown", handleClickOutside));
  onCleanup(() => document.removeEventListener("mousedown", handleClickOutside));

  function select(value: string) {
    props.onChange(value);
    setOpen(false);
  }

  function handleKeyDown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      setOpen(false);
      return;
    }

    if (!open() && (e.key === "Enter" || e.key === " " || e.key === "ArrowDown")) {
      e.preventDefault();
      setOpen(true);
      return;
    }

    if (!open()) return;

    const currentIdx = props.options.findIndex((o) => o.value === props.value);
    if (e.key === "ArrowDown") {
      e.preventDefault();
      const next = Math.min(currentIdx + 1, props.options.length - 1);
      select(props.options[next].value);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      const prev = Math.max(currentIdx - 1, 0);
      select(props.options[prev].value);
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      setOpen(false);
    }
  }

  return (
    <div ref={containerRef} class="dropdown-container relative">
      <button
        type="button"
        class="dropdown-trigger"
        classList={{ "dropdown-trigger--open": open() }}
        onClick={() => setOpen((s) => !s)}
        onKeyDown={handleKeyDown}
      >
        <span class="truncate">{selectedLabel()}</span>
        <svg
          class="dropdown-chevron"
          classList={{ "dropdown-chevron--open": open() }}
          viewBox="0 0 16 16"
          fill="currentColor"
        >
          <path d="M4.427 6.427a.75.75 0 011.06-.074L8 8.578l2.513-2.225a.75.75 0 01.994 1.124l-3 2.656a.75.75 0 01-.994 0l-3-2.656a.75.75 0 01-.086-1.05z" />
        </svg>
      </button>

      {open() && (
        <ul class="dropdown-menu">
          <For each={props.options}>
            {(option) => (
              <li
                class="dropdown-item"
                classList={{ "dropdown-item--selected": option.value === props.value }}
                onMouseDown={() => select(option.value)}
              >
                {option.label}
              </li>
            )}
          </For>
        </ul>
      )}
    </div>
  );
}
