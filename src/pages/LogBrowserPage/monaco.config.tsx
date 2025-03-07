import { loader } from "@monaco-editor/react"
import { getIconPath } from "./geticon"
import { invoke } from "@tauri-apps/api/core"
import { i18n } from "@lingui/core"
import { defineMessage } from "@lingui/core/macro"

export const LANGUAGE_ID = "dst_log_file"
export const THEME_ID = "theme"

window.visitMod = (id: string)=> {
  invoke("open_url", {url: `https://steamcommunity.com/sharedfiles/filedetails/?id=${id}`})
}

setInterval(()=> {
  for (let url of document.querySelectorAll("a")) {
    if (url.getAttribute("data-id")) {
      let id = url.getAttribute("data-id")
      url.removeAttribute("data-id")
      url.addEventListener("click", (e)=> {
        e.preventDefault()
        window.visitMod(id)
      })
    }
  }
}, 50)

loader.init().then(monaco=> {
  monaco.languages.register({id: LANGUAGE_ID})
  monaco.languages.setMonarchTokensProvider(LANGUAGE_ID, {
    tokenizer: {
      root: [
        [/^\[[0-9:]+\]:/, 'sim-time'],
        ["LUA ERROR stack traceback:", 'traceback'],
        [/.*in \(global\) StackTraceToLog \(Lua\).*/, "traceback-debug"],
        ["cGame::StartPlaying", "game-instance"],
        [/workshop-\d+/g, 'workshop-id'],
      ]
    }
  })
  monaco.languages.registerHoverProvider(LANGUAGE_ID, {
    provideHover: function (model, position) {
      let lineContent = model.getLineContent(position.lineNumber)
      let regex = /workshop-\d+\b/g
      let matches = [...lineContent.matchAll(regex)]
      for (const match of matches) {
        if (match.index <= position.column && position.column <= match.index + match[0].length) {
          const id = match[0].substring(9)
          let url = getIconPath(id)
          let is_steam_workshop = /[0-9]{5,15}/.test(id)
          let name = is_steam_workshop && window.globalModList 
            && window.globalModList.find((v)=> v.workshop_id === id)?.name

          return {
            range: new monaco.Range(position.lineNumber, match.index + 1, position.lineNumber, match.index + match[0].length + 1),
            contents: [
              is_steam_workshop && { supportHtml: true, value: '<img data-id="' + id + '" class="mod-icon" width=60 src="' + url + '"/>' },
              name && { value: `**${name}**` },
              { value: `id: ${id}` },
              is_steam_workshop && { supportHtml: true, value: '<a data-id="' + id + '" href="#" class="mod-url">more info..</a>' },
            ]
          }
        }
      }
    }
  })

  function getHintText(type: string): string {
    switch (type) {
      case "game": return " " + i18n._(defineMessage({ message: "In Game" }))
      case "mod": return " " + i18n._(defineMessage({ message: "In Mod" }))
    }
  }

  monaco.languages.registerInlayHintsProvider(LANGUAGE_ID, {
    provideInlayHints(model) {
      let inLuaError = false
      let luaErrorField = []
      
      let totalLines = model.getLineCount()
      let hints = []
      let modSrcLines = []
      for (let i = 1; i <= totalLines; i++) {
        let line = model.getLineContent(i)
        if (line.length >= 2000) continue
        if (line.startsWith("LUA ERROR stack traceback:")) {
          inLuaError = true
          luaErrorField.push([i - 1, -1])
          continue
        }
        if (inLuaError) {
          if (/^\[\d+:\d+:\d+\]:/.test(line)) {
            inLuaError = false
            luaErrorField[luaErrorField.length - 1][1] = i - 1
            continue
          }
          let lineLength = line.length
          line = line.trimStart()
          if (/^scripts\/[^:.]+\.lua(:\d+ in \(|\(\d+,1\))/.test(line) ||
              /^=\[C\] in function /.test(line) ||
              /^=\(tail call\)?/.test(line)) {
            hints.push({
              kind: monaco.languages.InlayHintKind.Type,
              position: { lineNumber: i, column: lineLength + 1},
              label: getHintText("game"),
            })
            continue
          }
          let result = /^\.\.\/mods\/([^/]+)\//.exec(line)
          if (result !== null) {
            modSrcLines.push(i)
            let moddir = result[1]
            let modname = window.globalModList.find((v)=> v.moddir === moddir)?.name || moddir
            hints.push({
              kind: monaco.languages.InlayHintKind.Type,
              position: { lineNumber: i, column: lineLength + 1},
              label: getHintText("mod") + `: ${modname}`,
            })
          }
        }
      }
      // NOT GOOD, but it works...
      let editor = monaco.editor.getEditors()[0]
      let decorations = luaErrorField.map(([start, end])=> {
        return {
          range: new monaco.Range(start, 1, end, 1),
          options: {
            linesDecorationsClassName: "lua_error_decoration",
          }
        }
      })
      // insert inline decorations
      luaErrorField.forEach(([start, end])=> {
        for (let line = start; line <= end; line++) {
          decorations.push({
            range: new monaco.Range(line, 1, line, model.getLineMaxColumn(line)),
            options: {
              /* @ts-ignore optional property */
              inlineClassName: "lua_error_decoration_inline",
            }
          })
        }
      })

      editor.createDecorationsCollection(decorations)
      // monaco.editor.setModelMarkers(model, "mod_src", modSrcLines.map((lineNumber)=> {
      //   return {
      //     startLineNumber: lineNumber,
      //     startColumn: 1,
      //     endLineNumber: lineNumber,
      //     endColumn: model.getLineMaxColumn(lineNumber),
      //     message: "Mod Source",
      //     severity: monaco.MarkerSeverity.Info,
      //   }
      // }))
      return {hints, dispose: ()=>{}}
    },
  })

  function getLenseText(): string {
    return i18n._(defineMessage({ message: "Copy Error Messages" }))
  }

  monaco.languages.registerCodeLensProvider(LANGUAGE_ID, {
    provideCodeLenses: function (model) {
      let inLuaError = false
      let luaErrorField = []
      let totalLines = model.getLineCount()
      for (let i = 1; i <= totalLines; i++) {
        let line = model.getLineContent(i)
        if (line.length >= 2000) continue
        if (line.startsWith("LUA ERROR stack traceback:")) {
          inLuaError = true
          luaErrorField.push([i - 1, -1])
          continue
        }
        if (inLuaError) {
          if (/^\[\d+:\d+:\d+\]:/.test(line)) {
            inLuaError = false
            luaErrorField[luaErrorField.length - 1][1] = i - 1
            continue
          }
        }
      }
      let editor = monaco.editor.getEditors()[0]
      let lenses = luaErrorField.map(([start, end])=> {
        /* @ts-ignore */
        let id = editor.addCommand(0, ()=> {
          // copy error text to clipboard
          let temp = []
          for (let i = start; i <= end; i++) {
            temp.push(model.getLineContent(i))
          }
          let text = temp.join("\n")
          navigator.clipboard.writeText(text)
          window.alert(
            i18n._(defineMessage({ message: "Successfully copied error messages to clipboard." }))
          )
        }, "")

        return {
          range: new monaco.Range(start, 1, end, 1),
          id: `${start}-${end}`,
          command: {
            id,
            title: getLenseText(),
          },
        }
      })
      return { lenses, dispose: () => {} }
    },
  })

  monaco.editor.defineTheme(THEME_ID, {
    base: 'vs',
    inherit: false,
    colors: {
      "editor.lineHighlightBackground": "#5000f010",
    },

    rules: [
      { token: 'traceback', foreground: 'ff0000', fontStyle: 'bold',  },
      { token: 'traceback-debug', foreground: 'ff9000', fontStyle: 'bold', },
      { token: 'sim-time', foreground: 'cccccc' },
      { token: 'game-instance', foreground: '#000000', fontStyle: 'bold' },
      { token: 'workshop-id', foreground: '#00a000', fontStyle: 'bold' },
    ]
  })
})