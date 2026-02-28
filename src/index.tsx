/* @refresh reload */
import "@fortawesome/fontawesome-free/css/all.min.css";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { render } from "solid-js/web";
import App from "./App.tsx";
import "./index.css";

render(() => <App />, document.getElementById("root")!);

requestAnimationFrame(() => {
  void getCurrentWindow().show();
});
