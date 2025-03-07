import { invoke } from '@tauri-apps/api/core'
import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
import { useCallback, useEffect, useRef, useState } from 'react'
import Editor from '@monaco-editor/react'
import { LANGUAGE_ID, THEME_ID } from './monaco.config'
import SidePanel from '../../components/SidePanel'
import { useModKey } from '../../hooks'

export type LogInitData = {
  label: string,
  exists: boolean,
  active?: boolean,
  mtime: number,
  comment: LogComment,

  debug_content: string,
}

export type LogField = {
  start: number,
  end: number,
  type: string,
  extra?: string,
}

export type ModInfo = {
  moddir: string,
  name: string,
  version?: string,
  workshop_id?: string,
}

export type LogComment = {
  fields: any[],
  has_stacktrace: boolean,
  has_lua_crash: boolean,
  has_c_crash: boolean,
  build_version: string,
  build_platform: string,
  build_arch: string,
  databundles_mounting_state: {[K: string]: boolean},
  total_time: number[],
  mods: ModInfo[],
}

export type NavigateAction = "next-error" | "prev-error" | "next-instance" | "prev-instance"

const GAME_INSTANCE_FLAG = "cGame::StartPlaying"
const LUA_ERROR_FLAG = "LUA ERROR stack traceback:"
function jumpTo(editor, neddle: string, dir: "up" | "down") {
  const model = editor.getModel()
  const currentPosition = editor.getPosition() || { lineNumber: 1, column: 1 }
  const searchOptions = {
    regex: false,
    matchCase: true,
    wholeWord: true,
    searchString: neddle,
  }
  let match = null
  if (dir === "up") {
    match = model.findPreviousMatch(neddle, currentPosition, searchOptions)
  }
  else {
    currentPosition.lineNumber += 1 // it's need for finding next match
    match = model.findNextMatch(neddle, currentPosition, searchOptions)
  }

  if (match) {
    const targetPosition = {
      lineNumber: match.range.startLineNumber,
      column: match.range.startColumn,
    }
    editor.setPosition(targetPosition)
    editor.revealPositionNearTop(targetPosition, 0 /* smooth */)
  }
}

export default function LogBrowserPage() {
  /*@ts-ignore*/
  const logPath = window.logPath
  const label = WebviewWindow.getCurrent().label
  const [content, setContent] = useState("")
  const [comment, setComment] = useState<LogComment>(null)
  const editorRef = useRef(null)

  useEffect(() => {
    invoke<string>("load_log_init", {id: label}).then(
      res=> {
        const data = JSON.parse(res) as LogInitData
        setContent(data.debug_content)
        setComment(data.comment)
      },
      err=> {
        console.error(err)
      }
    )
  }
  , [label])

  useEffect(()=> {
    if (comment) {
      // expose mod list to global
      window.globalModList = comment.mods
    }
  }, [comment])

  useEffect(()=> {
    editorRef.current?.setValue(content)
  }, [content])

  const onMount = useCallback(editor=> {
    editorRef.current = editor
  }, [])

  const modKey = useModKey()

  const navigate = useCallback((id: NavigateAction)=> {
    const editor = editorRef.current
    if (!editor) return
    switch (id) {
      case "next-error": return jumpTo(editor, LUA_ERROR_FLAG, "down")
      case "prev-error": return jumpTo(editor, LUA_ERROR_FLAG, "up")
      case "next-instance": return jumpTo(editor, GAME_INSTANCE_FLAG, "down")
      case "prev-instance": return jumpTo(editor, GAME_INSTANCE_FLAG, "up")
    }
  }, [])

  useEffect(()=> {
    // global key binding
    const keydownHandler = (e: KeyboardEvent)=> {
      console.log(e.key)
      let pressingCtrl = modKey === "meta" && e.metaKey || modKey === "ctrl" && e.ctrlKey
      if (e.key === "f" && pressingCtrl) 
        editorRef.current.trigger('key', 'actions.find')
      else if (e.key === "p" && pressingCtrl)
        editorRef.current.trigger('key', 
          e.shiftKey ? 'actions.quickCommandPalette' : 'actions.quickOutline')
      else if (e.key === "escape") 
        editorRef.current.trigger('key', 'closeFindWidget')
      else if (e.key === "=" && pressingCtrl)
        editorRef.current.trigger('key', 'editor.action.fontZoomIn')
      else if (e.key === "-" && pressingCtrl)
        editorRef.current.trigger('key', 'editor.action.fontZoomOut')
      else if (e.key === "0" && pressingCtrl)
        editorRef.current.trigger('key', 'editor.action.fontZoomReset')
    }
    window.addEventListener('keydown', keydownHandler)
    return ()=> window.removeEventListener('keydown', keydownHandler)
  }, [modKey])
    
  return (
    <div className="flex w-screen h-screen">
      <SidePanel
        navigate={navigate}
        logContent={content}
        logComment={comment}
      />
      <Editor
        defaultLanguage={LANGUAGE_ID}
        defaultValue={content}
        theme={THEME_ID}
        options={{
          readOnly: true,
          fontSize: 12,
          scrollBeyondLastLine: false,
          // smoothScrolling: true,
          minimap: {enabled: false, renderCharacters: true, size: "proportional", autohide: false},
        }}
        onMount={onMount}
      />
    </div>
  )
}
