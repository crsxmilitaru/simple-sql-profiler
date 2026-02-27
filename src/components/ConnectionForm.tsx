import { createSignal, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import type { ConnectionConfig } from "../lib/types.ts";
import Dropdown from "./Dropdown.tsx";

interface Props {
  onConnect: (config: ConnectionConfig, rememberPassword: boolean) => void;
  onClose: () => void;
  error: string | null;
  connected: boolean;
}

export default function ConnectionForm(props: Props) {
  const [serverName, setServerName] = createSignal("localhost");
  const [authentication, setAuthentication] = createSignal("sql");
  const [userName, setUserName] = createSignal("sa");
  const [password, setPassword] = createSignal("");
  const [rememberPassword, setRememberPassword] = createSignal(true);
  const [databaseName, setDatabaseName] = createSignal("");
  const [encrypt, setEncrypt] = createSignal("mandatory");
  const [trustCert, setTrustCert] = createSignal(true);
  const [connecting, setConnecting] = createSignal(false);

  onMount(async () => {
    try {
      const saved: any = await invoke("load_connection");
      setServerName(saved.server_name ?? "localhost");
      setAuthentication(saved.authentication ?? "sql");
      setUserName(saved.username ?? "sa");
      setPassword(saved.password ?? "");
      setDatabaseName(saved.database ?? "");
      setEncrypt(saved.encrypt ?? "mandatory");
      setTrustCert(saved.trust_cert ?? true);
      setRememberPassword(saved.remember_password ?? true);
    } catch {
      // No saved connection — use defaults
    }
  });

  async function handleSubmit(e: Event) {
    e.preventDefault();
    setConnecting(true);
    try {
      await props.onConnect(
        {
          server_name: serverName(),
          authentication: authentication(),
          username: userName(),
          password: password(),
          database: databaseName() || "",
          encrypt: encrypt(),
          trust_cert: trustCert(),
        },
        rememberPassword(),
      );
    } finally {
      setConnecting(false);
    }
  }

  const inputClass =
    "w-full px-3 py-2 bg-slate-800 border border-slate-700 rounded text-sm text-slate-100 placeholder-slate-500 focus:outline-none focus:border-blue-500 focus:ring-1 focus:ring-blue-500 transition-colors";
  const labelClass = "block text-xs font-medium text-slate-400 mb-1";

  return (
    <div class="absolute inset-0 z-50 flex items-center justify-center bg-slate-950/80 backdrop-blur-sm">
      <form
        onSubmit={handleSubmit}
        class="w-full max-w-md bg-slate-900 border border-slate-800 rounded-lg shadow-2xl p-6"
      >
        <div class="flex items-center justify-between mb-6">
          <h2 class="text-lg font-semibold text-slate-100">
            Connect to SQL Server
          </h2>
          {props.connected && (
            <button
              type="button"
              onClick={props.onClose}
              class="text-slate-500 hover:text-slate-300 text-lg leading-none"
            >
              &times;
            </button>
          )}
        </div>

        <div class="space-y-4">
          <div>
            <label class={labelClass}>Server</label>
            <input
              type="text"
              value={serverName()}
              onInput={(e) => setServerName(e.currentTarget.value)}
              placeholder="server\instance or server,port"
              class={inputClass}
            />
          </div>

          <div>
            <label class={labelClass}>Authentication</label>
            <Dropdown
              value={authentication()}
              options={[
                { value: "sql", label: "SQL Server Authentication" },
                { value: "windows", label: "Windows Authentication" },
              ]}
              onChange={setAuthentication}
            />
          </div>

          {authentication() === "sql" && (
            <>
              <div>
                <label class={labelClass}>Username</label>
                <input
                  type="text"
                  value={userName()}
                  onInput={(e) => setUserName(e.currentTarget.value)}
                  placeholder="sa"
                  class={inputClass}
                />
              </div>

              <div>
                <label class={labelClass}>Password</label>
                <input
                  type="password"
                  value={password()}
                  onInput={(e) => setPassword(e.currentTarget.value)}
                  placeholder="Enter password"
                  class={inputClass}
                />
              </div>

              <label class="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={rememberPassword()}
                  onChange={(e) => setRememberPassword(e.currentTarget.checked)}
                  class="custom-checkbox"
                />
                <span class="text-xs text-slate-400">Remember Password</span>
              </label>
            </>
          )}

          <div>
            <label class={labelClass}>Database </label>
            <input
              type="text"
              value={databaseName()}
              onInput={(e) => setDatabaseName(e.currentTarget.value)}
              placeholder="<default>"
              class={inputClass}
            />
          </div>

          <div>
            <label class={labelClass}>Encrypt</label>
            <Dropdown
              value={encrypt()}
              options={[
                { value: "mandatory", label: "Mandatory" },
                { value: "optional", label: "Optional" },
                { value: "strict", label: "Strict" },
              ]}
              onChange={setEncrypt}
            />
          </div>

          <label class="flex items-center gap-2 cursor-pointer">
            <input
              type="checkbox"
              checked={trustCert()}
              onChange={(e) => setTrustCert(e.currentTarget.checked)}
              class="custom-checkbox"
            />
            <span class="text-xs text-slate-400">Trust Server Certificate</span>
          </label>
        </div>

        {props.error && (
          <div class="mt-4 p-3 bg-red-500/10 border border-red-500/30 rounded text-sm text-red-400 select-text">
            {props.error}
          </div>
        )}

        <button
          type="submit"
          disabled={connecting()}
          class="mt-6 w-full py-2.5 bg-blue-600 hover:bg-blue-500 disabled:bg-slate-700 disabled:text-slate-500 text-white text-sm font-medium rounded transition-colors"
        >
          {connecting() ? "Connecting..." : "Connect"}
        </button>
      </form>
    </div>
  );
}
