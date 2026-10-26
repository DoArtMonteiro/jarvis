import "@fontsource-variable/literata/opsz.css";
import "@fontsource-variable/literata/opsz-italic.css";
import "@fontsource-variable/libre-franklin";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
import { render } from "preact";
import { App } from "./components/App";
import "./styles.css";

const root = document.getElementById("app");
if (root) render(<App />, root);
