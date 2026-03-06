import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { Show, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { createStore, produce } from "solid-js/store";
import AboutDialog from "./components/AboutDialog.tsx";
import AdvancedFilterDialog from "./components/AdvancedFilterDialog.tsx";
import ConnectionForm from "./components/ConnectionForm.tsx";
import ContextMenu from "./components/ContextMenu.tsx";
import QueryDetail from "./components/QueryDetail.tsx";
import QueryFeed from "./components/QueryFeed.tsx";
import TitleBar from "./components/TitleBar.tsx";
import Toolbar from "./components/Toolbar.tsx";
import UpdateDialog from "./components/UpdateDialog.tsx";
import { evaluateFilter, type AdvancedFilterCondition } from "./lib/advancedFilters.ts";
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

const MAX_QUERY_BUFFER = 5000;

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
  const [autoScroll, setAutoScroll] = createSignal<"on" | "off" | "smart">(
    (() => {
      const val = localStorage.getItem("auto-scroll");
      if (val === "on" || val === "off" || val === "smart") return val;
      if (val === "false") return "off";
      return "smart";
    })()
  );
  const [deduplicateRepeats, setDeduplicateRepeats] = createSignal(
    (() => {
      const hasExplicitPreference = localStorage.getItem("deduplicate-repeats-explicit") === "true";
      if (!hasExplicitPreference) {
        return false;
      }
      return localStorage.getItem("deduplicate-repeats") === "true";
    })()
  );
  const [updateStatus, setUpdateStatus] = createSignal<UpdateStatus>({
    checking: false,
    message: null,
    tone: "info",
  });
  const [updateAvailable, setUpdateAvailable] = createSignal<Update | null>(null);
  const [advancedFilters, setAdvancedFilters] = createSignal<AdvancedFilterCondition[]>(
    (() => {
      try {
        const stored = localStorage.getItem("advanced-filters");
        return stored ? JSON.parse(stored) : [];
      } catch {
        return [];
      }
    })()
  );
  const [showAdvancedFilter, setShowAdvancedFilter] = createSignal(false);
  const [starting, setStarting] = createSignal(false);
  const [stopping, setStopping] = createSignal(false);

  createEffect(() => {
    localStorage.setItem("advanced-filters", JSON.stringify(advancedFilters()));
  });

  createEffect(() => {
    localStorage.setItem("auto-scroll", String(autoScroll()));
  });

  const selectedQuery = () => queries.find((q) => q.id === selectedId()) ?? null;

  const filteredQueries = createMemo(() => {
    const filter = filterText().toLowerCase();
    const advFilters = advancedFilters();

    let result = queries.filter((q) => {
      // Basic text search (OR across common fields)
      if (filter) {
        const matchesText =
          (q.sql_text || "").toLowerCase().includes(filter) ||
          (q.current_statement || "").toLowerCase().includes(filter) ||
          (q.database_name || "").toLowerCase().includes(filter) ||
          (q.login_name || "").toLowerCase().includes(filter) ||
          (q.program_name || "").toLowerCase().includes(filter);

        if (!matchesText) return false;
      }

      // Advanced filters (AND across all conditions)
      if (advFilters.length > 0) {
        const matchesAdvanced = advFilters.every(f => evaluateFilter(q, f));
        if (!matchesAdvanced) return false;
      }

      return true;
    });

    if (deduplicateRepeats()) {
      result = result.filter((q, i, arr) => {
        if (i === 0) return true;
        const prev = arr[i - 1];
        const currentText = q.current_statement || q.sql_text;
        const previousText = prev.current_statement || prev.sql_text;
        const sameFingerprint =
          currentText === previousText &&
          q.database_name === prev.database_name &&
          q.event_name === prev.event_name;
        return !sameFingerprint;
      });
    }

    return result;
  });

  onMount(() => {
    let unlistenQuery: (() => void) | null = null;
    let unlistenStatus: (() => void) | null = null;
    let updateTimeout: number | undefined;

    onCleanup(() => {
      unlistenQuery?.();
      unlistenStatus?.();
      if (updateTimeout !== undefined) {
        clearTimeout(updateTimeout);
      }
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
        setQueries(
          produce((draft) => {
            draft.push(query);
            if (draft.length > MAX_QUERY_BUFFER) {
              draft.splice(0, draft.length - MAX_QUERY_BUFFER);
            }
          }),
        );
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

      updateTimeout = window.setTimeout(() => {
        void handleCheckForUpdates(false);
      }, 5000);
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
    setStarting(true);
    try {
      await invoke("start_capture");
    } catch (e) {
      setStatus((s) => ({ ...s, error: String(e) }));
      setShowConnection(true);
    } finally {
      setStarting(false);
    }
  }

  async function handleStopCapture() {
    setStopping(true);
    try {
      await invoke("stop_capture");
    } catch (e) {
      setStatus((s) => ({ ...s, error: String(e) }));
    } finally {
      setStopping(false);
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

      setUpdateAvailable(update);
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

  async function handleInstallUpdate(update: Update) {
    setUpdateAvailable(null);
    setUpdateStatus({
      checking: true,
      message: `Downloading and installing ${update.version}...`,
      tone: "info",
    });

    try {
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
      setUpdateStatus({
        checking: false,
        message,
        tone,
      });
      console.error("Update install failed:", error);
    }
  }

  function handleCancelUpdate(update: Update) {
    setUpdateAvailable(null);
    setUpdateStatus({
      checking: false,
      message: `Update ${update.version} is available.`,
      tone: "info",
    });
  }

  function handleClear() {
    setQueries([]);
    setSelectedId(null);
  }

  function handleToggleDeduplicateRepeats() {
    setDeduplicateRepeats((current) => {
      const next = !current;
      localStorage.setItem("deduplicate-repeats", String(next));
      localStorage.setItem("deduplicate-repeats-explicit", "true");
      return next;
    });
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

        {showAdvancedFilter() && (
          <AdvancedFilterDialog
            filters={advancedFilters()}
            onApply={setAdvancedFilters}
            onClose={() => setShowAdvancedFilter(false)}
          />
        )}

        <Show when={updateAvailable()} keyed>
          {(update) => (
            <UpdateDialog
              version={update.version}
              currentVersion={update.currentVersion}
              onInstall={() => void handleInstallUpdate(update)}
              onCancel={() => handleCancelUpdate(update)}
            />
          )}
        </Show>

        {/* Toolbar */}
        <Toolbar
          connected={status().connected}
          capturing={status().capturing}
          starting={starting()}
          stopping={stopping()}
          queryCount={queries.length}
          filterText={filterText()}
          advancedFilterCount={advancedFilters().length}
          autoScroll={autoScroll()}
          deduplicateRepeats={deduplicateRepeats()}
          error={status().connected ? status().error : null}
          onStartCapture={handleStartCapture}
          onStopCapture={handleStopCapture}
          onClear={handleClear}
          onFilterChange={setFilterText}
          onOpenAdvancedFilter={() => setShowAdvancedFilter(true)}
          onToggleAutoScroll={() => setAutoScroll((s) => s === "smart" ? "on" : s === "on" ? "off" : "smart")}
          onToggleDeduplicateRepeats={handleToggleDeduplicateRepeats}
        />

        {/* Main Content Area */}
        <div class="flex-1 flex flex-row min-h-0 relative">
          {/* List and Details */}
          <div class="flex-1 flex flex-col min-h-0 relative">
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
      </div>
      <ContextMenu />
    </div>
  );
}
