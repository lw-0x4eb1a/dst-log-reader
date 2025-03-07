import "./App.css"
import bg from "./assets/home_bg.png"
import LogList from "./components/LogList"
import MenuIcon from "./components/MenuIcon"
import { WebviewWindow } from "@tauri-apps/api/webviewWindow"

import h_line from "./assets/h_line.png"
import LogBrowserPage from "./pages/LogBrowserPage"
import SettingsPage from "./pages/SettingsPage"
import { Trans, useLingui } from "@lingui/react/macro"
import { useEffect } from "react"
import { listen } from "@tauri-apps/api/event"

const textShadow = `
  0 0 4px #0009,
  1px 1px   3px #0003,
  1px 0     3px #0003,
  0   1px   3px #0003,
  1px -1px  3px #0003,
  -1px 1px  3px #0003,
  -1px -1px 3px #0003,
  -1px 0    3px #0003,
  0 -1px    3px #0003
`

export default function App() {
  const label = WebviewWindow.getCurrent().label
  const { i18n, t } = useLingui()
  const { locale } = i18n

  useEffect(()=> {
    let title = ""
    if (label === "main") {
      title = t`Log Reader`
    }
    else if (label === "settings") {
      title = t`Settings`
    }
    else if (label === "about") {
      title = t`About`
    }
    else {
      // do not change log browser label
      return
    }
    WebviewWindow.getCurrent().setTitle(title)
  }, [locale])

  useEffect(()=> {
    const unlisten = listen("setting", e=> {
      if (e.event === "setting") {
        const { key, value } = e.payload as any
        if (key === "language" && ["zh", "en"].includes(value)) {
          i18n.activate(value)
          window.currentLocale = value
        }
      }
    })
    return ()=> { unlisten.then(f=> f()) }
  }, [])

  if (label === "settings") {
    return <SettingsPage/>
  }

  if (label.startsWith("ds")) {
    return <LogBrowserPage/>
  }

  if (label === "main") {
    return (
      <main className="relative select-none">
        <img 
          src={bg} alt="background" 
          className="absolute -z-10 opacity-80 max-w-screen bottom-0"
        />
        <div className="relative w-screen h-screen">
          <div className="relative p-8 overflow-auto w-full max-h-full">
            <h1 className="text-center bold text-2xl text-gray-200 mb-3"
              style={{textShadow}}>
              <Trans>Browse Logs</Trans>
            </h1>
            <div className="w-full h-2 opacity-80"
              style={{backgroundImage: `url(${h_line})`, backgroundRepeat: "no-repeat", backgroundSize: "100% 100%"}}/>
            <div className="w-full overflow-y-auto -my-1" style={{height: 420, backgroundColor: "#0002"}}>
              <LogList/>
            </div>
            <div className="w-full h-2 opacity-80"
              style={{backgroundImage: `url(${h_line})`, backgroundRepeat: "no-repeat", backgroundSize: "100% 100%"}}/>
          </div>
        </div>
        <footer className="absolute bottom-3 right-3 cursor-pointer">
          <MenuIcon/>
        </footer>
      </main>
    );
  }
}