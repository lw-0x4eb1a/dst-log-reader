import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import './polyfill'
import { i18n } from "@lingui/core";
import { I18nProvider } from "@lingui/react";
import { messages as en } from "./locales/en";
import { messages as zh } from "./locales/zh";

function getLanguage(): "zh" | "en" {
  if (["zh", "en"].includes(localStorage.getItem("language"))) {
    return localStorage.getItem("language") as any;
  }
  else {
    return ["zh-CN", "zh-TW", "zh-HK"].includes(navigator.language) ? "zh" : "en";
  }
}

i18n.load({en, zh})
i18n.activate(getLanguage());

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <I18nProvider i18n={i18n}>
      <App />
    </I18nProvider>
  </React.StrictMode>,
);
