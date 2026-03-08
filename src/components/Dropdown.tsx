import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";

interface Option {
  value: string;
  label: string;
}

interface Props {
  value: string;
  options: Option[];
  onChange: (value: string) => void;
  disabled?: boolean;
  title?: string;
  class?: string;
  triggerClass?: string;
}

export default function Dropdown(props: Props) {
  const [open, setOpen] = createSignal(false);
  const [coords, setCoords] = createSignal({ top: 0, left: 0, width: 0 });
  let containerRef!: HTMLDivElement;
  let triggerRef!: HTMLButtonElement;

  const selectedLabel = () =>
    props.options.find((o) => o.value === props.value)?.label ?? "";

  const updateCoords = () => {
    if (triggerRef) {
      const rect = triggerRef.getBoundingClientRect();
      setCoords({
        top: rect.bottom + window.scrollY,
        left: rect.left + window.scrollX,
        width: rect.width,
      });
    }
  };

  function handleClickOutside(e: MouseEvent) {
    if (!containerRef.contains(e.target as Node)) {
      setOpen(false);
    }
  }

  onMount(() => {
    document.addEventListener("mousedown", handleClickOutside);
    window.addEventListener("scroll", updateCoords, true);
    window.addEventListener("resize", updateCoords);
  });

  onCleanup(() => {
    document.removeEventListener("mousedown", handleClickOutside);
    window.removeEventListener("scroll", updateCoords, true);
    window.removeEventListener("resize", updateCoords);
  });

  function toggle() {
    if (props.disabled) return;
    updateCoords();
    setOpen((s) => !s);
  }

  function select(value: string) {
    if (props.disabled) return;
    props.onChange(value);
    setOpen(false);
  }

  function handleKeyDown(e: KeyboardEvent) {
    if (props.disabled) return;

    if (e.key === "Escape") {
      setOpen(false);
      return;
    }

    if (!open() && (e.key === "Enter" || e.key === " " || e.key === "ArrowDown")) {
      e.preventDefault();
      toggle();
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
    <div
      ref={containerRef}
      class={`dropdown-container relative ${props.class ?? ""}`}
    >
      <button
        ref={triggerRef}
        type="button"
        class={`dropdown-trigger ${props.triggerClass ?? ""}`}
        classList={{ "dropdown-trigger--open": open() }}
        disabled={props.disabled}
        title={props.title}
        onClick={toggle}
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

      <Show when={open()}>
        <Portal>
          <ul
            class="dropdown-menu fixed"
            style={{
              top: `${coords().top}px`,
              left: `${coords().left}px`,
              width: `${coords().width}px`,
              "z-index": 9999
            }}
          >
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
        </Portal>
      </Show>
    </div>
  );
}
