import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";
import { createEffect, createSignal, onCleanup, onMount, Show } from "solid-js";
import { createStore, produce } from "solid-js/store";
import AboutDialog from "./components/AboutDialog.tsx";
import ConnectionForm from "./components/ConnectionForm.tsx";
import ContextMenu from "./components/ContextMenu.tsx";
import QueryDetail from "./components/QueryDetail.tsx";
import QueryFeed from "./components/QueryFeed.tsx";
import TitleBar from "./components/TitleBar.tsx";
import Toolbar from "./components/Toolbar.tsx";
import type { ConnectionConfig, ProfilerStatus, QueryEvent } from "./lib/types.ts";

type UpdateMessageTone = "info" | "success" | "error";

interface UpdateStatus {
  checking: boolean;
  message: string | null;
  tone: UpdateMessageTone;
}

interface UpdaterErrorDetails {
  message: string;
  configurationIssue: boolean;
  tone: UpdateMessageTone;
}

const MISSING_UPDATER_CONFIG_MESSAGE =
  "Updater is not configured yet. Set plugins.updater.endpoints and plugins.updater.pubkey in src-tauri/tauri.conf.json.";
const INVALID_UPDATER_SIGNATURE_MESSAGE =
  "Updater signature verification failed. Ensure releases are signed with the private key matching plugins.updater.pubkey.";
const NO_RELEASE_METADATA_MESSAGE =
  "No published update metadata found yet.";

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
  const [showAbout, setShowAbout] = createSignal(false);
  const [appVersion, setAppVersion] = createSignal<string | null>(null);
  const [autoScroll, setAutoScroll] = createSignal(localStorage.getItem("auto-scroll") !== "false");
  const [deduplicateRepeats, setDeduplicateRepeats] = createSignal(localStorage.getItem("deduplicate-repeats") !== "false");
  const [updateStatus, setUpdateStatus] = createSignal<UpdateStatus>({
    checking: false,
    message: null,
    tone: "info",
  });

  createEffect(() => {
    localStorage.setItem("auto-scroll", String(autoScroll()));
  });

  createEffect(() => {
    localStorage.setItem("deduplicate-repeats", String(deduplicateRepeats()));
  });

  const selectedQuery = () => queries.find((q) => q.id === selectedId()) ?? null;

  const filteredQueries = () => {
    const filter = filterText().toLowerCase();
    let result: QueryEvent[] = filter
      ? queries.filter(
        (q) =>
          q.sql_text.toLowerCase().includes(filter) ||
          q.current_statement.toLowerCase().includes(filter) ||
          q.database_name.toLowerCase().includes(filter) ||
          q.login_name.toLowerCase().includes(filter) ||
          q.program_name.toLowerCase().includes(filter)
      )
      : [...queries];

    if (deduplicateRepeats()) {
      result = result.filter(
        (q, i, arr) => i === 0 || q.sql_text !== arr[i - 1].sql_text
      );
    }

    return result;
  };

  onMount(() => {
    let unlistenQuery: (() => void) | null = null;
    let unlistenStatus: (() => void) | null = null;

    onCleanup(() => {
      unlistenQuery?.();
      unlistenStatus?.();
    });

    void (async () => {
      try {
        setAppVersion(await getVersion());
      } catch (error) {
        console.error("Failed to read app version:", error);
        setAppVersion(null);
      }

      unlistenQuery = await listen<QueryEvent>("query-event", (event) => {
        const query = event.payload;
        const existingIdx = queries.findIndex((q) => q.id === query.id);
        if (existingIdx >= 0) {
          setQueries(existingIdx, query);
        } else {
          setQueries(produce((draft) => draft.push(query)));
        }
      });

      unlistenStatus = await listen<ProfilerStatus>(
        "profiler-status",
        (event) => {
          setStatus(event.payload);
          if (event.payload.connected) {
            setShowConnection(false);
          } else if (event.payload.capturing) {
            void handleStopCapture();
          }
        }
      );

      void handleCheckForUpdates(false);
    })();
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
      setShowConnection(true);
    }
  }

  async function handleStopCapture() {
    try {
      await invoke("stop_capture");
    } catch (e) {
      setStatus((s) => ({ ...s, error: String(e) }));
    }
  }

  function formatUpdaterError(error: unknown): UpdaterErrorDetails {
    const message = String(error);
    const normalized = message.toLowerCase();

    if (normalized.includes("updater does not have any endpoints set")) {
      return {
        message: MISSING_UPDATER_CONFIG_MESSAGE,
        configurationIssue: true,
        tone: "error",
      };
    }

    if (
      normalized.includes("public key") ||
      normalized.includes("pubkey") ||
      normalized.includes("signature") && normalized.includes("could not be decoded")
    ) {
      return {
        message: INVALID_UPDATER_SIGNATURE_MESSAGE,
        configurationIssue: true,
        tone: "error",
      };
    }

    if (normalized.includes("could not fetch a valid release json from the remote")) {
      return {
        message: NO_RELEASE_METADATA_MESSAGE,
        configurationIssue: false,
        tone: "info",
      };
    }

    return {
      message: `Update check failed: ${message}`,
      configurationIssue: false,
      tone: "error",
    };
  }

  async function handleCheckForUpdates(manual: boolean) {
    if (updateStatus().checking) {
      return;
    }

    setUpdateStatus({
      checking: true,
      message: manual ? "Checking for updates..." : null,
      tone: "info",
    });

    try {
      const update = await check();
      if (!update) {
        setUpdateStatus({
          checking: false,
          message: manual ? "You are running the latest version." : null,
          tone: "success",
        });
        return;
      }

      const shouldInstall = window.confirm(
        `Version ${update.version} is available (current ${update.currentVersion}). Install now?`
      );
      if (!shouldInstall) {
        setUpdateStatus({
          checking: false,
          message: `Update ${update.version} is available.`,
          tone: "info",
        });
        return;
      }

      setUpdateStatus({
        checking: true,
        message: `Downloading and installing ${update.version}...`,
        tone: "info",
      });

      await update.downloadAndInstall();
      setUpdateStatus({
        checking: true,
        message: `Update ${update.version} installed. Restarting...`,
        tone: "success",
      });

      try {
        await relaunch();
      } catch (restartError) {
        setUpdateStatus({
          checking: false,
          message: `Update ${update.version} installed. Please restart the app manually.`,
          tone: "success",
        });
        console.error("Update relaunch failed:", restartError);
      }
    } catch (error) {
      const { message, configurationIssue, tone } = formatUpdaterError(error);
      const shouldHideMessage = !manual && configurationIssue;
      setUpdateStatus({
        checking: false,
        message: shouldHideMessage ? null : message,
        tone: shouldHideMessage ? "info" : tone,
      });

      if (!manual && !configurationIssue) {
        console.error("Automatic update check failed:", error);
      }
    }
  }

  function handleClear() {
    setQueries([]);
    setSelectedId(null);
  }

  return (
    <div class="h-screen flex flex-col bg-slate-900">
      <TitleBar
        onToggleConnection={() => setShowConnection((s) => !s)}
        onShowAbout={() => setShowAbout(true)}
        connected={status().connected}
        disabled={showConnection()}
        aboutDisabled={showAbout()}
      />

      <div class="flex-1 flex flex-col min-h-0 relative">
        {/* Connection Form (overlay) */}
        {showConnection() && (
          <ConnectionForm
            onConnect={handleConnect}
            onClose={() => status().connected && setShowConnection(false)}
            error={!status().connected ? status().error : null}
            connected={status().connected}
          />
        )}

        {showAbout() && (
          <AboutDialog
            onClose={() => setShowAbout(false)}
            version={appVersion()}
            onCheckForUpdates={() => handleCheckForUpdates(true)}
            checkingForUpdates={updateStatus().checking}
            updateMessage={updateStatus().message}
            updateMessageTone={updateStatus().tone}
          />
        )}

        {/* Toolbar */}
        <Toolbar
          connected={status().connected}
          capturing={status().capturing}
          queryCount={queries.length}
          filterText={filterText()}
          autoScroll={autoScroll()}
          deduplicateRepeats={deduplicateRepeats()}
          error={status().connected ? status().error : null}
          onStartCapture={handleStartCapture}
          onStopCapture={handleStopCapture}
          onClear={handleClear}
          onFilterChange={setFilterText}
          onToggleAutoScroll={() => setAutoScroll((s) => !s)}
          onToggleDeduplicateRepeats={() => setDeduplicateRepeats((s) => !s)}
        />

        {/* Main Content */}
        <div class="flex-1 flex flex-col min-h-0">
          <QueryFeed
            queries={filteredQueries()}
            selectedId={selectedId()}
            autoScroll={autoScroll()}
            connected={status().connected}
            capturing={status().capturing}
            onSelect={setSelectedId}
          />

          <Show when={selectedQuery()} keyed>
            {(query) => (
              <QueryDetail
                query={query}
                onClose={() => setSelectedId(null)}
              />
            )}
          </Show>
        </div>
      </div>
      <ContextMenu />
    </div>
  );
}
