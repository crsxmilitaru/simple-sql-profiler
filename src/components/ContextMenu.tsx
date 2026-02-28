import { createSignal, onCleanup, onMount, Show } from "solid-js";

export default function ContextMenu() {
  const [pos, setPos] = createSignal({ x: 0, y: 0 });
  const [visible, setVisible] = createSignal(false);
  const [selection, setSelection] = createSignal("");

  function handleContextMenu(e: MouseEvent) {
    const text = window.getSelection()?.toString() || "";
    if (text) {
      e.preventDefault();
      setSelection(text);

      // Ensure menu stays within window bounds
      const x = Math.min(e.clientX, window.innerWidth - 160);
      const y = Math.min(e.clientY, window.innerHeight - 60);

      setPos({ x, y });
      setVisible(true);
    } else {
      // Keep background context menu disabled as per previous request
      e.preventDefault();
      setVisible(false);
    }
  }

  function handleClick() {
    setVisible(false);
  }

  async function handleCopy() {
    try {
      await navigator.clipboard.writeText(selection());
    } catch (err) {
      console.error("Failed to copy text:", err);
    }
    setVisible(false);
  }

  onMount(() => {
    document.addEventListener("contextmenu", handleContextMenu);
    document.addEventListener("click", handleClick);
    window.addEventListener("scroll", handleClick, true);
  });

  onCleanup(() => {
    document.removeEventListener("contextmenu", handleContextMenu);
    document.removeEventListener("click", handleClick);
    window.removeEventListener("scroll", handleClick, true);
  });

  return (
    <Show when={visible()}>
      <div
        class="context-menu"
        style={{ left: `${pos().x}px`, top: `${pos().y}px` }}
      >
        <button
          type="button"
          onClick={handleCopy}
          class="context-menu-item"
        >
          <span>Copy</span>
          <i class="fa-solid fa-copy" />
        </button>
      </div>
    </Show>
  );
}
