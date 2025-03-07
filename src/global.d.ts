import type { ModInfo } from "./pages/LogBrowserPage"

declare global {
  interface Window {
    logPath: {
      filepath: string,
      filename: string,
      is_zip: boolean,
    },
    globalModList: ModInfo[],
    currentLocale: string, // zh | en
    currentCopyErrorCommandId: string,
    visitMod: (id: string)=> void,
  }
}