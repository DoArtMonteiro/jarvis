import "@fontsource-variable/inter";
import "@fontsource-variable/fraunces";
import { render } from "preact";
import { App } from "./components/App";
import "./styles.css";

const root = document.getElementById("app");
if (root) render(<App />, root);
