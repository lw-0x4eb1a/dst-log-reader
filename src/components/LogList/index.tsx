import { invoke } from "@tauri-apps/api/core"
import React, { useEffect, useRef, useState } from "react"
import white from "../../assets/white_rect.png"
import white_70 from "../../assets/white_rect_70.png"
import ModifiedTime from "../ModifiedTime"
import { getName } from "../../util"
import { useIntersectionObserver } from "../../hooks"
import { formatRunTime } from "../RunTime"
import { Trans } from "@lingui/react/macro"


const rectStyle: React.CSSProperties = {
  backgroundImage: `url(${white})`,
  backgroundSize: "100% 100%",
  backgroundRepeat: "no-repeat",
}

type LogData = {
  game: "ds" | "dst",
  filepath: string,
  filename: string,
  is_zip: boolean,
  mtime: number,
  filesize: number,
  has_lua_crash: boolean,
  has_c_crash: boolean,
  total_time: [string, string, string],
}

function diffStringList(a: string[], b: string[]) {
  if (a.length !== b.length) {
    return true
  }
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) {
      return true
    }
  }
  return false
}

export default function LogList() {
  const [logs, setLogs] = useState<LogData[]>([])
  const [dirError, setDirError] = useState("")
  // const [loading, setLoading] = useState(false)

  useEffect(()=> {
    let lastResult = [""]
    let update = ()=> {
      invoke<string[]>("list_all_logs").then(
        res=> {
          if (diffStringList(res, lastResult)) {
            console.log("Update log list")
            setLogs(res.map((v)=> {
              let data = JSON.parse(v)
              return data
            }))
            lastResult = res
          }
        },
        setDirError,
      )
    }
    update()
    const timer = setInterval(update, 1000 * 10)
    return ()=> clearInterval(timer)
  }, [])

  return (
    <div className="p-2 w-full">
      {
        dirError && 
        <div className="bg-red-100/70 p-2 m-2 rounded-sm border-red-500 border-1">
          <p className="text-center text-red-500 select-text">
            <Trans>ERROR: {dirError}</Trans>
          </p>
        </div>
      }
      {
        // loading ? <p>Loading...</p> : null
      }
      {
        logs.map(v=> <LogItem {...v} key={`[${v.is_zip ? "zip": "file"}]${v.filepath}-${v.filename}`}/>)
      }
      <p className="mb-10 text-center">
        <span className="underline cursor-pointer p-1 text-gray-100 hover:text-blue-300">
          <Trans>Could not find the log file?</Trans>
        </span>
      </p>
      {/* add red filter */}
      {/* <svg>
        <defs>
          <filter id="set-add-color-red">
            <feColorMatrix 
              in="SourceGraphic" 
              type="matrix"
              values="0.9 0 0 0 0
                      0 0.4 0 0 0
                      0 0 0.4 0 0
                      0 0 0 1 0"/>
          </filter>
        </defs>
      </svg> */}
    </div>
  )
}

function LogItem(props: LogData) {
  const {filepath, filename, is_zip} = props
  const [hover, setHover] = useState(false)
  // const [error, setError] = useState("")
  const [hasLuaCrash, setHasLuaCrash] = useState(false)
  const [hasCCrash, setHasCCrash] = useState(false)
  const [totalTime, setTotalTime] = useState([0, 0, 0])
  const hasBug = hasLuaCrash || hasCCrash
  const div = useRef<HTMLDivElement>(null)
  const {appeared} = useIntersectionObserver({ref: div})

  useEffect(()=> {
    if (appeared) {
      invoke<string>("load_log_abstract", {filepath, filename, is_zip}).then(
        res=> {
          const data = JSON.parse(res)
          setHasLuaCrash(data.has_lua_crash)
          setHasCCrash(data.has_c_crash)
          setTotalTime(data.total_time.map((v: string)=> parseInt(v)))
        },
        console.error)
    }
  }, [appeared])

  const style = {
    ...rectStyle,
    height: 80,
    backgroundImage: `url(${hover ? white : white_70})`,
    // filter: hasBug ? `url(#set-add-color-red)` : "",
  }
  return (
    <div
      ref={div} 
      className="w-full overflow-hidden mb-2 p-2 cursor-pointer"
      style={style}
      onMouseEnter={()=> setHover(true)}
      onMouseLeave={()=> setHover(false)}
      onClick={()=> invoke("open_log", {filepath, filename, is_zip, from: "local"})}
      key={props.filepath}>
      <h1 className={"font-bold " + (hasBug ? " text-red-500" : "")}>
        {getName(props.filename)}
      </h1>
      {/* <p>{props.filepath}</p> */}
      {/* <p>{props.filesize}</p> */}
      <p className="text-sm text-black/50">
      <ModifiedTime mtime={props.mtime}/>
      <span className="mx-3 opacity-50">|</span>
      <span>
        Run for {formatRunTime(totalTime)}
      </span>
      </p>

      <br/>
    </div>
  )
}